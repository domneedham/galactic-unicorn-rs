use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, mutex::Mutex, signal::Signal};
use embassy_time::{Instant, Timer};
use embedded_graphics::{geometry::Point, pixelcolor::Rgb888};
use static_cell::make_static;

use crate::{
    app::{AppNotificationPolicy, AppRunner, AppRunnerInboxSubscribers, UnicornApp, UnicornAppRunner},
    display::{GraphicsBufferWriter, HEIGHT, WIDTH},
};

/// All the effects that can be displayed.
#[derive(Clone, Copy)]
pub enum Effects {
    /// The balls/fire effect.
    Balls,
}

/// Effects app. Show different effects.
pub struct EffectsApp {
    /// The current active effect.
    active_effect: Mutex<ThreadModeRawMutex, Effects>,

    /// Signal for swapping effects on a button press.
    swap_effect: Signal<ThreadModeRawMutex, bool>,
}

impl EffectsApp {
    /// Create the static ref to effects app.
    /// Must only be called once or will panic.
    pub fn new() -> &'static Self {
        make_static!(Self {
            active_effect: Mutex::new(Effects::Balls),
            swap_effect: Signal::new(),
        })
    }

    /// Get the current active effect.
    pub async fn get_active_effect(&self) -> Effects {
        *self.active_effect.lock().await
    }

    /// Set the active effect.
    pub async fn set_active_effect(&self, effect: Effects) {
        *self.active_effect.lock().await = effect;
    }
}

impl UnicornApp for EffectsApp {
    async fn create_runner(
        &'static self,
        graphics_buffer: GraphicsBufferWriter,
        inbox: AppRunnerInboxSubscribers,
        notification_policy: Signal<ThreadModeRawMutex, AppNotificationPolicy>,
    ) -> AppRunner {
        AppRunner::Effects(EffectsAppRunner::new(
            graphics_buffer,
            self,
            inbox,
            notification_policy,
        ))
    }
}

/// Runner for the effects app.
pub struct EffectsAppRunner {
    graphics_buffer: GraphicsBufferWriter,
    state: &'static EffectsApp,
    #[allow(dead_code)]
    inbox: AppRunnerInboxSubscribers,
    #[allow(dead_code)]
    notification_policy: Signal<ThreadModeRawMutex, AppNotificationPolicy>,
    /// Heat map for the fire/balls effect
    heat: [[f32; 13]; 53],
}

impl<'a> EffectsAppRunner {
    pub fn new(
        graphics_buffer: GraphicsBufferWriter,
        state: &'static EffectsApp,
        inbox: AppRunnerInboxSubscribers,
        notification_policy: Signal<ThreadModeRawMutex, AppNotificationPolicy>,
    ) -> Self {
        Self {
            graphics_buffer,
            state,
            inbox,
            notification_policy,
            heat: [[0.0; 13]; 53],
        }
    }

    /// Run the balls/fire effect
    async fn run_balls_effect(&mut self) {
        // Update and draw heat map
        {
            let mut pixels = self.graphics_buffer.pixels_mut().await;

            for y in 0..HEIGHT {
                for x in 0..WIDTH {
                    let coord = Point {
                        x: x as i32,
                        y: y as i32,
                    };

                    if self.heat[x][y] > 0.5 {
                        let color = Rgb888::new(255, 255, 180);
                        pixels.set_pixel(coord, color);
                    } else if self.heat[x][y] > 0.4 {
                        let color = Rgb888::new(220, 160, 0);
                        pixels.set_pixel(coord, color);
                    } else if self.heat[x][y] > 0.3 {
                        let color = Rgb888::new(180, 50, 0);
                        pixels.set_pixel(coord, color);
                    } else if self.heat[x][y] > 0.2 {
                        let color = Rgb888::new(40, 40, 40);
                        pixels.set_pixel(coord, color);
                    } else {
                        pixels.set_pixel(coord, Rgb888::new(0, 0, 0));
                    }

                    // Update this pixel by averaging the below pixels
                    if x == 0 {
                        self.heat[x][y] = (self.heat[x][y]
                            + self.heat[x][y + 2]
                            + self.heat[x][y + 1]
                            + self.heat[x + 1][y + 1])
                            / 4.0;
                    } else if x == 52 {
                        self.heat[x][y] = (self.heat[x][y]
                            + self.heat[x][y + 2]
                            + self.heat[x][y + 1]
                            + self.heat[x - 1][y + 1])
                            / 4.0;
                    } else {
                        self.heat[x][y] = (self.heat[x][y]
                            + self.heat[x][y + 2]
                            + self.heat[x][y + 1]
                            + self.heat[x - 1][y + 1]
                            + self.heat[x + 1][y + 1])
                            / 5.0;
                    }

                    self.heat[x][y] -= 0.01;
                    self.heat[x][y] = self.heat[x][y].max(0.0);
                }
            }
        }

        // Mark entire display as dirty since fire effect updates everything
        self.graphics_buffer.mark_all_dirty().await;

        // Clear the bottom row and then add a new fire seed to it
        for x in 0..WIDTH {
            self.heat[x][HEIGHT] = 0.0;
        }

        // Add a new random heat source
        for _ in 0..5 {
            let ticks = Instant::now().as_ticks();
            let px: usize = ticks as usize % 51 + 1;
            self.heat[px][HEIGHT] = 1.0;
            self.heat[px + 1][HEIGHT] = 1.0;
            self.heat[px - 1][HEIGHT] = 1.0;
            self.heat[px][HEIGHT + 1] = 1.0;
            self.heat[px + 1][HEIGHT + 1] = 1.0;
            self.heat[px - 1][HEIGHT + 1] = 1.0;
        }
    }
}

impl UnicornAppRunner for EffectsAppRunner {
    async fn run(&mut self) -> ! {
        loop {
            let effect = self.state.get_active_effect().await;

            match effect {
                Effects::Balls => {
                    self.run_balls_effect().await;
                }
            }

            self.graphics_buffer.send();
            Timer::after_millis(50).await;
        }
    }

    fn release_writer(self) -> GraphicsBufferWriter {
        self.graphics_buffer
    }
}
