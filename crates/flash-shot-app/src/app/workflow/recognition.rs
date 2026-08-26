//! QR, OCR, and translation workflows.

use super::*;
use crate::i18n::{Locale, UiText};

impl FlashShotApp {
    pub(in crate::app) fn recognize_qr_selection(&mut self, cx: &mut Context<Self>) {
        if let Some(status) =
            recognition_start_conflict_status(self.settings.locale, self.recognition_in_flight)
        {
            self.status = status;
            cx.notify();
            return;
        }
        let Some(selection) = self.session.selection() else {
            self.status = self
                .settings
                .locale
                .text(UiText::RecognitionSelectAreaQr)
                .to_owned();
            cx.notify();
            return;
        };
        let Some((frame, document)) = self.export_source() else {
            cx.notify();
            return;
        };

        let generation = self.begin_recognition_operation();
        self.status = self
            .settings
            .locale
            .text(UiText::RecognitionQrInProgress)
            .to_owned();
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
        if !recognition_completion_is_current(self.recognition_generation, generation) {
            return;
        }
        self.recognition_in_flight = false;
        let locale = self.settings.locale;
        self.status = match result {
            Ok(codes) if codes.is_empty() => locale.text(UiText::RecognitionQrNone).to_owned(),
            Ok(codes) => {
                let code_count = codes.len();
                self.recognition_result = Some(RecognitionResult {
                    title: if code_count == 1 {
                        locale.text(UiText::RecognitionQrCode)
                    } else {
                        locale.text(UiText::RecognitionQrCodes)
                    }
                    .to_owned(),
                    text: codes.join("\n"),
                });
                locale.format_template(
                    UiText::RecognitionQrFound,
                    &[("count", &code_count.to_string())],
                )
            }
            Err(error) => {
                log::warn!(target: "flash_shot::qr", "qr_recognition_failed error={error}");
                locale.format_template(
                    UiText::RecognitionQrFailed,
                    &[("error", &error.to_string())],
                )
            }
        };
        cx.notify();
    }

    pub(in crate::app) fn recognize_text_selection(&mut self, cx: &mut Context<Self>) {
        if let Some(status) =
            recognition_start_conflict_status(self.settings.locale, self.recognition_in_flight)
        {
            self.status = status;
            cx.notify();
            return;
        }
        let Some(selection) = self.session.selection() else {
            self.status = self
                .settings
                .locale
                .text(UiText::RecognitionSelectAreaText)
                .to_owned();
            cx.notify();
            return;
        };
        let Some((frame, document)) = self.export_source() else {
            cx.notify();
            return;
        };

        let ocr_language = self.settings.ocr_language.clone();
        let locale = self.settings.locale;
        let generation = self.begin_recognition_operation();
        self.status = locale.format_template(
            UiText::RecognitionTextInProgress,
            &[(
                "language",
                ocr_language_label(locale, ocr_language.as_deref()),
            )],
        );
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
        if !recognition_completion_is_current(self.recognition_generation, generation) {
            return;
        }
        self.recognition_in_flight = false;
        let locale = self.settings.locale;
        self.status = match result {
            Ok(text) if text.is_empty() => {
                self.recognition_retry = None;
                locale.text(UiText::RecognitionTextNone).to_owned()
            }
            Ok(text) => {
                self.recognition_retry = None;
                self.recognition_result = Some(RecognitionResult {
                    title: locale.text(UiText::RecognitionTextTitle).to_owned(),
                    text,
                });
                locale.text(UiText::RecognitionTextCompleted).to_owned()
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.recognition_retry = Some(RecognitionRetry::Ocr);
                locale.text(UiText::RecognitionOcrUnavailable).to_owned()
            }
            Err(error) => {
                self.recognition_retry = Some(RecognitionRetry::Ocr);
                log::warn!(target: "flash_shot::ocr", "text_recognition_failed error={error}");
                locale.format_template(
                    UiText::RecognitionOcrFailed,
                    &[("error", &error.to_string())],
                )
            }
        };
        cx.notify();
    }

    pub(in crate::app) fn translate_selection(&mut self, cx: &mut Context<Self>) {
        if let Some(status) =
            recognition_start_conflict_status(self.settings.locale, self.recognition_in_flight)
        {
            self.status = status;
            cx.notify();
            return;
        }
        let Some(selection) = self.session.selection() else {
            self.status = self
                .settings
                .locale
                .text(UiText::RecognitionSelectAreaTranslate)
                .to_owned();
            cx.notify();
            return;
        };
        let generation = self.begin_recognition_operation();
        let config = match crate::translation::TranslationConfig::from_environment() {
            Ok(Some(config)) => config,
            Ok(None) => {
                self.recognition_in_flight = false;
                self.recognition_retry = Some(RecognitionRetry::Translation);
                self.status = self
                    .settings
                    .locale
                    .text(UiText::TranslationDisabled)
                    .to_owned();
                cx.notify();
                return;
            }
            Err(error) => {
                self.recognition_in_flight = false;
                self.recognition_retry = Some(RecognitionRetry::Translation);
                self.status = self.settings.locale.format_template(
                    UiText::TranslationUnavailable,
                    &[("error", &error.to_string())],
                );
                cx.notify();
                return;
            }
        };
        let Some((frame, document)) = self.export_source() else {
            self.recognition_in_flight = false;
            cx.notify();
            return;
        };
        let ocr_language = self.settings.ocr_language.clone();

        self.status = self
            .settings
            .locale
            .text(UiText::TranslationInProgress)
            .to_owned();
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

    /// Starts one recognition request, clearing stale output and invalidating older async tasks.
    fn begin_recognition_operation(&mut self) -> u64 {
        self.recognition_generation = self.recognition_generation.wrapping_add(1);
        self.recognition_result = None;
        self.recognition_retry = None;
        self.recognition_in_flight = true;
        self.recognition_generation
    }

    /// Invalidates recognition output whenever its source editor is discarded or replaced.
    pub(in crate::app) fn invalidate_recognition(&mut self) {
        self.recognition_generation = self.recognition_generation.wrapping_add(1);
        self.recognition_result = None;
        self.recognition_retry = None;
        self.recognition_in_flight = false;
    }

    fn finish_translation(
        &mut self,
        result: TranslationOutcome,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if !recognition_completion_is_current(self.recognition_generation, generation) {
            return;
        }
        self.recognition_in_flight = false;
        let locale = self.settings.locale;
        self.status = match result {
            TranslationOutcome::Completed(text) if text.is_empty() => {
                self.recognition_retry = None;
                locale.text(UiText::TranslationNoText).to_owned()
            }
            TranslationOutcome::Completed(text) => {
                self.recognition_retry = None;
                self.recognition_result = Some(RecognitionResult {
                    title: locale.text(UiText::OverlayTranslate).to_owned(),
                    text,
                });
                locale.text(UiText::TranslationCompleted).to_owned()
            }
            TranslationOutcome::PreparationFailed(error) => {
                self.recognition_retry = Some(RecognitionRetry::Translation);
                log::warn!(target: "flash_shot::translation", "translation_preparation_failed error={error}");
                translation_failure_status(locale, &TranslationOutcome::PreparationFailed(error))
            }
            TranslationOutcome::OcrUnavailable => {
                self.recognition_retry = Some(RecognitionRetry::Translation);
                translation_failure_status(locale, &TranslationOutcome::OcrUnavailable)
            }
            TranslationOutcome::OcrFailed(error) => {
                self.recognition_retry = Some(RecognitionRetry::Translation);
                log::warn!(target: "flash_shot::translation", "translation_ocr_failed error={error}");
                translation_failure_status(locale, &TranslationOutcome::OcrFailed(error))
            }
            TranslationOutcome::ServiceFailed(error) => {
                self.recognition_retry = Some(RecognitionRetry::Translation);
                log::warn!(target: "flash_shot::translation", "translation_service_failed error={error}");
                translation_failure_status(locale, &TranslationOutcome::ServiceFailed(error))
            }
        };
        cx.notify();
    }
}

/// Accepts an async recognition result only while its original editor generation still owns it.
pub(crate) const fn recognition_completion_is_current(current: u64, task: u64) -> bool {
    current == task
}

/// Prevents overlapping OCR, translation, and QR requests from replacing one another.
pub(crate) fn recognition_start_conflict_status(locale: Locale, in_flight: bool) -> Option<String> {
    in_flight.then(|| locale.text(UiText::RecognitionBusy).to_owned())
}
