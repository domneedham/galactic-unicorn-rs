use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, signal::Signal};
use embassy_time::{Instant, Timer};
use embedded_graphics::{
    geometry::Point,
    pixelcolor::{Rgb888, WebColors},
    primitives::{Circle, Primitive, PrimitiveStyleBuilder},
    Drawable,
};
use micromath::F32Ext;
use static_cell::make_static;

use crate::{
    app::{AppNotificationPolicy, AppRunner, AppRunnerInboxSubscribers, UnicornApp, UnicornAppRunner},
    display::{DisplayState, GraphicsBufferWriter, HEIGHT},
};

/// System app. Shows a loading animation.
pub struct SystemApp {
    display_state: &'static DisplayState,
}

impl SystemApp {
    /// Create the static ref to system app.
    /// Must only be called once or will panic.
    pub fn new(display_state: &'static DisplayState) -> &'static Self {
        make_static!(Self { display_state })
    }
}

impl UnicornApp for SystemApp {
    async fn create_runner(
        &'static self,
        graphics_buffer: GraphicsBufferWriter,
        inbox: AppRunnerInboxSubscribers,
        notification_policy: Signal<ThreadModeRawMutex, AppNotificationPolicy>,
    ) -> AppRunner {
        AppRunner::System(SystemAppRunner::new(
            graphics_buffer,
            self,
            inbox,
            notification_policy,
        ))
    }
}

/// Runner for the system app. Shows a loading animation.
pub struct SystemAppRunner {
    graphics_buffer: GraphicsBufferWriter,
    state: &'static SystemApp,
    #[allow(dead_code)]
    inbox: AppRunnerInboxSubscribers,
    notification_policy: Signal<ThreadModeRawMutex, AppNotificationPolicy>,
}

impl<'a> SystemAppRunner {
    pub fn new(
        graphics_buffer: GraphicsBufferWriter,
        state: &'static SystemApp,
        inbox: AppRunnerInboxSubscribers,
        notification_policy: Signal<ThreadModeRawMutex, AppNotificationPolicy>,
    ) -> Self {
        Self {
            graphics_buffer,
            state,
            inbox,
            notification_policy,
        }
    }
}

const MAX_POSITION: f32 = (HEIGHT as i32 - 5) as f32;

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn ease_in(t: f32) -> f32 {
    t * t * t
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t) * (1.0 - t)
}

impl UnicornAppRunner for SystemAppRunner {
    async fn run(&mut self) -> ! {
        // Signal that this app is happy to be interrupted at all times
        self.notification_policy.signal(AppNotificationPolicy::AllowAll);

        let mut color_sub = self.state.display_state.color.receiver().unwrap();
        const ANIMATION_DURATION: f32 = 600.0;
        let mut start_time = Instant::now();
        let mut min_value = 0.0;
        let mut max_value = MAX_POSITION;

        loop {
            let color = color_sub.try_get().unwrap_or(Rgb888::CSS_PURPLE);

            let style = PrimitiveStyleBuilder::new()
                .fill_color(color)
                .build();

            {
                let mut pixels = self.graphics_buffer.pixels_mut().await;
                pixels.clear_all();

                let elapsed_millis = start_time.elapsed().as_millis() as f32;
                let progress = (elapsed_millis / ANIMATION_DURATION).min(1.0);

                // Left circle with ease_in
                let eased_progress = ease_in(progress);
                let animated_value = lerp(min_value, max_value, eased_progress);
                Circle::new(Point::new(10, animated_value.floor() as i32), 5)
                    .into_styled(style)
                    .draw(&mut *pixels)
                    .unwrap();

                // Center circle with linear easing
                let animated_value = lerp(min_value, max_value, progress);
                Circle::new(Point::new(24, animated_value.floor() as i32), 5)
                    .into_styled(style)
                    .draw(&mut *pixels)
                    .unwrap();

                // Right circle with ease_out
                let eased_progress = ease_out(progress);
                let animated_value = lerp(min_value, max_value, eased_progress);
                Circle::new(Point::new(38, animated_value.floor() as i32), 5)
                    .into_styled(style)
                    .draw(&mut *pixels)
                    .unwrap();

                // Mark entire screen as dirty after drawing
                pixels.mark_all_dirty();
            }

            self.graphics_buffer.send();

            // Check if animation cycle is complete
            if start_time.elapsed().as_millis() as f32 >= ANIMATION_DURATION {
                Timer::after_millis(25).await;
                start_time = Instant::now();

                // Reverse direction
                if max_value == MAX_POSITION {
                    max_value = 0.0;
                    min_value = MAX_POSITION + 0.1; // 0.1 stops jitter on reverse animation
                } else {
                    max_value = MAX_POSITION;
                    min_value = 0.0;
                }
            }

            Timer::after_millis(10).await;
        }
    }

    fn release_writer(self) -> GraphicsBufferWriter {
        self.graphics_buffer
    }
}
