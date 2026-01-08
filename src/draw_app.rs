use core::sync::atomic::{AtomicBool, Ordering};
use embassy_net::tcp::TcpSocket;
use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, mutex::Mutex, signal::Signal};
use embassy_time::{Duration, Timer};
use embedded_graphics::{pixelcolor::Rgb888, prelude::Point};
use galactic_unicorn_embassy::{HEIGHT, WIDTH};
use static_cell::make_static;
use unicorn_graphics::UnicornGraphics;

use crate::{
    app::{AppCapabilities, UnicornApp},
    buttons::ButtonPress,
    display::messages::{DisplayGraphicsMessage, DisplayTextMessage},
    mqtt::MqttReceiveMessage,
    network::{get_network_stack, NetworkState},
    system::SystemState,
};

const DRAW_SERVER_PORT: u16 = 8080;

/// Signal to stop TCP server
static STOP_TCP_SERVER: Signal<ThreadModeRawMutex, ()> = Signal::new();

pub struct DrawApp {
    system_state: &'static SystemState,
    drawing_buffer: Mutex<ThreadModeRawMutex, UnicornGraphics<WIDTH, HEIGHT>>,
    server_active: AtomicBool,
    is_active: AtomicBool,
}

impl DrawApp {
    /// Create the static ref to Draw app.
    /// Must only be called once or will panic.
    pub fn new(system_state: &'static SystemState) -> &'static Self {
        let mut graphics = UnicornGraphics::<WIDTH, HEIGHT>::new();
        graphics.clear_all();

        make_static!(Self {
            system_state,
            drawing_buffer: Mutex::new(graphics),
            server_active: AtomicBool::new(false),
            is_active: AtomicBool::new(false),
        })
    }

    async fn clear_drawing(&self) {
        let mut buffer = self.drawing_buffer.lock().await;
        buffer.clear_all();
        DisplayGraphicsMessage::from_app(buffer.get_pixels(), Duration::from_millis(1))
            .send_and_replace_queue_and_show_now()
            .await;
    }

    async fn set_pixel(&self, x: i32, y: i32, color: Rgb888) {
        if x >= 0 && y >= 0 && x < WIDTH as i32 && y < HEIGHT as i32 {
            let mut buffer = self.drawing_buffer.lock().await;
            buffer.set_pixel(Point::new(x, y), color);
            DisplayGraphicsMessage::from_app(buffer.get_pixels(), Duration::from_millis(1))
                .send_and_replace_queue_and_show_now()
                .await;
        }
    }

    async fn stop_tcp_server(&self) {
        if !self.server_active.load(Ordering::Relaxed) {
            return;
        }

        log::info!("Stopping TCP server...");
        STOP_TCP_SERVER.signal(());
        self.server_active.store(false, Ordering::Relaxed);
    }

    async fn handle_connection(
        &self,
        socket: &mut TcpSocket<'_>,
    ) -> Result<(), embassy_net::tcp::Error> {
        // clear to start with blank canvas
        self.clear_drawing().await;

        let mut buffer = [0u8; 256];

        loop {
            // Check if we should stop serving
            if STOP_TCP_SERVER.signaled() {
                log::info!("TCP server: Stop signal received, closing connection");
                return Ok(());
            }

            let n = match socket.read(&mut buffer).await {
                Ok(0) => return Ok(()),
                Ok(n) => n,
                Err(e) => return Err(e),
            };

            const SUCCESS: u8 = 0x01;
            const ERROR: u8 = 0x00;
            let response: u8 = if let Err(_) = self.parse_command(&buffer[..n]).await {
                ERROR
            } else {
                SUCCESS
            };

            // Write response (handle partial writes)
            socket.write(&[response]).await?;

            socket.flush().await?;
        }
    }

    async fn parse_command(&self, data: &[u8]) -> Result<(), &'static str> {
        if data.len() < 1 {
            return Err("Empty command");
        }

        const VERSION_1: u8 = 0x01;

        // version 1
        if data[0] == VERSION_1 {
            const CMD_CLEAR: u8 = 0x00;
            const CMD_SET_PIXEL: u8 = 0x01;

            return match data[1] {
                CMD_CLEAR => {
                    // clear
                    self.clear_drawing().await;
                    Ok(())
                }
                CMD_SET_PIXEL => {
                    // set pixel
                    if data.len() != 7 {
                        return Err("PXL requires 6 bytes");
                    }

                    let x = data[2] as usize;
                    let y = data[3] as usize;
                    let r = data[4];
                    let g = data[5];
                    let b = data[6];

                    self.set_pixel(x as i32, y as i32, Rgb888::new(r, g, b))
                        .await;
                    Ok(())
                }
                _ => Err("Unknown command"),
            };
        }

        return Err("Unknown version");
    }
}

impl UnicornApp for DrawApp {
    async fn display(&self) {
        loop {
            let network_state = self.system_state.get_network_state().await;

            match network_state {
                NetworkState::NotInitialised | NetworkState::Initializing => {
                    DisplayTextMessage::from_app(
                        "WiFi connecting...",
                        None,
                        None,
                        Some(Duration::from_secs(1)),
                    )
                    .send_and_replace_queue()
                    .await;

                    Timer::after(Duration::from_millis(500)).await;
                }
                NetworkState::Connected => {
                    let stack = get_network_stack().await;

                    let mut rx_buffer = [0; 1024];
                    let mut tx_buffer = [0; 1024];

                    loop {
                        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
                        socket.set_timeout(Some(Duration::from_secs(30)));

                        DisplayTextMessage::from_app(
                            "Waiting for connection",
                            None,
                            None,
                            Some(Duration::from_secs(1)),
                        )
                        .send_and_replace_queue()
                        .await;

                        log::info!("Listening on TCP:{}...", DRAW_SERVER_PORT);

                        if let Err(e) = socket.accept(DRAW_SERVER_PORT).await {
                            log::warn!("TCP accept error: {:?}", e);
                            Timer::after(Duration::from_secs(1)).await;
                            continue;
                        }

                        log::info!("Client connected");

                        match self.handle_connection(&mut socket).await {
                            Ok(_) => log::info!("Client disconnected"),
                            Err(e) => log::warn!("Connection error: {:?}", e),
                        }

                        if STOP_TCP_SERVER.signaled() {
                            STOP_TCP_SERVER.reset();
                            break;
                        }
                    }
                }
                NetworkState::Error => {
                    DisplayTextMessage::from_app(
                        "Network Error!",
                        None,
                        None,
                        Some(Duration::from_secs(1)),
                    )
                    .send_and_replace_queue()
                    .await;

                    Timer::after(Duration::from_millis(500)).await;
                }
            }
        }
    }

    async fn start(&self) {
        log::info!("DrawApp started");
        self.is_active.store(true, Ordering::Relaxed);
    }

    async fn stop(&self) {
        log::info!("DrawApp stopped");
        self.is_active.store(false, Ordering::Relaxed);

        if self.server_active.load(Ordering::Relaxed) {
            self.stop_tcp_server().await;
        }
    }

    async fn button_press(&self, press: ButtonPress) {
        match press {
            ButtonPress::Short => self.clear_drawing().await,
            _ => {}
        }
    }

    async fn process_mqtt_message(&self, _message: MqttReceiveMessage) {}

    async fn send_mqtt_state(&self) {}
}

impl AppCapabilities for DrawApp {
    fn requires_network(&self) -> bool {
        true
    }

    async fn on_network_ready(&self) {}

    async fn on_network_lost(&self) {
        log::info!("DrawApp: network lost");

        if self.server_active.load(Ordering::Relaxed) {
            self.stop_tcp_server().await;
        }
    }
}
