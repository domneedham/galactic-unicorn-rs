use embassy_net::tcp::TcpSocket;
use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, signal::Signal};
use embassy_time::{Duration, Timer};
use embedded_graphics::{geometry::Point, pixelcolor::Rgb888};
use static_cell::make_static;

use crate::{
    app::{
        AppNotificationPolicy, AppRunner, AppRunnerInboxSubscribers, UnicornApp, UnicornAppRunner,
    },
    display::{DisplayState, GraphicsBufferWriter},
    network::{get_network_stack, NetworkState},
    system::SystemState,
};

const DRAW_SERVER_PORT: u16 = 8080;

/// Signal to stop TCP server
static STOP_TCP_SERVER: Signal<ThreadModeRawMutex, ()> = Signal::new();

/// Error type for command parsing
enum ParseError {
    /// Not enough data available yet - need to wait for more bytes
    NeedMoreData,
    /// Invalid command or data
    Invalid(&'static str),
}

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

impl DrawAppRunner {
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

        let mut read_buffer = [0u8; 256];
        let mut accumulator = [0u8; 512]; // Buffer to accumulate partial commands
        let mut acc_len = 0; // How many bytes are currently in the accumulator

        loop {
            // Check if we should stop serving
            if STOP_TCP_SERVER.signaled() {
                log::info!("TCP server: Stop signal received, closing connection");
                return Ok(());
            }

            let n = match socket.read(&mut read_buffer).await {
                Ok(0) => return Ok(()),
                Ok(n) => n,
                Err(e) => return Err(e),
            };

            // Append new data to accumulator
            if acc_len + n > accumulator.len() {
                log::warn!("Accumulator overflow, resetting");
                acc_len = 0;
                socket.write(&[0x00]).await?; // Send error
                socket.flush().await?;
                continue;
            }

            accumulator[acc_len..acc_len + n].copy_from_slice(&read_buffer[..n]);
            acc_len += n;

            // Parse commands from the accumulator
            let mut consumed = 0;
            let mut made_progress = false;

            while consumed < acc_len {
                match self.try_parse_command(&accumulator[consumed..acc_len], socket).await {
                    Ok(bytes_used) => {
                        consumed += bytes_used;
                        made_progress = true;
                    }
                    Err(ParseError::NeedMoreData) => {
                        // Accumulator nearly full with no progress = malformed data
                        if acc_len > 400 && !made_progress {
                            log::warn!("Accumulator stalled at {} bytes, resetting", acc_len);
                            acc_len = 0;
                            consumed = 0;
                        }
                        break;
                    }
                    Err(ParseError::Invalid(msg)) => {
                        log::warn!("Invalid command: {}", msg);
                        consumed += 1; // Skip byte and try to resync
                        made_progress = true;
                    }
                }
            }

            // Shift remaining bytes to start of accumulator
            if consumed > 0 {
                accumulator.copy_within(consumed..acc_len, 0);
                acc_len -= consumed;
            }
        }
    }

    /// Try to parse a single command from the data buffer.
    /// Returns the number of bytes consumed if successful, or an error.
    async fn try_parse_command(
        &mut self,
        data: &[u8],
        socket: &mut TcpSocket<'_>,
    ) -> Result<usize, ParseError> {
        if data.len() < 2 {
            return Err(ParseError::NeedMoreData);
        }

        const VERSION_1: u8 = 0x01;

        if data[0] != VERSION_1 {
            return Err(ParseError::Invalid("Unknown version"));
        }

        const CMD_CLEAR: u8 = 0x00;
        const CMD_SET_PIXEL: u8 = 0x01;
        const CMD_SET_PIXELS: u8 = 0x02;
        const CMD_FILL: u8 = 0x03;

        const CMD_PING: u8 = 0xFE;
        const CMD_PONG: u8 = 0xFF;

        match data[1] {
            CMD_CLEAR => {
                // Clear and render
                self.graphics_buffer.clear().await;
                Ok(2) // Consumed 2 bytes: version + command
            }
            CMD_SET_PIXEL => {
                if data.len() < 7 {
                    return Err(ParseError::NeedMoreData);
                }

                // Batch process consecutive SET_PIXEL commands with single mutex lock
                let mut offset = 0;
                let mut pixels = self.graphics_buffer.pixels_mut().await;

                let mut min_x = i32::MAX;
                let mut min_y = i32::MAX;
                let mut max_x = i32::MIN;
                let mut max_y = i32::MIN;

                while offset + 7 <= data.len()
                    && data[offset] == VERSION_1
                    && data[offset + 1] == CMD_SET_PIXEL
                {
                    let x = data[offset + 2] as i32;
                    let y = data[offset + 3] as i32;
                    let r = data[offset + 4];
                    let g = data[offset + 5];
                    let b = data[offset + 6];

                    pixels.set_pixel(Point::new(x, y), Rgb888::new(r, g, b));

                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x);
                    max_y = max_y.max(y);

                    offset += 7;
                }

                if min_x != i32::MAX && min_x >= 0 && max_x >= 0 && min_y >= 0 && max_y >= 0 {
                    pixels.mark_dirty_region(
                        min_x as usize,
                        min_y as usize,
                        max_x as usize,
                        max_y as usize,
                    );
                }

                drop(pixels);
                self.graphics_buffer.send();
                Ok(offset)
            }
            CMD_SET_PIXELS => {
                if data.len() < 3 {
                    return Err(ParseError::NeedMoreData);
                }

                let num_pixels = data[2] as usize;
                let required_len = 3 + num_pixels * 5;
                if data.len() < required_len {
                    return Err(ParseError::NeedMoreData);
                }

                let mut pixels = self.graphics_buffer.pixels_mut().await;

                let mut min_x = i32::MAX;
                let mut min_y = i32::MAX;
                let mut max_x = i32::MIN;
                let mut max_y = i32::MIN;

                for i in 0..num_pixels {
                    let offset = 3 + i * 5;
                    let x = data[offset] as i32;
                    let y = data[offset + 1] as i32;
                    let r = data[offset + 2];
                    let g = data[offset + 3];
                    let b = data[offset + 4];

                    pixels.set_pixel(Point::new(x, y), Rgb888::new(r, g, b));

                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x);
                    max_y = max_y.max(y);
                }

                if min_x != i32::MAX && min_x >= 0 && max_x >= 0 && min_y >= 0 && max_y >= 0 {
                    pixels.mark_dirty_region(
                        min_x as usize,
                        min_y as usize,
                        max_x as usize,
                        max_y as usize,
                    );
                }

                drop(pixels);
                self.graphics_buffer.send();
                Ok(required_len)
            }
            CMD_FILL => {
                if data.len() < 5 {
                    return Err(ParseError::NeedMoreData);
                }

                let color = Rgb888::new(data[2], data[3], data[4]);

                let mut pixels = self.graphics_buffer.pixels_mut().await;
                pixels.fill(color);
                pixels.mark_all_dirty();
                drop(pixels);

                self.graphics_buffer.send();
                Ok(5)
            }
            CMD_PING => {
                // Respond with PONG
                let pong_response = [VERSION_1, CMD_PONG];
                if let Err(e) = socket.write(&pong_response).await {
                    log::warn!("Failed to send PONG: {:?}", e);
                }
                Ok(2) // Consumed 2 bytes: version + command
            }
            CMD_PONG => {
                // We shouldn't receive PONG (we don't send PING), but handle it gracefully
                log::info!("PONG received (unexpected)");
                Ok(2) // Consumed 2 bytes: version + command
            }
            _ => Err(ParseError::Invalid("Unknown command")),
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

                    // Run scrolling text and TCP accept concurrently
                    use embassy_futures::select::{select, Either};

                    let scroll_future = async {
                        loop {
                            self.graphics_buffer
                                .display_text(
                                    "Waiting for connection",
                                    None,
                                    None,
                                    None,
                                    self.display_state,
                                )
                                .await;
                        }
                    };

                    let accept_result =
                        select(scroll_future, socket.accept(DRAW_SERVER_PORT)).await;

                    match accept_result {
                        Either::First(_) => unreachable!(), // scroll_future never completes
                        Either::Second(Err(e)) => {
                            log::warn!("TCP accept error: {:?}", e);
                            Timer::after(Duration::from_secs(1)).await;
                            continue;
                        }
                        Either::Second(Ok(())) => {
                            log::info!("Client connected");

                            match self.handle_connection(&mut socket).await {
                                Ok(_) => log::info!("Client disconnected"),
                                Err(e) => log::warn!("Connection error: {:?}", e),
                            }
                        }
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
