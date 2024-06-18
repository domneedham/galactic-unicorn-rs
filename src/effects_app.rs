use embassy_futures::select::select;
use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, mutex::Mutex, signal::Signal};

use crate::{app::UnicornApp, buttons::ButtonPress};

use self::effects::{Balls, Effects};

pub struct EffectsApp {
    active_effect: Mutex<ThreadModeRawMutex, Effects>,
    swap_effect: Signal<ThreadModeRawMutex, bool>,
    balls: Balls,
}

impl EffectsApp {
    pub fn new() -> Self {
        Self {
            active_effect: Mutex::new(Effects::Balls),
            swap_effect: Signal::new(),
            balls: Balls::new(),
        }
    }
}

impl UnicornApp for EffectsApp {
    async fn display(&self) {
        loop {
            let active_app = *self.active_effect.lock().await;
            match active_app {
                Effects::Balls => select(self.balls.display(), self.swap_effect.wait()).await,
            };
        }
    }

    async fn start(&self) {}

    async fn stop(&self) {}

    async fn button_press(&self, _: ButtonPress) {
        let mut ae: embassy_sync::mutex::MutexGuard<'_, ThreadModeRawMutex, Effects> =
            self.active_effect.lock().await;
        let new_app = match *ae {
            Effects::Balls => Effects::Balls,
        };

        *ae = new_app;

        self.swap_effect.signal(true);
    }

    async fn process_mqtt_message(&self, _: crate::mqtt::MqttReceiveMessage) {}

    async fn send_state(&self) {}
}

mod effects {
    use embassy_time::{Duration, Instant, Timer};
    use embedded_graphics_core::{geometry::Point, pixelcolor::Rgb888};
    use galactic_unicorn_embassy::{HEIGHT, WIDTH};
    use unicorn_graphics::UnicornGraphics;

    use crate::unicorn::display::DisplayGraphicsMessage;

    #[derive(Clone, Copy)]
    pub enum Effects {
        Balls,
    }

    pub struct Balls;

    impl Balls {
        pub fn new() -> Self {
            Self {}
        }

        pub async fn display(&self) {
            let mut graphics: UnicornGraphics<WIDTH, HEIGHT> = UnicornGraphics::new();
            let mut heat: [[f32; 13]; 53] = [[0.0; 13]; 53];

            loop {
                for y in 0..11 {
                    for x in 0..53 {
                        let coord = Point { x, y };

                        let x = x as usize;
                        let y = y as usize;
                        if heat[x][y] > 0.5 {
                            let color = Rgb888::new(255, 255, 180);
                            graphics.set_pixel(coord, color);
                        } else if heat[x][y] > 0.4 {
                            let color = Rgb888::new(220, 160, 0);
                            graphics.set_pixel(coord, color);
                        } else if heat[x][y] > 0.3 {
                            let color = Rgb888::new(180, 50, 0);
                            graphics.set_pixel(coord, color);
                        } else if heat[x][y] > 0.2 {
                            let color = Rgb888::new(40, 40, 40);
                            graphics.set_pixel(coord, color);
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

                DisplayGraphicsMessage::from_app(
                    graphics.get_pixels(),
                    Some(Duration::from_millis(50)),
                )
                .send()
                .await;

                // clear the bottom row and then add a new fire seed to it
                for x in 0..53 {
                    heat[x as usize][11] = 0.0;
                }

                // add a new random heat source
                for _ in 0..5 {
                    let ticks = Instant::now().as_ticks();
                    let px: usize = ticks as usize % 51 + 1;
                    heat[px][11] = 1.0;
                    heat[px + 1][11] = 1.0;
                    heat[px - 1][11] = 1.0;
                    heat[px][12] = 1.0;
                    heat[px + 1][12] = 1.0;
                    heat[px - 1][12] = 1.0;
                }

                Timer::after_millis(50).await;
            }
        }
    }
}
