//! Manual scrolling-capture workflow.

use super::*;

impl FlashShotApp {
    pub(in crate::app) fn start_manual_scroll(&mut self, cx: &mut Context<Self>) {
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

    pub(in crate::app) fn capture_manual_scroll_frame(&mut self, cx: &mut Context<Self>) {
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

    pub(in crate::app) fn assist_manual_scroll(&mut self, cx: &mut Context<Self>) {
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
        let target = crate::domain::geometry::PhysicalPoint {
            x: selection.left + (selection.width() / 2) as i32,
            y: selection.top + (selection.height() / 2) as i32,
        };
        match crate::platform::scroll::scroll_notches_at(
            target,
            crate::platform::scroll::DEFAULT_SCROLL_NOTCHES,
        ) {
            Ok(()) => {
                self.status =
                    "Scrolled target content. Capture the next frame when it settles.".to_owned()
            }
            Err(error) => self.status = format!("Could not assist scroll: {error}"),
        }
        cx.notify();
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
                Err(error) => format!(
                    "That frame did not overlap the previous one: {error}. Adjust the scroll position and capture again."
                ),
            },
            Err(error) => format!("Could not capture scroll frame: {error}"),
        };
        cx.notify();
    }

    pub(in crate::app) fn finish_manual_scroll(&mut self, cx: &mut Context<Self>) {
        if self.manual_scroll_capture_in_flight {
            self.status = "Wait for the current scroll frame capture to finish".to_owned();
            cx.notify();
            return;
        }
        if !self.manual_scroll.can_finish() {
            self.status = "Capture another scroll frame before finishing".to_owned();
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
            self.text_edit_annotation = None;
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

    pub(in crate::app) fn cancel_manual_scroll(&mut self, cx: &mut Context<Self>) {
        self.abandon_manual_scroll();
        self.close_manual_scroll_window(cx);
        self.status = "Manual scroll capture cancelled".to_owned();
        self.return_to_background();
        cx.notify();
    }

    pub(in crate::app) fn manual_scroll_control_closed(&mut self, cx: &mut Context<Self>) {
        self.abandon_manual_scroll();
        self.scroll_window = None;
        self.status = "Manual scroll capture cancelled".to_owned();
        self.return_to_background();
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
}
