//! Galactic unicorn application.

#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]

mod app;
mod buttons;
mod clock_app;
mod config;
mod effects_app;
mod fonts;
mod graphics;
mod mqtt;
mod time;
mod unicorn;

use cyw43_pio::PioSpi;
use embassy_executor::Spawner;
use embassy_net::{Ipv4Address, Ipv4Cidr, Stack, StackResources};
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_rp::peripherals::{DMA_CH1, PIN_23, PIN_25, PIO1};
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::pubsub::PubSubChannel;
use embassy_time::{Duration, Timer};
use heapless::Vec;
use static_cell::make_static;
use static_cell::StaticCell;

use defmt_rtt as _;
use panic_halt as _;

use galactic_unicorn_embassy::pins::UnicornButtonPins;
use galactic_unicorn_embassy::pins::UnicornDisplayPins;

use crate::buttons::{
    brightness_down_task, brightness_up_task, button_a_task, button_b_task, button_c_task,
};
use crate::config::*;
use crate::mqtt::MqttReceiveMessage;
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

    unicorn::init(p.PIO0, p.DMA_CH0, display_pins).await;
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
    spawner.spawn(button_c_task(button_pins.switch_c)).unwrap();

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
                    .send_and_replace_queue()
                    .await;
                Timer::after(Duration::from_secs(2)).await;
            }
        }
    }

    let time = make_static!(time::Time::new());
    spawner.spawn(time::ntp::ntp_worker(stack, time)).unwrap();

    let clock_app = make_static!(clock_app::ClockApp::new(time));
    let effects_app = make_static!(effects_app::EffectsApp::new());
    let mqtt_app = make_static!(mqtt::MqttApp::new());

    let app_controller = make_static!(app::AppController::new(
        clock_app,
        effects_app,
        mqtt_app,
        spawner
    ));

    static MQTT_DISPLAY_CHANNEL: PubSubChannel<ThreadModeRawMutex, MqttReceiveMessage, 16, 1, 1> =
        PubSubChannel::new();

    static MQTT_APP_CHANNEL: PubSubChannel<ThreadModeRawMutex, MqttReceiveMessage, 16, 1, 1> =
        PubSubChannel::new();

    // mqtt clients
    spawner
        .spawn(mqtt::clients::mqtt_send_client(stack))
        .unwrap();

    spawner
        .spawn(mqtt::clients::mqtt_receive_client(
            stack,
            MQTT_DISPLAY_CHANNEL.publisher().unwrap(),
            MQTT_APP_CHANNEL.publisher().unwrap(),
        ))
        .unwrap();

    spawner
        .spawn(display::process_mqtt_messages_task(
            MQTT_DISPLAY_CHANNEL.subscriber().unwrap(),
        ))
        .unwrap();

    spawner
        .spawn(app::process_mqtt_messages_task(
            app_controller,
            MQTT_APP_CHANNEL.subscriber().unwrap(),
        ))
        .unwrap();

    app_controller.run().await;
}
