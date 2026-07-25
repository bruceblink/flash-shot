//! Lightweight always-on-top windows for keeping a captured selection visible.

use std::sync::Arc;

use gpui::{
    Context, Entity, FocusHandle, Focusable, KeyDownEvent, Keystroke, Render, Subscription, Window,
    div, img, prelude::*, px,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::FlashShotApp;
use crate::platform::{
    capture::CaptureFrame,
    clipboard::{ClipboardService, SystemClipboard},
};

const PIN_OPACITY_STEPS: [u8; 4] = [255, 191, 128, 64];

struct PinnedTooltip(&'static str, crate::theme::ThemeColors);

impl Render for PinnedTooltip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .bg(self.1.panel)
            .border_1()
            .border_color(self.1.border)
            .text_color(self.1.text)
            .text_xs()
            .child(self.0)
    }
}

/// Describes each compact pin control without requiring the image window to stay large.
fn pinned_control_tooltip(control: &str) -> &'static str {
    match control {
        "zoom-out" => "Zoom out (Ctrl+-)",
        "zoom-in" => "Zoom in (Ctrl++)",
        "opacity" => "Cycle opacity (Ctrl+O)",
        "copy" => "Copy image (Ctrl+C)",
        "save" => "Save image (Ctrl+S)",
        "close" => "Close pinned image (Escape)",
        _ => "",
    }
}

pub(super) struct PinnedImage {
    image: Arc<gpui::RenderImage>,
    frame: CaptureFrame,
    app: Entity<FlashShotApp>,
    focus_handle: FocusHandle,
    topmost_requested: bool,
    opacity: u8,
    status: &'static str,
    _app_observation: Subscription,
}

impl PinnedImage {
    pub(super) fn new(
        image: Arc<gpui::RenderImage>,
        frame: CaptureFrame,
        app: Entity<FlashShotApp>,
        cx: &mut Context<Self>,
    ) -> Self {
        let observation = cx.observe(&app, |_, _, cx| cx.notify());
        Self {
            image,
            frame,
            app,
            focus_handle: cx.focus_handle(),
            topmost_requested: false,
            opacity: 255,
            status: "Pinned capture",
            _app_observation: observation,
        }
    }

    fn copy_image(&mut self, cx: &mut Context<Self>) {
        self.status = match copy_pinned_image(&self.frame, &SystemClipboard) {
            Ok(()) => "Copied image",
            Err(error) => {
                log::warn!(target: "flash_shot::pinned", "pinned_window_copy_failed error={error}");
                "Could not copy image"
            }
        };
        cx.notify();
    }

    /// Delegates the file write to the capture service so history ownership stays centralized.
    fn save_image(&mut self, cx: &mut Context<Self>) {
        let frame = self.frame.clone();
        self.app
            .update(cx, |app, cx| app.quick_save_pinned_frame(frame, cx));
        self.status = "Saving image...";
        cx.notify();
    }

    /// Scales the complete native window so the contained image remains undistorted.
    fn zoom(&mut self, scale: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.status = match window.window_handle() {
            Ok(handle) => match handle.as_raw() {
                RawWindowHandle::Win32(handle) => {
                    match crate::platform::window_visibility::resize_centered(
                        handle.hwnd.get(),
                        scale,
                    ) {
                        Ok(()) if scale > 1.0 => "Zoomed in",
                        Ok(()) => "Zoomed out",
                        Err(error) => {
                            log::warn!(target: "flash_shot::pinned", "pinned_window_zoom_failed error={error}");
                            "Could not resize window"
                        }
                    }
                }
                _ => "Window zoom is unavailable",
            },
            Err(error) => {
                log::warn!(target: "flash_shot::pinned", "pinned_window_handle_failed error={error}");
                "Could not resize window"
            }
        };
        cx.notify();
    }

    /// Cycles through readable reference-image opacity levels without moving the window.
    fn cycle_opacity(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next = next_pin_opacity(self.opacity);
        self.status = match window.window_handle() {
            Ok(handle) => match handle.as_raw() {
                RawWindowHandle::Win32(handle) => {
                    match crate::platform::window_visibility::set_opacity(handle.hwnd.get(), next) {
                        Ok(()) => {
                            self.opacity = next;
                            pin_opacity_label(next)
                        }
                        Err(error) => {
                            log::warn!(target: "flash_shot::pinned", "pinned_window_opacity_failed error={error}");
                            "Could not change opacity"
                        }
                    }
                }
                _ => "Window opacity is unavailable",
            },
            Err(error) => {
                log::warn!(target: "flash_shot::pinned", "pinned_window_handle_failed error={error}");
                "Could not change opacity"
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
        let colors = self.app.read(cx).colors;
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
            .flex()
            .flex_col()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                match pinned_keyboard_command(&event.keystroke) {
                    Some(PinnedKeyboardCommand::Close) => this.close(window),
                    Some(PinnedKeyboardCommand::Copy) => this.copy_image(cx),
                    Some(PinnedKeyboardCommand::Save) => this.save_image(cx),
                    Some(PinnedKeyboardCommand::ZoomOut) => this.zoom(0.8, window, cx),
                    Some(PinnedKeyboardCommand::ZoomIn) => this.zoom(1.25, window, cx),
                    Some(PinnedKeyboardCommand::CycleOpacity) => this.cycle_opacity(window, cx),
                    None => {}
                }
            }))
            .bg(colors.background)
            .border_1()
            .border_color(colors.border)
            .child(
                div()
                    .id("pinned-toolbar")
                    .h(px(32.0))
                    .px_3()
                    .flex()
                    .items_center()
                    .bg(colors.panel)
                    .child(
                        // Native title-bar dragging provides the window move affordance.
                        // Keeping this status area client-only avoids turning toolbar clicks
                        // into non-client hit tests on Windows.
                        div()
                            .id("pinned-drag-area")
                            .flex_1()
                            .h_full()
                            .flex()
                            .items_center()
                            .child(div().text_xs().text_color(colors.muted).child(self.status)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .id("pinned-save")
                                    .px_3()
                                    .py_1()
                                    .bg(colors.background)
                                    .border_1()
                                    .border_color(colors.border)
                                    .text_color(colors.text)
                                    .text_xs()
                                    .cursor_pointer()
                                    .tooltip(move |_, cx| {
                                        cx.new(|_| {
                                            PinnedTooltip(pinned_control_tooltip("save"), colors)
                                        })
                                        .into()
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| this.save_image(cx)))
                                    .child("Save"),
                            )
                            .child(
                                div()
                                    .id("pinned-zoom-out")
                                    .w(px(24.0))
                                    .py_1()
                                    .bg(colors.background)
                                    .border_1()
                                    .border_color(colors.border)
                                    .text_color(colors.text)
                                    .text_xs()
                                    .cursor_pointer()
                                    .tooltip(move |_, cx| {
                                        cx.new(|_| {
                                            PinnedTooltip(
                                                pinned_control_tooltip("zoom-out"),
                                                colors,
                                            )
                                        })
                                        .into()
                                    })
                                    .on_click(
                                        cx.listener(|this, _, window, cx| {
                                            this.zoom(0.8, window, cx)
                                        }),
                                    )
                                    .child("-"),
                            )
                            .child(
                                div()
                                    .id("pinned-zoom-in")
                                    .w(px(24.0))
                                    .py_1()
                                    .bg(colors.background)
                                    .border_1()
                                    .border_color(colors.border)
                                    .text_color(colors.text)
                                    .text_xs()
                                    .cursor_pointer()
                                    .tooltip(move |_, cx| {
                                        cx.new(|_| {
                                            PinnedTooltip(pinned_control_tooltip("zoom-in"), colors)
                                        })
                                        .into()
                                    })
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.zoom(1.25, window, cx)
                                    }))
                                    .child("+"),
                            )
                            .child(
                                div()
                                    .id("pinned-opacity")
                                    .w(px(40.0))
                                    .py_1()
                                    .bg(colors.background)
                                    .border_1()
                                    .border_color(colors.border)
                                    .text_color(colors.text)
                                    .text_xs()
                                    .cursor_pointer()
                                    .tooltip(move |_, cx| {
                                        cx.new(|_| {
                                            PinnedTooltip(pinned_control_tooltip("opacity"), colors)
                                        })
                                        .into()
                                    })
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.cycle_opacity(window, cx)
                                    }))
                                    .child(format!("{}%", opacity_percentage(self.opacity))),
                            )
                            .child(
                                div()
                                    .id("pinned-copy")
                                    .px_3()
                                    .py_1()
                                    .bg(colors.background)
                                    .border_1()
                                    .border_color(colors.border)
                                    .text_color(colors.text)
                                    .text_xs()
                                    .cursor_pointer()
                                    .tooltip(move |_, cx| {
                                        cx.new(|_| {
                                            PinnedTooltip(pinned_control_tooltip("copy"), colors)
                                        })
                                        .into()
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| this.copy_image(cx)))
                                    .child("Copy"),
                            )
                            .child(
                                div()
                                    .id("pinned-close")
                                    .w(px(24.0))
                                    .py_1()
                                    .bg(colors.background)
                                    .border_1()
                                    .border_color(colors.border)
                                    .text_color(colors.text)
                                    .text_xs()
                                    .cursor_pointer()
                                    .tooltip(move |_, cx| {
                                        cx.new(|_| {
                                            PinnedTooltip(pinned_control_tooltip("close"), colors)
                                        })
                                        .into()
                                    })
                                    .on_click(cx.listener(|this, _, window, _| this.close(window)))
                                    .child("X"),
                            ),
                    ),
            )
            .child(
                div()
                    .id("pinned-image")
                    .flex_1()
                    .bg(colors.background)
                    .child(img(self.image.clone()).size_full()),
            )
    }
}

/// Keeps the local Escape shortcut separate from text or capture shortcuts.
fn pinned_close_key(key: &str) -> bool {
    key == "escape"
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PinnedKeyboardCommand {
    Close,
    Copy,
    Save,
    ZoomOut,
    ZoomIn,
    CycleOpacity,
}

/// Maps focused-window keys to local pin actions without changing global shortcuts.
fn pinned_keyboard_command(keystroke: &Keystroke) -> Option<PinnedKeyboardCommand> {
    let modifiers = keystroke.modifiers;
    if pinned_close_key(&keystroke.key) && !modifiers.shift {
        return Some(PinnedKeyboardCommand::Close);
    }
    if modifiers.secondary() && !modifiers.alt && !modifiers.function {
        return match keystroke.key.as_str() {
            "c" => Some(PinnedKeyboardCommand::Copy),
            "s" => Some(PinnedKeyboardCommand::Save),
            "-" => Some(PinnedKeyboardCommand::ZoomOut),
            "=" | "+" => Some(PinnedKeyboardCommand::ZoomIn),
            "o" => Some(PinnedKeyboardCommand::CycleOpacity),
            _ => None,
        };
    }
    None
}

fn next_pin_opacity(current: u8) -> u8 {
    PIN_OPACITY_STEPS
        .iter()
        .position(|opacity| *opacity == current)
        .and_then(|index| PIN_OPACITY_STEPS.get(index + 1))
        .copied()
        .unwrap_or(PIN_OPACITY_STEPS[0])
}

fn opacity_percentage(opacity: u8) -> u8 {
    ((u16::from(opacity) * 100 + 127) / 255) as u8
}

fn pin_opacity_label(opacity: u8) -> &'static str {
    match opacity {
        255 => "Opacity 100%",
        191 => "Opacity 75%",
        128 => "Opacity 50%",
        _ => "Opacity 25%",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PinnedKeyboardCommand, copy_pinned_image, next_pin_opacity, opacity_percentage,
        pinned_close_key, pinned_control_tooltip, pinned_keyboard_command,
    };
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

    #[test]
    fn compact_pin_controls_explain_their_actions() {
        for control in ["zoom-out", "zoom-in", "opacity", "copy", "save", "close"] {
            assert!(!pinned_control_tooltip(control).is_empty());
        }
        assert!(pinned_control_tooltip("close").contains("Escape"));
    }

    #[test]
    fn focused_pin_shortcuts_keep_copy_and_window_controls_local() {
        let control = gpui::Modifiers {
            control: true,
            ..Default::default()
        };
        let key = |key: &str, modifiers| gpui::Keystroke {
            key: key.into(),
            modifiers,
            key_char: None,
        };
        assert_eq!(
            pinned_keyboard_command(&key("escape", Default::default())),
            Some(PinnedKeyboardCommand::Close)
        );
        assert_eq!(
            pinned_keyboard_command(&key("c", control)),
            Some(PinnedKeyboardCommand::Copy)
        );
        assert_eq!(
            pinned_keyboard_command(&key("-", control)),
            Some(PinnedKeyboardCommand::ZoomOut)
        );
        assert_eq!(
            pinned_keyboard_command(&key("s", control)),
            Some(PinnedKeyboardCommand::Save)
        );
        assert_eq!(
            pinned_keyboard_command(&key("=", control)),
            Some(PinnedKeyboardCommand::ZoomIn)
        );
        assert_eq!(
            pinned_keyboard_command(&key("o", control)),
            Some(PinnedKeyboardCommand::CycleOpacity)
        );
        assert_eq!(pinned_keyboard_command(&key("c", Default::default())), None);
    }

    #[test]
    fn opacity_control_cycles_through_reference_friendly_levels() {
        assert_eq!(next_pin_opacity(255), 191);
        assert_eq!(next_pin_opacity(191), 128);
        assert_eq!(next_pin_opacity(128), 64);
        assert_eq!(next_pin_opacity(64), 255);
        assert_eq!(next_pin_opacity(99), 255);
        assert_eq!(opacity_percentage(191), 75);
    }
}
