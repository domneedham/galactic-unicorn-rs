use core::str::FromStr;

use embassy_executor::Spawner;
use embassy_futures::select::{select, select3, Either3};
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_sync::pubsub::Subscriber;
use embassy_sync::signal::Signal;
use embassy_time::Duration;

use galactic_unicorn_embassy::{HEIGHT, WIDTH};
use static_cell::make_static;
use strum_macros::{EnumString, IntoStaticStr};
use unicorn_graphics::UnicornGraphics;

use crate::buttons::{ButtonPress, SWITCH_A_PRESS, SWITCH_B_PRESS, SWITCH_C_PRESS};
use crate::clock_app::ClockApp;
use crate::effects_app::EffectsApp;
use crate::mqtt::topics::APP_STATE_TOPIC;
use crate::mqtt::{
    topics::{APP_SET_TOPIC, CLOCK_APP_SET_TOPIC, TEXT_SET_TOPIC},
    MqttMessage, MqttReceiveMessage,
};
use crate::mqtt_app::MqttApp;
use crate::network::NetworkState;
use crate::system::{StateUpdates, SystemState, STATE_CHANGED};
use crate::system_app::SystemApp;
use crate::unicorn;
use crate::unicorn::display::{DisplayGraphicsMessage, DisplayTextMessage};

/// Signal for an app change for the display task.
static CHANGE_APP: Signal<ThreadModeRawMutex, Apps> = Signal::new();

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
}

pub trait UnicornApp {
    /// The main display loop for this app.
    async fn display(&self);

    /// Start the app. Is called just before display.
    async fn start(&self);

    /// Stop the app. Is called just before display is cancelled.
    async fn stop(&self);

    /// Handle a user button press for this app.
    async fn button_press(&self, press: ButtonPress);

    /// Process MQTT messages for this app. Can be called whilst not active.
    async fn process_mqtt_message(&self, message: MqttReceiveMessage);

    /// Send MQTT state for this app. Can be called whilst not active.
    async fn send_mqtt_state(&self);
}

/// App controller is responsible for managing apps by:
/// - Starting and stopping apps on user selection
/// - Starting and stopping apps from MQTT
/// - Forwarding button presses to active apps
pub struct AppController {
    /// The current active app.
    active_app: Mutex<ThreadModeRawMutex, Apps>,

    /// The previous active app.
    previous_app: Mutex<ThreadModeRawMutex, Apps>,

    /// System app.
    system_app: &'static SystemApp,

    /// Clock app.
    clock_app: &'static ClockApp,

    /// Effects app.
    effects_app: &'static EffectsApp,

    /// MQTT app.
    mqtt_app: &'static MqttApp,

    /// System state.
    system_state: &'static SystemState,

    /// Embassy spawner.
    spawner: Spawner,
}

impl AppController {
    /// Create the static ref to app controller.
    /// Must only be called once or will panic.
    pub fn new(
        system_app: &'static SystemApp,
        clock_app: &'static ClockApp,
        effects_app: &'static EffectsApp,
        mqtt_app: &'static MqttApp,
        system_state: &'static SystemState,
        spawner: Spawner,
    ) -> &'static Self {
        let controller = make_static!(Self {
            active_app: Mutex::new(Apps::System),
            previous_app: Mutex::new(Apps::Clock),
            system_app,
            clock_app,
            effects_app,
            mqtt_app,
            system_state,
            spawner,
        });

        controller.init();

        controller
    }

    /// Start the embassy tasks.
    fn init(&'static self) {
        self.spawner.spawn(display_task(self)).unwrap();
        self.spawner.spawn(process_state_change_task(self)).unwrap();
    }

    /// The main program loop.
    pub async fn run_forever(&'static self) -> ! {
        loop {
            let (app, press): (Apps, ButtonPress) = match select3(
                SWITCH_A_PRESS.wait(),
                SWITCH_B_PRESS.wait(),
                SWITCH_C_PRESS.wait(),
            )
            .await
            {
                Either3::First(press) => (Apps::Clock, press),
                Either3::Second(press) => (Apps::Effects, press),
                Either3::Third(press) => (Apps::Mqtt, press),
            };

            if app == *self.active_app.lock().await {
                let current_app = *self.active_app.lock().await;

                match current_app {
                    Apps::System => self.system_app.button_press(press).await,
                    Apps::Clock => self.clock_app.button_press(press).await,
                    Apps::Effects => self.effects_app.button_press(press).await,
                    Apps::Mqtt => self.mqtt_app.button_press(press).await,
                }
            } else {
                self.change_app(app).await;
            }

            self.send_mqtt_states().await;
        }
    }

    /// Send MQTT states from each app.
    pub async fn send_mqtt_states(&self) {
        let active_app = *self.active_app.lock().await;
        let app_text = active_app.into();
        MqttMessage::enqueue_state(APP_STATE_TOPIC, app_text).await;

        self.clock_app.send_mqtt_state().await;
        self.effects_app.send_mqtt_state().await;
        self.mqtt_app.send_mqtt_state().await;
    }

    /// Change the current app by stopping the current and starting the new chosen app.
    async fn change_app(&self, new_app: Apps) {
        let mut current_app = *self.active_app.lock().await;

        if current_app == new_app {
            return;
        }

        match current_app {
            Apps::System => {
                self.system_app.stop().await;
                current_app = Apps::Clock
            }
            Apps::Clock => self.clock_app.stop().await,
            Apps::Effects => self.effects_app.stop().await,
            Apps::Mqtt => self.mqtt_app.stop().await,
        };

        *self.previous_app.lock().await = current_app;
        *self.active_app.lock().await = new_app;
        match new_app {
            Apps::System => self.system_app.start().await,
            Apps::Clock => self.clock_app.start().await,
            Apps::Effects => self.effects_app.start().await,
            Apps::Mqtt => self.mqtt_app.start().await,
        };
        CHANGE_APP.signal(new_app);
    }
}

/// Process MQTT messages related to app functionality.
#[embassy_executor::task]
pub async fn process_mqtt_messages_task(
    app_controller: &'static AppController,
    mut subscriber: Subscriber<'static, ThreadModeRawMutex, MqttReceiveMessage, 8, 1, 1>,
) {
    loop {
        let message = subscriber.next_message_pure().await;

        if message.topic == TEXT_SET_TOPIC {
            DisplayTextMessage::from_mqtt(&message.body, None, None)
                .send()
                .await;
            app_controller.mqtt_app.set_last_message(message.body).await;
        } else if message.topic == CLOCK_APP_SET_TOPIC {
            app_controller.clock_app.process_mqtt_message(message).await;
        } else if message.topic == APP_SET_TOPIC {
            if let Ok(new_app) = Apps::from_str(&message.body) {
                app_controller.change_app(new_app).await;
            }
        }

        app_controller.send_mqtt_states().await;
    }
}

/// Process state changes from app state.
#[embassy_executor::task]
async fn process_state_change_task(app_controller: &'static AppController) {
    loop {
        let state_update = STATE_CHANGED.wait().await;

        MqttMessage::enqueue_debug("State changed").await;

        match state_update {
            StateUpdates::Network => {
                match app_controller.system_state.get_network_state().await {
                    NetworkState::NotInitialised => {}
                    NetworkState::Connected => {
                        let previous_app = *app_controller.previous_app.lock().await;
                        app_controller.change_app(previous_app).await;
                    }
                    NetworkState::Error => app_controller.change_app(Apps::System).await,
                };
            }
        }
    }
}

/// Run the display function of the active app.  
#[embassy_executor::task]
async fn display_task(app_controller: &'static AppController) {
    let mut blank_graphics = UnicornGraphics::<WIDTH, HEIGHT>::new();
    blank_graphics.clear_all();
    loop {
        let app = *app_controller.active_app.lock().await;
        match app {
            Apps::System => {
                select(app_controller.system_app.display(), CHANGE_APP.wait()).await;
            }
            Apps::Clock => {
                select(app_controller.clock_app.display(), CHANGE_APP.wait()).await;
            }
            Apps::Effects => {
                select(app_controller.effects_app.display(), CHANGE_APP.wait()).await;
            }
            Apps::Mqtt => {
                select(app_controller.mqtt_app.display(), CHANGE_APP.wait()).await;
            }
        };

        unicorn::display::STOP_CURRENT_DISPLAY.signal(true);
        // when switching between apps we want to clear the old queue and blank the display ..
        DisplayGraphicsMessage::from_app(blank_graphics.get_pixels(), Duration::from_millis(10))
            .send_and_replace_queue()
            .await;
    }
}
