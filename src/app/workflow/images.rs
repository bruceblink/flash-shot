//! Image, project, and screenshot-history workflows.

use super::*;

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
                    .spawn(async move { CaptureFrame::open_png(&path) })
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
        result: std::io::Result<CaptureFrame>,
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
            Ok(frame) => self.open_pinned_frame(
                frame,
                "History image pinned in an always-on-top window",
                Some("Could not pin history image"),
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
        if !self.history_thumbnail_loading.insert(path.clone()) {
            return None;
        }
        let path = path.clone();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn({
                        let decode_path = path.clone();
                        async move {
                            let frame = CaptureFrame::open_png(&decode_path)?;
                            history_thumbnail_frame(&frame)
                        }
                    })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.finish_history_thumbnail(path, result, cx)
                    });
                }
            }
        })
        .detach();
        None
    }

    /// Stores a successfully decoded preview without surfacing transient list-rendering errors.
    fn finish_history_thumbnail(
        &mut self,
        path: PathBuf,
        result: std::io::Result<CaptureFrame>,
        cx: &mut Context<Self>,
    ) {
        self.history_thumbnail_loading.remove(&path);
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
