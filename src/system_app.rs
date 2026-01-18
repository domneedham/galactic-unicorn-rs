use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, signal::Signal};
use embassy_time::Timer;
use embedded_graphics::{
    geometry::Point,
    pixelcolor::{Rgb888, RgbColor, WebColors},
};
use static_cell::make_static;

use crate::{
    app::{AppNotificationPolicy, AppRunner, AppRunnerInboxSubscribers, UnicornApp, UnicornAppRunner},
    display::{DisplayState, GraphicsBufferWriter, HEIGHT, WIDTH},
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
    #[allow(dead_code)]
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

impl UnicornAppRunner for SystemAppRunner {
    async fn run(&mut self) -> ! {
        let mut color_sub = self.state.display_state.color.receiver().unwrap();
        let mut position: i32 = 0;
        let bar_width: i32 = 10;

        loop {
            let color = color_sub.try_get().unwrap_or(Rgb888::CSS_PURPLE);

            {
                let mut pixels = self.graphics_buffer.pixels_mut().await;
                pixels.clear_all();

                // Draw a moving bar animation
                for x in 0..bar_width {
                    let draw_x = (position + x) % (WIDTH as i32 + bar_width);
                    if draw_x >= 0 && draw_x < WIDTH as i32 {
                        for y in 0..HEIGHT as i32 {
                            // Fade effect based on position in bar
                            let intensity = ((bar_width - x) as f32 / bar_width as f32 * 255.0) as u8;
                            let fade_color = Rgb888::new(
                                (color.r() as u16 * intensity as u16 / 255) as u8,
                                (color.g() as u16 * intensity as u16 / 255) as u8,
                                (color.b() as u16 * intensity as u16 / 255) as u8,
                            );
                            pixels.set_pixel(Point::new(draw_x, y), fade_color);
                        }
                    }
                }
            }

            self.graphics_buffer.send();

            position = (position + 1) % (WIDTH as i32 + bar_width);
            Timer::after_millis(30).await;
        }
    }

    fn release_writer(self) -> GraphicsBufferWriter {
        self.graphics_buffer
    }
}
