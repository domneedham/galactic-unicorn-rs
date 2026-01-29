use core::str::FromStr;

use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::{CriticalSectionRawMutex, ThreadModeRawMutex};
use embassy_sync::mutex::Mutex;
use embassy_sync::pubsub::{PubSubChannel, Subscriber};
use embassy_sync::signal::Signal;

use embassy_sync::watch::Watch;
use galactic_unicorn_embassy::buttons::UnicornButtons;
use static_cell::{make_static, StaticCell};
use strum_macros::{EnumString, IntoStaticStr};

use crate::buttons::ButtonPress;
use crate::clock_app::{ClockAppRunner, ClockAppState};
use crate::display::{Display, DisplayState, GraphicsBuffer, GraphicsBufferWriter};
use crate::draw_app::{DrawApp, DrawAppRunner};
use crate::effects_app::{EffectsApp, EffectsAppRunner};
use crate::mqtt::topics::APP_STATE_TOPIC;
use crate::mqtt::{
    topics::{APP_SET_TOPIC, CLOCK_APP_SET_TOPIC, TEXT_SET_TOPIC},
    MqttMessage, MqttReceiveMessage,
};
use crate::mqtt_app::{MqttApp, MqttAppRunner};
use crate::network::NetworkState;
use crate::system::{SystemState, STATE_CHANGED};
use crate::system_app::{SystemApp, SystemAppRunner};

// Signal to tell hardware which buffer to render
#[derive(Clone, Copy, PartialEq)]
pub enum DisplayLayer {
    App,
    Notification,
}
pub static ACTIVE_LAYER: Watch<ThreadModeRawMutex, DisplayLayer, 2> = Watch::new();

/// All apps that can be switched to.
#[derive(Copy, Clone, PartialEq, Eq, EnumString, IntoStaticStr)]
#[strum(ascii_case_insensitive)]
pub enum Apps {
    /// The system app. This should only be changed to by the system.
    System,

    /// The clock app.
    Clock,

    /// The effects app.
    Effects,

    /// The MQTT app.
    Mqtt,

    /// The draw app (WebSocket draw).
    Draw,
}

pub struct AppRunnerInboxSubscribers {
    pub buttons: Subscriber<'static, ThreadModeRawMutex, ButtonPress, 4, 1, 1>,
    pub mqtt: Subscriber<'static, ThreadModeRawMutex, MqttReceiveMessage, 8, 1, 1>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum AppNotificationPolicy {
    AllowAll,
    DenyNormal, // Only Critical allowed
}

pub enum AppRunner {
    System(SystemAppRunner),
    Clock(ClockAppRunner),
    Effects(EffectsAppRunner),
    Mqtt(MqttAppRunner),
    Draw(DrawAppRunner),
}

impl AppRunner {
    pub async fn run(&mut self) {
        match self {
            AppRunner::System(runner) => runner.run().await,
            AppRunner::Clock(runner) => runner.run().await,
            AppRunner::Effects(runner) => runner.run().await,
            AppRunner::Mqtt(runner) => runner.run().await,
            AppRunner::Draw(runner) => runner.run().await,
        }
    }

    pub fn release_writer(self) -> GraphicsBufferWriter {
        match self {
            AppRunner::System(runner) => runner.release_writer(),
            AppRunner::Clock(runner) => runner.release_writer(),
            AppRunner::Effects(runner) => runner.release_writer(),
            AppRunner::Mqtt(runner) => runner.release_writer(),
            AppRunner::Draw(runner) => runner.release_writer(),
        }
    }
}

pub trait UnicornApp {
    /// Create an app runner.
    async fn create_runner(
        &'static self,
        graphics_buffer: GraphicsBufferWriter,
        inbox: AppRunnerInboxSubscribers,
    ) -> AppRunner;
}

pub trait UnicornAppRunner {
    async fn run(&mut self) -> !;

    fn release_writer(self) -> GraphicsBufferWriter;
}

pub static CHANGE_APP_SIGNAL: Signal<ThreadModeRawMutex, Apps> = Signal::new();
/// Shared notification policy state. Apps signal their policy here when they start running.
pub static NOTIFICATION_POLICY: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

impl AppNotificationPolicy {
    pub fn set(policy: Self) {
        NOTIFICATION_POLICY.store(policy as u8, core::sync::atomic::Ordering::Relaxed);
    }

    pub fn get() -> Self {
        match NOTIFICATION_POLICY.load(core::sync::atomic::Ordering::Relaxed) {
            1 => Self::DenyNormal,
            _ => Self::AllowAll,
        }
    }
}

/// Task that monitors network state and switches from System app to Clock app
/// when network connects for the first time.
#[embassy_executor::task]
async fn network_connected_switch_task(system_state: &'static SystemState) {
    // Check if already connected (in case network connects very quickly)
    if system_state.get_network_state().await == NetworkState::Connected {
        CHANGE_APP_SIGNAL.signal(Apps::Clock);
        return;
    }

    // Wait for network to connect
    loop {
        STATE_CHANGED.wait().await;

        if system_state.get_network_state().await == NetworkState::Connected {
            CHANGE_APP_SIGNAL.signal(Apps::Clock);
            return;
        }
    }
}

/// App controller is responsible for managing apps by:
/// - Starting and stopping apps on user selection
/// - Starting and stopping apps from MQTT
/// - Forwarding button presses to active apps
pub struct AppController {
    /// The current active app.
    active_app: Mutex<ThreadModeRawMutex, Apps>,

    /// Display reference.
    display: &'static Display,

    /// Display state.
    display_state: &'static DisplayState,

    /// System app.
    system_app: &'static SystemApp,

    /// Clock app state.
    clock_app: &'static ClockAppState,

    /// Effects app.
    effects_app: &'static EffectsApp,

    /// MQTT app.
    mqtt_app: &'static MqttApp,

    /// Draw app.
    draw_app: &'static DrawApp,

    // The Baton - the graphics writer passed between app runners
    // Wrapped in Mutex for interior mutability since AppController is &'static
    graphics_writer: Mutex<ThreadModeRawMutex, Option<GraphicsBufferWriter>>,

    // Notification graphics writer (for overlay messages)
    notification_writer: Mutex<ThreadModeRawMutex, GraphicsBufferWriter>,

    // Inputs - wrapped in Mutex for interior mutability
    btn_rx: Mutex<
        ThreadModeRawMutex,
        Subscriber<'static, ThreadModeRawMutex, (UnicornButtons, ButtonPress), 4, 1, 9>,
    >,
    mqtt_rx: Mutex<
        ThreadModeRawMutex,
        Subscriber<'static, ThreadModeRawMutex, MqttReceiveMessage, 8, 1, 1>,
    >,

    // App-Side Channels (Bridge)
    // These act as the internal pipes to the current runner
    app_btn_chan: &'static PubSubChannel<ThreadModeRawMutex, ButtonPress, 4, 1, 1>,
    app_mqtt_chan: &'static PubSubChannel<ThreadModeRawMutex, MqttReceiveMessage, 8, 1, 1>,
}

impl AppController {
    /// Create the static ref to app controller.
    /// Must only be called once or will panic.
    pub fn new(
        display: &'static Display,
        display_state: &'static DisplayState,
        system_state: &'static SystemState,
        system_app: &'static SystemApp,
        clock_app: &'static ClockAppState,
        effects_app: &'static EffectsApp,
        mqtt_app: &'static MqttApp,
        draw_app: &'static DrawApp,
        btn_rx: Subscriber<'static, ThreadModeRawMutex, (UnicornButtons, ButtonPress), 4, 1, 9>,
        mqtt_rx: Subscriber<'static, ThreadModeRawMutex, MqttReceiveMessage, 8, 1, 1>,
        spawner: Spawner,
    ) -> &'static Self {
        // Signals for graphics buffer updates (just notifications, not data)
        static APP_GRAPHICS_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();
        static NOTIFICATION_GRAPHICS_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

        // Shared pixel buffers (Mutex-protected)
        use crate::display::DirtyRect;
        use unicorn_graphics::UnicornGraphics;
        static APP_PIXELS: Mutex<CriticalSectionRawMutex, UnicornGraphics<53, 11>> =
            Mutex::new(UnicornGraphics::new());
        static NOTIFICATION_PIXELS: Mutex<CriticalSectionRawMutex, UnicornGraphics<53, 11>> =
            Mutex::new(UnicornGraphics::new());

        // Dirty rectangle tracking (Mutex-protected)
        static APP_DIRTY_RECT: Mutex<CriticalSectionRawMutex, DirtyRect> =
            Mutex::new(DirtyRect::new());
        static NOTIFICATION_DIRTY_RECT: Mutex<CriticalSectionRawMutex, DirtyRect> =
            Mutex::new(DirtyRect::new());

        // App-side channels for forwarding events to current runner
        static APP_BTN_CHAN: PubSubChannel<ThreadModeRawMutex, ButtonPress, 4, 1, 1> =
            PubSubChannel::new();
        static APP_MQTT_CHAN: PubSubChannel<ThreadModeRawMutex, MqttReceiveMessage, 8, 1, 1> =
            PubSubChannel::new();

        // Create graphics buffers as statics so we can get 'static writers
        static APP_GRAPHICS: StaticCell<GraphicsBuffer> = StaticCell::new();
        static NOTIFICATION_GRAPHICS: StaticCell<GraphicsBuffer> = StaticCell::new();

        // Initialize graphics buffers with shared mutex access
        let app_graphics = APP_GRAPHICS.init(GraphicsBuffer::new(
            &APP_PIXELS,
            &APP_GRAPHICS_SIGNAL,
            &APP_DIRTY_RECT,
        ));
        let notification_graphics = NOTIFICATION_GRAPHICS.init(GraphicsBuffer::new(
            &NOTIFICATION_PIXELS,
            &NOTIFICATION_GRAPHICS_SIGNAL,
            &NOTIFICATION_DIRTY_RECT,
        ));

        // Get readers and writers (both access the same underlying mutex)
        let app_reader = app_graphics.reader();
        let notification_reader = notification_graphics.reader();
        let app_writer = app_graphics.writer();
        let notification_writer = notification_graphics.writer();

        let controller = Self {
            active_app: Mutex::new(Apps::System),
            display,
            display_state,
            system_app,
            clock_app,
            effects_app,
            mqtt_app,
            draw_app,
            graphics_writer: Mutex::new(Some(app_writer)),
            notification_writer: Mutex::new(notification_writer),
            btn_rx: Mutex::new(btn_rx),
            mqtt_rx: Mutex::new(mqtt_rx),
            app_btn_chan: &APP_BTN_CHAN,
            app_mqtt_chan: &APP_MQTT_CHAN,
        };

        let controller = make_static!(controller);

        // Initialize ACTIVE_LAYER so render_task doesn't block waiting for a value
        ACTIVE_LAYER.sender().send(DisplayLayer::App);

        spawner
            .spawn(crate::display::render_task(
                controller.display,
                controller.display_state,
                app_reader,
                notification_reader,
            ))
            .unwrap();

        // Spawn task to switch from System app to Clock app when network connects
        spawner
            .spawn(network_connected_switch_task(system_state))
            .unwrap();

        controller
    }

    pub async fn run(&self) -> ! {
        loop {
            let writer = self.graphics_writer.lock().await.take().unwrap();
            let btn_sub = self.app_btn_chan.subscriber().unwrap();
            let mqtt_sub = self.app_mqtt_chan.subscriber().unwrap();
            let inbox = AppRunnerInboxSubscribers {
                buttons: btn_sub,
                mqtt: mqtt_sub,
            };

            let mut runner = match *self.active_app.lock().await {
                Apps::System => {
                    self.system_app
                        .create_runner(writer, inbox)
                        .await
                }
                Apps::Clock => {
                    self.clock_app
                        .create_runner(writer, inbox)
                        .await
                }
                Apps::Effects => {
                    self.effects_app
                        .create_runner(writer, inbox)
                        .await
                }
                Apps::Mqtt => {
                    self.mqtt_app
                        .create_runner(writer, inbox)
                        .await
                }
                Apps::Draw => {
                    self.draw_app
                        .create_runner(writer, inbox)
                        .await
                }
            };

            // 2. Drive the App and the Forwarder
            // If forward_and_intercept returns SwitchApp, the whole select finishes
            let result = embassy_futures::select::select4(
                runner.run(),
                self.handle_events(),
                CHANGE_APP_SIGNAL.wait(),
                crate::draw_app::wait_for_disconnection(),
            )
            .await;

            // 3. App Switch triggered: Recover the writer from the runner
            // Because the runner is dropped here, all its futures stop
            *self.graphics_writer.lock().await = Some(runner.release_writer());

            // Update active_app based on which signal fired
            match result {
                embassy_futures::select::Either4::Third(new_app) => {
                    // CHANGE_APP_SIGNAL fired - switch to requested app
                    *self.active_app.lock().await = new_app;
                    self.send_mqtt_states().await;
                }
                embassy_futures::select::Either4::Fourth(_) => {
                    // WS_DISCONNECTED fired - restore previous app
                    let previous_app = crate::draw_app::take_previous_app().await;
                    *self.active_app.lock().await = previous_app;
                    self.send_mqtt_states().await;
                }
                _ => {}
            }
        }
    }

    async fn handle_events(&self) -> ! {
        loop {
            match embassy_futures::select::select3(
                self.btn_rx.lock().await.next_message_pure(),
                self.mqtt_rx.lock().await.next_message_pure(),
                crate::draw_app::wait_for_connection(),
            )
            .await
            {
                embassy_futures::select::Either3::First((button, press)) => {
                    self.handle_button_event(button, press).await;
                }
                embassy_futures::select::Either3::Second(msg) => {
                    self.handle_mqtt_event(msg).await;
                }
                embassy_futures::select::Either3::Third(_) => {
                    // WebSocket connected - store current app and switch to Draw
                    let current_app = *self.active_app.lock().await;
                    crate::draw_app::store_previous_app(current_app).await;
                    CHANGE_APP_SIGNAL.signal(Apps::Draw);
                }
            }
        }
    }

    async fn handle_button_event(&self, button: UnicornButtons, press: ButtonPress) {
        let active_app = *self.active_app.lock().await;

        // Handle brightness buttons
        match button {
            UnicornButtons::BrightnessUp => {
                let mut brightness_sub = self.display_state.brightness.receiver().unwrap();
                let current = brightness_sub.get().await;
                let new_brightness = current.saturating_add(25).min(255);
                self.display_state.brightness.sender().send(new_brightness);
                return;
            }
            UnicornButtons::BrightnessDown => {
                let mut brightness_sub = self.display_state.brightness.receiver().unwrap();
                let current = brightness_sub.get().await;
                let new_brightness = current.saturating_sub(25);
                self.display_state.brightness.sender().send(new_brightness);
                return;
            }
            _ => {}
        }

        let target_app = match button {
            UnicornButtons::SwitchA => Apps::Clock,
            UnicornButtons::SwitchB => Apps::Effects,
            UnicornButtons::SwitchC => Apps::Mqtt,
            UnicornButtons::SwitchD => Apps::Draw,
            _ => return,
        };

        if target_app != active_app {
            CHANGE_APP_SIGNAL.signal(target_app);
            return;
        }

        self.app_btn_chan.publisher().unwrap().publish(press).await;
    }

    async fn handle_mqtt_event(&self, message: MqttReceiveMessage) {
        if message.topic == TEXT_SET_TOPIC {
            let active_app = *self.active_app.lock().await;

            // If MQTT app is active, just update the message directly
            // The MQTT app will display it on its own layer
            if active_app == Apps::Mqtt {
                self.mqtt_app.set_last_message(message.body).await;
                self.send_mqtt_states().await;
                return;
            }

            // For other apps, decide based on the active app's notification policy
            let should_show = AppNotificationPolicy::get() == AppNotificationPolicy::AllowAll;

            if !should_show {
                // Store the message but don't display it as notification
                self.mqtt_app.set_last_message(message.body).await;
                self.send_mqtt_states().await;
                return;
            }

            // Show as notification overlay for non-MQTT apps
            // First, prepare the notification buffer with initial content
            {
                let writer = self.notification_writer.lock().await;
                writer.clear().await;
            }

            // Now switch to notification layer
            ACTIVE_LAYER.sender().send(DisplayLayer::Notification);

            // Display the text (this will continuously update the notification buffer)
            // Use a minimum duration of 2 seconds to ensure the message is visible
            self.notification_writer
                .lock()
                .await
                .display_text(
                    &message.body,
                    None,
                    None,
                    Some(embassy_time::Duration::from_secs(2)),
                    self.display_state,
                )
                .await;
            self.mqtt_app.set_last_message(message.body).await;

            // Switch back to app layer
            ACTIVE_LAYER.sender().send(DisplayLayer::App);
        } else if message.topic == CLOCK_APP_SET_TOPIC {
            self.clock_app.process_mqtt_message(message).await;
        } else if message.topic == APP_SET_TOPIC {
            if let Ok(new_app) = Apps::from_str(&message.body) {
                CHANGE_APP_SIGNAL.signal(new_app);
                return;
            }
        }

        self.send_mqtt_states().await;
    }

    /// Send MQTT states from each app.
    pub async fn send_mqtt_states(&self) {
        let active_app = *self.active_app.lock().await;
        let app_text = active_app.into();
        MqttMessage::enqueue_state(APP_STATE_TOPIC, app_text).await;
    }
}
