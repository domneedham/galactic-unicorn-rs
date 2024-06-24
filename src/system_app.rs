use embassy_time::Timer;

use crate::{
    app::UnicornApp, buttons::ButtonPress, mqtt::MqttReceiveMessage, network::NetworkState,
    system::AppState, unicorn::display::DisplayTextMessage,
};

pub struct SystemApp {
    app_state: &'static AppState,
}

impl SystemApp {
    pub fn new(app_state: &'static AppState) -> Self {
        Self { app_state }
    }

    pub async fn prepare_for_app_change(&self) {
        Timer::after_secs(2).await;
    }
}

impl UnicornApp for SystemApp {
    async fn display(&self) {
        loop {
            match self.app_state.get_network_state().await {
                NetworkState::NotInitialised => {
                    DisplayTextMessage::from_app("Booting", None, None, None)
                        .send_and_replace_queue()
                        .await;
                }
                NetworkState::Connected => {
                    DisplayTextMessage::from_app("Connected", None, None, None)
                        .send_and_replace_queue()
                        .await;
                }
                NetworkState::Error => {
                    DisplayTextMessage::from_app("Net Error", None, None, None)
                        .send_and_replace_queue()
                        .await;
                }
            }

            Timer::after_secs(2).await;
        }
    }

    async fn start(&self) {}

    async fn stop(&self) {}

    async fn button_press(&self, _: ButtonPress) {}

    async fn process_mqtt_message(&self, _: MqttReceiveMessage) {}

    async fn send_mqtt_state(&self) {}
}
