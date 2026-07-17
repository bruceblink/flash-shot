//! Capture, selection, and clipboard workflow orchestration.

use std::{sync::Arc, time::Instant};

use gpui::{
    AsyncApp, Bounds, Context, Image, ImageFormat, KeyDownEvent, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, WeakEntity,
};

use super::FlashShotApp;
use crate::{
    domain::{
        geometry::PhysicalRect,
        selection::{PreviewTransform, ViewPoint, ViewRect},
        session::CaptureSessionState,
    },
    platform::{
        capture::{CaptureBackend, CaptureFrame, SystemCaptureBackend},
        clipboard::{ClipboardService, SystemClipboard},
        display::{DisplayProvider, SystemDisplayProvider},
    },
};

impl FlashShotApp {
    pub(super) fn start_capture(&mut self, cx: &mut Context<Self>) {
        if self.session.state() != CaptureSessionState::Idle {
            return;
        }
        if let Err(error) = self.session.begin() {
            self.status = error.to_string();
            cx.notify();
            return;
        }
        self.frame = None;
        self.preview = None;
        self.selection_drag.clear();
        self.hover_pixel = None;
        self.status = "Capturing primary display...".to_owned();
        cx.notify();

        let started_at = Instant::now();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move { capture_primary_display() })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.finish_capture(result, started_at, cx)
                    });
                }
            }
        })
        .detach();
    }

    fn finish_capture(
        &mut self,
        result: std::io::Result<(CaptureFrame, Vec<u8>)>,
        started_at: Instant,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok((frame, png)) => {
                if let Err(error) = self.session.frames_ready() {
                    self.status = error.to_string();
                    cx.notify();
                    return;
                }
                self.performance
                    .record_duration("shortcut_to_frame_ready", started_at.elapsed());
                self.status = format!(
                    "{} x {} physical pixels - {:.1} ms - {} CPU copy",
                    frame.width,
                    frame.height,
                    frame.capture_duration.as_secs_f64() * 1_000.0,
                    frame.cpu_copy_count
                );
                self.preview = Some(Arc::new(Image::from_bytes(ImageFormat::Png, png)));
                self.frame = Some(frame);
            }
            Err(error) => {
                let message = format!("Capture failed: {error}");
                let _ = self.session.fail(message.clone());
                self.status = message;
                log::warn!(target: "flash_shot::capture", "capture_failed error={error}");
            }
        }
        cx.notify();
    }

    pub(super) fn reset(&mut self, cx: &mut Context<Self>) {
        if matches!(
            self.session.state(),
            CaptureSessionState::Selecting
                | CaptureSessionState::Completed
                | CaptureSessionState::Cancelled
                | CaptureSessionState::Failed
        ) {
            if self.session.state() == CaptureSessionState::Selecting {
                let _ = self.session.cancel();
            }
            let _ = self.session.reset();
        }
        self.frame = None;
        self.preview = None;
        self.selection_drag.clear();
        self.hover_pixel = None;
        self.status = "Ready - Ctrl+Shift+Print Screen".to_owned();
        cx.notify();
    }

    pub(super) fn begin_selection(&mut self, event: &MouseDownEvent, viewport: Bounds<Pixels>) {
        let Some(transform) = self.preview_transform(viewport) else {
            return;
        };
        let point = view_point(event.position);
        if let Some((selection, handle)) = self.selection_drag.selection().and_then(|selection| {
            transform
                .resize_handle_at(selection, point, 10.0)
                .map(|handle| (selection, handle))
        }) {
            self.selection_drag.begin_resize(selection, handle);
        } else if let Some(point) = transform.view_to_physical(point) {
            self.selection_drag.begin(point);
        }
    }

    pub(super) fn nudge_selection(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if event.keystroke.modifiers.control
            || event.keystroke.modifiers.alt
            || event.keystroke.modifiers.platform
            || event.keystroke.modifiers.function
        {
            return;
        }
        let step = if event.keystroke.modifiers.shift {
            10
        } else {
            1
        };
        let (delta_x, delta_y) = match event.keystroke.key.as_str() {
            "left" => (-step, 0),
            "right" => (step, 0),
            "up" => (0, -step),
            "down" => (0, step),
            _ => return,
        };
        let Some(frame) = self.frame.as_ref() else {
            return;
        };
        let Some(selection) = self.selection_drag.nudge(frame.bounds, delta_x, delta_y) else {
            return;
        };
        if self.session.select(selection).is_ok() {
            self.status = selection_status(selection);
            cx.stop_propagation();
            cx.notify();
        }
    }

    pub(super) fn update_selection(
        &mut self,
        event: &MouseMoveEvent,
        viewport: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.update_hover_pixel(event, viewport, cx);
        if !event.dragging() {
            return;
        }
        let Some(transform) = self.preview_transform(viewport) else {
            return;
        };
        if let Some(point) = transform.view_to_physical(clamp_to_preview(transform, event.position))
        {
            self.selection_drag.update(point);
            if let Some(selection) = self.selection_drag.selection() {
                self.status = selection_status(selection);
            }
            cx.notify();
        }
    }

    fn update_hover_pixel(
        &mut self,
        event: &MouseMoveEvent,
        viewport: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let hover_pixel = self
            .preview_transform(viewport)
            .and_then(|transform| transform.view_to_pixel(view_point(event.position)));
        if self.hover_pixel == hover_pixel {
            return;
        }
        self.hover_pixel = hover_pixel;
        if let Some((point, color)) = hover_pixel.and_then(|point| {
            self.frame
                .as_ref()?
                .pixel_at(point)
                .map(|color| (point, color))
        }) {
            self.status = if let Some(selection) = self.selection_drag.selection() {
                format!(
                    "{} x {} px | ({}, {}) {}",
                    selection.width(),
                    selection.height(),
                    point.x,
                    point.y,
                    color.hex_rgb()
                )
            } else {
                format!("({}, {}) {}", point.x, point.y, color.hex_rgb())
            };
        } else if let Some(selection) = self.selection_drag.selection() {
            self.status = selection_status(selection);
        } else if let Some(frame) = self.frame.as_ref() {
            self.status = format!("{} x {} physical pixels", frame.width, frame.height);
        }
        cx.notify();
    }

    pub(super) fn finish_selection(
        &mut self,
        event: &MouseUpEvent,
        viewport: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(transform) = self.preview_transform(viewport) else {
            return;
        };
        if let Some(point) = transform.view_to_physical(clamp_to_preview(transform, event.position))
        {
            self.selection_drag.update(point);
        }
        if let Some(selection) = self.selection_drag.selection()
            && selection.width() > 0
            && selection.height() > 0
        {
            let _ = self.session.select(selection);
            self.status = selection_status(selection);
        }
        cx.notify();
    }

    pub(super) fn copy_selection(&mut self, cx: &mut Context<Self>) {
        let selection = match self.session.start_export() {
            Ok(selection) => selection,
            Err(error) => {
                self.status = error.to_string();
                cx.notify();
                return;
            }
        };
        let Some(frame) = self.frame.clone() else {
            let message = "capture frame is unavailable".to_owned();
            let _ = self.session.fail(message.clone());
            self.status = message;
            cx.notify();
            return;
        };

        self.status = "Copying selection...".to_owned();
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move { copy_frame_selection(&frame, selection, &SystemClipboard) })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| this.finish_copy(result, cx));
                }
            }
        })
        .detach();
    }

    fn finish_copy(&mut self, result: std::io::Result<()>, cx: &mut Context<Self>) {
        match result {
            Ok(()) => {
                if let Err(error) = self.session.export_completed() {
                    self.status = error.to_string();
                } else {
                    self.status = "Selection copied to clipboard".to_owned();
                }
            }
            Err(error) => {
                let message = format!("Copy failed: {error}");
                let _ = self.session.fail(message.clone());
                self.status = message;
            }
        }
        cx.notify();
    }

    pub(super) fn preview_transform(&self, viewport: Bounds<Pixels>) -> Option<PreviewTransform> {
        let frame = self.frame.as_ref()?;
        PreviewTransform::contain(frame.bounds, view_rect(viewport))
    }
}

fn capture_primary_display() -> std::io::Result<(CaptureFrame, Vec<u8>)> {
    let display = SystemDisplayProvider
        .displays()?
        .into_iter()
        .find(|display| display.primary)
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "primary display missing")
        })?;
    let frame = SystemCaptureBackend.capture(display.physical_bounds)?;
    let png = frame.encode_png()?;
    Ok((frame, png))
}

fn copy_frame_selection(
    frame: &CaptureFrame,
    selection: PhysicalRect,
    clipboard: &impl ClipboardService,
) -> std::io::Result<()> {
    clipboard.copy_image(&frame.crop(selection)?)
}

pub(super) fn view_rect(bounds: Bounds<Pixels>) -> ViewRect {
    ViewRect {
        left: f32::from(bounds.origin.x),
        top: f32::from(bounds.origin.y),
        width: f32::from(bounds.size.width),
        height: f32::from(bounds.size.height),
    }
}

fn view_point(position: gpui::Point<Pixels>) -> ViewPoint {
    ViewPoint {
        x: f32::from(position.x),
        y: f32::from(position.y),
    }
}

fn clamp_to_preview(transform: PreviewTransform, position: gpui::Point<Pixels>) -> ViewPoint {
    let fitted = transform.fitted_view();
    ViewPoint {
        x: f32::from(position.x).clamp(fitted.left, fitted.right()),
        y: f32::from(position.y).clamp(fitted.top, fitted.bottom()),
    }
}

fn selection_status(selection: PhysicalRect) -> String {
    format!(
        "Selection: {} x {} physical pixels",
        selection.width(),
        selection.height()
    )
}

#[cfg(test)]
mod tests {
    use super::copy_frame_selection;
    use crate::{
        domain::geometry::PhysicalRect,
        platform::{
            capture::{CaptureFrame, PixelFormat},
            clipboard::ClipboardService,
        },
    };
    use std::{cell::RefCell, io, sync::Arc, time::Duration};

    #[derive(Default)]
    struct RecordingClipboard {
        copied: RefCell<Option<CaptureFrame>>,
    }

    impl ClipboardService for RecordingClipboard {
        fn copy_image(&self, frame: &CaptureFrame) -> io::Result<()> {
            self.copied.replace(Some(frame.clone()));
            Ok(())
        }
    }

    #[test]
    fn copy_uses_the_pixel_correct_selected_region() {
        let frame = CaptureFrame {
            bounds: PhysicalRect {
                left: -2,
                top: 10,
                right: 1,
                bottom: 12,
            },
            width: 3,
            height: 2,
            stride: 12,
            format: PixelFormat::Bgra8,
            pixels: Arc::from([
                1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255, 13, 14, 15, 255, 16, 17,
                18, 255,
            ]),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 1,
        };
        let clipboard = RecordingClipboard::default();

        copy_frame_selection(
            &frame,
            PhysicalRect {
                left: -1,
                top: 10,
                right: 1,
                bottom: 12,
            },
            &clipboard,
        )
        .unwrap();

        let copied = clipboard.copied.borrow();
        let copied = copied.as_ref().unwrap();
        assert_eq!((copied.width, copied.height), (2, 2));
        assert_eq!(
            copied.pixels.as_ref(),
            &[4, 5, 6, 255, 7, 8, 9, 255, 13, 14, 15, 255, 16, 17, 18, 255]
        );
    }
}
