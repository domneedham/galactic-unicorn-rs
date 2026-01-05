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
use core::sync::atomic::{AtomicBool, Ordering};
use embassy_sync::{
    blocking_mutex::raw::{CriticalSectionRawMutex, ThreadModeRawMutex},
    mutex::Mutex,
    signal::Signal,
};
use embassy_time::{Duration, Timer};
use static_cell::StaticCell;

use crate::{
    config::*,
    mqtt::clients::{RECEIVE_CLIENT_ERROR, SEND_CLIENT_ERROR},
    system::SystemState,
};

/// Network states.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NetworkState {
    NotInitialised,
    Initializing,
    Connected,
    Error,
}

/// Mutex-protected Option for storing the network stack once ready.
/// Stack is Copy, so we can clone it out. We use Mutex with CriticalSectionRawMutex
/// to allow safe concurrent access from different tasks.
/// Using ConstStaticCell so we can use get() safely.
static NETWORK_STACK: StaticCell<Mutex<CriticalSectionRawMutex, Option<Stack<'static>>>> =
    StaticCell::new();

/// Store the initialized reference after init
static mut NETWORK_STACK_REF: Option<&'static Mutex<CriticalSectionRawMutex, Option<Stack<'static>>>> = None;

/// Atomic flag to track if the network stack has been initialized.
static NETWORK_STACK_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Signal that fires when network stack is ready.
static NETWORK_STACK_READY: Signal<ThreadModeRawMutex, ()> = Signal::new();

/// Get the network stack (blocks until ready). Returns a copy of the Stack (Stack is Copy).
pub async fn get_network_stack() -> Stack<'static> {
    log::info!("get_network_stack: Called, checking if initialized...");
    Timer::after(Duration::from_millis(10)).await;

    // Poll until StaticCell is initialized
    loop {
        // Check if initialized using atomic flag
        if NETWORK_STACK_INITIALIZED.load(Ordering::Acquire) {
            log::info!("get_network_stack: Stack is initialized, acquiring mutex...");
            Timer::after(Duration::from_millis(10)).await;

            // SAFETY: We checked the atomic flag, so we know NETWORK_STACK_REF has been set
            let stack_mutex = unsafe {
                NETWORK_STACK_REF.expect("NETWORK_STACK_REF should be set after initialization")
            };

            log::info!("get_network_stack: Got mutex reference, locking...");
            Timer::after(Duration::from_millis(10)).await;

            let stack_opt = stack_mutex.lock().await;

            log::info!("get_network_stack: Mutex locked, extracting stack...");
            Timer::after(Duration::from_millis(10)).await;

            let stack = stack_opt.expect("Network stack should be Some after initialization");

            log::info!("get_network_stack: Stack extracted successfully, returning");
            Timer::after(Duration::from_millis(10)).await;

            return stack;
        }
        // Small delay before checking again
        Timer::after(Duration::from_millis(10)).await;
    }
}

/// Helper to check if network is currently connected.
pub async fn is_network_ready(system_state: &'static SystemState) -> bool {
    matches!(
        system_state.get_network_state().await,
        NetworkState::Connected
    )
}

bind_interrupts!(struct Irqs {
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

/// Background task that initializes network without blocking main initialization.
#[embassy_executor::task]
pub async fn network_init_task(
    app_state: &'static SystemState,
    pin_23: embassy_rp::Peri<'static, PIN_23>,
    pin_24: embassy_rp::Peri<'static, PIN_24>,
    pin_25: embassy_rp::Peri<'static, PIN_25>,
    pin_29: embassy_rp::Peri<'static, PIN_29>,
    pio_1: embassy_rp::Peri<'static, PIO1>,
    dma_ch1: embassy_rp::Peri<'static, DMA_CH1>,
    spawner: Spawner,
) {
    log::info!("Network init task started (background)");
    app_state.set_network_state(NetworkState::Initializing).await;

    let stack = create_and_join_network(
        spawner, app_state, pin_23, pin_24, pin_25, pin_29, pio_1, dma_ch1,
    )
    .await;

    // Store the stack in the global mutex (Stack is Copy)
    // Initialize StaticCell once, then store the stack value
    let stack_ref = NETWORK_STACK.init(Mutex::new(Some(stack)));

    // Store the reference in the static mut
    unsafe {
        NETWORK_STACK_REF = Some(stack_ref);
    }

    // Set the atomic flag to indicate initialization is complete
    NETWORK_STACK_INITIALIZED.store(true, Ordering::Release);

    // Signal that stack is ready
    NETWORK_STACK_READY.signal(());
    log::info!("Network stack stored and ready");
    Timer::after(Duration::from_millis(50)).await;
}

/// Create and join the wifi network. Will wait until it has successfully joined.
pub async fn create_and_join_network(
    spawner: Spawner,
    app_state: &'static SystemState,
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
    let pio = Pio::new(pio_1, Irqs);

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

    let config = if USE_DHCP {
        log::info!("Network: Configuring DHCP");
        embassy_net::Config::dhcpv4(Default::default())
    } else {
        log::info!("Network: Configuring static IP is not currently supported");
        log::info!("Network: Configuring DHCP");
        embassy_net::Config::dhcpv4(Default::default())

        // let mut addresses: Vec<Ipv4Address, 3> = Vec::new();
        // addresses.insert(0, Ipv4Address::new(8, 8, 8, 8)).unwrap(); // Google DNS

        // let static_ip = Ipv4Address::new(IP_A1, IP_A2, IP_A3, IP_A4);
        // let gateway_ip = Ipv4Address::new(GW_A1, GW_A2, GW_A3, GW_A4);

        // log::info!("Network: Configuring static IP: {:?}", static_ip);
        // log::info!("Network: Gateway: {:?}", gateway_ip);
        // log::info!("Network: DNS: 8.8.8.8");

        // embassy_net::Config::ipv4_static(
        //     embassy_net::StaticConfigV4 {
        //         address: Ipv4Cidr::new(static_ip, PREFIX_LENGTH),
        //         dns_servers: addresses,
        //         gateway: Some(gateway_ip),
        //     },
        // )
    };

    // Generate random seed
    let seed = 0x0123_4567_89ab_cdef; // chosen by fair dice roll. guaranteed to be random.

    // Init network stack
    static RESOURCES: StaticCell<StackResources<13>> = StaticCell::new();
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

    log::info!("Network: Joining WiFi network: {}", WIFI_NETWORK);
    let mut attempts = 0;
    loop {
        attempts += 1;
        log::info!("Network: Join attempt {}", attempts);
        match control
            .join(WIFI_NETWORK, JoinOptions::new(WIFI_PASSWORD.as_bytes()))
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
