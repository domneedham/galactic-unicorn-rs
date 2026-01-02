use cyw43::JoinOptions;
use cyw43_pio::{PioSpi, DEFAULT_CLOCK_DIVIDER};
use embassy_executor::Spawner;
use embassy_futures::select::{select, Either};
use embassy_net::{Stack, StackResources};
use embassy_rp::{
    bind_interrupts,
    gpio::{Level, Output},
    peripherals::{DMA_CH1, PIN_23, PIN_24, PIN_25, PIN_29, PIO1},
    pio::{Common, InterruptHandler, Pio},
};
use embassy_time::{Duration, Timer};
use static_cell::StaticCell;

use crate::{
    config::*,
    mqtt::clients::{RECEIVE_CLIENT_ERROR, SEND_CLIENT_ERROR},
    settings::Settings,
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
    runner: cyw43::Runner<'static, Output<'static>, PioSpi<'static, PIO1, 0, DMA_CH1>>,
) -> ! {
    runner.run().await
}

/// Embassy net stack runner task.
#[embassy_executor::task]
async fn net_task(mut runner: embassy_net::Runner<'static, cyw43::NetDriver<'static>>) -> ! {
    runner.run().await
}

/// Create and join the wifi network. Will wait until it has successfully joined.
pub async fn create_and_join_network(
    spawner: Spawner,
    app_state: &'static SystemState,
    settings: &'static Settings,
    pin_23: embassy_rp::Peri<'static, PIN_23>,
    pin_24: embassy_rp::Peri<'static, PIN_24>,
    pin_25: embassy_rp::Peri<'static, PIN_25>,
    pin_29: embassy_rp::Peri<'static, PIN_29>,
    pio_1: embassy_rp::Peri<'static, PIO1>,
    dma_ch1: embassy_rp::Peri<'static, DMA_CH1>,
) -> Stack<'static> {
    log::info!("Network: Starting network initialization");
    let fw = include_bytes!("../cyw43-firmware/43439A0.bin");
    let clm = include_bytes!("../cyw43-firmware/43439A0_clm.bin");
    log::info!("Network: Firmware loaded");

    // wifi
    let pwr = Output::new(pin_23, Level::Low);
    let cs = Output::new(pin_25, Level::High);
    let pio = Pio::new(pio_1, PioIrqs);

    static PIO_COMMON: StaticCell<Common<PIO1>> = StaticCell::new();
    let common = PIO_COMMON.init(pio.common);

    let spi = PioSpi::new(
        common,
        pio.sm0,
        DEFAULT_CLOCK_DIVIDER,
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

    log::info!("Network: Initializing WiFi chip");
    control.init(clm).await;
    control
        .set_power_management(cyw43::PowerManagementMode::PowerSave)
        .await;
    log::info!("Network: WiFi chip initialized");

    let config: Option<embassy_net::Config>;
    if USE_DHCP {
        log::info!("Network: Configuring DHCP");
        config = Some(embassy_net::Config::dhcpv4(Default::default()));
    } else {
        log::info!("Network: Configuring static IP is not currently supported");
        log::info!("Network: Configuring DHCP");
        config = Some(embassy_net::Config::dhcpv4(Default::default()));

        // let mut addresses: Vec<Ipv4Address, 3> = Vec::new();
        // addresses.insert(0, Ipv4Address::new(8, 8, 8, 8)).unwrap(); // Google DNS

        // let static_ip = Ipv4Address::new(IP_A1, IP_A2, IP_A3, IP_A4);
        // let gateway_ip = Ipv4Address::new(GW_A1, GW_A2, GW_A3, GW_A4);

        // log::info!("Network: Configuring static IP: {:?}", static_ip);
        // log::info!("Network: Gateway: {:?}", gateway_ip);
        // log::info!("Network: DNS: 8.8.8.8");

        // config = Some(embassy_net::Config::ipv4_static(
        //     embassy_net::StaticConfigV4 {
        //         address: Ipv4Cidr::new(static_ip, PREFIX_LENGTH),
        //         dns_servers: addresses,
        //         gateway: Some(gateway_ip),
        //     },
        // ));
    }

    // Generate random seed
    let seed = 0x0123_4567_89ab_cdef; // chosen by fair dice roll. guaranteed to be random.

    // Init network stack
    static RESOURCES: StaticCell<StackResources<13>> = StaticCell::new();
    let (stack, runner) = embassy_net::new(
        net_device,
        config.unwrap(),
        RESOURCES.init(StackResources::new()),
        seed,
    );

    spawner.spawn(net_task(runner)).unwrap();
    // Small delay to ensure net_task has started before we proceed with WiFi join
    Timer::after(Duration::from_millis(100)).await;
    log::info!("Network: Stack initialized");

    log::info!("Network: Joining WiFi network: {}", settings.wifi_network);
    let mut attempts = 0;
    loop {
        attempts += 1;
        log::info!("Network: Join attempt {}", attempts);
        match control
            .join(
                &settings.wifi_network,
                JoinOptions::new(settings.wifi_password.as_bytes()),
            )
            .await
        {
            Ok(_) => {
                log::info!("Network: Successfully joined WiFi network");
                break;
            }
            Err(e) => {
                log::error!("Network: Join attempt {} failed: {:?}", attempts, e);
                Timer::after(Duration::from_secs(2)).await;
            }
        }
    }

    log::info!("Network: Waiting for link to be up...");
    stack.wait_link_up().await;
    log::info!("Network: Link is up");

    log::info!("Network: Waiting for config to be ready...");
    stack.wait_config_up().await;
    log::info!("Network: Config is ready");

    log::info!("Network: IP address: {:?}", stack.config_v4());

    app_state.set_network_state(NetworkState::Connected).await;
    log::info!("Network: Initialization complete");

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

    use cortex_m::peripheral::SCB;
    use cyw43_pio::{PioSpi, DEFAULT_CLOCK_DIVIDER};
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
        pio::{Common, Pio},
    };
    use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, signal::Signal};
    use embassy_time::{Duration, Timer};
    use picoserve::AppBuilder;
    use static_cell::{make_static, StaticCell};

    use crate::{
        config::*,
        display::messages::DisplayTextMessage,
        flash,
        network::{net_task, wifi_task, PioIrqs},
        settings::Settings,
    };

    /// Signal for when the DHCP server has given a lease.
    static LEASE_GIVEN: Signal<ThreadModeRawMutex, bool> = Signal::new();

    /// Start an open access point.
    pub async fn create_network(
        spawner: Spawner,
        pin_23: embassy_rp::Peri<'static, PIN_23>,
        pin_24: embassy_rp::Peri<'static, PIN_24>,
        pin_25: embassy_rp::Peri<'static, PIN_25>,
        pin_29: embassy_rp::Peri<'static, PIN_29>,
        pio_1: embassy_rp::Peri<'static, PIO1>,
        dma_ch1: embassy_rp::Peri<'static, DMA_CH1>,
    ) -> Stack<'static> {
        log::info!("Network: Starting network initialization");
        let fw = include_bytes!("../cyw43-firmware/43439A0.bin");
        let clm = include_bytes!("../cyw43-firmware/43439A0_clm.bin");
        log::info!("Network: Firmware loaded");

        // wifi
        let pwr = Output::new(pin_23, Level::Low);
        let cs = Output::new(pin_25, Level::High);
        let pio = Pio::new(pio_1, PioIrqs);

        static PIO_COMMON: StaticCell<Common<PIO1>> = StaticCell::new();
        let common = PIO_COMMON.init(pio.common);

        let spi = PioSpi::new(
            common,
            pio.sm0,
            DEFAULT_CLOCK_DIVIDER,
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

        log::info!("Network: Initializing WiFi chip");
        control.init(clm).await;
        control
            .set_power_management(cyw43::PowerManagementMode::PowerSave)
            .await;
        log::info!("Network: WiFi chip initialized");

        let config = embassy_net::Config::ipv4_static(embassy_net::StaticConfigV4 {
            address: Ipv4Cidr::new(Ipv4Address::new(192, 168, 1, 254), 24),
            dns_servers: heapless::Vec::new(),
            gateway: None,
        });

        // Generate random seed
        let seed = 0x0123_4567_89ab_cdef; // chosen by fair dice roll. guaranteed to be random.

        // Init network stack
        static RESOURCES: StaticCell<StackResources<10>> = StaticCell::new();
        let (stack, runner) = embassy_net::new(
            net_device,
            config,
            RESOURCES.init(StackResources::new()),
            seed,
        );

        spawner.spawn(net_task(runner)).unwrap();
        // Small delay to ensure net_task has started before we proceed with WiFi join
        Timer::after(Duration::from_millis(100)).await;
        log::info!("Network: Stack initialized");

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

        static APP: StaticCell<picoserve::AppRouter<AppProps>> = StaticCell::new();
        let app = APP.init(picoserve::AppRouter::<AppProps>::from(AppProps.build_app()));

        let config = make_static!(picoserve::Config::new(picoserve::Timeouts {
            start_read_request: Some(Duration::from_secs(5)),
            persistent_start_read_request: Some(Duration::from_secs(1)),
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
    async fn dhcp_server(stack: embassy_net::Stack<'static>) {
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
        let mut server = Server::<_, 10>::new(|| embassy_time::Instant::now().as_millis(), ip);
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

    struct AppProps;

    impl picoserve::AppBuilder for AppProps {
        type PathRouter = impl picoserve::routing::PathRouter;

        fn build_app(self) -> picoserve::Router<Self::PathRouter> {
            picoserve::Router::new()
                .route(
                    "/",
                    picoserve::routing::get_service(picoserve::response::File::html(include_str!(
                        "./web/index.html"
                    )))
                    .post(
                        async |picoserve::extract::Form(crate::settings::Settings {
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
                                   is_initialized,
                               })|
                               -> () {
                            flash::write_to_flash(&Settings::new(
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
                                is_initialized,
                            ))
                            .await;
                            SCB::sys_reset();
                        },
                    ),
                )
                .route(
                    "/index.css",
                    picoserve::routing::get_service(picoserve::response::File::css(include_str!(
                        "./web/index.css"
                    ))),
                )
        }
    }

    /// Start and run the web server.
    #[embassy_executor::task(pool_size = WEB_TASK_POOL_SIZE)]
    async fn picoserve_task(
        stack: embassy_net::Stack<'static>,
        id: usize,
        app: &'static picoserve::AppRouter<AppProps>,
        config: &'static picoserve::Config<Duration>,
    ) -> ! {
        let port = 80;
        let mut tcp_rx_buffer = [0; 1024];
        let mut tcp_tx_buffer = [0; 1024];
        let mut http_buffer = [0; 2048];

        loop {
            let _ = picoserve::Server::new(app, config, &mut http_buffer)
                .listen_and_serve(id, stack, port, &mut tcp_rx_buffer, &mut tcp_tx_buffer)
                .await;
        }
    }
}
