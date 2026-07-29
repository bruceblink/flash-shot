//! Saving, export, history mutation, and workflow cleanup.

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

    pub(in crate::app) fn quick_save_selection(&mut self, cx: &mut Context<Self>) {
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

    /// Saves an independent pinned frame and adds it to local history without reopening capture UI.
    pub(in crate::app) fn quick_save_pinned_frame(
        &mut self,
        frame: CaptureFrame,
        cx: &mut Context<Self>,
    ) {
        self.status = "Saving pinned image...".to_owned();
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move { quick_save_full_screen_frame(&frame) })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        match result {
                            Ok(path) => {
                                let history_note = this.history.record_with_source(path.clone(), crate::history::HistorySource::Pinned).err().map(|error| {
                                    log::warn!(target: "flash_shot::history", "pinned_save_history_record_failed error={error}");
                                    format!("; history unavailable: {error}")
                                });
                                this.status = format!("Pinned image saved to {}", path.display());
                                if let Some(history_note) = history_note {
                                    this.status.push_str(&history_note);
                                }
                                this.synchronize_history_preview_cache();
                                this.notify_user("Flash Shot", "Pinned image saved");
                            }
                            Err(error) => {
                                this.status = format!("Could not save pinned image: {error}");
                                log::warn!(target: "flash_shot::pinned", "pinned_save_failed error={error}");
                            }
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(in crate::app) fn save_annotation_document(&mut self, cx: &mut Context<Self>) {
        let Some(document) = self.annotation_document.clone() else {
            self.status = "Annotation document is unavailable".to_owned();
            cx.notify();
            return;
        };
        self.status = "Choose where to save annotations...".to_owned();
        cx.notify();
        let prompt =
            cx.prompt_for_new_path(&PathBuf::default(), Some("flash-shot.annotations.json"));
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = match prompt.await {
                    Ok(Ok(Some(path))) => {
                        let path = annotation_document_path(path);
                        cx.background_executor()
                            .spawn(async move {
                                save_annotation_document(&document, path.clone()).map(|()| path)
                            })
                            .await
                    }
                    Ok(Ok(None)) => return,
                    Ok(Err(error)) => Err(std::io::Error::other(error)),
                    Err(error) => Err(std::io::Error::other(error.to_string())),
                };
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.status = match result {
                            Ok(path) => format!("Annotations saved to {}", path.display()),
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
        self.status = "Choose where to save the editable image...".to_owned();
        cx.notify();
        let prompt = cx.prompt_for_new_path(&PathBuf::default(), Some("flash-shot-editable.png"));
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = match prompt.await {
                    Ok(Ok(Some(path))) => {
                        let path = png_path(path);
                        cx.background_executor()
                            .spawn(async move {
                                save_editable_project(&frame, &document, path.clone())
                                    .map(|()| path)
                            })
                            .await
                    }
                    Ok(Ok(None)) => return,
                    Ok(Err(error)) => Err(std::io::Error::other(error)),
                    Err(error) => Err(std::io::Error::other(error.to_string())),
                };
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.status = match result {
                            Ok(path) => format!(
                                "Editable project saved to {} and {}",
                                path.display(),
                                annotation_sidecar_path(&path).display()
                            ),
                            Err(error) => format!("Could not save editable project: {error}"),
                        };
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(in crate::app) fn open_annotation_document(&mut self, cx: &mut Context<Self>) {
        let Some(frame) = self.frame.as_ref() else {
            self.status = "Capture frame is unavailable".to_owned();
            cx.notify();
            return;
        };
        let bounds = frame.bounds;
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
                let result = match prompt.await {
                    Ok(Ok(Some(mut paths))) => match paths.pop() {
                        Some(path) => {
                            cx.background_executor()
                                .spawn(async move {
                                    load_annotation_document(&path, bounds)
                                        .map(|document| (path, document))
                                })
                                .await
                        }
                        None => return,
                    },
                    Ok(Ok(None)) => return,
                    Ok(Err(error)) => Err(std::io::Error::other(error)),
                    Err(error) => Err(std::io::Error::other(error.to_string())),
                };
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| match result {
                        Ok((path, document)) => {
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
                        Err(error) => {
                            this.status = format!("Could not open annotations: {error}");
                            cx.notify();
                        }
                    });
                }
            }
        })
        .detach();
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

    fn finish_save(&mut self, outcome: SaveOutcome, generation: u64, cx: &mut Context<Self>) {
        if !is_current_operation(self.operation_generation, generation) {
            return;
        }
        match outcome {
            SaveOutcome::Saved { path, managed } => {
                if let Err(error) = self.session.export_completed() {
                    self.status = error.to_string();
                } else {
                    let history_status = managed.then(|| self.history.record_with_source(path.clone(), crate::history::HistorySource::Selection)).transpose().err().map(|error| {
                        log::warn!(target: "flash_shot::history", "history_record_failed error={error}");
                        format!("; history unavailable: {error}")
                    });
                    self.status = format!("Selection saved to {}", path.display());
                    if let Some(history_status) = history_status {
                        self.status.push_str(&history_status);
                    }
                    self.synchronize_history_preview_cache();
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
        cx.notify();
    }

    pub(in crate::app) fn clear_history(&mut self, cx: &mut Context<Self>) {
        if self.history_clear_in_flight {
            return;
        }
        if !self.history_clear_confirmation {
            self.request_history_clear(cx);
            return;
        }
        let snapshot = self.history.clone();
        self.history_clear_in_flight = true;
        self.history_clear_confirmation = false;
        self.status = format!("Clearing {} saved capture(s)...", snapshot.entries().len());
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move { snapshot.delete_snapshot_files() })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| this.finish_clear_history(result, cx));
                }
            }
        })
        .detach();
    }

    fn finish_clear_history(
        &mut self,
        deletion: crate::history::HistoryFileDeletion,
        cx: &mut Context<Self>,
    ) {
        self.history_clear_in_flight = false;
        let deleted_count = deletion.deleted.len();
        let failure_count = deletion.failures.len();
        for (path, error) in &deletion.failures {
            log::warn!(target: "flash_shot::history", "history_file_delete_failed path={} error={error}", path.display());
        }
        self.status = match self.history.forget_deleted(&deletion.deleted) {
            Ok(()) if failure_count == 0 => "Screenshot history cleared".to_owned(),
            Ok(()) => {
                format!("Cleared {deleted_count} capture(s); {failure_count} could not be deleted")
            }
            Err(error) => {
                log::warn!(target: "flash_shot::history", "history_index_update_failed error={error}");
                format!("Capture files were cleared, but history could not be updated: {error}")
            }
        };
        self.history_expanded = false;
        self.synchronize_history_preview_cache();
        cx.notify();
    }

    pub(in crate::app) fn remove_history_image(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        match self.history.remove(&path) {
            Ok(true) => self.status = format!("Removed {} from screenshot history", path.display()),
            Ok(false) => {
                self.status = format!("Removed missing {} from screenshot history", path.display())
            }
            Err(error) => {
                self.status = format!("Could not remove screenshot history item: {error}");
                log::warn!(target: "flash_shot::history", "history_remove_failed path={} error={error}", path.display());
            }
        }
        self.history_thumbnails.remove(&path);
        self.history_thumbnail_loading.remove(&path);
        self.history_thumbnail_failed.remove(&path);
        cx.notify();
    }

    /// Drops decoded previews and retry state as soon as retention removes their history entries.
    pub(super) fn synchronize_history_preview_cache(&mut self) {
        let retained = self
            .history
            .entries()
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<std::collections::HashSet<_>>();
        self.history_thumbnails
            .retain(|path, _| retained.contains(path));
        self.history_thumbnail_loading
            .retain(|path| retained.contains(path));
        self.history_thumbnail_failed
            .retain(|path| retained.contains(path));
    }

    pub(super) fn finish_copy(
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
                    self.notify_user("Flash Shot", "Screenshot copied to clipboard");
                    self.close_capture_overlays(cx);
                    self.return_to_background();
                }
            }
            Err(error) => {
                let message = format!("Copy failed: {error}");
                let _ = self.session.fail(message.clone());
                self.status = message;
                self.close_capture_overlays(cx);
                self.return_to_background();
            }
        }
        cx.notify();
    }

    pub(super) fn finish_full_screen_copy(
        &mut self,
        result: std::io::Result<()>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if !claim_idle_completion(
            &mut self.full_screen_copy_generation,
            self.operation_generation,
            generation,
            self.session.state(),
        ) {
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
        result: std::io::Result<CaptureFrame>,
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
            Ok(frame) => self.open_pinned_frame(
                frame,
                "Full screen pinned in an always-on-top window",
                Some("Could not pin full screen"),
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
        cx: &mut Context<Self>,
    ) {
        if !claim_idle_completion(
            &mut self.full_screen_save_generation,
            self.operation_generation,
            generation,
            self.session.state(),
        ) {
            return;
        }
        match result {
            Ok(path) => {
                let history_status = self.history.record_with_source(path.clone(), crate::history::HistorySource::FullScreen).err().map(|error| {
                    log::warn!(target: "flash_shot::history", "history_record_failed error={error}");
                    format!("; history unavailable: {error}")
                });
                self.status = format!("Full screen saved to {}", path.display());
                if let Some(history_status) = history_status {
                    self.status.push_str(&history_status);
                }
                self.synchronize_history_preview_cache();
                self.notify_user("Flash Shot", "Full screen saved");
            }
            Err(error) => {
                self.status = format!("Could not save full screen: {error}");
                log::warn!(target: "flash_shot::capture", "full_screen_save_failed error={error}");
            }
        }
        cx.notify();
    }

    pub(super) fn close_capture_overlays(&mut self, cx: &mut Context<Self>) {
        let windows = std::mem::take(&mut self.overlay_windows);
        if !windows.is_empty() {
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
