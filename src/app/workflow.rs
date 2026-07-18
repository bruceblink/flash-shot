//! Capture, selection, and clipboard workflow orchestration.

use std::{
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use gpui::{
    AppContext, AsyncApp, Bounds, Context, DisplayId, Focusable, KeyDownEvent, Keystroke,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathPromptOptions, Pixels, RenderImage,
    WeakEntity, WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions, point, px,
    size,
};

use super::{
    FlashShotApp, overlay::CaptureOverlay, pinned::PinnedImage,
    render_image::render_image_from_capture, scroll_control::ManualScrollControl,
};
use crate::{
    domain::{
        annotation::{AnnotationCommand, AnnotationDocument, AnnotationId, AnnotationTool},
        geometry::PhysicalRect,
        selection::{PreviewTransform, ViewPoint, ViewRect},
        session::CaptureSessionState,
    },
    performance::CapturePipelineSample,
    platform::{
        capture::{
            CaptureBackend, CaptureFrame, SystemCaptureBackend, capture_displays,
            compose_virtual_desktop,
        },
        clipboard::{ClipboardService, SystemClipboard},
        display::{DisplayProvider, SystemDisplayProvider},
        window_inspector::{InspectionTarget, SystemWindowInspector, WindowInspector},
        window_visibility,
    },
    recording::{
        RecordingEvent, RecordingProgress, RecordingRequest, RecordingTarget, discover,
        start_recording,
    },
};

impl FlashShotApp {
    pub(super) fn toggle_primary_display_recording(&mut self, cx: &mut Context<Self>) {
        if let Some(control) = self.recording_control.as_ref() {
            match control.request_stop() {
                Ok(()) => self.status = "Stopping screen recording...".to_owned(),
                Err(error) => self.status = format!("Could not stop screen recording: {error}"),
            }
            cx.notify();
            return;
        }
        if self.recording_start_in_flight {
            self.status = "Screen recording startup is already in progress...".to_owned();
            cx.notify();
            return;
        }
        if self.session.state() != CaptureSessionState::Idle {
            self.status = "Finish or cancel the current screenshot before recording".to_owned();
            cx.notify();
            return;
        }
        self.recording_start_in_flight = true;
        self.status = "Discovering FFmpeg and preparing primary display recording...".to_owned();
        self.start_recording_request(None, cx);
    }

    pub(super) fn start_region_recording(&mut self, cx: &mut Context<Self>) {
        let Some(bounds) = self.selection_drag.selection() else {
            self.status = "Select a region before starting a recording".to_owned();
            cx.notify();
            return;
        };
        if self.recording_control.is_some() || self.recording_start_in_flight {
            return;
        }
        self.recording_start_in_flight = true;
        self.status = "Preparing region recording...".to_owned();
        self.close_capture_overlays(cx);
        let _ = self.session.cancel();
        let _ = self.session.reset();
        self.frame = None;
        self.preview = None;
        self.selection_drag.clear();
        self.annotation_document = None;
        self.annotation_history = Default::default();
        self.annotation_editor = Default::default();
        self.start_recording_request(Some(bounds), cx);
    }

    pub(super) fn start_selected_window_recording(&mut self, cx: &mut Context<Self>) {
        let Some(selection) = self.selection_drag.selection() else {
            self.status = "Select a window before starting a recording".to_owned();
            cx.notify();
            return;
        };
        if self.recording_control.is_some() || self.recording_start_in_flight {
            return;
        }
        let center = crate::domain::geometry::PhysicalPoint {
            x: selection.left + selection.width() as i32 / 2,
            y: selection.top + selection.height() as i32 / 2,
        };
        self.recording_start_in_flight = true;
        self.status = "Looking up selected window for recording...".to_owned();
        self.close_capture_overlays(cx);
        let _ = self.session.cancel();
        let _ = self.session.reset();
        self.frame = None;
        self.preview = None;
        self.selection_drag.clear();
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result =
                    cx.background_executor()
                        .spawn(async move {
                            let title = SystemWindowInspector.window_title_at(center)?.ok_or_else(
                                || {
                                    std::io::Error::new(
                                        std::io::ErrorKind::NotFound,
                                        "no recordable top-level window at the selected area",
                                    )
                                },
                            )?;
                            start_recording_target(Some(RecordingTarget::Window { title }))
                        })
                        .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| this.recording_started(result, cx));
                }
            }
        })
        .detach();
    }

    fn start_recording_request(&mut self, region: Option<PhysicalRect>, cx: &mut Context<Self>) {
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        start_recording_target(
                            region.map(|bounds| RecordingTarget::Region { bounds }),
                        )
                    })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| this.recording_started(result, cx));
                }
            }
        })
        .detach();
    }

    pub(super) fn toggle_recording_pause(&mut self, cx: &mut Context<Self>) {
        let Some(control) = self.recording_control.as_ref() else {
            return;
        };
        let paused = !self.recording_paused;
        match control.set_paused(paused) {
            Ok(()) => {
                self.status = if paused {
                    "Pausing screen recording...".to_owned()
                } else {
                    "Resuming screen recording...".to_owned()
                }
            }
            Err(error) => self.status = format!("Could not change recording pause state: {error}"),
        }
        cx.notify();
    }

    fn recording_started(
        &mut self,
        result: std::io::Result<crate::recording::RecordingControl>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(control) => {
                let events = control.events();
                self.recording_control = Some(control);
                self.recording_progress = Default::default();
                self.recording_paused = false;
                self.status = "Starting primary display recording...".to_owned();
                cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                    let mut cx = cx.clone();
                    async move {
                        while let Ok(event) = events.recv().await {
                            let Some(this) = this.upgrade() else {
                                break;
                            };
                            this.update(&mut cx, |this, cx| this.handle_recording_event(event, cx));
                        }
                    }
                })
                .detach();
            }
            Err(error) => self.status = format!("Could not start screen recording: {error}"),
        }
        self.recording_start_in_flight = false;
        cx.notify();
    }

    fn handle_recording_event(&mut self, event: RecordingEvent, cx: &mut Context<Self>) {
        match event {
            RecordingEvent::Started => self.status = "Recording primary display...".to_owned(),
            RecordingEvent::Paused => {
                self.recording_paused = true;
                self.status = "Screen recording paused".to_owned();
            }
            RecordingEvent::Resumed => {
                self.recording_paused = false;
                self.status = "Recording primary display...".to_owned();
            }
            RecordingEvent::Progress(progress) => {
                self.recording_progress = progress;
                self.status = format_recording_progress(progress);
            }
            RecordingEvent::Finished { output } => {
                self.recording_control = None;
                self.recording_progress = Default::default();
                self.recording_paused = false;
                self.status = format!("Screen recording saved to {}", output.display());
            }
            RecordingEvent::Failed { message } => {
                self.recording_control = None;
                self.recording_progress = Default::default();
                self.recording_paused = false;
                self.status = format!("Screen recording failed: {message}");
            }
        }
        cx.notify();
    }

    pub(super) fn open_image(&mut self, cx: &mut Context<Self>) {
        if self.session.state() != CaptureSessionState::Idle {
            return;
        }
        if let Err(error) = self.session.begin() {
            self.status = error.to_string();
            cx.notify();
            return;
        }
        self.operation_generation = self.operation_generation.wrapping_add(1);
        let generation = self.operation_generation;
        self.status = "Choose a PNG image to annotate...".to_owned();
        cx.notify();

        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open PNG image".into()),
        });
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let outcome = match prompt.await {
                    Ok(Ok(Some(mut paths))) => match paths.pop() {
                        Some(path) => match cx
                            .background_executor()
                            .spawn(async move {
                                CaptureFrame::open_png(&path).map(|frame| (path, frame))
                            })
                            .await
                        {
                            Ok((path, frame)) => OpenImageOutcome::Opened { path, frame },
                            Err(error) => OpenImageOutcome::Failed(error.to_string()),
                        },
                        None => OpenImageOutcome::Cancelled,
                    },
                    Ok(Ok(None)) => OpenImageOutcome::Cancelled,
                    Ok(Err(error)) => OpenImageOutcome::Failed(error.to_string()),
                    Err(error) => OpenImageOutcome::Failed(error.to_string()),
                };
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.finish_open_image(outcome, generation, cx)
                    });
                }
            }
        })
        .detach();
    }

    fn finish_open_image(
        &mut self,
        outcome: OpenImageOutcome,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if !is_current_operation(self.operation_generation, generation) {
            return;
        }
        match outcome {
            OpenImageOutcome::Opened { path, frame } => {
                let bounds = frame.bounds;
                let result = (|| -> std::io::Result<()> {
                    self.session.frames_ready().map_err(std::io::Error::other)?;
                    let preview = render_image_from_capture(&frame)?;
                    let document =
                        AnnotationDocument::new(bounds).map_err(std::io::Error::other)?;
                    self.session.select(bounds).map_err(std::io::Error::other)?;
                    self.preview = Some(preview.image);
                    self.frame = Some(frame);
                    self.annotation_document = Some(document);
                    self.annotation_history = Default::default();
                    self.annotation_editor = Default::default();
                    self.annotation_tool = None;
                    self.text_edit = None;
                    self.selected_annotation = None;
                    self.next_annotation_id = 1;
                    self.next_sequence_number = 1;
                    self.selection_drag.select(bounds);
                    Ok(())
                })();
                match result {
                    Ok(()) => {
                        self.status = format!("Opened {} for annotation", path.display());
                        if let Some(handle) = self.main_window_handle
                            && let Err(error) = window_visibility::hide(handle)
                        {
                            let message = format!("Could not hide the main window: {error}");
                            let _ = self.session.fail(message.clone());
                            self.status = message;
                            cx.notify();
                            return;
                        }
                        let app = cx.entity();
                        cx.defer(move |cx| open_image_overlay(app, bounds, cx));
                    }
                    Err(error) => {
                        let message = format!("Could not open image: {error}");
                        let _ = self.session.fail(message.clone());
                        self.status = message;
                    }
                }
            }
            OpenImageOutcome::Cancelled => {
                let _ = self.session.cancel();
                let _ = self.session.reset();
                self.status = "Open image cancelled".to_owned();
            }
            OpenImageOutcome::Failed(error) => {
                let message = format!("Could not open image: {error}");
                let _ = self.session.fail(message.clone());
                self.status = message;
            }
        }
        cx.notify();
    }

    pub(super) fn start_capture(&mut self, cx: &mut Context<Self>) {
        if self.session.state() != CaptureSessionState::Idle {
            return;
        }
        if let Err(error) = self.session.begin() {
            self.status = error.to_string();
            cx.notify();
            return;
        }
        self.operation_generation = self.operation_generation.wrapping_add(1);
        let generation = self.operation_generation;
        self.frame = None;
        self.annotation_document = None;
        self.annotation_history = Default::default();
        self.annotation_editor = Default::default();
        self.annotation_tool = None;
        self.selected_annotation = None;
        self.preview = None;
        self.selection_drag.clear();
        self.hover_pixel = None;
        self.inspection_target = None;
        self.pending_click_target = None;
        self.inspection_request = None;
        self.manual_scroll = Default::default();
        self.manual_scroll_selection = None;
        self.manual_scroll_capture_in_flight = false;
        self.status = "Capturing virtual desktop...".to_owned();
        if let Some(handle) = self.main_window_handle
            && let Err(error) = window_visibility::hide(handle)
        {
            let message = format!("Could not hide the main window: {error}");
            let _ = self.session.fail(message.clone());
            self.status = message;
            cx.notify();
            return;
        }
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
                        this.finish_capture(result, started_at, generation, cx)
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
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if !is_current_operation(self.operation_generation, generation) {
            return;
        }
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
                let frame_ready_at = Instant::now();
                self.performance.record_duration(
                    "shortcut_to_frame_ready",
                    frame_ready_at.duration_since(started_at),
                );
                self.status = format!(
                    "{} x {} physical pixels - {} display(s) - {:.1} ms - {} CPU copy",
                    capture.capture.frame.width,
                    capture.capture.frame.height,
                    capture.capture.display_count,
                    capture.capture.frame.capture_duration.as_secs_f64() * 1_000.0,
                    capture.capture.frame.cpu_copy_count
                );
                let pipeline = CapturePipelineMeasurement {
                    started_at,
                    frame_ready_at,
                    platform_capture: capture.capture.frame.capture_duration,
                    display_count: capture.capture.display_count,
                    frame_width: capture.capture.frame.width,
                    frame_height: capture.capture.frame.height,
                    capture_cpu_copy_count: capture.capture.frame.cpu_copy_count,
                    render_upload_copy_count: (capture.displays.len() + 1) as u32,
                    overlay_image_count: capture.displays.len(),
                    overlay_upload_bytes: capture
                        .displays
                        .iter()
                        .map(|display| display.upload_bytes)
                        .sum(),
                    workspace_upload_bytes: capture.workspace_preview.upload_bytes,
                };
                let annotation_document =
                    match AnnotationDocument::new(capture.capture.frame.bounds) {
                        Ok(document) => document,
                        Err(error) => {
                            let message = format!("Could not create annotation document: {error}");
                            let _ = self.session.fail(message.clone());
                            self.status = message;
                            self.restore_main_window();
                            cx.notify();
                            return;
                        }
                    };
                self.preview = Some(capture.workspace_preview.image);
                self.annotation_document = Some(annotation_document);
                self.annotation_history = Default::default();
                self.annotation_editor = Default::default();
                self.annotation_tool = None;
                self.text_edit = None;
                self.next_annotation_id = 1;
                self.next_sequence_number = 1;
                self.frame = Some(capture.capture.frame);
                let app = cx.entity();
                cx.defer(move |cx| open_capture_overlays(app, capture.displays, pipeline, cx));
            }
            Err(error) => {
                let message = format!("Capture failed: {error}");
                let _ = self.session.fail(message.clone());
                self.status = message;
                log::warn!(target: "flash_shot::capture", "capture_failed error={error}");
                self.restore_main_window();
            }
        }
        cx.notify();
    }

    pub(super) fn reset(&mut self, cx: &mut Context<Self>) {
        match self.session.state() {
            CaptureSessionState::Capturing
            | CaptureSessionState::Selecting
            | CaptureSessionState::Exporting => {
                let _ = self.session.cancel();
                let _ = self.session.reset();
            }
            CaptureSessionState::Completed
            | CaptureSessionState::Cancelled
            | CaptureSessionState::Failed => {
                let _ = self.session.reset();
            }
            CaptureSessionState::Idle => {}
        }
        self.operation_generation = self.operation_generation.wrapping_add(1);
        self.frame = None;
        self.annotation_document = None;
        self.annotation_history = Default::default();
        self.annotation_editor = Default::default();
        self.annotation_tool = None;
        self.text_edit = None;
        self.selected_annotation = None;
        self.preview = None;
        self.selection_drag.clear();
        self.hover_pixel = None;
        self.inspection_target = None;
        self.pending_click_target = None;
        self.inspection_request = None;
        self.manual_scroll = Default::default();
        self.manual_scroll_selection = None;
        self.manual_scroll_capture_in_flight = false;
        self.status = "Ready - Ctrl+Shift+Print Screen".to_owned();
        self.close_capture_overlays(cx);
        self.close_manual_scroll_window(cx);
        self.restore_main_window();
        cx.notify();
    }

    pub(super) fn shutdown(&mut self, _cx: &mut Context<Self>) {
        self.operation_generation = self.operation_generation.wrapping_add(1);
        if self.session.state() != CaptureSessionState::Idle {
            let _ = self.session.cancel();
        }
        self.frame = None;
        self.annotation_document = None;
        self.annotation_history = Default::default();
        self.annotation_editor = Default::default();
        self.annotation_tool = None;
        self.text_edit = None;
        self.preview = None;
        self.selection_drag.clear();
        self.hover_pixel = None;
        self.inspection_target = None;
        self.pending_click_target = None;
        self.inspection_request = None;
        self.manual_scroll = Default::default();
        self.manual_scroll_selection = None;
        self.manual_scroll_capture_in_flight = false;
        self.recording_control = None;
        self.recording_start_in_flight = false;
        self.recording_paused = false;
        // GPUI has already removed native windows before invoking on_app_quit.
        // Keeping the handles untouched avoids issuing late operations on closed HWNDs.
        log::info!(target: "flash_shot::lifecycle", "capture_workflow_shutdown");
    }

    pub(super) fn begin_overlay_selection(
        &mut self,
        point: crate::domain::geometry::PhysicalPoint,
        resize_handle: Option<crate::domain::selection::ResizeHandle>,
        annotation_resize_handle: Option<crate::domain::selection::ResizeHandle>,
    ) {
        if self.annotation_tool.is_some() {
            self.begin_annotation(point);
            return;
        }
        if let (Some(document), Some(id), Some(handle)) = (
            self.annotation_document.as_ref(),
            self.selected_annotation,
            annotation_resize_handle,
        ) && self
            .annotation_editor
            .begin_resize(document, id, handle)
            .is_ok()
        {
            self.status = "Resizing annotation...".to_owned();
            return;
        }
        if let Some(document) = self.annotation_document.as_ref()
            && let Some(annotation) = document.annotation_at(point, 6)
            && self
                .annotation_editor
                .begin_move(document, annotation.id, point)
                .is_ok()
        {
            self.selected_annotation = Some(annotation.id);
            self.annotation_style = annotation.style;
            self.status = "Moving annotation...".to_owned();
            return;
        }
        self.pending_click_target = self
            .inspection_target
            .filter(|target| target.bounds.contains(point));
        if let Some((selection, handle)) = self.selection_drag.selection().zip(resize_handle) {
            self.selection_drag.begin_resize(selection, handle);
        } else {
            self.selection_drag.begin(point);
        }
    }

    pub(super) fn update_overlay_selection(
        &mut self,
        point: crate::domain::geometry::PhysicalPoint,
        cx: &mut Context<Self>,
    ) {
        let Some(frame) = self.frame.as_ref() else {
            return;
        };
        if let Some(tool) = self.annotation_tool {
            let point = clamp_physical_point(point, frame.bounds);
            if let Some(document) = self.annotation_document.as_ref() {
                self.annotation_editor.update(document, point);
            }
            self.status = drawing_status(tool).to_owned();
            cx.notify();
            return;
        }
        if self.annotation_editor.moving().is_some() || self.annotation_editor.resizing().is_some()
        {
            if let Some(document) = self.annotation_document.as_ref() {
                self.annotation_editor.update(document, point);
            }
            self.status = if self.annotation_editor.resizing().is_some() {
                "Resizing annotation..."
            } else {
                "Moving annotation..."
            }
            .to_owned();
            cx.notify();
            return;
        }
        self.selection_drag
            .update(clamp_physical_point(point, frame.bounds));
        if let Some(selection) = self.selection_drag.selection() {
            self.status = selection_status(selection);
        }
        cx.notify();
    }

    pub(super) fn update_overlay_hover(
        &mut self,
        point: Option<crate::domain::geometry::PhysicalPoint>,
        cx: &mut Context<Self>,
    ) {
        if self.hover_pixel == point {
            return;
        }
        self.hover_pixel = point;
        if let Some(point) = point
            && self.selection_drag.selection().is_none()
            && !self
                .inspection_target
                .is_some_and(|target| target.bounds.contains(point))
        {
            self.request_inspection(point, cx);
        }
        self.update_status_for_hover();
        cx.notify();
    }

    pub(super) fn finish_overlay_selection(
        &mut self,
        point: crate::domain::geometry::PhysicalPoint,
        cx: &mut Context<Self>,
    ) {
        let Some(frame) = self.frame.as_ref() else {
            return;
        };
        if self.annotation_tool == Some(AnnotationTool::Text) && self.text_edit.is_some() {
            return;
        }
        if self.annotation_tool.is_some() {
            let point = clamp_physical_point(point, frame.bounds);
            if let Some(document) = self.annotation_document.as_ref() {
                self.annotation_editor.update(document, point);
            }
            self.finish_annotation(cx);
            return;
        }
        if self.annotation_editor.moving().is_some() || self.annotation_editor.resizing().is_some()
        {
            if let Some(document) = self.annotation_document.as_ref() {
                self.annotation_editor
                    .update(document, clamp_physical_point(point, frame.bounds));
            }
            self.finish_annotation(cx);
            return;
        }
        self.selection_drag
            .update(clamp_physical_point(point, frame.bounds));
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

    pub(super) fn select_rectangle_tool(&mut self, cx: &mut Context<Self>) {
        self.select_annotation_tool(AnnotationTool::Rectangle, cx);
    }

    pub(super) fn select_watermark_tool(&mut self, cx: &mut Context<Self>) {
        self.select_annotation_tool(AnnotationTool::Watermark, cx);
    }

    pub(super) fn select_text_tool(&mut self, cx: &mut Context<Self>) {
        self.select_annotation_tool(AnnotationTool::Text, cx);
    }

    pub(super) fn select_highlight_tool(&mut self, cx: &mut Context<Self>) {
        self.select_annotation_tool(AnnotationTool::Highlight, cx);
    }

    pub(super) fn select_mosaic_tool(&mut self, cx: &mut Context<Self>) {
        self.select_annotation_tool(AnnotationTool::Mosaic, cx);
    }

    pub(super) fn select_blur_tool(&mut self, cx: &mut Context<Self>) {
        self.select_annotation_tool(AnnotationTool::Blur, cx);
    }

    pub(super) fn select_number_tool(&mut self, cx: &mut Context<Self>) {
        self.select_annotation_tool(AnnotationTool::Number, cx);
    }

    pub(super) fn select_ellipse_tool(&mut self, cx: &mut Context<Self>) {
        self.select_annotation_tool(AnnotationTool::Ellipse, cx);
    }

    pub(super) fn select_line_tool(&mut self, cx: &mut Context<Self>) {
        self.select_annotation_tool(AnnotationTool::Line, cx);
    }

    pub(super) fn select_arrow_tool(&mut self, cx: &mut Context<Self>) {
        self.select_annotation_tool(AnnotationTool::Arrow, cx);
    }

    pub(super) fn select_freehand_tool(&mut self, cx: &mut Context<Self>) {
        self.select_annotation_tool(AnnotationTool::Freehand, cx);
    }

    pub(super) fn select_annotation_color(&mut self, color: u32, cx: &mut Context<Self>) {
        let opacity = self.annotation_style.stroke_rgba as u8;
        self.annotation_style.stroke_rgba = with_alpha(color, opacity);
        if self.selected_annotation.is_some() {
            self.annotation_style.fill_rgba =
                self.annotation_style.fill_rgba.map(|_| fill_color(color));
        }
        self.replace_selected_annotation_style(cx);
        self.status = "Annotation color selected".to_owned();
        cx.notify();
    }

    pub(super) fn select_annotation_width(&mut self, width: u32, cx: &mut Context<Self>) {
        self.annotation_style.stroke_width = width.max(1);
        self.replace_selected_annotation_style(cx);
        self.status = format!(
            "Annotation width: {} px",
            self.annotation_style.stroke_width
        );
        cx.notify();
    }

    pub(super) fn select_annotation_opacity(&mut self, opacity: u8, cx: &mut Context<Self>) {
        self.annotation_style.stroke_rgba = with_alpha(self.annotation_style.stroke_rgba, opacity);
        if let Some(fill) = self.annotation_style.fill_rgba {
            self.annotation_style.fill_rgba = Some(with_alpha(fill, fill_alpha(opacity)));
        }
        self.replace_selected_annotation_style(cx);
        self.status = format!("Annotation opacity: {}%", u16::from(opacity) * 100 / 255);
        cx.notify();
    }

    pub(super) fn toggle_annotation_fill(&mut self, cx: &mut Context<Self>) {
        self.annotation_style.fill_rgba = self
            .annotation_style
            .fill_rgba
            .is_none()
            .then(|| fill_color(self.annotation_style.stroke_rgba));
        self.replace_selected_annotation_style(cx);
        self.status = if self.annotation_style.fill_rgba.is_some() {
            "Shape fill enabled"
        } else {
            "Shape fill disabled"
        }
        .to_owned();
        cx.notify();
    }

    fn replace_selected_annotation_style(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(id) = self.selected_annotation else {
            return false;
        };
        let Some(document) = self.annotation_document.as_mut() else {
            return false;
        };
        let Some(existing) = document.annotation(id).cloned() else {
            self.selected_annotation = None;
            return false;
        };
        let replacement = crate::domain::annotation::Annotation {
            style: self.annotation_style,
            ..existing.clone()
        };
        if replacement == existing {
            return false;
        }
        match self
            .annotation_history
            .apply(document, AnnotationCommand::Replace(replacement))
        {
            Ok(()) => true,
            Err(error) => {
                self.status = error.to_string();
                cx.notify();
                false
            }
        }
    }

    pub(super) fn select_selection_tool(&mut self, cx: &mut Context<Self>) {
        self.annotation_editor.cancel();
        self.text_edit = None;
        self.annotation_tool = None;
        self.selected_annotation = None;
        self.status = "Selection tool selected".to_owned();
        cx.notify();
    }

    fn select_annotation_tool(&mut self, tool: AnnotationTool, cx: &mut Context<Self>) {
        self.annotation_editor.cancel();
        self.text_edit = None;
        self.annotation_tool = Some(tool);
        self.selected_annotation = None;
        self.status = tool_selected_status(tool).to_owned();
        cx.notify();
    }

    fn begin_annotation(&mut self, point: crate::domain::geometry::PhysicalPoint) {
        let (Some(document), Some(tool)) =
            (self.annotation_document.as_ref(), self.annotation_tool)
        else {
            return;
        };
        let id = AnnotationId::new(self.next_annotation_id);
        if tool == AnnotationTool::Text {
            self.annotation_editor.cancel();
            self.text_edit = Some(super::TextEdit::new(point));
            self.status = "Type text, then press Enter".to_owned();
            return;
        }
        let started = if tool == AnnotationTool::Number {
            self.annotation_editor.begin_number(
                document,
                id,
                style_for_tool(tool, self.annotation_style),
                point,
                self.next_sequence_number,
            )
        } else {
            self.annotation_editor.begin(
                document,
                id,
                tool,
                style_for_tool(tool, self.annotation_style),
                point,
            )
        };
        if started.is_ok() {
            self.next_annotation_id = self.next_annotation_id.saturating_add(1);
            self.status = drawing_status(tool).to_owned();
        }
    }

    fn finish_annotation(&mut self, cx: &mut Context<Self>) {
        let Some(document) = self.annotation_document.as_mut() else {
            return;
        };
        let tool = self.annotation_tool;
        let moving = self.annotation_editor.moving().is_some();
        let resizing = self.annotation_editor.resizing().is_some();
        let committed = match self
            .annotation_editor
            .commit(document, &mut self.annotation_history)
        {
            Ok(true) if moving => {
                self.status = "Annotation moved".to_owned();
                false
            }
            Ok(true) if resizing => {
                self.status = "Annotation resized".to_owned();
                false
            }
            Ok(true) => {
                self.status = annotation_added_status(tool).to_owned();
                tool == Some(AnnotationTool::Number)
            }
            Ok(false) if moving => {
                self.status = "Annotation move cancelled".to_owned();
                false
            }
            Ok(false) if resizing => {
                self.status = "Annotation resize cancelled".to_owned();
                false
            }
            Ok(false) => {
                self.status = annotation_cancelled_status(tool).to_owned();
                false
            }
            Err(error) => {
                self.status = error.to_string();
                false
            }
        };
        if committed {
            self.next_sequence_number = self.next_sequence_number.saturating_add(1);
        }
        cx.notify();
    }

    pub(super) fn commit_text_edit(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(edit) = self.text_edit.take() else {
            return false;
        };
        let Some(document) = self.annotation_document.as_ref() else {
            return false;
        };
        let id = AnnotationId::new(self.next_annotation_id);
        let started = self.annotation_editor.begin_text(
            document,
            id,
            self.annotation_style,
            edit.origin,
            edit.content,
        );
        if let Err(error) = started {
            self.status = error.to_string();
            cx.notify();
            return true;
        }
        self.next_annotation_id = self.next_annotation_id.saturating_add(1);
        self.finish_annotation(cx);
        true
    }

    pub(super) fn cancel_text_edit(&mut self, cx: &mut Context<Self>) -> bool {
        if self.text_edit.take().is_none() {
            return false;
        }
        self.status = "Text cancelled".to_owned();
        cx.notify();
        true
    }

    pub(super) fn text_edit(&self) -> Option<&super::TextEdit> {
        self.text_edit.as_ref()
    }

    pub(super) fn replace_text_edit(
        &mut self,
        replacement_range_utf16: Option<Range<usize>>,
        text: &str,
        marked_range_utf16: Option<Range<usize>>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(edit) = self.text_edit.as_mut() else {
            return false;
        };
        let range = replacement_range_utf16
            .as_ref()
            .map(|range| range_from_utf16(&edit.content, range))
            .or(edit.marked_range.clone())
            .unwrap_or(edit.selected_range.clone());
        edit.content.replace_range(range.clone(), text);
        let cursor = range.start + text.len();
        edit.selected_range = marked_range_utf16
            .as_ref()
            .map(|range| range_from_utf16(text, range))
            .map(|selection| range.start + selection.start..range.start + selection.end)
            .unwrap_or(cursor..cursor);
        edit.marked_range = marked_range_utf16.map(|_| range.start..cursor);
        self.status = "Editing text...".to_owned();
        cx.notify();
        true
    }

    pub(super) fn unmark_text_edit(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(edit) = self.text_edit.as_mut() else {
            return false;
        };
        edit.marked_range = None;
        cx.notify();
        true
    }

    pub(super) fn handle_text_edit_key(
        &mut self,
        keystroke: &Keystroke,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.text_edit.is_none() || keystroke.modifiers.shift || keystroke.modifiers.control {
            return false;
        }
        match keystroke.key.as_str() {
            "enter" => self.commit_text_edit(cx),
            "escape" => self.cancel_text_edit(cx),
            "backspace" => self.delete_text_edit(true, cx),
            "delete" => self.delete_text_edit(false, cx),
            "left" => self.move_text_cursor(false, cx),
            "right" => self.move_text_cursor(true, cx),
            _ => false,
        }
    }

    fn delete_text_edit(&mut self, backwards: bool, cx: &mut Context<Self>) -> bool {
        let Some(edit) = self.text_edit.as_ref() else {
            return false;
        };
        let range = if edit.selected_range.is_empty() {
            let cursor = edit.selected_range.end;
            if backwards {
                previous_char_boundary(&edit.content, cursor)..cursor
            } else {
                cursor..next_char_boundary(&edit.content, cursor)
            }
        } else {
            edit.selected_range.clone()
        };
        self.replace_text_edit(Some(range_to_utf16(&edit.content, &range)), "", None, cx)
    }

    fn move_text_cursor(&mut self, forward: bool, cx: &mut Context<Self>) -> bool {
        let Some(edit) = self.text_edit.as_mut() else {
            return false;
        };
        let cursor = if forward {
            next_char_boundary(&edit.content, edit.selected_range.end)
        } else {
            previous_char_boundary(&edit.content, edit.selected_range.start)
        };
        edit.selected_range = cursor..cursor;
        edit.marked_range = None;
        cx.notify();
        true
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
            KeyboardCommand::Undo => self.undo_annotation(cx),
            KeyboardCommand::Redo => self.redo_annotation(cx),
            KeyboardCommand::Delete => self.delete_selected_annotation(cx),
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
            KeyboardCommand::QuickSave => {
                if self.session.state() == CaptureSessionState::Selecting
                    && self.session.selection().is_some()
                {
                    self.quick_save_selection(cx);
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

    pub(super) fn undo_annotation(&mut self, cx: &mut Context<Self>) -> bool {
        self.annotation_editor.cancel();
        let Some(document) = self.annotation_document.as_mut() else {
            return false;
        };
        match self.annotation_history.undo(document) {
            Ok(true) => {
                self.status = "Annotation undone".to_owned();
                cx.notify();
                true
            }
            Ok(false) => false,
            Err(error) => {
                self.status = error.to_string();
                cx.notify();
                true
            }
        }
    }

    pub(super) fn redo_annotation(&mut self, cx: &mut Context<Self>) -> bool {
        self.annotation_editor.cancel();
        let Some(document) = self.annotation_document.as_mut() else {
            return false;
        };
        match self.annotation_history.redo(document) {
            Ok(true) => {
                self.status = "Annotation redone".to_owned();
                cx.notify();
                true
            }
            Ok(false) => false,
            Err(error) => {
                self.status = error.to_string();
                cx.notify();
                true
            }
        }
    }

    pub(super) fn delete_selected_annotation(&mut self, cx: &mut Context<Self>) -> bool {
        self.annotation_editor.cancel();
        let Some(id) = self.selected_annotation else {
            return false;
        };
        let Some(document) = self.annotation_document.as_mut() else {
            return false;
        };
        match self
            .annotation_history
            .apply(document, AnnotationCommand::Delete(id))
        {
            Ok(()) => {
                self.selected_annotation = None;
                self.status = "Annotation deleted".to_owned();
                cx.notify();
                true
            }
            Err(error) => {
                self.status = error.to_string();
                cx.notify();
                true
            }
        }
    }

    pub(super) fn bring_selected_annotation_to_front(&mut self, cx: &mut Context<Self>) -> bool {
        self.reorder_selected_annotation(usize::MAX, "Annotation brought to front", cx)
    }

    pub(super) fn send_selected_annotation_to_back(&mut self, cx: &mut Context<Self>) -> bool {
        self.reorder_selected_annotation(0, "Annotation sent to back", cx)
    }

    fn reorder_selected_annotation(
        &mut self,
        index: usize,
        status: &'static str,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(id) = self.selected_annotation else {
            return false;
        };
        let Some(document) = self.annotation_document.as_mut() else {
            return false;
        };
        let target = index.min(document.annotations().len().saturating_sub(1));
        match self
            .annotation_history
            .apply(document, AnnotationCommand::Reorder { id, index: target })
        {
            Ok(()) => {
                self.status = status.to_owned();
                cx.notify();
                true
            }
            Err(error) => {
                self.status = error.to_string();
                cx.notify();
                true
            }
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
        self.update_status_for_hover();
        cx.notify();
    }

    fn update_status_for_hover(&mut self) {
        if let Some((point, color)) = self.hover_pixel.and_then(|point| {
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
                self.update_status_for_hover();
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

    pub(super) fn pin_selection(&mut self, cx: &mut Context<Self>) {
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
        let pinned = match render_image_from_capture(&pinned_frame) {
            Ok(image) => image,
            Err(error) => {
                self.status = format!("Could not render pinned selection: {error}");
                cx.notify();
                return;
            }
        };
        let window_size = pinned_size(pinned_frame.width as f32, pinned_frame.height as f32);
        let window_bounds = WindowBounds::centered(window_size, cx);
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
                window_min_size: Some(size(px(180.0), px(140.0))),
                ..Default::default()
            },
            move |window, cx| {
                let pinned = cx.new(|cx| PinnedImage::new(pinned.image, cx));
                pinned.read(cx).focus_handle(cx).focus(window, cx);
                pinned
            },
        ) {
            Ok(_) => {
                self.status = "Selection pinned in an always-on-top window".to_owned();
            }
            Err(error) => {
                self.status = format!("Could not open pinned window: {error}");
                log::warn!(target: "flash_shot::pinned", "pinned_window_open_failed error={error}");
            }
        }
        cx.notify();
    }

    pub(super) fn start_manual_scroll(&mut self, cx: &mut Context<Self>) {
        let Some(selection) = self.session.selection() else {
            self.status = "Select an area before starting manual scroll capture".to_owned();
            cx.notify();
            return;
        };
        let Some(frame) = self.frame.as_ref() else {
            self.status = "Capture frame is unavailable".to_owned();
            cx.notify();
            return;
        };
        let first = match frame.crop(selection) {
            Ok(frame) => frame,
            Err(error) => {
                self.status = format!("Could not start manual scroll: {error}");
                cx.notify();
                return;
            }
        };
        if self.manual_scroll.state() == crate::scroll::ManualScrollState::Collecting {
            self.status = "Manual scroll capture is already active".to_owned();
            cx.notify();
            return;
        }
        if self.manual_scroll.state() != crate::scroll::ManualScrollState::Idle {
            let _ = self.manual_scroll.reset();
        }
        if let Err(error) = self.manual_scroll.begin(first) {
            self.status = format!("Could not start manual scroll: {error}");
            cx.notify();
            return;
        }
        self.manual_scroll_selection = Some(selection);
        self.status =
            "Manual scroll started. Scroll the target, then capture the next frame.".to_owned();
        self.close_capture_overlays(cx);
        let app = cx.entity();
        cx.defer(move |cx| open_manual_scroll_control(app, cx));
        cx.notify();
    }

    pub(super) fn capture_manual_scroll_frame(&mut self, cx: &mut Context<Self>) {
        let Some(selection) = self.manual_scroll_selection else {
            self.status = "Manual scroll capture is not active".to_owned();
            cx.notify();
            return;
        };
        if self.manual_scroll.state() != crate::scroll::ManualScrollState::Collecting {
            self.status = "Manual scroll capture is not collecting frames".to_owned();
            cx.notify();
            return;
        }
        if self.manual_scroll_capture_in_flight {
            self.status = "Scroll frame capture is already in progress".to_owned();
            cx.notify();
            return;
        }
        self.manual_scroll_capture_in_flight = true;
        self.status = "Capturing next scroll frame...".to_owned();
        let generation = self.operation_generation;
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move { SystemCaptureBackend.capture(selection) })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.finish_manual_scroll_frame(result, generation, cx)
                    });
                }
            }
        })
        .detach();
    }

    fn finish_manual_scroll_frame(
        &mut self,
        result: std::io::Result<CaptureFrame>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if !is_current_operation(self.operation_generation, generation) {
            return;
        }
        self.manual_scroll_capture_in_flight = false;
        self.status = match result {
            Ok(frame) => match self.manual_scroll.append(frame, Default::default()) {
                Ok(overlap) => format!(
                    "Captured scroll frame {} ({} px overlap)",
                    self.manual_scroll.frame_count(),
                    overlap
                ),
                Err(error) => format!("Manual scroll stopped: {error}"),
            },
            Err(error) => format!("Could not capture scroll frame: {error}"),
        };
        cx.notify();
    }

    pub(super) fn finish_manual_scroll(&mut self, cx: &mut Context<Self>) {
        if self.manual_scroll_capture_in_flight {
            self.status = "Wait for the current scroll frame capture to finish".to_owned();
            cx.notify();
            return;
        }
        let stitched = match self.manual_scroll.finish(Default::default()) {
            Ok(stitched) => stitched,
            Err(error) => {
                self.status = format!("Could not finish manual scroll: {error}");
                cx.notify();
                return;
            }
        };
        let frame = stitched.frame;
        let bounds = frame.bounds;
        let result = (|| -> std::io::Result<()> {
            let preview = render_image_from_capture(&frame)?;
            let document = AnnotationDocument::new(bounds).map_err(std::io::Error::other)?;
            self.session.select(bounds).map_err(std::io::Error::other)?;
            self.preview = Some(preview.image);
            self.frame = Some(frame);
            self.annotation_document = Some(document);
            self.annotation_history = Default::default();
            self.annotation_editor = Default::default();
            self.annotation_tool = None;
            self.text_edit = None;
            self.selected_annotation = None;
            self.selection_drag.select(bounds);
            self.manual_scroll_selection = None;
            self.manual_scroll_capture_in_flight = false;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.status = format!(
                    "Manual scroll stitched {} frames with {} overlap joins",
                    self.manual_scroll.frame_count(),
                    stitched.overlaps.len()
                );
                self.close_manual_scroll_window(cx);
                let _ = self.manual_scroll.reset();
                let app = cx.entity();
                cx.defer(move |cx| open_image_overlay(app, bounds, cx));
            }
            Err(error) => self.status = format!("Could not open stitched capture: {error}"),
        }
        cx.notify();
    }

    pub(super) fn cancel_manual_scroll(&mut self, cx: &mut Context<Self>) {
        self.abandon_manual_scroll();
        self.close_manual_scroll_window(cx);
        self.status = "Manual scroll capture cancelled".to_owned();
        self.restore_main_window();
        cx.notify();
    }

    pub(super) fn manual_scroll_control_closed(&mut self, cx: &mut Context<Self>) {
        self.abandon_manual_scroll();
        self.scroll_window = None;
        self.status = "Manual scroll capture cancelled".to_owned();
        self.restore_main_window();
        cx.notify();
    }

    fn abandon_manual_scroll(&mut self) {
        if self.manual_scroll.state() == crate::scroll::ManualScrollState::Collecting {
            let _ = self.manual_scroll.cancel();
        }
        if self.manual_scroll.state() != crate::scroll::ManualScrollState::Idle {
            let _ = self.manual_scroll.reset();
        }
        self.manual_scroll_selection = None;
        self.manual_scroll_capture_in_flight = false;
    }

    pub(super) fn recognize_qr_selection(&mut self, cx: &mut Context<Self>) {
        let Some(selection) = self.session.selection() else {
            self.status = "Select an area before recognizing a QR code".to_owned();
            cx.notify();
            return;
        };
        let Some((frame, document)) = self.export_source() else {
            cx.notify();
            return;
        };

        self.status = "Recognizing QR code locally...".to_owned();
        let generation = self.operation_generation;
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        frame
                            .composite_annotations(&document)?
                            .crop(selection)?
                            .decode_qr_codes()
                    })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.finish_qr_recognition(result, generation, cx)
                    });
                }
            }
        })
        .detach();
    }

    fn finish_qr_recognition(
        &mut self,
        result: std::io::Result<Vec<String>>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if !is_current_operation(self.operation_generation, generation) {
            return;
        }
        self.status = match result {
            Ok(codes) if codes.is_empty() => "No QR code found in the selection".to_owned(),
            Ok(codes) if codes.len() == 1 => format!("QR: {}", codes[0]),
            Ok(codes) => format!("Found {} QR codes: {}", codes.len(), codes.join(" | ")),
            Err(error) => {
                log::warn!(target: "flash_shot::qr", "qr_recognition_failed error={error}");
                format!("QR recognition failed: {error}")
            }
        };
        cx.notify();
    }

    pub(super) fn recognize_text_selection(&mut self, cx: &mut Context<Self>) {
        let Some(selection) = self.session.selection() else {
            self.status = "Select an area before recognizing text".to_owned();
            cx.notify();
            return;
        };
        let Some((frame, document)) = self.export_source() else {
            cx.notify();
            return;
        };

        self.status = "Recognizing text locally...".to_owned();
        let generation = self.operation_generation;
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let frame = frame.composite_annotations(&document)?.crop(selection)?;
                        crate::ocr::recognize(&frame)
                    })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.finish_text_recognition(result, generation, cx)
                    });
                }
            }
        })
        .detach();
    }

    fn finish_text_recognition(
        &mut self,
        result: std::io::Result<String>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if !is_current_operation(self.operation_generation, generation) {
            return;
        }
        self.status = match result {
            Ok(text) if text.is_empty() => "No text found in the selection".to_owned(),
            Ok(text) => format!("OCR: {text}"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                "Local OCR is unavailable. Install Tesseract or set FLASH_SHOT_TESSERACT."
                    .to_owned()
            }
            Err(error) => {
                log::warn!(target: "flash_shot::ocr", "text_recognition_failed error={error}");
                format!("OCR failed: {error}")
            }
        };
        cx.notify();
    }

    pub(super) fn translate_selection(&mut self, cx: &mut Context<Self>) {
        let Some(selection) = self.session.selection() else {
            self.status = "Select an area before translating text".to_owned();
            cx.notify();
            return;
        };
        let config = match crate::translation::TranslationConfig::from_environment() {
            Ok(Some(config)) => config,
            Ok(None) => {
                self.status =
                    "Translation is disabled. Configure FLASH_SHOT_TRANSLATION_ENDPOINT to opt in."
                        .to_owned();
                cx.notify();
                return;
            }
            Err(error) => {
                self.status = format!("Translation is unavailable: {error}");
                cx.notify();
                return;
            }
        };
        let Some((frame, document)) = self.export_source() else {
            cx.notify();
            return;
        };

        self.status = "Recognizing and translating text...".to_owned();
        let generation = self.operation_generation;
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let frame = frame.composite_annotations(&document)?.crop(selection)?;
                        let text = crate::ocr::recognize(&frame)?;
                        crate::translation::translate(&config, &text)
                    })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.finish_translation(result, generation, cx)
                    });
                }
            }
        })
        .detach();
    }

    fn finish_translation(
        &mut self,
        result: std::io::Result<String>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if !is_current_operation(self.operation_generation, generation) {
            return;
        }
        self.status = match result {
            Ok(text) if text.is_empty() => "No text found in the selection".to_owned(),
            Ok(text) => format!("Translation: {text}"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                "Local OCR is unavailable. Install Tesseract or set FLASH_SHOT_TESSERACT."
                    .to_owned()
            }
            Err(error) => {
                log::warn!(target: "flash_shot::translation", "translation_failed error={error}");
                format!("Translation failed: {error}")
            }
        };
        cx.notify();
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
        let Some((frame, document)) = self.export_source() else {
            cx.notify();
            return;
        };

        self.status = "Choose where to save the selection...".to_owned();
        let generation = self.operation_generation;
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
                                save_annotated_frame_selection(
                                    &frame,
                                    &document,
                                    selection,
                                    path.clone(),
                                )
                                .map(|()| path)
                            })
                            .await;
                        match result {
                            Ok(path) => SaveOutcome::Saved {
                                path,
                                managed: false,
                            },
                            Err(error) => SaveOutcome::Failed(error.to_string()),
                        }
                    }
                    Ok(Ok(None)) => SaveOutcome::Cancelled,
                    Ok(Err(error)) => SaveOutcome::Failed(error.to_string()),
                    Err(error) => SaveOutcome::Failed(error.to_string()),
                };
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.finish_save(outcome, generation, cx)
                    });
                }
            }
        })
        .detach();
    }

    pub(super) fn quick_save_selection(&mut self, cx: &mut Context<Self>) {
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

        self.status = "Quick saving selection...".to_owned();
        let generation = self.operation_generation;
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        quick_save_annotated_frame_selection(&frame, &document, selection)
                    })
                    .await;
                let outcome = match result {
                    Ok(path) => SaveOutcome::Saved {
                        path,
                        managed: true,
                    },
                    Err(error) => SaveOutcome::Failed(error.to_string()),
                };
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.finish_save(outcome, generation, cx)
                    });
                }
            }
        })
        .detach();
    }

    fn export_source(&mut self) -> Option<(CaptureFrame, AnnotationDocument)> {
        match (self.frame.clone(), self.annotation_document.clone()) {
            (Some(frame), Some(document)) => Some((frame, document)),
            _ => {
                let message = "capture frame or annotation document is unavailable".to_owned();
                let _ = self.session.fail(message.clone());
                self.status = message;
                None
            }
        }
    }

    fn finish_save(&mut self, outcome: SaveOutcome, generation: u64, cx: &mut Context<Self>) {
        if !is_current_operation(self.operation_generation, generation) {
            return;
        }
        match outcome {
            SaveOutcome::Saved { path, managed } => {
                if let Err(error) = self.session.export_completed() {
                    self.status = error.to_string();
                } else {
                    let history_status = managed.then(|| self.history.record(path.clone())).transpose().err().map(|error| {
                        log::warn!(target: "flash_shot::history", "history_record_failed error={error}");
                        format!("; history unavailable: {error}")
                    });
                    self.status = format!("Selection saved to {}", path.display());
                    if let Some(history_status) = history_status {
                        self.status.push_str(&history_status);
                    }
                    self.close_capture_overlays(cx);
                    self.restore_main_window();
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
                self.close_capture_overlays(cx);
                self.restore_main_window();
            }
        }
        cx.notify();
    }

    pub(super) fn clear_history(&mut self, cx: &mut Context<Self>) {
        match self.history.clear() {
            Ok(()) => self.status = "Screenshot history cleared".to_owned(),
            Err(error) => {
                self.status = format!("Could not clear screenshot history: {error}");
                log::warn!(target: "flash_shot::history", "history_clear_failed error={error}");
            }
        }
        cx.notify();
    }

    fn finish_copy(
        &mut self,
        result: std::io::Result<()>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if !is_current_operation(self.operation_generation, generation) {
            return;
        }
        match result {
            Ok(()) => {
                if let Err(error) = self.session.export_completed() {
                    self.status = error.to_string();
                } else {
                    self.status = "Selection copied to clipboard".to_owned();
                    self.close_capture_overlays(cx);
                    self.restore_main_window();
                }
            }
            Err(error) => {
                let message = format!("Copy failed: {error}");
                let _ = self.session.fail(message.clone());
                self.status = message;
                self.close_capture_overlays(cx);
                self.restore_main_window();
            }
        }
        cx.notify();
    }

    pub(super) fn preview_transform(&self, viewport: Bounds<Pixels>) -> Option<PreviewTransform> {
        let frame = self.frame.as_ref()?;
        PreviewTransform::contain(frame.bounds, view_rect(viewport))
    }

    fn close_capture_overlays(&mut self, cx: &mut Context<Self>) {
        let windows = std::mem::take(&mut self.overlay_windows);
        if !windows.is_empty() {
            cx.defer(move |cx| close_overlay_windows(windows, cx));
        }
    }

    fn close_manual_scroll_window(&mut self, cx: &mut Context<Self>) {
        if let Some(window) = self.scroll_window.take() {
            cx.defer(move |cx| {
                let _ = window.update(cx, |_, window, _| window.remove_window());
            });
        }
    }

    fn restore_main_window(&self) {
        if let Some(handle) = self.main_window_handle
            && let Err(error) = window_visibility::restore(handle)
        {
            log::warn!(target: "flash_shot::overlay", "main_window_restore_failed error={error}");
        }
    }
}

fn tool_selected_status(tool: AnnotationTool) -> &'static str {
    match tool {
        AnnotationTool::Text => "Text tool selected",
        AnnotationTool::Watermark => "Watermark tool selected",
        AnnotationTool::Number => "Number tool selected",
        AnnotationTool::Blur => "Blur tool selected",
        AnnotationTool::Mosaic => "Mosaic tool selected",
        AnnotationTool::Highlight => "Highlight tool selected",
        AnnotationTool::Rectangle => "Rectangle tool selected",
        AnnotationTool::Ellipse => "Ellipse tool selected",
        AnnotationTool::Line => "Line tool selected",
        AnnotationTool::Arrow => "Arrow tool selected",
        AnnotationTool::Freehand => "Freehand tool selected",
    }
}

fn drawing_status(tool: AnnotationTool) -> &'static str {
    match tool {
        AnnotationTool::Text => "Editing text...",
        AnnotationTool::Watermark => "Placing watermark...",
        AnnotationTool::Number => "Placing number...",
        AnnotationTool::Blur => "Drawing blur...",
        AnnotationTool::Mosaic => "Drawing mosaic...",
        AnnotationTool::Highlight => "Drawing highlight...",
        AnnotationTool::Rectangle => "Drawing rectangle...",
        AnnotationTool::Ellipse => "Drawing ellipse...",
        AnnotationTool::Line => "Drawing line...",
        AnnotationTool::Arrow => "Drawing arrow...",
        AnnotationTool::Freehand => "Drawing freehand...",
    }
}

fn annotation_added_status(tool: Option<AnnotationTool>) -> &'static str {
    match tool {
        Some(AnnotationTool::Text) => "Text added",
        Some(AnnotationTool::Watermark) => "Watermark added",
        Some(AnnotationTool::Number) => "Number added",
        Some(AnnotationTool::Blur) => "Blur added",
        Some(AnnotationTool::Mosaic) => "Mosaic added",
        Some(AnnotationTool::Highlight) => "Highlight added",
        Some(AnnotationTool::Rectangle) => "Rectangle added",
        Some(AnnotationTool::Ellipse) => "Ellipse added",
        Some(AnnotationTool::Line) => "Line added",
        Some(AnnotationTool::Arrow) => "Arrow added",
        Some(AnnotationTool::Freehand) => "Freehand stroke added",
        _ => "Annotation added",
    }
}

fn annotation_cancelled_status(tool: Option<AnnotationTool>) -> &'static str {
    match tool {
        Some(AnnotationTool::Text) => "Text cancelled",
        Some(AnnotationTool::Watermark) => "Watermark cancelled",
        Some(AnnotationTool::Number) => "Number cancelled",
        Some(AnnotationTool::Blur) => "Blur cancelled",
        Some(AnnotationTool::Mosaic) => "Mosaic cancelled",
        Some(AnnotationTool::Highlight) => "Highlight cancelled",
        Some(AnnotationTool::Rectangle) => "Rectangle cancelled",
        Some(AnnotationTool::Ellipse) => "Ellipse cancelled",
        Some(AnnotationTool::Line) => "Line cancelled",
        Some(AnnotationTool::Arrow) => "Arrow cancelled",
        Some(AnnotationTool::Freehand) => "Freehand stroke cancelled",
        _ => "Annotation cancelled",
    }
}

fn is_current_operation(current: u64, completed: u64) -> bool {
    current == completed
}

fn open_capture_overlays(
    app: gpui::Entity<FlashShotApp>,
    displays: Vec<CapturedDisplayPreview>,
    pipeline: CapturePipelineMeasurement,
    cx: &mut gpui::App,
) {
    if app.read(cx).session.state() != CaptureSessionState::Selecting {
        return;
    }
    let mut windows = Vec::with_capacity(displays.len());
    for display in displays {
        let bounds = display_window_bounds(&display.display);
        let display_id = DisplayId::new(display.display.platform_id);
        let info = display.display;
        let primary = info.primary;
        let preview = display.preview;
        let performance = app.read(cx).performance.clone();
        let primary_pipeline = primary.then_some(pipeline);
        match cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: None,
                focus: primary,
                show: true,
                kind: WindowKind::PopUp,
                is_movable: false,
                is_resizable: false,
                is_minimizable: false,
                display_id: Some(display_id),
                window_background: WindowBackgroundAppearance::Opaque,
                window_min_size: None,
                ..Default::default()
            },
            {
                let app = app.clone();
                move |window, cx| {
                    if let Some(pipeline) = primary_pipeline {
                        window.on_next_frame(move |_, _| {
                            performance.record_capture_pipeline(pipeline.finish(Instant::now()));
                        });
                    }
                    let overlay = cx.new(|cx| CaptureOverlay::new(app, info, preview, cx));
                    if primary {
                        overlay.read(cx).focus_handle(cx).focus(window, cx);
                    }
                    overlay
                }
            },
        ) {
            Ok(window) => windows.push(window),
            Err(error) => {
                close_overlay_windows(windows, cx);
                let message = format!("Capture overlay failed: {error}");
                app.update(cx, |app, cx| {
                    let _ = app.session.fail(message.clone());
                    app.status = message;
                    app.restore_main_window();
                    cx.notify();
                });
                log::warn!(target: "flash_shot::overlay", "overlay_open_failed error={error}");
                return;
            }
        }
    }
    app.update(cx, |app, _| app.overlay_windows = windows);
    cx.activate(true);
}

fn open_image_overlay(app: gpui::Entity<FlashShotApp>, bounds: PhysicalRect, cx: &mut gpui::App) {
    if app.read(cx).session.state() != CaptureSessionState::Selecting {
        return;
    }
    let Some(preview) = app.read(cx).preview.clone() else {
        return;
    };
    let display = crate::platform::display::DisplayInfo {
        id: "opened-image".to_owned(),
        platform_id: 0,
        physical_bounds: bounds,
        work_area: bounds,
        dpi_x: 96,
        dpi_y: 96,
        scale_factor: 1.0,
        rotation: crate::platform::display::DisplayRotation::Landscape,
        bits_per_pixel: 32,
        primary: true,
    };
    let window_size = pinned_size(bounds.width() as f32, bounds.height() as f32);
    let overlay_app = app.clone();
    match cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::centered(window_size, cx)),
            titlebar: Some(gpui::TitlebarOptions {
                title: Some("Flash Shot - Edit Image".into()),
                ..Default::default()
            }),
            focus: true,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: true,
            is_resizable: true,
            is_minimizable: false,
            window_background: WindowBackgroundAppearance::Opaque,
            window_min_size: Some(size(px(480.0), px(360.0))),
            ..Default::default()
        },
        move |window, cx| {
            let overlay = cx.new(|cx| CaptureOverlay::new(overlay_app, display, preview, cx));
            overlay.read(cx).focus_handle(cx).focus(window, cx);
            overlay
        },
    ) {
        Ok(window) => {
            app.update(cx, |app, _| app.overlay_windows = vec![window]);
            cx.activate(true);
        }
        Err(error) => {
            let message = format!("Image editor window failed: {error}");
            app.update(cx, |app, cx| {
                let _ = app.session.fail(message.clone());
                app.status = message;
                app.restore_main_window();
                cx.notify();
            });
            log::warn!(target: "flash_shot::image", "image_editor_open_failed error={error}");
        }
    }
}

fn open_manual_scroll_control(app: gpui::Entity<FlashShotApp>, cx: &mut gpui::App) {
    if app.read(cx).manual_scroll.state() != crate::scroll::ManualScrollState::Collecting {
        return;
    }
    let control_app = app.clone();
    match cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(390.0), px(120.0)), cx)),
            titlebar: Some(gpui::TitlebarOptions {
                title: Some("Flash Shot - Manual Scroll".into()),
                ..Default::default()
            }),
            focus: true,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: true,
            is_resizable: false,
            is_minimizable: false,
            window_background: WindowBackgroundAppearance::Opaque,
            ..Default::default()
        },
        move |window, cx| {
            let close_app = control_app.clone();
            window.on_window_should_close(cx, move |_, cx| {
                close_app.update(cx, |app, cx| app.manual_scroll_control_closed(cx));
                true
            });
            let control = cx.new(|cx| ManualScrollControl::new(control_app, cx));
            control.read(cx).focus_handle(cx).focus(window, cx);
            control
        },
    ) {
        Ok(window) => app.update(cx, |app, _| app.scroll_window = Some(window)),
        Err(error) => {
            app.update(cx, |app, cx| {
                let _ = app.manual_scroll.cancel();
                let _ = app.manual_scroll.reset();
                app.manual_scroll_selection = None;
                app.manual_scroll_capture_in_flight = false;
                app.status = format!("Could not open manual scroll controls: {error}");
                app.restore_main_window();
                cx.notify();
            });
            log::warn!(target: "flash_shot::scroll", "manual_scroll_control_open_failed error={error}");
        }
    }
}

fn close_overlay_windows(windows: Vec<gpui::WindowHandle<CaptureOverlay>>, cx: &mut gpui::App) {
    for window in windows {
        let _ = window.update(cx, |_, window, _| window.remove_window());
    }
}

struct CapturedDesktopPreview {
    capture: crate::platform::capture::VirtualDesktopCapture,
    workspace_preview: super::render_image::CaptureRenderImage,
    displays: Vec<CapturedDisplayPreview>,
}

#[derive(Clone, Copy)]
struct CapturePipelineMeasurement {
    started_at: Instant,
    frame_ready_at: Instant,
    platform_capture: std::time::Duration,
    display_count: usize,
    frame_width: u32,
    frame_height: u32,
    capture_cpu_copy_count: u32,
    render_upload_copy_count: u32,
    overlay_image_count: usize,
    overlay_upload_bytes: usize,
    workspace_upload_bytes: usize,
}

impl CapturePipelineMeasurement {
    fn finish(self, overlay_frame_at: Instant) -> CapturePipelineSample {
        CapturePipelineSample {
            shortcut_to_frame_ready: self.frame_ready_at.duration_since(self.started_at),
            shortcut_to_overlay_frame: overlay_frame_at.duration_since(self.started_at),
            platform_capture: self.platform_capture,
            display_count: self.display_count,
            frame_width: self.frame_width,
            frame_height: self.frame_height,
            capture_cpu_copy_count: self.capture_cpu_copy_count,
            render_upload_copy_count: self.render_upload_copy_count,
            overlay_image_count: self.overlay_image_count,
            overlay_upload_bytes: self.overlay_upload_bytes,
            workspace_upload_bytes: self.workspace_upload_bytes,
        }
    }
}

struct CapturedDisplayPreview {
    display: crate::platform::display::DisplayInfo,
    preview: Arc<RenderImage>,
    upload_bytes: usize,
}

fn capture_virtual_desktop_preview() -> std::io::Result<CapturedDesktopPreview> {
    let display_captures = capture_displays()?;
    let frame = compose_virtual_desktop(&display_captures)?;
    let workspace_preview = render_image_from_capture(&frame)?;
    let displays = display_captures
        .into_iter()
        .map(|capture| {
            let preview = render_image_from_capture(&capture.frame)?;
            Ok(CapturedDisplayPreview {
                display: capture.display,
                preview: preview.image,
                upload_bytes: preview.upload_bytes,
            })
        })
        .collect::<std::io::Result<Vec<_>>>()?;
    Ok(CapturedDesktopPreview {
        capture: crate::platform::capture::VirtualDesktopCapture {
            display_count: displays.len(),
            frame,
        },
        workspace_preview,
        displays,
    })
}

fn display_window_bounds(display: &crate::platform::display::DisplayInfo) -> Bounds<Pixels> {
    let scale = display.scale_factor.max(1.0);
    Bounds::new(
        point(
            px(display.physical_bounds.left as f32 / scale),
            px(display.physical_bounds.top as f32 / scale),
        ),
        size(
            px(display.physical_bounds.width() as f32 / scale),
            px(display.physical_bounds.height() as f32 / scale),
        ),
    )
}

fn clamp_physical_point(
    point: crate::domain::geometry::PhysicalPoint,
    bounds: PhysicalRect,
) -> crate::domain::geometry::PhysicalPoint {
    crate::domain::geometry::PhysicalPoint {
        x: point.x.clamp(bounds.left, bounds.right),
        y: point.y.clamp(bounds.top, bounds.bottom),
    }
}

fn utf16_offset(text: &str, byte_offset: usize) -> usize {
    text[..byte_offset].chars().map(char::len_utf16).sum()
}

fn byte_offset(text: &str, utf16_offset: usize) -> usize {
    let mut bytes = 0;
    let mut units = 0;
    for character in text.chars() {
        if units >= utf16_offset {
            break;
        }
        units += character.len_utf16();
        bytes += character.len_utf8();
    }
    bytes
}

fn range_to_utf16(text: &str, range: &Range<usize>) -> Range<usize> {
    utf16_offset(text, range.start)..utf16_offset(text, range.end)
}

fn range_from_utf16(text: &str, range: &Range<usize>) -> Range<usize> {
    byte_offset(text, range.start)..byte_offset(text, range.end)
}

fn previous_char_boundary(text: &str, offset: usize) -> usize {
    text.char_indices()
        .rev()
        .find_map(|(index, _)| (index < offset).then_some(index))
        .unwrap_or(0)
}

fn next_char_boundary(text: &str, offset: usize) -> usize {
    text.char_indices()
        .find_map(|(index, _)| (index > offset).then_some(index))
        .unwrap_or(text.len())
}

fn copy_annotated_frame_selection(
    frame: &CaptureFrame,
    document: &AnnotationDocument,
    selection: PhysicalRect,
    clipboard: &impl ClipboardService,
) -> std::io::Result<()> {
    clipboard.copy_image(&frame.composite_annotations(document)?.crop(selection)?)
}

fn save_annotated_frame_selection(
    frame: &CaptureFrame,
    document: &AnnotationDocument,
    selection: PhysicalRect,
    path: PathBuf,
) -> std::io::Result<()> {
    frame
        .composite_annotations(document)?
        .crop(selection)?
        .save_png(path)
}

fn quick_save_annotated_frame_selection(
    frame: &CaptureFrame,
    document: &AnnotationDocument,
    selection: PhysicalRect,
) -> std::io::Result<PathBuf> {
    let directory = quick_save_directory()?;
    quick_save_annotated_frame_selection_in(
        frame,
        document,
        selection,
        &directory,
        unix_timestamp_ms(),
    )
}

fn quick_save_annotated_frame_selection_in(
    frame: &CaptureFrame,
    document: &AnnotationDocument,
    selection: PhysicalRect,
    directory: &Path,
    timestamp_ms: u128,
) -> std::io::Result<PathBuf> {
    let path = next_quick_save_path(directory, timestamp_ms, Path::exists);
    save_annotated_frame_selection(frame, document, selection, path.clone())?;
    Ok(path)
}

fn quick_save_directory() -> std::io::Result<PathBuf> {
    crate::history::managed_history_directory()
}

fn start_recording_target(
    target: Option<RecordingTarget>,
) -> std::io::Result<crate::recording::RecordingControl> {
    let capabilities = discover()?;
    let target = match target {
        Some(target) => target,
        None => RecordingTarget::Display {
            bounds: SystemDisplayProvider
                .displays()?
                .into_iter()
                .find(|display| display.primary)
                .ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::NotFound, "primary display not found")
                })?
                .physical_bounds,
        },
    };
    let output = recording_output_path()?;
    start_recording(
        capabilities,
        RecordingRequest {
            target,
            audio: None,
            frame_rate: 30,
            output,
        },
    )
}

fn recording_output_path() -> std::io::Result<PathBuf> {
    let root = directories::UserDirs::new()
        .and_then(|directories| directories.video_dir().map(Path::to_owned))
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "recording directory unavailable",
            )
        })?;
    let directory = root.join("Flash Shot");
    std::fs::create_dir_all(&directory)?;
    Ok(directory.join(format!("FlashShot-{}.mp4", unix_timestamp_ms())))
}

fn format_recording_progress(progress: RecordingProgress) -> String {
    let seconds = progress.output_time_us.unwrap_or_default() / 1_000_000;
    let frames = progress.frame.unwrap_or_default();
    format!("Recording primary display: {seconds}s, {frames} frames")
}

fn next_quick_save_path(
    directory: &Path,
    timestamp_ms: u128,
    exists: impl Fn(&Path) -> bool,
) -> PathBuf {
    let stem = format!("FlashShot-{timestamp_ms}");
    let initial = directory.join(format!("{stem}.png"));
    if !exists(&initial) {
        return initial;
    }
    for index in 2_u32.. {
        let path = directory.join(format!("{stem}-{index}.png"));
        if !exists(&path) {
            return path;
        }
    }
    unreachable!("u32 path suffixes cannot be exhausted")
}

fn unix_timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
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

fn fill_color(stroke_rgba: u32) -> u32 {
    with_alpha(stroke_rgba, fill_alpha(stroke_rgba as u8))
}

fn pinned_size(image_width: f32, image_height: f32) -> gpui::Size<Pixels> {
    const HEADER_HEIGHT: f32 = 26.0;
    const MAX_WIDTH: f32 = 640.0;
    const MAX_HEIGHT: f32 = 540.0;
    let width = image_width.max(1.0);
    let height = image_height.max(1.0);
    let scale = (MAX_WIDTH / width)
        .min((MAX_HEIGHT - HEADER_HEIGHT) / height)
        .min(1.0);
    size(
        px((width * scale).max(180.0)),
        px((height * scale + HEADER_HEIGHT).max(140.0)),
    )
}

fn with_alpha(color: u32, alpha: u8) -> u32 {
    (color & 0xFFFFFF00) | u32::from(alpha)
}

fn fill_alpha(stroke_alpha: u8) -> u8 {
    (u16::from(stroke_alpha) * 0x66 / 255) as u8
}

fn style_for_tool(
    tool: AnnotationTool,
    style: crate::domain::annotation::AnnotationStyle,
) -> crate::domain::annotation::AnnotationStyle {
    if tool == AnnotationTool::Highlight {
        crate::domain::annotation::AnnotationStyle {
            stroke_rgba: fill_color(style.stroke_rgba),
            fill_rgba: None,
            stroke_width: 1,
        }
    } else {
        style
    }
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
    Undo,
    Redo,
    Delete,
    Cancel,
    Copy,
    QuickSave,
    Nudge(i32, i32),
}

enum SaveOutcome {
    Saved { path: PathBuf, managed: bool },
    Cancelled,
    Failed(String),
}

enum OpenImageOutcome {
    Opened { path: PathBuf, frame: CaptureFrame },
    Cancelled,
    Failed(String),
}

fn keyboard_command(keystroke: &Keystroke) -> Option<KeyboardCommand> {
    let modifiers = keystroke.modifiers;
    if modifiers.secondary()
        && !modifiers.alt
        && !modifiers.platform
        && !modifiers.function
        && keystroke.key == "z"
    {
        return Some(if modifiers.shift {
            KeyboardCommand::Redo
        } else {
            KeyboardCommand::Undo
        });
    }
    if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
        return None;
    }
    match keystroke.key.as_str() {
        "delete" | "backspace" if !modifiers.shift => Some(KeyboardCommand::Delete),
        "escape" if !modifiers.shift => Some(KeyboardCommand::Cancel),
        "enter" if !modifiers.shift => Some(KeyboardCommand::Copy),
        "enter" if modifiers.shift => Some(KeyboardCommand::QuickSave),
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
        KeyboardCommand, annotation_added_status, annotation_cancelled_status,
        copy_annotated_frame_selection, drawing_status, fill_alpha, fill_color,
        format_recording_progress, intersect_rect, is_current_operation, keyboard_command,
        next_quick_save_path, pinned_size, png_path, quick_save_annotated_frame_selection_in,
        resolve_pointer_selection, save_annotated_frame_selection, style_for_tool,
        tool_selected_status, with_alpha,
    };
    use crate::platform::window_inspector::{InspectionKind, InspectionTarget};
    use crate::{
        domain::{
            annotation::{
                Annotation, AnnotationCommand, AnnotationDocument, AnnotationId, AnnotationKind,
                AnnotationStyle, CommandHistory,
            },
            geometry::{PhysicalPoint, PhysicalRect},
        },
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
        let document = AnnotationDocument::new(frame.bounds).unwrap();

        copy_annotated_frame_selection(
            &frame,
            &document,
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
    fn annotated_copy_composites_before_cropping_the_selection() {
        let frame = CaptureFrame {
            bounds: PhysicalRect {
                left: -2,
                top: 10,
                right: 2,
                bottom: 11,
            },
            width: 4,
            height: 1,
            stride: 16,
            format: PixelFormat::Bgra8,
            pixels: Arc::from([10, 10, 10, 255].repeat(4)),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 1,
        };
        let mut document = AnnotationDocument::new(frame.bounds).unwrap();
        let mut history = CommandHistory::default();
        history
            .apply(
                &mut document,
                AnnotationCommand::Insert(Annotation {
                    id: AnnotationId::new(1),
                    kind: AnnotationKind::Line {
                        start: PhysicalPoint { x: -1, y: 10 },
                        end: PhysicalPoint { x: 0, y: 10 },
                    },
                    style: AnnotationStyle {
                        stroke_rgba: 0xFF0000FF,
                        fill_rgba: None,
                        stroke_width: 1,
                    },
                }),
            )
            .unwrap();
        let clipboard = RecordingClipboard::default();

        copy_annotated_frame_selection(
            &frame,
            &document,
            PhysicalRect {
                left: -1,
                top: 10,
                right: 1,
                bottom: 11,
            },
            &clipboard,
        )
        .unwrap();

        let copied = clipboard.copied.borrow();
        let copied = copied.as_ref().unwrap();
        assert_eq!((copied.width, copied.height), (2, 1));
        assert_eq!(
            copied.pixel_at(PhysicalPoint { x: -1, y: 10 }).unwrap().red,
            255
        );
        assert_eq!(
            copied.pixel_at(PhysicalPoint { x: 0, y: 10 }).unwrap().red,
            255
        );
        assert_eq!(
            frame.pixel_at(PhysicalPoint { x: -2, y: 10 }).unwrap().red,
            10
        );
    }

    #[test]
    fn keyboard_commands_cover_confirm_cancel_and_physical_nudging() {
        assert_eq!(
            keyboard_command(&Keystroke::parse("enter").unwrap()),
            Some(KeyboardCommand::Copy)
        );
        assert_eq!(
            keyboard_command(&Keystroke::parse("shift-enter").unwrap()),
            Some(KeyboardCommand::QuickSave)
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
        assert_eq!(
            keyboard_command(&Keystroke::parse("ctrl-z").unwrap()),
            Some(KeyboardCommand::Undo)
        );
        assert_eq!(
            keyboard_command(&Keystroke::parse("ctrl-shift-z").unwrap()),
            Some(KeyboardCommand::Redo)
        );
        assert_eq!(
            keyboard_command(&Keystroke::parse("delete").unwrap()),
            Some(KeyboardCommand::Delete)
        );
        assert_eq!(
            keyboard_command(&Keystroke::parse("backspace").unwrap()),
            Some(KeyboardCommand::Delete)
        );
    }

    #[test]
    fn freehand_tool_has_specific_user_feedback() {
        use crate::domain::annotation::AnnotationTool;

        assert_eq!(
            tool_selected_status(AnnotationTool::Freehand),
            "Freehand tool selected"
        );
        assert_eq!(
            drawing_status(AnnotationTool::Freehand),
            "Drawing freehand..."
        );
        assert_eq!(
            annotation_added_status(Some(AnnotationTool::Freehand)),
            "Freehand stroke added"
        );
        assert_eq!(
            annotation_cancelled_status(Some(AnnotationTool::Freehand)),
            "Freehand stroke cancelled"
        );
    }

    #[test]
    fn watermark_tool_has_specific_user_feedback() {
        use crate::domain::annotation::AnnotationTool;

        assert_eq!(
            tool_selected_status(AnnotationTool::Watermark),
            "Watermark tool selected"
        );
        assert_eq!(
            drawing_status(AnnotationTool::Watermark),
            "Placing watermark..."
        );
        assert_eq!(
            annotation_added_status(Some(AnnotationTool::Watermark)),
            "Watermark added"
        );
        assert_eq!(
            annotation_cancelled_status(Some(AnnotationTool::Watermark)),
            "Watermark cancelled"
        );
    }

    #[test]
    fn highlight_tool_has_specific_user_feedback_and_translucent_style() {
        use crate::domain::annotation::{AnnotationStyle, AnnotationTool};

        assert_eq!(
            tool_selected_status(AnnotationTool::Highlight),
            "Highlight tool selected"
        );
        assert_eq!(
            drawing_status(AnnotationTool::Highlight),
            "Drawing highlight..."
        );
        assert_eq!(
            annotation_added_status(Some(AnnotationTool::Highlight)),
            "Highlight added"
        );
        assert_eq!(
            style_for_tool(
                AnnotationTool::Highlight,
                AnnotationStyle {
                    stroke_rgba: 0xFFCC00FF,
                    fill_rgba: Some(0xFFFFFFFF),
                    stroke_width: 10,
                },
            ),
            AnnotationStyle {
                stroke_rgba: 0xFFCC0066,
                fill_rgba: None,
                stroke_width: 1,
            }
        );
    }

    #[test]
    fn mosaic_tool_has_specific_user_feedback() {
        use crate::domain::annotation::AnnotationTool;

        assert_eq!(
            tool_selected_status(AnnotationTool::Mosaic),
            "Mosaic tool selected"
        );
        assert_eq!(drawing_status(AnnotationTool::Mosaic), "Drawing mosaic...");
        assert_eq!(
            annotation_added_status(Some(AnnotationTool::Mosaic)),
            "Mosaic added"
        );
        assert_eq!(
            annotation_cancelled_status(Some(AnnotationTool::Mosaic)),
            "Mosaic cancelled"
        );
    }

    #[test]
    fn blur_tool_has_specific_user_feedback() {
        use crate::domain::annotation::AnnotationTool;

        assert_eq!(
            tool_selected_status(AnnotationTool::Blur),
            "Blur tool selected"
        );
        assert_eq!(drawing_status(AnnotationTool::Blur), "Drawing blur...");
        assert_eq!(
            annotation_added_status(Some(AnnotationTool::Blur)),
            "Blur added"
        );
        assert_eq!(
            annotation_cancelled_status(Some(AnnotationTool::Blur)),
            "Blur cancelled"
        );
    }

    #[test]
    fn fill_color_preserves_rgb_and_uses_transparent_alpha() {
        assert_eq!(fill_color(0xFF3B30FF), 0xFF3B3066);
        assert_eq!(fill_color(0xFF3B3080), 0xFF3B3033);
    }

    #[test]
    fn opacity_preserves_rgb_and_scales_the_shape_fill() {
        assert_eq!(with_alpha(0xFF3B30FF, 128), 0xFF3B3080);
        assert_eq!(fill_alpha(255), 0x66);
        assert_eq!(fill_alpha(128), 0x33);
    }

    #[test]
    fn pinned_window_size_preserves_small_images_and_constrains_large_ones() {
        let small = pinned_size(100.0, 80.0);
        assert_eq!(f32::from(small.width), 180.0);
        assert_eq!(f32::from(small.height), 140.0);

        let large = pinned_size(1_280.0, 720.0);
        assert_eq!(f32::from(large.width), 640.0);
        assert_eq!(f32::from(large.height), 386.0);
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

        let document = AnnotationDocument::new(frame.bounds).unwrap();
        save_annotated_frame_selection(
            &frame,
            &document,
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
    fn annotated_save_and_quick_save_encode_the_composited_selection() {
        let directory = std::env::temp_dir().join(format!(
            "flash-shot-annotated-save-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let path = directory.join("selection.png");
        let frame = CaptureFrame {
            bounds: PhysicalRect {
                left: 0,
                top: 0,
                right: 3,
                bottom: 1,
            },
            width: 3,
            height: 1,
            stride: 12,
            format: PixelFormat::Bgra8,
            pixels: Arc::from([0, 0, 0, 255].repeat(3)),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 1,
        };
        let mut document = AnnotationDocument::new(frame.bounds).unwrap();
        let mut history = CommandHistory::default();
        history
            .apply(
                &mut document,
                AnnotationCommand::Insert(Annotation {
                    id: AnnotationId::new(2),
                    kind: AnnotationKind::Line {
                        start: PhysicalPoint { x: 1, y: 0 },
                        end: PhysicalPoint { x: 2, y: 0 },
                    },
                    style: AnnotationStyle {
                        stroke_rgba: 0x00FF00FF,
                        fill_rgba: None,
                        stroke_width: 1,
                    },
                }),
            )
            .unwrap();
        let selection = PhysicalRect {
            left: 1,
            top: 0,
            right: 3,
            bottom: 1,
        };

        save_annotated_frame_selection(&frame, &document, selection, path.clone()).unwrap();
        let decoder = png::Decoder::new(BufReader::new(std::fs::File::open(&path).unwrap()));
        let mut reader = decoder.read_info().unwrap();
        let mut output = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut output).unwrap();
        assert_eq!((info.width, info.height), (2, 1));
        assert_eq!(
            &output[..info.buffer_size()],
            &[0, 255, 0, 255, 0, 255, 0, 255]
        );

        let quick = quick_save_annotated_frame_selection_in(
            &frame,
            &document,
            selection,
            &directory,
            1_725_000_000_123,
        )
        .unwrap();
        assert_eq!(quick, directory.join("FlashShot-1725000000123.png"));
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
    fn quick_save_names_are_timestamped_and_do_not_overwrite_existing_files() {
        let directory = PathBuf::from("Pictures").join("Flash Shot");
        let timestamp_ms = 1_725_000_000_123_u128;
        let first = super::next_quick_save_path(&directory, timestamp_ms, |_| false);

        assert_eq!(first, directory.join("FlashShot-1725000000123.png"));

        let second = next_quick_save_path(&directory, timestamp_ms, |path| {
            path.file_name()
                .is_some_and(|name| name == "FlashShot-1725000000123.png")
        });
        assert_eq!(second, directory.join("FlashShot-1725000000123-2.png"));
    }

    #[test]
    fn recording_status_uses_ffmpeg_progress_without_exposing_process_output() {
        assert_eq!(
            format_recording_progress(crate::recording::RecordingProgress {
                output_time_us: Some(3_900_000),
                frame: Some(117),
                finished: false,
            }),
            "Recording primary display: 3s, 117 frames"
        );
    }

    #[test]
    fn quick_save_writes_the_selected_png_to_the_default_style_directory() {
        let directory = std::env::temp_dir().join(format!(
            "flash-shot-quick-save-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
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

        let document = AnnotationDocument::new(frame.bounds).unwrap();
        let path = quick_save_annotated_frame_selection_in(
            &frame,
            &document,
            PhysicalRect {
                left: 1,
                top: 0,
                right: 2,
                bottom: 1,
            },
            &directory,
            1_725_000_000_123,
        )
        .unwrap();

        assert_eq!(path, directory.join("FlashShot-1725000000123.png"));
        let decoder = png::Decoder::new(BufReader::new(std::fs::File::open(&path).unwrap()));
        let mut reader = decoder.read_info().unwrap();
        let mut output = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut output).unwrap();
        assert_eq!((info.width, info.height), (1, 1));
        assert_eq!(&output[..info.buffer_size()], &[6, 5, 4, 255]);
        std::fs::remove_dir_all(directory).unwrap();
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
    fn display_window_bounds_convert_physical_pixels_with_monitor_scale() {
        let display = crate::platform::display::DisplayInfo {
            id: "secondary".to_owned(),
            platform_id: 42,
            physical_bounds: PhysicalRect {
                left: -2560,
                top: -200,
                right: 0,
                bottom: 1240,
            },
            work_area: PhysicalRect {
                left: -2560,
                top: -200,
                right: 0,
                bottom: 1200,
            },
            dpi_x: 144,
            dpi_y: 144,
            scale_factor: 1.5,
            rotation: crate::platform::display::DisplayRotation::Landscape,
            bits_per_pixel: 32,
            primary: false,
        };

        let bounds = super::display_window_bounds(&display);

        assert_eq!(f32::from(bounds.origin.x), -2560.0 / 1.5);
        assert_eq!(f32::from(bounds.origin.y), -200.0 / 1.5);
        assert_eq!(f32::from(bounds.size.width), 2560.0 / 1.5);
        assert_eq!(f32::from(bounds.size.height), 1440.0 / 1.5);
    }

    #[test]
    fn overlay_drag_clamps_to_virtual_desktop_edges() {
        let bounds = PhysicalRect {
            left: -1920,
            top: -200,
            right: 2560,
            bottom: 1440,
        };

        assert_eq!(
            super::clamp_physical_point(PhysicalPoint { x: -3000, y: 2000 }, bounds),
            PhysicalPoint { x: -1920, y: 1440 }
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

    #[test]
    fn stale_background_completion_is_ignored_after_a_new_operation_starts() {
        assert!(is_current_operation(4, 4));
        assert!(!is_current_operation(5, 4));
    }
}
