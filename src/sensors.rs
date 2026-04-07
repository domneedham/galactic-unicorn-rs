use core::fmt::Write;

use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, watch::Watch};
use heapless::String;
use static_cell::make_static;

use crate::mqtt::MqttReceiveMessage;

/// Global sensor state, updated when MQTT messages arrive on configured topics.
pub struct SensorState {
    /// Temperature in Celsius, or None if not yet received.
    pub temperature: Watch<ThreadModeRawMutex, Option<f32>, 4>,

    /// Relative humidity as a percentage, or None if not yet received.
    pub humidity: Watch<ThreadModeRawMutex, Option<f32>, 4>,

    /// PM2.5 particulate concentration, or None if not yet received.
    pub pm2: Watch<ThreadModeRawMutex, Option<f32>, 4>,
}

impl SensorState {
    /// Create the static ref to sensor state. Must only be called once.
    pub fn new() -> &'static Self {
        make_static!(Self {
            temperature: Watch::new(),
            humidity: Watch::new(),
            pm2: Watch::new(),
        })
    }

    /// Update temperature from a raw MQTT payload string.
    pub fn update_temperature(&self, body: &str) {
        if let Some(value) = parse_sensor_value(body) {
            self.temperature.sender().send(Some(value));
        }
    }

    /// Update humidity from a raw MQTT payload string.
    pub fn update_humidity(&self, body: &str) {
        if let Some(value) = parse_sensor_value(body) {
            self.humidity.sender().send(Some(value));
        }
    }

    /// Update PM2.5 from a raw MQTT payload string.
    pub fn update_pm2(&self, body: &str) {
        if let Some(value) = parse_sensor_value(body) {
            self.pm2.sender().send(Some(value));
        }
    }

    /// Build a summary string of all available sensor readings.
    /// Returns None if no sensor data has been received yet.
    pub fn summary(&self) -> Option<String<64>> {
        let temp = self.temperature.try_get().flatten();
        let humidity = self.humidity.try_get().flatten();
        let pm2 = self.pm2.try_get().flatten();

        if temp.is_none() && humidity.is_none() && pm2.is_none() {
            return None;
        }

        let mut result = String::<64>::new();
        let mut first = true;

        if let Some(t) = temp {
            let _ = write!(result, "{:.1}C", t);
            first = false;
        }
        if let Some(h) = humidity {
            if !first {
                let _ = result.push_str(" - ");
            }
            let _ = write!(result, "{:.0}%", h);
            first = false;
        }
        if let Some(p) = pm2 {
            if !first {
                let _ = result.push_str(" - ");
            }
            let _ = write!(result, "PM{:.1}", p);
        }

        Some(result)
    }
}

/// Parse a sensor value from a string. Accepts integers and decimals up to 2dp.
/// Does not accept units — only the number itself.
fn parse_sensor_value(s: &str) -> Option<f32> {
    let trimmed = s.trim();
    // Reject anything with non-numeric characters (except leading '-' and one '.')
    parse_f32(trimmed)
}

/// Minimal f32 parser for no_std: parses "[−]digits[.digits]".
fn parse_f32(s: &str) -> Option<f32> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }

    let (negative, digits) = if bytes[0] == b'-' {
        (true, &bytes[1..])
    } else {
        (false, bytes)
    };

    if digits.is_empty() {
        return None;
    }

    let mut integer_part: i32 = 0;
    let mut fractional_part: i32 = 0;
    let mut fractional_divisor: i32 = 1;
    let mut seen_dot = false;

    for &b in digits {
        if b == b'.' {
            if seen_dot {
                return None; // two dots
            }
            seen_dot = true;
        } else if b.is_ascii_digit() {
            let digit = (b - b'0') as i32;
            if seen_dot {
                fractional_part = fractional_part * 10 + digit;
                fractional_divisor *= 10;
            } else {
                integer_part = integer_part * 10 + digit;
            }
        } else {
            return None; // non-numeric character
        }
    }

    let value = integer_part as f32 + fractional_part as f32 / fractional_divisor as f32;
    Some(if negative { -value } else { value })
}

/// Task that processes incoming MQTT messages for sensor topics.
#[embassy_executor::task]
pub async fn process_sensor_mqtt_task(
    sensor_state: &'static SensorState,
    mut subscriber: embassy_sync::pubsub::Subscriber<
        'static,
        ThreadModeRawMutex,
        MqttReceiveMessage,
        8,
        1,
        1,
    >,
) {
    use crate::config::{SENSOR_HUMIDITY_TOPIC, SENSOR_PM2_TOPIC, SENSOR_TEMPERATURE_TOPIC};

    loop {
        let message = subscriber.next_message_pure().await;

        if let Some(topic) = SENSOR_TEMPERATURE_TOPIC {
            if message.topic == topic {
                sensor_state.update_temperature(&message.body);
                continue;
            }
        }

        if let Some(topic) = SENSOR_HUMIDITY_TOPIC {
            if message.topic == topic {
                sensor_state.update_humidity(&message.body);
                continue;
            }
        }

        if let Some(topic) = SENSOR_PM2_TOPIC {
            if message.topic == topic {
                sensor_state.update_pm2(&message.body);
            }
        }
    }
}
