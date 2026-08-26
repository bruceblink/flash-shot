//! Settings, support checks, and update workflow orchestration.

use super::super::{HistoryClearScope, history_entry_matches, selected_history_paths};
use super::*;
use crate::i18n::{Locale, UiText};

impl FlashShotApp {
    /// Selects the initial settings page for native acceptance screenshots.
    ///
    /// The production tray path keeps its persisted section unchanged; this narrow helper is
    /// only used by the disposable screenshot probe so every settings workflow can be reviewed.
    pub(crate) fn set_settings_section_for_acceptance(&mut self, section: &str) {
        self.settings_section = match section {
            "library" => SettingsSection::Files,
            "record" => SettingsSection::Recording,
            "app" => SettingsSection::System,
            _ => SettingsSection::Capture,
        };
    }

    /// Seeds one Record page lifecycle state for screenshots without spawning an FFmpeg process.
    /// The acceptance-only flag keeps the controls visible while production recording ownership
    /// remains represented by `recording_control`.
    pub(crate) fn set_recording_state_for_acceptance(
        &mut self,
        state: crate::RecordingUiAcceptanceState,
    ) {
        let idle = state == crate::RecordingUiAcceptanceState::Idle;
        let (active, starting, stopping, paused, progress, status) =
            acceptance_recording_state(state);
        self.recording_acceptance_active = active;
        self.recording_start_in_flight = starting;
        self.recording_stopping = stopping;
        self.recording_paused = paused;
        self.recording_progress = progress;
        self.status = if idle {
            self.settings
                .locale
                .ready_with_shortcut(&self.capture_shortcut)
        } else {
            status.to_owned()
        };
    }

    /// Seeds the FFmpeg support-check button for screenshots without launching local probes.
    pub(crate) fn set_recording_support_check_for_acceptance(
        &mut self,
        state: crate::RecordingSupportUiAcceptanceState,
    ) {
        self.recording_support_check_in_flight =
            state == crate::RecordingSupportUiAcceptanceState::Checking;
        if self.recording_support_check_in_flight {
            self.status = self
                .settings
                .locale
                .text(crate::i18n::UiText::RecordingSupportCheckInProgress)
                .to_owned();
        }
    }

    /// Seeds the translation service button for screenshots without contacting the configured
    /// endpoint. Production requests continue to own their state through the test workflow.
    pub(crate) fn set_translation_service_test_for_acceptance(
        &mut self,
        state: crate::TranslationServiceUiAcceptanceState,
    ) {
        self.translation_service_test_in_flight = false;
        match state {
            crate::TranslationServiceUiAcceptanceState::Idle => {}
            crate::TranslationServiceUiAcceptanceState::Testing => {
                self.translation_service_test_in_flight = true;
                self.status = self
                    .settings
                    .locale
                    .text(crate::i18n::UiText::TranslationServiceTestInProgress)
                    .to_owned();
            }
            crate::TranslationServiceUiAcceptanceState::Ready => {
                // Keep a fixed count so acceptance screenshots never contact or expose a service.
                self.status = self.settings.locale.format_template(
                    crate::i18n::UiText::TranslationServiceReady,
                    &[("count", "12")],
                );
            }
        }
    }

    /// Seeds the OCR support button for screenshots without probing the local installation.
    pub(crate) fn set_ocr_support_check_for_acceptance(
        &mut self,
        state: crate::OcrSupportUiAcceptanceState,
    ) {
        self.ocr_support_check_in_flight = state == crate::OcrSupportUiAcceptanceState::Checking;
        if self.ocr_support_check_in_flight {
            self.status = self
                .settings
                .locale
                .text(crate::i18n::UiText::OcrSupportCheckInProgress)
                .to_owned();
        }
    }

    /// Seeds the update-check button for screenshots without contacting a release endpoint.
    pub(crate) fn set_update_check_for_acceptance(
        &mut self,
        state: crate::UpdateUiAcceptanceState,
    ) {
        self.update_check_in_flight = state == crate::UpdateUiAcceptanceState::Checking;
        if self.update_check_in_flight {
            self.status = self
                .settings
                .locale
                .text(UiText::UpdateCheckInProgress)
                .to_owned();
        }
    }

    /// Opens a native folder picker, then swaps history only after the new private root is ready.
    pub(in crate::app) fn choose_quick_save_directory(&mut self, cx: &mut Context<Self>) {
        let locale = self.settings.locale;
        if self.history_root_change_in_flight
            || self.history_file_read_in_flight()
            || self.history_mutation_pending()
        {
            self.status = locale.text(UiText::QuickSaveFolderBusy).to_owned();
            cx.notify();
            return;
        }
        self.history_root_change_in_flight = true;
        self.status = locale.text(UiText::QuickSaveFolderChoosing).to_owned();
        cx.notify();
        let limit = usize::from(self.settings.history_limit);
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(locale.text(UiText::QuickSaveFolderPrompt).into()),
        });
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = match prompt.await {
                    Ok(Ok(Some(mut paths))) => match paths.pop() {
                        Some(path) => {
                            cx.background_executor()
                                .spawn(async move { open_verified_quick_save_history(path, limit) })
                                .await
                        }
                        None => {
                            if let Some(this) = this.upgrade() {
                                this.update(&mut cx, |this, cx| {
                                    this.history_root_change_in_flight = false;
                                    this.status = locale
                                        .text(UiText::QuickSaveFolderSelectionCancelled)
                                        .to_owned();
                                    cx.notify();
                                });
                            }
                            return;
                        }
                    },
                    Ok(Ok(None)) => {
                        if let Some(this) = this.upgrade() {
                            this.update(&mut cx, |this, cx| {
                                this.history_root_change_in_flight = false;
                                this.status = locale
                                    .text(UiText::QuickSaveFolderSelectionCancelled)
                                    .to_owned();
                                cx.notify();
                            });
                        }
                        return;
                    }
                    Ok(Err(error)) => Err(std::io::Error::other(error)),
                    Err(error) => Err(std::io::Error::other(error.to_string())),
                };
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.history_root_change_in_flight = false;
                        match result {
                            Ok(history) => {
                                let previous = this.settings.quick_save_directory.clone();
                                this.settings.quick_save_directory =
                                    Some(history.root().to_owned());
                                match this.settings.save(&this.settings_path) {
                                    Ok(()) => {
                                        this.history = history;
                                        this.synchronize_history_preview_cache();
                                        let path = this.history.root().display().to_string();
                                        this.status = locale.format_template(
                                            UiText::QuickSaveFolderChanged,
                                            &[("path", &path)],
                                        );
                                    }
                                    Err(error) => {
                                        this.settings.quick_save_directory = previous;
                                        let error_detail = error.to_string();
                                        this.status = locale.format_template(
                                            UiText::QuickSaveFolderPreferenceSaveFailed,
                                            &[("error", &error_detail)],
                                        );
                                    }
                                }
                            }
                            Err(error) => {
                                let error_detail = error.to_string();
                                this.status = locale.format_template(
                                    UiText::QuickSaveFolderUseFailed,
                                    &[("error", &error_detail)],
                                );
                            }
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    /// Checks the active quick-save root asynchronously so a permission failure is visible before
    /// the next screenshot export. The probe only creates and removes its own temporary file.
    pub(in crate::app) fn check_quick_save_directory(&mut self, cx: &mut Context<Self>) {
        if self.quick_save_directory_check_in_flight
            || self.history_root_change_in_flight
            || self.history_write_generation.is_some()
        {
            return;
        }
        self.quick_save_directory_check_in_flight = true;
        let directory = self.history.root().to_owned();
        let locale = self.settings.locale;
        let path = directory.display().to_string();
        self.status = locale.format_template(UiText::QuickSaveFolderChecking, &[("path", &path)]);
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let probe_directory = directory.clone();
                let result = cx
                    .background_executor()
                    .spawn(
                        async move { crate::history::verify_writable_directory(&probe_directory) },
                    )
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.quick_save_directory_check_in_flight = false;
                        if this.history.root() != directory {
                            cx.notify();
                            return;
                        }
                        this.status = match result {
                            Ok(()) => locale
                                .format_template(UiText::QuickSaveFolderReady, &[("path", &path)]),
                            Err(error) => {
                                let error_detail = error.to_string();
                                locale.format_template(
                                    UiText::QuickSaveFolderCheckFailed,
                                    &[("error", &error_detail)],
                                )
                            }
                        };
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    /// Cycles a persisted safe filename prefix shared by selection, full-screen, and pin saves.
    pub(in crate::app) fn cycle_quick_save_prefix(&mut self, cx: &mut Context<Self>) {
        let locale = self.settings.locale;
        let previous = self.settings.quick_save_prefix.clone();
        self.settings.quick_save_prefix = UserSettings::next_save_prefix(&previous);
        self.status = match self.settings.save(&self.settings_path) {
            Ok(()) => locale.format_template(
                UiText::QuickSavePrefixChanged,
                &[("prefix", &self.settings.quick_save_prefix)],
            ),
            Err(error) => {
                self.settings.quick_save_prefix = previous;
                let error_detail = error.to_string();
                locale.format_template(
                    UiText::QuickSavePrefixSaveFailed,
                    &[("error", &error_detail)],
                )
            }
        };
        cx.notify();
    }

    pub(in crate::app) fn select_settings_section(
        &mut self,
        section: SettingsSection,
        cx: &mut Context<Self>,
    ) {
        if self.settings_section != section {
            self.settings_section = section;
            if section != SettingsSection::Files {
                self.history_search.active = false;
                self.history_search.marked_range = None;
                self.history_keyboard_focus = None;
            }
            cx.notify();
        }
    }

    /// Expands the saved-capture list only after the user asks for it, avoiding thumbnail work in
    /// the default settings view while still making every retained capture reachable.
    pub(in crate::app) fn toggle_history_expanded(&mut self, cx: &mut Context<Self>) {
        self.history_expanded = !self.history_expanded;
        cx.notify();
    }

    pub(in crate::app) fn select_history_filter(
        &mut self,
        filter: HistoryFilter,
        cx: &mut Context<Self>,
    ) {
        if self.history_filter != filter {
            self.history_filter = filter;
            self.history_expanded = false;
            cx.notify();
        }
    }

    /// Toggles one managed history path while keeping batch actions independent from thumbnails.
    pub(in crate::app) fn toggle_history_selection(
        &mut self,
        path: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        if self.history_clear_in_flight
            || self.history_clear_confirmation
            || self.history_retention_target.is_some()
            || !self
                .history
                .entries()
                .iter()
                .any(|entry| entry.path == path)
        {
            return;
        }
        self.history_keyboard_focus = Some(path.clone());
        if self.history_selected_paths.remove(&path) {
            self.status = "Capture removed from selection".to_owned();
        } else {
            self.history_selected_paths.insert(path);
            self.status = format!("{} capture(s) selected", self.history_selected_paths.len());
        }
        cx.notify();
    }

    /// Adds every current filter match to the selection without deleting anything implicitly.
    pub(in crate::app) fn select_filtered_history(&mut self, cx: &mut Context<Self>) {
        if self.history_clear_in_flight
            || self.history_clear_confirmation
            || self.history_retention_target.is_some()
            || !self.history_deletions_in_flight.is_empty()
        {
            return;
        }
        let paths = selected_history_paths(
            self.history.entries(),
            &self.history_selected_paths,
            self.history_filter,
            self.history_search_query(),
        );
        let matching_paths = self.filtered_history_paths();
        let previous_count = self.history_selected_paths.len();
        self.history_selected_paths.extend(matching_paths);
        let added = self
            .history_selected_paths
            .len()
            .saturating_sub(previous_count);
        self.status = if added == 0 {
            if paths.is_empty() {
                "No captures match the current filter".to_owned()
            } else {
                format!("{} capture(s) already selected", paths.len())
            }
        } else {
            format!("Selected {added} additional capture(s)")
        };
        cx.notify();
    }

    /// Clears only the in-memory selection, leaving every saved file untouched.
    pub(in crate::app) fn clear_history_selection(&mut self, cx: &mut Context<Self>) {
        if self.history_selected_paths.is_empty() {
            return;
        }
        self.history_selected_paths.clear();
        self.status = "History selection cleared".to_owned();
        cx.notify();
    }

    /// Requires a deliberate second action before removing every managed screenshot.
    pub(in crate::app) fn request_history_clear(&mut self, cx: &mut Context<Self>) {
        self.request_history_clear_scope(HistoryClearScope::All, cx);
    }

    /// Starts the same guarded flow for only the captures currently visible through history filters.
    pub(in crate::app) fn request_filtered_history_clear(&mut self, cx: &mut Context<Self>) {
        self.request_history_clear_scope(HistoryClearScope::Filtered, cx);
    }

    /// Starts the guarded confirmation flow for the exact paths the user selected.
    pub(in crate::app) fn request_selected_history_clear(&mut self, cx: &mut Context<Self>) {
        self.request_history_clear_scope(HistoryClearScope::Selected, cx);
    }

    /// Captures the exact deletion set before asking for confirmation so later list changes cannot
    /// silently widen a destructive filtered-history operation.
    fn request_history_clear_scope(&mut self, scope: HistoryClearScope, cx: &mut Context<Self>) {
        if !self.history_mutation_can_start() {
            return;
        }
        let paths = match scope {
            HistoryClearScope::All => self
                .history
                .entries()
                .iter()
                .map(|entry| entry.path.clone())
                .collect(),
            HistoryClearScope::Filtered => self.filtered_history_paths(),
            HistoryClearScope::Selected => self
                .history
                .entries()
                .iter()
                .filter(|entry| self.history_selected_paths.contains(&entry.path))
                .map(|entry| entry.path.clone())
                .collect(),
        };
        if paths.is_empty() {
            self.history_clear_scope = HistoryClearScope::default();
            self.history_clear_count = 0;
            self.history_clear_paths.clear();
            self.status = match scope {
                HistoryClearScope::Selected => "Select at least one capture first".to_owned(),
                _ => "Screenshot history is already empty".to_owned(),
            };
        } else {
            self.history_clear_scope = scope;
            self.history_clear_count = paths.len();
            self.history_clear_paths = paths;
            self.history_clear_confirmation = true;
            self.status = format!(
                "Confirm deletion of {} {} saved capture(s)",
                self.history_clear_count,
                scope.label(),
            );
        }
        cx.notify();
    }

    /// Resolves the current filter and query into paths that the history store may safely delete.
    pub(in crate::app) fn filtered_history_paths(&self) -> Vec<std::path::PathBuf> {
        self.history
            .entries()
            .iter()
            .filter(|entry| {
                history_entry_matches(entry, self.history_filter, self.history_search_query())
            })
            .map(|entry| entry.path.clone())
            .collect()
    }

    /// Leaves every managed screenshot untouched after an accidental clear request.
    pub(in crate::app) fn cancel_history_clear(&mut self, cx: &mut Context<Self>) {
        self.history_clear_confirmation = false;
        self.history_clear_scope = HistoryClearScope::default();
        self.history_clear_count = 0;
        self.history_clear_paths.clear();
        self.status = "Screenshot history clear cancelled".to_owned();
        cx.notify();
    }

    /// Cycles the persisted local-OCR language preset without changing the executable location.
    pub(in crate::app) fn cycle_ocr_language(&mut self, cx: &mut Context<Self>) {
        let previous = self.settings.ocr_language.clone();
        let next = UserSettings::next_ocr_language(previous.as_deref());
        self.settings.ocr_language = next;
        let locale = self.settings.locale;
        self.status = match self.settings.save(&self.settings_path) {
            Ok(()) => locale.format_template(
                crate::i18n::UiText::OcrLanguageChanged,
                &[(
                    "language",
                    ocr_language_label(locale, self.settings.ocr_language.as_deref()),
                )],
            ),
            Err(error) => {
                self.settings.ocr_language = previous;
                locale.format_template(
                    crate::i18n::UiText::OcrLanguageSaveFailed,
                    &[("error", &error.to_string())],
                )
            }
        };
        cx.notify();
    }

    /// Probes Tesseract and the selected language before the user needs OCR on a screenshot.
    pub(in crate::app) fn check_ocr_support(&mut self, cx: &mut Context<Self>) {
        if self.ocr_support_check_in_flight {
            self.status = self
                .settings
                .locale
                .text(crate::i18n::UiText::OcrSupportCheckBusy)
                .to_owned();
            cx.notify();
            return;
        }
        let language = self.settings.ocr_language.clone();
        self.ocr_support_check_generation = self.ocr_support_check_generation.wrapping_add(1);
        let generation = self.ocr_support_check_generation;
        self.ocr_support_check_in_flight = true;
        self.status = self
            .settings
            .locale
            .text(crate::i18n::UiText::OcrSupportCheckInProgress)
            .to_owned();
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move { crate::ocr::check_support(language.as_deref()) })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        if this.ocr_support_check_generation != generation {
                            return;
                        }
                        this.ocr_support_check_in_flight = false;
                        this.status = ocr_support_status(this.settings.locale, result.as_ref());
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    /// Sends a fixed, non-user phrase to the configured HTTPS endpoint so service connectivity can
    /// be verified from settings without reusing or exposing text from a captured screenshot.
    pub(in crate::app) fn test_translation_service(&mut self, cx: &mut Context<Self>) {
        if self.translation_service_test_in_flight {
            self.status = self
                .settings
                .locale
                .text(crate::i18n::UiText::TranslationServiceTestBusy)
                .to_owned();
            cx.notify();
            return;
        }
        let config = match crate::translation::TranslationConfig::from_environment() {
            Ok(Some(config)) => config,
            Ok(None) => {
                self.status = translation_support_status(self.settings.locale, Ok(None));
                cx.notify();
                return;
            }
            Err(error) => {
                self.status = translation_support_status(self.settings.locale, Err(error));
                cx.notify();
                return;
            }
        };
        self.translation_service_test_generation =
            self.translation_service_test_generation.wrapping_add(1);
        let generation = self.translation_service_test_generation;
        self.translation_service_test_in_flight = true;
        self.status = self
            .settings
            .locale
            .text(crate::i18n::UiText::TranslationServiceTestInProgress)
            .to_owned();
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move { crate::translation::translate(&config, "Flash Shot") })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.finish_translation_service_test(result, generation, cx)
                    });
                }
            }
        })
        .detach();
    }

    /// Cancels the visible service check so a slow endpoint never leaves the settings UI stuck.
    /// The request may still finish in the background, but its generation can no longer update UI.
    pub(in crate::app) fn cancel_translation_service_test(&mut self, cx: &mut Context<Self>) {
        if !self.translation_service_test_in_flight {
            return;
        }
        self.translation_service_test_generation =
            self.translation_service_test_generation.wrapping_add(1);
        self.translation_service_test_in_flight = false;
        self.status = self
            .settings
            .locale
            .text(crate::i18n::UiText::TranslationServiceTestCancelled)
            .to_owned();
        cx.notify();
    }

    fn finish_translation_service_test(
        &mut self,
        result: std::io::Result<String>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if self.translation_service_test_generation != generation {
            return;
        }
        self.translation_service_test_in_flight = false;
        self.status = translation_service_test_status(self.settings.locale, &result);
        cx.notify();
    }

    pub(in crate::app) fn select_capture_shortcut(
        &mut self,
        preset: &'static str,
        cx: &mut Context<Self>,
    ) {
        if self.capture_shortcut == preset {
            return;
        }
        let shortcut = match crate::platform::shortcut::CaptureShortcut::parse_preset(preset) {
            Ok(shortcut) => shortcut,
            Err(error) => {
                self.status = format!("Could not use shortcut: {error}");
                cx.notify();
                return;
            }
        };
        let previous_label = self.capture_shortcut.clone();
        let previous_settings = self.settings.clone();
        self.capture_shortcut = shortcut.to_string();
        self.settings.capture_shortcut = Some(self.capture_shortcut.clone());
        self.apply_shortcut_change(
            previous_label,
            previous_settings,
            format!("Capture shortcut changed to {}", self.capture_shortcut),
            cx,
        );
    }

    /// Cycles the full-screen action key while keeping every active action distinct.
    pub(in crate::app) fn cycle_full_screen_shortcut(&mut self, cx: &mut Context<Self>) {
        let previous_label = self.capture_shortcut.clone();
        let previous_settings = self.settings.clone();
        self.settings.full_screen_shortcut = UserSettings::next_global_shortcut(
            self.settings.full_screen_shortcut.as_deref(),
            [
                Some(self.capture_shortcut.as_str()),
                self.settings.focused_window_shortcut.as_deref(),
            ],
        );
        self.apply_shortcut_change(
            previous_label,
            previous_settings,
            format!(
                "Full-screen shortcut: {}",
                shortcut_option_label(self.settings.full_screen_shortcut.as_deref())
            ),
            cx,
        );
    }

    /// Cycles the focus-window action key while keeping every active action distinct.
    pub(in crate::app) fn cycle_focused_window_shortcut(&mut self, cx: &mut Context<Self>) {
        let previous_label = self.capture_shortcut.clone();
        let previous_settings = self.settings.clone();
        self.settings.focused_window_shortcut = UserSettings::next_global_shortcut(
            self.settings.focused_window_shortcut.as_deref(),
            [
                Some(self.capture_shortcut.as_str()),
                self.settings.full_screen_shortcut.as_deref(),
            ],
        );
        self.apply_shortcut_change(
            previous_label,
            previous_settings,
            format!(
                "Focused-window shortcut: {}",
                shortcut_option_label(self.settings.focused_window_shortcut.as_deref())
            ),
            cx,
        );
    }

    /// Re-registers every configured action as one native set before persisting the new preference.
    fn apply_shortcut_change(
        &mut self,
        previous_label: String,
        previous_settings: UserSettings,
        success_status: String,
        cx: &mut Context<Self>,
    ) {
        if !self.settings.capture_shortcut_enabled {
            self.status = match self.settings.save(&self.settings_path) {
                Ok(()) => format!("{success_status}; global shortcuts remain disabled"),
                Err(error) => {
                    self.capture_shortcut = previous_label;
                    self.settings = previous_settings;
                    format!("Could not save shortcut preference: {error}")
                }
            };
            cx.notify();
            return;
        }
        drop(self._shortcut.take());
        let registration = self
            .capture_shortcut
            .parse()
            .map_err(std::io::Error::other)
            .and_then(|capture| super::super::register_global_shortcuts(capture, &self.settings));
        match registration {
            Ok((service, events)) => {
                if let Err(error) = self.settings.save(&self.settings_path) {
                    drop(service);
                    self.capture_shortcut = previous_label;
                    self.settings = previous_settings;
                    self.restore_global_shortcuts(cx);
                    self.status = format!("Could not save shortcut preference: {error}");
                } else {
                    Self::listen_for_shortcut(events, cx);
                    self._shortcut = Some(service);
                    self.capture_shortcut_enabled = true;
                    self.set_tray_capture_shortcut_enabled(true);
                    self.status = success_status;
                }
            }
            Err(error) => {
                self.capture_shortcut = previous_label;
                self.settings = previous_settings;
                self.restore_global_shortcuts(cx);
                self.capture_shortcut_enabled = self._shortcut.is_some();
                self.set_tray_capture_shortcut_enabled(self.capture_shortcut_enabled);
                self.status = format!("Could not register global shortcuts: {error}");
            }
        }
        cx.notify();
    }

    fn restore_global_shortcuts(&mut self, cx: &mut Context<Self>) {
        let Ok(capture) = self.capture_shortcut.parse() else {
            return;
        };
        match super::super::register_global_shortcuts(capture, &self.settings) {
            Ok((service, events)) => {
                Self::listen_for_shortcut(events, cx);
                self._shortcut = Some(service);
            }
            Err(error) => {
                log::warn!(target: "flash_shot::shortcut", "global_hotkey_restore_failed error={error}")
            }
        }
    }

    /// Applies and persists the hotkey switch while preserving its configured key combination.
    pub(in crate::app) fn toggle_capture_shortcut(&mut self, cx: &mut Context<Self>) {
        if self.capture_shortcut_enabled {
            self.settings.capture_shortcut_enabled = false;
            if let Err(error) = self.settings.save(&self.settings_path) {
                self.settings.capture_shortcut_enabled = true;
                self.status = format!("Could not disable global shortcut: {error}");
                cx.notify();
                return;
            }
            drop(self._shortcut.take());
            self.capture_shortcut_enabled = false;
            self.set_tray_capture_shortcut_enabled(false);
            self.status = "Global capture shortcut disabled".to_owned();
            self.notify_user("Flash Shot", "Global capture shortcut disabled");
            cx.notify();
            return;
        }

        let shortcut = match self.capture_shortcut.parse() {
            Ok(shortcut) => shortcut,
            Err(error) => {
                self.status = format!("Could not enable global shortcut: {error}");
                cx.notify();
                return;
            }
        };
        let (service, events) =
            match super::super::register_global_shortcuts(shortcut, &self.settings) {
                Ok(registered) => registered,
                Err(error) => {
                    self.status = format!("Could not register {}: {error}", self.capture_shortcut);
                    cx.notify();
                    return;
                }
            };
        self.settings.capture_shortcut_enabled = true;
        if let Err(error) = self.settings.save(&self.settings_path) {
            drop(service);
            self.settings.capture_shortcut_enabled = false;
            self.status = format!("Could not save global shortcut preference: {error}");
            cx.notify();
            return;
        }
        Self::listen_for_shortcut(events, cx);
        self._shortcut = Some(service);
        self.capture_shortcut_enabled = true;
        self.set_tray_capture_shortcut_enabled(true);
        self.status = format!("Global capture shortcut enabled: {}", self.capture_shortcut);
        self.notify_user("Flash Shot", "Global capture shortcut enabled");
        cx.notify();
    }

    pub(in crate::app) fn toggle_auto_start(&mut self, cx: &mut Context<Self>) {
        let executable = match std::env::current_exe() {
            Ok(executable) => executable,
            Err(error) => {
                self.status = format!("Could not find the application executable: {error}");
                cx.notify();
                return;
            }
        };
        let requested = !self.auto_start_enabled;
        match SystemAutoStart.set_enabled(&executable, requested) {
            Ok(AutoStartState::Enabled) => {
                self.auto_start_enabled = true;
                self.set_tray_auto_start_state(AutoStartState::Enabled);
                self.status = "Launch at sign-in enabled".to_owned();
                self.notify_user("Flash Shot", "Launch at sign-in enabled");
            }
            Ok(AutoStartState::Disabled) => {
                self.auto_start_enabled = false;
                self.set_tray_auto_start_state(AutoStartState::Disabled);
                self.status = "Launch at sign-in disabled".to_owned();
                self.notify_user("Flash Shot", "Launch at sign-in disabled");
            }
            Ok(AutoStartState::ManagedByAnotherExecutable) => {
                self.auto_start_enabled = false;
                self.set_tray_auto_start_state(AutoStartState::ManagedByAnotherExecutable);
                self.status =
                    "Launch at sign-in is managed by a different Flash Shot executable".to_owned();
            }
            Err(error) => {
                self.status = format!("Could not update launch at sign-in: {error}");
                log::warn!(target: "flash_shot::autostart", "auto_start_update_failed error={error}");
            }
        }
        cx.notify();
    }

    /// Switches the shared color palette only after the preference is safely persisted.
    pub(in crate::app) fn toggle_theme_mode(&mut self, cx: &mut Context<Self>) {
        let previous = self.settings.theme_mode;
        let next = previous.toggled();
        self.settings.theme_mode = next;
        if let Err(error) = self.settings.save(&self.settings_path) {
            self.settings.theme_mode = previous;
            self.status = format!("Could not save appearance preference: {error}");
            cx.notify();
            return;
        }
        self.colors = crate::theme::ThemeColors::for_mode(next);
        self.status = format!("Appearance changed to {}", next.label());
        cx.notify();
    }

    /// Switches the persisted UI language and lets every visible surface render the new catalog.
    pub(in crate::app) fn cycle_locale(&mut self, cx: &mut Context<Self>) {
        let previous = self.settings.locale;
        let next = previous.next();
        self.settings.locale = next;
        if let Err(error) = self.settings.save(&self.settings_path) {
            self.settings.locale = previous;
            self.status = previous.language_preference_save_failed(&error);
            cx.notify();
            return;
        }
        self.status = next.language_changed(next.label());
        cx.notify();
    }

    pub(in crate::app) fn check_for_updates(&mut self, cx: &mut Context<Self>) {
        let locale = self.settings.locale;
        if self.update_check_in_flight {
            self.status = locale.text(UiText::UpdateCheckBusy).to_owned();
            cx.notify();
            return;
        }
        let config = match UpdateConfig::from_environment() {
            Ok(Some(config)) => config,
            Ok(None) => {
                self.status = locale.text(UiText::UpdateChecksDisabled).to_owned();
                cx.notify();
                return;
            }
            Err(error) => {
                self.status = locale.format_template(
                    UiText::UpdateChecksUnavailable,
                    &[("error", &error.to_string())],
                );
                cx.notify();
                return;
            }
        };
        self.update_check_generation = self.update_check_generation.wrapping_add(1);
        let generation = self.update_check_generation;
        self.update_check_in_flight = true;
        self.status = locale.text(UiText::UpdateCheckInProgress).to_owned();
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move { crate::update::check(&config, env!("CARGO_PKG_VERSION")) })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.finish_update_check(result, generation, cx)
                    });
                }
            }
        })
        .detach();
    }

    /// Cancels the visible manifest request; late network results cannot replace this status.
    pub(in crate::app) fn cancel_update_check(&mut self, cx: &mut Context<Self>) {
        if !self.update_check_in_flight {
            return;
        }
        self.update_check_generation = self.update_check_generation.wrapping_add(1);
        self.update_check_in_flight = false;
        self.status = self
            .settings
            .locale
            .text(UiText::UpdateCheckCancelled)
            .to_owned();
        cx.notify();
    }

    fn finish_update_check(
        &mut self,
        result: std::io::Result<UpdateAvailability>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if self.update_check_generation != generation {
            return;
        }
        self.update_check_in_flight = false;
        self.status = update_check_status(self.settings.locale, result);
        cx.notify();
    }
}

/// Converts an update probe result into the active language's status text while retaining version
/// and error details for recovery decisions. The caller only owns UI state; no update is started.
pub(in crate::app) fn update_check_status(
    locale: Locale,
    result: std::io::Result<UpdateAvailability>,
) -> String {
    match result {
        Ok(UpdateAvailability::Available { version }) => {
            locale.format_template(UiText::UpdateAvailable, &[("version", &version)])
        }
        Ok(UpdateAvailability::Current { version }) => {
            locale.format_template(UiText::UpdateCurrent, &[("version", &version)])
        }
        Ok(UpdateAvailability::NewerLocal { version }) => {
            locale.format_template(UiText::UpdateNewerLocal, &[("version", &version)])
        }
        Err(error) => {
            log::warn!(target: "flash_shot::update", "update_check_failed error={error}");
            locale.format_template(UiText::UpdateCheckFailed, &[("error", &error.to_string())])
        }
    }
}

/// Verifies a user-selected folder before making it the managed quick-save history root.
///
/// Opening a history directory only needs metadata access, while a later export needs durable
/// writes. Running the storage probe first prevents Settings from persisting a read-only folder.
fn open_verified_quick_save_history(
    path: std::path::PathBuf,
    limit: usize,
) -> std::io::Result<crate::history::ScreenshotHistory> {
    crate::history::verify_writable_directory(&path)?;
    crate::history::ScreenshotHistory::open_with_limit(path, limit)
}

pub(in crate::app) fn shortcut_option_label(shortcut: Option<&str>) -> &str {
    shortcut.unwrap_or("Off")
}

fn acceptance_recording_state(
    state: crate::RecordingUiAcceptanceState,
) -> (
    bool,
    bool,
    bool,
    bool,
    crate::recording::RecordingProgress,
    &'static str,
) {
    match state {
        crate::RecordingUiAcceptanceState::Idle => (
            false,
            false,
            false,
            false,
            Default::default(),
            "Ready - Ctrl+Shift+Print Screen",
        ),
        crate::RecordingUiAcceptanceState::Starting => (
            false,
            true,
            false,
            false,
            Default::default(),
            "Discovering FFmpeg and preparing display recording...",
        ),
        crate::RecordingUiAcceptanceState::Recording => (
            true,
            false,
            false,
            false,
            crate::recording::RecordingProgress {
                output_time_us: Some(4_000_000),
                frame: Some(60),
                finished: false,
            },
            "Recording primary display - 4s, 60 frames",
        ),
        crate::RecordingUiAcceptanceState::Paused => (
            true,
            false,
            false,
            true,
            crate::recording::RecordingProgress {
                output_time_us: Some(4_000_000),
                frame: Some(60),
                finished: false,
            },
            "Paused primary display recording - 4s, 60 frames",
        ),
        crate::RecordingUiAcceptanceState::Stopping => (
            false,
            false,
            true,
            false,
            crate::recording::RecordingProgress {
                output_time_us: Some(4_000_000),
                frame: Some(60),
                finished: false,
            },
            "Stopping primary display recording...",
        ),
        crate::RecordingUiAcceptanceState::Cancelled => (
            false,
            false,
            false,
            false,
            Default::default(),
            "Screen recording startup cancelled",
        ),
        crate::RecordingUiAcceptanceState::Failed => (
            false,
            false,
            false,
            false,
            Default::default(),
            "Screen recording failed: FFmpeg exited before producing frames",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{acceptance_recording_state, open_verified_quick_save_history};
    use crate::RecordingUiAcceptanceState;
    use std::{fs, io};

    fn directory(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "flash-shot-quick-save-settings-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    #[test]
    fn selected_quick_save_folder_is_probed_before_history_is_opened() {
        let root = directory("probe");
        fs::create_dir_all(&root).unwrap();

        let history = open_verified_quick_save_history(root.clone(), 30).unwrap();

        assert_eq!(history.root(), root.canonicalize().unwrap());
        assert!(fs::read_dir(history.root()).unwrap().next().is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn selected_file_cannot_become_the_quick_save_folder() {
        let root = directory("file");
        fs::create_dir_all(&root).unwrap();
        let file = root.join("not-a-directory.txt");
        fs::write(&file, b"not a directory").unwrap();

        let error = open_verified_quick_save_history(file.clone(), 30).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(fs::read(&file).unwrap(), b"not a directory");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn acceptance_recording_states_keep_lifecycle_flags_and_progress_truthful() {
        let cases = [
            (
                RecordingUiAcceptanceState::Idle,
                false,
                false,
                false,
                false,
                None,
                "Ready - Ctrl+Shift+Print Screen",
            ),
            (
                RecordingUiAcceptanceState::Starting,
                false,
                true,
                false,
                false,
                None,
                "Discovering FFmpeg and preparing display recording...",
            ),
            (
                RecordingUiAcceptanceState::Recording,
                true,
                false,
                false,
                false,
                Some(4_000_000),
                "Recording primary display - 4s, 60 frames",
            ),
            (
                RecordingUiAcceptanceState::Paused,
                true,
                false,
                false,
                true,
                Some(4_000_000),
                "Paused primary display recording - 4s, 60 frames",
            ),
            (
                RecordingUiAcceptanceState::Stopping,
                false,
                false,
                true,
                false,
                Some(4_000_000),
                "Stopping primary display recording...",
            ),
            (
                RecordingUiAcceptanceState::Cancelled,
                false,
                false,
                false,
                false,
                None,
                "Screen recording startup cancelled",
            ),
            (
                RecordingUiAcceptanceState::Failed,
                false,
                false,
                false,
                false,
                None,
                "Screen recording failed: FFmpeg exited before producing frames",
            ),
        ];

        for (state, active, starting, stopping, paused, output_time_us, expected_status) in cases {
            let (actual_active, actual_starting, actual_stopping, actual_paused, progress, status) =
                acceptance_recording_state(state);

            assert_eq!(actual_active, active);
            assert_eq!(actual_starting, starting);
            assert_eq!(actual_stopping, stopping);
            assert_eq!(actual_paused, paused);
            assert_eq!(progress.output_time_us, output_time_us);
            assert_eq!(progress.frame, output_time_us.map(|_| 60));
            assert_eq!(status, expected_status);
        }
    }
}
