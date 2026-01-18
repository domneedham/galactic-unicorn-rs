use chrono::{Datelike, Timelike, Weekday};
use core::{fmt::Write, str::FromStr};
use embassy_futures::select::{select, Either};
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, mutex::Mutex};
use embassy_time::{Duration, Timer};
use embedded_graphics::{
    geometry::{Point, Size},
    mono_font::{iso_8859_13::FONT_5X7, MonoTextStyle},
    pixelcolor::{Rgb888, RgbColor, WebColors},
    primitives::{Primitive, PrimitiveStyleBuilder, Rectangle},
    text::Text,
};
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::signal::Signal;
use embedded_graphics_core::Drawable;
use heapless::{String, Vec};
use micromath::F32Ext;
use static_cell::make_static;
use strum_macros::{EnumString, IntoStaticStr};
use unicorn_graphics::UnicornGraphics;

use crate::{
    app::{AppNotificationPolicy, AppRunner, AppRunnerInboxSubscribers, UnicornApp, UnicornAppRunner},
    buttons::ButtonPress,
    display::{DisplayState, GraphicsBufferWriter, HEIGHT, WIDTH},
    fonts::DrawOntoGraphics,
    mqtt::{topics::CLOCK_APP_STATE_TOPIC, MqttMessage, MqttReceiveMessage},
    time::Time,
};

/// All the effects that can be displayed on the clock.
#[derive(Clone, Copy, EnumString, IntoStaticStr)]
#[strum(ascii_case_insensitive)]
pub enum ClockEffect {
    /// An animated rainbow effect.
    Rainbow,

    /// Display the active color.
    Color,
}

/// Clock app state. Holds persistent state across runner instances.
pub struct ClockAppState {
    /// Reference to the display state.
    display_state: &'static DisplayState,

    /// Reference to the time.
    time: &'static Time,

    /// The current effect of the clock.
    effect: Mutex<NoopRawMutex, ClockEffect>,
}

/// Trait for defining text width constant on the clock app struct.
trait AlternateTextWidth {
    /// Width of the clock text.
    const TEXT_WIDTH: usize;
}

impl AlternateTextWidth for ClockAppState {
    const TEXT_WIDTH: usize = 41;
}

impl ClockAppState {
    /// Create the static ref to clock app state.
    /// Must only be called once or will panic.
    pub fn new(display_state: &'static DisplayState, time: &'static Time) -> &'static Self {
        make_static!(Self {
            display_state,
            time,
            effect: Mutex::new(ClockEffect::Color),
        })
    }

    /// Set the active effect.
    pub async fn set_effect(&self, effect: ClockEffect) {
        *self.effect.lock().await = effect;
        self.send_mqtt_state().await;
    }

    /// Get the date str in format <day:3> <num:1/2> <mon:3>
    pub async fn get_date_str(&self) -> String<12> {
        let dt = self.time.now().await;
        let day_title = match dt.weekday() {
            Weekday::Mon => "Mon",
            Weekday::Tue => "Tue",
            Weekday::Wed => "Wed",
            Weekday::Thu => "Thu",
            Weekday::Fri => "Fri",
            Weekday::Sat => "Sat",
            Weekday::Sun => "Sun",
        };
        let day = dt.day();
        let month = match dt.month() {
            1 => "Jan",
            2 => "Feb",
            3 => "Mar",
            4 => "Apr",
            5 => "May",
            6 => "Jun",
            7 => "Jul",
            8 => "Aug",
            9 => "Sep",
            10 => "Oct",
            11 => "Nov",
            12 => "Dec",
            _ => "..",
        };
        let mut result = String::<12>::new();
        write!(result, "{day_title} {day} {month} ").unwrap();
        result
    }

    /// Get the current day as a string.
    /// Will prepend 0 if day is below 10.
    pub async fn get_day_str(&self) -> String<2> {
        let dt = self.time.now().await;
        let day = dt.day();

        let mut result = String::<2>::new();
        if day < 10 {
            let _ = write!(result, "0{day}");
        } else {
            let _ = write!(result, "{day}");
        }
        result
    }

    /// Draw a colon at `x` position.
    fn draw_colon(gr: &mut UnicornGraphics<WIDTH, HEIGHT>, x: u32) {
        let x = x as i32;
        gr.set_pixel(Point { x, y: 3 }, Rgb888::new(100, 100, 100));
        gr.set_pixel(Point { x, y: 4 }, Rgb888::new(100, 100, 100));
        gr.set_pixel(Point { x, y: 7 }, Rgb888::new(100, 100, 100));
        gr.set_pixel(Point { x, y: 8 }, Rgb888::new(100, 100, 100));
    }

    /// Draw the `num` at the `start` position in the `color`.
    /// Will prepend 0 if the `num` is below 10.
    fn draw_numbers(gr: &mut UnicornGraphics<WIDTH, HEIGHT>, num: u32, start: u32, color: Rgb888) {
        let mut num_str = heapless::String::<4>::new();
        if num < 10 {
            let _ = write!(num_str, "0{num}");
        } else {
            let _ = write!(num_str, "{num}");
        }

        num_str.as_str().draw(gr, start, color);
    }

    /// Turn hsv color into `Rgb888`.
    fn from_hsv(h: f32, s: f32, v: f32) -> Rgb888 {
        let i = (h * 6.0).floor();
        let f = h * 6.0 - i;
        let v = v * 255.0;
        let p = v * (1.0 - s);
        let q = v * (1.0 - f * s);
        let t = v * (1.0 - (1.0 - f) * s);

        let i = i.round() % 6.0;
        if i == 0.0 {
            return Rgb888::new(v.round() as u8, t.round() as u8, p.round() as u8);
        } else if i == 1.0 {
            return Rgb888::new(q.round() as u8, v.round() as u8, p.round() as u8);
        } else if i == 2.0 {
            return Rgb888::new(p.round() as u8, v.round() as u8, t.round() as u8);
        } else if i == 3.0 {
            return Rgb888::new(p.round() as u8, q.round() as u8, v.round() as u8);
        } else if i == 4.0 {
            return Rgb888::new(t.round() as u8, p.round() as u8, v.round() as u8);
        } else if i == 5.0 {
            return Rgb888::new(v.round() as u8, p.round() as u8, q.round() as u8);
        } else {
            return Rgb888::new(0, 0, 0);
        }
    }

    /// Generate the rainbow colors needed for the rainbow effect.
    fn generate_rainbow_colors() -> Vec<Rgb888, { Self::TEXT_WIDTH }> {
        let mut colors = Vec::<Rgb888, { Self::TEXT_WIDTH }>::new();

        for x in 0..Self::TEXT_WIDTH {
            let color = Self::from_hsv(x as f32 / Self::TEXT_WIDTH as f32, 1.0, 1.0);
            colors.push(color).unwrap();
        }

        colors
    }

    /// Send the current state to MQTT.
    pub async fn send_mqtt_state(&self) {
        let effect = *self.effect.lock().await;
        let text = effect.into();
        MqttMessage::enqueue_state(CLOCK_APP_STATE_TOPIC, text).await;
    }

    /// Process an MQTT message.
    pub async fn process_mqtt_message(&self, message: MqttReceiveMessage) {
        if let Ok(effect) = ClockEffect::from_str(&message.body) {
            self.set_effect(effect).await;
        }
    }
}

impl UnicornApp for ClockAppState {
    async fn create_runner(
        &'static self,
        graphics_buffer: GraphicsBufferWriter,
        inbox: AppRunnerInboxSubscribers,
        notification_policy: Signal<ThreadModeRawMutex, AppNotificationPolicy>,
    ) -> AppRunner {
        AppRunner::Clock(ClockAppRunner::new(graphics_buffer, self, inbox, notification_policy))
    }
}

/// Runner for the clock app. Handles the display loop.
pub struct ClockAppRunner {
    graphics_buffer: GraphicsBufferWriter,
    state: &'static ClockAppState,
    inbox: AppRunnerInboxSubscribers,
    notification_policy: Signal<ThreadModeRawMutex, AppNotificationPolicy>,
}

impl<'a> ClockAppRunner {
    pub fn new(
        graphics_buffer: GraphicsBufferWriter,
        state: &'static ClockAppState,
        inbox: AppRunnerInboxSubscribers,
        notification_policy: Signal<ThreadModeRawMutex, AppNotificationPolicy>,
    ) -> Self {
        Self {
            graphics_buffer,
            state,
            inbox,
            notification_policy,
        }
    }
}

impl UnicornAppRunner for ClockAppRunner {
    async fn run(&mut self) -> ! {
        let mut hue_offset: f32 = 0.0;
        let colors = ClockAppState::generate_rainbow_colors();

        let white_style = PrimitiveStyleBuilder::new()
            .fill_color(Rgb888::new(100, 100, 100))
            .build();
        let red_style = PrimitiveStyleBuilder::new().fill_color(Rgb888::RED).build();

        let mut color_sub = self.state.display_state.color.receiver().unwrap();

        loop {
            let effect = *self.state.effect.lock().await;

            let dt = self.state.time.now().await;
            let hour = dt.time().hour();
            let minute = dt.time().minute();
            let second = dt.time().second();

            self.graphics_buffer.clear().await;

            let color = color_sub.try_get().unwrap_or(Rgb888::CSS_PURPLE);

            {
                let mut pixels = self.graphics_buffer.pixels_mut().await;

                ClockAppState::draw_numbers(&mut *pixels, hour, 0, color);
                ClockAppState::draw_colon(&mut *pixels, 13);
                ClockAppState::draw_numbers(&mut *pixels, minute, 14, color);
                ClockAppState::draw_colon(&mut *pixels, 27);
                ClockAppState::draw_numbers(&mut *pixels, second, 28, color);

                Rectangle::new(
                    Point { x: 42, y: 3 },
                    Size {
                        height: 8,
                        width: 11,
                    },
                )
                .into_styled(white_style)
                .draw(&mut *pixels)
                .unwrap();

                Rectangle::new(
                    Point { x: 42, y: 0 },
                    Size {
                        height: 3,
                        width: 11,
                    },
                )
                .into_styled(red_style)
                .draw(&mut *pixels)
                .unwrap();

                let day = self.state.get_day_str().await;
                Text::new(
                    &day,
                    Point { x: 43, y: 9 },
                    MonoTextStyle::new(&FONT_5X7, Rgb888::RED),
                )
                .draw(&mut *pixels)
                .unwrap();
            }

            match effect {
                ClockEffect::Rainbow => {
                    for _ in 0..20 {
                        {
                            let mut pixels = self.graphics_buffer.pixels_mut().await;

                            for x in 0..ClockAppState::TEXT_WIDTH as u8 {
                                for y in 0..HEIGHT as u8 {
                                    let point = Point::new(x as i32, y as i32);
                                    if pixels.is_match(point, Rgb888::BLACK)
                                        || pixels.is_match(point, Rgb888::new(100, 100, 100))
                                    {
                                        continue;
                                    }

                                    let mut index =
                                        ((x as f32 + (hue_offset * ClockAppState::TEXT_WIDTH as f32))
                                            % ClockAppState::TEXT_WIDTH as f32)
                                            .round() as usize;

                                    if index >= 41 {
                                        index = 0;
                                    }
                                    let value = colors[index];
                                    pixels.set_pixel(point, value);
                                }
                            }
                        }

                        hue_offset += 0.01;

                        self.graphics_buffer.send();

                        // Check for button press during rainbow animation
                        match select(
                            Timer::after_millis(50),
                            self.inbox.buttons.next_message_pure(),
                        ).await {
                            Either::First(_) => { /* Timer expired, continue animation */ }
                            Either::Second(press) => {
                                self.handle_button_press(press).await;
                            }
                        }
                    }
                }
                ClockEffect::Color => {
                    self.graphics_buffer.send();

                    // Wait for timer or button press
                    match select(
                        Timer::after_millis(250),
                        self.inbox.buttons.next_message_pure(),
                    ).await {
                        Either::First(_) => { /* Timer expired, continue */ }
                        Either::Second(press) => {
                            self.handle_button_press(press).await;
                        }
                    }
                }
            }
        }
    }

    fn release_writer(self) -> GraphicsBufferWriter {
        self.graphics_buffer
    }
}

impl<'a> ClockAppRunner {
    /// Handle a button press - show the full date
    async fn handle_button_press(&mut self, press: ButtonPress) {
        if press == ButtonPress::Short {
            // Show the full date on short press
            let date_str = self.state.get_date_str().await;
            self.graphics_buffer
                .display_text(
                    &date_str,
                    None,
                    None,
                    Some(Duration::from_secs(2)),
                    self.state.display_state,
                )
                .await;
        }
    }
}
