//! Scrolling-screenshot workflow with assisted and manual frame capture.

use super::*;

const AUTO_SCROLL_SETTLE_DELAY: Duration = Duration::from_millis(400);

impl FlashShotApp {
    pub(in crate::app) fn start_manual_scroll(&mut self, cx: &mut Context<Self>) {
        let Some(selection) = self.session.selection() else {
            self.status = "Select an area before starting a scrolling screenshot".to_owned();
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
                self.status = format!("Could not start scrolling screenshot: {error}");
                cx.notify();
                return;
            }
        };
        if self.manual_scroll.state() == crate::scroll::ManualScrollState::Collecting {
            self.status = "A scrolling screenshot is already active".to_owned();
            cx.notify();
            return;
        }
        if self.manual_scroll.state() != crate::scroll::ManualScrollState::Idle {
            let _ = self.manual_scroll.reset();
        }
        if let Err(error) = self.manual_scroll.begin(first) {
            self.status = format!("Could not start scrolling screenshot: {error}");
            cx.notify();
            return;
        }
        self.manual_scroll_selection = Some(selection);
        self.status = "Scrolling screenshot ready. One frame captured.".to_owned();
        self.close_capture_overlays(cx);
        let app = cx.entity();
        cx.defer(move |cx| open_manual_scroll_control(app, cx));
        cx.notify();
    }

    pub(in crate::app) fn capture_manual_scroll_frame(&mut self, cx: &mut Context<Self>) {
        let Some(selection) = self.manual_scroll_selection else {
            self.status = "Scrolling screenshot is not active".to_owned();
            cx.notify();
            return;
        };
        if self.manual_scroll.state() != crate::scroll::ManualScrollState::Collecting {
            self.status = "Scrolling screenshot is not collecting frames".to_owned();
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

    /// Scrolls the selected content once and captures it after a short settle delay.
    ///
    /// The generation token makes a queued capture harmless after the user cancels or starts a
    /// new workflow, so delayed input never appends a frame to the wrong scrolling session.
    pub(in crate::app) fn auto_capture_manual_scroll_frame(&mut self, cx: &mut Context<Self>) {
        let Some(selection) = self.manual_scroll_selection else {
            self.status = "Scrolling screenshot is not active".to_owned();
            cx.notify();
            return;
        };
        if self.manual_scroll.state() != crate::scroll::ManualScrollState::Collecting {
            self.status = "Scrolling screenshot is not collecting frames".to_owned();
            cx.notify();
            return;
        }
        if self.manual_scroll_capture_in_flight
            || self.manual_scroll_auto_capture_generation.is_some()
        {
            self.status = "Scroll frame capture is already in progress".to_owned();
            cx.notify();
            return;
        }
        let target = scroll_target(selection);
        if let Err(error) = crate::platform::scroll::scroll_notches_at(
            target,
            crate::platform::scroll::DEFAULT_SCROLL_NOTCHES,
        ) {
            self.status = format!("Could not assist scroll: {error}");
            cx.notify();
            return;
        }

        let generation = self.operation_generation;
        self.manual_scroll_auto_capture_generation = Some(generation);
        self.status = "Scrolled target content. Capturing when it settles...".to_owned();
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                cx.background_executor()
                    .timer(AUTO_SCROLL_SETTLE_DELAY)
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.finish_auto_scroll_capture(generation, cx)
                    });
                }
            }
        })
        .detach();
    }

    /// Claims only the matching delayed request before beginning the normal capture pipeline.
    fn finish_auto_scroll_capture(&mut self, generation: u64, cx: &mut Context<Self>) {
        if self.manual_scroll_auto_capture_generation != Some(generation) {
            return;
        }
        self.manual_scroll_auto_capture_generation = None;
        if !is_current_operation(self.operation_generation, generation) {
            return;
        }
        self.capture_manual_scroll_frame(cx);
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
                self.abandon_manual_scroll();
                self.close_manual_scroll_window(cx);
                self.return_to_background();
                self.status = format!("Could not finish scrolling screenshot: {error}");
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
            self.history_source = crate::history::HistorySource::Scrolling;
            self.annotation_history = Default::default();
            self.annotation_editor = Default::default();
            self.annotation_tool = None;
            self.text_edit = None;
            self.text_edit_annotation = None;
            self.selected_annotation = None;
            self.selection_drag.select(bounds);
            self.manual_scroll_selection = None;
            self.manual_scroll_capture_in_flight = false;
            self.manual_scroll_auto_capture_generation = None;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.status = format!(
                    "Scrolling screenshot stitched {} frames with {} overlap joins",
                    self.manual_scroll.frame_count(),
                    stitched.overlaps.len()
                );
                self.close_manual_scroll_window(cx);
                let _ = self.manual_scroll.reset();
                let app = cx.entity();
                cx.defer(move |cx| open_image_overlay(app, bounds, cx));
            }
            Err(error) => {
                // A completed session cannot be captured again, so close the controller and
                // return to the tray instead of leaving the user with disabled actions.
                self.abandon_manual_scroll();
                self.close_manual_scroll_window(cx);
                self.return_to_background();
                self.status = format!("Could not open stitched capture: {error}");
            }
        }
        cx.notify();
    }

    pub(in crate::app) fn cancel_manual_scroll(&mut self, cx: &mut Context<Self>) {
        self.abandon_manual_scroll();
        self.close_manual_scroll_window(cx);
        self.status = "Scrolling screenshot cancelled".to_owned();
        self.return_to_background();
        cx.notify();
    }

    pub(in crate::app) fn manual_scroll_control_closed(&mut self, cx: &mut Context<Self>) {
        self.abandon_manual_scroll();
        self.scroll_window = None;
        self.status = "Scrolling screenshot cancelled".to_owned();
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
        self.manual_scroll_auto_capture_generation = None;
    }
}

/// Calculates the physical viewport center used for deliberate wheel input.
fn scroll_target(selection: PhysicalRect) -> PhysicalPoint {
    PhysicalPoint {
        x: selection.left + (selection.width() / 2) as i32,
        y: selection.top + (selection.height() / 2) as i32,
    }
}

#[cfg(test)]
mod tests {
    use super::scroll_target;
    use crate::domain::geometry::PhysicalRect;

    #[test]
    fn scroll_target_uses_the_selected_viewport_center() {
        assert_eq!(
            scroll_target(PhysicalRect {
                left: -100,
                top: 20,
                right: 300,
                bottom: 220,
            }),
            crate::domain::geometry::PhysicalPoint { x: 100, y: 120 }
        );
    }
}
