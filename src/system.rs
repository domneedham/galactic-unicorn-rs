use embassy_sync::{
    blocking_mutex::raw::ThreadModeRawMutex, mutex::Mutex, pubsub::Subscriber, signal::Signal,
};

use crate::{
    mqtt::{topics::NTP_SYNC_TOPIC, MqttReceiveMessage},
    network::NetworkState,
    time::ntp::SYNC_SIGNAL,
};

pub static STATE_CHANGED: Signal<ThreadModeRawMutex, StateUpdates> = Signal::new();

pub enum StateUpdates {
    Network,
}

pub struct AppState {
    network_state: Mutex<ThreadModeRawMutex, NetworkState>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            network_state: Mutex::new(NetworkState::NotInitialised),
        }
    }

    pub async fn get_network_state(&'static self) -> NetworkState {
        *self.network_state.lock().await
    }

    pub async fn set_network_state(&'static self, state: NetworkState) {
        *self.network_state.lock().await = state;
        STATE_CHANGED.signal(StateUpdates::Network);
    }
}

#[embassy_executor::task]
pub async fn process_mqtt_messages_task(
    mut subscriber: Subscriber<'static, ThreadModeRawMutex, MqttReceiveMessage, 8, 1, 1>,
) {
    loop {
        let message = subscriber.next_message_pure().await;

        if message.topic == NTP_SYNC_TOPIC {
            SYNC_SIGNAL.signal(true);
        }
    }
}
