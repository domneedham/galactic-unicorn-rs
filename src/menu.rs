use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::Timer;
use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyle},
    pixelcolor::Rgb888,
    prelude::*,
    text::{Alignment, Baseline, Text, TextStyleBuilder},
};

use crate::app::{Apps, DisplayLayer, ACTIVE_LAYER};
use crate::display::{DisplayState, GraphicsBufferWriter, HEIGHT, WIDTH};

/// State of the menu system
pub struct MenuState {
    pub is_open: bool,
    pub selected_index: usize,
}

impl MenuState {
    pub const fn new() -> Self {
        Self {
            is_open: false,
            selected_index: 0,
        }
    }
}

/// List of apps that appear in the menu (excludes System app)
const MENU_APPS: &[Apps] = &[Apps::Clock, Apps::Effects, Apps::Mqtt, Apps::Draw];

/// Menu controller - manages menu state and rendering
pub struct MenuController {
    state: Mutex<ThreadModeRawMutex, MenuState>,
    graphics_buffer: Mutex<ThreadModeRawMutex, Option<GraphicsBufferWriter>>,
    display_state: &'static DisplayState,
}

impl MenuController {
    pub fn new(
        display_state: &'static DisplayState,
        graphics_buffer: GraphicsBufferWriter,
    ) -> Self {
        Self {
            state: Mutex::new(MenuState::new()),
            graphics_buffer: Mutex::new(Some(graphics_buffer)),
            display_state,
        }
    }

    /// Check if menu is currently open
    pub async fn is_open(&self) -> bool {
        self.state.lock().await.is_open
    }

    /// Open the menu
    pub async fn open(&self) {
        {
            let mut state = self.state.lock().await;
            state.is_open = true;
        }

        ACTIVE_LAYER.sender().send(DisplayLayer::Menu);
        self.render().await;
    }

    /// Close the menu
    pub async fn close(&self) {
        {
            let mut state = self.state.lock().await;
            state.is_open = false;
        }

        ACTIVE_LAYER.sender().send(DisplayLayer::App);
    }

    /// Move selection to previous item (with wraparound) - animated
    pub async fn select_previous(&self) {
        let old_index = {
            let mut state = self.state.lock().await;
            let old = state.selected_index;
            if state.selected_index == 0 {
                state.selected_index = MENU_APPS.len() - 1;
            } else {
                state.selected_index -= 1;
            }
            old
        };

        // Animate sliding up (old slides up, new slides in from below)
        self.render_transition(old_index, true).await;
    }

    /// Move selection to next item (with wraparound) - animated
    pub async fn select_next(&self) {
        let old_index = {
            let mut state = self.state.lock().await;
            let old = state.selected_index;
            state.selected_index = (state.selected_index + 1) % MENU_APPS.len();
            old
        };

        // Animate sliding down (old slides down, new slides in from above)
        self.render_transition(old_index, false).await;
    }

    /// Get the currently selected app
    pub async fn get_selected_app(&self) -> Apps {
        let state = self.state.lock().await;
        MENU_APPS[state.selected_index]
    }

    /// Render the menu with animation transition
    async fn render_transition(&self, old_index: usize, going_up: bool) {
        let buffer_lock = self.graphics_buffer.lock().await;
        let graphics_buffer = match buffer_lock.as_ref() {
            Some(buf) => buf,
            None => return,
        };

        let selected_index = self.state.lock().await.selected_index;

        // Get the active color
        let mut color_sub = self.display_state.color.receiver().unwrap();
        let active_color = color_sub.try_get().unwrap_or(Rgb888::WHITE);

        let old_app_name: &str = MENU_APPS[old_index].into();
        let new_app_name: &str = MENU_APPS[selected_index].into();

        // Animate over 5 frames (about 50ms)
        for frame in 0..=5 {
            graphics_buffer.clear().await;

            let mut pixels = graphics_buffer.pixels_mut().await;
            let text_style = MonoTextStyle::new(&FONT_6X10, active_color);
            let layout_style = TextStyleBuilder::new()
                .alignment(Alignment::Center)
                .baseline(Baseline::Middle)
                .build();

            // Calculate offsets for slide animation
            let progress = frame as i32 * (HEIGHT as i32) / 5;

            if going_up {
                // Old text slides up and out
                let old_y = (HEIGHT as i32 / 2) - progress;
                if old_y >= -(FONT_6X10.character_size.height as i32) && old_y < HEIGHT as i32 {
                    let _ = Text::with_text_style(
                        old_app_name,
                        Point::new((WIDTH / 2) as i32, old_y),
                        text_style,
                        layout_style,
                    )
                    .draw(&mut *pixels);
                }

                // New text slides in from below
                let new_y = (HEIGHT as i32 / 2) + (HEIGHT as i32 - progress);
                if new_y >= 0 && new_y < HEIGHT as i32 + FONT_6X10.character_size.height as i32 {
                    let _ = Text::with_text_style(
                        new_app_name,
                        Point::new((WIDTH / 2) as i32, new_y),
                        text_style,
                        layout_style,
                    )
                    .draw(&mut *pixels);
                }
            } else {
                // Old text slides down and out
                let old_y = (HEIGHT as i32 / 2) + progress;
                if old_y >= 0 && old_y < HEIGHT as i32 + FONT_6X10.character_size.height as i32 {
                    let _ = Text::with_text_style(
                        old_app_name,
                        Point::new((WIDTH / 2) as i32, old_y),
                        text_style,
                        layout_style,
                    )
                    .draw(&mut *pixels);
                }

                // New text slides in from above
                let new_y = (HEIGHT as i32 / 2) - (HEIGHT as i32 - progress);
                if new_y >= -(FONT_6X10.character_size.height as i32) && new_y < HEIGHT as i32 {
                    let _ = Text::with_text_style(
                        new_app_name,
                        Point::new((WIDTH / 2) as i32, new_y),
                        text_style,
                        layout_style,
                    )
                    .draw(&mut *pixels);
                }
            }

            pixels.mark_all_dirty();
            drop(pixels);
            graphics_buffer.send();

            // Small delay between frames for smooth animation
            if frame < 5 {
                Timer::after_millis(10).await;
            }
        }
    }

    /// Render the menu (static, no animation)
    async fn render(&self) {
        let buffer_lock = self.graphics_buffer.lock().await;
        let graphics_buffer = match buffer_lock.as_ref() {
            Some(buf) => buf,
            None => return,
        };

        graphics_buffer.clear().await;

        let selected_index = self.state.lock().await.selected_index;

        // Get the active color
        let mut color_sub = self.display_state.color.receiver().unwrap();
        let active_color = color_sub.try_get().unwrap_or(Rgb888::WHITE);

        let app_name: &str = MENU_APPS[selected_index].into();

        let mut pixels = graphics_buffer.pixels_mut().await;
        let text_style = MonoTextStyle::new(&FONT_6X10, active_color);
        let layout_style = TextStyleBuilder::new()
            .alignment(Alignment::Center)
            .baseline(Baseline::Middle)
            .build();

        // Draw app name centered on screen (both horizontally and vertically)
        let _ = Text::with_text_style(
            app_name,
            Point::new((WIDTH / 2) as i32, (HEIGHT / 2) as i32),
            text_style,
            layout_style,
        )
        .draw(&mut *pixels);

        pixels.mark_all_dirty();
        drop(pixels);
        graphics_buffer.send();
    }
}
