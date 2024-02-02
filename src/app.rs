use crate::buttons::{SWITCH_A_PRESS, SWITCH_B_PRESS};
use crate::time::Clock;
use crate::unicorn::display::{DisplayGraphicsMessage, DisplayTextMessage};

use embassy_time::Timer;
use embassy_time::{Duration, Instant};

use galactic_unicorn_embassy::HEIGHT;
use galactic_unicorn_embassy::WIDTH;

use embedded_graphics_core::geometry::Point;
use embedded_graphics_core::pixelcolor::Rgb888;
use unicorn_graphics::UnicornGraphics;

enum Apps {
    Clock,
    Effects,
}

pub async fn app_loop(clock: &'static Clock) -> ! {
    let mut graphics: UnicornGraphics<WIDTH, HEIGHT> = UnicornGraphics::new();

    let mut current_app = Apps::Clock;
    let mut heat: [[f32; 13]; 53] = [[0.0; 13]; 53];

    loop {
        match current_app {
            Apps::Clock => {
                let time = clock.get_date_time_str().await;
                DisplayTextMessage::from_app(&time, None, None, Some(Duration::from_secs(1)))
                    .send_and_replace_queue()
                    .await;

                Timer::after_secs(1).await;
            }
            Apps::Effects => {
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
        };

        if SWITCH_A_PRESS.signaled() {
            SWITCH_A_PRESS.reset();
            current_app = Apps::Clock;
        }

        if SWITCH_B_PRESS.signaled() {
            SWITCH_B_PRESS.reset();
            current_app = Apps::Effects;
        }
    }
}
