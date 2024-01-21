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
    use embassy_futures::select::{select, Either};
    use embassy_net::{tcp::TcpSocket, Stack};
    use embassy_time::{Duration, Timer};
    use embedded_graphics_core::{
        geometry::Point,
        pixelcolor::{Rgb888, RgbColor},
    };
    use rust_mqtt::{
        client::{client::MqttClient, client_config::ClientConfig},
        packet::v5::reason_codes::ReasonCode,
        utils::rng_generator::CountingRng,
    };

    use super::{MqttMessage, SEND_CHANNEL};
    use crate::{unicorn::display::DisplayMessage, BASE_MQTT_TOPIC};

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
    pub async fn mqtt_receive_client(stack: &'static Stack<cyw43::NetDriver<'static>>) {
        let tx_buffer = singleton!(: [u8; 4096] = [0; 4096]).unwrap();
        let rx_buffer = singleton!(: [u8; 4096] = [0; 4096]).unwrap();

        let mut socket = TcpSocket::new(stack, rx_buffer, tx_buffer);
        socket.set_timeout(Some(Duration::from_secs(30)));
        let host_addr = embassy_net::Ipv4Address::new(192, 168, 1, 20);
        socket.connect((host_addr, 1883)).await.unwrap();

        let mut config = ClientConfig::new(
            rust_mqtt::client::client_config::MqttVersion::MQTTv5,
            CountingRng(50000),
        );
        config.add_max_subscribe_qos(rust_mqtt::packet::v5::publish_packet::QualityOfService::QoS1);
        config.add_client_id("receiver");
        // config.add_username(USERNAME);
        // config.add_password(PASSWORD);
        config.max_packet_size = 100;
        let mut recv_buffer = [0; 500];
        let mut write_buffer = [0; 500];

        let mut client: MqttClient<'_, TcpSocket<'_>, 5, CountingRng> = MqttClient::<_, 5, _>::new(
            socket,
            &mut write_buffer,
            500,
            &mut recv_buffer,
            500,
            config,
        );

        match client.connect_to_broker().await {
            Ok(_) => {
                MqttMessage::debug("Connected to receiver broker")
                    .send()
                    .await
            }
            Err(code) => send_reason_code(code).await,
        };

        let mut topic = heapless::String::<256>::new();
        _ = write!(topic, "{BASE_MQTT_TOPIC}");
        _ = write!(topic, "display");

        match client.subscribe_to_topic("galactic_unicorn/display").await {
            Ok(_) => MqttMessage::debug("Subscribed to topic").send().await,
            Err(code) => send_reason_code(code).await,
        }

        loop {
            match select(client.receive_message(), Timer::after_secs(5)).await {
                Either::First(received_message) => match received_message {
                    Ok(message) => {
                        MqttMessage::debug("Received text").send().await;
                        let text = core::str::from_utf8(message.1).unwrap();
                        DisplayMessage::from_mqtt(text, Some(Rgb888::RED), Some(Point::new(0, 7)))
                            .send()
                            .await;
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
        DisplayMessage::from_system(text, None, None).send().await;
    }
}
