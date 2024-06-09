use embassy_rp::peripherals::{DMA_CH0, PIO0};
use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, mutex::Mutex};
use galactic_unicorn_embassy::{pins::UnicornDisplayPins, GalacticUnicorn};

use crate::mqtt::MqttMessage;

type GalacticUnicornType = Mutex<ThreadModeRawMutex, Option<GalacticUnicorn>>;
static GALACTIC_UNICORN: GalacticUnicornType = Mutex::new(None);

pub async fn init(pio: PIO0, dma: DMA_CH0, pins: UnicornDisplayPins) {
    let gu = GalacticUnicorn::new(pio, pins, dma);
    GALACTIC_UNICORN.lock().await.replace(gu);
    MqttMessage::debug("Initialised display").send().await;
}

pub mod display {
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
        mqtt::{MqttMessage, MqttReceiveMessage, BRIGHTNESS_TOPIC, COLOR_TOPIC},
    };

    use super::{utils, GALACTIC_UNICORN};

    static CHANGE_COLOR_CHANNEL: PubSubChannel<ThreadModeRawMutex, Rgb888, 1, 2, 1> =
        PubSubChannel::new();
    pub static CURRENT_COLOR: Mutex<ThreadModeRawMutex, Rgb888> = Mutex::new(Rgb888::CSS_PURPLE);
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
        let text = utils::brightness_to_str(brightness);

        MqttMessage::new("display/brightness/state", text, true)
            .send()
            .await;
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
        mut subscriber: Subscriber<'static, ThreadModeRawMutex, MqttReceiveMessage, 16, 1, 1>,
    ) {
        loop {
            let message = subscriber.next_message_pure().await;

            if message.topic.contains(BRIGHTNESS_TOPIC) {
                let brightness: u8 = match message.body.parse() {
                    Ok(value) => value,
                    Err(_) => 255,
                };
                set_brightness(brightness).await;
            } else if message.topic.contains(COLOR_TOPIC) {
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

mod utils {
    const U8_STRINGS: [&str; 256] = [
        "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16",
        "17", "18", "19", "20", "21", "22", "23", "24", "25", "26", "27", "28", "29", "30", "31",
        "32", "33", "34", "35", "36", "37", "38", "39", "40", "41", "42", "43", "44", "45", "46",
        "47", "48", "49", "50", "51", "52", "53", "54", "55", "56", "57", "58", "59", "60", "61",
        "62", "63", "64", "65", "66", "67", "68", "69", "70", "71", "72", "73", "74", "75", "76",
        "77", "78", "79", "80", "81", "82", "83", "84", "85", "86", "87", "88", "89", "90", "91",
        "92", "93", "94", "95", "96", "97", "98", "99", "100", "101", "102", "103", "104", "105",
        "106", "107", "108", "109", "110", "111", "112", "113", "114", "115", "116", "117", "118",
        "119", "120", "121", "122", "123", "124", "125", "126", "127", "128", "129", "130", "131",
        "132", "133", "134", "135", "136", "137", "138", "139", "140", "141", "142", "143", "144",
        "145", "146", "147", "148", "149", "150", "151", "152", "153", "154", "155", "156", "157",
        "158", "159", "160", "161", "162", "163", "164", "165", "166", "167", "168", "169", "170",
        "171", "172", "173", "174", "175", "176", "177", "178", "179", "180", "181", "182", "183",
        "184", "185", "186", "187", "188", "189", "190", "191", "192", "193", "194", "195", "196",
        "197", "198", "199", "200", "201", "202", "203", "204", "205", "206", "207", "208", "209",
        "210", "211", "212", "213", "214", "215", "216", "217", "218", "219", "220", "221", "222",
        "223", "224", "225", "226", "227", "228", "229", "230", "231", "232", "233", "234", "235",
        "236", "237", "238", "239", "240", "241", "242", "243", "244", "245", "246", "247", "248",
        "249", "250", "251", "252", "253", "254", "255",
    ];

    pub(crate) fn brightness_to_str(value: u8) -> &'static str {
        U8_STRINGS[value as usize]
    }
}
