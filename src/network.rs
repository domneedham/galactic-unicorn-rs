use cyw43_pio::PioSpi;
use embassy_executor::Spawner;
use embassy_futures::select::{select, Either};
use embassy_net::{Ipv4Address, Ipv4Cidr, Stack, StackResources};
use embassy_rp::{
    bind_interrupts,
    gpio::{Level, Output},
    peripherals::{DMA_CH1, PIN_23, PIN_24, PIN_25, PIN_29, PIO1},
    pio::{InterruptHandler, Pio},
};
use embassy_time::{Duration, Timer};
use heapless::Vec;
use static_cell::StaticCell;

use crate::{
    config::*,
    mqtt::clients::{RECEIVE_CLIENT_ERROR, SEND_CLIENT_ERROR},
    system::SystemState,
};

/// Network states.
#[derive(Clone, Copy)]
pub enum NetworkState {
    NotInitialised,
    Connected,
    Error,
}

bind_interrupts!(struct PioIrqs {
    PIO1_IRQ_0 => InterruptHandler<PIO1>;
});

/// Cyw43 runner task.
#[embassy_executor::task]
async fn wifi_task(
    runner: cyw43::Runner<
        'static,
        Output<'static, PIN_23>,
        PioSpi<'static, PIN_25, PIO1, 0, DMA_CH1>,
    >,
) -> ! {
    runner.run().await
}

/// Embassy net stack runner task.
#[embassy_executor::task]
async fn net_task(stack: &'static Stack<cyw43::NetDriver<'static>>) -> ! {
    stack.run().await
}

/// Create and join the wifi network. Will wait until it has successfully joined.
pub async fn create_and_join_network(
    spawner: Spawner,
    app_state: &'static SystemState,
    pin_23: PIN_23,
    pin_24: PIN_24,
    pin_25: PIN_25,
    pin_29: PIN_29,
    pio_1: PIO1,
    dma_ch1: DMA_CH1,
) -> &'static Stack<cyw43::NetDriver<'static>> {
    let fw = include_bytes!("../cyw43-firmware/43439A0.bin");
    let clm = include_bytes!("../cyw43-firmware/43439A0_clm.bin");

    // wifi
    let pwr = Output::new(pin_23, Level::Low);
    let cs = Output::new(pin_25, Level::High);
    let mut pio = Pio::new(pio_1, PioIrqs);
    let spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        pio.irq0,
        cs,
        pin_24,
        pin_29,
        dma_ch1,
    );
    static STATE: StaticCell<cyw43::State> = StaticCell::new();
    let state = STATE.init(cyw43::State::new());

    let (net_device, mut control, runner) = cyw43::new(state, pwr, spi, fw).await;
    spawner.spawn(wifi_task(runner)).unwrap();

    control.init(clm).await;
    control
        .set_power_management(cyw43::PowerManagementMode::PowerSave)
        .await;

    let mut addresses: Vec<Ipv4Address, 3> = Vec::new();
    addresses.insert(0, Ipv4Address::new(1, 1, 1, 1)).unwrap();
    let config = embassy_net::Config::ipv4_static(embassy_net::StaticConfigV4 {
        address: Ipv4Cidr::new(Ipv4Address::new(IP_A1, IP_A2, IP_A3, IP_A4), PREFIX_LENGTH),
        dns_servers: addresses,
        gateway: Some(Ipv4Address::new(GW_A1, GW_A2, GW_A3, GW_A4)),
    });
    // Generate random seed
    let seed = 0x0123_4567_89ab_cdef; // chosen by fair dice roll. guarenteed to be random.

    // Init network stack
    static STACK: StaticCell<Stack<cyw43::NetDriver<'static>>> = StaticCell::new();
    static RESOURCES: StaticCell<StackResources<10>> = StaticCell::new();
    let stack = &*STACK.init(Stack::new(
        net_device,
        config,
        RESOURCES.init(StackResources::<10>::new()),
        seed,
    ));

    spawner.spawn(net_task(stack)).unwrap();

    loop {
        match control.join_wpa2(WIFI_NETWORK, WIFI_PASSWORD).await {
            Ok(_) => break,
            Err(_) => {
                Timer::after(Duration::from_secs(2)).await;
            }
        }
    }

    app_state.set_network_state(NetworkState::Connected).await;

    spawner.spawn(monitor_network_task(app_state)).unwrap();

    stack
}

/// Wait for messages from MQTT clients and update network state accordingly.
/// There is no built in detection for network errors hence the relying on MQTT net stack.
#[embassy_executor::task]
async fn monitor_network_task(app_state: &'static SystemState) {
    let res = match select(SEND_CLIENT_ERROR.wait(), RECEIVE_CLIENT_ERROR.wait()).await {
        Either::First(val) => val,
        Either::Second(val) => val,
    };

    if res {
        app_state.set_network_state(NetworkState::Connected).await;
    } else {
        app_state.set_network_state(NetworkState::Error).await;
    }
}

pub mod access_point {
    use core::net::{IpAddr, SocketAddr};

    use cyw43_pio::PioSpi;
    use edge_dhcp::{
        server::{Server, ServerOptions},
        Ipv4Addr,
    };
    use edge_nal::UdpBind;
    use edge_nal_embassy::{Udp, UdpBuffers};
    use embassy_executor::Spawner;
    use embassy_futures::select::select;
    use embassy_net::{Ipv4Address, Ipv4Cidr, Stack, StackResources};
    use embassy_rp::{
        gpio::{Level, Output},
        peripherals::{DMA_CH1, PIN_23, PIN_24, PIN_25, PIN_29, PIO1},
        pio::Pio,
    };
    use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, signal::Signal};
    use embassy_time::{Duration, Timer};
    use picoserve::routing::get;
    use static_cell::{make_static, StaticCell};

    use crate::{
        config::*,
        display::messages::DisplayTextMessage,
        network::{net_task, wifi_task, PioIrqs},
    };

    /// Signal for when the DHCP server has given a lease.
    static LEASE_GIVEN: Signal<ThreadModeRawMutex, bool> = Signal::new();

    /// Start an open access point.
    pub async fn create_network(
        spawner: Spawner,
        pin_23: PIN_23,
        pin_24: PIN_24,
        pin_25: PIN_25,
        pin_29: PIN_29,
        pio_1: PIO1,
        dma_ch1: DMA_CH1,
    ) -> &'static Stack<cyw43::NetDriver<'static>> {
        let fw = include_bytes!("../cyw43-firmware/43439A0.bin");
        let clm = include_bytes!("../cyw43-firmware/43439A0_clm.bin");

        // wifi
        let pwr = Output::new(pin_23, Level::Low);
        let cs = Output::new(pin_25, Level::High);
        let mut pio = Pio::new(pio_1, PioIrqs);
        let spi = PioSpi::new(
            &mut pio.common,
            pio.sm0,
            pio.irq0,
            cs,
            pin_24,
            pin_29,
            dma_ch1,
        );
        static STATE: StaticCell<cyw43::State> = StaticCell::new();
        let state = STATE.init(cyw43::State::new());

        let (net_device, mut control, runner) = cyw43::new(state, pwr, spi, fw).await;
        spawner.spawn(wifi_task(runner)).unwrap();

        control.init(clm).await;
        control
            .set_power_management(cyw43::PowerManagementMode::PowerSave)
            .await;

        let config = embassy_net::Config::ipv4_static(embassy_net::StaticConfigV4 {
            address: Ipv4Cidr::new(Ipv4Address::new(192, 168, 1, 254), 24),
            dns_servers: heapless::Vec::new(),
            gateway: None,
        });
        // Generate random seed
        let seed = 0x0123_4567_89ab_cdef; // chosen by fair dice roll. guarenteed to be random.

        // Init network stack
        static STACK: StaticCell<Stack<cyw43::NetDriver<'static>>> = StaticCell::new();
        static RESOURCES: StaticCell<StackResources<5>> = StaticCell::new();
        let stack = &*STACK.init(Stack::new(
            net_device,
            config,
            RESOURCES.init(StackResources::<5>::new()),
            seed,
        ));

        spawner.spawn(net_task(stack)).unwrap();

        control.start_ap_open(DEVICE_ID, 5).await;

        spawner.spawn(dhcp_server(stack)).unwrap();

        DisplayTextMessage::from_app(
            constcat::concat!("Connect to ", DEVICE_ID, " wifi network"),
            None,
            None,
            None,
        )
        .send()
        .await;

        // wait for a lease to be given before continuing
        LEASE_GIVEN.wait().await;

        fn make_app() -> picoserve::Router<AppRouter> {
            picoserve::Router::new()
                .route(
                    "/",
                    get(async || picoserve::response::File::html(include_str!("./web/index.html")))
                        .post(
                            |picoserve::extract::Form(Settings {
                                 wifi_network,
                                 wifi_password,
                                 ip_address,
                                 prefix_length,
                                 gateway,
                                 mqtt_broker,
                                 mqtt_broker_port,
                                 mqtt_username,
                                 mqtt_password,
                                 base_mqtt_topic,
                                 device_id,
                                 hass_base_mqtt_topic,
                             })| {
                                picoserve::response::DebugValue((
                                    ("wifi_network", wifi_network),
                                    ("wifi_password", wifi_password),
                                    ("ip_address", ip_address),
                                    ("prefix_length", prefix_length),
                                    ("gateway", gateway),
                                    ("mqtt_broker", mqtt_broker),
                                    ("mqtt_broker_port", mqtt_broker_port),
                                    ("mqtt_username", mqtt_username),
                                    ("mqtt_password", mqtt_password),
                                    ("base_mqtt_topic", base_mqtt_topic),
                                    ("device_id", device_id),
                                    ("hass_base_mqtt_topic", hass_base_mqtt_topic),
                                ))
                            },
                        ),
                )
                .route(
                    "/index.css",
                    get(|| picoserve::response::File::css(include_str!("./web/index.css"))),
                )
        }

        let app = make_static!(make_app());

        let config = make_static!(picoserve::Config::new(picoserve::Timeouts {
            start_read_request: Some(Duration::from_secs(5)),
            read_request: Some(Duration::from_secs(1)),
            write: Some(Duration::from_secs(1)),
        })
        .keep_connection_alive());

        for id in 0..WEB_TASK_POOL_SIZE {
            spawner.must_spawn(picoserve_task(stack, id, app, config));
        }

        // once a lease has been given, inform the user with an instruction
        DisplayTextMessage::from_app("Now go to 192.168.1.254 in your browser!", None, None, None)
            .send_and_show_now()
            .await;

        // this will just repeat
        DisplayTextMessage::from_app("Go to 192.168.1.254 in your browser!", None, None, None)
            .send()
            .await;

        stack
    }

    /// Start and run the DHCP server.
    #[embassy_executor::task]
    async fn dhcp_server(stack: &'static embassy_net::Stack<cyw43::NetDriver<'static>>) {
        let buffers = UdpBuffers::<2, 1024, 1024, 2>::new();
        let edge_net_socket = Udp::new(stack, &buffers);
        let mut socket = edge_net_socket
            .bind(SocketAddr::new(
                IpAddr::V4(core::net::Ipv4Addr::new(192, 168, 1, 254)),
                67,
            ))
            .await
            .unwrap();

        let ip = edge_dhcp::Ipv4Addr::new(192, 168, 1, 254);
        let mut server = Server::<10>::new(ip);
        server.range_start = Ipv4Addr::new(192, 168, 1, 50);
        server.range_end = Ipv4Addr::new(192, 168, 1, 200);

        let mut gw_buf = [Ipv4Addr::UNSPECIFIED];
        let dns = [Ipv4Addr::new(1, 1, 1, 1)];
        let mut server_options = ServerOptions::new(ip, Some(&mut gw_buf));
        server_options.dns = &dns;

        let mut buf = [0; 1500];

        // wait for a lease to be given then break loop
        loop {
            select(
                edge_dhcp::io::server::run(&mut server, &server_options, &mut socket, &mut buf),
                Timer::after_secs(5),
            )
            .await;

            if !server.leases.is_empty() {
                LEASE_GIVEN.signal(true);
                break;
            }
        }

        // once a lease is given, just run constantly
        edge_dhcp::io::server::run(&mut server, &server_options, &mut socket, &mut buf)
            .await
            .unwrap();
    }

    const WEB_TASK_POOL_SIZE: usize = 3;

    type AppRouter = impl picoserve::routing::PathRouter;

    /// Start and run the web server.
    #[embassy_executor::task(pool_size = WEB_TASK_POOL_SIZE)]
    async fn picoserve_task(
        stack: &'static embassy_net::Stack<cyw43::NetDriver<'static>>,
        id: usize,
        app: &'static picoserve::Router<AppRouter>,
        config: &'static picoserve::Config<Duration>,
    ) -> ! {
        let port = 80;
        let mut tcp_rx_buffer = [0; 1024];
        let mut tcp_tx_buffer = [0; 1024];
        let mut http_buffer = [0; 2048];

        picoserve::listen_and_serve(
            id,
            app,
            config,
            stack,
            port,
            &mut tcp_rx_buffer,
            &mut tcp_tx_buffer,
            &mut http_buffer,
        )
        .await
    }

    const MAX_STR_LEN: usize = 32;

    #[derive(serde::Serialize, serde::Deserialize)]
    struct Settings {
        wifi_network: heapless::String<MAX_STR_LEN>,
        wifi_password: heapless::String<MAX_STR_LEN>,
        ip_address: heapless::String<MAX_STR_LEN>,
        prefix_length: u8,
        gateway: heapless::String<MAX_STR_LEN>,
        mqtt_broker: Option<heapless::String<MAX_STR_LEN>>,
        mqtt_broker_port: u16,
        mqtt_username: Option<heapless::String<MAX_STR_LEN>>,
        mqtt_password: Option<heapless::String<MAX_STR_LEN>>,
        base_mqtt_topic: heapless::String<MAX_STR_LEN>,
        device_id: heapless::String<MAX_STR_LEN>,
        hass_base_mqtt_topic: heapless::String<MAX_STR_LEN>,
    }
}
