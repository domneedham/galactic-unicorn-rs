use embassy_time::{Duration, Instant, Timer};
use embedded_graphics::{
    geometry::Point,
    pixelcolor::{Rgb888, WebColors},
    primitives::{Circle, Primitive, PrimitiveStyleBuilder},
};
use embedded_graphics_core::Drawable;
use galactic_unicorn_embassy::{HEIGHT, WIDTH};
use static_cell::make_static;
use unicorn_graphics::UnicornGraphics;

use crate::{
    app::UnicornApp, buttons::ButtonPress, display::messages::DisplayGraphicsMessage,
    mqtt::MqttReceiveMessage,
};

use micromath::F32Ext;

pub struct SystemApp;

impl SystemApp {
    /// Create the static ref to system app.
    /// Must only be called once or will panic.
    pub fn new() -> &'static Self {
        make_static!(Self {})
    }

    /// Linear interpolation function.
    /// It linearly interpolates between a and b based on the value of t.
    ///
    /// a: The starting value.
    /// b: The ending value.
    /// t: The interpolation factor (between 0.0 and 1.0).
    fn lerp(a: f32, b: f32, t: f32) -> f32 {
        a + (b - a) * t
    }

    /// Ease in cubic function.
    ///
    /// Cube the input to make a gradual increase.
    fn ease_in(t: f32) -> f32 {
        t * t * t
    }

    /// Ease out cubic function.
    ///
    /// Calculates 1 minus (1 minus t) squared, which provides a gradual decrease in easing from 1 to 0.
    fn ease_out(t: f32) -> f32 {
        1.0 - (1.0 - t) * (1.0 - t)
    }
}

impl UnicornApp for SystemApp {
    async fn display(&self) {
        const MAX_POSITION: f32 = (HEIGHT as i32 - 5) as f32;

        let mut graphics = UnicornGraphics::<WIDTH, HEIGHT>::new();

        let style = PrimitiveStyleBuilder::new()
            .fill_color(Rgb888::CSS_PURPLE)
            .build();

        const ANIMATION_DURATION: f32 = 600.0;
        let mut start_time = Instant::now();

        let mut min_value = 0.0;
        let mut max_value = MAX_POSITION;

        loop {
            graphics.clear_all();

            let elapsed_millis = start_time.elapsed().as_millis() as f32;

            let progress = (elapsed_millis / ANIMATION_DURATION).min(1.0);

            // left circle
            let eased_progress = Self::ease_in(progress);
            let animated_value = Self::lerp(min_value, max_value, eased_progress);
            Circle::new(Point::new(10, animated_value.floor() as i32), 5)
                .into_styled(style)
                .draw(&mut graphics)
                .unwrap();

            // center circle
            let animated_value = Self::lerp(min_value, max_value, progress);
            Circle::new(Point::new(24, animated_value.floor() as i32), 5)
                .into_styled(style)
                .draw(&mut graphics)
                .unwrap();

            // right circle
            let eased_progress = Self::ease_out(progress);
            let animated_value = Self::lerp(min_value, max_value, eased_progress);
            Circle::new(Point::new(38, animated_value.floor() as i32), 5)
                .into_styled(style)
                .draw(&mut graphics)
                .unwrap();

            DisplayGraphicsMessage::from_app(graphics.get_pixels(), Duration::from_millis(10))
                .send_and_replace_queue()
                .await;

            Timer::after_millis(10).await;

            if elapsed_millis >= ANIMATION_DURATION {
                Timer::after_millis(25).await;

                start_time = Instant::now();

                if max_value == MAX_POSITION {
                    max_value = 0.0;
                    min_value = MAX_POSITION + 0.1; // 0.1 stops jitter on reverse animation
                } else {
                    max_value = MAX_POSITION;
                    min_value = 0.0;
                }
            }
        }
    }

    async fn start(&self) {}

    async fn stop(&self) {}

    async fn button_press(&self, _: ButtonPress) {}

    async fn process_mqtt_message(&self, _: MqttReceiveMessage) {}

    async fn send_mqtt_state(&self) {}
}
