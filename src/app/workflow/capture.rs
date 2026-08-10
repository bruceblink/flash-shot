//! Screenshot capture lifecycle and overlay-selection workflows.

use super::*;

impl FlashShotApp {
    pub(in crate::app) fn start_capture(&mut self, cx: &mut Context<Self>) {
        self.start_capture_with_options(self.capture_delay_seconds, false, cx);
    }

    pub(in crate::app) fn start_delayed_capture(
        &mut self,
        delay_seconds: u8,
        cx: &mut Context<Self>,
    ) {
        self.start_capture_with_options(delay_seconds, false, cx);
    }

    pub(in crate::app) fn start_full_screen_capture(&mut self, cx: &mut Context<Self>) {
        self.start_capture_with_options(0, true, cx);
    }

    /// Hides Flash Shot, captures the active external window, and opens it as an editable selection.
    pub(in crate::app) fn start_focused_window_capture(&mut self, cx: &mut Context<Self>) {
        if self.delayed_capture_generation.is_some()
            || self.session.state() != CaptureSessionState::Idle
        {
            return;
        }
        self.start_capture_immediately(false, true, cx);
    }

    pub(in crate::app) fn copy_full_screen(&mut self, cx: &mut Context<Self>) {
        if self.full_screen_copy_generation.is_some()
            || self.full_screen_save_generation.is_some()
            || self.full_screen_pin_generation.is_some()
            || self.clipboard_pin_generation.is_some()
            || self.history_pin_generation.is_some()
            || self.delayed_capture_generation.is_some()
            || self.session.state() != CaptureSessionState::Idle
        {
            return;
        }
        let generation = self.operation_generation;
        self.full_screen_copy_generation = Some(generation);
        self.status = "Capturing full screen for clipboard...".to_owned();
        self.hide_settings_window();
        cx.notify();

        let include_cursor = self.include_cursor;
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let frame = capture_virtual_desktop_frame(include_cursor)?;
                        SystemClipboard.copy_image(&frame)
                    })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.finish_full_screen_copy(result, generation, cx)
                    });
                }
            }
        })
        .detach();
    }

    /// Captures the virtual desktop and saves it through the managed screenshot-history path.
    ///
    /// This has no overlay or selection: it is the tray equivalent of a one-step full-screen save.
    pub(in crate::app) fn quick_save_full_screen(&mut self, cx: &mut Context<Self>) {
        if self.full_screen_copy_generation.is_some()
            || self.full_screen_save_generation.is_some()
            || self.full_screen_pin_generation.is_some()
            || self.clipboard_pin_generation.is_some()
            || self.history_pin_generation.is_some()
            || self.delayed_capture_generation.is_some()
            || self.session.state() != CaptureSessionState::Idle
        {
            return;
        }
        let generation = self.operation_generation;
        self.full_screen_save_generation = Some(generation);
        self.status = "Capturing full screen to save...".to_owned();
        self.hide_settings_window();
        cx.notify();

        let include_cursor = self.include_cursor;
        let directory = self.history.root().to_owned();
        let prefix = self.settings.quick_save_prefix.clone();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let frame = capture_virtual_desktop_frame(include_cursor)?;
                        let fallback = managed_history_fallback(&directory);
                        quick_save_full_screen_frame_with_fallback(
                            &frame,
                            &directory,
                            fallback.as_deref(),
                            &prefix,
                        )
                    })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.finish_full_screen_save(result, generation, cx)
                    });
                }
            }
        })
        .detach();
    }

    /// Captures the virtual desktop into a pinned reference window without using the clipboard.
    pub(in crate::app) fn pin_full_screen(&mut self, cx: &mut Context<Self>) {
        if self.full_screen_copy_generation.is_some()
            || self.full_screen_save_generation.is_some()
            || self.full_screen_pin_generation.is_some()
            || self.clipboard_pin_generation.is_some()
            || self.history_pin_generation.is_some()
            || self.delayed_capture_generation.is_some()
            || self.session.state() != CaptureSessionState::Idle
        {
            return;
        }
        let generation = self.operation_generation;
        self.full_screen_pin_generation = Some(generation);
        self.status = "Capturing full screen to pin...".to_owned();
        self.hide_settings_window();
        cx.notify();

        let include_cursor = self.include_cursor;
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move { capture_virtual_desktop_frame(include_cursor) })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.finish_full_screen_pin(result, generation, cx)
                    });
                }
            }
        })
        .detach();
    }

    fn start_capture_with_options(
        &mut self,
        delay_seconds: u8,
        preselect_full_screen: bool,
        cx: &mut Context<Self>,
    ) {
        if self.full_screen_copy_generation.is_some()
            || self.full_screen_save_generation.is_some()
            || self.full_screen_pin_generation.is_some()
            || self.clipboard_pin_generation.is_some()
            || self.history_pin_generation.is_some()
        {
            return;
        }
        if self.delayed_capture_generation.is_some() {
            self.cancel_delayed_capture(cx);
            return;
        }
        if self.session.state() != CaptureSessionState::Idle {
            return;
        }
        if delay_seconds == 0 {
            self.start_capture_immediately(preselect_full_screen, false, cx);
            return;
        }
        self.operation_generation = self.operation_generation.wrapping_add(1);
        let generation = self.operation_generation;
        self.delayed_capture_generation = Some(generation);
        self.delayed_capture_remaining_seconds = Some(delay_seconds);
        self.status = delayed_capture_status(delay_seconds);
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                for remaining in (0..delay_seconds).rev() {
                    cx.background_executor().timer(Duration::from_secs(1)).await;
                    let Some(this) = this.upgrade() else {
                        break;
                    };
                    let started = this.update(&mut cx, |this, cx| {
                        this.advance_delayed_capture(
                            generation,
                            remaining,
                            preselect_full_screen,
                            cx,
                        )
                    });
                    if started {
                        break;
                    }
                }
            }
        })
        .detach();
    }

    pub(in crate::app) fn cancel_delayed_capture(&mut self, cx: &mut Context<Self>) {
        if self.delayed_capture_generation.take().is_none() {
            return;
        }
        self.delayed_capture_remaining_seconds = None;
        self.operation_generation = self.operation_generation.wrapping_add(1);
        self.status = "Delayed capture cancelled".to_owned();
        cx.notify();
    }

    fn advance_delayed_capture(
        &mut self,
        generation: u64,
        remaining_seconds: u8,
        preselect_full_screen: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.delayed_capture_generation != Some(generation)
            || !is_current_operation(self.operation_generation, generation)
        {
            return true;
        }
        if remaining_seconds > 0 {
            self.delayed_capture_remaining_seconds = Some(remaining_seconds);
            self.status = delayed_capture_status(remaining_seconds);
            cx.notify();
            return false;
        }
        self.delayed_capture_generation = None;
        self.delayed_capture_remaining_seconds = None;
        self.start_capture_immediately(preselect_full_screen, false, cx);
        true
    }

    fn start_capture_immediately(
        &mut self,
        preselect_full_screen: bool,
        preselect_focused_window: bool,
        cx: &mut Context<Self>,
    ) {
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
        self.history_source = crate::history::HistorySource::Selection;
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
        self.manual_scroll_auto_capture_generation = None;
        self.recognition_result = None;
        self.recognition_retry = None;
        self.recognition_in_flight = false;
        self.overlay_more_actions = false;
        self.overlay_annotation_controls = false;
        self.status = "Capturing virtual desktop...".to_owned();
        self.hide_settings_window();
        cx.notify();

        let started_at = Instant::now();
        let include_cursor = self.include_cursor;
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                // Let the hidden settings popup return foreground ownership before reading it.
                if preselect_focused_window {
                    cx.background_executor()
                        .timer(Duration::from_millis(120))
                        .await;
                }
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let focused_window = if preselect_focused_window {
                            SystemWindowInspector
                                .focused_window_target()?
                                .map(|target| target.bounds)
                        } else {
                            None
                        };
                        capture_virtual_desktop_preview(include_cursor)
                            .map(|capture| (capture, focused_window))
                    })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.finish_capture(
                            result,
                            started_at,
                            generation,
                            preselect_full_screen,
                            preselect_focused_window,
                            cx,
                        )
                    });
                }
            }
        })
        .detach();
    }

    fn finish_capture(
        &mut self,
        result: std::io::Result<(CapturedDesktopPreview, Option<PhysicalRect>)>,
        started_at: Instant,
        generation: u64,
        preselect_full_screen: bool,
        preselect_focused_window: bool,
        cx: &mut Context<Self>,
    ) {
        if !is_current_operation(self.operation_generation, generation) {
            return;
        }
        if self.session.state() != CaptureSessionState::Capturing {
            return;
        }
        match result {
            Ok((capture, focused_window)) => {
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
                    render_upload_copy_count: capture.render_upload_copy_count,
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
                            self.return_to_background();
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
                self.text_edit_annotation = None;
                self.next_annotation_id = 1;
                self.next_sequence_number = 1;
                let frame_bounds = capture.capture.frame.bounds;
                self.frame = Some(capture.capture.frame);
                if preselect_full_screen {
                    if let Err(error) = self.session.select(frame_bounds) {
                        self.status = error.to_string();
                        cx.notify();
                        return;
                    }
                    self.selection_drag.select(frame_bounds);
                } else if preselect_focused_window {
                    let Some(selection) = focused_window_selection(focused_window, frame_bounds)
                    else {
                        let message =
                            "Could not find a focused window outside Flash Shot".to_owned();
                        let _ = self.session.fail(message.clone());
                        self.status = message;
                        self.return_to_background();
                        cx.notify();
                        return;
                    };
                    if let Err(error) = self.session.select(selection) {
                        self.status = error.to_string();
                        cx.notify();
                        return;
                    }
                    self.selection_drag.select(selection);
                    self.status = format!(
                        "Focused window: {} x {} physical pixels",
                        selection.width(),
                        selection.height()
                    );
                }
                let app = cx.entity();
                cx.defer(move |cx| open_capture_overlays(app, capture.displays, pipeline, cx));
            }
            Err(error) => {
                let message = format!("Capture failed: {error}");
                let _ = self.session.fail(message.clone());
                self.status = message;
                log::warn!(target: "flash_shot::capture", "capture_failed error={error}");
                self.return_to_background();
            }
        }
        cx.notify();
    }

    pub(in crate::app) fn reset(&mut self, cx: &mut Context<Self>) {
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
        self.delayed_capture_generation = None;
        self.delayed_capture_remaining_seconds = None;
        self.full_screen_copy_generation = None;
        self.full_screen_save_generation = None;
        self.full_screen_pin_generation = None;
        self.clipboard_pin_generation = None;
        self.history_pin_generation = None;
        self.history_source = crate::history::HistorySource::Selection;
        self.frame = None;
        self.annotation_document = None;
        self.annotation_history = Default::default();
        self.annotation_editor = Default::default();
        self.annotation_tool = None;
        self.text_edit = None;
        self.text_edit_annotation = None;
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
        self.manual_scroll_auto_capture_generation = None;
        self.recognition_result = None;
        self.recognition_retry = None;
        self.overlay_more_actions = false;
        self.overlay_annotation_controls = false;
        self.status = if self.capture_shortcut_enabled {
            format!("Ready - {}", self.capture_shortcut)
        } else {
            "Ready - global shortcut disabled".to_owned()
        };
        self.close_capture_overlays(cx);
        self.close_manual_scroll_window(cx);
        self.recording_audio_discovery_in_flight = false;
        self.recording_display_discovery_in_flight = false;
        self.return_to_background();
        cx.notify();
    }

    pub(in crate::app) fn shutdown(&mut self, _cx: &mut Context<Self>) {
        self.operation_generation = self.operation_generation.wrapping_add(1);
        self.delayed_capture_generation = None;
        self.delayed_capture_remaining_seconds = None;
        if self.session.state() != CaptureSessionState::Idle {
            let _ = self.session.cancel();
        }
        self.frame = None;
        self.annotation_document = None;
        self.annotation_history = Default::default();
        self.annotation_editor = Default::default();
        self.annotation_tool = None;
        self.text_edit = None;
        self.text_edit_annotation = None;
        self.preview = None;
        self.selection_drag.clear();
        self.hover_pixel = None;
        self.inspection_target = None;
        self.pending_click_target = None;
        self.inspection_request = None;
        self.manual_scroll = Default::default();
        self.manual_scroll_selection = None;
        self.manual_scroll_capture_in_flight = false;
        self.manual_scroll_auto_capture_generation = None;
        self.recognition_result = None;
        self.recognition_retry = None;
        self.recognition_in_flight = false;
        self.recording_control = None;
        self.recording_start_in_flight = false;
        self.recording_stopping = false;
        self.recording_paused = false;
        self.recording_audio_discovery_in_flight = false;
        self.recording_display_discovery_in_flight = false;
        // GPUI has already removed native windows before invoking on_app_quit.
        // Keeping the handles untouched avoids issuing late operations on closed HWNDs.
        log::info!(target: "flash_shot::lifecycle", "capture_workflow_shutdown");
    }

    pub(in crate::app) fn begin_overlay_selection(
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
            self.pending_click_target = None;
            self.selection_drag.begin_resize(selection, handle);
        } else if let Some(selection) = self
            .selection_drag
            .selection()
            .filter(|selection| selection.contains(point))
        {
            self.pending_click_target = None;
            self.selection_drag.begin_move(selection, point);
            self.status = "Moving selection...".to_owned();
        } else {
            self.selection_drag.begin(point);
        }
    }

    pub(in crate::app) fn update_overlay_selection(
        &mut self,
        point: crate::domain::geometry::PhysicalPoint,
        preserve_aspect_ratio: bool,
        resize_from_center: bool,
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
        if self.selection_drag.is_moving() {
            self.selection_drag.update_move(point, frame.bounds);
            self.status = "Moving selection...".to_owned();
        } else {
            let point = clamp_physical_point(point, frame.bounds);
            if resize_from_center {
                self.selection_drag
                    .update_from_center(point, frame.bounds, preserve_aspect_ratio);
            } else if preserve_aspect_ratio {
                self.selection_drag
                    .update_with_aspect_ratio(point, frame.bounds);
            } else {
                self.selection_drag.update(point);
            }
        }
        if let Some(selection) = self.selection_drag.selection()
            && !self.selection_drag.is_moving()
        {
            self.status = selection_status(selection);
        }
        cx.notify();
    }

    pub(in crate::app) fn update_overlay_hover(
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

    pub(in crate::app) fn finish_overlay_selection(
        &mut self,
        point: crate::domain::geometry::PhysicalPoint,
        preserve_aspect_ratio: bool,
        resize_from_center: bool,
        copy_on_double_click: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(frame) = self.frame.as_ref() else {
            return;
        };
        if self.text_edit.is_some() {
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
        if self.selection_drag.is_moving() {
            self.selection_drag.update_move(point, frame.bounds);
        } else {
            let point = clamp_physical_point(point, frame.bounds);
            if resize_from_center {
                self.selection_drag
                    .update_from_center(point, frame.bounds, preserve_aspect_ratio);
            } else if preserve_aspect_ratio {
                self.selection_drag
                    .update_with_aspect_ratio(point, frame.bounds);
            } else {
                self.selection_drag.update(point);
            }
        }
        let selection = self
            .selection_drag
            .selection()
            .and_then(|selection| resolve_pointer_selection(selection, self.pending_click_target));
        self.pending_click_target = None;
        if let Some(selection) = selection {
            self.selection_drag.select(selection);
            if let Err(error) = self.session.select(selection) {
                self.status = error.to_string();
                cx.notify();
                return;
            }
            self.status = selection_status(selection);
            if copy_on_double_click {
                self.copy_selection(cx);
                return;
            }
        }
        cx.notify();
    }
}

/// Clips the native foreground-window rectangle to the captured virtual desktop.
pub(super) fn focused_window_selection(
    focused_window: Option<PhysicalRect>,
    frame_bounds: PhysicalRect,
) -> Option<PhysicalRect> {
    focused_window.and_then(|target| intersect_rect(target, frame_bounds))
}
