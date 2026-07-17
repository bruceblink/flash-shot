//! Capture, selection, and clipboard workflow orchestration.

use std::{path::PathBuf, sync::Arc, time::Instant};

use gpui::{
    AsyncApp, Bounds, Context, Image, ImageFormat, KeyDownEvent, Keystroke, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, WeakEntity,
};

use super::FlashShotApp;
use crate::{
    domain::{
        geometry::PhysicalRect,
        selection::{PreviewTransform, ViewPoint, ViewRect},
        session::CaptureSessionState,
    },
    platform::{
        capture::{CaptureFrame, capture_virtual_desktop},
        clipboard::{ClipboardService, SystemClipboard},
        window_inspector::{InspectionTarget, SystemWindowInspector, WindowInspector},
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
        self.inspection_target = None;
        self.pending_click_target = None;
        self.inspection_request = None;
        self.status = "Capturing virtual desktop...".to_owned();
        cx.notify();

        let started_at = Instant::now();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move { capture_virtual_desktop_preview() })
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
        result: std::io::Result<CapturedDesktopPreview>,
        started_at: Instant,
        cx: &mut Context<Self>,
    ) {
        if self.session.state() != CaptureSessionState::Capturing {
            return;
        }
        match result {
            Ok(capture) => {
                if let Err(error) = self.session.frames_ready() {
                    self.status = error.to_string();
                    cx.notify();
                    return;
                }
                self.performance
                    .record_duration("shortcut_to_frame_ready", started_at.elapsed());
                self.status = format!(
                    "{} x {} physical pixels - {} display(s) - {:.1} ms - {} CPU copy",
                    capture.capture.frame.width,
                    capture.capture.frame.height,
                    capture.capture.display_count,
                    capture.capture.frame.capture_duration.as_secs_f64() * 1_000.0,
                    capture.capture.frame.cpu_copy_count
                );
                self.preview = Some(Arc::new(Image::from_bytes(ImageFormat::Png, capture.png)));
                self.frame = Some(capture.capture.frame);
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
        match self.session.state() {
            CaptureSessionState::Capturing | CaptureSessionState::Selecting => {
                let _ = self.session.cancel();
                let _ = self.session.reset();
            }
            CaptureSessionState::Completed
            | CaptureSessionState::Cancelled
            | CaptureSessionState::Failed => {
                let _ = self.session.reset();
            }
            CaptureSessionState::Idle => {}
            CaptureSessionState::Exporting => return,
        }
        self.frame = None;
        self.preview = None;
        self.selection_drag.clear();
        self.hover_pixel = None;
        self.inspection_target = None;
        self.pending_click_target = None;
        self.inspection_request = None;
        self.status = "Ready - Ctrl+Shift+Print Screen".to_owned();
        cx.notify();
    }

    pub(super) fn begin_selection(&mut self, event: &MouseDownEvent, viewport: Bounds<Pixels>) {
        let Some(transform) = self.preview_transform(viewport) else {
            return;
        };
        let point = view_point(event.position);
        let physical_point = transform.view_to_pixel(point);
        self.pending_click_target = physical_point.and_then(|point| {
            self.inspection_target
                .filter(|target| target.bounds.contains(point))
        });
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

    pub(super) fn handle_key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let Some(command) = keyboard_command(&event.keystroke) else {
            return;
        };
        let handled = match command {
            KeyboardCommand::Cancel => {
                if matches!(
                    self.session.state(),
                    CaptureSessionState::Capturing
                        | CaptureSessionState::Selecting
                        | CaptureSessionState::Completed
                        | CaptureSessionState::Failed
                ) {
                    self.reset(cx);
                    true
                } else {
                    false
                }
            }
            KeyboardCommand::Copy => {
                if self.session.state() == CaptureSessionState::Selecting
                    && self.session.selection().is_some()
                {
                    self.copy_selection(cx);
                    true
                } else {
                    false
                }
            }
            KeyboardCommand::Nudge(delta_x, delta_y) => self.nudge_selection(delta_x, delta_y, cx),
        };
        if handled {
            cx.stop_propagation();
        }
    }

    fn nudge_selection(&mut self, delta_x: i32, delta_y: i32, cx: &mut Context<Self>) -> bool {
        let Some(frame) = self.frame.as_ref() else {
            return false;
        };
        let Some(selection) = self.selection_drag.nudge(frame.bounds, delta_x, delta_y) else {
            return false;
        };
        if self.session.select(selection).is_ok() {
            self.status = selection_status(selection);
            cx.notify();
            true
        } else {
            false
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
        match hover_pixel {
            Some(point)
                if self.selection_drag.selection().is_none()
                    && !self
                        .inspection_target
                        .is_some_and(|target| target.bounds.contains(point)) =>
            {
                self.request_inspection(point, cx);
            }
            None => {
                self.inspection_target = None;
                self.inspection_request = None;
            }
            _ => {}
        }
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
        let selection = self
            .selection_drag
            .selection()
            .and_then(|selection| resolve_pointer_selection(selection, self.pending_click_target));
        self.pending_click_target = None;
        if let Some(selection) = selection {
            self.selection_drag.select(selection);
            let _ = self.session.select(selection);
            self.status = selection_status(selection);
        }
        cx.notify();
    }

    fn request_inspection(
        &mut self,
        point: crate::domain::geometry::PhysicalPoint,
        cx: &mut Context<Self>,
    ) {
        self.inspection_request = Some(point);
        if self.inspection_in_flight {
            return;
        }
        self.start_inspection(cx);
    }

    fn start_inspection(&mut self, cx: &mut Context<Self>) {
        let Some(point) = self.inspection_request.take() else {
            return;
        };
        self.inspection_in_flight = true;
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move { SystemWindowInspector.target_at(point) })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.finish_inspection(point, result, cx)
                    });
                }
            }
        })
        .detach();
    }

    fn finish_inspection(
        &mut self,
        point: crate::domain::geometry::PhysicalPoint,
        result: std::io::Result<Option<InspectionTarget>>,
        cx: &mut Context<Self>,
    ) {
        self.inspection_in_flight = false;
        match result {
            Ok(target) if self.hover_pixel == Some(point) => {
                self.inspection_target = target.and_then(|target| {
                    let bounds = intersect_rect(target.bounds, self.frame.as_ref()?.bounds)?;
                    Some(InspectionTarget {
                        bounds,
                        kind: target.kind,
                    })
                });
                cx.notify();
            }
            Ok(_) => {}
            Err(error) => {
                log::warn!(target: "flash_shot::inspection", "window_inspection_failed error={error}");
            }
        }
        if self.inspection_request.is_some() {
            self.start_inspection(cx);
        }
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

    pub(super) fn save_selection(&mut self, cx: &mut Context<Self>) {
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

        self.status = "Choose where to save the selection...".to_owned();
        cx.notify();
        let prompt = cx.prompt_for_new_path(&PathBuf::default(), Some("flash-shot.png"));
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let outcome = match prompt.await {
                    Ok(Ok(Some(path))) => {
                        let path = png_path(path);
                        let result = cx
                            .background_executor()
                            .spawn(async move {
                                save_frame_selection(&frame, selection, path.clone()).map(|()| path)
                            })
                            .await;
                        match result {
                            Ok(path) => SaveOutcome::Saved(path),
                            Err(error) => SaveOutcome::Failed(error.to_string()),
                        }
                    }
                    Ok(Ok(None)) => SaveOutcome::Cancelled,
                    Ok(Err(error)) => SaveOutcome::Failed(error.to_string()),
                    Err(error) => SaveOutcome::Failed(error.to_string()),
                };
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| this.finish_save(outcome, cx));
                }
            }
        })
        .detach();
    }

    fn finish_save(&mut self, outcome: SaveOutcome, cx: &mut Context<Self>) {
        match outcome {
            SaveOutcome::Saved(path) => {
                if let Err(error) = self.session.export_completed() {
                    self.status = error.to_string();
                } else {
                    self.status = format!("Selection saved to {}", path.display());
                }
            }
            SaveOutcome::Cancelled => {
                if let Err(error) = self.session.export_cancelled() {
                    self.status = error.to_string();
                } else if let Some(selection) = self.session.selection() {
                    self.status = selection_status(selection);
                }
            }
            SaveOutcome::Failed(error) => {
                let message = format!("Save failed: {error}");
                let _ = self.session.fail(message.clone());
                self.status = message;
            }
        }
        cx.notify();
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

struct CapturedDesktopPreview {
    capture: crate::platform::capture::VirtualDesktopCapture,
    png: Vec<u8>,
}

fn capture_virtual_desktop_preview() -> std::io::Result<CapturedDesktopPreview> {
    let capture = capture_virtual_desktop()?;
    let png = capture.frame.encode_png()?;
    Ok(CapturedDesktopPreview { capture, png })
}

fn copy_frame_selection(
    frame: &CaptureFrame,
    selection: PhysicalRect,
    clipboard: &impl ClipboardService,
) -> std::io::Result<()> {
    clipboard.copy_image(&frame.crop(selection)?)
}

fn save_frame_selection(
    frame: &CaptureFrame,
    selection: PhysicalRect,
    path: PathBuf,
) -> std::io::Result<()> {
    frame.crop(selection)?.save_png(path)
}

fn png_path(mut path: PathBuf) -> PathBuf {
    let is_png = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("png"));
    if !is_png {
        path.set_extension("png");
    }
    path
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

fn intersect_rect(left: PhysicalRect, right: PhysicalRect) -> Option<PhysicalRect> {
    let intersection = PhysicalRect {
        left: left.left.max(right.left),
        top: left.top.max(right.top),
        right: left.right.min(right.right),
        bottom: left.bottom.min(right.bottom),
    };
    (intersection.width() > 0 && intersection.height() > 0).then_some(intersection)
}

fn resolve_pointer_selection(
    dragged: PhysicalRect,
    smart_target: Option<InspectionTarget>,
) -> Option<PhysicalRect> {
    const CLICK_TOLERANCE: u32 = 3;
    if dragged.width() <= CLICK_TOLERANCE && dragged.height() <= CLICK_TOLERANCE {
        smart_target.map(|target| target.bounds)
    } else if dragged.width() > 0 && dragged.height() > 0 {
        Some(dragged)
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum KeyboardCommand {
    Cancel,
    Copy,
    Nudge(i32, i32),
}

enum SaveOutcome {
    Saved(PathBuf),
    Cancelled,
    Failed(String),
}

fn keyboard_command(keystroke: &Keystroke) -> Option<KeyboardCommand> {
    let modifiers = keystroke.modifiers;
    if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
        return None;
    }
    match keystroke.key.as_str() {
        "escape" if !modifiers.shift => Some(KeyboardCommand::Cancel),
        "enter" if !modifiers.shift => Some(KeyboardCommand::Copy),
        "left" => Some(KeyboardCommand::Nudge(
            if modifiers.shift { -10 } else { -1 },
            0,
        )),
        "right" => Some(KeyboardCommand::Nudge(
            if modifiers.shift { 10 } else { 1 },
            0,
        )),
        "up" => Some(KeyboardCommand::Nudge(
            0,
            if modifiers.shift { -10 } else { -1 },
        )),
        "down" => Some(KeyboardCommand::Nudge(
            0,
            if modifiers.shift { 10 } else { 1 },
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        KeyboardCommand, copy_frame_selection, intersect_rect, keyboard_command, png_path,
        resolve_pointer_selection, save_frame_selection,
    };
    use crate::platform::window_inspector::{InspectionKind, InspectionTarget};
    use crate::{
        domain::geometry::PhysicalRect,
        platform::{
            capture::{CaptureFrame, PixelFormat},
            clipboard::ClipboardService,
        },
    };
    use gpui::Keystroke;
    use std::{
        cell::RefCell,
        io::{self, BufReader},
        path::PathBuf,
        sync::Arc,
        time::Duration,
    };

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

    #[test]
    fn keyboard_commands_cover_confirm_cancel_and_physical_nudging() {
        assert_eq!(
            keyboard_command(&Keystroke::parse("enter").unwrap()),
            Some(KeyboardCommand::Copy)
        );
        assert_eq!(
            keyboard_command(&Keystroke::parse("escape").unwrap()),
            Some(KeyboardCommand::Cancel)
        );
        assert_eq!(
            keyboard_command(&Keystroke::parse("left").unwrap()),
            Some(KeyboardCommand::Nudge(-1, 0))
        );
        assert_eq!(
            keyboard_command(&Keystroke::parse("shift-down").unwrap()),
            Some(KeyboardCommand::Nudge(0, 10))
        );
        assert_eq!(
            keyboard_command(&Keystroke::parse("ctrl-enter").unwrap()),
            None
        );
    }

    #[test]
    fn save_writes_the_selected_region_as_png() {
        let directory = std::env::temp_dir().join(format!(
            "flash-shot-workflow-save-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let path = directory.join("selection.png");
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
            cpu_copy_count: 1,
        };

        save_frame_selection(
            &frame,
            PhysicalRect {
                left: 1,
                top: 0,
                right: 2,
                bottom: 1,
            },
            path.clone(),
        )
        .unwrap();

        let decoder = png::Decoder::new(BufReader::new(std::fs::File::open(&path).unwrap()));
        let mut reader = decoder.read_info().unwrap();
        let mut output = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut output).unwrap();
        assert_eq!((info.width, info.height), (1, 1));
        assert_eq!(&output[..info.buffer_size()], &[6, 5, 4, 255]);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn save_path_always_uses_a_png_extension() {
        assert_eq!(
            png_path(PathBuf::from("capture")),
            PathBuf::from("capture.png")
        );
        assert_eq!(
            png_path(PathBuf::from("capture.jpg")),
            PathBuf::from("capture.png")
        );
        assert_eq!(
            png_path(PathBuf::from("capture.PNG")),
            PathBuf::from("capture.PNG")
        );
    }

    #[test]
    fn inspected_targets_are_clipped_to_the_captured_desktop() {
        assert_eq!(
            intersect_rect(
                PhysicalRect {
                    left: -2200,
                    top: 100,
                    right: -200,
                    bottom: 900,
                },
                PhysicalRect {
                    left: -1920,
                    top: 0,
                    right: 1920,
                    bottom: 1080,
                },
            ),
            Some(PhysicalRect {
                left: -1920,
                top: 100,
                right: -200,
                bottom: 900,
            })
        );
    }

    #[test]
    fn click_jitter_uses_smart_target_but_drag_keeps_free_selection() {
        let target = InspectionTarget {
            bounds: PhysicalRect {
                left: 100,
                top: 100,
                right: 500,
                bottom: 400,
            },
            kind: InspectionKind::Control,
        };
        assert_eq!(
            resolve_pointer_selection(
                PhysicalRect {
                    left: 200,
                    top: 200,
                    right: 202,
                    bottom: 201,
                },
                Some(target),
            ),
            Some(target.bounds)
        );

        let drag = PhysicalRect {
            left: 200,
            top: 200,
            right: 240,
            bottom: 260,
        };
        assert_eq!(resolve_pointer_selection(drag, Some(target)), Some(drag));
    }
}
