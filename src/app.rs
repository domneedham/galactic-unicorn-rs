use core::str::FromStr;

use embassy_executor::Spawner;
use embassy_futures::select::{select, select3, Either, Either3};
use embassy_sync::blocking_mutex::raw::{CriticalSectionRawMutex, ThreadModeRawMutex};
use embassy_sync::mutex::Mutex;
use embassy_sync::pubsub::{PubSubChannel, Publisher, Subscriber};
use embassy_sync::signal::Signal;

use embassy_sync::watch::Watch;
use galactic_unicorn_embassy::buttons::UnicornButtons;
use galactic_unicorn_embassy::{HEIGHT, WIDTH};
use static_cell::make_static;
use strum_macros::{EnumString, IntoStaticStr};
use unicorn_graphics::UnicornGraphics;

use crate::buttons::ButtonPress;
use crate::clock_app::ClockApp;
use crate::display::{
    Display, DisplayState, GraphicsBuffer, GraphicsBufferReader, GraphicsBufferWriter,
};
use crate::draw_app::{DrawApp, DrawAppRunner};
use crate::effects_app::EffectsApp;
use crate::mqtt::topics::APP_STATE_TOPIC;
use crate::mqtt::{
    topics::{APP_SET_TOPIC, CLOCK_APP_SET_TOPIC, TEXT_SET_TOPIC},
    MqttMessage, MqttReceiveMessage,
};
use crate::mqtt_app::MqttApp;
use crate::system::SystemState;
use crate::system_app::SystemApp;

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
enum Apps {
    /// The system app. This should only be changed to by the system.
    System,

    /// The clock app.
    Clock,

    /// The effects app.
    Effects,

    /// The MQTT app.
    Mqtt,

    /// The draw app.
    Draw,
}

pub struct AppRunnerInboxSubscribers {
    pub buttons: Subscriber<'static, ThreadModeRawMutex, ButtonPress, 4, 1, 1>,
    pub mqtt: Subscriber<'static, ThreadModeRawMutex, MqttReceiveMessage, 8, 1, 1>,
}

pub struct AppRunnerInboxPublishers {
    pub buttons: Publisher<'static, ThreadModeRawMutex, ButtonPress, 4, 1, 1>,
    pub mqtt: Publisher<'static, ThreadModeRawMutex, MqttReceiveMessage, 8, 1, 1>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum AppNotificationPolicy {
    AllowAll,
    DenyNormal, // Only Critical allowed
    DenyAll,    // Queue everything
}

pub enum AppRunner<'a> {
    Draw(DrawAppRunner<'a>),
}

impl<'a> AppRunner<'a> {
    pub async fn run(&mut self) {
        match self {
            AppRunner::Draw(draw_app_runner) => draw_app_runner.run().await,
        }
    }

    pub fn release_writer(self) -> GraphicsBufferWriter<'a> {
        match self {
            AppRunner::Draw(draw_app_runner) => draw_app_runner.release_writer(),
        }
    }
}

pub trait UnicornApp {
    /// Create an app runner.
    async fn create_runner<'a>(
        &self,
        graphics_buffer: GraphicsBufferWriter<'a>,
        notification_policy: Signal<ThreadModeRawMutex, AppNotificationPolicy>,
    ) -> AppRunner<'a>;
}

pub trait UnicornAppRunner<'a> {
    async fn run(&mut self) -> !;

    fn release_writer(self) -> GraphicsBufferWriter<'a>;
}

static CHANGE_APP_SIGNAL: Signal<ThreadModeRawMutex, Apps> = Signal::new();

/// App controller is responsible for managing apps by:
/// - Starting and stopping apps on user selection
/// - Starting and stopping apps from MQTT
/// - Forwarding button presses to active apps
pub struct AppController {
    /// The current active app.
    active_app: Mutex<ThreadModeRawMutex, Apps>,

    /// Graphics buffer for the app to draw into.
    app_graphics: GraphicsBuffer,

    /// Graphics buffer for notifications.
    notification_graphics: GraphicsBuffer,

    /// Display reference.
    display: &'static Display,

    /// Display state.
    display_state: &'static DisplayState,

    /// System app.
    system_app: &'static SystemApp,

    /// Clock app.
    clock_app: &'static ClockApp,

    /// Effects app.
    effects_app: &'static EffectsApp,

    /// MQTT app.
    mqtt_app: &'static MqttApp,

    /// Draw app.
    draw_app: &'static DrawApp,

    /// System state.
    system_state: &'static SystemState,

    /// Embassy spawner.
    spawner: Spawner,

    // The Baton
    graphics_writer: Option<GraphicsBufferWriter<'static>>,

    // Inputs
    btn_rx: Subscriber<'static, ThreadModeRawMutex, (UnicornButtons, ButtonPress), 4, 1, 9>,
    mqtt_rx: Subscriber<'static, ThreadModeRawMutex, MqttReceiveMessage, 8, 1, 1>,

    // App-Side Channels (Bridge)
    // These act as the internal pipes to the current runner
    app_btn_chan: PubSubChannel<ThreadModeRawMutex, ButtonPress, 4, 1, 1>,
    app_mqtt_chan: PubSubChannel<ThreadModeRawMutex, MqttReceiveMessage, 8, 1, 1>,
}

impl AppController {
    /// Create the static ref to app controller.
    /// Must only be called once or will panic.
    pub fn new(
        display: &'static Display,
        display_state: &'static DisplayState,
        system_app: &'static SystemApp,
        clock_app: &'static ClockApp,
        effects_app: &'static EffectsApp,
        mqtt_app: &'static MqttApp,
        draw_app: &'static DrawApp,
        system_state: &'static SystemState,
        spawner: Spawner,
    ) -> &'static Self {
        static APP_GRAPHICS_SIGNAL: Signal<
            CriticalSectionRawMutex,
            UnicornGraphics<WIDTH, HEIGHT>,
        > = Signal::new();
        static NOTIFICATION_GRAPHICS_SIGNAL: Signal<
            CriticalSectionRawMutex,
            UnicornGraphics<WIDTH, HEIGHT>,
        > = Signal::new();
        let app_graphics = GraphicsBuffer::new(&APP_GRAPHICS_SIGNAL);
        let notification_graphics = GraphicsBuffer::new(&NOTIFICATION_GRAPHICS_SIGNAL);

        let controller = Self {
            active_app: Mutex::new(Apps::System),
            app_graphics,
            notification_graphics,
            display,
            display_state,
            system_app,
            clock_app,
            effects_app,
            mqtt_app,
            draw_app,
            system_state,
            spawner,
            graphics_writer: todo!(),
            btn_rx: todo!(),
            mqtt_rx: todo!(),
            app_btn_chan: todo!(),
            app_mqtt_chan: todo!(),
        };

        let controller = make_static!(controller);

        spawner
            .spawn(render_task(
                controller.display,
                controller.display_state,
                controller.app_graphics.reader(),
                controller.notification_graphics.reader(),
            ))
            .unwrap();

        controller
    }

    pub async fn run(&mut self) -> ! {
        loop {
            // 1. Prepare handles for the Draw app
            let writer = self.graphics_writer.take().unwrap();
            let inbox = AppRunnerInboxSubscribers {
                buttons: self.app_btn_chan.subscriber().unwrap(),
                mqtt: self.app_mqtt_chan.subscriber().unwrap(),
            };

            let runner = match *self.active_app.lock().await {
                Apps::Draw => self.draw_app.create_runner(writer, Signal::new()).await,
                Apps::Clock => self.clock_app.create_runner(writer, Signal::new()).await,
                _ => panic!("AppController run called but active app is not Draw"),
            };

            // 2. Drive the App and the Forwarder
            // If forward_and_intercept returns SwitchApp, the whole select finishes
            embassy_futures::select::select3(
                runner.run(),
                self.handle_events(),
                CHANGE_APP_SIGNAL.wait(),
            )
            .await;

            // 3. App Switch triggered: Recover the writer from the runner
            // Because DrawAppRunner is dropped here, all its futures stop
            self.graphics_writer = Some(runner.release_writer());
        }
    }

    async fn handle_events(&mut self) {
        match select(
            self.btn_rx.next_message_pure(),
            self.mqtt_rx.next_message_pure(),
        )
        .await
        {
            Either::First((button, press)) => {
                self.handle_button_event(button, press).await;
            }
            Either::Second(msg) => {
                self.handle_mqtt_event(msg).await;
            }
        }
    }

    async fn handle_button_event(&self, button: UnicornButtons, press: ButtonPress) {
        let active_app = *self.active_app.lock().await;

        // TODO: Handle brightness and sleep buttons here

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

    async fn handle_mqtt_event(&mut self, message: MqttReceiveMessage) {
        if message.topic == TEXT_SET_TOPIC {
            self.notification_graphics
                .writer()
                .display_text(&message.body, None, None, None, self.display_state)
                .await;
            self.mqtt_app.set_last_message(message.body).await;
        } else if message.topic == CLOCK_APP_SET_TOPIC {
            // self.clock_app.process_mqtt_message(message).await;
        } else if message.topic == APP_SET_TOPIC {
            if let Ok(new_app) = Apps::from_str(&message.body) {
                CHANGE_APP_SIGNAL.signal(new_app);
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

#[embassy_executor::task]
pub async fn render_task(
    display: &'static Display,
    state: &'static DisplayState,
    app_buffer: GraphicsBufferReader<'static>,
    notify_buffer: GraphicsBufferReader<'static>,
) {
    let mut layer_sub = ACTIVE_LAYER.receiver().unwrap();
    let mut bright_sub = state.brightness.receiver().unwrap();

    loop {
        let layer = layer_sub.get().await;
        let brightness = bright_sub.try_get().unwrap_or(128);

        let buffer = match layer {
            DisplayLayer::App => &app_buffer,
            DisplayLayer::Notification => &notify_buffer,
        };

        match select3(
            buffer.wait_for_update(),
            layer_sub.changed(),
            bright_sub.changed(),
        )
        .await
        {
            Either3::First(graphics) => {
                display.update(&graphics, brightness).await;
            }
            Either3::Second(layer) => {
                let buffer = match layer {
                    DisplayLayer::App => &app_buffer,
                    DisplayLayer::Notification => &notify_buffer,
                };
                display.update(&buffer.get(), brightness).await;
            }
            Either3::Third(new_brightness) => {
                display.update(&buffer.get(), new_brightness).await;
            }
        };
    }
}
