//! Image, project, and screenshot-history workflows.

use std::collections::{HashSet, VecDeque};

use super::*;

pub(super) const HISTORY_THUMBNAIL_MAX_IN_FLIGHT: usize = 2;

/// Removes queued thumbnail work for files that are no longer visible or retained.
pub(in crate::app) fn retain_history_thumbnail_pending(
    pending: &mut VecDeque<PathBuf>,
    retained: &HashSet<PathBuf>,
) {
    pending.retain(|path| retained.contains(path));
}

impl FlashShotApp {
    pub(in crate::app) fn open_image(&mut self, cx: &mut Context<Self>) {
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
                            .spawn(async move { open_image_project(&path) })
                            .await
                        {
                            Ok((path, frame, document, document_warning)) => {
                                OpenImageOutcome::Opened {
                                    path,
                                    frame,
                                    document,
                                    document_warning,
                                }
                            }
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

    pub(in crate::app) fn open_editable_project(&mut self, cx: &mut Context<Self>) {
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
        self.status = "Choose an editable annotation project...".to_owned();
        cx.notify();
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open annotation project".into()),
        });
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let outcome = match prompt.await {
                    Ok(Ok(Some(mut paths))) => match paths.pop() {
                        Some(path) => match cx
                            .background_executor()
                            .spawn(async move { open_annotation_project(&path) })
                            .await
                        {
                            Ok((path, frame, document)) => OpenImageOutcome::Opened {
                                path,
                                frame,
                                document: Some(document),
                                document_warning: None,
                            },
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

    pub(in crate::app) fn open_history_image(&mut self, path: PathBuf, cx: &mut Context<Self>) {
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
        self.status = format!("Opening {}...", path.display());
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let outcome = match cx
                    .background_executor()
                    .spawn(async move { open_image_project(&path) })
                    .await
                {
                    Ok((path, frame, document, document_warning)) => OpenImageOutcome::Opened {
                        path,
                        frame,
                        document,
                        document_warning,
                    },
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

    /// Decodes and copies a retained PNG without opening the annotation workflow.
    pub(in crate::app) fn copy_history_image(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.session.state() != CaptureSessionState::Idle {
            return;
        }
        self.operation_generation = self.operation_generation.wrapping_add(1);
        let generation = self.operation_generation;
        self.status = format!("Copying {}...", path.display());
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let frame = CaptureFrame::open_png(&path)?;
                        SystemClipboard.copy_image(&frame)
                    })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.finish_history_copy(result, generation, cx)
                    });
                }
            }
        })
        .detach();
    }

    /// Decodes a retained screenshot in the background before opening it as an always-on-top pin.
    pub(in crate::app) fn pin_history_image(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.history_pin_generation.is_some()
            || self.session.state() != CaptureSessionState::Idle
        {
            return;
        }
        self.operation_generation = self.operation_generation.wrapping_add(1);
        let generation = self.operation_generation;
        self.history_pin_generation = Some(generation);
        self.status = format!("Pinning {}...", path.display());
        self.hide_settings_window();
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        CaptureFrame::open_png(&path).and_then(super::pinning::prepare_pinned_frame)
                    })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.finish_history_pin(result, generation, cx)
                    });
                }
            }
        })
        .detach();
    }

    /// Opens only the most recently requested retained image and ignores stale decode results.
    fn finish_history_pin(
        &mut self,
        result: std::io::Result<super::pinning::PreparedPinnedFrame>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if !claim_idle_completion(
            &mut self.history_pin_generation,
            self.operation_generation,
            generation,
            self.session.state(),
        ) {
            return;
        }
        match result {
            Ok(prepared) => self.open_prepared_pinned_frame(
                prepared,
                "History image pinned in an always-on-top window",
                Some("Could not pin history image"),
                false,
                cx,
            ),
            Err(error) => {
                self.status = format!("Could not pin history image: {error}");
                log::warn!(target: "flash_shot::pinned", "history_pin_failed error={error}");
                self.notify_user("Flash Shot", "Could not pin history image");
                cx.notify();
            }
        }
    }

    /// Returns a cached history preview and starts one background decode when it is first needed.
    pub(in crate::app) fn history_thumbnail(
        &mut self,
        path: &PathBuf,
        cx: &mut Context<Self>,
    ) -> Option<Arc<RenderImage>> {
        if let Some(thumbnail) = self.history_thumbnails.get(path) {
            return Some(thumbnail.clone());
        }
        if self.history_thumbnail_failed.contains(path) {
            return None;
        }
        if !enqueue_history_thumbnail_path(
            path.clone(),
            &mut self.history_thumbnail_pending,
            &self.history_thumbnail_loading,
        ) {
            return None;
        }
        self.pump_history_thumbnail_queue(cx);
        None
    }

    /// Starts at most two PNG decodes at once so expanding a long history cannot flood the
    /// background executor or compete with a user-initiated capture/export.
    fn pump_history_thumbnail_queue(&mut self, cx: &mut Context<Self>) {
        while let Some(path) = take_next_history_thumbnail(
            &mut self.history_thumbnail_pending,
            &mut self.history_thumbnail_loading,
            HISTORY_THUMBNAIL_MAX_IN_FLIGHT,
        ) {
            let this_path = path.clone();
            cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                let mut cx = cx.clone();
                async move {
                    let result = cx
                        .background_executor()
                        .spawn({
                            let decode_path = this_path.clone();
                            async move {
                                let frame = CaptureFrame::open_png(&decode_path)?;
                                history_thumbnail_frame(&frame)
                            }
                        })
                        .await;
                    if let Some(this) = this.upgrade() {
                        this.update(&mut cx, |this, cx| {
                            this.finish_history_thumbnail(this_path, result, cx)
                        });
                    }
                }
            })
            .detach();
        }
    }

    /// Stores a successfully decoded preview without surfacing transient list-rendering errors.
    fn finish_history_thumbnail(
        &mut self,
        path: PathBuf,
        result: std::io::Result<CaptureFrame>,
        cx: &mut Context<Self>,
    ) {
        self.history_thumbnail_loading.remove(&path);
        let still_retained = self
            .history
            .entries()
            .iter()
            .any(|entry| entry.path == path);
        if still_retained {
            match result.and_then(|frame| render_image_from_capture(&frame)) {
                Ok(thumbnail) => {
                    self.history_thumbnail_failed.remove(&path);
                    self.history_thumbnails.insert(path, thumbnail.image);
                }
                Err(error) => {
                    log::warn!(target: "flash_shot::history", "history_thumbnail_failed path={} error={error}", path.display());
                    self.history_thumbnail_failed.insert(path);
                }
            }
        }
        self.pump_history_thumbnail_queue(cx);
        cx.notify();
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
            OpenImageOutcome::Opened {
                path,
                frame,
                document,
                document_warning,
            } => {
                let bounds = frame.bounds;
                let result = (|| -> std::io::Result<()> {
                    self.session.frames_ready().map_err(std::io::Error::other)?;
                    let preview = render_image_from_capture(&frame)?;
                    let document = document
                        .unwrap_or(AnnotationDocument::new(bounds).map_err(std::io::Error::other)?);
                    let (next_annotation_id, next_sequence_number) =
                        next_annotation_counters(&document);
                    self.session.select(bounds).map_err(std::io::Error::other)?;
                    self.history_source = crate::history::HistorySource::Selection;
                    self.preview = Some(preview.image);
                    self.frame = Some(frame);
                    self.annotation_document = Some(document);
                    self.annotation_history = Default::default();
                    self.annotation_editor = Default::default();
                    self.annotation_tool = None;
                    self.text_edit = None;
                    self.text_edit_annotation = None;
                    self.selected_annotation = None;
                    self.next_annotation_id = next_annotation_id;
                    self.next_sequence_number = next_sequence_number;
                    self.selection_drag.select(bounds);
                    Ok(())
                })();
                match result {
                    Ok(()) => {
                        self.status = match document_warning {
                            Some(warning) => {
                                format!("Opened {} without annotations: {warning}", path.display())
                            }
                            None => format!("Opened {} for annotation", path.display()),
                        };
                        if let Some(handle) = self.settings_window_handle
                            && let Err(error) = window_visibility::hide(handle)
                        {
                            let message = format!(
                                "Could not hide settings before opening the editor: {error}"
                            );
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

    fn finish_history_copy(
        &mut self,
        result: std::io::Result<()>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if !is_current_operation(self.operation_generation, generation)
            || self.session.state() != CaptureSessionState::Idle
        {
            return;
        }
        self.status = match result {
            Ok(()) => "History image copied to clipboard".to_owned(),
            Err(error) => format!("Could not copy history image: {error}"),
        };
        cx.notify();
    }
}

/// Adds a thumbnail request once, preserving FIFO order across repeated UI renders.
fn enqueue_history_thumbnail_path(
    path: PathBuf,
    pending: &mut VecDeque<PathBuf>,
    loading: &HashSet<PathBuf>,
) -> bool {
    if loading.contains(&path) || pending.contains(&path) {
        return false;
    }
    pending.push_back(path);
    true
}

/// Claims the next pending request only while the fixed decode budget has room.
fn take_next_history_thumbnail(
    pending: &mut VecDeque<PathBuf>,
    loading: &mut HashSet<PathBuf>,
    max_in_flight: usize,
) -> Option<PathBuf> {
    if loading.len() >= max_in_flight {
        return None;
    }
    let path = pending.pop_front()?;
    loading.insert(path.clone());
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::{
        HISTORY_THUMBNAIL_MAX_IN_FLIGHT, enqueue_history_thumbnail_path,
        retain_history_thumbnail_pending, take_next_history_thumbnail,
    };
    use std::{
        collections::{HashSet, VecDeque},
        path::PathBuf,
    };

    #[test]
    fn thumbnail_queue_deduplicates_pending_and_loading_paths() {
        let path = PathBuf::from("first.png");
        let mut pending = VecDeque::new();
        let mut loading = HashSet::new();

        assert!(enqueue_history_thumbnail_path(
            path.clone(),
            &mut pending,
            &loading
        ));
        assert!(!enqueue_history_thumbnail_path(
            path.clone(),
            &mut pending,
            &loading
        ));
        let claimed = take_next_history_thumbnail(&mut pending, &mut loading, 2);
        assert_eq!(claimed, Some(path.clone()));
        assert!(!enqueue_history_thumbnail_path(
            path,
            &mut pending,
            &loading
        ));
    }

    #[test]
    fn thumbnail_queue_preserves_fifo_and_caps_in_flight_work() {
        let mut pending = VecDeque::from([
            PathBuf::from("first.png"),
            PathBuf::from("second.png"),
            PathBuf::from("third.png"),
        ]);
        let mut loading = HashSet::new();

        assert_eq!(
            take_next_history_thumbnail(&mut pending, &mut loading, 2),
            Some(PathBuf::from("first.png"))
        );
        assert_eq!(
            take_next_history_thumbnail(&mut pending, &mut loading, 2),
            Some(PathBuf::from("second.png"))
        );
        assert_eq!(
            take_next_history_thumbnail(&mut pending, &mut loading, 2),
            None
        );
        loading.remove(&PathBuf::from("first.png"));
        assert_eq!(
            take_next_history_thumbnail(&mut pending, &mut loading, 2),
            Some(PathBuf::from("third.png"))
        );
    }

    #[test]
    fn thumbnail_queue_drops_paths_removed_from_history() {
        let removed = PathBuf::from("removed.png");
        let kept = PathBuf::from("kept.png");
        let mut pending = VecDeque::from([removed, kept.clone()]);
        let retained = HashSet::from([kept.clone()]);

        retain_history_thumbnail_pending(&mut pending, &retained);

        assert_eq!(pending.into_iter().collect::<Vec<_>>(), vec![kept]);
    }

    #[test]
    fn thumbnail_queue_keeps_300_history_requests_bounded() {
        let mut pending = VecDeque::new();
        let mut loading = HashSet::new();
        for index in 0..300 {
            assert!(enqueue_history_thumbnail_path(
                PathBuf::from(format!("capture-{index:03}.png")),
                &mut pending,
                &loading,
            ));
        }

        let first = take_next_history_thumbnail(
            &mut pending,
            &mut loading,
            HISTORY_THUMBNAIL_MAX_IN_FLIGHT,
        );
        let second = take_next_history_thumbnail(
            &mut pending,
            &mut loading,
            HISTORY_THUMBNAIL_MAX_IN_FLIGHT,
        );

        assert_eq!(first, Some(PathBuf::from("capture-000.png")));
        assert_eq!(second, Some(PathBuf::from("capture-001.png")));
        assert_eq!(loading.len(), HISTORY_THUMBNAIL_MAX_IN_FLIGHT);
        assert_eq!(pending.len(), 300 - HISTORY_THUMBNAIL_MAX_IN_FLIGHT);
        assert_eq!(
            take_next_history_thumbnail(
                &mut pending,
                &mut loading,
                HISTORY_THUMBNAIL_MAX_IN_FLIGHT,
            ),
            None,
        );

        loading.remove(&PathBuf::from("capture-000.png"));
        assert_eq!(
            take_next_history_thumbnail(
                &mut pending,
                &mut loading,
                HISTORY_THUMBNAIL_MAX_IN_FLIGHT,
            ),
            Some(PathBuf::from("capture-002.png")),
        );
        assert_eq!(loading.len(), HISTORY_THUMBNAIL_MAX_IN_FLIGHT);
    }
}
