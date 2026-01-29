use core::sync::atomic::{AtomicBool, Ordering};

use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, mutex::Mutex, signal::Signal};
use embassy_time::{Duration, Timer};
use heapless::String;
use static_cell::make_static;

use crate::{
    app::{AppRunner, AppRunnerInboxSubscribers, UnicornApp, UnicornAppRunner},
    display::{DisplayState, GraphicsBufferWriter},
};

/// MQTT app. Will display the latest MQTT message.
pub struct MqttApp {
    /// The last message received.
    pub last_message: Mutex<ThreadModeRawMutex, Option<String<64>>>,

    /// Signal to update the message displayed.
    pub update_message: Signal<ThreadModeRawMutex, bool>,

    /// Track if the app is active or not.
    pub is_active: AtomicBool,

    /// Display state reference.
    display_state: &'static DisplayState,
}

impl MqttApp {
    /// Create the static ref to MQTT app.
    /// Must only be called once or will panic.
    pub fn new(display_state: &'static DisplayState) -> &'static Self {
        make_static!(Self {
            last_message: Mutex::new(None),
            update_message: Signal::new(),
            is_active: AtomicBool::new(false),
            display_state,
        })
    }

    /// Set the last message received from MQTT.
    pub async fn set_last_message(&self, message: String<64>) {
        self.last_message.lock().await.replace(message);
        self.update_message.signal(true);
    }

    /// Get a copy of the last message.
    pub async fn get_last_message(&self) -> Option<String<64>> {
        self.last_message.lock().await.clone()
    }
}

impl UnicornApp for MqttApp {
    async fn create_runner(
        &'static self,
        graphics_buffer: GraphicsBufferWriter,
        inbox: AppRunnerInboxSubscribers,
    ) -> AppRunner {
        self.is_active.store(true, Ordering::Relaxed);
        AppRunner::Mqtt(MqttAppRunner::new(
            graphics_buffer,
            self,
            inbox,
        ))
    }
}

/// Runner for the MQTT app. Displays the last received message.
pub struct MqttAppRunner {
    graphics_buffer: GraphicsBufferWriter,
    state: &'static MqttApp,
    #[allow(dead_code)]
    inbox: AppRunnerInboxSubscribers,
}

impl<'a> MqttAppRunner {
    pub fn new(
        graphics_buffer: GraphicsBufferWriter,
        state: &'static MqttApp,
        inbox: AppRunnerInboxSubscribers,
    ) -> Self {
        Self {
            graphics_buffer,
            state,
            inbox,
        }
    }
}

impl UnicornAppRunner for MqttAppRunner {
    async fn run(&mut self) -> ! {
        // Signal that this app is happy to be interrupted at all times
        crate::app::AppNotificationPolicy::set(crate::app::AppNotificationPolicy::AllowAll);

        loop {
            // Get the last message
            let message = self.state.get_last_message().await;

            match message {
                Some(msg) => {
                    // Display the message (this will scroll if needed)
                    self.graphics_buffer
                        .display_text(
                            &msg,
                            Some(Duration::from_millis(100)),
                            None,
                            Some(Duration::from_secs(3)),
                            self.state.display_state,
                        )
                        .await;
                }
                None => {
                    // No message yet, show waiting text
                    self.graphics_buffer
                        .display_text(
                            "Waiting for MQTT...",
                            None,
                            None,
                            Some(Duration::from_secs(2)),
                            self.state.display_state,
                        )
                        .await;
                }
            }

            // Wait for either a new message or a timeout before re-displaying
            embassy_futures::select::select(
                self.state.update_message.wait(),
                Timer::after_secs(5),
            )
            .await;
        }
    }

    fn release_writer(self) -> GraphicsBufferWriter {
        self.state.is_active.store(false, Ordering::Relaxed);
        self.graphics_buffer
    }
}
