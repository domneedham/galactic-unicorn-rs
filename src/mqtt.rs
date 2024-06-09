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

pub const BRIGHTNESS_TOPIC: &str = "display/brightness";
pub const COLOR_TOPIC: &str = "display/color";
pub const TEXT_TOPIC: &str = "app/text";
pub const CLOCK_APP_TOPIC: &str = "app/clock";

static SEND_CHANNEL: Channel<ThreadModeRawMutex, MqttMessage, 16> = Channel::new();

#[derive(Clone)]
pub struct MqttMessage<'a> {
    pub topic: &'a str,
    pub text: &'a str,
    pub qos: QualityOfService,
    pub retain: bool,
    pub include_base_topic: bool,
}

impl<'a> MqttMessage<'a> {
    pub fn new(topic: &'a str, text: &'a str, include_base_topic: bool) -> Self {
        Self {
            topic,
            text,
            qos: QualityOfService::QoS0,
            retain: false,
            include_base_topic,
        }
    }

    pub fn debug(text: &'a str) -> Self {
        Self {
            topic: "debug",
            text,
            qos: QualityOfService::QoS0,
            retain: false,
            include_base_topic: true,
        }
    }

    pub fn hass(topic: &'a str, text: &'a str) -> Self {
        Self {
            topic,
            text,
            qos: QualityOfService::QoS0,
            retain: false,
            include_base_topic: false,
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

    async fn process_mqtt_message(&self, _: MqttReceiveMessage) {}
}

pub mod clients {
    use core::fmt::Write;

    use cortex_m::singleton;
    use embassy_futures::select::{select, Either};
    use embassy_net::{tcp::TcpSocket, Stack};
    use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, pubsub::Publisher};
    use embassy_time::{Duration, Timer};
    use heapless::Vec;
    use rust_mqtt::{
        client::{client::MqttClient, client_config::ClientConfig},
        packet::v5::reason_codes::ReasonCode,
        utils::rng_generator::CountingRng,
    };

    use super::{
        MqttMessage, MqttReceiveMessage, BRIGHTNESS_TOPIC, CLOCK_APP_TOPIC, COLOR_TOPIC,
        SEND_CHANNEL, TEXT_TOPIC,
    };
    use crate::{
        unicorn::display::{send_brightness_state, DisplayTextMessage},
        BASE_MQTT_TOPIC,
    };

    pub async fn send_home_assistant_discovery() {
        let topic = "homeassistant/select/galactic_unicorn/config";
        let payload = r#"{
            "name": null,
            "~": "galactic_unicorn/app/clock",
            "stat_t": "~/state",
            "cmd_t": "~",
            "uniq_id": "ga_clock_01",
            "dev": {
                "ids": "ga_01",
                "name": "Galactic Unicorn"
            },
            "options": ["rainbow", "color"]
        }"#;
        MqttMessage::hass(topic, payload).send().await;

        let topic = "homeassistant/number/galactic_unicorn/config";
        let payload = r#"{
            "name": "Brightness",
            "~": "galactic_unicorn/display/brightness",
            "stat_t": "~/state",
            "cmd_t": "~",
            "uniq_id": "ga_brightness_01",
            "dev": {
                "ids": "ga_01",
                "name": "Galactic Unicorn"
            },
            "min": 1,
            "max": 255,
        }"#;
        MqttMessage::hass(topic, payload).send().await;

        send_brightness_state().await;
    }

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
        config.max_packet_size = 600;

        let mut recv_buffer = [0; 512];
        let mut write_buffer = [0; 512];

        let mut client: MqttClient<'_, TcpSocket<'_>, 5, CountingRng> = MqttClient::<_, 5, _>::new(
            socket,
            &mut write_buffer,
            512,
            &mut recv_buffer,
            512,
            config,
        );

        client.connect_to_broker().await.unwrap();

        send_home_assistant_discovery().await;

        loop {
            match select(SEND_CHANNEL.receive(), Timer::after_secs(5)).await {
                Either::First(message) => {
                    let mut topic = heapless::String::<256>::new();
                    if message.include_base_topic {
                        _ = write!(topic, "{BASE_MQTT_TOPIC}");
                    }
                    let message_topic = message.topic;
                    _ = write!(topic, "{message_topic}");

                    match client
                        .send_message(
                            topic.as_str(),
                            message.text.as_bytes(),
                            message.qos,
                            message.retain,
                        )
                        .await
                    {
                        Ok(_) => {}
                        Err(x) => {
                            MqttMessage::debug(get_reason_code(x)).send().await;
                        }
                    }
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
        display_publisher: Publisher<'static, ThreadModeRawMutex, MqttReceiveMessage, 16, 1, 1>,
        app_publisher: Publisher<'static, ThreadModeRawMutex, MqttReceiveMessage, 16, 1, 1>,
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

        let mut brightness_topic = heapless::String::<64>::new();
        write!(brightness_topic, "{BASE_MQTT_TOPIC}{BRIGHTNESS_TOPIC}").unwrap();

        let mut color_topic = heapless::String::<64>::new();
        write!(color_topic, "{BASE_MQTT_TOPIC}{COLOR_TOPIC}").unwrap();

        let mut text_topic = heapless::String::<64>::new();
        write!(text_topic, "{BASE_MQTT_TOPIC}{TEXT_TOPIC}").unwrap();

        let mut clock_app_topic = heapless::String::<64>::new();
        write!(clock_app_topic, "{BASE_MQTT_TOPIC}{CLOCK_APP_TOPIC}").unwrap();

        let topics: Vec<&str, 4> = Vec::from_slice(&[
            brightness_topic.as_str(),
            color_topic.as_str(),
            text_topic.as_str(),
            clock_app_topic.as_str(),
        ])
        .unwrap();

        match client.subscribe_to_topics(&topics).await {
            Ok(_) => MqttMessage::debug("Subscribed to topics").send().await,
            Err(code) => send_reason_code(code).await,
        };

        loop {
            match select(client.receive_message(), Timer::after_secs(5)).await {
                Either::First(received_message) => match received_message {
                    Ok(mqtt_message) => {
                        let message = MqttReceiveMessage::new(mqtt_message.0, mqtt_message.1);

                        if mqtt_message.0.contains("display") {
                            display_publisher.publish(message).await;
                        } else if mqtt_message.0.contains("app") {
                            app_publisher.publish(message).await;
                        }
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
