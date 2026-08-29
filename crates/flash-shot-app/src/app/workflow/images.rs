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
    /// Returns whether a destructive history request has reserved the file set.
    ///
    /// Confirmation counts as reserved: letting a reader begin after the user confirms a snapshot
    /// would reintroduce the read/delete race when the second confirmation arrives.
    pub(in crate::app) fn history_mutation_pending(&self) -> bool {
        self.history_clear_confirmation
            || self.history_clear_in_flight
            || self.history_retention_target.is_some()
            || !self.history_deletions_in_flight.is_empty()
            || self.history_write_generation.is_some()
            || self.history_root_change_in_flight
    }

    /// Returns whether a managed PNG reader or a generic file-picker reservation is active.
    ///
    /// Thumbnail work is intentionally low priority, but it is still a real file reader and a
    /// delete must wait for its bounded decode to finish rather than racing Windows file sharing.
    pub(in crate::app) fn history_file_read_in_flight(&self) -> bool {
        self.history_reader.is_some()
            || self.generic_open_generation.is_some()
            || !self.history_thumbnail_loading.is_empty()
    }

    /// Allows a destructive history mutation only after every retained-file reader has finished.
    pub(in crate::app) fn history_mutation_can_start(&self) -> bool {
        self.session.state() == CaptureSessionState::Idle
            && history_mutation_can_start(
                self.history_reader.is_some(),
                self.generic_open_generation.is_some(),
                !self.history_thumbnail_loading.is_empty(),
                self.history_mutation_pending(),
            )
    }

    /// Allows the confirmation click to turn its reserved snapshot into a deletion task.
    pub(super) fn history_clear_can_commit(&self) -> bool {
        self.history_clear_confirmation
            && !self.history_file_read_in_flight()
            && self.history_write_generation.is_none()
            && !self.history_root_change_in_flight
            && !self.history_clear_in_flight
            && self.history_retention_target.is_none()
            && self.history_deletions_in_flight.is_empty()
    }

    /// Starts one user-visible history read only when its file cannot be deleted or replaced.
    fn can_start_history_reader(&self, path: &PathBuf) -> bool {
        history_reader_can_start(
            self.history_reader.is_some(),
            !self.capture_export_operations_idle() || self.delayed_capture_generation.is_some(),
            self.history_mutation_pending(),
            self.history
                .entries()
                .iter()
                .any(|entry| entry.path == *path),
            self.session.state(),
        )
    }

    /// Reserves the current history file until the matching background completion releases it.
    fn begin_history_reader(&mut self, kind: HistoryReaderKind, path: PathBuf) -> u64 {
        self.operation_generation = self.operation_generation.wrapping_add(1);
        let generation = self.operation_generation;
        self.history_reader = Some(HistoryReaderLease {
            kind,
            generation,
            path,
        });
        generation
    }

    /// Releases exactly the matching reader lease and accepts only a current completion.
    fn finish_history_reader(&mut self, kind: HistoryReaderKind, generation: u64) -> bool {
        let Some(lease) =
            claim_history_reader_completion(&mut self.history_reader, kind, generation)
        else {
            return false;
        };
        log::debug!(
            target: "flash_shot::history",
            "history_reader_finished kind={kind:?} path={}",
            lease.path.display()
        );
        is_current_operation(self.operation_generation, generation)
    }

    /// Invalidates queued or completed thumbnail work before a history file set changes.
    pub(in crate::app) fn invalidate_history_thumbnails(&mut self) {
        self.history_thumbnail_revision = self.history_thumbnail_revision.wrapping_add(1);
        self.history_thumbnail_pending.clear();
    }

    /// Resumes bounded preview decoding after a write lease releases and the current view asks
    /// for missing thumbnails again. Keeping the scheduler separate avoids starting new reads
    /// while the managed save may still prune an older PNG.
    pub(super) fn resume_history_thumbnail_queue(&mut self, cx: &mut Context<Self>) {
        self.pump_history_thumbnail_queue(cx);
    }

    /// Reserves one managed history write so a possible retention prune cannot race a reader.
    pub(super) fn begin_history_write(&mut self) -> Option<u64> {
        if self.history_write_generation.is_some()
            || self.history_root_change_in_flight
            || self.history_file_read_in_flight()
            || self.history_clear_confirmation
            || self.history_clear_in_flight
            || self.history_retention_target.is_some()
            || !self.history_deletions_in_flight.is_empty()
        {
            return None;
        }
        self.history_write_sequence = self.history_write_sequence.wrapping_add(1);
        let generation = self.history_write_sequence;
        self.history_write_generation = Some(generation);
        self.invalidate_history_thumbnails();
        Some(generation)
    }

    /// Releases only the matching managed-save reservation after its history update completes.
    pub(super) fn finish_history_write(&mut self, generation: u64) -> bool {
        claim_history_write_completion(&mut self.history_write_generation, generation)
    }

    /// Reserves a generic PNG or project picker before it can select a managed history file.
    ///
    /// The selected path is unknown while the native picker is visible. Holding this lease keeps a
    /// concurrent save, prune, or root change from deleting a history image between selection and
    /// the async result returns to the UI, even when the picker ultimately chooses an external
    /// file.
    fn begin_generic_open_request(
        &mut self,
    ) -> Result<Option<u64>, crate::domain::session::TransitionError> {
        if self.generic_open_generation.is_some()
            || self.history_reader.is_some()
            || self.history_mutation_pending()
            || self.session.state() != CaptureSessionState::Idle
        {
            return Ok(None);
        }
        self.session.begin()?;
        self.operation_generation = self.operation_generation.wrapping_add(1);
        let generation = self.operation_generation;
        self.generic_open_generation = Some(generation);
        Ok(Some(generation))
    }

    /// Releases only the matching picker reservation and rejects a reset or newer flow.
    fn finish_generic_open_request(&mut self, generation: u64) -> bool {
        claim_generic_open_completion(
            &mut self.generic_open_generation,
            self.operation_generation,
            generation,
        )
    }

    pub(in crate::app) fn open_image(&mut self, cx: &mut Context<Self>) {
        let generation = match self.begin_generic_open_request() {
            Ok(Some(generation)) => generation,
            Ok(None) => return,
            Err(error) => {
                let error_detail = error.to_string();
                self.status = self.settings.locale.format_template(
                    crate::i18n::UiText::OpenImageFailed,
                    &[("error", &error_detail)],
                );
                cx.notify();
                return;
            }
        };
        let locale = self.settings.locale;
        self.status = locale
            .text(crate::i18n::UiText::OpenImageChoosing)
            .to_owned();
        cx.notify();

        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(locale.text(crate::i18n::UiText::OpenImagePrompt).into()),
        });
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let outcome = match prompt.await {
                    Ok(Ok(Some(mut paths))) => match paths.pop() {
                        Some(path) => match cx
                            .background_executor()
                            .spawn(async move {
                                let (path, frame, document, document_warning) =
                                    open_image_project(&path)?;
                                let preview = render_image_from_capture(&frame)?.image;
                                Ok::<_, std::io::Error>((
                                    path,
                                    frame,
                                    preview,
                                    document,
                                    document_warning,
                                ))
                            })
                            .await
                        {
                            Ok((path, frame, preview, document, document_warning)) => {
                                OpenImageOutcome::Opened {
                                    path,
                                    frame,
                                    preview,
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
                        if this.finish_generic_open_request(generation) {
                            this.finish_open_image(outcome, generation, cx);
                        } else {
                            cx.notify();
                        }
                    });
                }
            }
        })
        .detach();
    }

    pub(in crate::app) fn open_editable_project(&mut self, cx: &mut Context<Self>) {
        let generation = match self.begin_generic_open_request() {
            Ok(Some(generation)) => generation,
            Ok(None) => return,
            Err(error) => {
                let error_detail = error.to_string();
                self.status = self.settings.locale.format_template(
                    crate::i18n::UiText::OpenImageFailed,
                    &[("error", &error_detail)],
                );
                cx.notify();
                return;
            }
        };
        let locale = self.settings.locale;
        self.status = locale
            .text(crate::i18n::UiText::OpenProjectChoosing)
            .to_owned();
        cx.notify();
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(locale.text(crate::i18n::UiText::OpenProjectPrompt).into()),
        });
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let outcome = match prompt.await {
                    Ok(Ok(Some(mut paths))) => match paths.pop() {
                        Some(path) => match cx
                            .background_executor()
                            .spawn(async move {
                                let (path, frame, document) = open_annotation_project(&path)?;
                                let preview = render_image_from_capture(&frame)?.image;
                                Ok::<_, std::io::Error>((path, frame, preview, document))
                            })
                            .await
                        {
                            Ok((path, frame, preview, document)) => OpenImageOutcome::Opened {
                                path,
                                frame,
                                preview,
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
                        if this.finish_generic_open_request(generation) {
                            this.finish_open_image(outcome, generation, cx);
                        } else {
                            cx.notify();
                        }
                    });
                }
            }
        })
        .detach();
    }

    pub(in crate::app) fn open_history_image(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !self.can_start_history_reader(&path) {
            return;
        }
        if let Err(error) = self.session.begin() {
            let error_detail = error.to_string();
            self.status = self.settings.locale.format_template(
                crate::i18n::UiText::OpenImageFailed,
                &[("error", &error_detail)],
            );
            cx.notify();
            return;
        }
        let generation = self.begin_history_reader(HistoryReaderKind::Open, path.clone());
        let path_detail = path.display().to_string();
        self.status = self.settings.locale.format_template(
            crate::i18n::UiText::OpenHistoryInProgress,
            &[("path", &path_detail)],
        );
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let outcome = match cx
                    .background_executor()
                    .spawn(async move {
                        let (path, frame, document, document_warning) = open_image_project(&path)?;
                        let preview = render_image_from_capture(&frame)?.image;
                        Ok::<_, std::io::Error>((path, frame, preview, document, document_warning))
                    })
                    .await
                {
                    Ok((path, frame, preview, document, document_warning)) => {
                        OpenImageOutcome::Opened {
                            path,
                            frame,
                            preview,
                            document,
                            document_warning,
                        }
                    }
                    Err(error) => OpenImageOutcome::Failed(error.to_string()),
                };
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.finish_history_open_image(outcome, generation, cx)
                    });
                }
            }
        })
        .detach();
    }

    /// Decodes and copies a retained PNG without opening the annotation workflow.
    pub(in crate::app) fn copy_history_image(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !self.can_start_history_reader(&path) {
            return;
        }
        let locale = self.settings.locale;
        let Some(clipboard_write_id) =
            self.try_begin_clipboard_write(crate::i18n::UiText::ClipboardActionHistoryImage, cx)
        else {
            return;
        };
        let generation = self.begin_history_reader(HistoryReaderKind::Copy, path.clone());
        let path_detail = path.display().to_string();
        self.status = locale.format_template(
            crate::i18n::UiText::HistoryCopyInProgress,
            &[("path", &path_detail)],
        );
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
                        this.finish_history_copy(result, generation, clipboard_write_id, cx)
                    });
                }
            }
        })
        .detach();
    }

    /// Decodes a retained screenshot in the background before opening it as an always-on-top pin.
    pub(in crate::app) fn pin_history_image(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !self.can_start_history_reader(&path) {
            return;
        }
        let generation = self.begin_history_reader(HistoryReaderKind::Pin, path.clone());
        let path_detail = path.display().to_string();
        self.status = self.settings.locale.format_template(
            crate::i18n::UiText::PinHistoryInProgress,
            &[("path", &path_detail)],
        );
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
        let current = self.finish_history_reader(HistoryReaderKind::Pin, generation);
        if !current || self.session.state() != CaptureSessionState::Idle {
            cx.notify();
            return;
        }
        match result {
            Ok(prepared) => self.open_prepared_pinned_frame(
                prepared,
                crate::i18n::UiText::PinHistoryOpened,
                Some(crate::i18n::UiText::PinHistoryFailed),
                false,
                cx,
            ),
            Err(error) => {
                let error_detail = error.to_string();
                self.status = self.settings.locale.format_template(
                    crate::i18n::UiText::PinHistoryError,
                    &[("error", &error_detail)],
                );
                log::warn!(target: "flash_shot::pinned", "history_pin_failed error={error}");
                self.notify_user(
                    self.settings.locale.text(crate::i18n::UiText::AppName),
                    self.settings
                        .locale
                        .text(crate::i18n::UiText::PinHistoryFailed),
                );
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
        if self.history_mutation_pending() {
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

    /// Clears one failed preview and schedules a bounded decode so a repaired or temporarily
    /// unavailable history file can recover without changing the saved capture list.
    pub(in crate::app) fn retry_history_thumbnail(
        &mut self,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let retained = self
            .history
            .entries()
            .iter()
            .any(|entry| entry.path == path);
        if !history_thumbnail_retry_can_start(
            self.history_thumbnail_failed.contains(&path),
            self.history_thumbnail_loading.contains(&path),
            self.history_reader.is_some(),
            self.generic_open_generation.is_some(),
            self.history_mutation_pending(),
            retained,
            self.session.state(),
        ) {
            return;
        }

        self.history_thumbnail_failed.remove(&path);
        let _ = enqueue_history_thumbnail_path(
            path,
            &mut self.history_thumbnail_pending,
            &self.history_thumbnail_loading,
        );
        self.status = self
            .settings
            .locale
            .text(crate::i18n::UiText::LibraryPreviewRetrying)
            .to_owned();
        self.pump_history_thumbnail_queue(cx);
        cx.notify();
    }

    /// Starts at most two PNG decodes at once so expanding a long history cannot flood the
    /// background executor or compete with a user-initiated capture/export.
    fn pump_history_thumbnail_queue(&mut self, cx: &mut Context<Self>) {
        if self.history_mutation_pending() {
            return;
        }
        while let Some(path) = take_next_history_thumbnail(
            &mut self.history_thumbnail_pending,
            &mut self.history_thumbnail_loading,
            HISTORY_THUMBNAIL_MAX_IN_FLIGHT,
        ) {
            let this_path = path.clone();
            let revision = self.history_thumbnail_revision;
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
                            this.finish_history_thumbnail(this_path, revision, result, cx)
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
        revision: u64,
        result: std::io::Result<CaptureFrame>,
        cx: &mut Context<Self>,
    ) {
        self.history_thumbnail_loading.remove(&path);
        let still_retained = self
            .history
            .entries()
            .iter()
            .any(|entry| entry.path == path);
        if thumbnail_completion_can_cache(
            revision,
            self.history_thumbnail_revision,
            self.history_mutation_pending(),
            still_retained,
        ) {
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
                preview,
                document,
                document_warning,
            } => {
                let bounds = frame.bounds;
                let result = (|| -> std::io::Result<()> {
                    self.session.frames_ready().map_err(std::io::Error::other)?;
                    let document = document
                        .unwrap_or(AnnotationDocument::new(bounds).map_err(std::io::Error::other)?);
                    let (next_annotation_id, next_sequence_number) =
                        next_annotation_counters(&document);
                    self.session.select(bounds).map_err(std::io::Error::other)?;
                    self.history_source = crate::history::HistorySource::Selection;
                    self.preview = Some(preview);
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
                            Some(warning) => self.settings.locale.format_template(
                                crate::i18n::UiText::OpenImageOpenedWithoutAnnotations,
                                &[("path", &path.display().to_string()), ("warning", &warning)],
                            ),
                            None => self.settings.locale.format_template(
                                crate::i18n::UiText::OpenImageOpened,
                                &[("path", &path.display().to_string())],
                            ),
                        };
                        if let Some(handle) = self.settings_window_handle
                            && let Err(error) = window_visibility::hide(handle)
                        {
                            let message = self.settings.locale.format_template(
                                crate::i18n::UiText::SettingsHideBeforeEditorFailed,
                                &[("error", &error.to_string())],
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
                        let message = self.settings.locale.format_template(
                            crate::i18n::UiText::OpenImageFailed,
                            &[("error", &error.to_string())],
                        );
                        let _ = self.session.fail(message.clone());
                        self.status = message;
                    }
                }
            }
            OpenImageOutcome::Cancelled => {
                let _ = self.session.cancel();
                let _ = self.session.reset();
                self.status = self
                    .settings
                    .locale
                    .text(crate::i18n::UiText::OpenImageCancelled)
                    .to_owned();
            }
            OpenImageOutcome::Failed(error) => {
                let message = self
                    .settings
                    .locale
                    .format_template(crate::i18n::UiText::OpenImageFailed, &[("error", &error)]);
                let _ = self.session.fail(message.clone());
                self.status = message;
            }
        }
        cx.notify();
    }

    /// Completes a history Open only after releasing the exact reader lease that decoded it.
    fn finish_history_open_image(
        &mut self,
        outcome: OpenImageOutcome,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if !self.finish_history_reader(HistoryReaderKind::Open, generation) {
            cx.notify();
            return;
        }
        self.finish_open_image(outcome, generation, cx);
    }

    fn finish_history_copy(
        &mut self,
        result: std::io::Result<()>,
        generation: u64,
        clipboard_write_id: u64,
        cx: &mut Context<Self>,
    ) {
        let released_clipboard = self.finish_clipboard_write(clipboard_write_id);
        let current = self.finish_history_reader(HistoryReaderKind::Copy, generation);
        if !released_clipboard || !current || self.session.state() != CaptureSessionState::Idle {
            cx.notify();
            return;
        }
        let locale = self.settings.locale;
        self.status = match result {
            Ok(()) => locale
                .text(crate::i18n::UiText::HistoryCopiedToClipboard)
                .to_owned(),
            Err(error) => {
                let error_detail = error.to_string();
                locale.format_template(
                    crate::i18n::UiText::HistoryCopyFailed,
                    &[("error", &error_detail)],
                )
            }
        };
        cx.notify();
    }
}

/// Answers whether one interactive history reader can own its source file.
fn history_reader_can_start(
    reader_in_flight: bool,
    conflicting_operation_in_flight: bool,
    history_mutation_pending: bool,
    path_is_managed: bool,
    session_state: CaptureSessionState,
) -> bool {
    !reader_in_flight
        && !conflicting_operation_in_flight
        && !history_mutation_pending
        && path_is_managed
        && session_state == CaptureSessionState::Idle
}

/// Reports whether a mutation can safely take ownership of the retained file set.
fn history_mutation_can_start(
    history_reader_in_flight: bool,
    generic_open_in_flight: bool,
    thumbnail_read_in_flight: bool,
    mutation_pending: bool,
) -> bool {
    !history_reader_in_flight
        && !generic_open_in_flight
        && !thumbnail_read_in_flight
        && !mutation_pending
}

/// Claims a completion only when it belongs to the exact reader lease that is still active.
fn claim_history_reader_completion(
    active: &mut Option<HistoryReaderLease>,
    kind: HistoryReaderKind,
    generation: u64,
) -> Option<HistoryReaderLease> {
    let lease = active.as_ref()?;
    if lease.kind != kind || lease.generation != generation {
        return None;
    }
    active.take()
}

/// Clears a managed-save lease only when its completion belongs to the active write.
///
/// Saves use their own sequence because a capture reset may advance the UI operation generation
/// while the file writer is still finishing. A late completion must never clear a newer save.
fn claim_history_write_completion(active: &mut Option<u64>, generation: u64) -> bool {
    if *active != Some(generation) {
        return false;
    }
    *active = None;
    true
}

/// Releases a matching generic picker lease before deciding whether its result can affect the UI.
///
/// Resetting advances the operation generation but intentionally leaves this reservation active;
/// the late completion releases its own lease without being able to change the newer UI state.
fn claim_generic_open_completion(
    active: &mut Option<u64>,
    current_generation: u64,
    completion_generation: u64,
) -> bool {
    if *active != Some(completion_generation) {
        return false;
    }
    *active = None;
    is_current_operation(current_generation, completion_generation)
}

/// Prevents an old thumbnail decode from repopulating a cache after its file set changed.
fn thumbnail_completion_can_cache(
    completion_revision: u64,
    current_revision: u64,
    mutation_pending: bool,
    path_is_still_retained: bool,
) -> bool {
    completion_revision == current_revision && !mutation_pending && path_is_still_retained
}

/// Allows a retry only for a failed retained file while every competing history reader is idle.
fn history_thumbnail_retry_can_start(
    failed: bool,
    loading: bool,
    history_reader_in_flight: bool,
    generic_open_in_flight: bool,
    mutation_pending: bool,
    path_is_still_retained: bool,
    session_state: CaptureSessionState,
) -> bool {
    failed
        && !loading
        && !history_reader_in_flight
        && !generic_open_in_flight
        && !mutation_pending
        && path_is_still_retained
        && session_state == CaptureSessionState::Idle
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
        HISTORY_THUMBNAIL_MAX_IN_FLIGHT, claim_generic_open_completion,
        claim_history_reader_completion, claim_history_write_completion,
        enqueue_history_thumbnail_path, history_mutation_can_start, history_reader_can_start,
        history_thumbnail_retry_can_start, retain_history_thumbnail_pending,
        take_next_history_thumbnail, thumbnail_completion_can_cache,
    };
    use crate::app::{HistoryReaderKind, HistoryReaderLease};
    use crate::domain::session::CaptureSessionState;
    use std::{
        collections::{HashSet, VecDeque},
        path::PathBuf,
    };

    #[test]
    fn history_reader_start_requires_an_idle_uncontended_managed_file() {
        let idle = CaptureSessionState::Idle;
        assert!(history_reader_can_start(false, false, false, true, idle));

        // Open, Copy, and Pin share one lease so no completion can write stale data after a
        // destructive history request or another clipboard-producing operation begins.
        assert!(!history_reader_can_start(true, false, false, true, idle));
        assert!(!history_reader_can_start(false, true, false, true, idle));
        assert!(!history_reader_can_start(false, false, true, true, idle));
        assert!(!history_reader_can_start(false, false, false, false, idle));
        assert!(!history_reader_can_start(
            false,
            false,
            false,
            true,
            CaptureSessionState::Selecting,
        ));
    }

    #[test]
    fn history_reader_completion_only_releases_its_matching_lease() {
        let path = PathBuf::from("managed.png");
        let lease = HistoryReaderLease {
            kind: HistoryReaderKind::Copy,
            generation: 42,
            path: path.clone(),
        };
        let mut active = Some(lease.clone());

        assert_eq!(
            claim_history_reader_completion(&mut active, HistoryReaderKind::Pin, 42),
            None
        );
        assert_eq!(active, Some(lease.clone()));
        assert_eq!(
            claim_history_reader_completion(&mut active, HistoryReaderKind::Copy, 41),
            None
        );
        assert_eq!(active, Some(lease.clone()));
        assert_eq!(
            claim_history_reader_completion(&mut active, HistoryReaderKind::Copy, 42),
            Some(lease)
        );
        assert_eq!(active, None);
    }

    #[test]
    fn history_write_completion_cannot_release_a_newer_save() {
        let mut active = Some(18);

        assert!(!claim_history_write_completion(&mut active, 17));
        assert_eq!(active, Some(18));
        assert!(claim_history_write_completion(&mut active, 18));
        assert_eq!(active, None);
    }

    #[test]
    fn stale_generic_open_completion_releases_only_its_own_picker_lease() {
        let mut active = Some(42);

        assert!(!claim_generic_open_completion(&mut active, 43, 42));
        assert_eq!(active, None);

        let mut newer_open = Some(43);
        assert!(!claim_generic_open_completion(&mut newer_open, 43, 42));
        assert_eq!(newer_open, Some(43));
    }

    #[test]
    fn history_mutation_waits_for_every_retained_file_reader() {
        assert!(history_mutation_can_start(false, false, false, false));
        assert!(!history_mutation_can_start(true, false, false, false));
        assert!(!history_mutation_can_start(false, true, false, false));
        assert!(!history_mutation_can_start(false, false, true, false));
        assert!(!history_mutation_can_start(false, false, false, true));
    }

    #[test]
    fn stale_thumbnail_completion_cannot_repopulate_a_changed_history() {
        assert!(thumbnail_completion_can_cache(8, 8, false, true));
        assert!(!thumbnail_completion_can_cache(7, 8, false, true));
        assert!(!thumbnail_completion_can_cache(8, 8, true, true));
        assert!(!thumbnail_completion_can_cache(8, 8, false, false));
    }

    #[test]
    fn thumbnail_retry_requires_a_failed_retained_idle_entry() {
        let idle = CaptureSessionState::Idle;
        assert!(history_thumbnail_retry_can_start(
            true, false, false, false, false, true, idle,
        ));
        assert!(!history_thumbnail_retry_can_start(
            false, false, false, false, false, true, idle,
        ));
        assert!(!history_thumbnail_retry_can_start(
            true, true, false, false, false, true, idle,
        ));
        assert!(!history_thumbnail_retry_can_start(
            true, false, true, false, false, true, idle,
        ));
        assert!(!history_thumbnail_retry_can_start(
            true, false, false, true, false, true, idle,
        ));
        assert!(!history_thumbnail_retry_can_start(
            true, false, false, false, true, true, idle,
        ));
        assert!(!history_thumbnail_retry_can_start(
            true, false, false, false, false, false, idle,
        ));
        assert!(!history_thumbnail_retry_can_start(
            true,
            false,
            false,
            false,
            false,
            true,
            CaptureSessionState::Selecting,
        ));
    }

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
