use embassy_futures::select::{select, Either};
use embassy_rp::{
    gpio::Input,
    peripherals::{PIN_0, PIN_1, PIN_21, PIN_26, PIN_3, PIN_6},
    Peri,
};
use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, pubsub::Publisher};
use embassy_time::{Duration, Timer};
use galactic_unicorn_embassy::buttons::UnicornButtons;

/// Type of button press made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonPress {
    /// When the button click duration is <=500ms.
    Short,

    /// When the button click duration is >500ms.
    Long,

    /// When the button click duration is <=500ms and a second click happens in the next 300ms.
    Double,
}

/// Wait for changes async on the brightness up button being pressed.
///
/// Will inform signal of button press after the full press has been completed.
/// The type of press is recorded in the ButtonPress enum.
///
/// This task has no way of cancellation.
#[embassy_executor::task]
pub async fn brightness_up_task(
    button_peri: Peri<'static, PIN_21>,
    publisher: Publisher<'static, ThreadModeRawMutex, (UnicornButtons, ButtonPress), 4, 1, 9>,
) -> ! {
    let mut button = Input::new(button_peri, embassy_rp::gpio::Pull::Up);
    loop {
        // sit here until button is pressed down
        button.wait_for_low().await;

        let press: ButtonPress = button_pressed(&mut button).await;
        publisher
            .publish((UnicornButtons::BrightnessUp, press))
            .await;

        // wait for button to be released
        if button.is_low() {
            button.wait_for_high().await;
        }

        // add debounce
        Timer::after(Duration::from_millis(200)).await;
    }
}

/// Wait for changes async on the brightness down button being pressed.
///
/// Will inform signal of button press after the full press has been completed.
/// The type of press is recorded in the ButtonPress enum.
///
/// This task has no way of cancellation.
#[embassy_executor::task]
pub async fn brightness_down_task(
    button_peri: Peri<'static, PIN_26>,
    publisher: Publisher<'static, ThreadModeRawMutex, (UnicornButtons, ButtonPress), 4, 1, 9>,
) -> ! {
    let mut button = Input::new(button_peri, embassy_rp::gpio::Pull::Up);
    loop {
        // sit here until button is pressed down
        button.wait_for_low().await;

        let press: ButtonPress = button_pressed(&mut button).await;
        publisher
            .publish((UnicornButtons::BrightnessDown, press))
            .await;

        // wait for button to be released
        if button.is_low() {
            button.wait_for_high().await;
        }

        // add debounce
        Timer::after(Duration::from_millis(200)).await;
    }
}

/// Wait for changes async on the switch a button being pressed.
///
/// Will inform signal of button press after the full press has been completed.
/// The type of press is recorded in the ButtonPress enum.
///
/// This task has no way of cancellation.
#[embassy_executor::task]
pub async fn button_a_task(
    button_peri: Peri<'static, PIN_0>,
    publisher: Publisher<'static, ThreadModeRawMutex, (UnicornButtons, ButtonPress), 4, 1, 9>,
) -> ! {
    let mut button = Input::new(button_peri, embassy_rp::gpio::Pull::Up);
    loop {
        // sit here until button is pressed down
        button.wait_for_low().await;

        let press: ButtonPress = button_pressed(&mut button).await;
        publisher.publish((UnicornButtons::SwitchA, press)).await;

        // wait for button to be released
        if button.is_low() {
            button.wait_for_high().await;
        }

        // add debounce
        Timer::after(Duration::from_millis(200)).await;
    }
}

/// Wait for changes async on the switch b button being pressed.
///
/// Will inform signal of button press after the full press has been completed.
/// The type of press is recorded in the ButtonPress enum.
///
/// This task has no way of cancellation.
#[embassy_executor::task]
pub async fn button_b_task(
    button_peri: Peri<'static, PIN_1>,
    publisher: Publisher<'static, ThreadModeRawMutex, (UnicornButtons, ButtonPress), 4, 1, 9>,
) -> ! {
    let mut button = Input::new(button_peri, embassy_rp::gpio::Pull::Up);
    loop {
        // sit here until button is pressed down
        button.wait_for_low().await;

        let press: ButtonPress = button_pressed(&mut button).await;
        publisher.publish((UnicornButtons::SwitchB, press)).await;

        // wait for button to be released
        if button.is_low() {
            button.wait_for_high().await;
        }

        // add debounce
        Timer::after(Duration::from_millis(200)).await;
    }
}

/// Wait for changes async on the switch c button being pressed.
///
/// Will inform signal of button press after the full press has been completed.
/// The type of press is recorded in the ButtonPress enum.
///
/// This task has no way of cancellation.
#[embassy_executor::task]
pub async fn button_c_task(
    button_peri: Peri<'static, PIN_3>,
    publisher: Publisher<'static, ThreadModeRawMutex, (UnicornButtons, ButtonPress), 4, 1, 9>,
) -> ! {
    let mut button = Input::new(button_peri, embassy_rp::gpio::Pull::Up);
    loop {
        // sit here until button is pressed down
        button.wait_for_low().await;

        let press: ButtonPress = button_pressed(&mut button).await;
        publisher.publish((UnicornButtons::SwitchC, press)).await;

        // wait for button to be released
        if button.is_low() {
            button.wait_for_high().await;
        }

        // add debounce
        Timer::after(Duration::from_millis(200)).await;
    }
}

/// Wait for changes async on the switch d button being pressed.
///
/// Will inform signal of button press after the full press has been completed.
/// The type of press is recorded in the ButtonPress enum.
///
/// This task has no way of cancellation.
#[embassy_executor::task]
pub async fn button_d_task(
    button_peri: Peri<'static, PIN_6>,
    publisher: Publisher<'static, ThreadModeRawMutex, (UnicornButtons, ButtonPress), 4, 1, 9>,
) -> ! {
    let mut button = Input::new(button_peri, embassy_rp::gpio::Pull::Up);
    loop {
        // sit here until button is pressed down
        button.wait_for_low().await;

        let press: ButtonPress = button_pressed(&mut button).await;
        publisher.publish((UnicornButtons::SwitchD, press)).await;

        // wait for button to be released
        if button.is_low() {
            button.wait_for_high().await;
        }

        // add debounce
        Timer::after(Duration::from_millis(200)).await;
    }
}

/// Determine the type of press performed on the button.
async fn button_pressed(button: &mut Input<'_>) -> ButtonPress {
    // wait until button is released or 500ms (long press)
    let res = select(
        button.wait_for_high(),
        Timer::after(Duration::from_millis(500)),
    )
    .await;

    match res {
        // button is released before 500ms
        Either::First(_) => {
            // add debounce
            Timer::after(Duration::from_millis(50)).await;

            // see if button is pressed down again or 250ms
            let res = select(
                button.wait_for_low(),
                Timer::after(Duration::from_millis(250)),
            )
            .await;

            match res {
                // button is released before 250ms
                Either::First(_) => ButtonPress::Double,
                // 250ms passed by
                Either::Second(_) => ButtonPress::Short,
            }
        }

        // 500ms passed by
        Either::Second(_) => ButtonPress::Long,
    }
}
