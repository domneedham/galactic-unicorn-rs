use embassy_sync::{
    blocking_mutex::raw::ThreadModeRawMutex, channel::Channel, mutex::Mutex, signal::Signal,
};
use heapless::Vec;
use static_cell::make_static;

use crate::{
    app::{
        AppNotificationPolicy, AppRunner, AppRunnerInboxSubscribers, Apps, UnicornApp,
        UnicornAppRunner,
    },
    display::{DisplayState, GraphicsBufferWriter},
    draw_protocol,
};

// Raw data channel: WebSocket → WebAppRunner
pub static WS_DATA_CHANNEL: Channel<ThreadModeRawMutex, Vec<u8, 2048>, 8> = Channel::new();

// Connection lifecycle signals
pub static WS_CONNECTED: Signal<ThreadModeRawMutex, ()> = Signal::new();
pub static WS_DISCONNECTED: Signal<ThreadModeRawMutex, ()> = Signal::new();

// Previous app tracking
static PREVIOUS_APP: Mutex<ThreadModeRawMutex, Option<Apps>> = Mutex::new(None);

/// Store the currently active app before switching to Web app
pub async fn store_previous_app(app: Apps) {
    *PREVIOUS_APP.lock().await = Some(app);
}

/// Retrieve the previous app, returning Clock as fallback
pub async fn take_previous_app() -> Apps {
    PREVIOUS_APP.lock().await.take().unwrap_or(Apps::Clock)
}

pub struct WebApp {
    display_state: &'static DisplayState,
}
impl WebApp {
    /// Create the static ref to Web app.
    /// Must only be called once or will panic.
    pub fn new(display_state: &'static DisplayState) -> &'static Self {
        make_static!(Self { display_state })
    }
}

impl UnicornApp for WebApp {
    async fn create_runner(
        &'static self,
        graphics_buffer: GraphicsBufferWriter,
        inbox: AppRunnerInboxSubscribers,
        notification_policy: Signal<ThreadModeRawMutex, AppNotificationPolicy>,
    ) -> AppRunner {
        AppRunner::Web(WebAppRunner::new(
            graphics_buffer,
            self.display_state,
            inbox,
            notification_policy,
        ))
    }
}

pub struct WebAppRunner {
    pub graphics_buffer: GraphicsBufferWriter,
    pub display_state: &'static DisplayState,
    pub inbox: AppRunnerInboxSubscribers,
    pub notification_policy: Signal<ThreadModeRawMutex, AppNotificationPolicy>,
}

impl WebAppRunner {
    pub fn new(
        graphics_buffer: GraphicsBufferWriter,
        display_state: &'static DisplayState,
        inbox: AppRunnerInboxSubscribers,
        notification_policy: Signal<ThreadModeRawMutex, AppNotificationPolicy>,
    ) -> Self {
        Self {
            graphics_buffer,
            display_state,
            inbox,
            notification_policy,
        }
    }
}

impl UnicornAppRunner for WebAppRunner {
    async fn run(&mut self) -> ! {
        // Set notification policy to deny normal notifications
        self.notification_policy
            .signal(AppNotificationPolicy::DenyNormal);

        // Show "Waiting for connection" until first WebSocket data arrives
        self.graphics_buffer.clear().await;

        let data = {
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

            match select(scroll_future, WS_DATA_CHANNEL.receive()).await {
                Either::First(_) => unreachable!(),
                Either::Second(data) => data,
            }
        };

        // Clear screen for drawing
        self.graphics_buffer.clear().await;

        let mut accumulator = [0u8; 3072]; // 1.5x channel size for buffering
        let mut acc_len = 0;

        // Process the first chunk received during the wait
        Self::process_data(&data, &mut self.graphics_buffer, &mut accumulator, &mut acc_len).await;

        loop {
            let data = WS_DATA_CHANNEL.receive().await;
            Self::process_data(&data, &mut self.graphics_buffer, &mut accumulator, &mut acc_len)
                .await;
        }
    }

    fn release_writer(self) -> GraphicsBufferWriter {
        self.graphics_buffer
    }
}

impl WebAppRunner {
    async fn process_data(
        data: &[u8],
        graphics_buffer: &mut GraphicsBufferWriter,
        accumulator: &mut [u8; 3072],
        acc_len: &mut usize,
    ) {
        // Append to accumulator
        if *acc_len + data.len() > accumulator.len() {
            log::warn!("WebApp: Accumulator overflow, resetting");
            *acc_len = 0;
            return;
        }

        accumulator[*acc_len..*acc_len + data.len()].copy_from_slice(data);
        *acc_len += data.len();

        // Parse commands from the accumulator
        let mut consumed = 0;
        let mut made_progress = false;

        while consumed < *acc_len {
            match draw_protocol::try_parse_command(
                &accumulator[consumed..*acc_len],
                graphics_buffer,
            )
            .await
            {
                Ok(bytes_used) => {
                    consumed += bytes_used;
                    made_progress = true;
                }
                Err(draw_protocol::ParseError::NeedMoreData) => {
                    // Accumulator nearly full with no progress = malformed data
                    if *acc_len > 2400 && !made_progress {
                        log::warn!(
                            "WebApp: Accumulator stalled at {} bytes, resetting",
                            *acc_len
                        );
                        *acc_len = 0;
                        consumed = 0;
                    }
                    break;
                }
                Err(draw_protocol::ParseError::Invalid(msg)) => {
                    log::warn!("WebApp: Invalid command: {}", msg);
                    consumed += 1; // Skip byte and try to resync
                    made_progress = true;
                }
            }
        }

        // Shift remaining bytes to start of accumulator
        if consumed > 0 {
            accumulator.copy_within(consumed..*acc_len, 0);
            *acc_len -= consumed;
        }
    }
}
