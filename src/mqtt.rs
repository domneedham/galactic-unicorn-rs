use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, channel::Channel};
use rust_mqtt::packet::v5::publish_packet::QualityOfService;

static SEND_CHANNEL: Channel<ThreadModeRawMutex, MqttMessage, 16> = Channel::new();

pub struct MqttMessage<'a> {
    pub topic: &'a str,
    pub text: &'a str,
    pub qos: QualityOfService,
    pub retain: bool,
}

impl<'a> MqttMessage<'a> {
    pub fn debug(text: &'a str) -> Self {
        Self {
            topic: "debug",
            text,
            qos: QualityOfService::QoS0,
            retain: false,
        }
    }
}

impl MqttMessage<'static> {
    pub async fn send(self) {
        SEND_CHANNEL.send(self).await;
    }
}

pub mod clients {
    use core::fmt::Write;

    use cortex_m::singleton;
    use embassy_net::{tcp::TcpSocket, Stack};
    use embassy_time::Duration;
    use rust_mqtt::{
        client::{client::MqttClient, client_config::ClientConfig},
        utils::rng_generator::CountingRng,
    };

    use super::SEND_CHANNEL;
    use crate::BASE_MQTT_TOPIC;

    #[embassy_executor::task]
    pub async fn mqtt_send_client(stack: &'static Stack<cyw43::NetDriver<'static>>) {
        let tx_buffer = singleton!(: [u8; 4096] = [0; 4096]).unwrap();
        let rx_buffer = singleton!(: [u8; 4096] = [0; 4096]).unwrap();

        let mut socket = TcpSocket::new(stack, rx_buffer, tx_buffer);
        socket.set_timeout(Some(Duration::from_secs(10)));
        let host_addr = embassy_net::Ipv4Address::new(192, 168, 1, 20);
        socket.connect((host_addr, 1883)).await.unwrap();

        let mut config = ClientConfig::new(
            rust_mqtt::client::client_config::MqttVersion::MQTTv5,
            CountingRng(20000),
        );
        config.add_max_subscribe_qos(rust_mqtt::packet::v5::publish_packet::QualityOfService::QoS1);
        config.add_client_id("client");
        // config.add_username(USERNAME);
        // config.add_password(PASSWORD);
        config.max_packet_size = 100;

        let mut recv_buffer = [0; 80];
        let mut write_buffer = [0; 80];

        let mut client: MqttClient<'_, TcpSocket<'_>, 5, CountingRng> =
            MqttClient::<_, 5, _>::new(socket, &mut write_buffer, 80, &mut recv_buffer, 80, config);

        client.connect_to_broker().await.unwrap();

        loop {
            let message = SEND_CHANNEL.receive().await;

            let mut topic = heapless::String::<256>::new();
            _ = write!(topic, "{BASE_MQTT_TOPIC}");
            let message_topic = message.topic;
            _ = write!(topic, "{message_topic}");

            let _ = client
                .send_message(
                    topic.as_str(),
                    message.text.as_bytes(),
                    message.qos,
                    message.retain,
                )
                .await;
        }
    }
}
