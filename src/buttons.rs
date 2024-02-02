use embassy_futures::select::{select, Either};
use embassy_rp::{
    gpio::Input,
    peripherals::{PIN_0, PIN_1, PIN_21, PIN_26},
};
use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, signal::Signal};
use embassy_time::{Duration, Timer};
use galactic_unicorn_embassy::buttons::UnicornButtons;

/// Type of button press made.
pub enum ButtonPress {
    /// When the button click duration is <=500ms.
    Short,

    /// When the button click duration is >500ms.
    Long,

    /// When the button click duration is <=500ms and a second click happens in the next 300ms.
    Double,
}

/// Signal for when the brightness up button has been pressed.
pub static BRIGHTNESS_UP_PRESS: Signal<ThreadModeRawMutex, ButtonPress> = Signal::new();

/// Signal for when the brightness down button has been pressed.
pub static BRIGHTNESS_DOWN_PRESS: Signal<ThreadModeRawMutex, ButtonPress> = Signal::new();

/// Signal for when the switch a button has been pressed.
pub static SWITCH_A_PRESS: Signal<ThreadModeRawMutex, ButtonPress> = Signal::new();

/// Signal for when the switch b button has been pressed.
pub static SWITCH_B_PRESS: Signal<ThreadModeRawMutex, ButtonPress> = Signal::new();

/// Wait for changes async on the brightness up button being pressed.
///
/// Will inform signal of button press after the full press has been completed.
/// The type of press is recorded in the ButtonPress enum.
///
/// This task has no way of cancellation.
#[embassy_executor::task]
pub async fn brightness_up_task(mut button: Input<'static, PIN_21>) -> ! {
    loop {
        // sit here until button is pressed down
        button.wait_for_low().await;

        let press: ButtonPress = button_pressed(&mut button).await;
        publish_to_channel(press, &UnicornButtons::BrightnessUp);

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
pub async fn brightness_down_task(mut button: Input<'static, PIN_26>) -> ! {
    loop {
        // sit here until button is pressed down
        button.wait_for_low().await;

        let press: ButtonPress = button_pressed(&mut button).await;
        publish_to_channel(press, &UnicornButtons::BrightnessDown);

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
pub async fn button_a_task(mut button: Input<'static, PIN_0>) -> ! {
    loop {
        // sit here until button is pressed down
        button.wait_for_low().await;

        let press: ButtonPress = button_pressed(&mut button).await;
        publish_to_channel(press, &UnicornButtons::SwitchA);

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
pub async fn button_b_task(mut button: Input<'static, PIN_1>) -> ! {
    loop {
        // sit here until button is pressed down
        button.wait_for_low().await;

        let press: ButtonPress = button_pressed(&mut button).await;
        publish_to_channel(press, &UnicornButtons::SwitchB);

        // wait for button to be released
        if button.is_low() {
            button.wait_for_high().await;
        }

        // add debounce
        Timer::after(Duration::from_millis(200)).await;
    }
}

/// Determine the type of press performed on the button.
#[allow(clippy::needless_pass_by_ref_mut)] // needs to be mutable to use wait_for_*()
async fn button_pressed<T>(button: &mut Input<'_, T>) -> ButtonPress
where
    T: embassy_rp::gpio::Pin,
{
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

fn publish_to_channel(press: ButtonPress, button_type: &UnicornButtons) {
    match button_type {
        UnicornButtons::SwitchA => SWITCH_A_PRESS.signal(press),
        UnicornButtons::SwitchB => SWITCH_B_PRESS.signal(press),
        UnicornButtons::SwitchC => todo!(),
        UnicornButtons::SwitchD => todo!(),
        UnicornButtons::BrightnessUp => BRIGHTNESS_UP_PRESS.signal(press),
        UnicornButtons::BrightnessDown => BRIGHTNESS_DOWN_PRESS.signal(press),
        UnicornButtons::VolumeUp => todo!(),
        UnicornButtons::VolumeDown => todo!(),
        UnicornButtons::Sleep => todo!(),
    }
}
