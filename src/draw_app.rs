use embassy_net::tcp::TcpSocket;
use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, signal::Signal};
use embassy_time::{Duration, Timer};
use embedded_graphics::{geometry::Point, pixelcolor::Rgb888};
use static_cell::make_static;

use crate::{
    app::{AppNotificationPolicy, AppRunner, AppRunnerInboxSubscribers, UnicornApp, UnicornAppRunner},
    display::{DisplayState, GraphicsBufferWriter},
    network::{get_network_stack, NetworkState},
    system::SystemState,
};

const DRAW_SERVER_PORT: u16 = 8080;

/// Signal to stop TCP server
static STOP_TCP_SERVER: Signal<ThreadModeRawMutex, ()> = Signal::new();

pub struct DrawApp {
    system_state: &'static SystemState,
    display_state: &'static DisplayState,
}

impl DrawApp {
    /// Create the static ref to Draw app.
    /// Must only be called once or will panic.
    pub fn new(
        system_state: &'static SystemState,
        display_state: &'static DisplayState,
    ) -> &'static Self {
        make_static!(Self {
            system_state,
            display_state
        })
    }
}

impl UnicornApp for DrawApp {
    async fn create_runner(
        &'static self,
        graphics_buffer: GraphicsBufferWriter,
        inbox: AppRunnerInboxSubscribers,
        notification_policy: Signal<ThreadModeRawMutex, AppNotificationPolicy>,
    ) -> AppRunner {
        AppRunner::Draw(DrawAppRunner::new(
            graphics_buffer,
            self.display_state,
            self.system_state,
            inbox,
            notification_policy,
        ))
    }
}

pub struct DrawAppRunner {
    pub graphics_buffer: GraphicsBufferWriter,
    pub display_state: &'static DisplayState,
    pub system_state: &'static SystemState,
    #[allow(dead_code)]
    pub inbox: AppRunnerInboxSubscribers,
    pub notification_policy: Signal<ThreadModeRawMutex, AppNotificationPolicy>,
}

impl<'a> DrawAppRunner {
    pub fn new(
        graphics_buffer: GraphicsBufferWriter,
        display_state: &'static DisplayState,
        system_state: &'static SystemState,
        inbox: AppRunnerInboxSubscribers,
        notification_policy: Signal<ThreadModeRawMutex, AppNotificationPolicy>,
    ) -> Self {
        Self {
            graphics_buffer,
            display_state,
            system_state,
            inbox,
            notification_policy,
        }
    }

    async fn handle_connection(
        &mut self,
        socket: &mut TcpSocket<'_>,
    ) -> Result<(), embassy_net::tcp::Error> {
        // clear to start with blank canvas
        self.graphics_buffer.clear().await;

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

    async fn parse_command(&mut self, data: &[u8]) -> Result<(), &'static str> {
        if data.len() < 2 {
            return Err("Command too short");
        }

        const VERSION_1: u8 = 0x01;

        if data[0] != VERSION_1 {
            return Err("Unknown version");
        }

        const CMD_CLEAR: u8 = 0x00;
        const CMD_SET_PIXEL: u8 = 0x01;

        match data[1] {
            CMD_CLEAR => {
                // Clear and render
                self.graphics_buffer.clear().await;
                Ok(())
            }
            CMD_SET_PIXEL => {
                // Process ALL pixel commands in this buffer with ONE mutex lock
                let mut offset = 0;
                let mut pixels = self.graphics_buffer.pixels_mut().await;

                while offset + 7 <= data.len() {
                    if data[offset] != VERSION_1 || data[offset + 1] != CMD_SET_PIXEL {
                        break;
                    }

                    let x = data[offset + 2] as i32;
                    let y = data[offset + 3] as i32;
                    let r = data[offset + 4];
                    let g = data[offset + 5];
                    let b = data[offset + 6];

                    pixels.set_pixel(Point::new(x, y), Rgb888::new(r, g, b));
                    offset += 7;
                }

                // Drop the mutex lock before signaling
                drop(pixels);

                // Signal render once for all pixels
                self.graphics_buffer.send();
                Ok(())
            }
            _ => Err("Unknown command"),
        }
    }
}

impl UnicornAppRunner for DrawAppRunner {
    async fn run(&mut self) -> ! {
        loop {
            let network_state = self.system_state.get_network_state().await;

            match network_state {
                NetworkState::NotInitialised | NetworkState::Initializing => {
                    self.notification_policy
                        .signal(AppNotificationPolicy::AllowAll);
                    self.graphics_buffer
                        .display_text("WiFi connecting", None, None, None, self.display_state)
                        .await;

                    Timer::after(Duration::from_millis(500)).await;
                }
                NetworkState::Connected => {
                    self.notification_policy
                        .signal(AppNotificationPolicy::DenyNormal);
                    let stack = get_network_stack().await;

                    let mut rx_buffer = [0; 1024];
                    let mut tx_buffer = [0; 1024];

                    let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
                    socket.set_timeout(Some(Duration::from_secs(30)));

                    log::info!("Listening on TCP:{}...", DRAW_SERVER_PORT);

                    self.graphics_buffer
                        .display_text(
                            "Waiting for connection",
                            None,
                            None,
                            Some(Duration::from_millis(100)),
                            self.display_state,
                        )
                        .await;

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
                }
                NetworkState::Error => {
                    self.notification_policy
                        .signal(AppNotificationPolicy::AllowAll);
                    self.graphics_buffer
                        .display_text("Network Error!", None, None, None, self.display_state)
                        .await;

                    Timer::after(Duration::from_millis(500)).await;
                }
            }
        }
    }

    fn release_writer(self) -> GraphicsBufferWriter {
        self.graphics_buffer
    }
}
