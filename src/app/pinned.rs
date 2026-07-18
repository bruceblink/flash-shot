//! Lightweight always-on-top windows for keeping a captured selection visible.

use std::sync::Arc;

use gpui::{FocusHandle, Focusable, Render, Window, WindowControlArea, div, img, prelude::*, px};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::theme::ThemeColors;

pub(super) struct PinnedImage {
    image: Arc<gpui::RenderImage>,
    focus_handle: FocusHandle,
    topmost_requested: bool,
}

impl PinnedImage {
    pub(super) fn new(image: Arc<gpui::RenderImage>, cx: &mut gpui::Context<Self>) -> Self {
        Self {
            image,
            focus_handle: cx.focus_handle(),
            topmost_requested: false,
        }
    }
}

impl Focusable for PinnedImage {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for PinnedImage {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let colors = ThemeColors::default();
        if !self.topmost_requested
            && let Ok(handle) = window.window_handle()
            && let RawWindowHandle::Win32(handle) = handle.as_raw()
        {
            self.topmost_requested = true;
            if let Err(error) = crate::platform::window_visibility::make_topmost(handle.hwnd.get())
            {
                log::warn!(target: "flash_shot::pinned", "pinned_window_topmost_failed error={error}");
            }
        }
        div()
            .size_full()
            .track_focus(&self.focus_handle)
            .bg(colors.background)
            .border_1()
            .border_color(colors.border)
            .child(
                div()
                    .h(px(26.0))
                    .px_2()
                    .flex()
                    .items_center()
                    .justify_between()
                    .bg(colors.panel)
                    .window_control_area(WindowControlArea::Drag)
                    .child(
                        div()
                            .text_xs()
                            .text_color(colors.muted)
                            .child("Pinned capture"),
                    )
                    .child(
                        div()
                            .id("pinned-close")
                            .px_2()
                            .text_color(colors.text)
                            .cursor_pointer()
                            .on_click(cx.listener(|_, _, window, _| window.remove_window()))
                            .child("Close"),
                    ),
            )
            .child(img(self.image.clone()).size_full())
    }
}
