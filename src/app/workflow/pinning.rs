//! Clipboard and pinned-capture workflows.

use super::*;

impl FlashShotApp {
    pub(in crate::app) fn copy_selection(&mut self, cx: &mut Context<Self>) {
        let selection = match self.session.start_export() {
            Ok(selection) => selection,
            Err(error) => {
                self.status = error.to_string();
                cx.notify();
                return;
            }
        };
        let Some((frame, document)) = self.export_source() else {
            cx.notify();
            return;
        };

        self.status = "Copying selection...".to_owned();
        let generation = self.operation_generation;
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        copy_annotated_frame_selection(
                            &frame,
                            &document,
                            selection,
                            &SystemClipboard,
                        )
                    })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| this.finish_copy(result, generation, cx));
                }
            }
        })
        .detach();
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
        let pinned_frame = match frame
            .composite_annotations(&document)
            .and_then(|frame| frame.crop(selection))
        {
            Ok(frame) => frame,
            Err(error) => {
                self.status = format!("Could not pin selection: {error}");
                cx.notify();
                return;
            }
        };
        // Closing an overlay while handling one of its events must complete before a
        // normal topmost window is opened. Re-entering the window API from nested
        // deferred callbacks can leave the Windows event loop blocked instead.
        self.reset(cx);
        let generation = self.operation_generation;
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                // Yield to the native message loop so reset's deferred overlay
                // teardown runs before this pin creates and focuses its own window.
                cx.background_executor()
                    .timer(Duration::from_millis(1))
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |app, cx| {
                        if app.operation_generation != generation
                            || app.session.state() != CaptureSessionState::Idle
                        {
                            return;
                        }
                        app.open_pinned_frame(
                            pinned_frame,
                            "Selection pinned in an always-on-top window",
                            None,
                            false,
                            cx,
                        );
                    });
                }
            }
        })
        .detach();
    }

    /// Reads the current clipboard image away from the UI thread and pins only the latest request.
    pub(in crate::app) fn pin_clipboard_image(&mut self, cx: &mut Context<Self>) {
        if self.clipboard_pin_generation.is_some()
            || self.full_screen_copy_generation.is_some()
            || self.full_screen_save_generation.is_some()
            || self.full_screen_pin_generation.is_some()
            || self.history_pin_generation.is_some()
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
                    .spawn(async move { SystemClipboard.read_image() })
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
        result: std::io::Result<CaptureFrame>,
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
            Ok(frame) => self.open_pinned_frame(
                frame,
                "Clipboard image pinned in an always-on-top window",
                Some("Could not pin clipboard image"),
                false,
                cx,
            ),
            Err(error) => {
                self.status = format!("Could not pin clipboard image: {error}");
                log::warn!(target: "flash_shot::pinned", "clipboard_pin_failed error={error}");
                self.notify_user("Flash Shot", "Could not pin clipboard image");
                cx.notify();
            }
        }
    }

    /// Opens one reusable always-on-top image window from an already decoded frame.
    pub(super) fn open_pinned_frame(
        &mut self,
        pinned_frame: CaptureFrame,
        success_status: &'static str,
        failure_notification: Option<&'static str>,
        show_saved_feedback: bool,
        cx: &mut Context<Self>,
    ) {
        let pinned = match render_image_from_capture(&pinned_frame) {
            Ok(image) => image,
            Err(error) => {
                self.status = format!("Could not render pinned image: {error}");
                if let Some(message) = failure_notification {
                    self.notify_user("Flash Shot", message);
                }
                cx.notify();
                return;
            }
        };
        let window_size = pinned_size(pinned_frame.width as f32, pinned_frame.height as f32);
        let window_bounds = WindowBounds::centered(window_size, cx);
        let pinned_app = cx.entity();
        let pinned_colors = self.colors;
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
                    PinnedImage::new(pinned.image, pinned_frame, pinned_app, pinned_colors, cx)
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
                self.status = success_status.to_owned();
            }
            Err(error) => {
                self.status = format!("Could not open pinned window: {error}");
                log::warn!(target: "flash_shot::pinned", "pinned_window_open_failed error={error}");
                if let Some(message) = failure_notification {
                    self.notify_user("Flash Shot", message);
                }
            }
        }
        cx.notify();
    }

    /// Opens an isolated preview that exercises the Pin window's saved-state feedback for UI QA.
    pub(crate) fn open_pinned_saved_feedback_preview(&mut self, cx: &mut Context<Self>) {
        self.open_pinned_frame(
            pinned_saved_feedback_preview_frame(),
            "Pinned saved-feedback preview opened",
            None,
            true,
            cx,
        );
    }

    /// Hides every live pinned window except the focused reference image.
    /// Closed windows are dropped from the registry so later focus commands remain harmless.
    pub(in crate::app) fn hide_other_pinned_windows(
        &mut self,
        current_handle: isize,
        cx: &mut Context<Self>,
    ) -> usize {
        let mut hidden = 0;
        self.pinned_windows.retain(|pinned| {
            match pinned.update(cx, |_, window, _| -> Result<bool, String> {
                let handle = native_window_handle(window);
                if handle == Some(current_handle) {
                    return Ok(false);
                }
                handle
                    .ok_or_else(|| "Pinned window handle is unavailable".to_owned())
                    .and_then(|handle| {
                        window_visibility::hide(handle).map_err(|error| error.to_string())
                    })?;
                Ok(true)
            }) {
                Ok(Ok(true)) => {
                    hidden += 1;
                    true
                }
                Ok(Ok(false)) => true,
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
    pub(in crate::app) fn show_all_pinned_windows(&mut self, cx: &mut Context<Self>) -> usize {
        let mut shown = 0;
        self.pinned_windows.retain(|pinned| {
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
        self.status = if restored == 0 {
            "No pinned windows needed input recovery".to_owned()
        } else {
            format!("Restored mouse input for {restored} pinned window(s)")
        };
        cx.notify();
    }
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
    use super::pinned_saved_feedback_preview_frame;
    use crate::domain::geometry::PhysicalPoint;

    #[test]
    fn saved_feedback_preview_frame_has_valid_physical_pixels() {
        let frame = pinned_saved_feedback_preview_frame();

        assert_eq!(frame.bounds.width(), 760);
        assert_eq!(frame.bounds.height(), 480);
        assert!(frame.validate().is_ok());
        assert!(frame.pixel_at(PhysicalPoint { x: 0, y: 0 }).is_some());
        assert!(frame.pixel_at(PhysicalPoint { x: 760, y: 480 }).is_none());
    }
}
