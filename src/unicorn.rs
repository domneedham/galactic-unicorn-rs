use embassy_rp::peripherals::{DMA_CH0, PIO0};
use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, mutex::Mutex};
use galactic_unicorn_embassy::{pins::UnicornDisplayPins, GalacticUnicorn};

type GalacticUnicornType = Mutex<ThreadModeRawMutex, Option<GalacticUnicorn>>;
static GALACTIC_UNICORN: GalacticUnicornType = Mutex::new(None);

pub async fn init(pio: PIO0, dma: DMA_CH0, pins: UnicornDisplayPins) {
    let gu = GalacticUnicorn::new(pio, pins, dma);
    GALACTIC_UNICORN.lock().await.replace(gu);
}

pub mod display {
    use core::fmt::Write;
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
        pixelcolor::RgbColor,
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
        mqtt::{
            topics::{
                BRIGHTNESS_SET_TOPIC, BRIGHTNESS_STATE_TOPIC, RGB_SET_TOPIC, RGB_STATE_TOPIC,
            },
            MqttMessage, MqttReceiveMessage,
        },
    };

    use super::GALACTIC_UNICORN;

    static CHANGE_COLOR_CHANNEL: PubSubChannel<ThreadModeRawMutex, Rgb888, 1, 2, 1> =
        PubSubChannel::new();
    pub static CURRENT_COLOR: Mutex<ThreadModeRawMutex, Rgb888> = Mutex::new(Rgb888::CSS_PURPLE);
    static CURRENT_GRAPHICS: Mutex<ThreadModeRawMutex, Option<UnicornGraphics<WIDTH, HEIGHT>>> =
        Mutex::new(None);

    static INTERRUPT_DISPLAY_CHANNEL: Channel<ThreadModeRawMutex, DisplayMessage, 1> =
        Channel::new();
    static MQTT_DISPLAY_CHANNEL: Channel<ThreadModeRawMutex, DisplayMessage, 16> = Channel::new();
    static APP_DISPLAY_CHANNEL: Channel<ThreadModeRawMutex, DisplayMessage, 16> = Channel::new();

    pub static STOP_CURRENT_DISPLAY: Signal<ThreadModeRawMutex, bool> = Signal::new();

    enum DisplayChannels {
        MQTT,
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
        duration: Option<Duration>,
        first_shown: Option<Instant>,
        channel: DisplayChannels,
    }

    impl DisplayGraphicsMessage {
        pub fn from_app(
            pixels: UnicornGraphicsPixels<WIDTH, HEIGHT>,
            duration: Option<Duration>,
        ) -> Self {
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
            if self.duration.is_none() {
                return true;
            }

            if self.first_shown.is_none() {
                return false;
            }

            self.first_shown.unwrap().elapsed() > self.duration.unwrap()
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
                DisplayChannels::APP => {
                    // clear channel
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

        send_brightness_state().await;
    }

    pub async fn send_brightness_state() {
        let brightness = GALACTIC_UNICORN.lock().await.as_ref().unwrap().brightness;

        let mut text = String::<3>::new();
        write!(text, "{brightness}").unwrap();

        MqttMessage::enqueue_state(BRIGHTNESS_STATE_TOPIC, &text).await;
    }

    pub async fn set_color(color: Rgb888) {
        let old_color = *CURRENT_COLOR.lock().await;
        *CURRENT_COLOR.lock().await = color;

        CURRENT_GRAPHICS
            .lock()
            .await
            .as_mut()
            .unwrap()
            .replace_color_with_new(old_color, color);

        CHANGE_COLOR_CHANNEL
            .publisher()
            .unwrap()
            .publish_immediate(color);

        send_color_state().await;
    }

    pub async fn send_color_state() {
        let color = *CURRENT_COLOR.lock().await;
        let r = color.r();
        let g = color.g();
        let b = color.b();

        let mut text = String::<11>::new();
        write!(text, "{r},{g},{b}").unwrap();

        MqttMessage::enqueue_state(RGB_STATE_TOPIC, &text).await;
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

        graphics.set_pixels(message.pixels);
        set_graphics(graphics).await;

        loop {
            if message.has_min_duration_passed() || STOP_CURRENT_DISPLAY.signaled() {
                STOP_CURRENT_DISPLAY.reset();
                break;
            } else {
                Timer::after_millis(1).await;
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

                x += 0.05;
                Timer::after_millis(1).await;
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
        mut subscriber: Subscriber<'static, ThreadModeRawMutex, MqttReceiveMessage, 8, 1, 1>,
    ) {
        loop {
            let message = subscriber.next_message_pure().await;

            if message.topic == BRIGHTNESS_SET_TOPIC {
                let brightness: u8 = match message.body.parse() {
                    Ok(value) => value,
                    Err(_) => 255,
                };
                set_brightness(brightness).await;
            } else if message.topic == RGB_SET_TOPIC {
                let mut r = String::<3>::new();
                let mut g = String::<3>::new();
                let mut b = String::<3>::new();

                let mut r_compl = false;
                let mut g_compl = false;
                let mut b_compl = false;
                for c in message.body.chars() {
                    if !r_compl {
                        if c == ',' {
                            r_compl = true;
                        } else {
                            write!(r, "{c}").unwrap();
                        }

                        continue;
                    }

                    if !g_compl {
                        if c == ',' {
                            g_compl = true;
                        } else {
                            write!(g, "{c}").unwrap();
                        }

                        continue;
                    }

                    if !b_compl {
                        if c == ',' {
                            b_compl = true;
                        } else {
                            write!(b, "{c}").unwrap();
                        }

                        continue;
                    }
                }

                let r = r.parse::<u8>().unwrap_or_default();
                let g = g.parse::<u8>().unwrap_or_default();
                let b = b.parse::<u8>().unwrap_or_default();

                set_color(Rgb888::new(r, g, b)).await;
            }
        }
    }
}
