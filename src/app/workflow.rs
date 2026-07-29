//! Capture, selection, and clipboard workflow orchestration.

mod annotation;
mod capture;
mod images;
mod pinning;
mod recording;
mod scrolling;
mod settings;

#[cfg(test)]
use recording::{
    format_recording_progress, next_recording_audio_selection, next_recording_display_selection,
    recording_start_failure_status, recording_support_status, recording_target_label,
};
pub(super) use recording::{recording_audio_selection_label, recording_display_selection_label};

use std::{
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use gpui::{
    AppContext, AsyncApp, Bounds, Context, DisplayId, Focusable, KeyDownEvent, Keystroke,
    PathPromptOptions, Pixels, RenderImage, WeakEntity, WindowBackgroundAppearance, WindowBounds,
    WindowKind, WindowOptions, point, px, size,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::{
    FlashShotApp, RecognitionResult, RecordingAudioSelection, RecordingDisplaySelection,
    SettingsSection,
    overlay::CaptureOverlay,
    pinned::PinnedImage,
    render_image::{history_thumbnail_frame, render_image_from_capture},
    scroll_control::ManualScrollControl,
};
use crate::{
    domain::{
        annotation::{
            Annotation, AnnotationCommand, AnnotationDocument, AnnotationId, AnnotationKind,
            AnnotationTool,
        },
        geometry::{PhysicalPoint, PhysicalRect},
        session::CaptureSessionState,
    },
    performance::CapturePipelineSample,
    platform::{
        autostart::{AutoStartService, AutoStartState, SystemAutoStart},
        capture::{
            CaptureBackend, CaptureFrame, CaptureOptions, DisplayCapture, SystemCaptureBackend,
            capture_displays_with_options, compose_virtual_desktop,
        },
        clipboard::{ClipboardService, SystemClipboard},
        directory,
        display::{DisplayProvider, SystemDisplayProvider},
        shortcut::GlobalShortcutService,
        window_inspector::{
            InspectionKind, InspectionTarget, SystemWindowInspector, WindowInspector,
        },
        window_visibility,
    },
    recording::{
        AudioSource, RecordingAudioConfig, RecordingEvent, RecordingProgress, RecordingRequest,
        RecordingTarget, discover, discover_audio_sources, start_recording,
    },
    settings::UserSettings,
    update::{UpdateAvailability, UpdateConfig},
};

impl FlashShotApp {
    /// Persists one of the supported delayed-capture choices selected in settings.
    pub(super) fn set_capture_delay(&mut self, delay_seconds: u8, cx: &mut Context<Self>) {
        let next_delay = UserSettings::normalize_capture_delay(delay_seconds);
        if next_delay != delay_seconds || self.capture_delay_seconds == next_delay {
            return;
        }
        let previous_delay = self.capture_delay_seconds;
        self.capture_delay_seconds = next_delay;
        self.settings.capture_delay_seconds = next_delay;
        if let Err(error) = self.settings.save(&self.settings_path) {
            self.capture_delay_seconds = previous_delay;
            self.settings.capture_delay_seconds = previous_delay;
            self.status = format!("Could not save capture delay: {error}");
            cx.notify();
            return;
        }
        self.status = if self.capture_delay_seconds == 0 {
            "Capture delay disabled".to_owned()
        } else {
            format!(
                "Capture delay set to {} seconds",
                self.capture_delay_seconds
            )
        };
        cx.notify();
    }

    pub(super) fn toggle_capture_cursor(&mut self, cx: &mut Context<Self>) {
        let previous_value = self.include_cursor;
        self.include_cursor = !previous_value;
        self.settings.include_cursor = self.include_cursor;
        if let Err(error) = self.settings.save(&self.settings_path) {
            self.include_cursor = previous_value;
            self.settings.include_cursor = previous_value;
            self.status = format!("Could not save cursor preference: {error}");
            cx.notify();
            return;
        }
        self.set_tray_capture_cursor_enabled(self.include_cursor);
        self.status = if self.include_cursor {
            "Capture will include the system cursor".to_owned()
        } else {
            "Capture will omit the system cursor".to_owned()
        };
        cx.notify();
    }

    pub(super) fn cycle_history_limit(&mut self, cx: &mut Context<Self>) {
        let previous_limit = self.settings.history_limit;
        let next_limit = next_history_limit(previous_limit);
        if let Err(error) = self.history.set_limit(usize::from(next_limit)) {
            self.status = format!("Could not update screenshot history retention: {error}");
            cx.notify();
            return;
        }
        self.settings.history_limit = next_limit;
        if let Err(error) = self.settings.save(&self.settings_path) {
            self.status = format!(
                "History retention is {} captures for this session but could not be saved: {error}",
                next_limit
            );
            cx.notify();
            return;
        }
        self.status = format!("Screenshot history retains the latest {next_limit} captures");
        cx.notify();
    }

    pub(super) fn copy_recognition_result(&mut self, cx: &mut Context<Self>) {
        let Some(result) = self.recognition_result.as_ref() else {
            return;
        };
        self.status = match SystemClipboard.copy_text(&result.text) {
            Ok(()) => format!("{} copied to clipboard", result.title),
            Err(error) => format!("Could not copy {}: {error}", result.title),
        };
        cx.notify();
    }

    /// Copies the current physical-pixel sample without changing the capture or annotation state.
    pub(super) fn copy_hover_color(&mut self, cx: &mut Context<Self>) {
        let format = ColorFormat::from_setting(self.settings.color_format);
        let Some(color) = hovered_color(self.frame.as_ref(), self.hover_pixel, format) else {
            self.status = "Move over the captured image to copy a color".to_owned();
            cx.notify();
            return;
        };
        self.status = match SystemClipboard.copy_text(&color) {
            Ok(()) => format!("{color} copied to clipboard"),
            Err(error) => format!("Could not copy {color}: {error}"),
        };
        cx.notify();
    }

    /// Cycles the saved output syntax used by the overlay's pixel color copy action.
    pub(super) fn cycle_color_format(&mut self, cx: &mut Context<Self>) {
        let previous = self.settings.color_format;
        let next = ColorFormat::from_setting(previous).next();
        self.settings.color_format = next.setting_value();
        if let Err(error) = self.settings.save(&self.settings_path) {
            self.settings.color_format = previous;
            self.status = format!("Could not save color format preference: {error}");
            cx.notify();
            return;
        }
        self.status = format!("Color copy format: {}", next.label());
        cx.notify();
    }

    pub(super) fn color_format_label(&self) -> &'static str {
        ColorFormat::from_setting(self.settings.color_format).label()
    }

    pub(super) fn clear_recognition_result(&mut self, cx: &mut Context<Self>) {
        self.recognition_result = None;
        cx.notify();
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
            Ok(codes) => {
                let code_count = codes.len();
                self.recognition_result = Some(RecognitionResult {
                    title: if code_count == 1 {
                        "QR code"
                    } else {
                        "QR codes"
                    }
                    .to_owned(),
                    text: codes.join("\n"),
                });
                format!("Found {code_count} QR code(s)")
            }
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

        let ocr_language = self.settings.ocr_language.clone();
        self.status = format!(
            "Recognizing text locally ({})...",
            ocr_language_label(ocr_language.as_deref())
        );
        let generation = self.operation_generation;
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let frame = frame.composite_annotations(&document)?.crop(selection)?;
                        crate::ocr::recognize_with_language(&frame, ocr_language.as_deref())
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
            Ok(text) => {
                self.recognition_result = Some(RecognitionResult {
                    title: "Recognized text".to_owned(),
                    text,
                });
                "Text recognized locally".to_owned()
            }
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
        let ocr_language = self.settings.ocr_language.clone();

        self.status = "Recognizing and translating text...".to_owned();
        let generation = self.operation_generation;
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        translate_selected_frame(frame, document, selection, config, ocr_language)
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
        result: TranslationOutcome,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if !is_current_operation(self.operation_generation, generation) {
            return;
        }
        self.status = match result {
            TranslationOutcome::Completed(text) if text.is_empty() => {
                "No text found in the selection".to_owned()
            }
            TranslationOutcome::Completed(text) => {
                self.recognition_result = Some(RecognitionResult {
                    title: "Translation".to_owned(),
                    text,
                });
                "Translation completed".to_owned()
            }
            TranslationOutcome::PreparationFailed(error) => {
                log::warn!(target: "flash_shot::translation", "translation_preparation_failed error={error}");
                translation_failure_status(&TranslationOutcome::PreparationFailed(error))
            }
            TranslationOutcome::OcrUnavailable => {
                translation_failure_status(&TranslationOutcome::OcrUnavailable)
            }
            TranslationOutcome::OcrFailed(error) => {
                log::warn!(target: "flash_shot::translation", "translation_ocr_failed error={error}");
                translation_failure_status(&TranslationOutcome::OcrFailed(error))
            }
            TranslationOutcome::ServiceFailed(error) => {
                log::warn!(target: "flash_shot::translation", "translation_service_failed error={error}");
                translation_failure_status(&TranslationOutcome::ServiceFailed(error))
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

    /// Saves an independent pinned frame and adds it to local history without reopening capture UI.
    pub(super) fn quick_save_pinned_frame(&mut self, frame: CaptureFrame, cx: &mut Context<Self>) {
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

    pub(super) fn save_annotation_document(&mut self, cx: &mut Context<Self>) {
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

    pub(super) fn save_editable_project(&mut self, cx: &mut Context<Self>) {
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

    pub(super) fn open_annotation_document(&mut self, cx: &mut Context<Self>) {
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
                    let history_status = managed.then(|| self.history.record_with_source(path.clone(), crate::history::HistorySource::Selection)).transpose().err().map(|error| {
                        log::warn!(target: "flash_shot::history", "history_record_failed error={error}");
                        format!("; history unavailable: {error}")
                    });
                    self.status = format!("Selection saved to {}", path.display());
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
        cx.notify();
    }

    pub(super) fn clear_history(&mut self, cx: &mut Context<Self>) {
        if !self.history_clear_confirmation {
            self.request_history_clear(cx);
            return;
        }
        match self.history.clear() {
            Ok(()) => self.status = "Screenshot history cleared".to_owned(),
            Err(error) => {
                self.status = format!("Could not clear screenshot history: {error}");
                log::warn!(target: "flash_shot::history", "history_clear_failed error={error}");
            }
        }
        self.history_thumbnails.clear();
        self.history_thumbnail_loading.clear();
        self.history_expanded = false;
        self.history_clear_confirmation = false;
        cx.notify();
    }

    pub(super) fn remove_history_image(&mut self, path: PathBuf, cx: &mut Context<Self>) {
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

    fn finish_full_screen_copy(
        &mut self,
        result: std::io::Result<()>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if !full_screen_copy_is_current(
            self.full_screen_copy_generation,
            self.operation_generation,
            generation,
            self.session.state(),
        ) {
            return;
        }
        self.full_screen_copy_generation = None;
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
    fn finish_full_screen_pin(
        &mut self,
        result: std::io::Result<CaptureFrame>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if !full_screen_pin_is_current(
            self.full_screen_pin_generation,
            self.operation_generation,
            generation,
            self.session.state(),
        ) {
            return;
        }
        self.full_screen_pin_generation = None;
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
    fn finish_full_screen_save(
        &mut self,
        result: std::io::Result<PathBuf>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if self.full_screen_save_generation != Some(generation)
            || !is_current_operation(self.operation_generation, generation)
            || self.session.state() != CaptureSessionState::Idle
        {
            return;
        }
        self.full_screen_save_generation = None;
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
                self.notify_user("Flash Shot", "Full screen saved");
            }
            Err(error) => {
                self.status = format!("Could not save full screen: {error}");
                log::warn!(target: "flash_shot::capture", "full_screen_save_failed error={error}");
            }
        }
        cx.notify();
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

    pub(super) fn show_settings_window(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.settings_window_handle
            && let Err(error) = window_visibility::restore(handle)
        {
            self.status = format!("Could not open settings: {error}");
            log::warn!(target: "flash_shot::settings", "settings_window_restore_failed error={error}");
        }
        cx.notify();
    }

    pub(super) fn show_history_window(&mut self, cx: &mut Context<Self>) {
        self.select_settings_section(SettingsSection::Files, cx);
        self.show_settings_window(cx);
    }

    pub(super) fn open_history_directory(&mut self, cx: &mut Context<Self>) {
        self.status = match crate::history::managed_history_directory()
            .and_then(|path| directory::open(&path).map(|()| path))
        {
            Ok(path) => format!("Opened screenshot folder {}", path.display()),
            Err(error) => {
                log::warn!(target: "flash_shot::history", "history_directory_open_failed error={error}");
                format!("Could not open screenshot folder: {error}")
            }
        };
        cx.notify();
    }

    pub(crate) fn hide_settings_window(&mut self) {
        if let Some(handle) = self.settings_window_handle
            && let Err(error) = window_visibility::hide(handle)
        {
            log::warn!(target: "flash_shot::settings", "settings_window_hide_failed error={error}");
        }
    }

    pub(super) fn toggle_overlay_more_actions(&mut self, cx: &mut Context<Self>) {
        self.overlay_more_actions = !self.overlay_more_actions;
        cx.notify();
    }

    pub(super) fn toggle_overlay_annotation_controls(&mut self, cx: &mut Context<Self>) {
        self.overlay_annotation_controls = !self.overlay_annotation_controls;
        cx.notify();
    }

    fn return_to_background(&mut self) {
        self.hide_settings_window();
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

/// Confirms a history decode belongs to the latest pin request before opening a new window.
fn history_pin_is_current(
    active_generation: Option<u64>,
    current_generation: u64,
    completion_generation: u64,
    session_state: CaptureSessionState,
) -> bool {
    active_generation == Some(completion_generation)
        && is_current_operation(current_generation, completion_generation)
        && session_state == CaptureSessionState::Idle
}

fn full_screen_copy_is_current(
    active_generation: Option<u64>,
    current_generation: u64,
    completion_generation: u64,
    session_state: CaptureSessionState,
) -> bool {
    active_generation == Some(completion_generation)
        && is_current_operation(current_generation, completion_generation)
        && session_state == CaptureSessionState::Idle
}

/// Prevents a late full-screen pin capture from opening after another workflow takes over.
fn full_screen_pin_is_current(
    active_generation: Option<u64>,
    current_generation: u64,
    completion_generation: u64,
    session_state: CaptureSessionState,
) -> bool {
    active_generation == Some(completion_generation)
        && is_current_operation(current_generation, completion_generation)
        && session_state == CaptureSessionState::Idle
}

fn next_history_limit(current: u16) -> u16 {
    match current {
        10 => 30,
        30 => 100,
        100 => 300,
        _ => 10,
    }
}

fn delayed_capture_status(remaining_seconds: u8) -> String {
    format!("Capture scheduled in {remaining_seconds} seconds")
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
                    app.return_to_background();
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
                app.return_to_background();
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
                app.return_to_background();
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

/// Extracts the HWND of a GPUI window for the small set of native visibility controls.
fn native_window_handle(window: &gpui::Window) -> Option<isize> {
    HasWindowHandle::window_handle(window)
        .ok()
        .and_then(|handle| match handle.as_raw() {
            RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
            _ => None,
        })
}

struct CapturedDesktopPreview {
    capture: crate::platform::capture::VirtualDesktopCapture,
    workspace_preview: super::render_image::CaptureRenderImage,
    displays: Vec<CapturedDisplayPreview>,
    render_upload_copy_count: u32,
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

fn capture_virtual_desktop_preview(
    include_cursor: bool,
) -> std::io::Result<CapturedDesktopPreview> {
    let display_captures = capture_displays_with_options(CaptureOptions { include_cursor })?;
    let frame = compose_captured_displays(&display_captures)?;
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
    let workspace_preview = if displays.len() == 1 {
        // The main workspace and the only overlay show identical pixels. Reuse
        // the decoded image instead of allocating and uploading it a second time.
        super::render_image::CaptureRenderImage {
            image: displays[0].preview.clone(),
            upload_bytes: 0,
        }
    } else {
        render_image_from_capture(&frame)?
    };
    let render_upload_copy_count =
        displays.len() as u32 + u32::from(workspace_preview.upload_bytes != 0);
    Ok(CapturedDesktopPreview {
        capture: crate::platform::capture::VirtualDesktopCapture {
            display_count: displays.len(),
            frame,
        },
        workspace_preview,
        displays,
        render_upload_copy_count,
    })
}

fn capture_virtual_desktop_frame(include_cursor: bool) -> std::io::Result<CaptureFrame> {
    let display_captures = capture_displays_with_options(CaptureOptions { include_cursor })?;
    compose_captured_displays(&display_captures)
}

fn compose_captured_displays(display_captures: &[DisplayCapture]) -> std::io::Result<CaptureFrame> {
    match display_captures {
        [capture] => Ok(capture.frame.clone()),
        captures => compose_virtual_desktop(captures),
    }
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

/// Writes an unannotated full-screen frame to the same managed directory as quick-saved selections.
fn quick_save_full_screen_frame(frame: &CaptureFrame) -> std::io::Result<PathBuf> {
    let directory = quick_save_directory()?;
    quick_save_full_screen_frame_in(frame, &directory, unix_timestamp_ms())
}

/// Saves a full capture using the caller-provided directory and timestamp.
///
/// Keeping the path policy here lets the tray command share the collision-safe quick-save naming
/// scheme and allows the PNG output to be verified without depending on a user's Pictures folder.
fn quick_save_full_screen_frame_in(
    frame: &CaptureFrame,
    directory: &Path,
    timestamp_ms: u128,
) -> std::io::Result<PathBuf> {
    let path = next_quick_save_path(directory, timestamp_ms, Path::exists);
    frame.save_png(path.clone())?;
    Ok(path)
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

fn next_quick_save_path(
    directory: &Path,
    timestamp_ms: u128,
    exists: impl Fn(&Path) -> bool,
) -> PathBuf {
    let prefix = quick_save_prefix();
    next_quick_save_path_with_prefix(directory, &prefix, timestamp_ms, exists)
}

fn next_quick_save_path_with_prefix(
    directory: &Path,
    prefix: &str,
    timestamp_ms: u128,
    exists: impl Fn(&Path) -> bool,
) -> PathBuf {
    let stem = format!("{prefix}-{timestamp_ms}");
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

fn quick_save_prefix() -> String {
    std::env::var("FLASH_SHOT_SAVE_PREFIX")
        .ok()
        .map(|prefix| sanitize_save_prefix(&prefix))
        .filter(|prefix| !prefix.is_empty())
        .unwrap_or_else(|| "FlashShot".to_owned())
}

fn sanitize_save_prefix(prefix: &str) -> String {
    prefix
        .trim()
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        .take(48)
        .collect()
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

fn annotation_document_path(mut path: PathBuf) -> PathBuf {
    let is_json = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"));
    if !is_json {
        path.set_extension("annotations.json");
    }
    path
}

fn annotation_sidecar_path(image_path: &Path) -> PathBuf {
    image_path.with_extension("annotations.json")
}

fn save_annotation_document(document: &AnnotationDocument, path: PathBuf) -> std::io::Result<()> {
    let json = document.to_json().map_err(std::io::Error::other)?;
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    let mut file = std::fs::File::create(&temporary)?;
    use std::io::Write;
    file.write_all(json.as_bytes())?;
    file.sync_all()?;
    drop(file);
    crate::image::replace_file(&temporary, &path)
}

fn save_editable_project(
    frame: &CaptureFrame,
    document: &AnnotationDocument,
    image_path: PathBuf,
) -> std::io::Result<()> {
    let local_bounds = PhysicalRect {
        left: 0,
        top: 0,
        right: i32::try_from(frame.width).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "frame width overflow")
        })?,
        bottom: i32::try_from(frame.height).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "frame height overflow")
        })?,
    };
    let local_document = document
        .rebased_to(local_bounds)
        .map_err(std::io::Error::other)?;
    frame.save_png(&image_path)?;
    save_annotation_document(&local_document, annotation_sidecar_path(&image_path))
}

fn load_annotation_document(
    path: &Path,
    expected_canvas: PhysicalRect,
) -> std::io::Result<AnnotationDocument> {
    let json = std::fs::read_to_string(path)?;
    let document = AnnotationDocument::from_json(&json).map_err(std::io::Error::other)?;
    if document.canvas_bounds() != expected_canvas {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "annotation document canvas does not match the current screenshot",
        ));
    }
    Ok(document)
}

fn open_image_project(
    path: &Path,
) -> std::io::Result<(
    PathBuf,
    CaptureFrame,
    Option<AnnotationDocument>,
    Option<String>,
)> {
    let frame = CaptureFrame::open_png(path)?;
    let sidecar = annotation_sidecar_path(path);
    if !sidecar.exists() {
        return Ok((path.to_owned(), frame, None, None));
    }
    match load_annotation_document(&sidecar, frame.bounds) {
        Ok(document) => Ok((path.to_owned(), frame, Some(document), None)),
        Err(error) => Ok((
            path.to_owned(),
            frame,
            None,
            Some(format!("could not load {}: {error}", sidecar.display())),
        )),
    }
}

fn open_annotation_project(
    path: &Path,
) -> std::io::Result<(PathBuf, CaptureFrame, AnnotationDocument)> {
    let image_path = project_image_path(path)?;
    let frame = CaptureFrame::open_png(&image_path)?;
    let document = load_annotation_document(path, frame.bounds)?;
    Ok((image_path, frame, document))
}

fn project_image_path(sidecar_path: &Path) -> std::io::Result<PathBuf> {
    let filename = sidecar_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "annotation project has no file name",
            )
        })?;
    let stem = filename.strip_suffix(".annotations.json").ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "annotation project file must end with .annotations.json",
        )
    })?;
    Ok(sidecar_path.with_file_name(format!("{stem}.png")))
}

fn next_annotation_counters(document: &AnnotationDocument) -> (u64, u32) {
    let next_id = document
        .annotations()
        .iter()
        .map(|annotation| annotation.id.value())
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let next_sequence = document
        .annotations()
        .iter()
        .filter_map(|annotation| match annotation.kind {
            AnnotationKind::Number { value, .. } => Some(value),
            _ => None,
        })
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    (next_id, next_sequence)
}

fn selection_status(selection: PhysicalRect) -> String {
    format!(
        "Selection: {} x {} physical pixels",
        selection.width(),
        selection.height()
    )
}

fn smart_target_status(target: InspectionTarget, point: PhysicalPoint, color: String) -> String {
    let kind = match target.kind {
        InspectionKind::Control => "Control",
        InspectionKind::Window => "Window",
    };
    format!(
        "{kind}: {} x {} px | ({}, {}) {color}",
        target.bounds.width(),
        target.bounds.height(),
        point.x,
        point.y,
    )
}

fn fill_color(stroke_rgba: u32) -> u32 {
    with_alpha(stroke_rgba, fill_alpha(stroke_rgba as u8))
}

fn pinned_size(image_width: f32, image_height: f32) -> gpui::Size<Pixels> {
    const HEADER_HEIGHT: f32 = 26.0;
    let width = image_width.max(1.0);
    let height = image_height.max(1.0);
    size(
        px(width.max(180.0)),
        px((height + HEADER_HEIGHT).max(140.0)),
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
            text_font_size: style.text_font_size,
        }
    } else {
        style
    }
}

fn text_annotation_with_content(annotation: Annotation, content: String) -> Option<Annotation> {
    let kind = match annotation.kind {
        AnnotationKind::Text { origin, .. } => AnnotationKind::Text { origin, content },
        AnnotationKind::Watermark { origin, .. } => AnnotationKind::Watermark { origin, content },
        _ => return None,
    };
    Some(Annotation {
        id: annotation.id,
        kind,
        style: annotation.style,
    })
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

/// Formats the exact RGB value at the active overlay pointer, if it still belongs to this frame.
fn hovered_color(
    frame: Option<&CaptureFrame>,
    hover_pixel: Option<PhysicalPoint>,
    format: ColorFormat,
) -> Option<String> {
    frame?
        .pixel_at(hover_pixel?)
        .map(|color| format.format(color))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ColorFormat {
    Hex,
    Rgb,
    Hsl,
}

impl ColorFormat {
    const fn from_setting(value: u8) -> Self {
        match value {
            1 => Self::Rgb,
            2 => Self::Hsl,
            _ => Self::Hex,
        }
    }

    const fn setting_value(self) -> u8 {
        match self {
            Self::Hex => 0,
            Self::Rgb => 1,
            Self::Hsl => 2,
        }
    }

    const fn next(self) -> Self {
        match self {
            Self::Hex => Self::Rgb,
            Self::Rgb => Self::Hsl,
            Self::Hsl => Self::Hex,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Hex => "HEX",
            Self::Rgb => "RGB",
            Self::Hsl => "HSL",
        }
    }

    fn format(self, color: crate::platform::capture::PixelColor) -> String {
        match self {
            Self::Hex => color.hex_rgb(),
            Self::Rgb => format!("rgb({}, {}, {})", color.red, color.green, color.blue),
            Self::Hsl => format_hsl(color.red, color.green, color.blue),
        }
    }
}

fn format_hsl(red: u8, green: u8, blue: u8) -> String {
    let red = f32::from(red) / 255.0;
    let green = f32::from(green) / 255.0;
    let blue = f32::from(blue) / 255.0;
    let minimum = red.min(green).min(blue);
    let maximum = red.max(green).max(blue);
    let lightness = (minimum + maximum) / 2.0;
    let delta = maximum - minimum;
    if delta == 0.0 {
        return format!("hsl(0, 0%, {:.1}%)", lightness * 100.0);
    }
    let saturation = delta / (1.0 - (2.0 * lightness - 1.0).abs());
    let hue = if maximum == red {
        60.0 * ((green - blue) / delta).rem_euclid(6.0)
    } else if maximum == green {
        60.0 * ((blue - red) / delta + 2.0)
    } else {
        60.0 * ((red - green) / delta + 4.0)
    };
    format!(
        "hsl({hue:.1}, {:.1}%, {:.1}%)",
        saturation * 100.0,
        lightness * 100.0
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum KeyboardCommand {
    Undo,
    Redo,
    Duplicate,
    BringForward,
    SendBackward,
    RotateClockwise,
    SelectNextAnnotation,
    SelectPreviousAnnotation,
    Delete,
    Cancel,
    Copy,
    QuickSave,
    CopyColor,
    MoveColorCursor(i32, i32),
    Nudge(i32, i32),
    SelectTool(Option<AnnotationTool>),
}

enum SaveOutcome {
    Saved { path: PathBuf, managed: bool },
    Cancelled,
    Failed(String),
}

enum OpenImageOutcome {
    Opened {
        path: PathBuf,
        frame: CaptureFrame,
        document: Option<AnnotationDocument>,
        document_warning: Option<String>,
    },
    Cancelled,
    Failed(String),
}

/// Separates local OCR and remote translation failures so the overlay can suggest the right fix.
enum TranslationOutcome {
    Completed(String),
    PreparationFailed(String),
    OcrUnavailable,
    OcrFailed(String),
    ServiceFailed(String),
}

/// Runs the selection pipeline outside the UI thread while retaining the failure stage.
fn translate_selected_frame(
    frame: CaptureFrame,
    document: AnnotationDocument,
    selection: PhysicalRect,
    config: crate::translation::TranslationConfig,
    ocr_language: Option<String>,
) -> TranslationOutcome {
    let frame = match frame
        .composite_annotations(&document)
        .and_then(|frame| frame.crop(selection))
    {
        Ok(frame) => frame,
        Err(error) => return TranslationOutcome::PreparationFailed(error.to_string()),
    };
    let text = match crate::ocr::recognize_with_language(&frame, ocr_language.as_deref()) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return TranslationOutcome::OcrUnavailable;
        }
        Err(error) => return TranslationOutcome::OcrFailed(error.to_string()),
    };
    match crate::translation::translate(&config, &text) {
        Ok(text) => TranslationOutcome::Completed(text),
        Err(error) => TranslationOutcome::ServiceFailed(error.to_string()),
    }
}

/// Formats the persisted OCR choice so status messages never expose a raw optional value.
pub(super) fn ocr_language_label(language: Option<&str>) -> &'static str {
    match language {
        None => "automatic",
        Some("eng") => "English",
        Some("chi_sim") => "Simplified Chinese",
        Some("eng+chi_sim") => "English + Simplified Chinese",
        Some(_) => "automatic",
    }
}

/// Turns a local OCR probe into a concise readiness result with a concrete recovery action.
fn ocr_support_status(result: Result<&crate::ocr::OcrSupport, &std::io::Error>) -> String {
    match result {
        Ok(support) if support.language_available() => format!(
            "Local OCR ready: {} with {}",
            support.version(),
            ocr_language_label(Some(support.language()))
        ),
        Ok(support) => format!(
            "Tesseract is installed but the {} language data is missing. Install that language pack or choose another OCR language.",
            ocr_language_label(Some(support.language()))
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            format!(
                "Local OCR is unavailable. Install Tesseract or set FLASH_SHOT_TESSERACT: {error}"
            )
        }
        Err(error) => format!("Could not check local OCR support: {error}"),
    }
}

/// Describes only local translation configuration so checking support never contacts a service.
fn translation_support_status(
    result: std::io::Result<Option<crate::translation::TranslationConfig>>,
) -> String {
    match result {
        Ok(Some(config)) => format!(
            "Translation ready: HTTPS endpoint configured for {}",
            config.target_language()
        ),
        Ok(None) => {
            "Translation is disabled. Set FLASH_SHOT_TRANSLATION_ENDPOINT to opt in.".to_owned()
        }
        Err(error) => format!("Translation configuration needs attention: {error}"),
    }
}

/// Turns each translation-stage failure into a recovery action instead of a generic error.
fn translation_failure_status(outcome: &TranslationOutcome) -> String {
    match outcome {
        TranslationOutcome::PreparationFailed(error) => {
            format!("Could not prepare the selection for translation: {error}")
        }
        TranslationOutcome::OcrUnavailable => {
            "Local OCR is unavailable. Install Tesseract or set FLASH_SHOT_TESSERACT.".to_owned()
        }
        TranslationOutcome::OcrFailed(error) => {
            format!("Could not recognize text for translation: {error}")
        }
        TranslationOutcome::ServiceFailed(error) => {
            format!("Translation service failed: {error}. Check the endpoint and try again.")
        }
        TranslationOutcome::Completed(_) => String::new(),
    }
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
    if modifiers.secondary()
        && !modifiers.alt
        && !modifiers.platform
        && !modifiers.function
        && keystroke.key == "d"
    {
        return Some(KeyboardCommand::Duplicate);
    }
    if modifiers.secondary()
        && modifiers.shift
        && !modifiers.alt
        && !modifiers.platform
        && !modifiers.function
        && keystroke.key == "]"
    {
        return Some(KeyboardCommand::BringForward);
    }
    if modifiers.secondary()
        && modifiers.shift
        && !modifiers.alt
        && !modifiers.platform
        && !modifiers.function
        && keystroke.key == "["
    {
        return Some(KeyboardCommand::SendBackward);
    }
    if modifiers.secondary()
        && modifiers.shift
        && !modifiers.alt
        && !modifiers.platform
        && !modifiers.function
        && keystroke.key == "r"
    {
        return Some(KeyboardCommand::RotateClockwise);
    }
    if modifiers.control && !modifiers.alt && !modifiers.platform && !modifiers.function {
        return match keystroke.key.as_str() {
            "left" => Some(KeyboardCommand::MoveColorCursor(-1, 0)),
            "right" => Some(KeyboardCommand::MoveColorCursor(1, 0)),
            "up" => Some(KeyboardCommand::MoveColorCursor(0, -1)),
            "down" => Some(KeyboardCommand::MoveColorCursor(0, 1)),
            _ => None,
        };
    }
    if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
        return None;
    }
    match keystroke.key.as_str() {
        "a" => Some(KeyboardCommand::SelectTool(Some(AnnotationTool::Arrow))),
        "b" => Some(KeyboardCommand::SelectTool(Some(AnnotationTool::Blur))),
        "c" => Some(KeyboardCommand::CopyColor),
        "e" => Some(KeyboardCommand::SelectTool(Some(AnnotationTool::Ellipse))),
        "h" => Some(KeyboardCommand::SelectTool(Some(AnnotationTool::Highlight))),
        "l" => Some(KeyboardCommand::SelectTool(Some(AnnotationTool::Line))),
        "m" => Some(KeyboardCommand::SelectTool(Some(AnnotationTool::Mosaic))),
        "n" => Some(KeyboardCommand::SelectTool(Some(AnnotationTool::Number))),
        "p" => Some(KeyboardCommand::SelectTool(Some(AnnotationTool::Freehand))),
        "r" => Some(KeyboardCommand::SelectTool(Some(AnnotationTool::Rectangle))),
        "s" => Some(KeyboardCommand::SelectTool(None)),
        "t" => Some(KeyboardCommand::SelectTool(Some(AnnotationTool::Text))),
        "w" => Some(KeyboardCommand::SelectTool(Some(AnnotationTool::Watermark))),
        "tab" if modifiers.shift => Some(KeyboardCommand::SelectPreviousAnnotation),
        "tab" => Some(KeyboardCommand::SelectNextAnnotation),
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

fn next_annotation_selection(
    annotations: &[AnnotationId],
    selected: Option<AnnotationId>,
    reverse: bool,
) -> Option<AnnotationId> {
    let len = annotations.len();
    let current = selected.and_then(|id| annotations.iter().position(|candidate| *candidate == id));
    let index = match (current, reverse) {
        (Some(index), false) => (index + 1) % len,
        (Some(0), true) => len - 1,
        (Some(index), true) => index - 1,
        (None, false) => 0,
        (None, true) => len - 1,
    };
    annotations.get(index).copied()
}

fn annotation_position(annotations: &[AnnotationId], selected: AnnotationId) -> usize {
    annotations
        .iter()
        .position(|candidate| *candidate == selected)
        .map_or(0, |index| index + 1)
}

fn adjusted_number_value(value: u32, delta: i32) -> u32 {
    i64::from(value)
        .saturating_add(i64::from(delta))
        .clamp(1, i64::from(u32::MAX)) as u32
}

#[cfg(test)]
mod tests {
    use super::{
        ColorFormat, KeyboardCommand, TranslationOutcome, adjusted_number_value,
        annotation_added_status, annotation_cancelled_status, annotation_document_path,
        annotation_position, annotation_sidecar_path, compose_captured_displays,
        copy_annotated_frame_selection, delayed_capture_status, drawing_status, fill_alpha,
        fill_color, format_hsl, format_recording_progress, full_screen_copy_is_current,
        full_screen_pin_is_current, history_pin_is_current, hovered_color, intersect_rect,
        is_current_operation, keyboard_command, load_annotation_document, next_annotation_counters,
        next_annotation_selection, next_quick_save_path, next_quick_save_path_with_prefix,
        next_recording_audio_selection, next_recording_display_selection, ocr_language_label,
        ocr_support_status, open_annotation_project, open_image_project, pinned_size, png_path,
        project_image_path, quick_save_annotated_frame_selection_in,
        quick_save_full_screen_frame_in, recording_audio_selection_label,
        recording_display_selection_label, recording_start_failure_status,
        recording_support_status, recording_target_label, resolve_pointer_selection,
        sanitize_save_prefix, save_annotated_frame_selection, save_annotation_document,
        save_editable_project, smart_target_status, style_for_tool, text_annotation_with_content,
        tool_selected_status, translation_failure_status, translation_support_status, with_alpha,
    };
    use crate::{
        domain::{
            annotation::{
                Annotation, AnnotationCommand, AnnotationDocument, AnnotationId, AnnotationKind,
                AnnotationStyle, AnnotationTool, CommandHistory,
            },
            geometry::{PhysicalPoint, PhysicalRect},
            session::CaptureSessionState,
        },
        platform::{
            capture::{CaptureFrame, DisplayCapture, PixelFormat},
            clipboard::ClipboardService,
            display::{DisplayInfo, DisplayRotation},
            window_inspector::{InspectionKind, InspectionTarget},
        },
        recording::AudioSource,
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

        fn copy_text(&self, _text: &str) -> io::Result<()> {
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
                        text_font_size: 24,
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
    fn hovered_color_uses_the_frame_physical_coordinates_and_rejects_missing_samples() {
        let frame = CaptureFrame {
            bounds: PhysicalRect {
                left: -2,
                top: 10,
                right: 0,
                bottom: 11,
            },
            width: 2,
            height: 1,
            stride: 8,
            format: PixelFormat::Bgra8,
            pixels: Arc::from([5, 171, 18, 255, 7, 8, 9, 255]),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 1,
        };

        assert_eq!(
            hovered_color(
                Some(&frame),
                Some(PhysicalPoint { x: -2, y: 10 }),
                ColorFormat::Hex
            ),
            Some("#12AB05".to_owned())
        );
        assert_eq!(hovered_color(Some(&frame), None, ColorFormat::Hex), None);
        assert_eq!(
            hovered_color(
                Some(&frame),
                Some(PhysicalPoint { x: 0, y: 10 }),
                ColorFormat::Hex,
            ),
            None
        );
    }

    #[test]
    fn color_copy_formats_are_stable_and_cycle_through_all_supported_syntaxes() {
        let color = crate::platform::capture::PixelColor {
            red: 18,
            green: 171,
            blue: 5,
            alpha: 255,
        };

        assert_eq!(ColorFormat::Hex.format(color), "#12AB05");
        assert_eq!(ColorFormat::Rgb.format(color), "rgb(18, 171, 5)");
        assert_eq!(ColorFormat::Hsl.format(color), "hsl(115.3, 94.3%, 34.5%)");
        assert_eq!(format_hsl(128, 128, 128), "hsl(0, 0%, 50.2%)");
        assert_eq!(ColorFormat::Hex.next(), ColorFormat::Rgb);
        assert_eq!(ColorFormat::Rgb.next(), ColorFormat::Hsl);
        assert_eq!(ColorFormat::Hsl.next(), ColorFormat::Hex);
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
            keyboard_command(&Keystroke::parse("c").unwrap()),
            Some(KeyboardCommand::CopyColor)
        );
        assert_eq!(keyboard_command(&Keystroke::parse("ctrl-c").unwrap()), None);
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
            keyboard_command(&Keystroke::parse("ctrl-left").unwrap()),
            Some(KeyboardCommand::MoveColorCursor(-1, 0))
        );
        assert_eq!(
            keyboard_command(&Keystroke::parse("ctrl-down").unwrap()),
            Some(KeyboardCommand::MoveColorCursor(0, 1))
        );
        assert_eq!(
            keyboard_command(&Keystroke::parse("ctrl-shift-left").unwrap()),
            Some(KeyboardCommand::MoveColorCursor(-1, 0))
        );
        assert_eq!(
            keyboard_command(&Keystroke::parse("ctrl-enter").unwrap()),
            None
        );
        assert_eq!(
            keyboard_command(&Keystroke::parse("ctrl-d").unwrap()),
            Some(KeyboardCommand::Duplicate)
        );
        assert_eq!(
            keyboard_command(&Keystroke::parse("ctrl-shift-]").unwrap()),
            Some(KeyboardCommand::BringForward)
        );
        assert_eq!(
            keyboard_command(&Keystroke::parse("ctrl-shift-[").unwrap()),
            Some(KeyboardCommand::SendBackward)
        );
        assert_eq!(
            keyboard_command(&Keystroke::parse("ctrl-shift-r").unwrap()),
            Some(KeyboardCommand::RotateClockwise)
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
        assert_eq!(
            keyboard_command(&Keystroke::parse("tab").unwrap()),
            Some(KeyboardCommand::SelectNextAnnotation)
        );
        assert_eq!(
            keyboard_command(&Keystroke::parse("shift-tab").unwrap()),
            Some(KeyboardCommand::SelectPreviousAnnotation)
        );
        assert_eq!(
            keyboard_command(&Keystroke::parse("r").unwrap()),
            Some(KeyboardCommand::SelectTool(Some(AnnotationTool::Rectangle)))
        );
        assert_eq!(
            keyboard_command(&Keystroke::parse("t").unwrap()),
            Some(KeyboardCommand::SelectTool(Some(AnnotationTool::Text)))
        );
        assert_eq!(
            keyboard_command(&Keystroke::parse("s").unwrap()),
            Some(KeyboardCommand::SelectTool(None))
        );
        assert_eq!(keyboard_command(&Keystroke::parse("ctrl-r").unwrap()), None);
    }

    #[test]
    fn annotation_selection_cycles_in_layer_order() {
        let annotations = [
            AnnotationId::new(1),
            AnnotationId::new(2),
            AnnotationId::new(3),
        ];

        assert_eq!(
            next_annotation_selection(&annotations, None, false),
            Some(AnnotationId::new(1))
        );
        assert_eq!(
            next_annotation_selection(&annotations, Some(AnnotationId::new(1)), false),
            Some(AnnotationId::new(2))
        );
        assert_eq!(
            next_annotation_selection(&annotations, Some(AnnotationId::new(3)), false),
            Some(AnnotationId::new(1))
        );
        assert_eq!(
            next_annotation_selection(&annotations, None, true),
            Some(AnnotationId::new(3))
        );
        assert_eq!(
            next_annotation_selection(&annotations, Some(AnnotationId::new(1)), true),
            Some(AnnotationId::new(3))
        );
        assert_eq!(annotation_position(&annotations, AnnotationId::new(2)), 2);
        assert_eq!(next_annotation_selection(&[], None, false), None);
    }

    #[test]
    fn number_marker_adjustment_clamps_to_the_supported_range() {
        assert_eq!(adjusted_number_value(7, 2), 9);
        assert_eq!(adjusted_number_value(1, -1), 1);
        assert_eq!(adjusted_number_value(u32::MAX, 1), u32::MAX);
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
    fn text_edit_replaces_content_without_changing_annotation_identity_or_style() {
        let style = AnnotationStyle {
            stroke_rgba: 0xFFCC00FF,
            fill_rgba: None,
            stroke_width: 6,
            text_font_size: 24,
        };
        let text = Annotation {
            id: AnnotationId::new(7),
            kind: AnnotationKind::Text {
                origin: PhysicalPoint { x: 12, y: 16 },
                content: "Before".to_owned(),
            },
            style,
        };
        let watermark = Annotation {
            id: AnnotationId::new(8),
            kind: AnnotationKind::Watermark {
                origin: PhysicalPoint { x: 20, y: 24 },
                content: "Old mark".to_owned(),
            },
            style,
        };

        assert_eq!(
            text_annotation_with_content(text.clone(), "After".to_owned()).unwrap(),
            Annotation {
                id: text.id,
                kind: AnnotationKind::Text {
                    origin: PhysicalPoint { x: 12, y: 16 },
                    content: "After".to_owned(),
                },
                style,
            }
        );
        assert_eq!(
            text_annotation_with_content(watermark, "New mark".to_owned())
                .unwrap()
                .kind,
            AnnotationKind::Watermark {
                origin: PhysicalPoint { x: 20, y: 24 },
                content: "New mark".to_owned(),
            }
        );
        assert!(
            text_annotation_with_content(
                Annotation {
                    id: AnnotationId::new(9),
                    kind: AnnotationKind::Rectangle {
                        bounds: PhysicalRect {
                            left: 0,
                            top: 0,
                            right: 10,
                            bottom: 10,
                        },
                    },
                    style,
                },
                "not text".to_owned(),
            )
            .is_none()
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
                    text_font_size: 24,
                },
            ),
            AnnotationStyle {
                stroke_rgba: 0xFFCC0066,
                fill_rgba: None,
                stroke_width: 1,
                text_font_size: 24,
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
    fn pinned_window_size_matches_the_image_without_downscaling() {
        let small = pinned_size(100.0, 80.0);
        assert_eq!(f32::from(small.width), 180.0);
        assert_eq!(f32::from(small.height), 140.0);

        let large = pinned_size(1_280.0, 720.0);
        assert_eq!(f32::from(large.width), 1_280.0);
        assert_eq!(f32::from(large.height), 746.0);
    }

    #[test]
    fn delayed_capture_status_reports_each_remaining_second() {
        let remaining = (1..=10)
            .rev()
            .map(delayed_capture_status)
            .collect::<Vec<_>>();

        assert_eq!(
            remaining.first().unwrap(),
            "Capture scheduled in 10 seconds"
        );
        assert_eq!(remaining.last().unwrap(), "Capture scheduled in 1 seconds");
        assert_eq!(remaining.len(), 10);
    }

    #[test]
    fn full_screen_copy_completion_does_not_override_a_new_capture_session() {
        assert!(full_screen_copy_is_current(
            Some(12),
            12,
            12,
            CaptureSessionState::Idle
        ));
        assert!(!full_screen_copy_is_current(
            Some(12),
            13,
            12,
            CaptureSessionState::Idle
        ));
        assert!(!full_screen_copy_is_current(
            Some(13),
            13,
            12,
            CaptureSessionState::Idle
        ));
        assert!(!full_screen_copy_is_current(
            Some(12),
            12,
            12,
            CaptureSessionState::Capturing
        ));
    }

    #[test]
    fn full_screen_pin_completion_does_not_open_after_a_new_operation() {
        assert!(full_screen_pin_is_current(
            Some(12),
            12,
            12,
            CaptureSessionState::Idle
        ));
        assert!(!full_screen_pin_is_current(
            Some(12),
            13,
            12,
            CaptureSessionState::Idle
        ));
        assert!(!full_screen_pin_is_current(
            Some(12),
            12,
            12,
            CaptureSessionState::Capturing
        ));
    }

    #[test]
    fn captured_display_composition_reuses_one_frame_without_an_extra_copy() {
        let bounds = PhysicalRect {
            left: 0,
            top: 0,
            right: 2,
            bottom: 1,
        };
        let frame = CaptureFrame {
            bounds,
            width: 2,
            height: 1,
            stride: 8,
            format: PixelFormat::Bgra8,
            pixels: Arc::from(vec![1, 2, 3, 255, 4, 5, 6, 255]),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 1,
        };
        let captures = [DisplayCapture {
            display: DisplayInfo {
                id: "primary".to_owned(),
                platform_id: 1,
                physical_bounds: bounds,
                work_area: bounds,
                dpi_x: 96,
                dpi_y: 96,
                scale_factor: 1.0,
                primary: true,
                rotation: DisplayRotation::Landscape,
                bits_per_pixel: 32,
            },
            frame: frame.clone(),
        }];

        assert_eq!(
            compose_captured_displays(&captures).unwrap().pixels,
            frame.pixels
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
                        text_font_size: 24,
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
    fn annotation_document_path_uses_a_json_extension() {
        assert_eq!(
            annotation_document_path(PathBuf::from("capture")),
            PathBuf::from("capture.annotations.json")
        );
        assert_eq!(
            annotation_document_path(PathBuf::from("capture.JSON")),
            PathBuf::from("capture.JSON")
        );
    }

    #[test]
    fn annotation_document_save_writes_valid_versioned_json() {
        let directory = std::env::temp_dir().join(format!(
            "flash-shot-annotation-document-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("capture.annotations.json");
        let document = AnnotationDocument::new(PhysicalRect {
            left: 0,
            top: 0,
            right: 10,
            bottom: 10,
        })
        .unwrap();

        save_annotation_document(&document, path.clone()).unwrap();
        assert_eq!(
            AnnotationDocument::from_json(&std::fs::read_to_string(&path).unwrap()).unwrap(),
            document
        );
        assert!(!path.with_extension("json.tmp").exists());
        std::fs::write(&path, "stale annotation document").unwrap();
        save_annotation_document(&document, path.clone()).unwrap();
        assert_eq!(
            AnnotationDocument::from_json(&std::fs::read_to_string(&path).unwrap()).unwrap(),
            document
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn editable_project_saves_original_png_and_rebased_annotation_sidecar() {
        let directory = std::env::temp_dir().join(format!(
            "flash-shot-editable-project-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let image_path = directory.join("capture.png");
        let frame = CaptureFrame {
            bounds: PhysicalRect {
                left: -10,
                top: 20,
                right: -8,
                bottom: 21,
            },
            width: 2,
            height: 1,
            stride: 8,
            format: PixelFormat::Bgra8,
            pixels: Arc::from([1, 2, 3, 255, 4, 5, 6, 255]),
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
                        start: PhysicalPoint { x: -10, y: 20 },
                        end: PhysicalPoint { x: -8, y: 20 },
                    },
                    style: AnnotationStyle::default(),
                }),
            )
            .unwrap();

        save_editable_project(&frame, &document, image_path.clone()).unwrap();
        let reopened = CaptureFrame::open_png(&image_path).unwrap();
        assert_eq!(reopened.bounds.left, 0);
        assert_eq!(reopened.bounds.top, 0);
        assert_eq!((reopened.width, reopened.height), (2, 1));
        let sidecar = annotation_sidecar_path(&image_path);
        let loaded = load_annotation_document(&sidecar, reopened.bounds).unwrap();
        assert_eq!(
            loaded.annotation(AnnotationId::new(1)).unwrap().bounds(),
            PhysicalRect {
                left: 0,
                top: 0,
                right: 2,
                bottom: 0,
            }
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn open_image_project_restores_a_valid_sidecar_and_tolerates_a_bad_one() {
        let directory = std::env::temp_dir().join(format!(
            "flash-shot-open-project-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let image_path = directory.join("capture.png");
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
            pixels: Arc::from([0, 0, 0, 255, 0, 0, 0, 255]),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 1,
        };
        let document = AnnotationDocument::new(frame.bounds).unwrap();
        save_editable_project(&frame, &document, image_path.clone()).unwrap();

        let (_, _, loaded, warning) = open_image_project(&image_path).unwrap();
        assert_eq!(loaded, Some(document));
        assert_eq!(warning, None);

        std::fs::write(annotation_sidecar_path(&image_path), "not json").unwrap();
        let (_, _, loaded, warning) = open_image_project(&image_path).unwrap();
        assert_eq!(loaded, None);
        assert!(warning.unwrap().contains("could not load"));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn opening_annotation_project_requires_the_matching_png_and_sidecar_name() {
        let directory = std::env::temp_dir().join(format!(
            "flash-shot-open-annotation-project-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let image_path = directory.join("capture.png");
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
            pixels: Arc::from([0, 0, 0, 255, 0, 0, 0, 255]),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 1,
        };
        let document = AnnotationDocument::new(frame.bounds).unwrap();
        save_editable_project(&frame, &document, image_path.clone()).unwrap();
        let sidecar = annotation_sidecar_path(&image_path);

        assert_eq!(project_image_path(&sidecar).unwrap(), image_path);
        assert!(project_image_path(&directory.join("capture.json")).is_err());
        let (opened_path, opened_frame, opened_document) =
            open_annotation_project(&sidecar).unwrap();
        assert_eq!(opened_path, image_path);
        assert_eq!(opened_frame.bounds, document.canvas_bounds());
        assert_eq!(opened_document, document);

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn annotation_document_load_requires_the_current_frame_canvas() {
        let directory = std::env::temp_dir().join(format!(
            "flash-shot-annotation-load-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("capture.annotations.json");
        let document = AnnotationDocument::new(PhysicalRect {
            left: 0,
            top: 0,
            right: 10,
            bottom: 10,
        })
        .unwrap();
        save_annotation_document(&document, path.clone()).unwrap();

        assert_eq!(
            load_annotation_document(&path, document.canvas_bounds()).unwrap(),
            document
        );
        assert!(
            load_annotation_document(
                &path,
                PhysicalRect {
                    left: 0,
                    top: 0,
                    right: 11,
                    bottom: 10,
                }
            )
            .is_err()
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn loaded_annotation_counters_continue_existing_ids_and_sequence_numbers() {
        let mut document = AnnotationDocument::new(PhysicalRect {
            left: 0,
            top: 0,
            right: 20,
            bottom: 20,
        })
        .unwrap();
        let mut history = CommandHistory::default();
        history
            .apply(
                &mut document,
                AnnotationCommand::Insert(Annotation {
                    id: AnnotationId::new(8),
                    kind: AnnotationKind::Number {
                        center: PhysicalPoint { x: 10, y: 10 },
                        value: 3,
                    },
                    style: AnnotationStyle::default(),
                }),
            )
            .unwrap();
        assert_eq!(next_annotation_counters(&document), (9, 4));
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
    fn quick_save_prefix_is_safe_and_part_of_the_collision_resistant_name() {
        assert_eq!(
            sanitize_save_prefix("  My Report: Q3/2026  "),
            "MyReportQ32026"
        );
        assert_eq!(sanitize_save_prefix("___"), "___");
        assert_eq!(sanitize_save_prefix("<>:\\|?*"), "");

        let directory = PathBuf::from("Pictures").join("Flash Shot");
        assert_eq!(
            next_quick_save_path_with_prefix(&directory, "Release_Notes", 42, |_| false),
            directory.join("Release_Notes-42.png")
        );
    }

    #[test]
    fn recording_status_uses_ffmpeg_progress_without_exposing_process_output() {
        assert_eq!(
            format_recording_progress(
                "selected area",
                crate::recording::RecordingProgress {
                    output_time_us: Some(3_900_000),
                    frame: Some(117),
                    finished: false,
                }
            ),
            "Recording selected area: 3s, 117 frames"
        );
    }

    #[test]
    fn recording_start_failures_name_the_available_recovery_path() {
        let missing = std::io::Error::new(std::io::ErrorKind::NotFound, "ffmpeg.exe");
        assert!(recording_start_failure_status(&missing).contains("FLASH_SHOT_FFMPEG"));

        let unsupported =
            std::io::Error::new(std::io::ErrorKind::Unsupported, "ddagrab unavailable");
        assert!(recording_start_failure_status(&unsupported).contains("ddagrab or gdigrab"));
    }

    #[test]
    fn recording_support_probe_reuses_actionable_missing_ffmpeg_guidance() {
        let missing = std::io::Error::new(std::io::ErrorKind::NotFound, "ffmpeg.exe");

        assert!(recording_support_status(Err(&missing)).contains("FLASH_SHOT_FFMPEG"));
    }

    #[test]
    fn recording_status_identifies_each_capture_target() {
        assert_eq!(
            recording_target_label(&crate::recording::RecordingTarget::Display {
                bounds: PhysicalRect {
                    left: 0,
                    top: 0,
                    right: 1920,
                    bottom: 1080,
                },
            }),
            "display"
        );
        assert_eq!(
            recording_target_label(&crate::recording::RecordingTarget::Window {
                title: "Editor".to_owned(),
            }),
            "window"
        );
        assert_eq!(
            recording_target_label(&crate::recording::RecordingTarget::Region {
                bounds: PhysicalRect {
                    left: 10,
                    top: 10,
                    right: 100,
                    bottom: 100,
                },
            }),
            "selected area"
        );
    }

    #[test]
    fn recording_audio_selection_cycles_from_auto_to_off_then_local_sources() {
        let sources = [
            AudioSource::Microphone {
                device: "USB Mic".to_owned(),
            },
            AudioSource::SystemAudio {
                device: "default".to_owned(),
            },
        ];
        let off =
            next_recording_audio_selection(super::RecordingAudioSelection::Automatic, &sources);
        assert_eq!(off, super::RecordingAudioSelection::Disabled);
        let microphone = next_recording_audio_selection(off, &sources);
        assert_eq!(
            microphone,
            super::RecordingAudioSelection::Source(sources[0].clone())
        );
        assert_eq!(recording_audio_selection_label(&microphone), "mic: USB Mic");
        assert_eq!(
            next_recording_audio_selection(
                super::RecordingAudioSelection::Source(sources[1].clone()),
                &sources,
            ),
            super::RecordingAudioSelection::Automatic
        );
    }

    #[test]
    fn recording_display_selection_cycles_in_stable_primary_first_order() {
        let display = |id: &str, left, top, width, height, primary| DisplayInfo {
            id: id.to_owned(),
            platform_id: 0,
            physical_bounds: PhysicalRect {
                left,
                top,
                right: left + width,
                bottom: top + height,
            },
            work_area: PhysicalRect {
                left,
                top,
                right: left + width,
                bottom: top + height,
            },
            dpi_x: 96,
            dpi_y: 96,
            scale_factor: 1.0,
            rotation: DisplayRotation::Landscape,
            bits_per_pixel: 32,
            primary,
        };
        let displays = [
            display("secondary", -2560, -100, 2560, 1440, false),
            display("primary", 0, 0, 1920, 1080, true),
        ];
        let selected =
            next_recording_display_selection(super::RecordingDisplaySelection::Primary, &displays);
        assert_eq!(
            selected,
            super::RecordingDisplaySelection::Display {
                id: "primary".to_owned(),
                label: "1 (1920x1080)".to_owned(),
            }
        );
        let secondary = next_recording_display_selection(selected, &displays);
        assert_eq!(
            secondary,
            super::RecordingDisplaySelection::Display {
                id: "secondary".to_owned(),
                label: "2 (2560x1440)".to_owned(),
            }
        );
        assert_eq!(
            recording_display_selection_label(&secondary),
            "display 2 (2560x1440)"
        );
        assert_eq!(
            next_recording_display_selection(secondary, &displays),
            super::RecordingDisplaySelection::Primary
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
    fn full_screen_quick_save_writes_the_entire_png_with_the_managed_name() {
        let directory = std::env::temp_dir().join(format!(
            "flash-shot-full-screen-quick-save-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let frame = CaptureFrame {
            bounds: PhysicalRect {
                left: -1,
                top: 4,
                right: 1,
                bottom: 5,
            },
            width: 2,
            height: 1,
            stride: 8,
            format: PixelFormat::Bgra8,
            pixels: Arc::from([1, 2, 3, 255, 4, 5, 6, 255]),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 1,
        };

        let path = quick_save_full_screen_frame_in(&frame, &directory, 1_725_000_000_123).unwrap();

        assert_eq!(path, directory.join("FlashShot-1725000000123.png"));
        let decoder = png::Decoder::new(BufReader::new(std::fs::File::open(&path).unwrap()));
        let mut reader = decoder.read_info().unwrap();
        let mut output = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut output).unwrap();
        assert_eq!((info.width, info.height), (2, 1));
        assert_eq!(&output[..info.buffer_size()], &[3, 2, 1, 255, 6, 5, 4, 255]);
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
    fn smart_target_status_includes_target_kind_bounds_and_pixel_details() {
        let target = InspectionTarget {
            bounds: PhysicalRect {
                left: -200,
                top: 50,
                right: 300,
                bottom: 250,
            },
            kind: InspectionKind::Control,
        };

        assert_eq!(
            smart_target_status(target, PhysicalPoint { x: 12, y: 34 }, "#AABBCC".to_owned()),
            "Control: 500 x 200 px | (12, 34) #AABBCC"
        );
    }

    #[test]
    fn stale_background_completion_is_ignored_after_a_new_operation_starts() {
        assert!(is_current_operation(4, 4));
        assert!(!is_current_operation(5, 4));
    }

    #[test]
    fn translation_failure_messages_identify_the_recovery_step() {
        assert!(
            translation_failure_status(&TranslationOutcome::OcrUnavailable)
                .contains("Install Tesseract")
        );
        assert!(
            translation_failure_status(&TranslationOutcome::OcrFailed("bad image".to_owned()))
                .contains("recognize text")
        );
        assert!(
            translation_failure_status(&TranslationOutcome::ServiceFailed("timeout".to_owned()))
                .contains("Check the endpoint")
        );
    }

    #[test]
    fn translation_support_status_keeps_disabled_configuration_local_and_actionable() {
        assert!(translation_support_status(Ok(None)).contains("FLASH_SHOT_TRANSLATION_ENDPOINT"));

        let invalid = std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "translation endpoint must use HTTPS",
        );
        assert!(translation_support_status(Err(invalid)).contains("needs attention"));
    }

    #[test]
    fn ocr_language_labels_make_each_saved_preset_readable() {
        assert_eq!(ocr_language_label(None), "automatic");
        assert_eq!(ocr_language_label(Some("eng")), "English");
        assert_eq!(ocr_language_label(Some("chi_sim")), "Simplified Chinese");
        assert_eq!(
            ocr_language_label(Some("eng+chi_sim")),
            "English + Simplified Chinese"
        );
        assert_eq!(ocr_language_label(Some("unknown")), "automatic");
    }

    #[test]
    fn ocr_support_probe_names_the_local_installation_recovery_step() {
        let missing = std::io::Error::new(std::io::ErrorKind::NotFound, "tesseract.exe");

        assert!(ocr_support_status(Err(&missing)).contains("FLASH_SHOT_TESSERACT"));
    }

    #[test]
    fn only_the_latest_idle_history_pin_request_can_open_a_window() {
        assert!(history_pin_is_current(
            Some(7),
            7,
            7,
            CaptureSessionState::Idle
        ));
        assert!(!history_pin_is_current(
            Some(7),
            8,
            7,
            CaptureSessionState::Idle
        ));
        assert!(!history_pin_is_current(
            Some(7),
            7,
            7,
            CaptureSessionState::Selecting
        ));
    }
}
