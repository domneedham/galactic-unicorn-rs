use chrono::{Datelike, Timelike, Weekday};
use core::{fmt::Write, str::FromStr};
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, mutex::Mutex};
use embassy_time::Timer;
use embedded_graphics::{
    geometry::{Point, Size},
    mono_font::{iso_8859_13::FONT_5X7, MonoTextStyle},
    pixelcolor::{Rgb888, RgbColor},
    primitives::{Primitive, PrimitiveStyleBuilder, Rectangle},
    text::Text,
};
use embedded_graphics_core::Drawable;
use galactic_unicorn_embassy::{HEIGHT, WIDTH};
use heapless::{String, Vec};
use micromath::F32Ext as _; // needed for rem_euclid, floor, abs and round
use strum_macros::{EnumString, IntoStaticStr};
use unicorn_graphics::UnicornGraphics;

use crate::{
    app::UnicornApp,
    buttons::ButtonPress,
    fonts,
    mqtt::{topics::CLOCK_APP_STATE_TOPIC, MqttMessage},
    time::Time,
    unicorn::{
        self,
        display::{DisplayGraphicsMessage, DisplayTextMessage},
    },
};

#[derive(Clone, Copy, EnumString, IntoStaticStr)]
#[strum(ascii_case_insensitive)]
pub enum ClockEffect {
    Rainbow,
    Color,
}

pub struct ClockApp {
    time: &'static Time,
    effect: Mutex<NoopRawMutex, ClockEffect>,
}

impl ClockApp {
    pub fn new(time: &'static Time) -> Self {
        Self {
            time,
            effect: Mutex::new(ClockEffect::Color),
        }
    }

    pub async fn set_effect(&self, effect: ClockEffect) {
        *self.effect.lock().await = effect;
        self.send_state().await;
    }

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

    fn draw_colon(gr: &mut UnicornGraphics<WIDTH, HEIGHT>, x: u32) {
        let x = x as i32;
        gr.set_pixel(Point { x, y: 3 }, Rgb888::new(100, 100, 100));
        gr.set_pixel(Point { x, y: 4 }, Rgb888::new(100, 100, 100));
        gr.set_pixel(Point { x, y: 7 }, Rgb888::new(100, 100, 100));
        gr.set_pixel(Point { x, y: 8 }, Rgb888::new(100, 100, 100));
    }

    fn draw_numbers(gr: &mut UnicornGraphics<WIDTH, HEIGHT>, num: u32, start: u32, color: Rgb888) {
        let mut num_str = heapless::String::<4>::new();
        if num < 10 {
            let _ = write!(num_str, "0{num}");
        } else {
            let _ = write!(num_str, "{num}");
        }

        fonts::draw_str(gr, &num_str, start, color);
    }
}

impl UnicornApp for ClockApp {
    async fn display(&self) {
        let mut hue_offset: f32 = 0.0;
        let colors = generate_rainbow_colors();

        let mut gr = UnicornGraphics::<WIDTH, HEIGHT>::new();

        let white_style = PrimitiveStyleBuilder::new()
            .fill_color(Rgb888::new(100, 100, 100))
            .build();
        let red_style = PrimitiveStyleBuilder::new().fill_color(Rgb888::RED).build();

        loop {
            let effect = *self.effect.lock().await;

            let dt = self.time.now().await;
            let hour = dt.time().hour();
            let minute = dt.time().minute();
            let second = dt.time().second();

            gr.clear_all();

            let color = *unicorn::display::CURRENT_COLOR.lock().await;

            Self::draw_numbers(&mut gr, hour, 0, color);
            Self::draw_colon(&mut gr, 13);
            Self::draw_numbers(&mut gr, minute, 14, color);
            Self::draw_colon(&mut gr, 27);
            Self::draw_numbers(&mut gr, second, 28, color);

            Rectangle::new(
                Point { x: 41, y: 3 },
                Size {
                    height: 8,
                    width: 12,
                },
            )
            .into_styled(white_style)
            .draw(&mut gr)
            .unwrap();

            Rectangle::new(
                Point { x: 41, y: 0 },
                Size {
                    height: 3,
                    width: 12,
                },
            )
            .into_styled(red_style)
            .draw(&mut gr)
            .unwrap();

            let day = self.get_day_str().await;
            Text::new(
                &day,
                Point { x: 42, y: 9 },
                MonoTextStyle::new(&FONT_5X7, Rgb888::RED),
            )
            .draw(&mut gr)
            .unwrap();

            match effect {
                ClockEffect::Rainbow => {
                    for _ in 0..20 {
                        for x in 0..TEXT_WIDTH as u8 {
                            for y in 0..HEIGHT as u8 {
                                let point = Point::new(x as i32, y as i32);
                                if gr.is_match(point, Rgb888::BLACK)
                                    || gr.is_match(point, Rgb888::new(100, 100, 100))
                                {
                                    continue;
                                }

                                let mut index = ((x as f32 + (hue_offset * TEXT_WIDTH as f32))
                                    % TEXT_WIDTH as f32)
                                    .round()
                                    as usize;

                                if index >= 41 {
                                    index = 0;
                                }
                                let value = colors[index];
                                gr.set_pixel(point, value);
                            }
                        }

                        hue_offset += 0.01;

                        let duration = embassy_time::Duration::from_millis(50);
                        DisplayGraphicsMessage::from_app(gr.get_pixels(), Some(duration))
                            .send_and_replace_queue()
                            .await;
                        Timer::after(duration).await;
                    }
                }
                ClockEffect::Color => {
                    let duration = embassy_time::Duration::from_millis(250);
                    DisplayGraphicsMessage::from_app(gr.get_pixels(), Some(duration))
                        .send_and_replace_queue()
                        .await;
                    Timer::after(duration).await;
                }
            }
        }
    }

    async fn start(&self) {}

    async fn stop(&self) {}

    async fn button_press(&self, press: ButtonPress) {
        match press {
            ButtonPress::Short => {
                let date = self.get_date_str().await;
                DisplayTextMessage::from_app(
                    &date,
                    None,
                    None,
                    Some(embassy_time::Duration::from_secs(2)),
                )
                .send_and_show_now()
                .await;
            }
            ButtonPress::Long => {}
            ButtonPress::Double => {
                let current = *self.effect.lock().await;
                let new = match current {
                    ClockEffect::Color => ClockEffect::Rainbow,
                    ClockEffect::Rainbow => ClockEffect::Color,
                };
                self.set_effect(new).await;
            }
        }
    }

    async fn process_mqtt_message(&self, message: crate::mqtt::MqttReceiveMessage) {
        if let Ok(effect) = ClockEffect::from_str(&message.body) {
            self.set_effect(effect).await;
        }
    }

    async fn send_state(&self) {
        let effect = *self.effect.lock().await;
        let text = effect.into();
        MqttMessage::enqueue_state(CLOCK_APP_STATE_TOPIC, text).await;
    }
}

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

const TEXT_WIDTH: usize = 41;
fn generate_rainbow_colors() -> Vec<Rgb888, TEXT_WIDTH> {
    let mut colors = Vec::<Rgb888, TEXT_WIDTH>::new();

    for x in 0..TEXT_WIDTH {
        let color = from_hsv(x as f32 / TEXT_WIDTH as f32, 1.0, 1.0);
        colors.push(color).unwrap();
    }

    colors
}
