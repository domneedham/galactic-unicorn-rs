use embassy_rp::peripherals::{DMA_CH0, PIO0};
use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, mutex::Mutex};
use galactic_unicorn_embassy::{pins::UnicornDisplayPins, GalacticUnicorn};

use crate::mqtt::MqttMessage;

type GalacticUnicornType = Mutex<ThreadModeRawMutex, Option<GalacticUnicorn<'static>>>;
static GALACTIC_UNICORN: GalacticUnicornType = Mutex::new(None);

pub async fn init(pio: PIO0, dma: DMA_CH0, pins: UnicornDisplayPins) {
    let gu = GalacticUnicorn::new(pio, pins, dma);
    GALACTIC_UNICORN.lock().await.replace(gu);
    MqttMessage::debug("Initialised display").send().await;
}

pub mod display {
    use embassy_futures::select::{select, Either};
    use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, channel::Channel, mutex::Mutex};
    use embassy_time::{Duration, Instant, Timer};
    use embedded_graphics::{
        mono_font::{ascii::FONT_5X8, MonoTextStyle},
        text::Text,
    };
    use embedded_graphics_core::{
        geometry::Point,
        pixelcolor::{Rgb888, RgbColor},
        Drawable,
    };
    use galactic_unicorn_embassy::{HEIGHT, WIDTH};
    use heapless::String;
    use unicorn_graphics::UnicornGraphics;

    use crate::buttons::{self, BRIGHTNESS_DOWN_PRESS, BRIGHTNESS_UP_PRESS};

    use super::GALACTIC_UNICORN;

    static CURRENT_GRAPHICS: Mutex<ThreadModeRawMutex, Option<UnicornGraphics<WIDTH, HEIGHT>>> =
        Mutex::new(None);

    static MQTT_DISPLAY_CHANNEL: Channel<ThreadModeRawMutex, DisplayMessage, 16> = Channel::new();
    static SYSTEM_DISPLAY_CHANNEL: Channel<ThreadModeRawMutex, DisplayMessage, 16> = Channel::new();

    enum DisplayChannels {
        MQTT,
        SYSTEM,
    }

    pub struct DisplayMessage {
        text: String<256>,
        color: Rgb888,
        point: Point,
        duration: Duration,
        first_shown: Option<Instant>,
        channel: DisplayChannels,
    }

    impl DisplayMessage {
        pub fn from_mqtt(text: &str, color: Option<Rgb888>, point: Option<Point>) -> Self {
            let color = match color {
                Some(x) => x,
                None => Rgb888::RED,
            };

            let point = match point {
                Some(x) => x,
                None => Point::new(0, 7),
            };

            let mut heapless_text = String::<256>::new();
            match heapless_text.push_str(text) {
                Ok(_) => {}
                Err(_) => {
                    heapless_text.push_str("Too many characters!").unwrap();
                }
            };

            Self {
                text: heapless_text,
                color,
                point,
                duration: Duration::from_secs(3),
                first_shown: None,
                channel: DisplayChannels::MQTT,
            }
        }

        pub fn from_system(text: &str, color: Option<Rgb888>, point: Option<Point>) -> Self {
            let color = match color {
                Some(x) => x,
                None => Rgb888::RED,
            };

            let point = match point {
                Some(x) => x,
                None => Point::new(0, 7),
            };

            let mut heapless_text = String::<256>::new();
            match heapless_text.push_str(text) {
                Ok(_) => {}
                Err(_) => {
                    heapless_text.push_str("Too many characters!").unwrap();
                }
            };

            Self {
                text: heapless_text,
                color,
                point,
                duration: Duration::from_secs(3),
                first_shown: None,
                channel: DisplayChannels::SYSTEM,
            }
        }
    }

    impl DisplayMessage {
        pub async fn send(self) {
            match self.channel {
                DisplayChannels::MQTT => MQTT_DISPLAY_CHANNEL.send(self).await,
                DisplayChannels::SYSTEM => SYSTEM_DISPLAY_CHANNEL.send(self).await,
            }
        }

        pub async fn send_and_replace_queue(self) {
            match self.channel {
                DisplayChannels::MQTT => {
                    // clear channel
                    while MQTT_DISPLAY_CHANNEL.try_receive().is_ok() {}
                    self.send().await;
                }
                DisplayChannels::SYSTEM => {
                    // clear channel
                    while SYSTEM_DISPLAY_CHANNEL.try_receive().is_ok() {}
                    self.send().await;
                }
            }
        }
    }

    impl DisplayMessage {
        pub fn set_first_shown(&mut self) {
            if self.first_shown.is_none() {
                self.first_shown.replace(Instant::now());
            }
        }

        pub fn has_min_duration_passed(&self) -> bool {
            if self.first_shown.is_none() {
                return false;
            }

            self.first_shown.unwrap().elapsed() > self.duration
        }
    }

    pub async fn set_brightness(brightness: u8) {
        GALACTIC_UNICORN
            .lock()
            .await
            .as_mut()
            .unwrap()
            .set_brightness(brightness);

        redraw_graphics().await;
    }

    async fn set_graphics(graphics: &UnicornGraphics<WIDTH, HEIGHT>) {
        CURRENT_GRAPHICS.lock().await.replace(graphics.clone());

        GALACTIC_UNICORN
            .lock()
            .await
            .as_mut()
            .unwrap()
            .set_pixels(graphics);
    }

    async fn redraw_graphics() {
        GALACTIC_UNICORN
            .lock()
            .await
            .as_mut()
            .unwrap()
            .set_pixels(CURRENT_GRAPHICS.lock().await.as_ref().unwrap());
    }

    async fn display_internal(
        graphics: &mut UnicornGraphics<WIDTH, HEIGHT>,
        message: &mut DisplayMessage,
    ) {
        let style = MonoTextStyle::new(&FONT_5X8, message.color);
        let width = message.text.len() * style.font.character_size.width as usize;

        message.set_first_shown();

        if width > WIDTH {
            let mut x: i32 = -(WIDTH as i32);
            loop {
                // if message has done a full scroll
                if x > width as i32 {
                    // if message has been shown for minimum duration then break
                    if message.has_min_duration_passed() {
                        break;
                    }

                    // otherwise, reset scroll and go again
                    x = -(WIDTH as i32);
                }

                graphics.clear_all();
                Text::new(
                    message.text.as_str(),
                    Point::new((message.point.x - x) as i32, message.point.y),
                    style,
                )
                .draw(graphics)
                .unwrap();
                set_graphics(graphics).await;

                x += 1;
                Timer::after_millis(10).await;
            }
        } else {
            graphics.clear_all();
            Text::new(message.text.as_str(), message.point, style)
                .draw(graphics)
                .unwrap();
            set_graphics(graphics).await;

            if !message.has_min_duration_passed() {
                Timer::after(message.duration).await;
            } else {
                // let other things get processed
                Timer::after_millis(10).await;
            }
        }
    }

    #[embassy_executor::task]
    pub async fn process_display_queue_task() {
        let mut graphics = UnicornGraphics::new();
        let mut message: Option<DisplayMessage> = None;

        loop {
            match MQTT_DISPLAY_CHANNEL.try_receive() {
                Ok(value) => {
                    message.replace(value);
                    continue;
                }
                Err(_) => {}
            }

            match SYSTEM_DISPLAY_CHANNEL.try_receive() {
                Ok(value) => {
                    message.replace(value);
                    continue;
                }
                Err(_) => {}
            }

            if message.is_some() {
                display_internal(&mut graphics, message.as_mut().unwrap()).await;
            } else {
                Timer::after_millis(200).await;
            }
        }
    }

    #[embassy_executor::task]
    pub async fn process_brightness_buttons_task() {
        loop {
            let press_type = select(BRIGHTNESS_UP_PRESS.wait(), BRIGHTNESS_DOWN_PRESS.wait()).await;

            let current_brightness = GALACTIC_UNICORN.lock().await.as_ref().unwrap().brightness;

            match press_type {
                Either::First(press) => match press {
                    buttons::ButtonPress::Short => {
                        set_brightness(current_brightness.saturating_add(10)).await;
                    }
                    buttons::ButtonPress::Long => {
                        set_brightness(255).await;
                    }
                    buttons::ButtonPress::Double => {
                        set_brightness(current_brightness.saturating_add(50)).await
                    }
                },
                Either::Second(press) => match press {
                    buttons::ButtonPress::Short => {
                        set_brightness(current_brightness.saturating_sub(10)).await;
                    }
                    buttons::ButtonPress::Long => {
                        set_brightness(20).await;
                    }
                    buttons::ButtonPress::Double => {
                        set_brightness(current_brightness.saturating_sub(50)).await
                    }
                },
            }
        }
    }

    #[embassy_executor::task]
    pub async fn draw_on_display_task() {
        loop {
            GALACTIC_UNICORN.lock().await.as_mut().unwrap().draw().await;
            Timer::after_millis(10).await;
        }
    }
}
