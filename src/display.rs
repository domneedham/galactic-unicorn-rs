use core::fmt::Write;
use embassy_executor::Spawner;
use embassy_futures::select::{select, Either};
use embassy_rp::peripherals::{DMA_CH0, PIO0};
use embassy_sync::{
    blocking_mutex::raw::ThreadModeRawMutex,
    channel::Channel,
    mutex::Mutex,
    pubsub::{PubSubChannel, Subscriber},
    signal::Signal,
};
use embassy_time::Timer;

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
use galactic_unicorn_embassy::{pins::UnicornDisplayPins, GalacticUnicorn, HEIGHT, WIDTH};
use heapless::String;
use messages::{DisplayGraphicsMessage, DisplayMessage, DisplayTextMessage};
use static_cell::make_static;
use unicorn_graphics::UnicornGraphics;

use crate::{
    buttons::{self, BRIGHTNESS_DOWN_PRESS, BRIGHTNESS_UP_PRESS},
    mqtt::{
        topics::{BRIGHTNESS_SET_TOPIC, BRIGHTNESS_STATE_TOPIC, RGB_SET_TOPIC, RGB_STATE_TOPIC},
        MqttMessage, MqttReceiveMessage,
    },
};

/// Channel for color changes to be published into.
static CHANGE_COLOR_CHANNEL: PubSubChannel<ThreadModeRawMutex, Rgb888, 1, 2, 1> =
    PubSubChannel::new();

/// Channel for display message that will interrupt anything on the display.
static INTERRUPT_DISPLAY_CHANNEL: Channel<ThreadModeRawMutex, DisplayMessage, 1> = Channel::new();

/// Channel for messages from MQTT.
static MQTT_DISPLAY_CHANNEL: Channel<ThreadModeRawMutex, DisplayMessage, 8> = Channel::new();

/// Channel for messages from apps.
static APP_DISPLAY_CHANNEL: Channel<ThreadModeRawMutex, DisplayMessage, 8> = Channel::new();

/// Signal for stopping the display message, ready for the next one.
pub static STOP_CURRENT_DISPLAY: Signal<ThreadModeRawMutex, bool> = Signal::new();

/// Galactic unicorn display.
pub struct Display {
    /// The galactic unicorn board core.
    galactic_unicorn: Mutex<ThreadModeRawMutex, GalacticUnicorn>,

    /// The current graphics being displayed.
    current_graphics: Mutex<ThreadModeRawMutex, UnicornGraphics<WIDTH, HEIGHT>>,

    /// The current active color.
    current_color: Mutex<ThreadModeRawMutex, Rgb888>,
}

impl Display {
    /// Create the static ref to display.
    /// Must only be called once or will panic.
    pub fn new(
        pio: PIO0,
        dma: DMA_CH0,
        pins: UnicornDisplayPins,
        spawner: Spawner,
    ) -> &'static Self {
        let display = make_static!(Self {
            galactic_unicorn: Mutex::new(GalacticUnicorn::new(pio, pins, dma)),
            current_graphics: Mutex::new(UnicornGraphics::new()),
            current_color: Mutex::new(Rgb888::CSS_PURPLE),
        });

        spawner.spawn(process_display_queue_task(display)).unwrap();
        spawner
            .spawn(process_brightness_buttons_task(display))
            .unwrap();

        display
    }

    /// Get the current brightness of the display.
    pub async fn get_brightness(&'static self) -> u8 {
        self.galactic_unicorn.lock().await.brightness
    }

    /// Set the brightness on the display and send the state over MQTT.
    pub async fn set_brightness(&'static self, brightness: u8) {
        self.galactic_unicorn.lock().await.brightness = brightness;
        self.redraw_graphics().await;

        self.send_brightness_state().await;
    }

    /// Send the current brightness state over MQTT.
    pub async fn send_brightness_state(&'static self) {
        let brightness = self.galactic_unicorn.lock().await.brightness;

        let mut text = String::<3>::new();
        write!(text, "{brightness}").unwrap();

        MqttMessage::enqueue_state(BRIGHTNESS_STATE_TOPIC, &text).await;
    }

    /// Get the current active color.
    pub async fn get_color(&'static self) -> Rgb888 {
        *self.current_color.lock().await
    }

    /// Set the color on the display and send the state over MQTT.
    pub async fn set_color(&'static self, color: Rgb888) {
        let old_color = *self.current_color.lock().await;
        *self.current_color.lock().await = color;

        self.current_graphics
            .lock()
            .await
            .replace_color_with_new(old_color, color);

        CHANGE_COLOR_CHANNEL
            .publisher()
            .unwrap()
            .publish_immediate(color);

        self.send_color_state().await;
    }

    /// Send the current color state over MQTT.
    pub async fn send_color_state(&'static self) {
        let color = *self.current_color.lock().await;
        let r = color.r();
        let g = color.g();
        let b = color.b();

        let mut text = String::<11>::new();
        write!(text, "{r},{g},{b}").unwrap();

        MqttMessage::enqueue_state(RGB_STATE_TOPIC, &text).await;
    }

    /// Set the current graphics being displayed.
    pub async fn set_graphics(&'static self, graphics: &UnicornGraphics<WIDTH, HEIGHT>) {
        self.galactic_unicorn.lock().await.set_pixels(graphics);
        *self.current_graphics.lock().await = *graphics;
    }

    /// Redraw the current graphics being displayed.
    pub async fn redraw_graphics(&'static self) {
        self.galactic_unicorn
            .lock()
            .await
            .set_pixels(&*self.current_graphics.lock().await);
    }

    /// Display a graphical message. Has a minimum of 1ms on the display.
    async fn display_graphics_message(
        &'static self,
        graphics: &mut UnicornGraphics<WIDTH, HEIGHT>,
        message: &mut DisplayGraphicsMessage,
    ) {
        graphics.set_pixels(message.pixels);
        self.set_graphics(graphics).await;

        message.set_first_shown();

        Timer::after_millis(1).await;

        loop {
            if message.has_min_duration_passed() || STOP_CURRENT_DISPLAY.signaled() {
                STOP_CURRENT_DISPLAY.reset();
                break;
            } else {
                Timer::after_millis(1).await;
            }
        }
    }

    /// Display a text message on the display.
    /// Will scroll the text if it exceeds the width, otherwise will center the text.
    async fn display_text_message(
        &'static self,
        graphics: &mut UnicornGraphics<WIDTH, HEIGHT>,
        message: &mut DisplayTextMessage,
    ) {
        let color = match message.color {
            Some(x) => x,
            None => self.get_color().await,
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
                self.set_graphics(graphics).await;

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
            self.set_graphics(graphics).await;

            loop {
                Timer::after_millis(10).await;

                if message.has_min_duration_passed() || STOP_CURRENT_DISPLAY.signaled() {
                    STOP_CURRENT_DISPLAY.reset();
                    break;
                }
            }
        }
    }
}

/// Process the display queues.
/// Queues are prioritised by:
/// - Interrupt channel
/// - MQTT channel
/// - App channel
#[embassy_executor::task]
async fn process_display_queue_task(display: &'static Display) {
    let mut graphics = UnicornGraphics::new();
    let mut message: Option<DisplayMessage> = None;

    let mut color_subscriber = CHANGE_COLOR_CHANNEL.subscriber().unwrap();

    let mut is_message_replaced = false;

    loop {
        match INTERRUPT_DISPLAY_CHANNEL.try_receive() {
            Ok(value) => match value {
                DisplayMessage::Graphics(mut value) => {
                    display
                        .display_graphics_message(&mut graphics, &mut value)
                        .await;
                }
                DisplayMessage::Text(mut value) => {
                    display
                        .display_text_message(&mut graphics, &mut value)
                        .await;
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
                    display.display_graphics_message(&mut graphics, value).await;
                }
                DisplayMessage::Text(value) => {
                    // replace color in message if needed
                    if !is_message_replaced {
                        match color_subscriber.try_next_message_pure() {
                            Some(color) => value.color = Some(color),
                            None => {}
                        }
                    }

                    display.display_text_message(&mut graphics, value).await;
                }
            }

            is_message_replaced = false;
        } else {
            Timer::after_millis(200).await;
        }
    }
}

/// Process any brightness button presses and update the display.
#[embassy_executor::task]
async fn process_brightness_buttons_task(display: &'static Display) {
    loop {
        let press_type = select(BRIGHTNESS_UP_PRESS.wait(), BRIGHTNESS_DOWN_PRESS.wait()).await;

        let current_brightness = display.get_brightness().await;

        match press_type {
            Either::First(press) => match press {
                buttons::ButtonPress::Short => {
                    display
                        .set_brightness(current_brightness.saturating_add(10))
                        .await;
                }
                buttons::ButtonPress::Long => {
                    display.set_brightness(255).await;
                }
                buttons::ButtonPress::Double => {
                    display
                        .set_brightness(current_brightness.saturating_add(50))
                        .await
                }
            },
            Either::Second(press) => match press {
                buttons::ButtonPress::Short => {
                    display
                        .set_brightness(current_brightness.saturating_sub(10))
                        .await;
                }
                buttons::ButtonPress::Long => {
                    display.set_brightness(20).await;
                }
                buttons::ButtonPress::Double => {
                    display
                        .set_brightness(current_brightness.saturating_sub(50))
                        .await
                }
            },
        }
    }
}

/// Process MQTT messages related to the display.
#[embassy_executor::task]
pub async fn process_mqtt_messages_task(
    display: &'static Display,
    mut subscriber: Subscriber<'static, ThreadModeRawMutex, MqttReceiveMessage, 8, 1, 1>,
) {
    loop {
        let message = subscriber.next_message_pure().await;

        if message.topic == BRIGHTNESS_SET_TOPIC {
            let brightness: u8 = match message.body.parse() {
                Ok(value) => value,
                Err(_) => 255,
            };
            display.set_brightness(brightness).await;
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

            display.set_color(Rgb888::new(r, g, b)).await;
        }
    }
}

/// Message structs for sending into the display channels.
pub mod messages {
    use embassy_time::{Duration, Instant};
    use embedded_graphics::{geometry::Point, pixelcolor::Rgb888};
    use galactic_unicorn_embassy::{HEIGHT, WIDTH};
    use heapless::String;
    use unicorn_graphics::UnicornGraphicsPixels;

    use super::{
        APP_DISPLAY_CHANNEL, INTERRUPT_DISPLAY_CHANNEL, MQTT_DISPLAY_CHANNEL, STOP_CURRENT_DISPLAY,
    };

    /// Possible display channels.
    enum DisplayChannels {
        /// MQTT display channel.
        MQTT,

        /// App display channel.
        APP,
    }

    /// Types of message that can be displayed.
    pub(super) enum DisplayMessage {
        /// A graphics message that contains the pixel buffer.
        Graphics(DisplayGraphicsMessage),

        /// A text message that contains the text to be displayed.
        Text(DisplayTextMessage),
    }

    /// Show some text on the display. Has a 64 byte maximum size.
    pub struct DisplayTextMessage {
        /// The text to display.
        pub(super) text: String<64>,

        /// The color to display. If `None` will use the active color.
        pub(super) color: Option<Rgb888>,

        /// Where to start the text vertically.
        pub(super) point: Point,

        /// The minimum duration to show the text for.
        pub(super) duration: Duration,

        /// When the message was first shown on the display.
        pub(super) first_shown: Option<Instant>,

        /// What channel to publish the message into.
        channel: DisplayChannels,
    }

    impl DisplayTextMessage {
        /// Display a text message on the MQTT channel.
        /// A `None` for `color` will use the active color.
        /// A `None` for `point` will center the text.
        /// Shows for a minimum of 3 seconds.
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

        /// Display a text message on the app channel.
        /// A `None` for `color` will use the active color.
        /// A `None` for `point` will center the text.
        /// A `None` for `duration` will display the message for a minimum of 3 seconds.
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
        /// Queue a message into the end of the channel and consume itself.
        pub async fn send(self) {
            match self.channel {
                DisplayChannels::MQTT => {
                    MQTT_DISPLAY_CHANNEL.send(DisplayMessage::Text(self)).await
                }
                DisplayChannels::APP => APP_DISPLAY_CHANNEL.send(DisplayMessage::Text(self)).await,
            }
        }

        /// Queue a message into the channel, clearing anything before it.
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

        /// Show the text immediately, skipping the display channel queue.
        pub async fn send_and_show_now(self) {
            STOP_CURRENT_DISPLAY.signal(true);
            INTERRUPT_DISPLAY_CHANNEL
                .send(DisplayMessage::Text(self))
                .await;
        }
    }

    impl DisplayTextMessage {
        /// Set the text has being shown on the display.
        pub fn set_first_shown(&mut self) {
            if self.first_shown.is_none() {
                self.first_shown.replace(Instant::now());
            }
        }

        /// Check if the minimum duration of display has passed.
        pub fn has_min_duration_passed(&self) -> bool {
            if self.first_shown.is_none() {
                return false;
            }

            self.first_shown.unwrap().elapsed() > self.duration
        }
    }

    /// Show a message using the pixel buffer.
    pub struct DisplayGraphicsMessage {
        /// The pixel buffer that will be displayed.
        pub(super) pixels: UnicornGraphicsPixels<WIDTH, HEIGHT>,

        /// The minimum duration to show the message for.
        pub(super) duration: Duration,

        /// When the message was first shown on the display.
        pub(super) first_shown: Option<Instant>,

        /// What channel to publish the message into.
        channel: DisplayChannels,
    }

    impl DisplayGraphicsMessage {
        /// Display the pixels on the display for the duration specified.
        pub fn from_app(pixels: UnicornGraphicsPixels<WIDTH, HEIGHT>, duration: Duration) -> Self {
            Self {
                pixels,
                duration,
                first_shown: None,
                channel: DisplayChannels::APP,
            }
        }
    }

    impl DisplayGraphicsMessage {
        /// Set the text has being shown on the display.
        pub fn set_first_shown(&mut self) {
            if self.first_shown.is_none() {
                self.first_shown.replace(Instant::now());
            }
        }

        /// Check if the minimum duration of display has passed.
        pub fn has_min_duration_passed(&self) -> bool {
            if self.first_shown.is_none() {
                return false;
            }

            self.first_shown.unwrap().elapsed() > self.duration
        }
    }

    impl DisplayGraphicsMessage {
        /// Queue a message into the end of the channel and consume itself.
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

        /// Queue a message into the channel, clearing anything before it.
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
    }
}