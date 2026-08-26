//! Screen-recording workflow orchestration.

use super::*;
use crate::i18n::{Locale, UiText};

impl FlashShotApp {
    /// Owns the display recording command lifecycle and keeps repeated stop/pause input harmless
    /// while FFmpeg finishes its graceful container shutdown.
    pub(in crate::app) fn toggle_display_recording(&mut self, cx: &mut Context<Self>) {
        let locale = self.settings.locale;
        if self.recording_stopping {
            self.status = locale.text(UiText::RecordingStoppingAlready).to_owned();
            cx.notify();
            return;
        }
        if let Some(control) = self.recording_control.as_ref() {
            match control.request_stop() {
                Ok(()) => {
                    self.recording_stopping = true;
                    self.status = format_recording_stopping(
                        locale,
                        recording_target_label(locale, control.target()),
                    );
                    self.set_tray_recording_state(
                        crate::platform::tray::TrayRecordingState::Stopping,
                    );
                }
                Err(error) => {
                    self.status = locale.format_template(
                        UiText::RecordingStopFailed,
                        &[("error", &error.to_string())],
                    )
                }
            }
            cx.notify();
            return;
        }
        if self.recording_start_in_flight {
            self.cancel_recording_start(cx);
            return;
        }
        if let Some(status) = recording_discovery_conflict_status(
            locale,
            self.recording_display_discovery_in_flight,
            self.recording_audio_discovery_in_flight,
        ) {
            self.status = status.to_owned();
            cx.notify();
            return;
        }
        if let Some(status) =
            recording_support_check_conflict_status(locale, self.recording_support_check_in_flight)
        {
            self.status = status.to_owned();
            cx.notify();
            return;
        }
        if self.recording_directory_check_in_flight {
            self.status = locale.text(UiText::RecordingWaitDirectoryCheck).to_owned();
            cx.notify();
            return;
        }
        if self.session.state() != CaptureSessionState::Idle {
            self.status = locale
                .text(UiText::RecordingFinishScreenshotFirst)
                .to_owned();
            cx.notify();
            return;
        }
        self.recording_start_in_flight = true;
        self.set_tray_recording_target(crate::platform::tray::TrayRecordingTarget::Display);
        self.set_tray_recording_state(crate::platform::tray::TrayRecordingState::Starting);
        self.status = locale.text(UiText::RecordingPreparingDisplay).to_owned();
        self.start_recording_request(
            None,
            self.recording_audio.clone(),
            self.recording_display.clone(),
            cx,
        );
    }

    /// Cancels a pending FFmpeg startup without waiting for discovery or process creation to end.
    ///
    /// The operation generation invalidates a late `RecordingControl`; dropping that control then
    /// requests its normal shutdown instead of reviving recording after the user has cancelled.
    pub(in crate::app) fn cancel_recording_start(&mut self, cx: &mut Context<Self>) {
        let Some(next_generation) = recording_start_cancellation_generation(
            self.operation_generation,
            self.recording_start_in_flight,
        ) else {
            return;
        };
        self.operation_generation = next_generation;
        self.recording_start_in_flight = false;
        self.recording_stopping = false;
        self.recording_paused = false;
        self.recording_progress = Default::default();
        self.reset_tray_recording_to_idle();
        self.status = self
            .settings
            .locale
            .text(UiText::RecordingStartupCancelled)
            .to_owned();
        cx.notify();
    }

    /// Probes FFmpeg without opening a recording process so users can fix local prerequisites first.
    pub(in crate::app) fn check_recording_support(&mut self, cx: &mut Context<Self>) {
        let locale = self.settings.locale;
        if self.recording_support_check_in_flight {
            self.status = locale.text(UiText::RecordingSupportCheckBusy).to_owned();
            cx.notify();
            return;
        }
        if self.recording_control.is_some()
            || self.recording_start_in_flight
            || self.recording_stopping
        {
            self.status = locale
                .text(UiText::RecordingStopBeforeSupportCheck)
                .to_owned();
            cx.notify();
            return;
        }
        if self.recording_directory_check_in_flight {
            self.status = locale.text(UiText::RecordingWaitDirectoryCheck).to_owned();
            cx.notify();
            return;
        }
        if let Some(status) = recording_discovery_conflict_status(
            locale,
            self.recording_display_discovery_in_flight,
            self.recording_audio_discovery_in_flight,
        ) {
            self.status = status.to_owned();
            cx.notify();
            return;
        }
        if let Some(status) =
            recording_support_check_conflict_status(locale, self.recording_support_check_in_flight)
        {
            self.status = status.to_owned();
            cx.notify();
            return;
        }
        self.recording_support_check_generation =
            self.recording_support_check_generation.wrapping_add(1);
        let generation = self.recording_support_check_generation;
        self.recording_support_check_in_flight = true;
        self.status = locale
            .text(UiText::RecordingSupportCheckInProgress)
            .to_owned();
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move { discover() })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        if this.recording_support_check_generation != generation {
                            return;
                        }
                        this.recording_support_check_in_flight = false;
                        this.status =
                            recording_support_status(this.settings.locale, result.as_ref());
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    /// Cancels the visible FFmpeg support probe while allowing its background process to finish.
    /// Advancing the generation prevents stale success or failure text from replacing the cancel.
    pub(in crate::app) fn cancel_recording_support_check(&mut self, cx: &mut Context<Self>) {
        if !self.recording_support_check_in_flight {
            return;
        }
        self.recording_support_check_generation =
            self.recording_support_check_generation.wrapping_add(1);
        self.recording_support_check_in_flight = false;
        self.status = self
            .settings
            .locale
            .text(UiText::RecordingSupportCheckCancelled)
            .to_owned();
        cx.notify();
    }

    /// Lets a user select a writable MP4 destination without relying on an environment variable.
    ///
    /// The choice is committed only after the private write probe and settings save both succeed,
    /// so cancelling the picker or selecting a read-only folder cannot break the next recording.
    pub(in crate::app) fn choose_recording_directory(&mut self, cx: &mut Context<Self>) {
        let locale = self.settings.locale;
        if let Some(directory) = recording_directory_override() {
            self.status = locale.format_template(
                UiText::RecordingDirectoryControlled,
                &[
                    ("env", RECORDING_DIRECTORY_ENV),
                    ("path", &directory.display().to_string()),
                ],
            );
            cx.notify();
            return;
        }
        if self.recording_control.is_some()
            || self.recording_start_in_flight
            || self.recording_stopping
            || self.recording_support_check_in_flight
            || self.recording_display_discovery_in_flight
            || self.recording_audio_discovery_in_flight
            || self.recording_directory_check_in_flight
        {
            self.status = locale
                .text(UiText::RecordingWaitBeforeDirectoryChange)
                .to_owned();
            cx.notify();
            return;
        }
        self.recording_directory_check_in_flight = true;
        self.status = locale.text(UiText::RecordingChooseDirectory).to_owned();
        cx.notify();
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(locale.text(UiText::RecordingChooseDirectoryPrompt).into()),
        });
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let selection = match prompt.await {
                    Ok(Ok(Some(mut paths))) => match paths.pop() {
                        Some(path) => Some(
                            cx.background_executor()
                                .spawn(async move { verify_recording_directory(path) })
                                .await,
                        ),
                        None => None,
                    },
                    Ok(Ok(None)) => None,
                    Ok(Err(error)) => Some(Err(std::io::Error::other(error))),
                    Err(error) => Some(Err(std::io::Error::other(error.to_string()))),
                };
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.recording_directory_check_in_flight = false;
                        match selection {
                            Some(Ok(directory)) => {
                                let previous = this.settings.recording_directory.clone();
                                this.settings.recording_directory = Some(directory.clone());
                                match this.settings.save(&this.settings_path) {
                                    Ok(()) => {
                                        this.status = this.settings.locale.format_template(
                                            UiText::RecordingDirectorySaved,
                                            &[("path", &directory.display().to_string())],
                                        );
                                    }
                                    Err(error) => {
                                        this.settings.recording_directory = previous;
                                        this.status = this.settings.locale.format_template(
                                            UiText::RecordingDirectorySaveFailed,
                                            &[("error", &error.to_string())],
                                        );
                                    }
                                }
                            }
                            Some(Err(error)) => {
                                this.status = this.settings.locale.format_template(
                                    UiText::RecordingDirectoryUseFailed,
                                    &[("error", &error.to_string())],
                                );
                            }
                            None => {
                                this.status = this
                                    .settings
                                    .locale
                                    .text(UiText::RecordingDirectoryUnchanged)
                                    .to_owned();
                            }
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    /// Clears the persisted MP4 folder so the next recording returns to the Windows default.
    pub(in crate::app) fn use_default_recording_directory(&mut self, cx: &mut Context<Self>) {
        let locale = self.settings.locale;
        if let Some(directory) = recording_directory_override() {
            self.status = locale.format_template(
                UiText::RecordingDirectoryControlled,
                &[
                    ("env", RECORDING_DIRECTORY_ENV),
                    ("path", &directory.display().to_string()),
                ],
            );
            cx.notify();
            return;
        }
        if self.recording_control.is_some()
            || self.recording_start_in_flight
            || self.recording_stopping
            || self.recording_support_check_in_flight
            || self.recording_display_discovery_in_flight
            || self.recording_audio_discovery_in_flight
            || self.recording_directory_check_in_flight
        {
            self.status = locale
                .text(UiText::RecordingWaitBeforeDirectoryChange)
                .to_owned();
            cx.notify();
            return;
        }
        let Some(previous) = self.settings.recording_directory.take() else {
            self.status = locale
                .text(UiText::RecordingDirectoryDefaultAlready)
                .to_owned();
            cx.notify();
            return;
        };
        self.status = match self.settings.save(&self.settings_path) {
            Ok(()) => recording_directory_for_display(None).map_or_else(
                || locale.text(UiText::RecordingDirectoryReset).to_owned(),
                |directory| {
                    locale.format_template(
                        UiText::RecordingDirectoryResetPath,
                        &[("path", &directory.display().to_string())],
                    )
                },
            ),
            Err(error) => {
                self.settings.recording_directory = Some(previous);
                locale.format_template(
                    UiText::RecordingDirectoryResetFailed,
                    &[("error", &error.to_string())],
                )
            }
        };
        cx.notify();
    }

    /// Verifies the effective MP4 folder asynchronously before the user begins a recording.
    pub(in crate::app) fn check_recording_directory(&mut self, cx: &mut Context<Self>) {
        if self.recording_directory_check_in_flight {
            return;
        }
        if self.recording_control.is_some()
            || self.recording_start_in_flight
            || self.recording_stopping
            || self.recording_support_check_in_flight
            || self.recording_display_discovery_in_flight
            || self.recording_audio_discovery_in_flight
        {
            self.status = self
                .settings
                .locale
                .text(UiText::RecordingWaitBeforeDirectoryCheck)
                .to_owned();
            cx.notify();
            return;
        }
        self.recording_directory_check_in_flight = true;
        let preferred = self.settings.recording_directory.clone();
        self.status = self
            .settings
            .locale
            .text(UiText::RecordingDirectoryCheckInProgress)
            .to_owned();
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move { recording_output_directory(preferred.as_deref()) })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.recording_directory_check_in_flight = false;
                        this.status = match result {
                            Ok(directory) => this.settings.locale.format_template(
                                UiText::RecordingDirectoryReady,
                                &[("path", &directory.display().to_string())],
                            ),
                            Err(error) => this.settings.locale.format_template(
                                UiText::RecordingDirectoryCheckFailed,
                                &[("error", &error.to_string())],
                            ),
                        };
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    /// Opens the same writable folder selected by the recording output fallback rules.
    pub(in crate::app) fn open_recording_directory(&mut self, cx: &mut Context<Self>) {
        if self.recording_directory_check_in_flight {
            return;
        }
        let preferred = self.settings.recording_directory.clone();
        self.status = match recording_output_directory(preferred.as_deref())
            .and_then(|directory| directory::open(&directory).map(|()| directory))
        {
            Ok(directory) => self.settings.locale.format_template(
                UiText::RecordingDirectoryOpened,
                &[("path", &directory.display().to_string())],
            ),
            Err(error) => self.settings.locale.format_template(
                UiText::RecordingDirectoryOpenFailed,
                &[("error", &error.to_string())],
            ),
        };
        cx.notify();
    }

    pub(in crate::app) fn start_region_recording(&mut self, cx: &mut Context<Self>) {
        let locale = self.settings.locale;
        let Some(bounds) = self.selection_drag.selection() else {
            self.status = locale.text(UiText::RecordingSelectRegion).to_owned();
            cx.notify();
            return;
        };
        if let Some(status) = recording_start_conflict_status(
            locale,
            self.recording_control.is_some(),
            self.recording_start_in_flight,
            self.recording_stopping,
        ) {
            self.status = status.to_owned();
            cx.notify();
            return;
        }
        if let Some(status) = recording_discovery_conflict_status(
            locale,
            self.recording_display_discovery_in_flight,
            self.recording_audio_discovery_in_flight,
        ) {
            self.status = status.to_owned();
            cx.notify();
            return;
        }
        if self.recording_support_check_in_flight {
            self.status = locale
                .text(UiText::RecordingSupportCheckBeforeStart)
                .to_owned();
            cx.notify();
            return;
        }
        if self.recording_directory_check_in_flight {
            self.status = locale.text(UiText::RecordingWaitDirectoryCheck).to_owned();
            cx.notify();
            return;
        }
        self.recording_start_in_flight = true;
        self.set_tray_recording_target(crate::platform::tray::TrayRecordingTarget::Region);
        self.set_tray_recording_state(crate::platform::tray::TrayRecordingState::Starting);
        self.status = locale.text(UiText::RecordingPreparingRegion).to_owned();
        self.close_capture_overlays(cx);
        let _ = self.session.cancel();
        let _ = self.session.reset();
        self.frame = None;
        self.preview = None;
        self.selection_drag.clear();
        self.annotation_document = None;
        self.annotation_history = Default::default();
        self.annotation_editor = Default::default();
        self.start_recording_request(
            Some(bounds),
            self.recording_audio.clone(),
            self.recording_display.clone(),
            cx,
        );
    }

    pub(in crate::app) fn start_selected_window_recording(&mut self, cx: &mut Context<Self>) {
        let locale = self.settings.locale;
        let Some(selection) = self.selection_drag.selection() else {
            self.status = locale.text(UiText::RecordingSelectWindow).to_owned();
            cx.notify();
            return;
        };
        if let Some(status) = recording_start_conflict_status(
            locale,
            self.recording_control.is_some(),
            self.recording_start_in_flight,
            self.recording_stopping,
        ) {
            self.status = status.to_owned();
            cx.notify();
            return;
        }
        if let Some(status) = recording_discovery_conflict_status(
            locale,
            self.recording_display_discovery_in_flight,
            self.recording_audio_discovery_in_flight,
        ) {
            self.status = status.to_owned();
            cx.notify();
            return;
        }
        if let Some(status) =
            recording_support_check_conflict_status(locale, self.recording_support_check_in_flight)
        {
            self.status = status.to_owned();
            cx.notify();
            return;
        }
        if self.recording_directory_check_in_flight {
            self.status = locale.text(UiText::RecordingWaitDirectoryCheck).to_owned();
            cx.notify();
            return;
        }
        let center = crate::domain::geometry::PhysicalPoint {
            x: selection.left + selection.width() as i32 / 2,
            y: selection.top + selection.height() as i32 / 2,
        };
        self.recording_start_in_flight = true;
        self.set_tray_recording_target(crate::platform::tray::TrayRecordingTarget::Window);
        self.set_tray_recording_state(crate::platform::tray::TrayRecordingState::Starting);
        self.status = locale.text(UiText::RecordingResolvingWindow).to_owned();
        self.close_capture_overlays(cx);
        let _ = self.session.cancel();
        let _ = self.session.reset();
        self.frame = None;
        self.preview = None;
        self.selection_drag.clear();
        let generation = self.operation_generation;
        let audio = self.recording_audio.clone();
        let display = self.recording_display.clone();
        let recording_directory = self.settings.recording_directory.clone();
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        // GPU-backed windows can return black pixels through gdigrab's
                        // title input. Resolve the actual visible window bounds so recording
                        // follows the desktop capture path that users see on screen.
                        let target = SystemWindowInspector
                            .window_capture_target_at(center)?
                            .ok_or_else(|| {
                                std::io::Error::new(
                                    std::io::ErrorKind::NotFound,
                                    "no recordable top-level window at the selected area",
                                )
                            })?;
                        start_recording_target(
                            Some(RecordingTarget::Window {
                                title: target.title,
                                bounds: target.bounds,
                            }),
                            audio,
                            display,
                            recording_directory,
                        )
                    })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.recording_started(result, generation, cx)
                    });
                }
            }
        })
        .detach();
    }

    fn start_recording_request(
        &mut self,
        region: Option<PhysicalRect>,
        audio: RecordingAudioSelection,
        display: RecordingDisplaySelection,
        cx: &mut Context<Self>,
    ) {
        let generation = self.operation_generation;
        let recording_directory = self.settings.recording_directory.clone();
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        start_recording_target(
                            region.map(|bounds| RecordingTarget::Region { bounds }),
                            audio,
                            display,
                            recording_directory,
                        )
                    })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.recording_started(result, generation, cx)
                    });
                }
            }
        })
        .detach();
    }

    pub(in crate::app) fn cycle_recording_display(&mut self, cx: &mut Context<Self>) {
        if self.recording_control.is_some()
            || self.recording_start_in_flight
            || self.recording_stopping
            || self.recording_display_discovery_in_flight
            || self.recording_audio_discovery_in_flight
            || self.recording_support_check_in_flight
            || self.recording_directory_check_in_flight
        {
            return;
        }
        self.recording_display_discovery_in_flight = true;
        self.status = self
            .settings
            .locale
            .text(UiText::RecordingDisplayDiscoveryInProgress)
            .to_owned();
        let current = self.recording_display.clone();
        let generation = self.operation_generation;
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move { SystemDisplayProvider.displays() })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.finish_recording_display_discovery(current, result, generation, cx)
                    });
                }
            }
        })
        .detach();
    }

    fn finish_recording_display_discovery(
        &mut self,
        current: RecordingDisplaySelection,
        result: std::io::Result<Vec<crate::platform::display::DisplayInfo>>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if !recording_discovery_result_is_applicable(
            self.operation_generation,
            generation,
            self.recording_control.is_some(),
            self.recording_start_in_flight,
            self.recording_stopping,
        ) {
            return;
        }
        self.recording_display_discovery_in_flight = false;
        match result {
            Ok(displays) => {
                self.recording_display = next_recording_display_selection(current, &displays);
                self.status = self.settings.locale.format_template(
                    UiText::RecordingDisplayChanged,
                    &[(
                        "display",
                        &recording_display_selection_label(
                            self.settings.locale,
                            &self.recording_display,
                        ),
                    )],
                );
            }
            Err(error) => {
                self.status = self.settings.locale.format_template(
                    UiText::RecordingDisplayDiscoveryFailed,
                    &[("error", &error.to_string())],
                )
            }
        }
        cx.notify();
    }

    pub(in crate::app) fn cycle_recording_audio(&mut self, cx: &mut Context<Self>) {
        if self.recording_control.is_some()
            || self.recording_start_in_flight
            || self.recording_stopping
            || self.recording_audio_discovery_in_flight
            || self.recording_display_discovery_in_flight
            || self.recording_support_check_in_flight
            || self.recording_directory_check_in_flight
        {
            return;
        }
        self.recording_audio_discovery_in_flight = true;
        self.status = self
            .settings
            .locale
            .text(UiText::RecordingAudioDiscoveryInProgress)
            .to_owned();
        let current = self.recording_audio.clone();
        let generation = self.operation_generation;
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move { discover_audio_sources() })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.finish_recording_audio_discovery(current, result, generation, cx)
                    });
                }
            }
        })
        .detach();
    }

    fn finish_recording_audio_discovery(
        &mut self,
        current: RecordingAudioSelection,
        result: std::io::Result<Vec<AudioSource>>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if !recording_discovery_result_is_applicable(
            self.operation_generation,
            generation,
            self.recording_control.is_some(),
            self.recording_start_in_flight,
            self.recording_stopping,
        ) {
            return;
        }
        self.recording_audio_discovery_in_flight = false;
        match result {
            Ok(sources) => {
                self.recording_audio = next_recording_audio_selection(current, &sources);
                self.status = self.settings.locale.format_template(
                    UiText::RecordingAudioChanged,
                    &[(
                        "audio",
                        &recording_audio_selection_label(
                            self.settings.locale,
                            &self.recording_audio,
                        ),
                    )],
                );
            }
            Err(error) => {
                self.status = self.settings.locale.format_template(
                    UiText::RecordingAudioDiscoveryFailed,
                    &[("error", &error.to_string())],
                )
            }
        }
        cx.notify();
    }

    pub(in crate::app) fn toggle_recording_pause(&mut self, cx: &mut Context<Self>) {
        let locale = self.settings.locale;
        if self.recording_stopping {
            self.status = locale.text(UiText::RecordingStoppingAlready).to_owned();
            cx.notify();
            return;
        }
        let Some(control) = self.recording_control.as_ref() else {
            return;
        };
        let paused = !self.recording_paused;
        match control.set_paused(paused) {
            Ok(()) => {
                self.set_tray_recording_state(if paused {
                    crate::platform::tray::TrayRecordingState::Pausing
                } else {
                    crate::platform::tray::TrayRecordingState::Resuming
                });
                self.status = if paused {
                    locale.text(UiText::RecordingPausing).to_owned()
                } else {
                    locale.text(UiText::RecordingResuming).to_owned()
                }
            }
            Err(error) => {
                self.status = locale.format_template(
                    UiText::RecordingPauseFailed,
                    &[("error", &error.to_string())],
                )
            }
        }
        cx.notify();
    }

    fn recording_started(
        &mut self,
        result: std::io::Result<crate::recording::RecordingControl>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if !recording_start_result_is_applicable(
            self.operation_generation,
            generation,
            self.recording_start_in_flight,
        ) {
            drop(result);
            return;
        }
        match result {
            Ok(control) => {
                let events = control.events();
                let locale = self.settings.locale;
                let target = recording_target_label(locale, control.target());
                self.set_tray_recording_target(tray_recording_target(control.target()));
                self.recording_control = Some(control);
                self.recording_stopping = false;
                self.set_tray_recording_state(crate::platform::tray::TrayRecordingState::Starting);
                self.recording_progress = Default::default();
                self.recording_paused = false;
                self.status =
                    locale.format_template(UiText::RecordingStarting, &[("target", target)]);
                cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                    let mut cx = cx.clone();
                    async move {
                        while let Ok(event) = events.recv().await {
                            let Some(this) = this.upgrade() else {
                                break;
                            };
                            this.update(&mut cx, |this, cx| this.handle_recording_event(event, cx));
                        }
                    }
                })
                .detach();
            }
            Err(error) => {
                log::warn!(target: "flash_shot::recording", "recording_start_failed error={error}");
                self.status = recording_start_failure_status(self.settings.locale, &error);
                self.recording_stopping = false;
                self.reset_tray_recording_to_idle();
            }
        }
        self.recording_start_in_flight = false;
        cx.notify();
    }

    fn handle_recording_event(&mut self, event: RecordingEvent, cx: &mut Context<Self>) {
        let locale = self.settings.locale;
        let target = self
            .recording_control
            .as_ref()
            .map(|control| recording_target_label(locale, control.target()))
            .unwrap_or_else(|| locale.text(UiText::RecordingTargetScreen));
        match event {
            RecordingEvent::Started => {
                self.recording_stopping = false;
                self.status =
                    locale.format_template(UiText::RecordingActive, &[("target", target)]);
                self.set_tray_recording_state(crate::platform::tray::TrayRecordingState::Recording);
            }
            RecordingEvent::Paused => {
                self.recording_paused = true;
                self.set_tray_recording_state(crate::platform::tray::TrayRecordingState::Paused);
                self.status =
                    locale.format_template(UiText::RecordingPaused, &[("target", target)]);
            }
            RecordingEvent::Resumed => {
                self.recording_paused = false;
                self.set_tray_recording_state(crate::platform::tray::TrayRecordingState::Recording);
                self.status =
                    locale.format_template(UiText::RecordingActive, &[("target", target)]);
            }
            RecordingEvent::Progress(progress) => {
                self.recording_progress = progress;
                self.status = if self.recording_stopping {
                    format_recording_stopping(locale, target)
                } else {
                    format_recording_progress(locale, target, progress)
                };
            }
            RecordingEvent::Finished { output } => {
                self.recording_control = None;
                self.recording_stopping = false;
                self.reset_tray_recording_to_idle();
                self.recording_progress = Default::default();
                self.recording_paused = false;
                self.status = locale.format_template(
                    UiText::RecordingSaved,
                    &[("path", &output.display().to_string())],
                );
                self.notify_user(
                    locale.text(UiText::AppName),
                    locale.text(UiText::RecordingSavedNotification),
                );
            }
            RecordingEvent::Failed { message } => {
                self.recording_control = None;
                self.recording_stopping = false;
                self.reset_tray_recording_to_idle();
                self.recording_progress = Default::default();
                self.recording_paused = false;
                self.status =
                    locale.format_template(UiText::RecordingFailed, &[("error", &message)]);
            }
        }
        cx.notify();
    }
}

pub(super) fn start_recording_target(
    target: Option<RecordingTarget>,
    audio_selection: RecordingAudioSelection,
    display_selection: RecordingDisplaySelection,
    recording_directory: Option<PathBuf>,
) -> std::io::Result<crate::recording::RecordingControl> {
    let capabilities = discover()?;
    let audio = match audio_selection {
        RecordingAudioSelection::Automatic => {
            RecordingAudioConfig::from_environment()?.source().cloned()
        }
        RecordingAudioSelection::Disabled => None,
        RecordingAudioSelection::Source(source) => Some(source),
    };
    let target = match target {
        Some(target) => target,
        None => recording_display_target(&display_selection)?,
    };
    let output = recording_output_path(recording_directory.as_deref())?;
    start_recording(
        capabilities,
        RecordingRequest {
            target,
            audio,
            frame_rate: 30,
            output,
        },
    )
}

pub(super) fn recording_display_target(
    selection: &RecordingDisplaySelection,
) -> std::io::Result<RecordingTarget> {
    let displays = SystemDisplayProvider.displays()?;
    let display = match selection {
        RecordingDisplaySelection::Primary => displays.into_iter().find(|display| display.primary),
        RecordingDisplaySelection::Display { id, .. } => {
            displays.into_iter().find(|display| display.id == *id)
        }
    }
    .ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "selected display not found")
    })?;
    Ok(RecordingTarget::Display {
        bounds: display.physical_bounds,
    })
}

const RECORDING_DIRECTORY_ENV: &str = "FLASH_SHOT_RECORDING_DIRECTORY";

/// Chooses a writable recording directory before FFmpeg starts writing its MP4.
///
/// A folder selected in Settings is tried before the user's Videos folder and Flash Shot's
/// application-data fallback. An explicit environment override remains authoritative and returns
/// its own error instead of silently redirecting a recording elsewhere.
pub(super) fn recording_output_path(
    preferred_directory: Option<&Path>,
) -> std::io::Result<PathBuf> {
    let timestamp_ms = unix_timestamp_ms();
    if let Some(directory) = recording_directory_override() {
        return recording_output_path_in(&directory, timestamp_ms);
    }
    let candidates = recording_directory_candidates(preferred_directory);
    recording_output_path_from_candidates(&candidates, timestamp_ms)
}

/// Returns the folder shown in Settings without creating it on the UI thread.
pub(in crate::app) fn recording_directory_for_display(
    preferred_directory: Option<&Path>,
) -> Option<PathBuf> {
    recording_directory_override().or_else(|| {
        recording_directory_candidates(preferred_directory)
            .into_iter()
            .next()
    })
}

/// Resolves and probes the folder that a new recording would use.
fn recording_output_directory(preferred_directory: Option<&Path>) -> std::io::Result<PathBuf> {
    if let Some(directory) = recording_directory_override() {
        return verify_recording_directory(directory);
    }
    recording_output_directory_from_candidates(&recording_directory_candidates(preferred_directory))
}

/// Keeps explicit settings ahead of conventional Windows and application-data fallbacks.
pub(super) fn recording_directory_candidates(preferred_directory: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = Vec::with_capacity(3);
    if let Some(directory) = preferred_directory {
        candidates.push(directory.to_owned());
    }
    if let Some(videos) = directories::UserDirs::new()
        .and_then(|directories| directories.video_dir().map(Path::to_owned))
    {
        let directory = videos.join("Flash Shot");
        if !candidates.contains(&directory) {
            candidates.push(directory);
        }
    }
    if let Ok(paths) = crate::diagnostics::AppPaths::discover() {
        let directory = paths.data_dir.join("recordings");
        if !candidates.contains(&directory) {
            candidates.push(directory);
        }
    }
    candidates
}

fn recording_directory_override() -> Option<PathBuf> {
    std::env::var_os(RECORDING_DIRECTORY_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Tries recording roots in preference order and retains every failure when none are writable.
pub(super) fn recording_output_path_from_candidates(
    candidates: &[PathBuf],
    timestamp_ms: u128,
) -> std::io::Result<PathBuf> {
    let directory = recording_output_directory_from_candidates(candidates)?;
    Ok(next_recording_output_path(
        &directory,
        timestamp_ms,
        Path::exists,
    ))
}

/// Tries recording roots in preference order and retains every failure when none are writable.
fn recording_output_directory_from_candidates(candidates: &[PathBuf]) -> std::io::Result<PathBuf> {
    let mut failures = Vec::with_capacity(candidates.len());
    for directory in candidates {
        match verify_recording_directory(directory.to_owned()) {
            Ok(directory) => return Ok(directory),
            Err(error) => failures.push(format!("{}: {error}", directory.display())),
        }
    }
    if failures.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "recording directory unavailable",
        ));
    }
    Err(std::io::Error::other(format!(
        "no writable recording directory: {}",
        failures.join("; ")
    )))
}

/// Creates and probes one directory, returning the final timestamped MP4 path only when writable.
fn recording_output_path_in(directory: &Path, timestamp_ms: u128) -> std::io::Result<PathBuf> {
    let directory = verify_recording_directory(directory.to_owned())?;
    Ok(next_recording_output_path(
        &directory,
        timestamp_ms,
        Path::exists,
    ))
}

/// Selects a new MP4 name without replacing a capture created in the same millisecond.
///
/// The recording command also uses FFmpeg's `-n` flag, which keeps this best-effort name choice
/// safe when another process creates the same path between the existence check and process start.
fn next_recording_output_path(
    directory: &Path,
    timestamp_ms: u128,
    exists: impl Fn(&Path) -> bool,
) -> PathBuf {
    let stem = format!("FlashShot-{timestamp_ms}");
    let initial = directory.join(format!("{stem}.mp4"));
    if !exists(&initial) {
        return initial;
    }
    for index in 2_u32.. {
        let path = directory.join(format!("{stem}-{index}.mp4"));
        if !exists(&path) {
            return path;
        }
    }
    unreachable!("u32 recording filename suffixes cannot be exhausted")
}

/// Creates and probes a candidate without leaving a test file in the user's video folder.
fn verify_recording_directory(directory: PathBuf) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(&directory)?;
    crate::history::verify_writable_directory(&directory)?;
    Ok(directory)
}

pub(super) fn recording_target_label(locale: Locale, target: &RecordingTarget) -> &'static str {
    match target {
        RecordingTarget::Display { .. } => locale.text(UiText::RecordingTargetDisplay),
        RecordingTarget::Window { .. } => locale.text(UiText::RecordingTargetWindow),
        RecordingTarget::Region { .. } => locale.text(UiText::RecordingTargetRegion),
    }
}

/// Converts a live recording request into the compact target vocabulary shown by the tray.
fn tray_recording_target(target: &RecordingTarget) -> crate::platform::tray::TrayRecordingTarget {
    match target {
        RecordingTarget::Display { .. } => crate::platform::tray::TrayRecordingTarget::Display,
        RecordingTarget::Window { .. } => crate::platform::tray::TrayRecordingTarget::Window,
        RecordingTarget::Region { .. } => crate::platform::tray::TrayRecordingTarget::Region,
    }
}

pub(in crate::app) fn next_recording_audio_selection(
    current: RecordingAudioSelection,
    sources: &[AudioSource],
) -> RecordingAudioSelection {
    let mut selections = Vec::with_capacity(sources.len() + 2);
    selections.push(RecordingAudioSelection::Automatic);
    selections.push(RecordingAudioSelection::Disabled);
    selections.extend(sources.iter().cloned().map(RecordingAudioSelection::Source));
    let index = selections
        .iter()
        .position(|selection| selection == &current)
        .map(|index| (index + 1) % selections.len())
        .unwrap_or(1);
    selections[index].clone()
}

pub(in crate::app) fn recording_audio_selection_label(
    locale: Locale,
    selection: &RecordingAudioSelection,
) -> String {
    match selection {
        RecordingAudioSelection::Automatic => {
            locale.text(UiText::RecordingAudioAutomatic).to_owned()
        }
        RecordingAudioSelection::Disabled => locale.text(UiText::RecordingAudioDisabled).to_owned(),
        RecordingAudioSelection::Source(AudioSource::Microphone { device }) => locale
            .format_template(
                UiText::RecordingAudioMicrophone,
                &[("device", &truncate_recording_audio_label(device))],
            ),
        RecordingAudioSelection::Source(AudioSource::SystemAudio { .. }) => {
            locale.text(UiText::RecordingAudioSystem).to_owned()
        }
    }
}

pub(in crate::app) fn next_recording_display_selection(
    current: RecordingDisplaySelection,
    displays: &[crate::platform::display::DisplayInfo],
) -> RecordingDisplaySelection {
    let mut displays = displays.to_vec();
    displays.sort_by(|left, right| {
        (
            !left.primary,
            left.physical_bounds.left,
            left.physical_bounds.top,
            &left.id,
        )
            .cmp(&(
                !right.primary,
                right.physical_bounds.left,
                right.physical_bounds.top,
                &right.id,
            ))
    });
    let mut selections = Vec::with_capacity(displays.len() + 1);
    selections.push(RecordingDisplaySelection::Primary);
    selections.extend(displays.iter().enumerate().map(|(index, display)| {
        RecordingDisplaySelection::Display {
            id: display.id.clone(),
            label: format!(
                "{} ({}x{})",
                index + 1,
                display.physical_bounds.width(),
                display.physical_bounds.height()
            ),
        }
    }));
    let index = selections
        .iter()
        .position(|selection| selection == &current)
        .map(|index| (index + 1) % selections.len())
        .unwrap_or(1.min(selections.len().saturating_sub(1)));
    selections[index].clone()
}

pub(in crate::app) fn recording_display_selection_label(
    locale: Locale,
    selection: &RecordingDisplaySelection,
) -> String {
    match selection {
        RecordingDisplaySelection::Primary => {
            locale.text(UiText::RecordingDisplayPrimary).to_owned()
        }
        RecordingDisplaySelection::Display { label, .. } => {
            locale.format_template(UiText::RecordingDisplayLabel, &[("label", label)])
        }
    }
}

pub(super) fn truncate_recording_audio_label(label: &str) -> String {
    const MAX_CHARS: usize = 20;
    let mut result: String = label.chars().take(MAX_CHARS).collect();
    if label.chars().nth(MAX_CHARS).is_some() {
        result.push_str("...");
    }
    result
}

pub(super) fn format_recording_progress(
    locale: Locale,
    target: &str,
    progress: RecordingProgress,
) -> String {
    let seconds = progress.output_time_us.unwrap_or_default() / 1_000_000;
    let frames = progress.frame.unwrap_or_default();
    locale.format_template(
        UiText::RecordingProgress,
        &[
            ("target", target),
            ("seconds", &seconds.to_string()),
            ("frames", &frames.to_string()),
        ],
    )
}

/// Maps FFmpeg startup failures to the concrete local recovery step while retaining diagnostics.
pub(super) fn recording_start_failure_status(locale: Locale, error: &std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::NotFound => locale.format_template(
            UiText::RecordingStartFailureMissingFfmpeg,
            &[("error", &error.to_string())],
        ),
        std::io::ErrorKind::Unsupported => locale.format_template(
            UiText::RecordingStartFailureUnsupported,
            &[("error", &error.to_string())],
        ),
        _ => locale.format_template(
            UiText::RecordingStartFailureGeneric,
            &[("error", &error.to_string())],
        ),
    }
}

/// Keeps the user-facing status in the stopping phase while late FFmpeg progress frames arrive.
pub(super) fn format_recording_stopping(locale: Locale, target: &str) -> String {
    locale.format_template(UiText::RecordingStopping, &[("target", target)])
}

/// Names the safe next action when a new overlay recording would overlap an active lifecycle.
pub(super) fn recording_start_conflict_status(
    locale: Locale,
    recording_active: bool,
    recording_starting: bool,
    recording_stopping: bool,
) -> Option<&'static str> {
    if recording_stopping {
        Some(locale.text(UiText::RecordingStartConflictStopping))
    } else if recording_active {
        Some(locale.text(UiText::RecordingStartConflictActive))
    } else if recording_starting {
        Some(locale.text(UiText::RecordingStartConflictStarting))
    } else {
        None
    }
}

/// Prevents a recording from starting while display or audio source discovery is still changing
/// the selected input; the async completion must settle before a request can snapshot it.
pub(super) fn recording_discovery_conflict_status(
    locale: Locale,
    display_discovery_in_flight: bool,
    audio_discovery_in_flight: bool,
) -> Option<&'static str> {
    (display_discovery_in_flight || audio_discovery_in_flight)
        .then_some(locale.text(UiText::RecordingDiscoveryConflict))
}

/// Blocks recording starts while the local FFmpeg capability probe owns the same inputs.
pub(super) fn recording_support_check_conflict_status(
    locale: Locale,
    support_check_in_flight: bool,
) -> Option<&'static str> {
    support_check_in_flight.then_some(locale.text(UiText::RecordingSupportCheckConflict))
}

/// Accepts a completed FFmpeg startup only while the original startup request is still current.
///
/// If the user cancels the capture flow or starts a new workflow before FFmpeg answers, dropping
/// the stale `RecordingControl` requests a normal stop instead of replacing the current UI state.
pub(super) fn recording_start_result_is_applicable(
    current_operation: u64,
    completed_operation: u64,
    recording_starting: bool,
) -> bool {
    is_current_operation(current_operation, completed_operation) && recording_starting
}

/// Advances the shared operation generation only when a recording startup is still cancellable.
///
/// The caller stores the returned token before clearing its busy flag, which makes every late
/// FFmpeg result fail `recording_start_result_is_applicable` and lets its control shut down.
pub(super) const fn recording_start_cancellation_generation(
    current_operation: u64,
    recording_starting: bool,
) -> Option<u64> {
    if recording_starting {
        Some(current_operation.wrapping_add(1))
    } else {
        None
    }
}

/// Accepts a discovery result only for the current workflow while no recording lifecycle owns the
/// selected input; stale work leaves any newer discovery request untouched.
pub(super) fn recording_discovery_result_is_applicable(
    current_operation: u64,
    completed_operation: u64,
    recording_active: bool,
    recording_starting: bool,
    recording_stopping: bool,
) -> bool {
    is_current_operation(current_operation, completed_operation)
        && !recording_active
        && !recording_starting
        && !recording_stopping
}

/// Converts a read-only FFmpeg probe into an actionable recording readiness message.
pub(super) fn recording_support_status(
    locale: Locale,
    result: Result<&crate::recording::FfmpegCapabilities, &std::io::Error>,
) -> String {
    match result {
        Ok(capabilities) if capabilities.supports_display_capture() => locale.format_template(
            UiText::RecordingSupportReady,
            &[
                ("version", &capabilities.version_label()),
                (
                    "backend",
                    if capabilities.supports_input("ddagrab") {
                        "DDagrab"
                    } else {
                        "GDI"
                    },
                ),
            ],
        ),
        Ok(capabilities) => locale.format_template(
            UiText::RecordingSupportDesktopUnavailable,
            &[("version", &capabilities.version_label())],
        ),
        Err(error) => recording_start_failure_status(locale, error),
    }
}
