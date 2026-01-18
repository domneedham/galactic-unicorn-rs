use core::fmt::Write;
use embassy_rp::peripherals::{ADC, DMA_CH0, PIO0, USB};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_sync::watch::Watch;
use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, signal::Signal};
use embassy_time::{Duration, Instant, Timer};
use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyle},
    pixelcolor::Rgb888,
    prelude::*,
    text::{Alignment, Baseline, Text},
};
use galactic_unicorn_embassy::{
    pins::{UnicornDisplayPins, UnicornSensorPins},
    GalacticUnicorn,
};
use heapless::String;
use static_cell::make_static;
use unicorn_graphics::UnicornGraphics;

use crate::mqtt::{
    topics::{AUTO_BRIGHTNESS_STATE_TOPIC, BRIGHTNESS_STATE_TOPIC, RGB_STATE_TOPIC},
    MqttMessage,
};

pub const WIDTH: usize = 53;
pub const HEIGHT: usize = 11;

// --- 1. Global State (The Source of Truth) ---

pub struct DisplayState {
    pub brightness: Watch<ThreadModeRawMutex, u8, 4>,
    pub color: Watch<ThreadModeRawMutex, Rgb888, 4>,
    pub auto_brightness: Watch<ThreadModeRawMutex, bool, 4>,
}

impl DisplayState {
    pub fn new() -> &'static Self {
        let brightness = Watch::new();
        let color = Watch::new();
        let auto_brightness = Watch::new();

        brightness.sender().send(128);
        color.sender().send(Rgb888::CSS_PURPLE);
        auto_brightness.sender().send(false);

        make_static!(Self {
            brightness,
            color,
            auto_brightness,
        })
    }
}

// --- 2. Graphics Buffers (The Canvases) ---
pub struct GraphicsBuffer {
    pixels: &'static Mutex<CriticalSectionRawMutex, UnicornGraphics<WIDTH, HEIGHT>>,
    buffer_change_signal: &'static Signal<CriticalSectionRawMutex, ()>,
}

impl GraphicsBuffer {
    pub const fn new(
        pixels: &'static Mutex<CriticalSectionRawMutex, UnicornGraphics<WIDTH, HEIGHT>>,
        buffer_change_signal: &'static Signal<CriticalSectionRawMutex, ()>,
    ) -> Self {
        Self {
            pixels,
            buffer_change_signal,
        }
    }

    pub fn reader(&self) -> GraphicsBufferReader {
        GraphicsBufferReader::new(self.pixels, self.buffer_change_signal)
    }

    pub fn writer(&self) -> GraphicsBufferWriter {
        GraphicsBufferWriter::new(self.pixels, self.buffer_change_signal)
    }
}

pub struct GraphicsBufferReader {
    pixels: &'static Mutex<CriticalSectionRawMutex, UnicornGraphics<WIDTH, HEIGHT>>,
    buffer_change_signal: &'static Signal<CriticalSectionRawMutex, ()>,
}

impl GraphicsBufferReader {
    pub const fn new(
        pixels: &'static Mutex<CriticalSectionRawMutex, UnicornGraphics<WIDTH, HEIGHT>>,
        buffer_change_signal: &'static Signal<CriticalSectionRawMutex, ()>,
    ) -> Self {
        GraphicsBufferReader {
            pixels,
            buffer_change_signal,
        }
    }

    /// Get a copy of the current buffer contents
    pub async fn get(&self) -> UnicornGraphics<WIDTH, HEIGHT> {
        *self.pixels.lock().await
    }

    /// Wait for the buffer to be updated, then return a copy
    pub async fn wait_for_update(&self) -> UnicornGraphics<WIDTH, HEIGHT> {
        self.buffer_change_signal.wait().await;
        *self.pixels.lock().await
    }

    /// Get read-only access to the buffer. The lock is held for the duration of the guard.
    ///
    /// # Performance
    /// Use this when you need direct access without copying the buffer.
    /// The mutex will be locked until the returned guard is dropped.
    pub async fn lock(&self) -> impl core::ops::Deref<Target = UnicornGraphics<WIDTH, HEIGHT>> + '_ {
        self.pixels.lock().await
    }

    /// Wait for an update signal, then get read-only access to the buffer.
    pub async fn wait_and_lock(&self) -> impl core::ops::Deref<Target = UnicornGraphics<WIDTH, HEIGHT>> + '_ {
        self.buffer_change_signal.wait().await;
        self.pixels.lock().await
    }
}

pub struct GraphicsBufferWriter {
    pixels: &'static Mutex<CriticalSectionRawMutex, UnicornGraphics<WIDTH, HEIGHT>>,
    buffer_change_signal: &'static Signal<CriticalSectionRawMutex, ()>,
}

impl GraphicsBufferWriter {
    pub const fn new(
        pixels: &'static Mutex<CriticalSectionRawMutex, UnicornGraphics<WIDTH, HEIGHT>>,
        buffer_change_signal: &'static Signal<CriticalSectionRawMutex, ()>,
    ) -> Self {
        GraphicsBufferWriter {
            pixels,
            buffer_change_signal,
        }
    }

    /// Get mutable access to the underlying graphics buffer for direct drawing.
    /// Locks the buffer, so keep the scope short!
    pub async fn pixels_mut(&self) -> impl core::ops::DerefMut<Target = UnicornGraphics<WIDTH, HEIGHT>> + '_ {
        self.pixels.lock().await
    }

    /// Signal that the buffer has been updated and should be rendered.
    pub fn send(&self) {
        self.buffer_change_signal.signal(());
    }

    pub async fn clear(&self) {
        self.pixels.lock().await.clear_all();
        self.send();
    }

    /// Set a single pixel.
    ///
    /// # Performance Warning
    /// This acquires and releases the mutex for each pixel. For bulk operations,
    /// use `pixels_mut()` to get the buffer and set multiple pixels in one lock:
    /// ```
    /// let mut pixels = writer.pixels_mut().await;
    /// pixels.set_pixel(Point::new(x1, y1), color1);
    /// pixels.set_pixel(Point::new(x2, y2), color2);
    /// drop(pixels);
    /// writer.send();
    /// ```
    pub async fn set_pixel(&self, x: i32, y: i32, color: Rgb888) {
        let point = Point::new(x, y);
        self.pixels.lock().await.set_pixel(point, color);
        self.send();
    }

    pub async fn display_text(
        &self,
        msg: &str,
        speed: Option<Duration>,
        color_override: Option<Rgb888>,
        min_duration: Option<Duration>,
        state: &'static DisplayState,
    ) {
        let mut color_sub = state.color.receiver().unwrap();
        let mut x = WIDTH as i32;
        let text_width = (msg.len() * 6) as i32;
        let speed = speed.unwrap_or(Duration::from_millis(50));
        let start_time = Instant::now();
        let min_duration = min_duration.unwrap_or(Duration::from_secs(0));

        if text_width < WIDTH as i32 {
            loop {
                let current_color = match color_override {
                    Some(c) => c,
                    None => color_sub.get().await,
                };
                let mut style = MonoTextStyle::new(&FONT_6X10, current_color);
                style.text_color = Some(current_color);

                {
                    let mut pixels = self.pixels.lock().await;
                    pixels.fill(Rgb888::new(5, 5, 5)); // Match your original background

                    let mut text = Text::new(msg, Point::new((WIDTH / 2) as i32, 5), style);
                    text.text_style.alignment = Alignment::Center;
                    text.text_style.baseline = Baseline::Middle;
                    let _ = text.draw(&mut *pixels);
                }
                self.send();

                // Check if we've shown it long enough or if cancelled
                if start_time.elapsed() >= min_duration {
                    break;
                }

                // Minimal sleep to allow color updates without slamming the CPU
                Timer::after_millis(50).await;
            }
        } else {
            while x > -text_width {
                let current_color = match color_override {
                    Some(c) => c,
                    None => color_sub.get().await,
                };
                let style = MonoTextStyle::new(&FONT_6X10, current_color);

                {
                    let mut pixels = self.pixels.lock().await;
                    pixels.clear_all();
                    let mut text = Text::new(msg, Point::new(x, 5), style);
                    text.text_style.baseline = Baseline::Middle;
                    let _ = text.draw(&mut *pixels);
                }
                self.send();

                Timer::after(speed).await;
                x -= 1;
            }
        }
    }
}

// --- 3. The Hardware Wrapper (The Driver) ---

pub struct Display {
    galactic_unicorn: Mutex<ThreadModeRawMutex, GalacticUnicorn<'static>>,
}

impl Display {
    pub fn new(
        pio: embassy_rp::Peri<'static, PIO0>,
        dma: embassy_rp::Peri<'static, DMA_CH0>,
        adc: embassy_rp::Peri<'static, ADC>,
        usb: embassy_rp::Peri<'static, USB>,
        display_pins: UnicornDisplayPins,
        sensor_pins: UnicornSensorPins,
    ) -> &'static Self {
        make_static!(Self {
            galactic_unicorn: Mutex::new(GalacticUnicorn::new(
                pio,
                display_pins,
                sensor_pins,
                adc,
                dma,
                usb,
            ))
        })
    }

    pub async fn update(&self, graphics: &UnicornGraphics<WIDTH, HEIGHT>, brightness: u8) {
        let mut hw = self.galactic_unicorn.lock().await;
        hw.brightness = brightness;
        hw.set_pixels(graphics);
    }

    pub async fn get_light_level(&self) -> u16 {
        self.galactic_unicorn.lock().await.get_light_level().await
    }
}

// --- 5. Background Tasks ---

#[embassy_executor::task]
pub async fn auto_brightness_task(display: &'static Display, state: &'static DisplayState) {
    let mut enabled_sub = state.auto_brightness.receiver().unwrap();

    loop {
        if enabled_sub.get().await {
            let lux = display.get_light_level().await;
            // Map 0-0xFFFF lux to 32-255 brightness with a minimum threshold
            // This ensures the display is never completely dark
            let brightness = ((lux >> 8) as u8).max(32);
            state.brightness.sender().send(brightness);
        }

        // Wait for setting change OR periodic check
        match embassy_futures::select::select(enabled_sub.changed(), Timer::after_secs(2)).await {
            _ => {}
        }
    }
}

// --- MQTT Message Processing Task ---
// This task processes incoming MQTT messages for display settings

#[embassy_executor::task]
pub async fn process_mqtt_messages_task(
    state: &'static DisplayState,
    mut mqtt_sub: embassy_sync::pubsub::Subscriber<
        'static,
        embassy_sync::blocking_mutex::raw::ThreadModeRawMutex,
        crate::mqtt::MqttReceiveMessage,
        8,
        1,
        1,
    >,
) {
    use crate::mqtt::topics::{AUTO_BRIGHTNESS_SET_TOPIC, BRIGHTNESS_SET_TOPIC, RGB_SET_TOPIC};
    use core::str::FromStr;

    loop {
        let message = mqtt_sub.next_message_pure().await;

        if message.topic == BRIGHTNESS_SET_TOPIC {
            if let Ok(brightness) = u8::from_str(&message.body) {
                state.brightness.sender().send(brightness);
            }
        } else if message.topic == RGB_SET_TOPIC {
            // Parse "R,G,B" format
            let parts: heapless::Vec<&str, 3> = message.body.split(',').collect();
            if parts.len() == 3 {
                if let (Ok(r), Ok(g), Ok(b)) = (
                    u8::from_str(parts[0]),
                    u8::from_str(parts[1]),
                    u8::from_str(parts[2]),
                ) {
                    state.color.sender().send(Rgb888::new(r, g, b));
                }
            }
        } else if message.topic == AUTO_BRIGHTNESS_SET_TOPIC {
            let enabled = message.body.as_str() == "ON" || message.body.as_str() == "on";
            state.auto_brightness.sender().send(enabled);
        }
    }
}

// --- State Sync Task ---
// This task watches the Global State. If the brightness or color
// changes (from a button, a sensor, or a local app), it sends the update to MQTT.

#[embassy_executor::task]
pub async fn state_to_mqtt_broadcast_task(state: &'static DisplayState) {
    let mut bright_sub = state.brightness.receiver().unwrap();
    let mut color_sub = state.color.receiver().unwrap();
    let mut auto_sub = state.auto_brightness.receiver().unwrap();

    loop {
        // Wait for ANY of the state values to change
        let update = embassy_futures::select::select3(
            bright_sub.changed(),
            color_sub.changed(),
            auto_sub.changed(),
        )
        .await;

        match update {
            embassy_futures::select::Either3::First(_) => {
                let val = bright_sub.get().await;
                let mut text = String::<3>::new();
                write!(text, "{}", val).unwrap();
                MqttMessage::enqueue_state(BRIGHTNESS_STATE_TOPIC, &text).await;
            }
            embassy_futures::select::Either3::Second(_) => {
                let color = color_sub.get().await;
                let mut text = String::<11>::new();
                write!(text, "{},{},{}", color.r(), color.g(), color.b()).unwrap();
                MqttMessage::enqueue_state(RGB_STATE_TOPIC, &text).await;
            }
            embassy_futures::select::Either3::Third(_) => {
                let enabled = auto_sub.get().await;
                let text = if enabled { "ON" } else { "OFF" };
                MqttMessage::enqueue_state(AUTO_BRIGHTNESS_STATE_TOPIC, text).await;
            }
        }
    }
}
