use chrono::{Datelike, Timelike, Weekday};
use core::fmt::Write;
use embassy_time::Timer;
use embedded_graphics::{
    geometry::{Point, Size},
    mono_font::{ascii::FONT_6X10, iso_8859_10::FONT_8X13, iso_8859_13::FONT_5X7, MonoTextStyle},
    pixelcolor::{Rgb888, RgbColor, WebColors},
    primitives::{Primitive, PrimitiveStyleBuilder, Rectangle},
    text::Text,
};
use embedded_graphics_core::Drawable;
use galactic_unicorn_embassy::{HEIGHT, WIDTH};
use heapless::String;
use unicorn_graphics::UnicornGraphics;

use crate::{
    app::UnicornApp,
    buttons::ButtonPress,
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

    pub async fn get_time_str(&self) -> String<10> {
        let dt = self.time.now().await;
        let hours = dt.hour();
        let minutes = dt.minute();
        let seconds = dt.second();

        let mut result = String::<10>::new();
        let time_delimiter = if seconds % 2 == 0 { ":" } else { " " };
        write!(result, "{hours:02}{time_delimiter}{minutes:02}").unwrap();
        result
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

    fn draw_number(gr: &mut UnicornGraphics<WIDTH, HEIGHT>, num: u32, start: u32) {
        match num {
            0..=9 => {
                draw(gr, 0, start);
                draw(gr, num, start + 7);
            }
            10..=100 => {
                draw(gr, num, start);
                draw(gr, num, start + 7);
            }
            _ => {}
        }

        fn get_point(x: u32, y: u32) -> Point {
            Point {
                x: x as i32,
                y: y as i32,
            }
        }

        fn draw(gr: &mut UnicornGraphics<WIDTH, HEIGHT>, num: u32, start: u32) {
            let end = start + 6;
            for x in start..end {
                for y in 0..11 {
                    if x == start || x == start + 1 {
                        match y {
                            2..=3 => gr.set_pixel(get_point(x, y), Rgb888::RED),
                            9..=11 => gr.set_pixel(get_point(x, y), Rgb888::RED),
                            _ => {}
                        }
                    } else if x == start + 2 {
                        match y {
                            1..=11 => gr.set_pixel(get_point(x, y), Rgb888::RED),
                            _ => {}
                        }
                    } else if x == start + 3 {
                        match y {
                            0..=11 => gr.set_pixel(get_point(x, y), Rgb888::RED),
                            _ => {}
                        }
                    } else {
                        match y {
                            9..=11 => gr.set_pixel(get_point(x, y), Rgb888::RED),
                            _ => {}
                        }
                    }
                }
            }
        }
    }
}

impl UnicornApp for ClockApp {
    async fn display(&self) {
        let mut gr = UnicornGraphics::<WIDTH, HEIGHT>::new();
        loop {
            // let time = self.get_time_str().await;
            let dt = self.time.now().await;
            let hour = dt.time().hour();
            let minute = dt.time().minute();
            let second = dt.time().second();

            gr.clear_all();

            for item in 0..7 {
                if item == 0 {
                    Self::draw_number(&mut gr, hour, item);
                } else if item == 1 {
                    Self::draw_colon(&mut gr, item + 12);
                } else if item == 2 {
                    Self::draw_number(&mut gr, minute, item * 7);
                } else if item == 3 {
                    Self::draw_colon(&mut gr, item + 24);
                } else if item == 4 {
                    Self::draw_number(&mut gr, second, item * 7);
                }
            }

            // Text::new(
            //     &time,
            //     Point { x: 0, y: 8 },
            //     MonoTextStyle::new(&FONT_8X13, Rgb888::CSS_PURPLE),
            // )
            // .draw(&mut gr)
            // .unwrap();

            let white_style = PrimitiveStyleBuilder::new()
                .fill_color(Rgb888::new(100, 100, 100))
                .build();
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

            let red_style = PrimitiveStyleBuilder::new().fill_color(Rgb888::RED).build();
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

            DisplayGraphicsMessage::from_app(gr.pixels, Some(embassy_time::Duration::from_secs(1)))
                .send_and_replace_queue()
                .await;

            // DisplayTextMessage::from_app(
            //     &time,
            //     None,
            //     Some(Point { x: 3, y: 2 }),
            //     Some(embassy_time::Duration::from_secs(1)),
            // )
            // .send_and_replace_queue()
            // .await;

            Timer::after_secs(1).await;
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
