use embassy_sync::{
    blocking_mutex::raw::ThreadModeRawMutex, mutex::Mutex, pubsub::Subscriber, signal::Signal,
};
use static_cell::make_static;

use crate::{
    mqtt::{topics::NTP_SYNC_TOPIC, MqttReceiveMessage},
    network::NetworkState,
    time::ntp::SYNC_SIGNAL,
};

/// State changed signal for when any app state changes.
pub static STATE_CHANGED: Signal<ThreadModeRawMutex, StateUpdates> = Signal::new();

/// Possible states than can update.
pub enum StateUpdates {
    Network,
}

/// App state. Encapsulates all needed states in the system.
pub struct SystemState {
    network_state: Mutex<ThreadModeRawMutex, NetworkState>,
}

impl SystemState {
    /// Create the static ref to system state.
    /// Must only be called once or will panic.
    pub fn new() -> &'static Self {
        make_static!(Self {
            network_state: Mutex::new(NetworkState::NotInitialised),
        })
    }

    /// Get the current network state.
    pub async fn get_network_state(&'static self) -> NetworkState {
        *self.network_state.lock().await
    }

    /// Set the network state and update the `STATE_CHANGED` signal.
    pub async fn set_network_state(&'static self, state: NetworkState) {
        *self.network_state.lock().await = state;
        STATE_CHANGED.signal(StateUpdates::Network);
    }
}

/// Process MQTT messages that apply to the system.
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
