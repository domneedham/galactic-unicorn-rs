use core::{
    fmt::Write,
    sync::atomic::{AtomicBool, Ordering},
};

use embassy_sync::{
    blocking_mutex::raw::ThreadModeRawMutex, channel::Channel, mutex::Mutex, signal::Signal,
};
use embassy_time::Duration;
use heapless::String;
use rust_mqtt::packet::v5::publish_packet::QualityOfService;

use crate::{app::UnicornApp, buttons::ButtonPress, unicorn::display::DisplayTextMessage};

static SEND_CHANNEL: Channel<ThreadModeRawMutex, MqttMessage, 16> = Channel::new();

#[derive(Clone)]
pub struct DisplayTopics {
    pub display_topic: String<64>,
    pub display_interrupt_topic: String<64>,
    pub brightness_topic: String<64>,
    pub color_topic: String<64>,
}

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

#[derive(Clone)]
pub struct MqttReceiveMessage {
    pub topic: String<64>,
    pub body: String<64>,
}

impl MqttReceiveMessage {
    pub fn new(topic: &str, body_bytes: &[u8]) -> Self {
        let mut h_topic = heapless::String::<64>::new();
        write!(h_topic, "{topic}").unwrap();

        let body = core::str::from_utf8(body_bytes).unwrap();
        let mut h_body = heapless::String::<64>::new();
        write!(h_body, "{body}").unwrap();

        Self {
            topic: h_topic,
            body: h_body,
        }
    }
}

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
}

pub mod clients {
    use core::fmt::Write;

    use cortex_m::singleton;
    use embassy_futures::select::{select, Either};
    use embassy_net::{tcp::TcpSocket, Stack};
    use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, pubsub::Publisher};
    use embassy_time::{Duration, Timer};
    use rust_mqtt::{
        client::{client::MqttClient, client_config::ClientConfig},
        packet::v5::reason_codes::ReasonCode,
        utils::rng_generator::CountingRng,
    };

    use super::{DisplayTopics, MqttMessage, MqttReceiveMessage, SEND_CHANNEL};
    use crate::{unicorn::display::DisplayTextMessage, BASE_MQTT_TOPIC};

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
            match select(SEND_CHANNEL.receive(), Timer::after_secs(5)).await {
                Either::First(message) => {
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
                Either::Second(_) => match client.send_ping().await {
                    Ok(_) => {}
                    Err(code) => show_reason_code(code).await,
                },
            }
        }
    }

    #[embassy_executor::task]
    pub async fn mqtt_receive_client(
        stack: &'static Stack<cyw43::NetDriver<'static>>,
        display_topics: DisplayTopics,
        publisher: Publisher<'static, ThreadModeRawMutex, MqttReceiveMessage, 16, 1, 1>,
    ) {
        let tx_buffer = singleton!(: [u8; 4096] = [0; 4096]).unwrap();
        let rx_buffer = singleton!(: [u8; 4096] = [0; 4096]).unwrap();

        let mut socket = TcpSocket::new(stack, rx_buffer, tx_buffer);
        socket.set_timeout(Some(Duration::from_secs(30)));
        let host_addr = embassy_net::Ipv4Address::new(192, 168, 1, 20);
        match socket.connect((host_addr, 1883)).await {
            Ok(_) => {}
            Err(_) => loop {
                DisplayTextMessage::from_mqtt("Failed to connect to socket", None, None)
                    .send()
                    .await;
                Timer::after_secs(1).await;
            },
        };

        let mut config = ClientConfig::new(
            rust_mqtt::client::client_config::MqttVersion::MQTTv5,
            CountingRng(50000),
        );
        config.add_max_subscribe_qos(rust_mqtt::packet::v5::publish_packet::QualityOfService::QoS1);
        config.add_client_id("receiver");
        // config.add_username(USERNAME);
        // config.add_password(PASSWORD);
        config.max_packet_size = 100;
        let recv_buffer = singleton!(: [u8; 500] = [0; 500]).unwrap();
        let write_buffer = singleton!(: [u8; 500] = [0; 500]).unwrap();

        let mut client: MqttClient<'_, TcpSocket<'_>, 5, CountingRng> =
            MqttClient::<_, 5, _>::new(socket, write_buffer, 500, recv_buffer, 500, config);

        match client.connect_to_broker().await {
            Ok(_) => {
                MqttMessage::debug("Connected to receiver broker")
                    .send()
                    .await
            }
            Err(code) => send_reason_code(code).await,
        };

        match client
            .subscribe_to_topic(&display_topics.display_topic)
            .await
        {
            Ok(_) => {
                MqttMessage::debug("Subscribed to display topic")
                    .send()
                    .await
            }
            Err(code) => send_reason_code(code).await,
        }

        match client
            .subscribe_to_topic(&display_topics.display_interrupt_topic)
            .await
        {
            Ok(_) => {
                MqttMessage::debug("Subscribed to display interrupt topic")
                    .send()
                    .await
            }
            Err(code) => send_reason_code(code).await,
        }

        match client
            .subscribe_to_topic(&display_topics.brightness_topic)
            .await
        {
            Ok(_) => {
                MqttMessage::debug("Subscribed to brightness topic")
                    .send()
                    .await
            }
            Err(code) => send_reason_code(code).await,
        }

        match client.subscribe_to_topic(&display_topics.color_topic).await {
            Ok(_) => MqttMessage::debug("Subscribed to color topic").send().await,
            Err(code) => send_reason_code(code).await,
        }

        loop {
            match select(client.receive_message(), Timer::after_secs(5)).await {
                Either::First(received_message) => match received_message {
                    Ok(message) => {
                        MqttMessage::debug("Received mqtt message").send().await;
                        let message = MqttReceiveMessage::new(message.0, message.1);
                        publisher.publish(message).await;
                    }
                    Err(code) => {
                        show_reason_code(code).await;
                    }
                },
                Either::Second(_) => match client.send_ping().await {
                    Ok(_) => {}
                    Err(code) => show_reason_code(code).await,
                },
            }
        }
    }

    fn get_reason_code(code: ReasonCode) -> &'static str {
        match code {
            ReasonCode::Success => "Success",
            ReasonCode::GrantedQoS1 => "GrantedQoS1",
            ReasonCode::GrantedQoS2 => "GrantedQoS2",
            ReasonCode::DisconnectWithWillMessage => "DisconnectWithWillMessage",
            ReasonCode::NoMatchingSubscribers => "NoMatchingSubscribers",
            ReasonCode::NoSubscriptionExisted => "NoSubscriptionExisted",
            ReasonCode::ContinueAuth => "ContinueAuth",
            ReasonCode::ReAuthenticate => "ReAuthenticate",
            ReasonCode::UnspecifiedError => "UnspecifiedError",
            ReasonCode::MalformedPacket => "MalformedPacket",
            ReasonCode::ProtocolError => "ProtocolError",
            ReasonCode::ImplementationSpecificError => "ImplementationSpecificError",
            ReasonCode::UnsupportedProtocolVersion => "UnsupportedProtocol",
            ReasonCode::ClientIdNotValid => "ClientIdNotValid",
            ReasonCode::BadUserNameOrPassword => "BadUserNameOrPassword",
            ReasonCode::NotAuthorized => "NotAuthorized",
            ReasonCode::ServerUnavailable => "ServerUnavailable",
            ReasonCode::ServerBusy => "ServerBusy",
            ReasonCode::Banned => "Banned",
            ReasonCode::ServerShuttingDown => "ServerShuttingDown",
            ReasonCode::BadAuthMethod => "BadAuthMethod",
            ReasonCode::KeepAliveTimeout => "KeepAliveTimeout",
            ReasonCode::SessionTakeOver => "SessionTakeOver",
            ReasonCode::TopicFilterInvalid => "TopicFilterInvalid",
            ReasonCode::TopicNameInvalid => "TopicNameInvalid",
            ReasonCode::PacketIdentifierInUse => "PacketIdentifierInUse",
            ReasonCode::PacketIdentifierNotFound => "PacketIdentifierNotFound",
            ReasonCode::ReceiveMaximumExceeded => "ReceiveMaximumExceeded",
            ReasonCode::TopicAliasInvalid => "TopicAliasInvalid",
            ReasonCode::PacketTooLarge => "PacketTooLarge",
            ReasonCode::MessageRateTooHigh => "MessageRateTooHigh",
            ReasonCode::QuotaExceeded => "QuotaExceeded",
            ReasonCode::AdministrativeAction => "AdministrativeAction",
            ReasonCode::PayloadFormatInvalid => "PayloadFormatInvalid",
            ReasonCode::RetainNotSupported => "RetainNotSupported",
            ReasonCode::QoSNotSupported => "QoSNotSupported",
            ReasonCode::UseAnotherServer => "UseAnotherServer",
            ReasonCode::ServerMoved => "ServerMoved",
            ReasonCode::SharedSubscriptionNotSupported => "SharedSubscriptionNotSupported",
            ReasonCode::ConnectionRateExceeded => "ConnectionRateExceeded",
            ReasonCode::MaximumConnectTime => "MaximumConnectTime",
            ReasonCode::SubscriptionIdentifiersNotSupported => {
                "SubscriptionIdentifiersNotSupported"
            }
            ReasonCode::WildcardSubscriptionNotSupported => "WildcardSubscriptionNotSupported",
            ReasonCode::TimerNotSupported => "TimerNotSupported",
            ReasonCode::BuffError => "BuffError",
            ReasonCode::NetworkError => "NetworkError",
        }
    }

    async fn send_reason_code(code: ReasonCode) {
        let message = get_reason_code(code);
        MqttMessage::debug(message).send().await;
    }

    async fn show_reason_code(code: ReasonCode) {
        let text = get_reason_code(code);
        DisplayTextMessage::from_system(text, None, None)
            .send_and_replace_queue()
            .await;
    }
}
