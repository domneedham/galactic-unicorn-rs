use embassy_rp::peripherals::{DMA_CH0, PIO0};
use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, mutex::Mutex};
use embassy_time::Timer;
use galactic_unicorn_embassy::{pins::UnicornPins, GalacticUnicorn, HEIGHT, WIDTH};
use unicorn_graphics::UnicornGraphics;

type GalacticUnicornType = Mutex<ThreadModeRawMutex, Option<GalacticUnicorn<'static>>>;
static GALACTIC_UNICORN: GalacticUnicornType = Mutex::new(None);

pub async fn init(pio: PIO0, dma: DMA_CH0, pins: UnicornPins<'static>) {
    let gu = GalacticUnicorn::new(pio, pins, dma);
    GALACTIC_UNICORN.lock().await.replace(gu);
}

pub async fn update_display(graphics: &UnicornGraphics<WIDTH, HEIGHT>) {
    GALACTIC_UNICORN
        .lock()
        .await
        .as_mut()
        .unwrap()
        .set_pixels(graphics);
}

#[embassy_executor::task]
pub async fn draw_on_display_task() {
    loop {
        GALACTIC_UNICORN.lock().await.as_mut().unwrap().draw().await;
        Timer::after_millis(10).await;
    }
}
