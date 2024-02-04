use embassy_time::{Duration, Instant, Timer};
use embedded_graphics_core::{geometry::Point, pixelcolor::Rgb888};
use galactic_unicorn_embassy::{HEIGHT, WIDTH};
use unicorn_graphics::UnicornGraphics;

use crate::{app::UnicornApp, unicorn::display::DisplayGraphicsMessage};

pub struct EffectsApp {}

impl EffectsApp {
    pub fn new() -> Self {
        Self {}
    }
}

impl UnicornApp for EffectsApp {
    async fn display(&self) {
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

            DisplayGraphicsMessage::from_app(graphics.pixels, Some(Duration::from_millis(50)))
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
