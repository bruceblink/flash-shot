//! Capture, selection, and clipboard workflow orchestration.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use gpui::{
    AppContext, AsyncApp, Bounds, Context, DisplayId, Focusable, KeyDownEvent, Keystroke,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, RenderImage, WeakEntity,
    WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions, point, px, size,
};

use super::{FlashShotApp, overlay::CaptureOverlay, render_image::render_image_from_capture};
use crate::{
    domain::{
        annotation::{AnnotationCommand, AnnotationDocument, AnnotationId, AnnotationTool},
        geometry::PhysicalRect,
        selection::{PreviewTransform, ViewPoint, ViewRect},
        session::CaptureSessionState,
    },
    performance::CapturePipelineSample,
    platform::{
        capture::{CaptureFrame, capture_displays, compose_virtual_desktop},
        clipboard::{ClipboardService, SystemClipboard},
        window_inspector::{InspectionTarget, SystemWindowInspector, WindowInspector},
        window_visibility,
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
                self.next_annotation_id = 1;
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
        self.selected_annotation = None;
        self.preview = None;
        self.selection_drag.clear();
        self.hover_pixel = None;
        self.inspection_target = None;
        self.pending_click_target = None;
        self.inspection_request = None;
        self.status = "Ready - Ctrl+Shift+Print Screen".to_owned();
        self.close_capture_overlays(cx);
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
        self.preview = None;
        self.selection_drag.clear();
        self.hover_pixel = None;
        self.inspection_target = None;
        self.pending_click_target = None;
        self.inspection_request = None;
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
        self.annotation_style.stroke_rgba = color;
        self.status = "Annotation color selected".to_owned();
        cx.notify();
    }

    pub(super) fn select_annotation_width(&mut self, width: u32, cx: &mut Context<Self>) {
        self.annotation_style.stroke_width = width.max(1);
        self.status = format!(
            "Annotation width: {} px",
            self.annotation_style.stroke_width
        );
        cx.notify();
    }

    pub(super) fn select_selection_tool(&mut self, cx: &mut Context<Self>) {
        self.annotation_editor.cancel();
        self.annotation_tool = None;
        self.selected_annotation = None;
        self.status = "Selection tool selected".to_owned();
        cx.notify();
    }

    fn select_annotation_tool(&mut self, tool: AnnotationTool, cx: &mut Context<Self>) {
        self.annotation_editor.cancel();
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
        if self
            .annotation_editor
            .begin(document, id, tool, self.annotation_style, point)
            .is_ok()
        {
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
        match self
            .annotation_editor
            .commit(document, &mut self.annotation_history)
        {
            Ok(true) if moving => self.status = "Annotation moved".to_owned(),
            Ok(true) if resizing => self.status = "Annotation resized".to_owned(),
            Ok(true) => self.status = annotation_added_status(tool).to_owned(),
            Ok(false) if moving => self.status = "Annotation move cancelled".to_owned(),
            Ok(false) if resizing => self.status = "Annotation resize cancelled".to_owned(),
            Ok(false) => self.status = annotation_cancelled_status(tool).to_owned(),
            Err(error) => self.status = error.to_string(),
        }
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
                            Ok(path) => SaveOutcome::Saved(path),
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
                    Ok(path) => SaveOutcome::Saved(path),
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
            SaveOutcome::Saved(path) => {
                if let Err(error) = self.session.export_completed() {
                    self.status = error.to_string();
                } else {
                    self.status = format!("Selection saved to {}", path.display());
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
        AnnotationTool::Rectangle => "Rectangle tool selected",
        AnnotationTool::Ellipse => "Ellipse tool selected",
        AnnotationTool::Line => "Line tool selected",
        AnnotationTool::Arrow => "Arrow tool selected",
        AnnotationTool::Freehand => "Freehand tool selected",
    }
}

fn drawing_status(tool: AnnotationTool) -> &'static str {
    match tool {
        AnnotationTool::Rectangle => "Drawing rectangle...",
        AnnotationTool::Ellipse => "Drawing ellipse...",
        AnnotationTool::Line => "Drawing line...",
        AnnotationTool::Arrow => "Drawing arrow...",
        AnnotationTool::Freehand => "Drawing freehand...",
    }
}

fn annotation_added_status(tool: Option<AnnotationTool>) -> &'static str {
    match tool {
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
    let user_dirs = directories::UserDirs::new().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "user picture directory is unavailable",
        )
    })?;
    let pictures = user_dirs.picture_dir().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "user picture directory is unavailable",
        )
    })?;
    let directory = pictures.join("Flash Shot");
    fs::create_dir_all(&directory)?;
    Ok(directory)
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
    Saved(PathBuf),
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
        copy_annotated_frame_selection, drawing_status, intersect_rect, is_current_operation,
        keyboard_command, next_quick_save_path, png_path, quick_save_annotated_frame_selection_in,
        resolve_pointer_selection, save_annotated_frame_selection, tool_selected_status,
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
