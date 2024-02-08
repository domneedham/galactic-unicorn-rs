use embassy_executor::Spawner;
use embassy_rp::peripherals::{DMA_CH0, PIO0};
use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, mutex::Mutex};
use galactic_unicorn_embassy::{pins::UnicornDisplayPins, GalacticUnicorn};

use crate::mqtt::MqttMessage;

type GalacticUnicornType = Mutex<ThreadModeRawMutex, Option<GalacticUnicorn>>;
static GALACTIC_UNICORN: GalacticUnicornType = Mutex::new(None);

pub async fn init(pio: PIO0, dma: DMA_CH0, pins: UnicornDisplayPins, spawner: Spawner) {
    let gu = GalacticUnicorn::new(pio, pins, dma, spawner);
    GALACTIC_UNICORN.lock().await.replace(gu);
    MqttMessage::debug("Initialised display").send().await;
}

pub mod display {
    use core::sync::atomic::Ordering;

    use embassy_futures::select::{select, Either};
    use embassy_sync::{
        blocking_mutex::raw::ThreadModeRawMutex,
        channel::Channel,
        mutex::Mutex,
        pubsub::{PubSubChannel, Subscriber},
        signal::Signal,
    };
    use embassy_time::{Duration, Instant, Timer};
    use embedded_graphics::{
        mono_font::{ascii::FONT_6X10, MonoTextStyle},
        text::{Alignment, Baseline, Text},
    };
    use embedded_graphics_core::{
        geometry::Point,
        pixelcolor::{Rgb888, WebColors},
        Drawable,
    };
    use galactic_unicorn_embassy::{HEIGHT, WIDTH};
    use heapless::String;
    use unicorn_graphics::{UnicornGraphics, UnicornGraphicsPixels};

    use crate::{
        buttons::{self, BRIGHTNESS_DOWN_PRESS, BRIGHTNESS_UP_PRESS},
        graphics::colors::Rgb888Str,
        mqtt::{DisplayTopics, MqttApp, MqttReceiveMessage},
    };

    use super::GALACTIC_UNICORN;

    static CHANGE_COLOR_CHANNEL: PubSubChannel<ThreadModeRawMutex, Rgb888, 1, 2, 1> =
        PubSubChannel::new();
    static CURRENT_COLOR: Mutex<ThreadModeRawMutex, Rgb888> = Mutex::new(Rgb888::CSS_PURPLE);
    static CURRENT_GRAPHICS: Mutex<ThreadModeRawMutex, Option<UnicornGraphics<WIDTH, HEIGHT>>> =
        Mutex::new(None);

    static INTERRUPT_DISPLAY_CHANNEL: Channel<ThreadModeRawMutex, DisplayMessage, 1> =
        Channel::new();
    static MQTT_DISPLAY_CHANNEL: Channel<ThreadModeRawMutex, DisplayMessage, 16> = Channel::new();
    static SYSTEM_DISPLAY_CHANNEL: Channel<ThreadModeRawMutex, DisplayMessage, 16> = Channel::new();
    static APP_DISPLAY_CHANNEL: Channel<ThreadModeRawMutex, DisplayMessage, 16> = Channel::new();

    pub static STOP_CURRENT_DISPLAY: Signal<ThreadModeRawMutex, bool> = Signal::new();

    enum DisplayChannels {
        MQTT,
        SYSTEM,
        APP,
    }

    enum DisplayMessage {
        Graphics(DisplayGraphicsMessage),
        Text(DisplayTextMessage),
    }

    pub struct DisplayTextMessage {
        text: String<64>,
        color: Option<Rgb888>,
        point: Point,
        duration: Duration,
        first_shown: Option<Instant>,
        channel: DisplayChannels,
    }

    impl DisplayTextMessage {
        pub fn from_mqtt(text: &str, color: Option<Rgb888>, point: Option<Point>) -> Self {
            let point = match point {
                Some(x) => x,
                None => Point::new(0, (HEIGHT / 2) as i32),
            };

            let mut heapless_text = String::<64>::new();
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
            let point = match point {
                Some(x) => x,
                None => Point::new(0, (HEIGHT / 2) as i32),
            };

            let mut heapless_text = String::<64>::new();
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

        pub fn from_app(
            text: &str,
            color: Option<Rgb888>,
            point: Option<Point>,
            duration: Option<Duration>,
        ) -> Self {
            let point = match point {
                Some(x) => x,
                None => Point::new(0, (HEIGHT / 2) as i32),
            };

            let duration = match duration {
                Some(x) => x,
                None => Duration::from_secs(3),
            };

            let mut heapless_text = String::<64>::new();
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
                duration,
                first_shown: None,
                channel: DisplayChannels::APP,
            }
        }
    }

    impl DisplayTextMessage {
        pub async fn send(self) {
            match self.channel {
                DisplayChannels::MQTT => {
                    MQTT_DISPLAY_CHANNEL.send(DisplayMessage::Text(self)).await
                }
                DisplayChannels::SYSTEM => {
                    SYSTEM_DISPLAY_CHANNEL
                        .send(DisplayMessage::Text(self))
                        .await
                }
                DisplayChannels::APP => APP_DISPLAY_CHANNEL.send(DisplayMessage::Text(self)).await,
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
                DisplayChannels::APP => {
                    while APP_DISPLAY_CHANNEL.try_receive().is_ok() {}
                    self.send().await;
                }
            }
        }

        pub async fn send_and_show_now(self) {
            STOP_CURRENT_DISPLAY.signal(true);
            INTERRUPT_DISPLAY_CHANNEL
                .send(DisplayMessage::Text(self))
                .await;
        }
    }

    impl DisplayTextMessage {
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

    pub struct DisplayGraphicsMessage {
        pixels: UnicornGraphicsPixels<WIDTH, HEIGHT>,
        duration: Duration,
        first_shown: Option<Instant>,
        channel: DisplayChannels,
    }

    impl DisplayGraphicsMessage {
        pub fn from_app(
            pixels: UnicornGraphicsPixels<WIDTH, HEIGHT>,
            duration: Option<Duration>,
        ) -> Self {
            let duration = match duration {
                Some(x) => x,
                None => Duration::from_secs(3),
            };

            Self {
                pixels,
                duration,
                first_shown: None,
                channel: DisplayChannels::APP,
            }
        }
    }

    impl DisplayGraphicsMessage {
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

    impl DisplayGraphicsMessage {
        pub async fn send(self) {
            match self.channel {
                DisplayChannels::MQTT => {
                    MQTT_DISPLAY_CHANNEL
                        .send(DisplayMessage::Graphics(self))
                        .await
                }
                DisplayChannels::SYSTEM => {
                    SYSTEM_DISPLAY_CHANNEL
                        .send(DisplayMessage::Graphics(self))
                        .await
                }
                DisplayChannels::APP => {
                    APP_DISPLAY_CHANNEL
                        .send(DisplayMessage::Graphics(self))
                        .await
                }
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
                DisplayChannels::APP => {
                    while APP_DISPLAY_CHANNEL.try_receive().is_ok() {}
                    self.send().await;
                }
            }
        }

        pub async fn send_and_show_now(self) {
            STOP_CURRENT_DISPLAY.signal(true);
            INTERRUPT_DISPLAY_CHANNEL
                .send(DisplayMessage::Graphics(self))
                .await;
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

    pub async fn set_color(color: Rgb888) {
        CURRENT_GRAPHICS
            .lock()
            .await
            .as_mut()
            .unwrap()
            .replace_color_with_new(*CURRENT_COLOR.lock().await, color);

        *CURRENT_COLOR.lock().await = color;
        CHANGE_COLOR_CHANNEL
            .publisher()
            .unwrap()
            .publish_immediate(color);

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

    async fn display_graphics_message(
        graphics: &mut UnicornGraphics<WIDTH, HEIGHT>,
        message: &mut DisplayGraphicsMessage,
    ) {
        message.set_first_shown();

        graphics.pixels = message.pixels;
        set_graphics(graphics).await;

        loop {
            Timer::after_millis(10).await;

            if message.has_min_duration_passed() || STOP_CURRENT_DISPLAY.signaled() {
                STOP_CURRENT_DISPLAY.reset();
                break;
            }
        }
    }

    async fn display_text_message(
        graphics: &mut UnicornGraphics<WIDTH, HEIGHT>,
        message: &mut DisplayTextMessage,
    ) {
        let color = match message.color {
            Some(x) => x,
            None => *CURRENT_COLOR.lock().await,
        };
        let mut style = MonoTextStyle::new(&FONT_6X10, color);
        let width = message.text.len() * style.font.character_size.width as usize;
        let mut color_subscriber = CHANGE_COLOR_CHANNEL.subscriber().unwrap();

        message.set_first_shown();

        if width > WIDTH {
            let mut x: f32 = -(WIDTH as f32);

            loop {
                // if message has done a full scroll
                if x > width as f32 {
                    // if message has been shown for minimum duration then break
                    if message.has_min_duration_passed() {
                        break;
                    }

                    // otherwise, reset scroll and go again
                    x = -(WIDTH as f32);
                }

                if STOP_CURRENT_DISPLAY.signaled() {
                    STOP_CURRENT_DISPLAY.reset();
                    break;
                }

                match color_subscriber.try_next_message_pure() {
                    Some(color) => style.text_color = Some(color),
                    None => {}
                }

                graphics.fill(Rgb888::new(5, 5, 5));
                let mut text = Text::new(
                    message.text.as_str(),
                    Point::new((message.point.x - x as i32) as i32, message.point.y),
                    style,
                );
                text.text_style.baseline = Baseline::Middle;
                text.draw(graphics).unwrap();
                set_graphics(graphics).await;

                x += 0.15;
                Timer::after_millis(10).await;
            }
        } else {
            graphics.fill(Rgb888::new(5, 5, 5));

            let mut text = Text::new(
                message.text.as_str(),
                Point::new((WIDTH / 2) as i32, message.point.y),
                style,
            );
            text.text_style.alignment = Alignment::Center;
            text.text_style.baseline = Baseline::Middle;

            text.draw(graphics).unwrap();
            set_graphics(graphics).await;

            loop {
                Timer::after_millis(10).await;

                if message.has_min_duration_passed() || STOP_CURRENT_DISPLAY.signaled() {
                    STOP_CURRENT_DISPLAY.reset();
                    break;
                }
            }
        }
    }

    #[embassy_executor::task]
    pub async fn process_display_queue_task() {
        let mut graphics = UnicornGraphics::new();
        let mut message: Option<DisplayMessage> = None;

        let mut color_subscriber = CHANGE_COLOR_CHANNEL.subscriber().unwrap();

        let mut is_message_replaced = false;

        loop {
            match INTERRUPT_DISPLAY_CHANNEL.try_receive() {
                Ok(value) => match value {
                    DisplayMessage::Graphics(mut value) => {
                        display_graphics_message(&mut graphics, &mut value).await;
                    }
                    DisplayMessage::Text(mut value) => {
                        display_text_message(&mut graphics, &mut value).await;
                    }
                },
                Err(_) => {}
            };

            if !is_message_replaced {
                match MQTT_DISPLAY_CHANNEL.try_receive() {
                    Ok(value) => {
                        is_message_replaced = true;
                        message.replace(value);
                    }
                    Err(_) => {}
                }
            }

            if !is_message_replaced {
                match SYSTEM_DISPLAY_CHANNEL.try_receive() {
                    Ok(value) => {
                        is_message_replaced = true;
                        message.replace(value);
                    }
                    Err(_) => {}
                }
            }

            if !is_message_replaced {
                match APP_DISPLAY_CHANNEL.try_receive() {
                    Ok(value) => {
                        is_message_replaced = true;
                        message.replace(value);
                    }
                    Err(_) => {}
                }
            }

            if message.is_some() {
                match message.as_mut().unwrap() {
                    DisplayMessage::Graphics(value) => {
                        display_graphics_message(&mut graphics, value).await;
                    }
                    DisplayMessage::Text(value) => {
                        // replace color in message if needed
                        if !is_message_replaced {
                            match color_subscriber.try_next_message_pure() {
                                Some(color) => value.color = Some(color),
                                None => {}
                            }
                        }

                        display_text_message(&mut graphics, value).await;
                    }
                }

                is_message_replaced = false;
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
    pub async fn process_mqtt_messages_task(
        topics: DisplayTopics,
        mqtt_app: &'static MqttApp,
        mut subscriber: Subscriber<'static, ThreadModeRawMutex, MqttReceiveMessage, 16, 1, 1>,
    ) {
        loop {
            let message = subscriber.next_message_pure().await;

            if &message.topic == &topics.display_topic {
                let display_message = DisplayTextMessage::from_mqtt(&message.body, None, None);
                if mqtt_app.is_active.load(Ordering::Relaxed) {
                    display_message.send_and_show_now().await;
                } else {
                    display_message.send().await;
                }

                mqtt_app.set_last_message(message.body).await;
            } else if &message.topic == &topics.display_interrupt_topic {
                DisplayTextMessage::from_mqtt(&message.body, None, None)
                    .send_and_show_now()
                    .await;
            } else if &message.topic == &topics.brightness_topic {
                let brightness: u8 = match message.body.parse() {
                    Ok(value) => value,
                    Err(_) => 255,
                };
                set_brightness(brightness).await;
            } else if &message.topic == &topics.color_topic {
                match Rgb888::from_str(&message.body) {
                    Some(color) => {
                        set_color(color).await;
                    }
                    None => {}
                };
            }
        }
    }
}
