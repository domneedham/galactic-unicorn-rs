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

        brightness.sender().send(255);
        color.sender().send(Rgb888::CSS_PURPLE);
        auto_brightness.sender().send(true);

        make_static!(Self {
            brightness,
            color,
            auto_brightness,
        })
    }
}

// --- 2. Graphics Buffers (The Canvases) ---
pub struct GraphicsBuffer {
    pixels: UnicornGraphics<WIDTH, HEIGHT>,
    buffer_change_signal: &'static Signal<CriticalSectionRawMutex, UnicornGraphics<WIDTH, HEIGHT>>,
}

impl GraphicsBuffer {
    pub const fn new(
        buffer_change_signal: &'static Signal<
            CriticalSectionRawMutex,
            UnicornGraphics<WIDTH, HEIGHT>,
        >,
    ) -> Self {
        Self {
            pixels: UnicornGraphics::<WIDTH, HEIGHT>::new(),
            buffer_change_signal,
        }
    }

    pub fn reader(&self) -> GraphicsBufferReader {
        GraphicsBufferReader::new(&self.pixels, self.buffer_change_signal)
    }

    pub fn writer(&mut self) -> GraphicsBufferWriter<'_> {
        GraphicsBufferWriter::new(&mut self.pixels, self.buffer_change_signal)
    }
}

pub struct GraphicsBufferReader<'a> {
    latest_value: &'a UnicornGraphics<WIDTH, HEIGHT>,
    buffer_change_signal: &'static Signal<CriticalSectionRawMutex, UnicornGraphics<WIDTH, HEIGHT>>,
}

impl<'a> GraphicsBufferReader<'a> {
    pub const fn new(
        latest_value: &'a UnicornGraphics<WIDTH, HEIGHT>,
        buffer_change_signal: &'static Signal<
            CriticalSectionRawMutex,
            UnicornGraphics<WIDTH, HEIGHT>,
        >,
    ) -> Self {
        GraphicsBufferReader {
            latest_value,
            buffer_change_signal,
        }
    }

    pub fn get(&self) -> &UnicornGraphics<WIDTH, HEIGHT> {
        self.latest_value
    }

    pub async fn wait_for_update(&self) -> UnicornGraphics<WIDTH, HEIGHT> {
        self.buffer_change_signal.wait().await
    }
}

pub struct GraphicsBufferWriter<'a> {
    pixels: &'a mut UnicornGraphics<WIDTH, HEIGHT>,
    buffer_change_signal: &'static Signal<CriticalSectionRawMutex, UnicornGraphics<WIDTH, HEIGHT>>,
}

impl<'a> GraphicsBufferWriter<'a> {
    pub const fn new(
        pixels: &'a mut UnicornGraphics<WIDTH, HEIGHT>,
        buffer_change_signal: &'static Signal<
            CriticalSectionRawMutex,
            UnicornGraphics<WIDTH, HEIGHT>,
        >,
    ) -> Self {
        GraphicsBufferWriter {
            pixels,
            buffer_change_signal,
        }
    }

    fn send(&self) {
        self.buffer_change_signal.signal(*self.pixels);
    }

    pub async fn clear(&mut self) {
        self.pixels.clear_all();
        self.send();
    }

    pub async fn set_pixel(&mut self, x: i32, y: i32, color: Rgb888) {
        let point = Point::new(x, y);
        self.pixels.set_pixel(point, color);

        self.send();
    }

    pub async fn display_text(
        &mut self,
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

                self.pixels.fill(Rgb888::new(5, 5, 5)); // Match your original background

                let mut text = Text::new(msg, Point::new((WIDTH / 2) as i32, 5), style);
                text.text_style.alignment = Alignment::Center;
                text.text_style.baseline = Baseline::Middle;
                let _ = text.draw(self.pixels);
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

                // Data logic: Held for the shortest time possible

                self.pixels.clear_all();
                let mut text = Text::new(msg, Point::new(x, 5), style);
                text.text_style.baseline = Baseline::Middle;
                let _ = text.draw(self.pixels);
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
            // Map 0-0xFFFF lux to 0-255 brightness (simplified logic)
            let brightness = (lux >> 8) as u8;
            state.brightness.sender().send(brightness);
        }

        // Wait for setting change OR periodic check
        match embassy_futures::select::select(enabled_sub.changed(), Timer::after_secs(2)).await {
            _ => {}
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
