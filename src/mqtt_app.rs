use core::sync::atomic::{AtomicBool, Ordering};

use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, mutex::Mutex, signal::Signal};
use embassy_time::Duration;
use heapless::String;

use crate::{
    app::UnicornApp, buttons::ButtonPress, mqtt::MqttReceiveMessage,
    unicorn::display::DisplayTextMessage,
};

/// MQTT app. Will display the latest MQTT message.
pub struct MqttApp {
    /// The last message received.
    pub last_message: Mutex<ThreadModeRawMutex, Option<String<64>>>,

    /// Signal to update the message displayed.
    pub update_message: Signal<ThreadModeRawMutex, bool>,

    /// Track if the app is active or not.
    pub is_active: AtomicBool,
}

impl MqttApp {
    /// Create the static ref to MQTT app.
    /// Must only be called once or will panic.
    pub fn new() -> &'static Self {
        make_static!(Self {
            last_message: Mutex::new(None),
            update_message: Signal::new(),
            is_active: AtomicBool::new(false),
        })
    }

    /// Set the last message received from MQTT.
    pub async fn set_last_message(&self, message: String<64>) {
        self.last_message.lock().await.replace(message);
        self.update_message.signal(true);
    }
}

impl UnicornApp for MqttApp {
    async fn display(&self) {
        loop {
            match self.last_message.lock().await.as_ref() {
                Some(val) => {
                    DisplayTextMessage::from_app(&val, None, None, Some(Duration::from_secs(1)))
                        .send_and_replace_queue()
                        .await
                }
                None => {
                    DisplayTextMessage::from_app(
                        "No message!",
                        None,
                        None,
                        Some(Duration::from_secs(1)),
                    )
                    .send_and_replace_queue()
                    .await
                }
            };

            self.update_message.wait().await;
        }
    }

    async fn start(&self) {
        self.is_active.store(true, Ordering::Relaxed);
    }

    async fn stop(&self) {
        self.is_active.store(false, Ordering::Relaxed);
    }

    async fn button_press(&self, _: ButtonPress) {}

    async fn process_mqtt_message(&self, _: MqttReceiveMessage) {}

    async fn send_mqtt_state(&self) {}
}
