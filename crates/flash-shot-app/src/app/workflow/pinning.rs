//! Clipboard and pinned-capture workflows.

use super::*;
use crate::i18n::UiText;

/// Immutable pixels prepared before a Pin window is opened.
pub(in crate::app) struct PreparedPinnedFrame {
    pub(super) frame: CaptureFrame,
    pub(super) image: Arc<RenderImage>,
}

/// Converts captured pixels into the render image used by the Pin window.
/// This runs on the background executor before native window creation.
pub(super) fn prepare_pinned_frame(frame: CaptureFrame) -> std::io::Result<PreparedPinnedFrame> {
    let rendered = render_image_from_capture(&frame)?;
    Ok(PreparedPinnedFrame {
        frame,
        image: rendered.image,
    })
}

impl FlashShotApp {
    pub(in crate::app) fn copy_selection(&mut self, cx: &mut Context<Self>) {
        let Some(selection) = (self.session.state() == CaptureSessionState::Selecting)
            .then(|| self.session.selection())
            .flatten()
        else {
            self.status = "Select an area before copying".to_owned();
            cx.notify();
            return;
        };
        let Some((frame, document)) = self.export_source() else {
            cx.notify();
            return;
        };
        // Copy owns only its immutable pixel snapshot and the clipboard. The editable capture
        // stays in Selecting so Save, Pin, Scroll shot, and annotation work remain responsive.
        // The cloned source goes to the worker, which composites and crops it away from GPUI.
        let Some(copy_id) = self.try_begin_clipboard_write("copying a selection", cx) else {
            return;
        };
        self.status = "Copying selection in the background...".to_owned();
        let status_generation = self.operation_generation;
        let cancellation = Arc::new(SelectionCopyCancellation::default());
        self.selection_copy = Some(SelectionCopyLease {
            id: copy_id,
            status_generation,
            recognition_generation: self.recognition_generation,
            cancellation: cancellation.clone(),
            cancel_requested: false,
        });
        let clipboard = self.image_clipboard.clone();
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        copy_selection_snapshot_cancellable(
                            &frame,
                            &document,
                            selection,
                            clipboard.as_ref(),
                            cancellation.as_ref(),
                        )
                    })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| this.finish_copy(result, copy_id, cx));
                }
            }
        })
        .detach();
    }

    /// Requests cancellation without resetting the editable capture that a background Copy reads.
    pub(in crate::app) fn cancel_selection_copy(&mut self, cx: &mut Context<Self>) {
        let Some(copy) = self.selection_copy.as_mut() else {
            return;
        };
        copy.cancel_requested = true;
        self.status = match copy.cancellation.request_cancel() {
            SelectionCopyCancelRequest::CancelledBeforeCommit
            | SelectionCopyCancelRequest::AlreadyCancelled => {
                "Cancelling background clipboard copy...".to_owned()
            }
            SelectionCopyCancelRequest::ClipboardCommitStarted => {
                "Clipboard write already started; waiting for copy to finish...".to_owned()
            }
        };
        cx.notify();
    }

    /// Reports whether this editor still owns a large-image Copy worker.
    pub(in crate::app) fn selection_copy_is_active(&self) -> bool {
        self.selection_copy.is_some()
    }

    /// Gives Copy only the first Escape so cancellation never traps the editor until encoding ends.
    pub(in crate::app) fn selection_copy_owns_escape(&self) -> bool {
        self.selection_copy
            .as_ref()
            .is_some_and(SelectionCopyLease::owns_escape)
    }

    pub(in crate::app) fn pin_selection(&mut self, cx: &mut Context<Self>) {
        let Some(selection) = self.session.selection() else {
            self.status = "Select an area before pinning".to_owned();
            cx.notify();
            return;
        };
        let Some((frame, document)) = self.export_source() else {
            cx.notify();
            return;
        };
        // Closing an overlay while handling one of its events must complete before a
        // normal topmost window is opened. Re-entering the window API from nested
        // deferred callbacks can leave the Windows event loop blocked instead.
        self.reset(cx);
        let generation = self.operation_generation;
        self.status = "Preparing pinned image...".to_owned();
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                // Annotation compositing and cropping can copy a large 4K frame, so keep that
                // CPU work off the GPUI thread. The native Pin window is still opened on the UI
                // thread after the prepared pixels arrive and capture teardown is complete.
                let prepared = cx
                    .background_executor()
                    .spawn(async move {
                        let pinned_frame = frame
                            .composite_annotations(&document)
                            .and_then(|frame| frame.crop(selection))?;
                        prepare_pinned_frame(pinned_frame)
                    })
                    .await;
                let mut prepared = Some(prepared);
                // Yield to the native message loop, then wait until every old overlay HWND has
                // reported its close callback before creating a topmost Pin window.
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(1))
                        .await;
                    let Some(this) = this.upgrade() else {
                        break;
                    };
                    let finished = this.update(&mut cx, |app, cx| {
                        if app.operation_generation != generation
                            || app.session.state() != CaptureSessionState::Idle
                        {
                            return true;
                        }
                        if app.capture_teardown_pending {
                            return false;
                        }
                        let Some(prepared) = prepared.take() else {
                            return true;
                        };
                        match prepared {
                            Ok(prepared) => app.open_prepared_pinned_frame(
                                prepared,
                                UiText::PinSelectionOpened,
                                None,
                                false,
                                cx,
                            ),
                            Err(error) => {
                                let error_detail = error.to_string();
                                app.status = app.settings.locale.format_template(
                                    UiText::PinSelectionError,
                                    &[("error", &error_detail)],
                                );
                                app.return_to_background();
                                cx.notify();
                            }
                        }
                        true
                    });
                    if finished {
                        break;
                    }
                }
            }
        })
        .detach();
    }

    /// Reads the current clipboard image away from the UI thread and pins only the latest request.
    pub(in crate::app) fn pin_clipboard_image(&mut self, cx: &mut Context<Self>) {
        if self.clipboard_write_lease.is_some() {
            self.status =
                "Wait for the current clipboard copy to finish before pinning it".to_owned();
            cx.notify();
            return;
        }
        if self.clipboard_pin_generation.is_some()
            || self.full_screen_copy_generation.is_some()
            || self.full_screen_save_generation.is_some()
            || self.full_screen_pin_generation.is_some()
            || self.history_reader.is_some()
            || self.delayed_capture_generation.is_some()
            || self.session.state() != CaptureSessionState::Idle
        {
            return;
        }
        self.operation_generation = self.operation_generation.wrapping_add(1);
        let generation = self.operation_generation;
        self.clipboard_pin_generation = Some(generation);
        self.status = "Reading clipboard image...".to_owned();
        self.hide_settings_window();
        cx.notify();

        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(
                        async move { SystemClipboard.read_image().and_then(prepare_pinned_frame) },
                    )
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.finish_pin_clipboard_image(result, generation, cx)
                    });
                }
            }
        })
        .detach();
    }

    fn finish_pin_clipboard_image(
        &mut self,
        result: std::io::Result<PreparedPinnedFrame>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if !claim_idle_completion(
            &mut self.clipboard_pin_generation,
            self.operation_generation,
            generation,
            self.session.state(),
        ) {
            return;
        }
        match result {
            Ok(prepared) => self.open_prepared_pinned_frame(
                prepared,
                UiText::PinClipboardOpened,
                Some(UiText::PinClipboardFailed),
                false,
                cx,
            ),
            Err(error) => {
                let error_detail = error.to_string();
                self.status = self
                    .settings
                    .locale
                    .format_template(UiText::PinClipboardError, &[("error", &error_detail)]);
                log::warn!(target: "flash_shot::pinned", "clipboard_pin_failed error={error}");
                self.notify_user(
                    self.settings.locale.text(UiText::AppName),
                    self.settings.locale.text(UiText::PinClipboardFailed),
                );
                cx.notify();
            }
        }
    }

    /// Opens one reusable always-on-top image window from an already decoded frame.
    pub(in crate::app) fn open_pinned_frame(
        &mut self,
        pinned_frame: CaptureFrame,
        success_status: UiText,
        failure_notification: Option<UiText>,
        show_saved_feedback: bool,
        cx: &mut Context<Self>,
    ) {
        let pinned = match render_image_from_capture(&pinned_frame) {
            Ok(image) => image,
            Err(error) => {
                let error_detail = error.to_string();
                self.status = self
                    .settings
                    .locale
                    .format_template(UiText::PinRenderFailed, &[("error", &error_detail)]);
                if let Some(message) = failure_notification {
                    self.notify_user(
                        self.settings.locale.text(UiText::AppName),
                        self.settings.locale.text(message),
                    );
                }
                cx.notify();
                return;
            }
        };
        self.open_prepared_pinned_frame(
            PreparedPinnedFrame {
                frame: pinned_frame,
                image: pinned.image,
            },
            success_status,
            failure_notification,
            show_saved_feedback,
            cx,
        );
    }

    /// Opens a Pin after pixel preparation has completed, keeping native window work on the UI thread.
    pub(super) fn open_prepared_pinned_frame(
        &mut self,
        prepared: PreparedPinnedFrame,
        success_status: UiText,
        failure_notification: Option<UiText>,
        show_saved_feedback: bool,
        cx: &mut Context<Self>,
    ) {
        let PreparedPinnedFrame { frame, image } = prepared;
        let window_size = pinned_size(frame.width as f32, frame.height as f32);
        let window_bounds = WindowBounds::centered(window_size, cx);
        let pinned_app = cx.entity();
        let pinned_colors = self.colors;
        let pinned_locale = self.settings.locale;
        match cx.open_window(
            WindowOptions {
                window_bounds: Some(window_bounds),
                titlebar: None,
                focus: true,
                show: true,
                kind: WindowKind::PopUp,
                is_movable: true,
                is_resizable: true,
                is_minimizable: false,
                window_background: WindowBackgroundAppearance::Opaque,
                window_min_size: None,
                ..Default::default()
            },
            move |window, cx| {
                let pinned = cx.new(|cx| {
                    PinnedImage::new(image, frame, pinned_app, pinned_colors, pinned_locale, cx)
                });
                if show_saved_feedback {
                    pinned.update(cx, |pinned, cx| pinned.finish_save_status(true, cx));
                }
                pinned.read(cx).focus_handle(cx).focus(window, cx);
                pinned
            },
        ) {
            Ok(window) => {
                self.pinned_windows.push(window);
                self.status = self.settings.locale.text(success_status).to_owned();
            }
            Err(error) => {
                let error_detail = error.to_string();
                self.status = self
                    .settings
                    .locale
                    .format_template(UiText::PinWindowOpenFailed, &[("error", &error_detail)]);
                log::warn!(target: "flash_shot::pinned", "pinned_window_open_failed error={error}");
                if let Some(message) = failure_notification {
                    self.notify_user(
                        self.settings.locale.text(UiText::AppName),
                        self.settings.locale.text(message),
                    );
                }
            }
        }
        cx.notify();
    }

    /// Opens an isolated preview that exercises the Pin window's saved-state feedback for UI QA.
    pub(crate) fn open_pinned_saved_feedback_preview(&mut self, cx: &mut Context<Self>) {
        self.open_pinned_frame(
            pinned_saved_feedback_preview_frame(),
            UiText::PinSavedPreviewOpened,
            None,
            true,
            cx,
        );
    }

    /// Hides every live pinned window except the focused reference image.
    /// Closed windows are dropped from the registry so later focus commands remain harmless.
    pub(in crate::app) fn hide_other_pinned_windows(
        &mut self,
        current_window: gpui::WindowHandle<PinnedImage>,
        cx: &mut Context<Self>,
    ) -> usize {
        let mut hidden = 0;
        self.pinned_windows.retain(|pinned| {
            if is_current_pinned_window(pinned, current_window) {
                return true;
            }
            match pinned.update(cx, |_, window, _| -> Result<(), String> {
                native_window_handle(window)
                    .ok_or_else(|| "Pinned window handle is unavailable".to_owned())
                    .and_then(|handle| {
                        window_visibility::hide(handle).map_err(|error| error.to_string())
                    })
            }) {
                Ok(Ok(())) => {
                    hidden += 1;
                    true
                }
                Ok(Err(error)) => {
                    log::warn!(target: "flash_shot::pinned", "pinned_window_hide_failed error={error}");
                    true
                }
                Err(error) => {
                    log::debug!(target: "flash_shot::pinned", "stale_pinned_window_removed error={error}");
                    false
                }
            }
        });
        hidden
    }

    /// Restores all live pinned references without stealing focus from the current image.
    pub(in crate::app) fn show_all_pinned_windows(
        &mut self,
        current_window: gpui::WindowHandle<PinnedImage>,
        cx: &mut Context<Self>,
    ) -> usize {
        let mut shown = 0;
        self.pinned_windows.retain(|pinned| {
            if is_current_pinned_window(pinned, current_window) {
                return true;
            }
            match pinned.update(cx, |_, window, _| -> Result<(), String> {
                let handle = native_window_handle(window)
                    .ok_or_else(|| "Pinned window handle is unavailable".to_owned())?;
                window_visibility::show(handle).map_err(|error| error.to_string())?;
                Ok(())
            }) {
                Ok(Ok(())) => {
                    shown += 1;
                    true
                }
                Ok(Err(error)) => {
                    log::warn!(target: "flash_shot::pinned", "pinned_window_show_failed error={error}");
                    true
                }
                Err(error) => {
                    log::debug!(target: "flash_shot::pinned", "stale_pinned_window_removed error={error}");
                    false
                }
            }
        });
        shown
    }

    /// Removes exactly the Pin whose GPUI window is closing without probing sibling windows
    /// during native teardown, when otherwise-live handles can be temporarily unavailable.
    pub(in crate::app) fn unregister_pinned_window(
        &mut self,
        closing_id: gpui::WindowId,
        cx: &mut Context<Self>,
    ) -> bool {
        let removed = remove_pinned_window_by_id(&mut self.pinned_windows, closing_id);
        if removed {
            cx.notify();
        }
        removed
    }

    /// Defensively drops stale handles; normal closes are removed exactly by the app observer.
    pub(in crate::app) fn prune_closed_pinned_windows(&mut self, cx: &mut Context<Self>) {
        self.pinned_windows.retain(|pinned| match pinned.update(cx, |_, _, _| {}) {
            Ok(()) => true,
            Err(error) => {
                log::debug!(target: "flash_shot::pinned", "closed_pinned_window_removed error={error}");
                false
            }
        });
    }

    /// Provides a recovery route when a click-through pin no longer owns keyboard focus.
    pub(in crate::app) fn restore_pinned_window_input(&mut self, cx: &mut Context<Self>) {
        let mut restored = 0;
        self.pinned_windows.retain(|pinned| {
            match pinned.update(cx, |pinned, window, cx| {
                pinned.restore_mouse_input(window, cx)
            }) {
                Ok(Ok(true)) => {
                    restored += 1;
                    true
                }
                Ok(Ok(false)) => true,
                Ok(Err(error)) => {
                    log::warn!(target: "flash_shot::pinned", "pinned_window_input_restore_failed error={error}");
                    true
                }
                Err(error) => {
                    log::debug!(target: "flash_shot::pinned", "stale_pinned_window_removed error={error}");
                    false
                }
            }
        });
        let locale = self.settings.locale;
        self.status = if restored == 0 {
            locale
                .text(crate::i18n::UiText::NoPinnedWindowsNeededInputRecovery)
                .to_owned()
        } else {
            locale.pinned_window_input_restored(restored)
        };
        cx.notify();
    }
}

/// Skips the active Pin before nested updates because GPUI temporarily removes that window from
/// its app registry while dispatching the current Pin action.
fn is_current_pinned_window(
    candidate: &gpui::WindowHandle<PinnedImage>,
    current: gpui::WindowHandle<PinnedImage>,
) -> bool {
    *candidate == current
}

/// Applies the exact-ID registry rule independently from GPUI/native window state.
fn remove_pinned_window_by_id(
    windows: &mut Vec<gpui::WindowHandle<PinnedImage>>,
    closing_id: gpui::WindowId,
) -> bool {
    let previous_len = windows.len();
    windows.retain(|window| window.window_id() != closing_id);
    windows.len() != previous_len
}

/// Builds a patterned frame so native Pin screenshots show both the image surface and toolbar.
fn pinned_saved_feedback_preview_frame() -> CaptureFrame {
    const WIDTH: u32 = 760;
    const HEIGHT: u32 = 480;
    const BLOCK_SIZE: usize = 76;
    let stride = WIDTH as usize * 4;
    let mut pixels = vec![0_u8; stride * HEIGHT as usize];

    for y in 0..HEIGHT as usize {
        for x in 0..WIDTH as usize {
            let block_is_light = (x / BLOCK_SIZE + y / BLOCK_SIZE).is_multiple_of(2);
            let color = if block_is_light {
                [82, 104, 128, 255]
            } else {
                [49, 65, 82, 255]
            };
            let offset = y * stride + x * 4;
            pixels[offset..offset + 4].copy_from_slice(&color);
        }
    }

    CaptureFrame {
        bounds: PhysicalRect {
            left: 0,
            top: 0,
            right: WIDTH as i32,
            bottom: HEIGHT as i32,
        },
        width: WIDTH,
        height: HEIGHT,
        stride,
        format: crate::platform::capture::PixelFormat::Bgra8,
        pixels: Arc::from(pixels),
        capture_duration: Duration::ZERO,
        cpu_copy_count: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PinnedImage, is_current_pinned_window, pinned_saved_feedback_preview_frame,
        remove_pinned_window_by_id,
    };
    use crate::domain::geometry::PhysicalPoint;
    use gpui::{WindowHandle, WindowId};

    #[test]
    fn saved_feedback_preview_frame_has_valid_physical_pixels() {
        let frame = pinned_saved_feedback_preview_frame();

        assert_eq!(frame.bounds.width(), 760);
        assert_eq!(frame.bounds.height(), 480);
        assert!(frame.validate().is_ok());
        assert!(frame.pixel_at(PhysicalPoint { x: 0, y: 0 }).is_some());
        assert!(frame.pixel_at(PhysicalPoint { x: 760, y: 480 }).is_none());
    }

    #[test]
    fn closing_one_pin_removes_only_its_registered_window() {
        let first_id = WindowId::from(1_u64);
        let closing_id = WindowId::from(2_u64);
        let third_id = WindowId::from(3_u64);
        let mut windows = vec![
            WindowHandle::<PinnedImage>::new(first_id),
            WindowHandle::<PinnedImage>::new(closing_id),
            WindowHandle::<PinnedImage>::new(third_id),
        ];

        assert!(remove_pinned_window_by_id(&mut windows, closing_id));
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].window_id(), first_id);
        assert_eq!(windows[1].window_id(), third_id);
        assert!(!remove_pinned_window_by_id(&mut windows, closing_id));
    }

    #[test]
    fn active_pin_is_identified_before_nested_window_updates() {
        let active = WindowHandle::<PinnedImage>::new(WindowId::from(7_u64));
        let sibling = WindowHandle::<PinnedImage>::new(WindowId::from(8_u64));

        assert!(is_current_pinned_window(&active, active));
        assert!(!is_current_pinned_window(&sibling, active));
    }
}
