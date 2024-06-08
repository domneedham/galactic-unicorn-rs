use embassy_executor::Spawner;
use embassy_futures::select::{select, select3, Either3};
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_sync::pubsub::Subscriber;
use embassy_sync::signal::Signal;
use embassy_time::Duration;

use galactic_unicorn_embassy::{HEIGHT, WIDTH};
use unicorn_graphics::UnicornGraphics;

use crate::buttons::{ButtonPress, SWITCH_A_PRESS, SWITCH_B_PRESS, SWITCH_C_PRESS};
use crate::clock_app::ClockApp;
use crate::effects_app::EffectsApp;
use crate::mqtt::{MqttApp, MqttReceiveMessage, CLOCK_APP_TOPIC, TEXT_TOPIC};
use crate::unicorn;
use crate::unicorn::display::{DisplayGraphicsMessage, DisplayTextMessage};

static CHANGE_APP: Signal<ThreadModeRawMutex, Apps> = Signal::new();

#[derive(Copy, Clone, PartialEq, Eq)]
enum Apps {
    Clock,
    Effects,
    Mqtt,
}

pub trait UnicornApp {
    async fn display(&self);

    async fn start(&self);
    async fn stop(&self);

    async fn button_press(&self, press: ButtonPress);

    async fn process_mqtt_message(&self, message: MqttReceiveMessage);
}

pub struct AppController {
    active_app: Mutex<ThreadModeRawMutex, Apps>,
    clock_app: &'static ClockApp,
    effects_app: &'static EffectsApp,
    mqtt_app: &'static MqttApp,
    spawner: Spawner,
}

impl AppController {
    pub fn new(
        clock_app: &'static ClockApp,
        effects_app: &'static EffectsApp,
        mqtt_app: &'static MqttApp,
        spawner: Spawner,
    ) -> Self {
        Self {
            active_app: Mutex::new(Apps::Clock),
            clock_app,
            effects_app,
            mqtt_app,
            spawner,
        }
    }

    pub async fn run(&'static self) -> ! {
        self.spawner.spawn(display_task(self)).unwrap();
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

            let current_app = *self.active_app.lock().await;
            if app == *self.active_app.lock().await {
                match current_app {
                    Apps::Clock => self.clock_app.button_press(press).await,
                    Apps::Effects => self.effects_app.button_press(press).await,
                    Apps::Mqtt => self.mqtt_app.button_press(press).await,
                }
            } else {
                match current_app {
                    Apps::Clock => self.clock_app.stop().await,
                    Apps::Effects => self.effects_app.stop().await,
                    Apps::Mqtt => self.mqtt_app.stop().await,
                };

                *self.active_app.lock().await = app;
                match app {
                    Apps::Clock => self.clock_app.start().await,
                    Apps::Effects => self.effects_app.start().await,
                    Apps::Mqtt => self.mqtt_app.start().await,
                };
                CHANGE_APP.signal(app);
            }
        }
    }
}

#[embassy_executor::task]
pub async fn process_mqtt_messages_task(
    app_controller: &'static AppController,
    mut subscriber: Subscriber<'static, ThreadModeRawMutex, MqttReceiveMessage, 16, 1, 1>,
) {
    loop {
        let message = subscriber.next_message_pure().await;

        if message.topic.contains(TEXT_TOPIC) {
            DisplayTextMessage::from_mqtt(&message.body, None, None)
                .send()
                .await;
            app_controller.mqtt_app.set_last_message(message.body).await;
        } else if message.topic.contains(CLOCK_APP_TOPIC) {
            app_controller.clock_app.process_mqtt_message(message).await
        }
    }
}

#[embassy_executor::task]
async fn display_task(app_controller: &'static AppController) {
    let mut blank_graphics = UnicornGraphics::<WIDTH, HEIGHT>::new();
    blank_graphics.clear_all();
    loop {
        let app = *app_controller.active_app.lock().await;
        match app {
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
        DisplayGraphicsMessage::from_app(
            blank_graphics.get_pixels(),
            Some(Duration::from_millis(10)),
        )
        .send_and_replace_queue()
        .await;
    }
}
