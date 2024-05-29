use chrono::{Datelike, Timelike, Weekday};
use core::fmt::Write;
use embassy_time::Timer;
use embedded_graphics::{
    geometry::{Point, Size},
    mono_font::{iso_8859_13::FONT_5X7, MonoTextStyle},
    pixelcolor::{Rgb888, RgbColor, WebColors},
    primitives::{Primitive, PrimitiveStyleBuilder, Rectangle},
    text::Text,
};
use embedded_graphics_core::Drawable;
use galactic_unicorn_embassy::{HEIGHT, WIDTH};
use heapless::{String, Vec};
use micromath::F32Ext as _; // needed for rem_euclid, floor, abs and round
use unicorn_graphics::UnicornGraphics;

use crate::{
    app::UnicornApp,
    buttons::ButtonPress,
    fonts,
    time::Time,
    unicorn::display::{DisplayGraphicsMessage, DisplayTextMessage},
};

pub struct ClockApp {
    time: &'static Time,
}

impl ClockApp {
    pub fn new(time: &'static Time) -> Self {
        Self { time }
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
        write!(result, "{day}").unwrap();
        result
    }

    fn draw_colon(gr: &mut UnicornGraphics<WIDTH, HEIGHT>, x: u32) {
        let x = x as i32;
        gr.set_pixel(Point { x, y: 3 }, Rgb888::new(100, 100, 100));
        gr.set_pixel(Point { x, y: 4 }, Rgb888::new(100, 100, 100));
        gr.set_pixel(Point { x, y: 7 }, Rgb888::new(100, 100, 100));
        gr.set_pixel(Point { x, y: 8 }, Rgb888::new(100, 100, 100));
    }

    fn draw_numbers(gr: &mut UnicornGraphics<WIDTH, HEIGHT>, num: u32, start: u32) {
        let mut num_str = heapless::String::<4>::new();
        if num < 10 {
            let _ = write!(num_str, "0{num}");
        } else {
            let _ = write!(num_str, "{num}");
        }

        fonts::draw_str(gr, &num_str, start, Rgb888::CSS_PINK);
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
            // let time = self.get_time_str().await;
            let dt = self.time.now().await;
            let hour = dt.time().hour();
            let minute = dt.time().minute();
            let second = dt.time().second();

            gr.clear_all();

            for item in 0..7 {
                if item == 0 {
                    Self::draw_numbers(&mut gr, hour, item);
                } else if item == 1 {
                    Self::draw_colon(&mut gr, item + 12);
                } else if item == 2 {
                    Self::draw_numbers(&mut gr, minute, item * 7);
                } else if item == 3 {
                    Self::draw_colon(&mut gr, item + 24);
                } else if item == 4 {
                    Self::draw_numbers(&mut gr, second, item * 7);
                }
            }

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
                            .round() as usize;

                        if index >= 41 {
                            index = 0;
                        }
                        let value = colors[index];
                        gr.set_pixel(point, value);
                    }
                }

                hue_offset += 0.01;

                DisplayGraphicsMessage::from_app(
                    gr.get_pixels(),
                    Some(embassy_time::Duration::from_millis(50)),
                )
                .send_and_replace_queue()
                .await;
                Timer::after_millis(50).await;
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
            ButtonPress::Double => {}
        }
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
