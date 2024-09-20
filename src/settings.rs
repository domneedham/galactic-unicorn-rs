use crate::flash::{FlashStorable, FlashType};
use core::str::FromStr;
use heapless::String;

const MAX_STR_LEN: usize = 32;
const MARKER: [u8; 4] = [0xDE, 0xAD, 0xBE, 0xEF];

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct Settings {
    pub(crate) wifi_network: heapless::String<MAX_STR_LEN>,
    pub(crate) wifi_password: heapless::String<MAX_STR_LEN>,
    pub(crate) ip_address: heapless::String<MAX_STR_LEN>,
    pub(crate) prefix_length: u8,
    pub(crate) gateway: heapless::String<MAX_STR_LEN>,
    pub(crate) mqtt_broker: Option<heapless::String<MAX_STR_LEN>>,
    pub(crate) mqtt_broker_port: u16,
    pub(crate) mqtt_username: Option<heapless::String<MAX_STR_LEN>>,
    pub(crate) mqtt_password: Option<heapless::String<MAX_STR_LEN>>,
    pub(crate) base_mqtt_topic: heapless::String<MAX_STR_LEN>,
    pub(crate) device_id: heapless::String<MAX_STR_LEN>,
    pub(crate) hass_base_mqtt_topic: heapless::String<MAX_STR_LEN>,
    pub(crate) is_initialized: bool, // New field to indicate if the flash has been initialized
}

impl Settings {
    pub fn new(
        wifi_network: heapless::String<MAX_STR_LEN>,
        wifi_password: heapless::String<MAX_STR_LEN>,
        ip_address: heapless::String<MAX_STR_LEN>,
        prefix_length: u8,
        gateway: heapless::String<MAX_STR_LEN>,
        mqtt_broker: Option<heapless::String<MAX_STR_LEN>>,
        mqtt_broker_port: u16,
        mqtt_username: Option<heapless::String<MAX_STR_LEN>>,
        mqtt_password: Option<heapless::String<MAX_STR_LEN>>,
        base_mqtt_topic: heapless::String<MAX_STR_LEN>,
        device_id: heapless::String<MAX_STR_LEN>,
        hass_base_mqtt_topic: heapless::String<MAX_STR_LEN>,
        is_initialized: bool,
    ) -> Self {
        Self {
            wifi_network,
            wifi_password,
            ip_address,
            prefix_length,
            gateway,
            mqtt_broker,
            mqtt_broker_port,
            mqtt_username,
            mqtt_password,
            base_mqtt_topic,
            device_id,
            hass_base_mqtt_topic,
            is_initialized,
        }
    }

    // Function to check if the flash memory has been written to
    fn is_flash_written(data: &[u8]) -> bool {
        data.starts_with(&MARKER)
    }
}

impl FlashStorable<512> for Settings {
    const MAX_SIZE: usize = 512;
    const FLASH_TYPE: FlashType = FlashType::Settings;

    fn serialize(&self) -> &[u8] {
        const MAX_SIZE: usize = 512;
        static mut BUFFER: [u8; MAX_SIZE] = [0; MAX_SIZE];

        unsafe {
            let mut buffer_index = 0;

            fn write_to_buffer(index: &mut usize, data: &[u8]) {
                unsafe {
                    for &byte in data {
                        if *index < BUFFER.len() {
                            BUFFER[*index] = byte;
                            *index += 1;
                        }
                    }
                }
            }

            fn write_byte(index: &mut usize, byte: u8) {
                write_to_buffer(index, &[byte])
            }

            // Write the marker to the beginning of the buffer
            write_to_buffer(&mut buffer_index, &MARKER);

            write_to_buffer(&mut buffer_index, self.wifi_network.as_bytes());
            write_byte(&mut buffer_index, 0);

            write_to_buffer(&mut buffer_index, self.wifi_password.as_bytes());
            write_byte(&mut buffer_index, 0);

            write_to_buffer(&mut buffer_index, self.ip_address.as_bytes());
            write_byte(&mut buffer_index, 0);

            write_byte(&mut buffer_index, self.prefix_length);

            write_to_buffer(&mut buffer_index, self.gateway.as_bytes());
            write_byte(&mut buffer_index, 0);

            match &self.mqtt_broker {
                Some(broker) => {
                    write_byte(&mut buffer_index, 1);
                    write_to_buffer(&mut buffer_index, broker.as_bytes());
                    write_byte(&mut buffer_index, 0);
                }
                None => write_byte(&mut buffer_index, 0),
            }

            write_to_buffer(&mut buffer_index, &self.mqtt_broker_port.to_le_bytes());

            match &self.mqtt_username {
                Some(username) => {
                    write_byte(&mut buffer_index, 1);
                    write_to_buffer(&mut buffer_index, username.as_bytes());
                    write_byte(&mut buffer_index, 0);
                }
                None => write_byte(&mut buffer_index, 0),
            }

            match &self.mqtt_password {
                Some(password) => {
                    write_byte(&mut buffer_index, 1);
                    write_to_buffer(&mut buffer_index, password.as_bytes());
                    write_byte(&mut buffer_index, 0);
                }
                None => write_byte(&mut buffer_index, 0),
            }

            write_to_buffer(&mut buffer_index, self.base_mqtt_topic.as_bytes());
            write_byte(&mut buffer_index, 0);

            write_to_buffer(&mut buffer_index, self.device_id.as_bytes());
            write_byte(&mut buffer_index, 0);

            write_to_buffer(&mut buffer_index, self.hass_base_mqtt_topic.as_bytes());
            write_byte(&mut buffer_index, 0);

            &BUFFER[..buffer_index]
        }
    }

    async fn deserialize(data: &[u8]) -> Self {
        // Check if the flash memory has been written to
        let is_initialized = Settings::is_flash_written(data);

        if !is_initialized {
            return Self {
                wifi_network: String::new(),
                wifi_password: String::new(),
                ip_address: String::new(),
                prefix_length: 0,
                gateway: String::new(),
                mqtt_broker: None,
                mqtt_broker_port: 0,
                mqtt_username: None,
                mqtt_password: None,
                base_mqtt_topic: String::new(),
                device_id: String::new(),
                hass_base_mqtt_topic: String::new(),
                is_initialized,
            };
        }

        // Helper function to read a null-terminated string from the buffer
        fn read_string(data: &[u8], index: &mut usize) -> String<32> {
            let start = *index;
            while *index < data.len() && data[*index] != 0 {
                *index += 1;
            }
            let end = *index;
            *index += 1; // Skip the null terminator
            if start < end {
                String::from_str(core::str::from_utf8(&data[start..end]).unwrap_or_default())
                    .unwrap_or_default()
            } else {
                String::new()
            }
        }

        // Helper function to read a single byte from the buffer
        fn read_byte(data: &[u8], index: &mut usize) -> u8 {
            let byte = data[*index];
            *index += 1;
            byte
        }

        // Helper function to read a u16 from the buffer (little-endian)
        fn read_u16(data: &[u8], index: &mut usize) -> u16 {
            let value = u16::from_le_bytes([data[*index], data[*index + 1]]);
            *index += 2;
            value
        }

        // Index to track the current position in the data buffer
        let mut index = MARKER.len(); // Skip the marker

        // Deserialize fields in the same order they were serialized
        let wifi_network = read_string(data, &mut index);
        let wifi_password = read_string(data, &mut index);
        let ip_address = read_string(data, &mut index);
        let prefix_length = read_byte(data, &mut index);
        let gateway = read_string(data, &mut index);
        let mqtt_broker = match read_byte(data, &mut index) {
            1 => Some(read_string(data, &mut index)),
            _ => None,
        };
        let mqtt_broker_port = read_u16(data, &mut index);
        let mqtt_username = match read_byte(data, &mut index) {
            1 => Some(read_string(data, &mut index)),
            _ => None,
        };
        let mqtt_password = match read_byte(data, &mut index) {
            1 => Some(read_string(data, &mut index)),
            _ => None,
        };
        let base_mqtt_topic = read_string(data, &mut index);
        let device_id = read_string(data, &mut index);
        let hass_base_mqtt_topic = read_string(data, &mut index);

        Settings {
            wifi_network,
            wifi_password,
            ip_address,
            prefix_length,
            gateway,
            mqtt_broker,
            mqtt_broker_port,
            mqtt_username,
            mqtt_password,
            base_mqtt_topic,
            device_id,
            hass_base_mqtt_topic,
            is_initialized, // Set the is_initialized field based on the marker check
        }
    }
}
