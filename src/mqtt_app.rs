use core::sync::atomic::{AtomicBool, Ordering};

use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, mutex::Mutex, signal::Signal};
use embassy_time::Duration;
use heapless::String;

use crate::{
    app::UnicornApp, buttons::ButtonPress, mqtt::MqttReceiveMessage,
    unicorn::display::DisplayTextMessage,
};

pub struct MqttApp {
    pub last_message: Mutex<ThreadModeRawMutex, Option<String<64>>>,
    pub update_message: Signal<ThreadModeRawMutex, bool>,
    pub is_active: AtomicBool,
}

impl MqttApp {
    pub fn new() -> Self {
        Self {
            last_message: Mutex::new(None),
            update_message: Signal::new(),
            is_active: AtomicBool::new(false),
        }
    }

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
