use core::fmt::Write;

use embassy_sync::{
    blocking_mutex::raw::ThreadModeRawMutex,
    channel::Channel,
    mutex::{Mutex, MutexGuard},
};
use embassy_time::Timer;
use heapless::String;
use rust_mqtt::packet::v5::publish_packet::QualityOfService;
use topics::DEBUG_TOPIC;

static SEND_CHANNEL: Channel<ThreadModeRawMutex, MutexGuard<ThreadModeRawMutex, MqttMessage>, 4> =
    Channel::new();

static MESSAGE_POOL: [Mutex<ThreadModeRawMutex, MqttMessage>; 4] = [
    Mutex::new(MqttMessage::new()),
    Mutex::new(MqttMessage::new()),
    Mutex::new(MqttMessage::new()),
    Mutex::new(MqttMessage::new()),
];

pub struct MqttMessage {
    topic: &'static str,
    text: String<512>,
    qos: QualityOfService,
    retain: bool,
}

impl MqttMessage {
    const fn new() -> Self {
        MqttMessage {
            topic: "",
            text: String::new(),
            qos: QualityOfService::QoS0,
            retain: false,
        }
    }

    fn reuse(&mut self, topic: &'static str, content: &str, qos: QualityOfService, retain: bool) {
        self.topic = topic;
        self.text.clear();
        self.text.push_str(content).unwrap();
        self.qos = qos;
        self.retain = retain;
    }

    pub async fn enqueue_state(topic: &'static str, content: &str) {
        Self::enqueue(topic, content, QualityOfService::QoS0, false).await;
    }

    pub async fn enqueue_debug(content: &str) {
        Self::enqueue(DEBUG_TOPIC, content, QualityOfService::QoS0, false).await;
    }

    pub async fn enqueue(topic: &'static str, content: &str, qos: QualityOfService, retain: bool) {
        let mut queued = false;
        while !queued {
            for msg_mutex in &MESSAGE_POOL {
                let msg_lock = msg_mutex.try_lock();
                match msg_lock {
                    Ok(mut message) => {
                        message.reuse(topic, content, qos, retain);

                        SEND_CHANNEL.send(message).await;
                        queued = true;
                        break;
                    }
                    Err(_) => {}
                }
            }

            if !queued {
                Timer::after_millis(25).await;
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

pub mod topics {
    use crate::config::*;
    use constcat::concat;

    pub(super) const SET: &str = "set";
    pub(super) const STATE: &str = "state";
    pub(super) const STATUS: &str = "status";

    pub(super) const DEBUG_TOPIC: &str = concat!(BASE_MQTT_TOPIC, "/debug");

    pub const BRIGHTNESS_BASE_TOPIC: &str = concat!(BASE_MQTT_TOPIC, "/display/brightness");
    pub const BRIGHTNESS_SET_TOPIC: &str = concat!(BRIGHTNESS_BASE_TOPIC, "/", SET);
    pub const BRIGHTNESS_STATE_TOPIC: &str = concat!(BRIGHTNESS_BASE_TOPIC, "/", STATE);

    pub const RGB_BASE_TOPIC: &str = concat!(BASE_MQTT_TOPIC, "/display/rgb");
    pub const RGB_SET_TOPIC: &str = concat!(RGB_BASE_TOPIC, "/", SET);
    pub const RGB_STATE_TOPIC: &str = concat!(RGB_BASE_TOPIC, "/", STATE);

    pub const TEXT_BASE_TOPIC: &str = concat!(BASE_MQTT_TOPIC, "/app/text");
    pub const TEXT_SET_TOPIC: &str = concat!(TEXT_BASE_TOPIC, "/", SET);

    pub const APP_BASE_TOPIC: &str = concat!(BASE_MQTT_TOPIC, "/app");
    pub const APP_SET_TOPIC: &str = concat!(APP_BASE_TOPIC, "/", SET);
    pub const APP_STATE_TOPIC: &str = concat!(APP_BASE_TOPIC, "/", STATE);

    pub const CLOCK_APP_BASE_TOPIC: &str = concat!(BASE_MQTT_TOPIC, "/app/clock");
    pub const CLOCK_APP_SET_TOPIC: &str = concat!(CLOCK_APP_BASE_TOPIC, "/", SET);
    pub const CLOCK_APP_STATE_TOPIC: &str = concat!(CLOCK_APP_BASE_TOPIC, "/", STATE);

    pub const NTP_SYNC_TOPIC: &str = concat!(BASE_MQTT_TOPIC, "/system/ntp/sync");
}

pub mod clients {
    use cortex_m::singleton;
    use embassy_futures::select::{select, Either};
    use embassy_net::{tcp::TcpSocket, Stack};
    use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, pubsub::Publisher};
    use embassy_time::Timer;
    use heapless::Vec;
    use rust_mqtt::{
        client::{client::MqttClient, client_config::ClientConfig},
        packet::v5::reason_codes::ReasonCode,
        utils::rng_generator::CountingRng,
    };

    use super::{
        homeassistant,
        topics::{
            APP_SET_TOPIC, BRIGHTNESS_SET_TOPIC, CLOCK_APP_SET_TOPIC, NTP_SYNC_TOPIC,
            RGB_SET_TOPIC, TEXT_SET_TOPIC,
        },
        MqttMessage, MqttReceiveMessage, SEND_CHANNEL,
    };
    use crate::config::{BASE_MQTT_TOPIC, HASS_BASE_MQTT_TOPIC};
    use crate::unicorn::display::DisplayTextMessage;

    #[embassy_executor::task]
    pub async fn mqtt_send_client(stack: &'static Stack<cyw43::NetDriver<'static>>) {
        let tx_buffer = singleton!(: [u8; 2048] = [0; 2048]).unwrap();
        let rx_buffer = singleton!(: [u8; 2048] = [0; 2048]).unwrap();

        let mut socket = TcpSocket::new(stack, rx_buffer, tx_buffer);
        socket.set_timeout(None);
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
                    match client
                        .send_message(
                            message.topic,
                            message.text.as_bytes(),
                            message.qos,
                            message.retain,
                        )
                        .await
                    {
                        Ok(_) => {}
                        Err(_) => {}
                    }

                    drop(message);
                }
                Either::Second(_) => _ = client.send_ping().await,
            }
        }
    }

    #[embassy_executor::task]
    pub async fn mqtt_receive_client(
        stack: &'static Stack<cyw43::NetDriver<'static>>,
        display_publisher: Publisher<'static, ThreadModeRawMutex, MqttReceiveMessage, 8, 1, 1>,
        app_publisher: Publisher<'static, ThreadModeRawMutex, MqttReceiveMessage, 8, 1, 1>,
        system_publisher: Publisher<'static, ThreadModeRawMutex, MqttReceiveMessage, 8, 1, 1>,
    ) {
        let tx_buffer = singleton!(: [u8; 2048] = [0; 2048]).unwrap();
        let rx_buffer = singleton!(: [u8; 2048] = [0; 2048]).unwrap();

        let mut socket = TcpSocket::new(stack, rx_buffer, tx_buffer);
        socket.set_timeout(None);
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
        let recv_buffer = singleton!(: [u8; 512] = [0; 512]).unwrap();
        let write_buffer = singleton!(: [u8; 512] = [0; 512]).unwrap();

        let mut client: MqttClient<'_, TcpSocket<'_>, 5, CountingRng> =
            MqttClient::<_, 5, _>::new(socket, write_buffer, 512, recv_buffer, 512, config);

        match client.connect_to_broker().await {
            Ok(_) => {
                MqttMessage::enqueue_debug("Connected to receiver broker").await;
            }
            Err(code) => send_reason_code(code).await,
        };

        let topics: Vec<&str, 7> = Vec::from_slice(&[
            BRIGHTNESS_SET_TOPIC,
            RGB_SET_TOPIC,
            TEXT_SET_TOPIC,
            APP_SET_TOPIC,
            CLOCK_APP_SET_TOPIC,
            NTP_SYNC_TOPIC,
            homeassistant::HASS_STATUS_TOPIC,
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
                        } else if mqtt_message.0.contains("system") {
                            system_publisher.publish(message).await;
                        } else if mqtt_message.0.contains(HASS_BASE_MQTT_TOPIC) {
                            homeassistant::HASS_RECIEVE_CHANNEL.send(message).await;
                        }
                    }
                    Err(code) => {
                        show_reason_code(code).await;
                    }
                },
                Either::Second(_) => _ = client.send_ping().await,
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
    use core::fmt::Write;

    use constcat::concat;

    use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
    use embassy_sync::channel::Channel;
    use embassy_time::Timer;
    use heapless::String;
    use rust_mqtt::packet::v5::publish_packet::QualityOfService;

    use crate::app::AppController;
    use crate::config::{DEVICE_ID, HASS_BASE_MQTT_TOPIC};
    use crate::display::{send_brightness_state, send_color_state};
    use crate::mqtt::MqttMessage;

    use super::{topics::*, MqttReceiveMessage};

    pub const HASS_STATUS_TOPIC: &str = concat!(HASS_BASE_MQTT_TOPIC, "/", STATUS);

    pub static HASS_RECIEVE_CHANNEL: Channel<ThreadModeRawMutex, MqttReceiveMessage, 2> =
        Channel::new();

    async fn send_home_assistant_discovery() {
        let topic = concat!(
            HASS_BASE_MQTT_TOPIC,
            "/select/",
            DEVICE_ID,
            "/clock_effect/config"
        );
        let mut payload = String::<512>::new();
        write!(
            payload,
            r#"
{{
  "dev" : {{
    "ids": "{DEVICE_ID}",
    "name": "Galactic Unicorn",
    "manufacturer": "Pimoroni",
    "model": "Galactic Unicorn"
  }},
  "name": "Clock effect",
  "stat_t": "{CLOCK_APP_STATE_TOPIC}",
  "cmd_t": "{CLOCK_APP_SET_TOPIC}",
  "options": ["Rainbow", "Color"],
  "uniq_id": "{DEVICE_ID}_clock_01"
}}"#
        )
        .unwrap();
        MqttMessage::enqueue_hass(topic, &payload).await;

        let topic = concat!(
            HASS_BASE_MQTT_TOPIC,
            "/select/",
            DEVICE_ID,
            "/active_app/config"
        );
        let mut payload = String::<256>::new();
        write!(
            payload,
            r#"
{{
  "dev" : {{
    "ids": "{DEVICE_ID}"
  }},
  "name": "Active app",
  "stat_t": "{APP_STATE_TOPIC}",
  "cmd_t": "{APP_SET_TOPIC}",
  "options": ["Clock", "Effects", "Mqtt"],
  "uniq_id": "{DEVICE_ID}_apps_01"
}}"#
        )
        .unwrap();
        MqttMessage::enqueue_hass(topic, &payload).await;

        let topic = concat!(
            HASS_BASE_MQTT_TOPIC,
            "/notify/",
            DEVICE_ID,
            "/mqtt_message/config"
        );
        let mut payload = String::<256>::new();
        write!(
            payload,
            r#"
{{
  "dev" : {{
    "ids": "{DEVICE_ID}"
  }},
  "name": "Display text",
  "cmd_t": "{TEXT_SET_TOPIC}",
  "uniq_id": "{DEVICE_ID}_display_text_01"
}}"#
        )
        .unwrap();
        MqttMessage::enqueue_hass(topic, &payload).await;

        let topic = concat!(HASS_BASE_MQTT_TOPIC, "/light/", DEVICE_ID, "/board/config");
        let mut payload = String::<512>::new();
        write!(
            payload,
            r#"
{{
  "dev" : {{
    "ids": "{DEVICE_ID}"
  }},
  "name": "Display",
  "cmd_t": "{BRIGHTNESS_SET_TOPIC}",
  "pl_off": 0,
  "rgb_stat_t": "{RGB_STATE_TOPIC}",
  "rgb_cmd_t": "{RGB_SET_TOPIC}",
  "bri_stat_t": "{BRIGHTNESS_STATE_TOPIC}",
  "bri_cmd_t": "{BRIGHTNESS_SET_TOPIC}",
  "on_cmd_type": "brightness",
  "uniq_id": "{DEVICE_ID}_light_01"
}}"#
        )
        .unwrap();
        MqttMessage::enqueue_hass(topic, &payload).await;

        let topic = concat!(
            HASS_BASE_MQTT_TOPIC,
            "/button/",
            DEVICE_ID,
            "/ntp_sync/config"
        );
        let mut payload = String::<256>::new();
        write!(
            payload,
            r#"
{{
  "dev" : {{
    "ids": "{DEVICE_ID}"
  }},
  "name": "NTP Sync",
  "cmd_t": "{NTP_SYNC_TOPIC}",
  "uniq_id": "{DEVICE_ID}_button_01"
}}"#
        )
        .unwrap();
        MqttMessage::enqueue_hass(topic, &payload).await;
    }

    async fn send_states(app_controller: &'static AppController) {
        send_brightness_state().await;
        send_color_state().await;
        app_controller.send_states().await;
    }

    impl MqttMessage {
        async fn enqueue_hass(topic: &'static str, content: &str) {
            Self::enqueue(topic, content, QualityOfService::QoS0, false).await;
        }
    }

    #[embassy_executor::task]
    pub async fn hass_discovery_task(app_controller: &'static AppController) {
        send_home_assistant_discovery().await;
        Timer::after_secs(3).await;
        send_states(app_controller).await;

        loop {
            let message = HASS_RECIEVE_CHANNEL.receive().await;
            if message.topic == HASS_STATUS_TOPIC {
                send_home_assistant_discovery().await;
                Timer::after_secs(1).await;
                send_states(app_controller).await;
            }
        }
    }
}
