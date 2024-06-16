use core::fmt::Write;

use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, channel::Channel};
use heapless::String;
use rust_mqtt::packet::v5::publish_packet::QualityOfService;

pub const BRIGHTNESS_TOPIC: &str = "display/brightness";
pub const COLOR_TOPIC: &str = "display/color";
pub const RGB_TOPIC: &str = "display/rgb";
pub const TEXT_TOPIC: &str = "app/text";
pub const APP_TOPIC: &str = "app";
pub const CLOCK_APP_TOPIC: &str = "app/clock";

static SEND_CHANNEL: Channel<ThreadModeRawMutex, &'static mut MqttMessage, 4> = Channel::new();

static mut MESSAGE_POOL: [MqttMessage; 4] = [
    MqttMessage::new(),
    MqttMessage::new(),
    MqttMessage::new(),
    MqttMessage::new(),
];

pub struct MqttMessage {
    topic: &'static str,
    text: String<512>,
    qos: QualityOfService,
    retain: bool,
    include_base_topic: bool,
    in_use: bool,
}

impl MqttMessage {
    const fn new() -> Self {
        MqttMessage {
            topic: "",
            text: String::new(),
            qos: QualityOfService::QoS0,
            retain: false,
            include_base_topic: false,
            in_use: false,
        }
    }

    fn reuse(
        &mut self,
        topic: &'static str,
        content: &str,
        qos: QualityOfService,
        retain: bool,
        include_base_topic: bool,
    ) {
        self.in_use = true;

        self.topic = topic;
        self.text.clear();
        self.text.push_str(content).unwrap();
        self.qos = qos;
        self.retain = retain;
        self.include_base_topic = include_base_topic;
    }

    fn release(&mut self) {
        self.in_use = false;
    }

    pub async fn enqueue_state(topic: &'static str, content: &str) {
        Self::enqueue(topic, content, QualityOfService::QoS0, false, true).await;
    }

    pub async fn enqueue_debug(content: &str) {
        Self::enqueue("debug", content, QualityOfService::QoS0, false, true).await;
    }

    pub async fn enqueue(
        topic: &'static str,
        content: &str,
        qos: QualityOfService,
        retain: bool,
        include_base_topic: bool,
    ) {
        for message in unsafe { &mut MESSAGE_POOL } {
            if !message.in_use {
                message.reuse(topic, content, qos, retain, include_base_topic);

                SEND_CHANNEL.send(message).await;
                break;
            }
        }
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
        homeassistant, MqttMessage, MqttReceiveMessage, APP_TOPIC, BRIGHTNESS_TOPIC,
        CLOCK_APP_TOPIC, COLOR_TOPIC, RGB_TOPIC, SEND_CHANNEL, TEXT_TOPIC,
    };
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
                        Ok(_) => {
                            message.release();
                        }
                        Err(x) => {
                            message.release();
                            MqttMessage::enqueue_debug(get_reason_code(x)).await;
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
                MqttMessage::enqueue_debug("Connected to receiver broker").await;
            }
            Err(code) => send_reason_code(code).await,
        };

        let mut brightness_topic = heapless::String::<64>::new();
        write!(brightness_topic, "{BASE_MQTT_TOPIC}{BRIGHTNESS_TOPIC}").unwrap();

        let mut color_topic = heapless::String::<64>::new();
        write!(color_topic, "{BASE_MQTT_TOPIC}{COLOR_TOPIC}").unwrap();

        let mut rgb_topic = heapless::String::<64>::new();
        write!(rgb_topic, "{BASE_MQTT_TOPIC}{RGB_TOPIC}").unwrap();

        let mut text_topic = heapless::String::<64>::new();
        write!(text_topic, "{BASE_MQTT_TOPIC}{TEXT_TOPIC}").unwrap();

        let mut app_topic = heapless::String::<64>::new();
        write!(app_topic, "{BASE_MQTT_TOPIC}{APP_TOPIC}").unwrap();

        let mut clock_app_topic = heapless::String::<64>::new();
        write!(clock_app_topic, "{BASE_MQTT_TOPIC}{CLOCK_APP_TOPIC}").unwrap();

        let hass_topic = "homeassistant/status";

        let topics: Vec<&str, 7> = Vec::from_slice(&[
            brightness_topic.as_str(),
            color_topic.as_str(),
            rgb_topic.as_str(),
            text_topic.as_str(),
            app_topic.as_str(),
            clock_app_topic.as_str(),
            hass_topic,
        ])
        .unwrap();

        match client.subscribe_to_topics(&topics).await {
            Ok(_) => MqttMessage::enqueue_debug("Subscribed to topics").await,
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
                        } else if mqtt_message.0.contains("homeassistant") {
                            homeassistant::HASS_RECIEVE_CHANNEL.send(message).await;
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
        MqttMessage::enqueue_debug(message).await;
    }

    async fn show_reason_code(code: ReasonCode) {
        let text = get_reason_code(code);
        DisplayTextMessage::from_system(text, None, None)
            .send_and_replace_queue()
            .await;
    }
}

pub mod homeassistant {
    use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
    use embassy_sync::channel::Channel;
    use embassy_time::Timer;
    use rust_mqtt::packet::v5::publish_packet::QualityOfService;

    use crate::app::AppController;
    use crate::display::{send_brightness_state, send_color_state};
    use crate::mqtt::MqttMessage;

    use super::MqttReceiveMessage;

    pub static HASS_RECIEVE_CHANNEL: Channel<ThreadModeRawMutex, MqttReceiveMessage, 2> =
        Channel::new();

    async fn send_home_assistant_discovery() {
        let topic = "homeassistant/select/galactic_unicorn/clock_effect/config";
        let payload = r#"
{
  "dev" : {
    "name": "Galactic Unicorn",
    "ids": "ga_01"
  },
  "name": "Clock effect",
  "uniq_id": "ga_clock_01",
  "~": "galactic_unicorn/app/clock",
  "stat_t": "~/state",
  "cmd_t": "~",
  "options": ["Rainbow", "Color"]
}"#
        .trim();
        MqttMessage::enqueue_hass(topic, payload).await;

        let topic = "homeassistant/select/galactic_unicorn/active_app/config";
        let payload = r#"
{
  "dev" : {
    "name": "Galactic Unicorn",
    "ids": "ga_01"
  },
  "name": "Active app",
  "uniq_id": "ga_apps_01",
  "~": "galactic_unicorn/app",
  "stat_t": "~/state",
  "cmd_t": "~",
  "options": ["Clock", "Effects", "Mqtt"]
}"#
        .trim();
        MqttMessage::enqueue_hass(topic, payload).await;

        let topic = "homeassistant/notify/galactic_unicorn/mqtt_message/config";
        let payload = r#"
{
  "dev" : {
    "name": "Galactic Unicorn",
    "ids": "ga_01"
  },
  "name": "Display text",
  "cmd_t": "galactic_unicorn/app/text",
  "uniq_id": "ga_display_text_01"
}"#
        .trim();
        MqttMessage::enqueue_hass(topic, payload).await;

        let topic = "homeassistant/light/galactic_unicorn/board/config";
        let payload = r#"
{
  "dev" : {
    "name": "Galactic Unicorn",
    "ids": "ga_01"
  },
  "name": "Display",
  "~": "galactic_unicorn/display",
  "cmd_t": "~/brightness",
  "pl_off": 0,
  "rgb_stat_t": "~/rgb/state",
  "rgb_cmd_t": "~/rgb",
  "bri_stat_t": "~/brightness/state",
  "bri_cmd_t": "~/brightness",
  "on_cmd_type": "brightness",
  "uniq_id": "ga_light_01"
}"#
        .trim();
        MqttMessage::enqueue_hass(topic, payload).await;
    }

    async fn send_states(app_controller: &'static AppController) {
        send_brightness_state().await;
        send_color_state().await;
        app_controller.send_states().await;
    }

    impl MqttMessage {
        async fn enqueue_hass(topic: &'static str, content: &str) {
            Self::enqueue(topic, content, QualityOfService::QoS0, false, false).await;
        }
    }

    #[embassy_executor::task]
    pub async fn hass_discovery_task(app_controller: &'static AppController) {
        send_home_assistant_discovery().await;
        Timer::after_secs(3).await;
        send_states(app_controller).await;

        loop {
            let message = HASS_RECIEVE_CHANNEL.receive().await;
            if message.topic == "homeassistant/status" {
                send_home_assistant_discovery().await;
                Timer::after_secs(1).await;
                send_states(app_controller).await;
            }
        }
    }
}
