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
