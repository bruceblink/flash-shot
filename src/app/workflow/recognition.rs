//! QR, OCR, and translation workflows.

use super::*;

impl FlashShotApp {
    pub(in crate::app) fn recognize_qr_selection(&mut self, cx: &mut Context<Self>) {
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

    pub(in crate::app) fn recognize_text_selection(&mut self, cx: &mut Context<Self>) {
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
        self.recognition_retry = None;
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
            Ok(text) if text.is_empty() => {
                self.recognition_retry = None;
                "No text found in the selection".to_owned()
            }
            Ok(text) => {
                self.recognition_retry = None;
                self.recognition_result = Some(RecognitionResult {
                    title: "Recognized text".to_owned(),
                    text,
                });
                "Text recognized locally".to_owned()
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.recognition_retry = Some(RecognitionRetry::Ocr);
                "Local OCR is unavailable. Install Tesseract or set FLASH_SHOT_TESSERACT."
                    .to_owned()
            }
            Err(error) => {
                self.recognition_retry = Some(RecognitionRetry::Ocr);
                log::warn!(target: "flash_shot::ocr", "text_recognition_failed error={error}");
                format!("OCR failed: {error}")
            }
        };
        cx.notify();
    }

    pub(in crate::app) fn translate_selection(&mut self, cx: &mut Context<Self>) {
        let Some(selection) = self.session.selection() else {
            self.status = "Select an area before translating text".to_owned();
            cx.notify();
            return;
        };
        self.recognition_retry = None;
        let config = match crate::translation::TranslationConfig::from_environment() {
            Ok(Some(config)) => config,
            Ok(None) => {
                self.recognition_retry = Some(RecognitionRetry::Translation);
                self.status =
                    "Translation is disabled. Configure FLASH_SHOT_TRANSLATION_ENDPOINT to opt in."
                        .to_owned();
                cx.notify();
                return;
            }
            Err(error) => {
                self.recognition_retry = Some(RecognitionRetry::Translation);
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
                self.recognition_retry = None;
                "No text found in the selection".to_owned()
            }
            TranslationOutcome::Completed(text) => {
                self.recognition_retry = None;
                self.recognition_result = Some(RecognitionResult {
                    title: "Translation".to_owned(),
                    text,
                });
                "Translation completed".to_owned()
            }
            TranslationOutcome::PreparationFailed(error) => {
                self.recognition_retry = Some(RecognitionRetry::Translation);
                log::warn!(target: "flash_shot::translation", "translation_preparation_failed error={error}");
                translation_failure_status(&TranslationOutcome::PreparationFailed(error))
            }
            TranslationOutcome::OcrUnavailable => {
                self.recognition_retry = Some(RecognitionRetry::Translation);
                translation_failure_status(&TranslationOutcome::OcrUnavailable)
            }
            TranslationOutcome::OcrFailed(error) => {
                self.recognition_retry = Some(RecognitionRetry::Translation);
                log::warn!(target: "flash_shot::translation", "translation_ocr_failed error={error}");
                translation_failure_status(&TranslationOutcome::OcrFailed(error))
            }
            TranslationOutcome::ServiceFailed(error) => {
                self.recognition_retry = Some(RecognitionRetry::Translation);
                log::warn!(target: "flash_shot::translation", "translation_service_failed error={error}");
                translation_failure_status(&TranslationOutcome::ServiceFailed(error))
            }
        };
        cx.notify();
    }
}
