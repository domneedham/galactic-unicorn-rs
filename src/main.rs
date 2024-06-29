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
mod mqtt;
mod mqtt_app;
mod network;
mod system;
mod system_app;
mod time;
mod unicorn;

use embassy_executor::Spawner;
use embassy_rp::gpio::{Input, Pull};
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::pubsub::PubSubChannel;
use static_cell::make_static;

use defmt_rtt as _;
use panic_halt as _;

use galactic_unicorn_embassy::pins::UnicornButtonPins;
use galactic_unicorn_embassy::pins::UnicornDisplayPins;

use crate::buttons::{
    brightness_down_task, brightness_up_task, button_a_task, button_b_task, button_c_task,
};
use crate::mqtt::MqttReceiveMessage;
use crate::unicorn::display;

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

    let app_state = make_static!(system::AppState::new());
    let system_app = make_static!(system_app::SystemApp::new());
    let time = make_static!(time::Time::new());
    let clock_app = make_static!(clock_app::ClockApp::new(time));
    let effects_app = make_static!(effects_app::EffectsApp::new());
    let mqtt_app = make_static!(mqtt_app::MqttApp::new());

    let app_controller = app::AppController::new(
        system_app,
        clock_app,
        effects_app,
        mqtt_app,
        app_state,
        spawner,
    );

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

    let stack = network::create_and_join_network(
        spawner, app_state, p.PIN_23, p.PIN_24, p.PIN_25, p.PIN_29, p.PIO1, p.DMA_CH1,
    )
    .await;

    static MQTT_DISPLAY_CHANNEL: PubSubChannel<ThreadModeRawMutex, MqttReceiveMessage, 8, 1, 1> =
        PubSubChannel::new();

    static MQTT_APP_CHANNEL: PubSubChannel<ThreadModeRawMutex, MqttReceiveMessage, 8, 1, 1> =
        PubSubChannel::new();

    static MQTT_SYSTEM_CHANNEL: PubSubChannel<ThreadModeRawMutex, MqttReceiveMessage, 8, 1, 1> =
        PubSubChannel::new();

    spawner.spawn(time::ntp::ntp_worker(stack, time)).unwrap();

    // mqtt clients
    spawner
        .spawn(mqtt::clients::mqtt_send_client(stack))
        .unwrap();

    spawner
        .spawn(mqtt::clients::mqtt_receive_client(
            stack,
            MQTT_DISPLAY_CHANNEL.publisher().unwrap(),
            MQTT_APP_CHANNEL.publisher().unwrap(),
            MQTT_SYSTEM_CHANNEL.publisher().unwrap(),
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

    spawner
        .spawn(system::process_mqtt_messages_task(
            MQTT_SYSTEM_CHANNEL.subscriber().unwrap(),
        ))
        .unwrap();

    spawner
        .spawn(mqtt::homeassistant::hass_discovery_task(app_controller))
        .unwrap();

    app_controller.run_forever().await;
}
