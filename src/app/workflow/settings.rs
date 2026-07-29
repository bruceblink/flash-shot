//! Settings, support checks, and update workflow orchestration.

use super::*;

impl FlashShotApp {
    pub(in crate::app) fn select_settings_section(
        &mut self,
        section: SettingsSection,
        cx: &mut Context<Self>,
    ) {
        if self.settings_section != section {
            self.settings_section = section;
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

    /// Requires a deliberate second action before removing every managed screenshot.
    pub(in crate::app) fn request_history_clear(&mut self, cx: &mut Context<Self>) {
        if self.history_clear_in_flight || self.history_retention_target.is_some() {
            return;
        }
        if self.history.entries().is_empty() {
            self.status = "Screenshot history is already empty".to_owned();
        } else {
            self.history_clear_confirmation = true;
            self.status = format!(
                "Confirm deletion of {} saved capture(s)",
                self.history.entries().len()
            );
        }
        cx.notify();
    }

    /// Leaves every managed screenshot untouched after an accidental clear request.
    pub(in crate::app) fn cancel_history_clear(&mut self, cx: &mut Context<Self>) {
        self.history_clear_confirmation = false;
        self.status = "Screenshot history clear cancelled".to_owned();
        cx.notify();
    }

    /// Cycles the persisted local-OCR language preset without changing the executable location.
    pub(in crate::app) fn cycle_ocr_language(&mut self, cx: &mut Context<Self>) {
        let previous = self.settings.ocr_language.clone();
        let next = UserSettings::next_ocr_language(previous.as_deref());
        self.settings.ocr_language = next;
        self.status = match self.settings.save(&self.settings_path) {
            Ok(()) => format!(
                "Local OCR language: {}",
                ocr_language_label(self.settings.ocr_language.as_deref())
            ),
            Err(error) => {
                self.settings.ocr_language = previous;
                format!("Could not save OCR language preference: {error}")
            }
        };
        cx.notify();
    }

    /// Probes Tesseract and the selected language before the user needs OCR on a screenshot.
    pub(in crate::app) fn check_ocr_support(&mut self, cx: &mut Context<Self>) {
        let language = self.settings.ocr_language.clone();
        self.status = "Checking local OCR support...".to_owned();
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
                        this.status = ocr_support_status(result.as_ref());
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    /// Checks the opt-in translation configuration without sending screenshot text or a network request.
    pub(in crate::app) fn check_translation_support(&mut self, cx: &mut Context<Self>) {
        self.status =
            translation_support_status(crate::translation::TranslationConfig::from_environment());
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
        if !self.settings.capture_shortcut_enabled {
            let previous_label = self.capture_shortcut.clone();
            let previous_preference = self.settings.capture_shortcut.clone();
            self.capture_shortcut = shortcut.to_string();
            self.settings.capture_shortcut = Some(self.capture_shortcut.clone());
            self.status = match self.settings.save(&self.settings_path) {
                Ok(()) => format!(
                    "Capture shortcut changed to {}; global shortcut remains disabled",
                    self.capture_shortcut
                ),
                Err(error) => {
                    self.capture_shortcut = previous_label;
                    self.settings.capture_shortcut = previous_preference;
                    format!("Could not save shortcut preference: {error}")
                }
            };
            cx.notify();
            return;
        }
        let previous_label = self.capture_shortcut.clone();
        let previous_preference = self.settings.capture_shortcut.clone();
        let previous_service = self._shortcut.take();
        drop(previous_service);
        let replacement = match GlobalShortcutService::register_capture(shortcut) {
            Ok((service, events)) => {
                Self::listen_for_shortcut(events, cx);
                service
            }
            Err(error) => {
                self.restore_capture_shortcut(&previous_label, cx);
                self.capture_shortcut_enabled = self._shortcut.is_some();
                self.set_tray_capture_shortcut_enabled(self.capture_shortcut_enabled);
                self.status = format!("Could not register {preset}: {error}");
                cx.notify();
                return;
            }
        };
        self._shortcut = Some(replacement);
        self.capture_shortcut_enabled = true;
        self.capture_shortcut = shortcut.to_string();
        self.settings.capture_shortcut = Some(self.capture_shortcut.clone());
        match self.settings.save(&self.settings_path) {
            Ok(()) => {
                self.status = format!("Capture shortcut changed to {}", self.capture_shortcut);
            }
            Err(error) => {
                let replacement = self._shortcut.take();
                drop(replacement);
                self.restore_capture_shortcut(&previous_label, cx);
                self.capture_shortcut_enabled = self._shortcut.is_some();
                self.capture_shortcut = previous_label;
                self.settings.capture_shortcut = previous_preference;
                self.status = format!("Could not save shortcut preference: {error}");
            }
        }
        self.set_tray_capture_shortcut_enabled(self.capture_shortcut_enabled);
        cx.notify();
    }

    fn restore_capture_shortcut(&mut self, label: &str, cx: &mut Context<Self>) {
        let Ok(shortcut) = label.parse() else {
            return;
        };
        match GlobalShortcutService::register_capture(shortcut) {
            Ok((service, events)) => {
                Self::listen_for_shortcut(events, cx);
                self._shortcut = Some(service);
            }
            Err(error) => {
                log::warn!(target: "flash_shot::shortcut", "capture_hotkey_restore_failed shortcut={label} error={error}");
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
        let (service, events) = match GlobalShortcutService::register_capture(shortcut) {
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

    pub(in crate::app) fn check_for_updates(&mut self, cx: &mut Context<Self>) {
        if self.update_check_in_flight {
            return;
        }
        let config = match UpdateConfig::from_environment() {
            Ok(Some(config)) => config,
            Ok(None) => {
                self.status =
                    "Update checks are disabled: set FLASH_SHOT_UPDATE_ENDPOINT".to_owned();
                cx.notify();
                return;
            }
            Err(error) => {
                self.status = format!("Update checks are unavailable: {error}");
                cx.notify();
                return;
            }
        };
        self.update_check_in_flight = true;
        self.status = "Checking for updates...".to_owned();
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move { crate::update::check(&config, env!("CARGO_PKG_VERSION")) })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| this.finish_update_check(result, cx));
                }
            }
        })
        .detach();
    }

    fn finish_update_check(
        &mut self,
        result: std::io::Result<UpdateAvailability>,
        cx: &mut Context<Self>,
    ) {
        self.update_check_in_flight = false;
        self.status = match result {
            Ok(UpdateAvailability::Available { version }) => {
                format!(
                    "Update available: {version} (download from your configured release channel)"
                )
            }
            Ok(UpdateAvailability::Current { version }) => {
                format!("Flash Shot {version} is up to date")
            }
            Ok(UpdateAvailability::NewerLocal { version }) => {
                format!("Installed version is newer than release manifest {version}")
            }
            Err(error) => {
                log::warn!(target: "flash_shot::update", "update_check_failed error={error}");
                format!("Could not check for updates: {error}")
            }
        };
        cx.notify();
    }
}
