use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, pubsub::Subscriber};

use crate::{
    mqtt::{topics::NTP_SYNC_TOPIC, MqttReceiveMessage},
    time::ntp::SYNC_SIGNAL,
};

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
