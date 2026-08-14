//! Saving, export, history mutation, and workflow cleanup.

use super::super::HistoryClearScope;
use super::*;

impl FlashShotApp {
    pub(in crate::app) fn save_selection(&mut self, cx: &mut Context<Self>) {
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
        let demoted_overlays = match self.demote_capture_overlays_for_dialog(cx) {
            Ok(handles) => handles,
            Err(error) => {
                let _ = self.session.export_cancelled();
                self.status = format!("Could not show Save dialog above capture: {error}");
                cx.notify();
                return;
            }
        };

        self.status = "Choose where to save the selection...".to_owned();
        let generation = self.operation_generation;
        // The source label belongs to this export, not to whichever capture may be active when the
        // native dialog and background writer eventually return.
        let history_source = self.history_source;
        cx.notify();
        let suggested_name = default_image_filename(self.settings.export_format);
        let prompt = cx.prompt_for_new_path(&PathBuf::default(), Some(&suggested_name));
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let prompt_result = prompt.await;
                // The common dialog is gone now; restore z-order before potentially slow file I/O.
                Self::restore_capture_overlays_after_dialog(&demoted_overlays);
                let outcome = match prompt_result {
                    Ok(Ok(Some(path))) => {
                        let path = export_path(path);
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
                        this.finish_save(outcome, generation, None, history_source, cx)
                    });
                }
            }
        })
        .detach();
    }

    pub(in crate::app) fn quick_save_selection(&mut self, cx: &mut Context<Self>) {
        let Some(history_write_generation) = self.begin_history_write() else {
            self.status = "Waiting for active history work before saving...".to_owned();
            cx.notify();
            return;
        };
        let selection = match self.session.start_export() {
            Ok(selection) => selection,
            Err(error) => {
                self.finish_history_write(history_write_generation);
                self.status = error.to_string();
                cx.notify();
                return;
            }
        };
        let Some((frame, document)) = self.export_source() else {
            self.finish_history_write(history_write_generation);
            cx.notify();
            return;
        };

        self.status = "Quick saving selection...".to_owned();
        let generation = self.operation_generation;
        // The worker can finish after Reset or a new capture changes the live app state. Keep the
        // source that produced these pixels with the task so its managed-history entry is exact.
        let history_source = self.history_source;
        let directory = self.history.root().to_owned();
        let prefix = self.settings.quick_save_prefix.clone();
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let fallback = managed_history_fallback(&directory);
                        quick_save_annotated_frame_selection_with_fallback(
                            &frame,
                            &document,
                            selection,
                            &directory,
                            fallback.as_deref(),
                            &prefix,
                        )
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
                        this.finish_save(
                            outcome,
                            generation,
                            Some(history_write_generation),
                            history_source,
                            cx,
                        )
                    });
                }
            }
        })
        .detach();
    }

    /// Saves an independent pinned frame and adds it to local history without reopening capture UI.
    pub(in crate::app) fn quick_save_pinned_frame(
        &mut self,
        frame: CaptureFrame,
        pin: WeakEntity<PinnedImage>,
        cx: &mut Context<Self>,
    ) -> bool {
        if !claim_pinned_save_slot(&mut self.pinned_save_in_flight) {
            self.status = "Another pinned image is already saving. Try again shortly.".to_owned();
            cx.notify();
            return false;
        }
        let Some(history_write_generation) = self.begin_history_write() else {
            self.pinned_save_in_flight = false;
            self.status = "Waiting for active history work before saving...".to_owned();
            cx.notify();
            return false;
        };
        self.status = "Saving pinned image...".to_owned();
        let directory = self.history.root().to_owned();
        let prefix = self.settings.quick_save_prefix.clone();
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move {
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
                        this.pinned_save_in_flight = false;
                        let pin_saved = result.is_ok();
                        match result {
                            Ok(path) => {
                                let history_note = this.record_managed_save_with_recovery(
                                    &path,
                                    crate::history::HistorySource::Pinned,
                                );
                                this.status = format!("Pinned image saved to {}", path.display());
                                if let Some(history_note) = history_note {
                                    this.status.push_str(&history_note);
                                }
                                this.notify_user("Flash Shot", "Pinned image saved");
                            }
                            Err(error) => {
                                this.status = format!("Could not save pinned image: {error}");
                                log::warn!(target: "flash_shot::pinned", "pinned_save_failed error={error}");
                            }
                        }
                        if this.finish_history_write(history_write_generation) && pin_saved {
                            this.synchronize_history_preview_cache();
                            this.resume_history_thumbnail_queue(cx);
                        }
                        let _ = pin.update(cx, |pin, cx| {
                            pin.finish_save_status(pin_saved, cx);
                        });
                        cx.notify();
                    });
                }
            }
        })
        .detach();
        true
    }

    pub(in crate::app) fn save_annotation_document(&mut self, cx: &mut Context<Self>) {
        let Some(document) = self.annotation_document.clone() else {
            self.status = "Annotation document is unavailable".to_owned();
            cx.notify();
            return;
        };
        let demoted_overlays = match self.demote_capture_overlays_for_dialog(cx) {
            Ok(handles) => handles,
            Err(error) => {
                self.status = format!("Could not show annotation Save dialog: {error}");
                cx.notify();
                return;
            }
        };
        self.status = "Choose where to save annotations...".to_owned();
        cx.notify();
        let prompt =
            cx.prompt_for_new_path(&PathBuf::default(), Some("flash-shot.annotations.json"));
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let prompt_result = prompt.await;
                // The common dialog is gone now; restore z-order before potentially slow file I/O.
                Self::restore_capture_overlays_after_dialog(&demoted_overlays);
                let result = match prompt_result {
                    Ok(Ok(Some(path))) => {
                        let path = annotation_document_path(path);
                        cx.background_executor()
                            .spawn(async move {
                                save_annotation_document(&document, path.clone()).map(|()| path)
                            })
                            .await
                            .map(Some)
                    }
                    Ok(Ok(None)) => Ok(None),
                    Ok(Err(error)) => Err(std::io::Error::other(error)),
                    Err(error) => Err(std::io::Error::other(error.to_string())),
                };
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.status = match result {
                            Ok(Some(path)) => {
                                format!("Annotations saved to {}", path.display())
                            }
                            Ok(None) => "Annotation save cancelled".to_owned(),
                            Err(error) => format!("Could not save annotations: {error}"),
                        };
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(in crate::app) fn save_editable_project(&mut self, cx: &mut Context<Self>) {
        let Some((frame, document)) = self.export_source() else {
            cx.notify();
            return;
        };
        let demoted_overlays = match self.demote_capture_overlays_for_dialog(cx) {
            Ok(handles) => handles,
            Err(error) => {
                self.status = format!("Could not show editable-project Save dialog: {error}");
                cx.notify();
                return;
            }
        };
        self.status = "Choose where to save the editable image...".to_owned();
        cx.notify();
        let prompt = cx.prompt_for_new_path(&PathBuf::default(), Some("flash-shot-editable.png"));
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let prompt_result = prompt.await;
                // The common dialog is gone now; restore z-order before potentially slow file I/O.
                Self::restore_capture_overlays_after_dialog(&demoted_overlays);
                let result = match prompt_result {
                    Ok(Ok(Some(path))) => {
                        let path = png_path(path);
                        cx.background_executor()
                            .spawn(async move {
                                save_editable_project(&frame, &document, path.clone())
                                    .map(|()| path)
                            })
                            .await
                            .map(Some)
                    }
                    Ok(Ok(None)) => Ok(None),
                    Ok(Err(error)) => Err(std::io::Error::other(error)),
                    Err(error) => Err(std::io::Error::other(error.to_string())),
                };
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.status = match result {
                            Ok(Some(path)) => format!(
                                "Editable project saved to {} and {}",
                                path.display(),
                                annotation_sidecar_path(&path).display()
                            ),
                            Ok(None) => "Editable-project save cancelled".to_owned(),
                            Err(error) => format!("Could not save editable project: {error}"),
                        };
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    /// Opens annotations for this capture and ignores a result after a newer capture replaces it.
    pub(in crate::app) fn open_annotation_document(&mut self, cx: &mut Context<Self>) {
        let Some(frame) = self.frame.as_ref() else {
            self.status = "Capture frame is unavailable".to_owned();
            cx.notify();
            return;
        };
        let bounds = frame.bounds;
        let generation = self.operation_generation;
        let demoted_overlays = match self.demote_capture_overlays_for_dialog(cx) {
            Ok(handles) => handles,
            Err(error) => {
                self.status = format!("Could not show annotation Open dialog: {error}");
                cx.notify();
                return;
            }
        };
        self.status = "Choose annotations to open...".to_owned();
        cx.notify();
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open annotation document".into()),
        });
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let prompt_result = prompt.await;
                // The common dialog is gone now; restore z-order before potentially slow file I/O.
                Self::restore_capture_overlays_after_dialog(&demoted_overlays);
                let result = match prompt_result {
                    Ok(Ok(Some(mut paths))) => match paths.pop() {
                        Some(path) => cx
                            .background_executor()
                            .spawn(async move {
                                load_annotation_document(&path, bounds)
                                    .map(|document| (path, document))
                            })
                            .await
                            .map(Some),
                        None => Ok(None),
                    },
                    Ok(Ok(None)) => Ok(None),
                    Ok(Err(error)) => Err(std::io::Error::other(error)),
                    Err(error) => Err(std::io::Error::other(error.to_string())),
                };
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        if !is_current_operation(this.operation_generation, generation) {
                            return;
                        }
                        match result {
                            Ok(Some((path, document))) => {
                                let (next_id, next_sequence) = next_annotation_counters(&document);
                                this.annotation_document = Some(document);
                                this.annotation_history = Default::default();
                                this.annotation_editor = Default::default();
                                this.annotation_tool = None;
                                this.text_edit = None;
                                this.text_edit_annotation = None;
                                this.selected_annotation = None;
                                this.next_annotation_id = next_id;
                                this.next_sequence_number = next_sequence;
                                this.status = format!("Loaded annotations from {}", path.display());
                                cx.notify();
                            }
                            Ok(None) => {
                                this.status = "Open annotations cancelled".to_owned();
                                cx.notify();
                            }
                            Err(error) => {
                                this.status = format!("Could not open annotations: {error}");
                                cx.notify();
                            }
                        }
                    });
                }
            }
        })
        .detach();
    }

    /// Returns every live capture-overlay HWND so native dialogs can be placed above them.
    #[cfg(windows)]
    fn capture_overlay_native_handles(
        &self,
        cx: &mut Context<Self>,
    ) -> std::io::Result<Vec<isize>> {
        self.overlay_windows
            .iter()
            .map(|overlay| {
                overlay
                    .update(cx, |_, window, _| {
                        let handle = window
                            .window_handle()
                            .map_err(|error| std::io::Error::other(error.to_string()))?;
                        match handle.as_raw() {
                            RawWindowHandle::Win32(handle) => Ok(handle.hwnd.get()),
                            _ => Err(std::io::Error::new(
                                std::io::ErrorKind::Unsupported,
                                "capture overlay does not expose a Win32 HWND",
                            )),
                        }
                    })
                    .map_err(|error| std::io::Error::other(error.to_string()))?
            })
            .collect()
    }

    /// Demotes all overlays transactionally and returns the exact HWNDs changed by this operation.
    #[cfg(windows)]
    fn demote_capture_overlays_for_dialog(
        &self,
        cx: &mut Context<Self>,
    ) -> std::io::Result<Vec<isize>> {
        let handles = self.capture_overlay_native_handles(cx)?;
        let mut demoted = Vec::with_capacity(handles.len());
        for handle in handles {
            if let Err(error) = window_visibility::make_not_topmost(handle) {
                for handle in demoted {
                    let _ = window_visibility::make_topmost(handle);
                }
                return Err(error);
            }
            demoted.push(handle);
        }
        Ok(demoted)
    }

    /// Other platforms do not have the Windows topmost z-order that obscures common dialogs.
    #[cfg(not(windows))]
    fn demote_capture_overlays_for_dialog(
        &self,
        _cx: &mut Context<Self>,
    ) -> std::io::Result<Vec<isize>> {
        Ok(Vec::new())
    }

    /// Restores every surviving HWND changed by the matching dialog operation independently.
    fn restore_capture_overlays_after_dialog(handles: &[isize]) {
        for handle in handles {
            if let Err(error) = window_visibility::make_topmost(*handle) {
                log::warn!(target: "flash_shot::overlay", "overlay_topmost_restore_failed error={error}");
            }
        }
    }

    /// Removes a natively closed overlay by exact ID and clears deferred teardown when all
    /// windows from the same close batch have reported their native close callback.
    pub(in crate::app) fn unregister_capture_overlay(
        &mut self,
        closing_id: gpui::WindowId,
        cx: &mut Context<Self>,
    ) -> bool {
        let teardown_complete =
            finish_capture_teardown(&mut self.capture_teardown_windows, closing_id);
        if teardown_complete {
            self.capture_teardown_pending = false;
        }
        let removed = remove_capture_overlay_by_id(&mut self.overlay_windows, closing_id);
        if !removed {
            if teardown_complete {
                cx.notify();
            }
            return teardown_complete;
        }
        if self.overlay_windows.is_empty() && self.session.state() != CaptureSessionState::Idle {
            self.reset(cx);
        } else {
            cx.notify();
        }
        true
    }

    pub(super) fn export_source(&mut self) -> Option<(CaptureFrame, AnnotationDocument)> {
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

    fn finish_save(
        &mut self,
        outcome: SaveOutcome,
        generation: u64,
        history_write_generation: Option<u64>,
        history_source: crate::history::HistorySource,
        cx: &mut Context<Self>,
    ) {
        if !is_current_operation(self.operation_generation, generation) {
            let refresh_history_preview =
                matches!(&outcome, SaveOutcome::Saved { managed: true, .. });
            if let SaveOutcome::Saved {
                path,
                managed: true,
            } = &outcome
            {
                // The file was already written even though the capture UI was reset. Record it
                // before releasing the lease so retention still sees the managed PNG.
                let _ = self.record_managed_save_with_recovery(path, history_source);
            }
            if let Some(history_write_generation) = history_write_generation
                && self.finish_history_write(history_write_generation)
                && refresh_history_preview
            {
                self.synchronize_history_preview_cache();
                self.resume_history_thumbnail_queue(cx);
            }
            cx.notify();
            return;
        }
        let refresh_history_preview = matches!(&outcome, SaveOutcome::Saved { managed: true, .. });
        match outcome {
            SaveOutcome::Saved { path, managed } => {
                if let Err(error) = self.session.export_completed() {
                    self.status = error.to_string();
                } else {
                    let history_status = managed
                        .then(|| self.record_managed_save_with_recovery(&path, history_source))
                        .flatten();
                    self.status = format!("{} saved to {}", history_source.label(), path.display());
                    if let Some(history_status) = history_status {
                        self.status.push_str(&history_status);
                    }
                    self.notify_user("Flash Shot", "Screenshot saved");
                    self.close_capture_overlays(cx);
                    self.return_to_background();
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
                self.return_to_background();
            }
        }
        if let Some(history_write_generation) = history_write_generation
            && self.finish_history_write(history_write_generation)
            && refresh_history_preview
        {
            self.synchronize_history_preview_cache();
            self.resume_history_thumbnail_queue(cx);
        }
        cx.notify();
    }

    /// Records a managed save and adopts its parent when a configured history root became stale.
    /// The image is already safe on disk; this keeps the history index and future quick saves
    /// aligned with the recovery directory instead of reporting a false save failure.
    fn record_managed_save_with_recovery(
        &mut self,
        path: &Path,
        source: crate::history::HistorySource,
    ) -> Option<String> {
        if path.starts_with(self.history.root()) {
            return self
                .history
                .record_with_source(path.to_owned(), source)
                .err()
                .map(|error| {
                    log::warn!(target: "flash_shot::history", "history_record_failed error={error}");
                    format!("; history unavailable: {error}")
                });
        }

        let Some(parent) = path.parent() else {
            return Some("; history unavailable: saved path has no parent".to_owned());
        };
        let mut recovered = match crate::history::ScreenshotHistory::open_with_limit(
            parent,
            self.history.limit(),
        ) {
            Ok(history) => history,
            Err(error) => {
                log::warn!(target: "flash_shot::history", "history_recovery_open_failed error={error}");
                return Some(format!("; history unavailable: {error}"));
            }
        };
        if let Err(error) = recovered.record_with_source(path.to_owned(), source) {
            log::warn!(target: "flash_shot::history", "history_recovery_record_failed error={error}");
            return Some(format!("; history unavailable: {error}"));
        }
        self.history = recovered;

        let mut note = format!(
            "; quick-save folder unavailable; using {}",
            self.history.root().display()
        );
        if self.settings.quick_save_directory.take().is_some()
            && let Err(error) = self.settings.save(&self.settings_path)
        {
            log::warn!(target: "flash_shot::history", "history_recovery_preference_clear_failed error={error}");
            note.push_str(&format!(" (could not persist fallback: {error})"));
        }
        Some(note)
    }

    pub(in crate::app) fn clear_history(&mut self, cx: &mut Context<Self>) {
        if !self.history_clear_confirmation && !self.history_mutation_can_start() {
            return;
        }
        if !self.history_clear_confirmation {
            self.request_history_clear(cx);
            return;
        }
        if !self.history_clear_can_commit() {
            self.status = "Waiting for active history reads before deleting...".to_owned();
            cx.notify();
            return;
        }
        let scope = self.history_clear_scope;
        let paths = std::mem::take(&mut self.history_clear_paths);
        let snapshot = self.history.clone();
        self.history_clear_in_flight = true;
        self.invalidate_history_thumbnails();
        self.history_clear_confirmation = false;
        self.history_clear_scope = HistoryClearScope::default();
        self.history_clear_count = 0;
        self.status = format!("Clearing {} saved capture(s)...", paths.len());
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move { snapshot.delete_managed_paths(paths) })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.finish_clear_history(scope, result, cx)
                    });
                }
            }
        })
        .detach();
    }

    fn finish_clear_history(
        &mut self,
        scope: HistoryClearScope,
        deletion: crate::history::HistoryFileDeletion,
        cx: &mut Context<Self>,
    ) {
        self.history_clear_in_flight = false;
        let deleted_count = deletion.deleted.len();
        let failure_count = deletion.failures.len();
        for (path, error) in &deletion.failures {
            log::warn!(target: "flash_shot::history", "history_file_delete_failed path={} error={error}", path.display());
        }
        self.history_selected_paths
            .retain(|path| !deletion.deleted.contains(path));
        self.status = match self.history.forget_deleted(&deletion.deleted) {
            Ok(()) if failure_count == 0 && scope == HistoryClearScope::All => {
                "Screenshot history cleared".to_owned()
            }
            Ok(()) if failure_count == 0 && scope == HistoryClearScope::Selected => {
                format!("Deleted {deleted_count} selected capture(s)")
            }
            Ok(()) if failure_count == 0 => {
                format!("Cleared {deleted_count} filtered capture(s)")
            }
            Ok(()) => {
                format!("Cleared {deleted_count} capture(s); {failure_count} could not be deleted")
            }
            Err(error) => {
                log::warn!(target: "flash_shot::history", "history_index_update_failed error={error}");
                format!("Capture files were cleared, but history could not be updated: {error}")
            }
        };
        if scope == HistoryClearScope::All {
            self.history_expanded = false;
        }
        self.synchronize_history_preview_cache();
        cx.notify();
    }

    pub(in crate::app) fn remove_history_image(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !self.history_mutation_can_start()
            || !self
                .history
                .entries()
                .iter()
                .any(|entry| entry.path == path)
            || !self.history_deletions_in_flight.insert(path.clone())
        {
            return;
        }
        self.invalidate_history_thumbnails();
        self.status = format!("Removing {}...", path.display());
        cx.notify();
        let snapshot = self.history.clone();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let deletion = cx
                    .background_executor()
                    .spawn({
                        let path = path.clone();
                        async move { snapshot.delete_managed_paths([path]) }
                    })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.finish_remove_history_image(path, deletion, cx)
                    });
                }
            }
        })
        .detach();
    }

    fn finish_remove_history_image(
        &mut self,
        path: PathBuf,
        deletion: crate::history::HistoryFileDeletion,
        cx: &mut Context<Self>,
    ) {
        self.history_deletions_in_flight.remove(&path);
        for (failed_path, error) in &deletion.failures {
            log::warn!(target: "flash_shot::history", "history_remove_failed path={} error={error}", failed_path.display());
        }
        self.status = if let Some((_, error)) = deletion.failures.first() {
            format!("Could not remove screenshot history item: {error}")
        } else {
            match self.history.forget_deleted(&deletion.deleted) {
                Ok(()) => format!("Removed {} from screenshot history", path.display()),
                Err(error) => {
                    log::warn!(target: "flash_shot::history", "history_index_update_failed error={error}");
                    format!("Capture was removed, but history could not be updated: {error}")
                }
            }
        };
        self.synchronize_history_preview_cache();
        cx.notify();
    }

    /// Drops decoded previews and stale batch selections as soon as history entries disappear.
    pub(super) fn synchronize_history_preview_cache(&mut self) {
        let retained = self
            .history
            .entries()
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<std::collections::HashSet<_>>();
        self.history_selected_paths
            .retain(|path| retained.contains(path));
        if self
            .history_keyboard_focus
            .as_ref()
            .is_some_and(|path| !retained.contains(path))
        {
            self.history_keyboard_focus = None;
        }
        self.history_thumbnails
            .retain(|path, _| retained.contains(path));
        super::images::retain_history_thumbnail_pending(
            &mut self.history_thumbnail_pending,
            &retained,
        );
        self.history_thumbnail_failed
            .retain(|path| retained.contains(path));
    }

    pub(super) fn finish_copy(
        &mut self,
        result: std::io::Result<bool>,
        copy_id: u64,
        cx: &mut Context<Self>,
    ) {
        let completion_context = self
            .selection_copy
            .as_ref()
            .filter(|copy| copy.id == copy_id)
            .map(|copy| (copy.status_generation, copy.recognition_generation));
        let owns_completion = completion_context.is_some();
        let released_clipboard = self.finish_clipboard_write(copy_id);
        if owns_completion {
            self.selection_copy = None;
        }
        if !owns_completion || !released_clipboard {
            cx.notify();
            return;
        }
        // Pointer movement and duplicate Copy commands may change status text without replacing
        // this editor. Only a new editor operation or recognition generation suppresses the late
        // result; the worker still releases both leases in every case.
        let Some((operation_generation, recognition_generation)) = completion_context else {
            cx.notify();
            return;
        };
        if !selection_copy_completion_can_report(
            operation_generation,
            self.operation_generation,
            recognition_generation,
            self.recognition_generation,
            self.session.state(),
        ) {
            cx.notify();
            return;
        }
        match result {
            Ok(true) => {
                self.status = "Selection copied to clipboard".to_owned();
                self.notify_user("Flash Shot", "Screenshot copied to clipboard");
            }
            Ok(false) => {
                self.status = "Copy cancelled before the clipboard changed".to_owned();
            }
            Err(error) => {
                let message = format!("Copy failed: {error}");
                self.status = message;
            }
        }
        cx.notify();
    }

    pub(super) fn finish_full_screen_copy(
        &mut self,
        result: std::io::Result<()>,
        generation: u64,
        clipboard_write_id: u64,
        cx: &mut Context<Self>,
    ) {
        let released_clipboard = self.finish_clipboard_write(clipboard_write_id);
        if !claim_idle_completion(
            &mut self.full_screen_copy_generation,
            self.operation_generation,
            generation,
            self.session.state(),
        ) || !released_clipboard
        {
            cx.notify();
            return;
        }
        match result {
            Ok(()) => {
                self.status = "Full screen copied to clipboard".to_owned();
                self.notify_user("Flash Shot", "Full screen copied to clipboard");
            }
            Err(error) => {
                self.status = format!("Could not copy full screen: {error}");
                log::warn!(target: "flash_shot::capture", "full_screen_copy_failed error={error}");
            }
        }
        cx.notify();
    }

    /// Opens the captured virtual desktop only when this remains the active pin request.
    pub(super) fn finish_full_screen_pin(
        &mut self,
        result: std::io::Result<super::pinning::PreparedPinnedFrame>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if !claim_idle_completion(
            &mut self.full_screen_pin_generation,
            self.operation_generation,
            generation,
            self.session.state(),
        ) {
            return;
        }
        match result {
            Ok(prepared) => self.open_prepared_pinned_frame(
                prepared,
                "Full screen pinned in an always-on-top window",
                Some("Could not pin full screen"),
                false,
                cx,
            ),
            Err(error) => {
                self.status = format!("Could not pin full screen: {error}");
                log::warn!(target: "flash_shot::pinned", "full_screen_pin_failed error={error}");
                self.notify_user("Flash Shot", "Could not pin full screen");
                cx.notify();
            }
        }
    }

    /// Finishes a tray full-screen save, recording the managed file only after it was written.
    pub(super) fn finish_full_screen_save(
        &mut self,
        result: std::io::Result<PathBuf>,
        generation: u64,
        history_write_generation: u64,
        cx: &mut Context<Self>,
    ) {
        if !claim_idle_completion(
            &mut self.full_screen_save_generation,
            self.operation_generation,
            generation,
            self.session.state(),
        ) {
            if let Ok(path) = &result {
                // A reset cannot cancel the file write. Preserve managed-history invariants even
                // when the visible tray action has been superseded by a newer UI operation.
                let _ = self.record_managed_save_with_recovery(
                    path,
                    crate::history::HistorySource::FullScreen,
                );
            }
            if self.finish_history_write(history_write_generation) && result.is_ok() {
                self.synchronize_history_preview_cache();
                self.resume_history_thumbnail_queue(cx);
            }
            cx.notify();
            return;
        }
        let refresh_history_preview = result.is_ok();
        match result {
            Ok(path) => {
                let history_status = self.record_managed_save_with_recovery(
                    &path,
                    crate::history::HistorySource::FullScreen,
                );
                self.status = format!("Full screen saved to {}", path.display());
                if let Some(history_status) = history_status {
                    self.status.push_str(&history_status);
                }
                self.notify_user("Flash Shot", "Full screen saved");
            }
            Err(error) => {
                self.status = format!("Could not save full screen: {error}");
                log::warn!(target: "flash_shot::capture", "full_screen_save_failed error={error}");
            }
        }
        if self.finish_history_write(history_write_generation) && refresh_history_preview {
            self.synchronize_history_preview_cache();
            self.resume_history_thumbnail_queue(cx);
        }
        cx.notify();
    }

    pub(super) fn close_capture_overlays(&mut self, cx: &mut Context<Self>) {
        let windows = std::mem::take(&mut self.overlay_windows);
        if !windows.is_empty() {
            // Invalidate callbacks queued by the old windows before their native teardown runs.
            self.operation_generation = self.operation_generation.wrapping_add(1);
            // OCR, QR, and translation own their own generation so they can coexist with Copy.
            // Closing the source editor must invalidate that generation explicitly.
            self.invalidate_recognition();
            self.capture_teardown_windows =
                windows.iter().map(|window| window.window_id()).collect();
            self.capture_teardown_pending = true;
            cx.defer(move |cx| close_overlay_windows(windows, cx));
        }
    }

    pub(super) fn close_manual_scroll_window(&mut self, cx: &mut Context<Self>) {
        if let Some(window) = self.scroll_window.take() {
            cx.defer(move |cx| {
                let _ = window.update(cx, |_, window, _| window.remove_window());
            });
        }
    }
}

/// Allows Copy feedback only while both its source editor and recognition context remain current.
pub(super) const fn selection_copy_completion_can_report(
    task_operation_generation: u64,
    current_operation_generation: u64,
    task_recognition_generation: u64,
    current_recognition_generation: u64,
    session_state: CaptureSessionState,
) -> bool {
    task_operation_generation == current_operation_generation
        && task_recognition_generation == current_recognition_generation
        && matches!(session_state, CaptureSessionState::Selecting)
}

/// Applies exact-ID removal without probing a window while its native close is in progress.
fn remove_capture_overlay_by_id(
    windows: &mut Vec<gpui::WindowHandle<CaptureOverlay>>,
    closing_id: gpui::WindowId,
) -> bool {
    let previous_len = windows.len();
    windows.retain(|window| window.window_id() != closing_id);
    windows.len() != previous_len
}

/// Returns true only when the last window from one deferred native teardown has closed.
fn finish_capture_teardown(
    pending_windows: &mut std::collections::HashSet<gpui::WindowId>,
    closing_id: gpui::WindowId,
) -> bool {
    pending_windows.remove(&closing_id) && pending_windows.is_empty()
}

/// Claims the single managed pinned-save slot so concurrent Pin windows cannot race on history.
pub(super) fn claim_pinned_save_slot(in_flight: &mut bool) -> bool {
    if *in_flight {
        false
    } else {
        *in_flight = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{CaptureOverlay, finish_capture_teardown, remove_capture_overlay_by_id};
    use gpui::{WindowHandle, WindowId};
    use std::collections::HashSet;

    #[test]
    fn closing_one_capture_overlay_preserves_other_registered_displays() {
        let first_id = WindowId::from(11_u64);
        let closing_id = WindowId::from(12_u64);
        let third_id = WindowId::from(13_u64);
        let mut windows = vec![
            WindowHandle::<CaptureOverlay>::new(first_id),
            WindowHandle::<CaptureOverlay>::new(closing_id),
            WindowHandle::<CaptureOverlay>::new(third_id),
        ];

        assert!(remove_capture_overlay_by_id(&mut windows, closing_id));
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].window_id(), first_id);
        assert_eq!(windows[1].window_id(), third_id);
        assert!(!remove_capture_overlay_by_id(&mut windows, closing_id));
    }

    #[test]
    fn capture_teardown_is_pending_until_the_last_native_window_closes() {
        let first_id = WindowId::from(21_u64);
        let second_id = WindowId::from(22_u64);
        let mut pending = HashSet::from([first_id, second_id]);

        assert!(!finish_capture_teardown(&mut pending, first_id));
        assert!(!pending.is_empty());
        assert!(!finish_capture_teardown(&mut pending, first_id));
        assert!(finish_capture_teardown(&mut pending, second_id));
        assert!(pending.is_empty());
    }
}
