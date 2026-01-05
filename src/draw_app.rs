use core::sync::atomic::{AtomicBool, Ordering};
use embassy_executor::Spawner;
use embassy_futures::select::select;
use embassy_net::{tcp::TcpSocket, Stack};
use embassy_sync::{
    blocking_mutex::raw::ThreadModeRawMutex,
    mutex::Mutex,
    signal::Signal,
};
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
    network::{get_network_stack, is_network_ready, NetworkState},
    system::SystemState,
};

const DRAW_SERVER_PORT: u16 = 8080;

/// Signal to stop TCP server
static STOP_TCP_SERVER: Signal<ThreadModeRawMutex, ()> = Signal::new();

/// Signal to update drawing
static UPDATE_DRAWING: Signal<ThreadModeRawMutex, ()> = Signal::new();

pub struct DrawApp {
    system_state: &'static SystemState,
    drawing_buffer: Mutex<ThreadModeRawMutex, UnicornGraphics<WIDTH, HEIGHT>>,
    server_active: AtomicBool,
    is_active: AtomicBool,
    spawner: Spawner,
}

impl DrawApp {
    /// Create the static ref to Draw app.
    /// Must only be called once or will panic.
    pub fn new(system_state: &'static SystemState, spawner: Spawner) -> &'static Self {
        let mut graphics = UnicornGraphics::<WIDTH, HEIGHT>::new();
        graphics.clear_all();

        make_static!(Self {
            system_state,
            drawing_buffer: Mutex::new(graphics),
            server_active: AtomicBool::new(false),
            is_active: AtomicBool::new(false),
            spawner,
        })
    }

    pub async fn clear_drawing(&self) {
        let mut buffer = self.drawing_buffer.lock().await;
        buffer.clear_all();
        UPDATE_DRAWING.signal(());
    }

    pub async fn set_pixel(&self, x: usize, y: usize, color: Rgb888) {
        if x < WIDTH && y < HEIGHT {
            let mut buffer = self.drawing_buffer.lock().await;
            buffer.set_pixel(Point::new(x as i32, y as i32), color);
            UPDATE_DRAWING.signal(());
        }
    }

    fn start_tcp_server(&'static self) {
        if self.server_active.load(Ordering::Relaxed) {
            return;
        }

        log::info!("Starting TCP server on port {}...", DRAW_SERVER_PORT);
        self.server_active.store(true, Ordering::Relaxed);

        // Spawn a task that waits for network and starts the server
        self.spawner.spawn(start_tcp_server_task(self)).unwrap();
    }

    async fn stop_tcp_server(&self) {
        if !self.server_active.load(Ordering::Relaxed) {
            return;
        }

        log::info!("Stopping TCP server...");
        STOP_TCP_SERVER.signal(());
        self.server_active.store(false, Ordering::Relaxed);
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
                    if self.server_active.load(Ordering::Relaxed) {
                        // Show drawing buffer
                        let buffer = self.drawing_buffer.lock().await;
                        DisplayGraphicsMessage::from_app(
                            buffer.get_pixels(),
                            Duration::from_millis(50)
                        )
                        .send_and_replace_queue()
                        .await;

                        select(
                            UPDATE_DRAWING.wait(),
                            Timer::after(Duration::from_millis(100))
                        ).await;
                    } else {
                        DisplayTextMessage::from_app(
                            "Server starting...",
                            None,
                            None,
                            Some(Duration::from_secs(1)),
                        )
                        .send_and_replace_queue()
                        .await;

                        Timer::after(Duration::from_millis(500)).await;
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

        // If network ready, start server immediately
        if is_network_ready(self.system_state).await {
            // Safe because DrawApp is always created as 'static via make_static!
            let static_self = unsafe { &*(self as *const _ as *const DrawApp) };
            static_self.start_tcp_server();
        }
        // Otherwise wait for on_network_ready callback
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

    async fn on_network_ready(&self) {
        log::info!("DrawApp: network ready");

        if self.is_active.load(Ordering::Relaxed) {
            // Safe because DrawApp is always created as 'static via make_static!
            let static_self = unsafe { &*(self as *const _ as *const DrawApp) };
            static_self.start_tcp_server();
        }
    }

    async fn on_network_lost(&self) {
        log::info!("DrawApp: network lost");

        if self.server_active.load(Ordering::Relaxed) {
            self.stop_tcp_server().await;
        }
    }
}

#[embassy_executor::task]
async fn start_tcp_server_task(app: &'static DrawApp) {
    log::info!("TCP server: Waiting for network...");
    let stack = get_network_stack().await;
    log::info!("TCP server: Starting");

    app.spawner.spawn(tcp_server_task(app, stack)).unwrap();
}

#[embassy_executor::task]
async fn tcp_server_task(
    app: &'static DrawApp,
    stack: Stack<'static>,
) {
    log::info!("TCP server task started");

    let mut rx_buffer = [0; 1024];
    let mut tx_buffer = [0; 1024];

    loop {
        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(Duration::from_secs(10)));

        log::info!("Listening on TCP:{}...", DRAW_SERVER_PORT);

        if let Err(e) = socket.accept(DRAW_SERVER_PORT).await {
            log::warn!("TCP accept error: {:?}", e);
            Timer::after(Duration::from_secs(1)).await;
            continue;
        }

        log::info!("Client connected");

        match handle_connection(&mut socket, app).await {
            Ok(_) => log::info!("Client disconnected"),
            Err(e) => log::warn!("Connection error: {:?}", e),
        }

        if STOP_TCP_SERVER.signaled() {
            STOP_TCP_SERVER.reset();
            break;
        }
    }

    log::info!("TCP server stopped");
}

async fn handle_connection(
    socket: &mut TcpSocket<'_>,
    app: &'static DrawApp,
) -> Result<(), embassy_net::tcp::Error> {
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

        let response: &[u8] = if let Err(_) = parse_command(&buffer[..n], app).await {
            b"E\n"
        } else {
            b"K\n"
        };

        // Write response (handle partial writes)
        let mut written = 0;
        while written < response.len() {
            let n = socket.write(&response[written..]).await?;
            written += n;
        }

        socket.flush().await?;
    }
}

async fn parse_command(
    data: &[u8],
    app: &'static DrawApp,
) -> Result<(), &'static str> {
    let cmd = core::str::from_utf8(data).map_err(|_| "Invalid UTF-8")?;
    let cmd = cmd.trim();

    let parts: heapless::Vec<&str, 8> = cmd.split_whitespace().collect();

    if parts.is_empty() {
        return Err("Empty command");
    }

    match parts[0] {
        "CLR" => {
            app.clear_drawing().await;
            Ok(())
        }
        "PXL" => {
            if parts.len() != 6 {
                return Err("PXL requires: x y r g b");
            }

            let x: usize = parts[1].parse().map_err(|_| "Invalid x")?;
            let y: usize = parts[2].parse().map_err(|_| "Invalid y")?;
            let r: u8 = parts[3].parse().map_err(|_| "Invalid r")?;
            let g: u8 = parts[4].parse().map_err(|_| "Invalid g")?;
            let b: u8 = parts[5].parse().map_err(|_| "Invalid b")?;

            app.set_pixel(x, y, Rgb888::new(r, g, b)).await;
            Ok(())
        }
        _ => Err("Unknown command"),
    }
}
