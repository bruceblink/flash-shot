//! Lightweight always-on-top windows for keeping a captured selection visible.

use std::{sync::Arc, time::Duration};

use gpui::{
    AsyncApp, Context, Entity, FocusHandle, Focusable, FontWeight, KeyDownEvent, Keystroke, Pixels,
    Render, Size, WeakEntity, Window, WindowControlArea, div, img, prelude::*, px, size,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::FlashShotApp;
use crate::{
    i18n::{Locale, UiText},
    platform::{capture::CaptureFrame, clipboard::ClipboardService},
};

const PIN_OPACITY_STEPS: [u8; 4] = [255, 191, 128, 64];
const PIN_FEEDBACK_VISIBLE_FOR: Duration = Duration::from_secs(3);
const PIN_TOP_CONTROLS_HEIGHT: f32 = 62.0;

struct PinnedTooltip(&'static str, crate::theme::ThemeColors);

impl Render for PinnedTooltip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .bg(self.1.panel)
            .border_1()
            .border_color(self.1.border)
            .rounded_sm()
            .shadow_lg()
            .text_color(self.1.text)
            .text_xs()
            .child(self.0)
    }
}

/// Describes each compact pin control without requiring the image window to stay large.
fn pinned_control_tooltip(locale: Locale, control: &str) -> &'static str {
    match control {
        "zoom-out" => locale.text(UiText::PinZoomOutTooltip),
        "zoom-in" => locale.text(UiText::PinZoomInTooltip),
        "opacity" => locale.text(UiText::PinOpacityTooltip),
        "mouse-through" => locale.text(UiText::PinMouseThroughTooltip),
        "solo" => locale.text(UiText::PinSoloTooltip),
        "show-all" => locale.text(UiText::PinShowAllTooltip),
        "copy" => locale.text(UiText::PinCopyTooltip),
        "save" => locale.text(UiText::PinSaveTooltip),
        "close" => locale.text(UiText::PinCloseTooltip),
        _ => "",
    }
}

pub(super) struct PinnedImage {
    image: Arc<gpui::RenderImage>,
    frame: CaptureFrame,
    app: Entity<FlashShotApp>,
    colors: crate::theme::ThemeColors,
    locale: Locale,
    focus_handle: FocusHandle,
    topmost_requested: bool,
    opacity: u8,
    mouse_through: bool,
    status: &'static str,
    feedback_visible: bool,
    feedback_generation: u64,
    copy_in_flight: bool,
    pending_zoom_size: Option<Size<Pixels>>,
}

impl PinnedImage {
    /// Creates a Pin with an immutable image snapshot and the locale active when its window opens.
    pub(super) fn new(
        image: Arc<gpui::RenderImage>,
        frame: CaptureFrame,
        app: Entity<FlashShotApp>,
        colors: crate::theme::ThemeColors,
        locale: Locale,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            image,
            frame,
            app,
            colors,
            locale,
            focus_handle: cx.focus_handle(),
            topmost_requested: false,
            opacity: 255,
            mouse_through: false,
            status: locale.text(UiText::PinCapture),
            feedback_visible: false,
            feedback_generation: 0,
            copy_in_flight: false,
            pending_zoom_size: None,
        }
    }

    pub(super) fn copy_image(&mut self, cx: &mut Context<Self>) {
        if !pinned_copy_can_start(self.copy_in_flight) {
            self.show_operation_feedback(self.locale.text(UiText::PinCopyingImage), cx);
            return;
        }
        let Some((write_id, clipboard)) = self.app.update(cx, |app, cx| {
            app.try_begin_clipboard_write(UiText::ClipboardActionPinnedImage, cx)
                .map(|write_id| (write_id, app.image_clipboard.clone()))
        }) else {
            self.show_operation_feedback(self.locale.text(UiText::PinWaitingClipboard), cx);
            return;
        };

        // Keep native clipboard encoding and retries off GPUI's event thread. The shared lease
        // prevents another screen surface from replacing the system clipboard while this Pin runs.
        self.copy_in_flight = true;
        self.show_operation_feedback(self.locale.text(UiText::PinCopyingImage), cx);
        let frame = self.frame.clone();
        let app = self.app.clone();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move { copy_pinned_image(&frame, clipboard.as_ref()) })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| this.finish_copy_status(result, cx));
                }
                // A Pin may close before its worker returns. The app-level lease must still be
                // released so a later screenshot, history, or Pin copy cannot remain blocked.
                app.update(&mut cx, |app, _| {
                    app.finish_clipboard_write(write_id);
                });
            }
        })
        .detach();
    }

    /// Applies a completed background copy only to this Pin's small feedback state.
    fn finish_copy_status(&mut self, result: std::io::Result<()>, cx: &mut Context<Self>) {
        self.copy_in_flight = false;
        let feedback = match result {
            Ok(()) => self.locale.text(UiText::PinCopiedImage),
            Err(error) => {
                log::warn!(target: "flash_shot::pinned", "pinned_window_copy_failed error={error}");
                self.locale.text(UiText::PinCopyFailed)
            }
        };
        self.show_operation_feedback(feedback, cx);
    }

    /// Delegates the file write to the capture service so history ownership stays centralized.
    pub(super) fn save_image(&mut self, cx: &mut Context<Self>) {
        let frame = self.frame.clone();
        let pin = cx.entity().downgrade();
        let accepted = self
            .app
            .update(cx, |app, cx| app.quick_save_pinned_frame(frame, pin, cx));
        let feedback = if accepted {
            self.locale.text(UiText::PinSavingImage)
        } else {
            self.locale.text(UiText::PinSaveBusy)
        };
        self.show_operation_feedback(feedback, cx);
    }

    /// Keeps controls visible while a no-input acceptance runner exercises native Pin actions.
    pub(super) fn show_controls_for_acceptance(&mut self, cx: &mut Context<Self>) {
        self.show_operation_feedback(self.locale.text(UiText::PinCapture), cx);
    }

    /// Exposes only task completion to the no-input Pin lifecycle probe.
    pub(super) const fn copy_in_flight_for_acceptance(&self) -> bool {
        self.copy_in_flight
    }

    /// Returns the uncapped source dimensions while the native acceptance runner owns this Pin.
    pub(super) const fn source_bounds_for_acceptance(
        &self,
    ) -> crate::domain::geometry::PhysicalRect {
        self.frame.bounds
    }

    /// Clones the immutable Pin source so acceptance can compare content without screen scraping.
    pub(super) fn frame_for_acceptance(&self) -> crate::platform::capture::CaptureFrame {
        self.frame.clone()
    }

    /// Applies the async save result to the originating pin while ignoring a closed window.
    pub(super) fn finish_save_status(&mut self, saved: bool, cx: &mut Context<Self>) {
        self.show_operation_feedback(pinned_save_result_status(self.locale, saved), cx);
    }

    /// Keeps a completed action visible long enough for keyboard and pointer users to read it.
    fn show_operation_feedback(&mut self, status: &'static str, cx: &mut Context<Self>) {
        self.status = status;
        self.feedback_visible = true;
        self.feedback_generation = self.feedback_generation.wrapping_add(1);
        let generation = self.feedback_generation;
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                cx.background_executor()
                    .timer(PIN_FEEDBACK_VISIBLE_FOR)
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        if pin_feedback_timer_is_current(this.feedback_generation, generation) {
                            this.feedback_visible = false;
                            cx.notify();
                        }
                    });
                }
            }
        })
        .detach();
    }

    /// Scales the complete native window so the contained image remains undistorted.
    pub(super) fn zoom(&mut self, scale: f32, window: &mut Window, cx: &mut Context<Self>) {
        let target = next_pin_zoom_size(window.bounds().size, self.pending_zoom_size, scale);
        self.pending_zoom_size = Some(target);
        let saved_center = native_window_handle(window).and_then(|handle| {
            match crate::platform::window_visibility::snapshot_window_center(handle) {
                Ok(center) => Some((handle, center)),
                Err(error) => {
                    log::warn!(target: "flash_shot::pinned", "pinned_window_center_snapshot_failed error={error}");
                    None
                }
            }
        });
        // GPUI owns the drawable size and swap chain; resizing through it keeps native bounds,
        // rendered content, and pointer hit testing synchronized on the next platform tick.
        window.resize(target);
        if let Some((handle, center)) = saved_center {
            // GPUI queues its native resize on this executor. Queueing a move-only task next
            // preserves the old center without re-entering WM_SIZE from the current callback.
            cx.foreground_executor()
                .spawn(async move {
                    if let Err(error) =
                        crate::platform::window_visibility::recenter_window(handle, center)
                    {
                        log::warn!(target: "flash_shot::pinned", "pinned_window_recenter_failed error={error}");
                    }
                })
                .detach();
        }
        let feedback = if scale > 1.0 {
            self.locale.text(UiText::PinZoomedIn)
        } else {
            self.locale.text(UiText::PinZoomedOut)
        };
        self.show_operation_feedback(feedback, cx);
    }

    /// Cycles through readable reference-image opacity levels without moving the window.
    pub(super) fn cycle_opacity(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next = next_pin_opacity(self.opacity);
        let feedback = match window.window_handle() {
            Ok(handle) => match handle.as_raw() {
                RawWindowHandle::Win32(handle) => {
                    match crate::platform::window_visibility::set_opacity(handle.hwnd.get(), next) {
                        Ok(()) => {
                            self.opacity = next;
                            pin_opacity_label(self.locale, next)
                        }
                        Err(error) => {
                            log::warn!(target: "flash_shot::pinned", "pinned_window_opacity_failed error={error}");
                            self.locale.text(UiText::PinOpacityChangeFailed)
                        }
                    }
                }
                _ => self.locale.text(UiText::PinOpacityUnavailable),
            },
            Err(error) => {
                log::warn!(target: "flash_shot::pinned", "pinned_window_handle_failed error={error}");
                self.locale.text(UiText::PinOpacityChangeFailed)
            }
        };
        self.show_operation_feedback(feedback, cx);
    }

    /// Lets clicks reach the app below while this pinned image remains usable through Ctrl+M.
    fn toggle_mouse_through(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next = !self.mouse_through;
        let feedback = match window.window_handle() {
            Ok(handle) => match handle.as_raw() {
                RawWindowHandle::Win32(handle) => {
                    match crate::platform::window_visibility::set_mouse_through(
                        handle.hwnd.get(),
                        next,
                    ) {
                        Ok(()) => {
                            self.mouse_through = next;
                            if next {
                                self.app.read(cx).notify_user(
                                    self.locale.text(UiText::AppName),
                                    self.locale.text(UiText::PinMouseThroughNotification),
                                );
                                self.locale.text(UiText::PinMouseThroughEnabled)
                            } else {
                                self.locale.text(UiText::PinMouseThroughDisabled)
                            }
                        }
                        Err(error) => {
                            log::warn!(target: "flash_shot::pinned", "pinned_window_mouse_through_failed error={error}");
                            self.locale.text(UiText::PinMouseThroughFailed)
                        }
                    }
                }
                _ => self.locale.text(UiText::PinMouseThroughUnavailable),
            },
            Err(error) => {
                log::warn!(target: "flash_shot::pinned", "pinned_window_handle_failed error={error}");
                self.locale.text(UiText::PinMouseThroughFailed)
            }
        };
        self.show_operation_feedback(feedback, cx);
    }

    pub(super) fn restore_mouse_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, String> {
        if !self.mouse_through {
            return Ok(false);
        }
        let handle = native_window_handle(window).ok_or_else(|| {
            self.locale
                .text(UiText::PinWindowHandleUnavailable)
                .to_owned()
        })?;
        crate::platform::window_visibility::set_mouse_through(handle, false)
            .map_err(|error| error.to_string())?;
        self.mouse_through = false;
        self.show_operation_feedback(self.locale.text(UiText::PinMouseThroughDisabled), cx);
        Ok(true)
    }

    /// Keeps one reference image visible without closing the user's other pinned captures.
    pub(super) fn hide_other_pinned_images(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let feedback = match gpui::Window::window_handle(window).downcast::<PinnedImage>() {
            Some(current_window) => {
                let hidden = self.app.update(cx, |app, cx| {
                    app.hide_other_pinned_windows(current_window, cx)
                });
                if hidden == 0 {
                    self.locale.text(UiText::PinNoOtherImages)
                } else {
                    self.locale.text(UiText::PinOtherImagesHidden)
                }
            }
            None => self.locale.text(UiText::PinWindowHandleUnavailable),
        };
        self.show_operation_feedback(feedback, cx);
    }

    /// Restores hidden reference images while preserving the active pin's keyboard focus.
    pub(super) fn show_all_pinned_images(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let feedback = match gpui::Window::window_handle(window).downcast::<PinnedImage>() {
            Some(current_window) => {
                let shown = self.app.update(cx, |app, cx| {
                    app.show_all_pinned_windows(current_window, cx)
                });
                if shown == 0 {
                    self.locale.text(UiText::PinNoImagesToShow)
                } else {
                    self.locale.text(UiText::PinAllImagesShown)
                }
            }
            None => self.locale.text(UiText::PinWindowHandleUnavailable),
        };
        self.show_operation_feedback(feedback, cx);
    }

    /// Closes this independent Pin; the app's post-close observer unregisters its exact window ID.
    pub(super) fn close(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        window.remove_window();
    }
}

fn copy_pinned_image(
    frame: &CaptureFrame,
    clipboard: &(impl ClipboardService + ?Sized),
) -> std::io::Result<()> {
    clipboard.copy_image(frame)
}

/// Allows only one background clipboard write for an individual Pin window at a time.
const fn pinned_copy_can_start(copy_in_flight: bool) -> bool {
    !copy_in_flight
}

impl Focusable for PinnedImage {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// Returns the native handle only for the focused pin's visibility commands.
pub(super) fn native_window_handle(window: &Window) -> Option<isize> {
    HasWindowHandle::window_handle(window)
        .ok()
        .and_then(|handle| match handle.as_raw() {
            RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
            _ => None,
        })
}

#[derive(Clone, Copy)]
enum PinnedButtonTone {
    Neutral,
    Selected,
    Primary,
    Destructive,
}

/// Builds one consistent pin control with readable state, focus, hover, and tooltip feedback.
fn pinned_tool_button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<String>,
    control: &'static str,
    colors: crate::theme::ThemeColors,
    locale: Locale,
    tone: PinnedButtonTone,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let emphasized = matches!(tone, PinnedButtonTone::Selected | PinnedButtonTone::Primary);
    let destructive = matches!(tone, PinnedButtonTone::Destructive);
    div()
        .id(id)
        .h(px(30.0))
        .px_3()
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .border_1()
        .border_color(if destructive {
            colors.danger
        } else if emphasized {
            colors.accent
        } else {
            colors.border
        })
        .bg(if emphasized {
            colors.accent
        } else {
            colors.background
        })
        .text_color(if emphasized {
            colors.background
        } else if destructive {
            colors.danger
        } else {
            colors.text
        })
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .focusable()
        .focus_visible(|style| style.border_color(colors.accent))
        .cursor_pointer()
        .hover(move |style| {
            style
                .bg(colors.panel)
                .border_color(if destructive {
                    colors.danger
                } else {
                    colors.accent
                })
                .text_color(if destructive {
                    colors.danger
                } else {
                    colors.text
                })
        })
        .tooltip(move |_, cx| {
            cx.new(|_| PinnedTooltip(pinned_control_tooltip(locale, control), colors))
                .into()
        })
        .on_click(on_click)
        .child(label.into())
}

impl Render for PinnedImage {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        if self.pending_zoom_size.is_some_and(|target| {
            pin_zoom_target_reached(window.bounds().size, target, window.scale_factor())
        }) {
            self.pending_zoom_size = None;
        }
        let colors = self.colors;
        let locale = self.locale;
        if !self.topmost_requested
            && let Ok(handle) = window.window_handle()
            && let RawWindowHandle::Win32(handle) = handle.as_raw()
        {
            self.topmost_requested = true;
            let hwnd = handle.hwnd.get();
            // Rendering runs from GPUI's native window dispatch. Defer the Win32
            // z-order change so it cannot synchronously re-enter that dispatch.
            cx.defer(move |_| {
                if let Err(error) = crate::platform::window_visibility::make_topmost(hwnd) {
                    log::warn!(target: "flash_shot::pinned", "pinned_window_topmost_failed error={error}");
                }
            });
        }
        let toolbar = div()
            .id("pinned-toolbar")
            .absolute()
            .top(px(8.0))
            .left(px(8.0))
            // Keep the hover toolbar clear of the persistent close button at the top right.
            .right(px(48.0))
            .p_2()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .bg(colors.panel)
            .border_1()
            .border_color(colors.border)
            .rounded_lg()
            .shadow_lg()
            .when(!self.feedback_visible && !self.copy_in_flight, |toolbar| {
                toolbar
                    .invisible()
                    .group_hover("pinned-window", |toolbar| toolbar.visible())
            })
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .h(px(30.0))
                            .px_2()
                            .flex()
                            .items_center()
                            .rounded_md()
                            .bg(colors.background)
                            .text_xs()
                            .text_color(colors.muted)
                            .child(self.status),
                    )
                    .child(pinned_tool_button(
                        "pinned-save",
                        locale.text(UiText::PinSave),
                        "save",
                        colors,
                        locale,
                        PinnedButtonTone::Neutral,
                        cx.listener(|this, _, _, cx| this.save_image(cx)),
                    ))
                    .child(pinned_tool_button(
                        "pinned-zoom-out",
                        "-",
                        "zoom-out",
                        colors,
                        locale,
                        PinnedButtonTone::Neutral,
                        cx.listener(|this, _, window, cx| this.zoom(0.8, window, cx)),
                    ))
                    .child(pinned_tool_button(
                        "pinned-zoom-in",
                        "+",
                        "zoom-in",
                        colors,
                        locale,
                        PinnedButtonTone::Neutral,
                        cx.listener(|this, _, window, cx| this.zoom(1.25, window, cx)),
                    ))
                    .child(pinned_tool_button(
                        "pinned-opacity",
                        format!("{}%", opacity_percentage(self.opacity)),
                        "opacity",
                        colors,
                        locale,
                        PinnedButtonTone::Neutral,
                        cx.listener(|this, _, window, cx| this.cycle_opacity(window, cx)),
                    ))
                    .child(pinned_tool_button(
                        "pinned-mouse-through",
                        locale.text(UiText::PinMouseThrough),
                        "mouse-through",
                        colors,
                        locale,
                        if self.mouse_through {
                            PinnedButtonTone::Selected
                        } else {
                            PinnedButtonTone::Neutral
                        },
                        cx.listener(|this, _, window, cx| this.toggle_mouse_through(window, cx)),
                    ))
                    .child(pinned_tool_button(
                        "pinned-solo",
                        locale.text(UiText::PinSolo),
                        "solo",
                        colors,
                        locale,
                        PinnedButtonTone::Neutral,
                        cx.listener(|this, _, window, cx| {
                            this.hide_other_pinned_images(window, cx)
                        }),
                    ))
                    .child(pinned_tool_button(
                        "pinned-show-all",
                        locale.text(UiText::PinShowAll),
                        "show-all",
                        colors,
                        locale,
                        PinnedButtonTone::Neutral,
                        cx.listener(|this, _, window, cx| this.show_all_pinned_images(window, cx)),
                    ))
                    .child(pinned_tool_button(
                        "pinned-copy",
                        locale.text(UiText::PinCopy),
                        "copy",
                        colors,
                        locale,
                        PinnedButtonTone::Primary,
                        cx.listener(|this, _, _, cx| this.copy_image(cx)),
                    )),
            );
        // A Pin has no native title bar, so closing it must never depend on discovering the
        // hover-only toolbar. This compact control remains reachable whenever input is enabled.
        let close_button = div().absolute().top(px(8.0)).right(px(8.0)).child(
            pinned_tool_button(
                "pinned-close",
                "X",
                "close",
                colors,
                locale,
                PinnedButtonTone::Destructive,
                cx.listener(|this, _, window, cx| this.close(window, cx)),
            )
            .w(px(32.0))
            .px_0()
            // Expose the client control as native close chrome so borderless Windows delivers
            // the same close gesture instead of treating it as an unhandled title-bar click.
            .window_control_area(WindowControlArea::Close),
        );
        let image = div()
            .id("pinned-image")
            .size_full()
            .bg(colors.background)
            .child(img(self.image.clone()).size_full());
        // The top controls are client widgets, not title-bar chrome. Restricting native dragging
        // below them keeps their clicks reachable while leaving most of the image draggable.
        let drag_region = div()
            .id("pinned-drag-region")
            .absolute()
            .top(px(PIN_TOP_CONTROLS_HEIGHT))
            .right(px(0.0))
            .bottom(px(0.0))
            .left(px(0.0))
            .window_control_area(WindowControlArea::Drag);

        div()
            .size_full()
            .relative()
            .group("pinned-window")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                match pinned_keyboard_command(&event.keystroke) {
                    Some(PinnedKeyboardCommand::Close) => this.close(window, cx),
                    Some(PinnedKeyboardCommand::Copy) => this.copy_image(cx),
                    Some(PinnedKeyboardCommand::Save) => this.save_image(cx),
                    Some(PinnedKeyboardCommand::ZoomOut) => this.zoom(0.8, window, cx),
                    Some(PinnedKeyboardCommand::ZoomIn) => this.zoom(1.25, window, cx),
                    Some(PinnedKeyboardCommand::CycleOpacity) => this.cycle_opacity(window, cx),
                    Some(PinnedKeyboardCommand::ToggleMouseThrough) => {
                        this.toggle_mouse_through(window, cx)
                    }
                    Some(PinnedKeyboardCommand::HideOthers) => {
                        this.hide_other_pinned_images(window, cx)
                    }
                    Some(PinnedKeyboardCommand::ShowAll) => this.show_all_pinned_images(window, cx),
                    None => {}
                }
            }))
            .bg(colors.background)
            .border_1()
            .border_color(colors.border)
            .child(image)
            .child(drag_region)
            .child(toolbar)
            .child(close_button)
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
    ToggleMouseThrough,
    HideOthers,
    ShowAll,
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
            "m" => Some(PinnedKeyboardCommand::ToggleMouseThrough),
            "h" if modifiers.shift => Some(PinnedKeyboardCommand::ShowAll),
            "h" => Some(PinnedKeyboardCommand::HideOthers),
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

/// Uses an already queued size as the base so rapid zoom commands accumulate predictably.
fn next_pin_zoom_size(
    current: Size<Pixels>,
    pending: Option<Size<Pixels>>,
    scale: f32,
) -> Size<Pixels> {
    let base = pending.unwrap_or(current);
    let (width, height) = crate::platform::window_visibility::scaled_pin_size(
        f32::from(base.width),
        f32::from(base.height),
        scale,
    );
    size(px(width), px(height))
}

/// Treats sizes as equal when both resolve to the same device pixels at the window's DPI.
fn pin_zoom_target_reached(actual: Size<Pixels>, target: Size<Pixels>, scale_factor: f32) -> bool {
    let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    let device_extent = |extent: Pixels| (f32::from(extent) * scale).round();
    device_extent(actual.width) == device_extent(target.width)
        && device_extent(actual.height) == device_extent(target.height)
}

fn opacity_percentage(opacity: u8) -> u8 {
    ((u16::from(opacity) * 100 + 127) / 255) as u8
}

/// Converts a bounded opacity level into a localized feedback label for the active Pin.
fn pin_opacity_label(locale: Locale, opacity: u8) -> &'static str {
    match opacity {
        255 => locale.text(UiText::PinOpacity100),
        191 => locale.text(UiText::PinOpacity75),
        128 => locale.text(UiText::PinOpacity50),
        _ => locale.text(UiText::PinOpacity25),
    }
}

/// Keeps the Pin toolbar's completion text short enough to remain visible beside its controls.
fn pinned_save_result_status(locale: Locale, saved: bool) -> &'static str {
    if saved {
        locale.text(UiText::PinSavedImage)
    } else {
        locale.text(UiText::PinSaveFailed)
    }
}

/// Rejects a stale delay so an older action cannot hide newer feedback early.
fn pin_feedback_timer_is_current(current_generation: u64, timer_generation: u64) -> bool {
    current_generation == timer_generation
}

#[cfg(test)]
mod tests {
    use super::{
        PinnedKeyboardCommand, copy_pinned_image, next_pin_opacity, next_pin_zoom_size,
        opacity_percentage, pin_feedback_timer_is_current, pin_zoom_target_reached,
        pinned_close_key, pinned_control_tooltip, pinned_copy_can_start, pinned_keyboard_command,
        pinned_save_result_status,
    };
    use crate::i18n::Locale;
    use crate::{
        domain::geometry::PhysicalRect,
        platform::{
            capture::{CaptureFrame, PixelFormat},
            clipboard::ClipboardService,
        },
    };
    use std::{cell::RefCell, io, sync::Arc, time::Duration};

    #[test]
    fn rapid_pin_zoom_uses_the_pending_target_as_its_next_base() {
        let current = gpui::size(gpui::px(360.0), gpui::px(240.0));
        let first = next_pin_zoom_size(current, None, 1.25);
        let second = next_pin_zoom_size(current, Some(first), 1.25);

        assert_eq!(first, gpui::size(gpui::px(450.0), gpui::px(300.0)));
        assert_eq!(second, gpui::size(gpui::px(563.0), gpui::px(375.0)));
    }

    #[test]
    fn fractional_dpi_round_trip_clears_only_the_matching_zoom_target() {
        let target = gpui::size(gpui::px(563.0), gpui::px(375.0));
        let rounded = gpui::size(gpui::px(563.2), gpui::px(375.2));
        let manually_resized = gpui::size(gpui::px(564.0), gpui::px(375.2));

        assert!(pin_zoom_target_reached(rounded, target, 1.25));
        assert!(!pin_zoom_target_reached(manually_resized, target, 1.25));
    }

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
    fn pinned_copy_rejects_duplicate_clicks_while_its_worker_is_running() {
        assert!(pinned_copy_can_start(false));
        assert!(!pinned_copy_can_start(true));
    }

    #[test]
    fn escape_is_the_only_keyboard_close_command() {
        assert!(pinned_close_key("escape"));
        assert!(!pinned_close_key("enter"));
        assert!(!pinned_close_key("shift-escape"));
    }

    #[test]
    fn compact_pin_controls_explain_their_actions() {
        for control in [
            "zoom-out",
            "zoom-in",
            "opacity",
            "mouse-through",
            "solo",
            "show-all",
            "copy",
            "save",
            "close",
        ] {
            assert!(!pinned_control_tooltip(Locale::English, control).is_empty());
        }
        assert!(pinned_control_tooltip(Locale::English, "close").contains("Escape"));
        assert_eq!(
            pinned_control_tooltip(Locale::SimplifiedChinese, "copy"),
            "复制图片（Ctrl+C）"
        );
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
        assert_eq!(
            pinned_keyboard_command(&key("m", control)),
            Some(PinnedKeyboardCommand::ToggleMouseThrough)
        );
        assert_eq!(
            pinned_keyboard_command(&key("h", control)),
            Some(PinnedKeyboardCommand::HideOthers)
        );
        assert_eq!(
            pinned_keyboard_command(&key(
                "h",
                gpui::Modifiers {
                    control: true,
                    shift: true,
                    ..Default::default()
                },
            )),
            Some(PinnedKeyboardCommand::ShowAll)
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

    #[test]
    fn pinned_save_result_status_distinguishes_success_and_failure() {
        assert_eq!(
            pinned_save_result_status(Locale::English, true),
            "Saved image"
        );
        assert_eq!(
            pinned_save_result_status(Locale::SimplifiedChinese, false),
            "无法保存图片"
        );
    }

    #[test]
    fn current_pin_feedback_timer_does_not_hide_newer_feedback() {
        assert!(pin_feedback_timer_is_current(4, 4));
        assert!(!pin_feedback_timer_is_current(5, 4));
    }
}
