use core::net::{IpAddr, SocketAddr};

use cyw43_pio::PioSpi;
use edge_dhcp::{
    server::{Server, ServerOptions},
    Ipv4Addr,
};
use edge_nal::UdpBind;
use edge_nal_embassy::{Udp, UdpBuffers};
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
use picoserve::routing::get;
use static_cell::{make_static, StaticCell};

use crate::{
    config::*,
    display::messages::DisplayTextMessage,
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

bind_interrupts!(struct Irqs {
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
    let mut pio = Pio::new(pio_1, Irqs);
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

/// Create and join the wifi network. Will wait until it has successfully joined.
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
    let mut pio = Pio::new(pio_1, Irqs);
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
    static RESOURCES: StaticCell<StackResources<10>> = StaticCell::new();
    let stack = &*STACK.init(Stack::new(
        net_device,
        config,
        RESOURCES.init(StackResources::<10>::new()),
        seed,
    ));

    spawner.spawn(net_task(stack)).unwrap();

    control.start_ap_open(DEVICE_ID, 5).await;

    spawner.spawn(dhcp_server(stack)).unwrap();

    fn create_picoserve_router() -> picoserve::Router<AppRouter> {
        picoserve::Router::new().route(
            "/",
            get(|| picoserve::response::File::html(include_str!("./web/index.html"))),
        )
    }

    let app = make_static!(create_picoserve_router());

    let config = make_static!(picoserve::Config::new(picoserve::Timeouts {
        start_read_request: Some(Duration::from_secs(5)),
        read_request: Some(Duration::from_secs(1)),
        write: Some(Duration::from_secs(1)),
    })
    .keep_connection_alive());

    spawner
        .spawn(picoserve_task(0, stack, app, config))
        .unwrap();

    // app_state.set_network_state(NetworkState::Connected).await;

    // spawner.spawn(monitor_network_task(app_state)).unwrap();

    DisplayTextMessage::from_app("Go to 192.168.1.254 in your browser!", None, None, None)
        .send()
        .await;

    stack
}

pub type AppRouter = impl picoserve::routing::PathRouter;

#[embassy_executor::task]
async fn picoserve_task(
    id: usize,
    stack: &'static embassy_net::Stack<cyw43::NetDriver<'static>>,
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

    edge_dhcp::io::server::run(&mut server, &server_options, &mut socket, &mut buf)
        .await
        .unwrap();
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
