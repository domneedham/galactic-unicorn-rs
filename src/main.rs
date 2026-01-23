//! Galactic unicorn application.

#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]
#![feature(impl_trait_in_assoc_type)]
#![feature(ip_as_octets)]

mod app;
mod buttons;
mod clock_app;
mod config;
mod display;
mod draw_app;
mod effects_app;
mod fonts;
mod mqtt;
mod mqtt_app;
mod network;
mod system;
mod system_app;
mod time;

use buttons::ButtonPress;
use display::{Display, DisplayState};
use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::pubsub::PubSubChannel;
use galactic_unicorn_embassy::buttons::UnicornButtons;

use defmt_rtt as _;
use embassy_time::Duration;
use embassy_time::Timer;
use galactic_unicorn_embassy::pins::UnicornSensorPins;
use panic_halt as _;

use galactic_unicorn_embassy::pins::UnicornButtonPins;
use galactic_unicorn_embassy::pins::UnicornDisplayPins;

use crate::buttons::button_d_task;
use crate::buttons::{
    brightness_down_task, brightness_up_task, button_a_task, button_b_task, button_c_task,
};
use crate::mqtt::MqttReceiveMessage;

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

    let sensor_pins = UnicornSensorPins {
        light_sensor: p.PIN_28,
    };

    let button_pins = UnicornButtonPins {
        switch_a: p.PIN_0,
        switch_b: p.PIN_1,
        switch_c: p.PIN_3,
        switch_d: p.PIN_6,
        brightness_up: p.PIN_21,
        brightness_down: p.PIN_26,
        volume_up: p.PIN_7,
        volume_down: p.PIN_8,
        sleep: p.PIN_27,
    };

    let display = Display::new(
        p.PIO0,
        p.DMA_CH0,
        p.ADC,
        p.USB,
        display_pins,
        sensor_pins,
    );

    let display_state = DisplayState::new();
    let system_state = system::SystemState::new();
    let system_app = system_app::SystemApp::new(display_state);
    let time = time::Time::new();
    let clock_app = clock_app::ClockAppState::new(display_state, time);
    let effects_app = effects_app::EffectsApp::new();
    let mqtt_app = mqtt_app::MqttApp::new(display_state);
    let draw_app = draw_app::DrawApp::new(system_state, display_state);

    // Button channel: 4 capacity, 1 subscriber (AppController), 9 publishers (button tasks)
    static BUTTON_CHANNEL: PubSubChannel<ThreadModeRawMutex, (UnicornButtons, ButtonPress), 4, 1, 9> =
        PubSubChannel::new();

    let app_controller = app::AppController::new(
        display,
        display_state,
        system_state,
        system_app,
        clock_app,
        effects_app,
        mqtt_app,
        draw_app,
        BUTTON_CHANNEL.subscriber().unwrap(),
        MQTT_APP_CHANNEL.subscriber().unwrap(),
        spawner,
    );

    spawner
        .spawn(brightness_up_task(
            button_pins.brightness_up,
            BUTTON_CHANNEL.publisher().unwrap(),
        ))
        .unwrap();
    spawner
        .spawn(brightness_down_task(
            button_pins.brightness_down,
            BUTTON_CHANNEL.publisher().unwrap(),
        ))
        .unwrap();
    spawner
        .spawn(button_a_task(
            button_pins.switch_a,
            BUTTON_CHANNEL.publisher().unwrap(),
        ))
        .unwrap();
    spawner
        .spawn(button_b_task(
            button_pins.switch_b,
            BUTTON_CHANNEL.publisher().unwrap(),
        ))
        .unwrap();
    spawner
        .spawn(button_c_task(
            button_pins.switch_c,
            BUTTON_CHANNEL.publisher().unwrap(),
        ))
        .unwrap();
    spawner
        .spawn(button_d_task(
            button_pins.switch_d,
            BUTTON_CHANNEL.publisher().unwrap(),
        ))
        .unwrap();

    Timer::after(Duration::from_millis(2000)).await;

    log::info!("Starting background services...");

    // Spawn network init in background (doesn't block)
    spawner
        .spawn(network::network_init_task(
            system_state, p.PIN_23, p.PIN_24, p.PIN_25, p.PIN_29, p.PIO1, p.DMA_CH1, spawner,
        ))
        .unwrap();

    static MQTT_DISPLAY_CHANNEL: PubSubChannel<ThreadModeRawMutex, MqttReceiveMessage, 8, 1, 1> =
        PubSubChannel::new();

    static MQTT_APP_CHANNEL: PubSubChannel<ThreadModeRawMutex, MqttReceiveMessage, 8, 1, 1> =
        PubSubChannel::new();

    static MQTT_SYSTEM_CHANNEL: PubSubChannel<ThreadModeRawMutex, MqttReceiveMessage, 8, 1, 1> =
        PubSubChannel::new();

    // Spawn display state management tasks
    spawner
        .spawn(display::auto_brightness_task(display, display_state))
        .unwrap();

    spawner
        .spawn(display::state_to_mqtt_broadcast_task(display_state))
        .unwrap();

    // These tasks spawn immediately but wait internally for network stack
    spawner
        .spawn(time::ntp::ntp_worker(time, system_state))
        .unwrap();

    spawner
        .spawn(mqtt::clients::mqtt_send_client(system_state))
        .unwrap();

    spawner
        .spawn(mqtt::clients::mqtt_receive_client(
            system_state,
            MQTT_DISPLAY_CHANNEL.publisher().unwrap(),
            MQTT_APP_CHANNEL.publisher().unwrap(),
            MQTT_SYSTEM_CHANNEL.publisher().unwrap(),
        ))
        .unwrap();

    spawner
        .spawn(display::process_mqtt_messages_task(
            display_state,
            MQTT_DISPLAY_CHANNEL.subscriber().unwrap(),
        ))
        .unwrap();

    // spawner
    //     .spawn(app::process_mqtt_messages_task(
    //         app_controller,
    //         MQTT_APP_CHANNEL.subscriber().unwrap(),
    //     ))
    //     .unwrap();

    // spawner
    //     .spawn(system::process_mqtt_messages_task(
    //         MQTT_SYSTEM_CHANNEL.subscriber().unwrap(),
    //     ))
    //     .unwrap();

    spawner
        .spawn(mqtt::homeassistant::hass_discovery_task(
            display,
            app_controller,
        ))
        .unwrap();

    app_controller.run().await;
}
