//! Lightweight always-on-top windows for keeping a captured selection visible.

use std::sync::Arc;

use gpui::{
    FocusHandle, Focusable, KeyDownEvent, Render, Window, WindowControlArea, div, img, prelude::*,
    px,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::{
    platform::{
        capture::CaptureFrame,
        clipboard::{ClipboardService, SystemClipboard},
    },
    theme::ThemeColors,
};

pub(super) struct PinnedImage {
    image: Arc<gpui::RenderImage>,
    frame: CaptureFrame,
    focus_handle: FocusHandle,
    topmost_requested: bool,
    status: &'static str,
}

impl PinnedImage {
    pub(super) fn new(
        image: Arc<gpui::RenderImage>,
        frame: CaptureFrame,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        Self {
            image,
            frame,
            focus_handle: cx.focus_handle(),
            topmost_requested: false,
            status: "Pinned capture",
        }
    }

    fn copy_image(&mut self, cx: &mut gpui::Context<Self>) {
        self.status = match copy_pinned_image(&self.frame, &SystemClipboard) {
            Ok(()) => "Copied image",
            Err(error) => {
                log::warn!(target: "flash_shot::pinned", "pinned_window_copy_failed error={error}");
                "Could not copy image"
            }
        };
        cx.notify();
    }

    /// Closes this independent pinned window without affecting the capture service.
    fn close(&mut self, window: &mut Window) {
        window.remove_window();
    }
}

fn copy_pinned_image(
    frame: &CaptureFrame,
    clipboard: &impl ClipboardService,
) -> std::io::Result<()> {
    clipboard.copy_image(frame)
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
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, _| {
                if pinned_close_key(&event.keystroke.key) {
                    this.close(window);
                }
            }))
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
                    .child(div().text_xs().text_color(colors.muted).child(self.status))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                div()
                                    .id("pinned-copy")
                                    .px_2()
                                    .text_color(colors.text)
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| this.copy_image(cx)))
                                    .child("Copy"),
                            )
                            .child(
                                div()
                                    .id("pinned-close")
                                    .px_2()
                                    .text_color(colors.text)
                                    .cursor_pointer()
                                    .window_control_area(WindowControlArea::Close)
                                    .on_click(cx.listener(|this, _, window, _| this.close(window)))
                                    .child("Close"),
                            ),
                    ),
            )
            .child(img(self.image.clone()).size_full())
    }
}

/// Keeps the local Escape shortcut separate from text or capture shortcuts.
fn pinned_close_key(key: &str) -> bool {
    key == "escape"
}

#[cfg(test)]
mod tests {
    use super::{copy_pinned_image, pinned_close_key};
    use crate::{
        domain::geometry::PhysicalRect,
        platform::{
            capture::{CaptureFrame, PixelFormat},
            clipboard::ClipboardService,
        },
    };
    use std::{cell::RefCell, io, sync::Arc, time::Duration};

    #[derive(Default)]
    struct RecordingClipboard(RefCell<Option<CaptureFrame>>);

    impl ClipboardService for RecordingClipboard {
        fn copy_image(&self, frame: &CaptureFrame) -> io::Result<()> {
            self.0.replace(Some(frame.clone()));
            Ok(())
        }

        fn copy_text(&self, _text: &str) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn pinned_image_copy_keeps_the_composited_frame_intact() {
        let frame = CaptureFrame {
            bounds: PhysicalRect {
                left: 0,
                top: 0,
                right: 2,
                bottom: 1,
            },
            width: 2,
            height: 1,
            stride: 8,
            format: PixelFormat::Bgra8,
            pixels: Arc::from([1, 2, 3, 255, 4, 5, 6, 255]),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 2,
        };
        let clipboard = RecordingClipboard::default();

        copy_pinned_image(&frame, &clipboard).unwrap();

        let copied = clipboard.0.borrow();
        let copied = copied.as_ref().unwrap();
        assert_eq!(copied.bounds, frame.bounds);
        assert_eq!(copied.pixels.as_ref(), frame.pixels.as_ref());
        assert_eq!(copied.cpu_copy_count, frame.cpu_copy_count);
    }

    #[test]
    fn escape_is_the_only_keyboard_close_command() {
        assert!(pinned_close_key("escape"));
        assert!(!pinned_close_key("enter"));
        assert!(!pinned_close_key("shift-escape"));
    }
}
