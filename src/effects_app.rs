use embassy_futures::select::{select3, Either3};
use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, mutex::Mutex, signal::Signal};
use embassy_time::{Instant, Timer};
use embedded_graphics::{geometry::Point, pixelcolor::Rgb888};
use static_cell::make_static;

use crate::{
    app::{
        AppNotificationPolicy, AppRunner, AppRunnerInboxSubscribers, UnicornApp, UnicornAppRunner,
    },
    display::{GraphicsBufferWriter, HEIGHT, WIDTH},
};

/// All the effects that can be displayed.
#[derive(Clone, Copy)]
pub enum Effects {
    /// The balls/fire effect.
    Balls,
    /// Rainbow cycling effect.
    Rainbow,
    /// Matrix-style rain effect.
    Matrix,
}

impl Effects {
    /// Get the next effect in the cycle
    pub fn next(self) -> Self {
        match self {
            Effects::Balls => Effects::Rainbow,
            Effects::Rainbow => Effects::Matrix,
            Effects::Matrix => Effects::Balls,
        }
    }
}

/// Effects app. Show different effects.
pub struct EffectsApp {
    /// The current active effect.
    active_effect: Mutex<ThreadModeRawMutex, Effects>,
}

impl EffectsApp {
    /// Create the static ref to effects app.
    /// Must only be called once or will panic.
    pub fn new() -> &'static Self {
        make_static!(Self {
            active_effect: Mutex::new(Effects::Balls),
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
    inbox: AppRunnerInboxSubscribers,
    notification_policy: Signal<ThreadModeRawMutex, AppNotificationPolicy>,
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
        }
    }

    /// Run the balls/fire effect
    async fn run_balls_effect(&mut self) {
        let mut heat = [[0.0f32; 13]; 53];

        loop {
            // Update and draw heat map
            {
                let mut pixels = self.graphics_buffer.pixels_mut().await;

                for y in 0..HEIGHT {
                    for x in 0..WIDTH {
                        let coord = Point {
                            x: x as i32,
                            y: y as i32,
                        };

                        if heat[x][y] > 0.5 {
                            let color = Rgb888::new(255, 255, 180);
                            pixels.set_pixel(coord, color);
                        } else if heat[x][y] > 0.4 {
                            let color = Rgb888::new(220, 160, 0);
                            pixels.set_pixel(coord, color);
                        } else if heat[x][y] > 0.3 {
                            let color = Rgb888::new(180, 50, 0);
                            pixels.set_pixel(coord, color);
                        } else if heat[x][y] > 0.2 {
                            let color = Rgb888::new(40, 40, 40);
                            pixels.set_pixel(coord, color);
                        } else {
                            pixels.set_pixel(coord, Rgb888::new(0, 0, 0));
                        }

                        // Update this pixel by averaging the below pixels
                        if x == 0 {
                            heat[x][y] =
                                (heat[x][y] + heat[x][y + 2] + heat[x][y + 1] + heat[x + 1][y + 1])
                                    / 4.0;
                        } else if x == 52 {
                            heat[x][y] =
                                (heat[x][y] + heat[x][y + 2] + heat[x][y + 1] + heat[x - 1][y + 1])
                                    / 4.0;
                        } else {
                            heat[x][y] = (heat[x][y]
                                + heat[x][y + 2]
                                + heat[x][y + 1]
                                + heat[x - 1][y + 1]
                                + heat[x + 1][y + 1])
                                / 5.0;
                        }

                        heat[x][y] -= 0.01;
                        heat[x][y] = heat[x][y].max(0.0);
                    }
                }

                // Mark dirty while still holding lock
                pixels.mark_all_dirty();
            }

            // Clear the bottom row and then add a new fire seed to it
            for x in 0..WIDTH {
                heat[x][HEIGHT] = 0.0;
            }

            // Add a new random heat source
            for _ in 0..5 {
                let ticks = Instant::now().as_ticks();
                let px: usize = ticks as usize % 51 + 1;
                heat[px][HEIGHT] = 1.0;
                heat[px + 1][HEIGHT] = 1.0;
                heat[px - 1][HEIGHT] = 1.0;
                heat[px][HEIGHT + 1] = 1.0;
                heat[px + 1][HEIGHT + 1] = 1.0;
                heat[px - 1][HEIGHT + 1] = 1.0;
            }

            self.graphics_buffer.send();

            // Wait for timer or button press
            match select3(
                Timer::after_millis(50),
                self.inbox.buttons.next_message_pure(),
                self.inbox.mqtt.next_message_pure(),
            )
            .await
            {
                Either3::First(_) => { /* Timer expired, continue animation */ }
                Either3::Second(_) => {
                    // Button pressed - cycle to next effect
                    let current = self.state.get_active_effect().await;
                    self.state.set_active_effect(current.next()).await;
                    return; // Exit this effect loop
                }
                Either3::Third(_msg) => {
                    // MQTT message - could handle effect changes here if needed
                }
            }
        }
    }

    /// Run the rainbow effect
    async fn run_rainbow_effect(&mut self) {
        let mut frame = 0u32;

        loop {
            {
                let mut pixels = self.graphics_buffer.pixels_mut().await;

                for y in 0..HEIGHT {
                    for x in 0..WIDTH {
                        let coord = Point {
                            x: x as i32,
                            y: y as i32,
                        };

                        // Calculate rainbow hue based on position and frame (faster animation)
                        let hue = ((x + y + (frame * 2) as usize) % 256) as f32 / 256.0;
                        let color = Self::hsv_to_rgb(hue, 1.0, 1.0);
                        pixels.set_pixel(coord, color);
                    }
                }

                pixels.mark_all_dirty();
            }

            self.graphics_buffer.send();
            frame = frame.wrapping_add(1);

            // Wait for timer or button press
            match select3(
                Timer::after_millis(50),
                self.inbox.buttons.next_message_pure(),
                self.inbox.mqtt.next_message_pure(),
            )
            .await
            {
                Either3::First(_) => { /* Timer expired, continue animation */ }
                Either3::Second(_) => {
                    let current = self.state.get_active_effect().await;
                    self.state.set_active_effect(current.next()).await;
                    return;
                }
                Either3::Third(_msg) => {}
            }
        }
    }

    /// Run the matrix rain effect
    async fn run_matrix_effect(&mut self) {
        let mut frame = 0u32;
        // Each column tracks its current Y position (starts at different offsets)
        let mut matrix_cols: [i16; 53] = core::array::from_fn(|i| -(i as i16 % 11));

        loop {
            {
                let mut pixels = self.graphics_buffer.pixels_mut().await;

                // Clear screen
                for y in 0..HEIGHT {
                    for x in 0..WIDTH {
                        let coord = Point {
                            x: x as i32,
                            y: y as i32,
                        };
                        pixels.set_pixel(coord, Rgb888::new(0, 0, 0));
                    }
                }

                // Update and draw falling columns
                for x in 0..WIDTH {
                    let y = matrix_cols[x];

                    // Draw column head in bright green
                    if y >= 0 && y < HEIGHT as i16 {
                        let coord = Point {
                            x: x as i32,
                            y: y as i32,
                        };
                        pixels.set_pixel(coord, Rgb888::new(0, 255, 0));
                    }

                    // Draw trail in darker green (above the head)
                    for trail in 1..6 {
                        let trail_y = y - trail;
                        if trail_y >= 0 && trail_y < HEIGHT as i16 {
                            let coord = Point {
                                x: x as i32,
                                y: trail_y as i32,
                            };
                            let brightness = 200 - (trail * 35);
                            if brightness > 0 {
                                pixels.set_pixel(coord, Rgb888::new(0, brightness as u8, 0));
                            }
                        }
                    }

                    // Move column down every frame
                    matrix_cols[x] += 1;

                    // Reset column when it goes off screen (with some delay)
                    if matrix_cols[x] > HEIGHT as i16 + 5 {
                        // Stagger the restart times based on column
                        matrix_cols[x] = -((frame % 20) as i16 + (x as i16 % 10));
                    }
                }

                pixels.mark_all_dirty();
            }

            self.graphics_buffer.send();
            frame = frame.wrapping_add(1);

            // Wait for timer or button press
            match select3(
                Timer::after_millis(50),
                self.inbox.buttons.next_message_pure(),
                self.inbox.mqtt.next_message_pure(),
            )
            .await
            {
                Either3::First(_) => { /* Timer expired, continue animation */ }
                Either3::Second(_) => {
                    let current = self.state.get_active_effect().await;
                    self.state.set_active_effect(current.next()).await;
                    return;
                }
                Either3::Third(_msg) => {}
            }
        }
    }

    /// Helper: Convert HSV to RGB
    fn hsv_to_rgb(mut h: f32, s: f32, v: f32) -> Rgb888 {
        // Wrap h to 0-1 range
        while h >= 1.0 {
            h -= 1.0;
        }
        while h < 0.0 {
            h += 1.0;
        }
        let i = (h * 6.0) as u8;
        let f = h * 6.0 - i as f32;
        let p = v * (1.0 - s);
        let q = v * (1.0 - f * s);
        let t = v * (1.0 - (1.0 - f) * s);

        let (r, g, b) = match i {
            0 => (v, t, p),
            1 => (q, v, p),
            2 => (p, v, t),
            3 => (p, q, v),
            4 => (t, p, v),
            _ => (v, p, q),
        };

        Rgb888::new((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
    }
}

impl UnicornAppRunner for EffectsAppRunner {
    async fn run(&mut self) -> ! {
        // Signal that this app is happy to be interrupted at all times
        self.notification_policy
            .signal(AppNotificationPolicy::AllowAll);

        loop {
            let effect = self.state.get_active_effect().await;

            // Each effect runs its own loop and returns when button is pressed
            match effect {
                Effects::Balls => {
                    self.run_balls_effect().await;
                }
                Effects::Rainbow => {
                    self.run_rainbow_effect().await;
                }
                Effects::Matrix => {
                    self.run_matrix_effect().await;
                }
            }
            // Effect changed, loop will restart with new effect
        }
    }

    fn release_writer(self) -> GraphicsBufferWriter {
        self.graphics_buffer
    }
}
