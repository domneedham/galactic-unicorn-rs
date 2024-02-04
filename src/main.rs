//! Galactic unicorn application.

#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]

mod app;
mod buttons;
mod config;
mod effects_app;
mod graphics;
mod mqtt;
mod time;
mod unicorn;

use embassy_executor::Spawner;
use embassy_net::Ipv4Address;
use embassy_net::Ipv4Cidr;
use embassy_net::Stack;
use embassy_net::StackResources;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::Input;
use embassy_rp::gpio::Level;
use embassy_rp::gpio::Output;
use embassy_rp::gpio::Pull;
use embassy_rp::peripherals::DMA_CH1;
use embassy_rp::peripherals::PIN_23;
use embassy_rp::peripherals::PIN_25;
use embassy_rp::peripherals::PIO1;
use embassy_rp::pio::InterruptHandler;
use embassy_rp::pio::Pio;
use embassy_time::Duration;

use cyw43_pio::PioSpi;

use defmt_rtt as _;
use embassy_time::Timer;
use galactic_unicorn_embassy::pins::UnicornButtonPins;
use heapless::Vec;
use panic_halt as _;
use static_cell::make_static;
use static_cell::StaticCell;

use galactic_unicorn_embassy::pins::UnicornDisplayPins;

use crate::buttons::brightness_down_task;
use crate::buttons::brightness_up_task;
use crate::buttons::button_a_task;
use crate::buttons::button_b_task;
use crate::config::*;
use crate::unicorn::display;
use crate::unicorn::display::DisplayTextMessage;

bind_interrupts!(struct Irqs {
    PIO1_IRQ_0 => InterruptHandler<PIO1>;
});

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

#[embassy_executor::task]
async fn net_task(stack: &'static Stack<cyw43::NetDriver<'static>>) -> ! {
    stack.run().await
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    let display_pins = UnicornDisplayPins {
        column_clock: p.PIN_13,
        column_data: p.PIN_14,
        column_latch: p.PIN_15,
        column_blank: p.PIN_16,
        row_bit_0: p.PIN_17,
        row_bit_1: p.PIN_18,
        row_bit_2: p.PIN_19,
        row_bit_3: p.PIN_20,
    };

    let button_pins = UnicornButtonPins {
        switch_a: Input::new(p.PIN_0, Pull::Up),
        switch_b: Input::new(p.PIN_1, Pull::Up),
        switch_c: Input::new(p.PIN_3, Pull::Up),
        switch_d: Input::new(p.PIN_6, Pull::Up),
        brightness_up: Input::new(p.PIN_21, Pull::Up),
        brightness_down: Input::new(p.PIN_26, Pull::Up),
        volume_up: Input::new(p.PIN_7, Pull::Up),
        volume_down: Input::new(p.PIN_8, Pull::Up),
        sleep: Input::new(p.PIN_27, Pull::Up),
    };

    unicorn::init(p.PIO0, p.DMA_CH0, display_pins, spawner).await;
    DisplayTextMessage::from_system("Initialising...", None, None)
        .send()
        .await;

    spawner
        .spawn(display::process_display_queue_task())
        .unwrap();

    spawner
        .spawn(brightness_up_task(button_pins.brightness_up))
        .unwrap();
    spawner
        .spawn(brightness_down_task(button_pins.brightness_down))
        .unwrap();
    spawner
        .spawn(display::process_brightness_buttons_task())
        .unwrap();

    spawner.spawn(button_a_task(button_pins.switch_a)).unwrap();
    spawner.spawn(button_b_task(button_pins.switch_b)).unwrap();

    let fw = include_bytes!("../cyw43-firmware/43439A0.bin");
    let clm = include_bytes!("../cyw43-firmware/43439A0_clm.bin");

    // wifi
    let pwr = Output::new(p.PIN_23, Level::Low);
    let cs = Output::new(p.PIN_25, Level::High);
    let mut pio = Pio::new(p.PIO1, Irqs);
    let spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        pio.irq0,
        cs,
        p.PIN_24,
        p.PIN_29,
        p.DMA_CH1,
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
                DisplayTextMessage::from_system("Joining wifi...", None, None)
                    .send()
                    .await;
                Timer::after(Duration::from_secs(2)).await;
            }
        }
    }

    // mqtt clients
    spawner
        .spawn(mqtt::clients::mqtt_send_client(stack))
        .unwrap();

    spawner
        .spawn(mqtt::clients::mqtt_receive_client(stack))
        .unwrap();

    let clock = make_static!(time::Clock::new());
    spawner.spawn(time::ntp_worker(stack, clock)).unwrap();

    let effects_app = make_static!(effects_app::EffectsApp::new());

    let app_controller = make_static!(app::AppController::new(clock, effects_app, spawner));
    app_controller.run().await;
}
