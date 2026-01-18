use embassy_net::tcp::TcpSocket;
use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, signal::Signal};
use embassy_time::{Duration, Timer};
use embedded_graphics::pixelcolor::Rgb888;
use static_cell::make_static;

use crate::{
    app::{AppNotificationPolicy, AppRunner, UnicornApp, UnicornAppRunner},
    buttons::ButtonPress,
    display::{DisplayState, GraphicsBufferWriter},
    mqtt::MqttReceiveMessage,
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
    async fn create_runner<'a>(
        &self,
        graphics_buffer: GraphicsBufferWriter<'a>,
        notification_policy: Signal<ThreadModeRawMutex, AppNotificationPolicy>,
    ) -> AppRunner<'a> {
        AppRunner::Draw(DrawAppRunner::new(
            graphics_buffer,
            self.display_state,
            self.system_state,
            notification_policy,
        ))
    }
}

pub struct DrawAppRunner<'a> {
    pub graphics_buffer: GraphicsBufferWriter<'a>,
    pub display_state: &'static DisplayState,
    pub system_state: &'static SystemState,
    pub notification_policy: Signal<ThreadModeRawMutex, AppNotificationPolicy>,
}

impl<'a> DrawAppRunner<'a> {
    pub fn new(
        graphics_buffer: GraphicsBufferWriter<'a>,
        display_state: &'static DisplayState,
        system_state: &'static SystemState,
        notification_policy: Signal<ThreadModeRawMutex, AppNotificationPolicy>,
    ) -> Self {
        Self {
            graphics_buffer,
            display_state,
            system_state,
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
                    self.graphics_buffer.clear().await;
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

                    self.graphics_buffer
                        .set_pixel(x as i32, y as i32, Rgb888::new(r, g, b))
                        .await;
                    Ok(())
                }
                _ => Err("Unknown command"),
            };
        }

        return Err("Unknown version");
    }
}

impl<'a> UnicornAppRunner<'a> for DrawAppRunner<'a> {
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
                            Some(Duration::from_secs(1)),
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

    fn release_writer(self) -> GraphicsBufferWriter<'a> {
        self.graphics_buffer
    }
}
