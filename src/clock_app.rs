use chrono::{Datelike, Timelike, Weekday};
use core::fmt::Write;
use embassy_time::Timer;
use heapless::String;

use crate::{
    app::UnicornApp, buttons::ButtonPress, time::Time, unicorn::display::DisplayTextMessage,
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

    pub async fn get_date_str(&self) -> String<10> {
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
        let mut result = String::<10>::new();
        write!(result, "{day_title} {day} {month} ").unwrap();
        result
    }
}

impl UnicornApp for ClockApp {
    async fn display(&self) {
        loop {
            let time = self.get_time_str().await;
            DisplayTextMessage::from_app(
                &time,
                None,
                None,
                Some(embassy_time::Duration::from_secs(1)),
            )
            .send_and_replace_queue()
            .await;

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
