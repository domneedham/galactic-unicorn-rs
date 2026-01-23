use core::fmt::Write;
use embassy_futures::select::{select3, Either3};
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

// --- 0. Dirty Rectangle Tracking ---

/// Tracks the bounding rectangle of changed pixels
#[derive(Copy, Clone, Debug)]
pub struct DirtyRect {
    min_x: usize,
    min_y: usize,
    max_x: usize,
    max_y: usize,
    is_dirty: bool,
}

impl DirtyRect {
    pub const fn new() -> Self {
        Self {
            min_x: usize::MAX,
            min_y: usize::MAX,
            max_x: 0,
            max_y: 0,
            is_dirty: false,
        }
    }

    /// Mark a single pixel as dirty
    #[inline]
    pub fn mark_pixel(&mut self, x: usize, y: usize) {
        if !self.is_dirty {
            self.min_x = x;
            self.min_y = y;
            self.max_x = x;
            self.max_y = y;
            self.is_dirty = true;
        } else {
            self.min_x = self.min_x.min(x);
            self.min_y = self.min_y.min(y);
            self.max_x = self.max_x.max(x);
            self.max_y = self.max_y.max(y);
        }
    }

    /// Mark a rectangular region as dirty
    #[inline]
    pub fn mark_region(&mut self, x1: usize, y1: usize, x2: usize, y2: usize) {
        if !self.is_dirty {
            self.min_x = x1;
            self.min_y = y1;
            self.max_x = x2;
            self.max_y = y2;
            self.is_dirty = true;
        } else {
            self.min_x = self.min_x.min(x1);
            self.min_y = self.min_y.min(y1);
            self.max_x = self.max_x.max(x2);
            self.max_y = self.max_y.max(y2);
        }
    }

    /// Mark the entire display as dirty
    #[inline]
    pub fn mark_all(&mut self, width: usize, height: usize) {
        self.min_x = 0;
        self.min_y = 0;
        self.max_x = width - 1;
        self.max_y = height - 1;
        self.is_dirty = true;
    }

    /// Clear the dirty flag
    #[inline]
    pub fn clear(&mut self) {
        *self = Self::new();
    }

    /// Get the dirty bounds if any
    #[inline]
    pub fn get_bounds(&self) -> Option<(usize, usize, usize, usize)> {
        if self.is_dirty {
            Some((self.min_x, self.min_y, self.max_x, self.max_y))
        } else {
            None
        }
    }

    /// Check if any pixels are dirty
    #[inline]
    pub fn is_dirty(&self) -> bool {
        self.is_dirty
    }
}

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
    dirty_rect: &'static Mutex<CriticalSectionRawMutex, DirtyRect>,
}

impl GraphicsBuffer {
    pub const fn new(
        pixels: &'static Mutex<CriticalSectionRawMutex, UnicornGraphics<WIDTH, HEIGHT>>,
        buffer_change_signal: &'static Signal<CriticalSectionRawMutex, ()>,
        dirty_rect: &'static Mutex<CriticalSectionRawMutex, DirtyRect>,
    ) -> Self {
        Self {
            pixels,
            buffer_change_signal,
            dirty_rect,
        }
    }

    pub fn reader(&self) -> GraphicsBufferReader {
        GraphicsBufferReader::new(self.pixels, self.buffer_change_signal, self.dirty_rect)
    }

    pub fn writer(&self) -> GraphicsBufferWriter {
        GraphicsBufferWriter::new(self.pixels, self.buffer_change_signal, self.dirty_rect)
    }
}

pub struct GraphicsBufferReader {
    pixels: &'static Mutex<CriticalSectionRawMutex, UnicornGraphics<WIDTH, HEIGHT>>,
    buffer_change_signal: &'static Signal<CriticalSectionRawMutex, ()>,
    dirty_rect: &'static Mutex<CriticalSectionRawMutex, DirtyRect>,
}

impl GraphicsBufferReader {
    pub const fn new(
        pixels: &'static Mutex<CriticalSectionRawMutex, UnicornGraphics<WIDTH, HEIGHT>>,
        buffer_change_signal: &'static Signal<CriticalSectionRawMutex, ()>,
        dirty_rect: &'static Mutex<CriticalSectionRawMutex, DirtyRect>,
    ) -> Self {
        GraphicsBufferReader {
            pixels,
            buffer_change_signal,
            dirty_rect,
        }
    }

    /// Get a copy of the current buffer contents
    pub async fn get(&self) -> UnicornGraphics<WIDTH, HEIGHT> {
        self.pixels.lock().await.clone()
    }

    /// Wait for the buffer to be updated, then return a copy
    pub async fn wait_for_update(&self) -> UnicornGraphics<WIDTH, HEIGHT> {
        self.buffer_change_signal.wait().await;
        self.pixels.lock().await.clone()
    }

    /// Get read-only access to the buffer. The lock is held for the duration of the guard.
    ///
    /// # Performance
    /// Use this when you need direct access without copying the buffer.
    /// The mutex will be locked until the returned guard is dropped.
    pub async fn lock(
        &self,
    ) -> impl core::ops::Deref<Target = UnicornGraphics<WIDTH, HEIGHT>> + '_ {
        self.pixels.lock().await
    }

    /// Wait for an update signal, then get read-only access to the buffer.
    pub async fn wait_and_lock(
        &self,
    ) -> impl core::ops::Deref<Target = UnicornGraphics<WIDTH, HEIGHT>> + '_ {
        self.buffer_change_signal.wait().await;
        self.pixels.lock().await
    }

    /// Get dirty bounds and clear them atomically
    pub async fn consume_dirty_bounds(&self) -> Option<(usize, usize, usize, usize)> {
        let mut dirty = self.dirty_rect.lock().await;
        let bounds = dirty.get_bounds();
        dirty.clear();
        bounds
    }
}

pub struct GraphicsBufferWriter {
    pixels: &'static Mutex<CriticalSectionRawMutex, UnicornGraphics<WIDTH, HEIGHT>>,
    buffer_change_signal: &'static Signal<CriticalSectionRawMutex, ()>,
    dirty_rect: &'static Mutex<CriticalSectionRawMutex, DirtyRect>,
}

/// Combined guard that holds both pixel buffer and dirty tracking locks.
/// Ensures atomic modification of pixels + dirty region marking.
pub struct PixelsGuard<'a> {
    pixels: embassy_sync::mutex::MutexGuard<'a, CriticalSectionRawMutex, UnicornGraphics<WIDTH, HEIGHT>>,
    dirty_rect: embassy_sync::mutex::MutexGuard<'a, CriticalSectionRawMutex, DirtyRect>,
}

impl<'a> PixelsGuard<'a> {
    /// Mark a rectangular region as dirty
    pub fn mark_dirty_region(&mut self, x1: usize, y1: usize, x2: usize, y2: usize) {
        self.dirty_rect.mark_region(x1, y1, x2, y2);
    }

    /// Mark the entire display as dirty
    pub fn mark_all_dirty(&mut self) {
        self.dirty_rect.mark_all(WIDTH, HEIGHT);
    }
}

impl<'a> core::ops::Deref for PixelsGuard<'a> {
    type Target = UnicornGraphics<WIDTH, HEIGHT>;
    fn deref(&self) -> &Self::Target {
        &self.pixels
    }
}

impl<'a> core::ops::DerefMut for PixelsGuard<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.pixels
    }
}

impl GraphicsBufferWriter {
    pub const fn new(
        pixels: &'static Mutex<CriticalSectionRawMutex, UnicornGraphics<WIDTH, HEIGHT>>,
        buffer_change_signal: &'static Signal<CriticalSectionRawMutex, ()>,
        dirty_rect: &'static Mutex<CriticalSectionRawMutex, DirtyRect>,
    ) -> Self {
        GraphicsBufferWriter {
            pixels,
            buffer_change_signal,
            dirty_rect,
        }
    }

    /// Get mutable access to pixels with integrated dirty tracking.
    /// Both locks are held together and released when the guard is dropped.
    pub async fn pixels_mut(&self) -> PixelsGuard<'_> {
        PixelsGuard {
            pixels: self.pixels.lock().await,
            dirty_rect: self.dirty_rect.lock().await,
        }
    }

    /// Signal that the buffer has been updated and should be rendered.
    pub fn send(&self) {
        self.buffer_change_signal.signal(());
    }

    pub async fn clear(&self) {
        self.pixels.lock().await.clear_all();
        self.dirty_rect.lock().await.mark_all(WIDTH, HEIGHT);
        self.send();
    }

    /// Mark a region as dirty. Use this after manually updating pixels via `pixels_mut()`.
    pub async fn mark_dirty_region(&self, x1: usize, y1: usize, x2: usize, y2: usize) {
        self.dirty_rect.lock().await.mark_region(x1, y1, x2, y2);
    }

    /// Mark entire display as dirty. Use this after operations that affect the whole screen.
    pub async fn mark_all_dirty(&self) {
        self.dirty_rect.lock().await.mark_all(WIDTH, HEIGHT);
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
                self.dirty_rect.lock().await.mark_all(WIDTH, HEIGHT);
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
                self.dirty_rect.lock().await.mark_all(WIDTH, HEIGHT);
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

    pub async fn update(
        &self,
        graphics: &UnicornGraphics<WIDTH, HEIGHT>,
        brightness: u8,
        dirty_bounds: Option<(usize, usize, usize, usize)>,
    ) {
        let mut hw = self.galactic_unicorn.lock().await;

        // Check if brightness changed (requires full update)
        if hw.brightness != brightness {
            hw.brightness = brightness;
            hw.set_pixels(graphics); // Full update
        } else {
            // Use partial update with dirty bounds
            hw.set_pixels_partial(graphics, dirty_bounds);
        }
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

// --- Render Task ---

#[embassy_executor::task]
pub async fn render_task(
    display: &'static Display,
    state: &'static DisplayState,
    app_buffer: GraphicsBufferReader,
    notify_buffer: GraphicsBufferReader,
) {
    use crate::app::{DisplayLayer, ACTIVE_LAYER};
    use embassy_futures::select::Either;

    let mut layer_sub = ACTIVE_LAYER.receiver().unwrap();
    let mut bright_sub = state.brightness.receiver().unwrap();

    loop {
        let layer = layer_sub.get().await;
        let brightness = bright_sub.try_get().unwrap_or(128);

        match select3(
            async {
                match layer {
                    DisplayLayer::App => Either::First(app_buffer.wait_and_lock().await),
                    DisplayLayer::Notification => {
                        Either::Second(notify_buffer.wait_and_lock().await)
                    }
                }
            },
            layer_sub.changed(),
            bright_sub.changed(),
        )
        .await
        {
            Either3::First(graphics_guard) => {
                // Get dirty bounds from the appropriate buffer
                let dirty_bounds = match graphics_guard {
                    Either::First(_) => app_buffer.consume_dirty_bounds().await,
                    Either::Second(_) => notify_buffer.consume_dirty_bounds().await,
                };

                // Hold the lock guard while updating to avoid copy
                match graphics_guard {
                    Either::First(ref g) => display.update(&**g, brightness, dirty_bounds).await,
                    Either::Second(ref g) => display.update(&**g, brightness, dirty_bounds).await,
                };
            }
            Either3::Second(new_layer) => {
                // Layer changed - need to read current buffer and render
                match new_layer {
                    DisplayLayer::App => {
                        let graphics = app_buffer.lock().await;
                        let dirty_bounds = app_buffer.consume_dirty_bounds().await;
                        display.update(&graphics, brightness, dirty_bounds).await;
                    }
                    DisplayLayer::Notification => {
                        let graphics = notify_buffer.lock().await;
                        let dirty_bounds = notify_buffer.consume_dirty_bounds().await;
                        display.update(&graphics, brightness, dirty_bounds).await;
                    }
                };
            }
            Either3::Third(new_brightness) => {
                // Brightness changed - forces full update regardless of dirty bounds
                match layer {
                    DisplayLayer::App => {
                        let graphics = app_buffer.lock().await;
                        // Clear dirty tracking since we're doing a full update anyway
                        let _ = app_buffer.consume_dirty_bounds().await;
                        display.update(&graphics, new_brightness, None).await;
                    }
                    DisplayLayer::Notification => {
                        let graphics = notify_buffer.lock().await;
                        // Clear dirty tracking since we're doing a full update anyway
                        let _ = notify_buffer.consume_dirty_bounds().await;
                        display.update(&graphics, new_brightness, None).await;
                    }
                };
            }
        };
    }
}
