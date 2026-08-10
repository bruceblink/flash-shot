//! Screen-recording workflow orchestration.

use super::*;

impl FlashShotApp {
    /// Owns the display recording command lifecycle and keeps repeated stop/pause input harmless
    /// while FFmpeg finishes its graceful container shutdown.
    pub(in crate::app) fn toggle_display_recording(&mut self, cx: &mut Context<Self>) {
        if self.recording_stopping {
            self.status = "Screen recording is already stopping...".to_owned();
            cx.notify();
            return;
        }
        if let Some(control) = self.recording_control.as_ref() {
            match control.request_stop() {
                Ok(()) => {
                    self.recording_stopping = true;
                    self.status = "Stopping screen recording...".to_owned();
                    self.set_tray_recording_state(
                        crate::platform::tray::TrayRecordingState::Stopping,
                    );
                }
                Err(error) => self.status = format!("Could not stop screen recording: {error}"),
            }
            cx.notify();
            return;
        }
        if self.recording_start_in_flight {
            self.status = "Screen recording startup is already in progress...".to_owned();
            cx.notify();
            return;
        }
        if self.session.state() != CaptureSessionState::Idle {
            self.status = "Finish or cancel the current screenshot before recording".to_owned();
            cx.notify();
            return;
        }
        self.recording_start_in_flight = true;
        self.set_tray_recording_state(crate::platform::tray::TrayRecordingState::Starting);
        self.status = "Discovering FFmpeg and preparing display recording...".to_owned();
        self.start_recording_request(
            None,
            self.recording_audio.clone(),
            self.recording_display.clone(),
            cx,
        );
    }

    /// Probes FFmpeg without opening a recording process so users can fix local prerequisites first.
    pub(in crate::app) fn check_recording_support(&mut self, cx: &mut Context<Self>) {
        if self.recording_control.is_some()
            || self.recording_start_in_flight
            || self.recording_stopping
        {
            self.status = "Stop the current recording before checking support".to_owned();
            cx.notify();
            return;
        }
        self.status = "Checking FFmpeg recording support...".to_owned();
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
                        this.status = recording_support_status(result.as_ref());
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(in crate::app) fn start_region_recording(&mut self, cx: &mut Context<Self>) {
        let Some(bounds) = self.selection_drag.selection() else {
            self.status = "Select a region before starting a recording".to_owned();
            cx.notify();
            return;
        };
        if let Some(status) = recording_start_conflict_status(
            self.recording_control.is_some(),
            self.recording_start_in_flight,
            self.recording_stopping,
        ) {
            self.status = status.to_owned();
            cx.notify();
            return;
        }
        self.recording_start_in_flight = true;
        self.set_tray_recording_state(crate::platform::tray::TrayRecordingState::Starting);
        self.status = "Preparing region recording...".to_owned();
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
        let Some(selection) = self.selection_drag.selection() else {
            self.status = "Select a window before starting a recording".to_owned();
            cx.notify();
            return;
        };
        if let Some(status) = recording_start_conflict_status(
            self.recording_control.is_some(),
            self.recording_start_in_flight,
            self.recording_stopping,
        ) {
            self.status = status.to_owned();
            cx.notify();
            return;
        }
        let center = crate::domain::geometry::PhysicalPoint {
            x: selection.left + selection.width() as i32 / 2,
            y: selection.top + selection.height() as i32 / 2,
        };
        self.recording_start_in_flight = true;
        self.set_tray_recording_state(crate::platform::tray::TrayRecordingState::Starting);
        self.status = "Looking up selected window for recording...".to_owned();
        self.close_capture_overlays(cx);
        let _ = self.session.cancel();
        let _ = self.session.reset();
        self.frame = None;
        self.preview = None;
        self.selection_drag.clear();
        let audio = self.recording_audio.clone();
        let display = self.recording_display.clone();
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result =
                    cx.background_executor()
                        .spawn(async move {
                            let title = SystemWindowInspector.window_title_at(center)?.ok_or_else(
                                || {
                                    std::io::Error::new(
                                        std::io::ErrorKind::NotFound,
                                        "no recordable top-level window at the selected area",
                                    )
                                },
                            )?;
                            start_recording_target(
                                Some(RecordingTarget::Window { title }),
                                audio,
                                display,
                            )
                        })
                        .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| this.recording_started(result, cx));
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
                        )
                    })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| this.recording_started(result, cx));
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
        {
            return;
        }
        self.recording_display_discovery_in_flight = true;
        self.status = "Discovering displays for recording...".to_owned();
        let current = self.recording_display.clone();
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
                        this.finish_recording_display_discovery(current, result, cx)
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
        cx: &mut Context<Self>,
    ) {
        self.recording_display_discovery_in_flight = false;
        match result {
            Ok(displays) => {
                self.recording_display = next_recording_display_selection(current, &displays);
                self.status = format!(
                    "Recording display: {}",
                    recording_display_selection_label(&self.recording_display)
                );
            }
            Err(error) => self.status = format!("Could not discover displays: {error}"),
        }
        cx.notify();
    }

    pub(in crate::app) fn cycle_recording_audio(&mut self, cx: &mut Context<Self>) {
        if self.recording_control.is_some()
            || self.recording_start_in_flight
            || self.recording_stopping
            || self.recording_audio_discovery_in_flight
        {
            return;
        }
        self.recording_audio_discovery_in_flight = true;
        self.status = "Discovering recording audio sources...".to_owned();
        let current = self.recording_audio.clone();
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
                        this.finish_recording_audio_discovery(current, result, cx)
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
        cx: &mut Context<Self>,
    ) {
        self.recording_audio_discovery_in_flight = false;
        match result {
            Ok(sources) => {
                self.recording_audio = next_recording_audio_selection(current, &sources);
                self.status = format!(
                    "Recording audio: {}",
                    recording_audio_selection_label(&self.recording_audio)
                );
            }
            Err(error) => self.status = format!("Could not discover recording audio: {error}"),
        }
        cx.notify();
    }

    pub(in crate::app) fn toggle_recording_pause(&mut self, cx: &mut Context<Self>) {
        if self.recording_stopping {
            self.status = "Screen recording is already stopping...".to_owned();
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
                    "Pausing screen recording...".to_owned()
                } else {
                    "Resuming screen recording...".to_owned()
                }
            }
            Err(error) => self.status = format!("Could not change recording pause state: {error}"),
        }
        cx.notify();
    }

    fn recording_started(
        &mut self,
        result: std::io::Result<crate::recording::RecordingControl>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(control) => {
                let events = control.events();
                let target = recording_target_label(control.target());
                self.recording_control = Some(control);
                self.recording_stopping = false;
                self.set_tray_recording_state(crate::platform::tray::TrayRecordingState::Starting);
                self.recording_progress = Default::default();
                self.recording_paused = false;
                self.status = format!("Starting {target} recording...");
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
                self.status = recording_start_failure_status(&error);
                self.recording_stopping = false;
                self.set_tray_recording_state(crate::platform::tray::TrayRecordingState::Idle);
            }
        }
        self.recording_start_in_flight = false;
        cx.notify();
    }

    fn handle_recording_event(&mut self, event: RecordingEvent, cx: &mut Context<Self>) {
        let target = self
            .recording_control
            .as_ref()
            .map(|control| recording_target_label(control.target()))
            .unwrap_or("screen");
        match event {
            RecordingEvent::Started => {
                self.recording_stopping = false;
                self.status = format!("Recording {target}...");
                self.set_tray_recording_state(crate::platform::tray::TrayRecordingState::Recording);
            }
            RecordingEvent::Paused => {
                self.recording_paused = true;
                self.set_tray_recording_state(crate::platform::tray::TrayRecordingState::Paused);
                self.status = format!("{target} recording paused");
            }
            RecordingEvent::Resumed => {
                self.recording_paused = false;
                self.set_tray_recording_state(crate::platform::tray::TrayRecordingState::Recording);
                self.status = format!("Recording {target}...");
            }
            RecordingEvent::Progress(progress) => {
                self.recording_progress = progress;
                self.status = format_recording_progress(target, progress);
            }
            RecordingEvent::Finished { output } => {
                self.recording_control = None;
                self.recording_stopping = false;
                self.set_tray_recording_state(crate::platform::tray::TrayRecordingState::Idle);
                self.recording_progress = Default::default();
                self.recording_paused = false;
                self.status = format!("Screen recording saved to {}", output.display());
                self.notify_user("Flash Shot", "Screen recording saved");
            }
            RecordingEvent::Failed { message } => {
                self.recording_control = None;
                self.recording_stopping = false;
                self.set_tray_recording_state(crate::platform::tray::TrayRecordingState::Idle);
                self.recording_progress = Default::default();
                self.recording_paused = false;
                self.status = format!("Screen recording failed: {message}");
            }
        }
        cx.notify();
    }
}

pub(super) fn start_recording_target(
    target: Option<RecordingTarget>,
    audio_selection: RecordingAudioSelection,
    display_selection: RecordingDisplaySelection,
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
    let output = recording_output_path()?;
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

pub(super) fn recording_output_path() -> std::io::Result<PathBuf> {
    let root = directories::UserDirs::new()
        .and_then(|directories| directories.video_dir().map(Path::to_owned))
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "recording directory unavailable",
            )
        })?;
    let directory = root.join("Flash Shot");
    std::fs::create_dir_all(&directory)?;
    Ok(directory.join(format!("FlashShot-{}.mp4", unix_timestamp_ms())))
}

pub(super) fn recording_target_label(target: &RecordingTarget) -> &'static str {
    match target {
        RecordingTarget::Display { .. } => "display",
        RecordingTarget::Window { .. } => "window",
        RecordingTarget::Region { .. } => "selected area",
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
    selection: &RecordingAudioSelection,
) -> String {
    match selection {
        RecordingAudioSelection::Automatic => "auto".to_owned(),
        RecordingAudioSelection::Disabled => "off".to_owned(),
        RecordingAudioSelection::Source(AudioSource::Microphone { device }) => {
            format!("mic: {}", truncate_recording_audio_label(device))
        }
        RecordingAudioSelection::Source(AudioSource::SystemAudio { .. }) => {
            "system audio".to_owned()
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
    selection: &RecordingDisplaySelection,
) -> String {
    match selection {
        RecordingDisplaySelection::Primary => "primary".to_owned(),
        RecordingDisplaySelection::Display { label, .. } => format!("display {label}"),
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

pub(super) fn format_recording_progress(target: &str, progress: RecordingProgress) -> String {
    let seconds = progress.output_time_us.unwrap_or_default() / 1_000_000;
    let frames = progress.frame.unwrap_or_default();
    format!("Recording {target}: {seconds}s, {frames} frames")
}

/// Maps FFmpeg startup failures to the concrete local recovery step while retaining diagnostics.
pub(super) fn recording_start_failure_status(error: &std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::NotFound => format!(
            "Recording is unavailable because FFmpeg was not found. Install FFmpeg or set FLASH_SHOT_FFMPEG: {error}"
        ),
        std::io::ErrorKind::Unsupported => format!(
            "This FFmpeg build cannot record the selected source. Use a build with ddagrab or gdigrab: {error}"
        ),
        _ => format!("Could not start screen recording: {error}"),
    }
}

/// Names the safe next action when a new overlay recording would overlap an active lifecycle.
pub(super) fn recording_start_conflict_status(
    recording_active: bool,
    recording_starting: bool,
    recording_stopping: bool,
) -> Option<&'static str> {
    if recording_stopping {
        Some("Screen recording is already stopping...")
    } else if recording_active {
        Some("Stop the current recording before starting another")
    } else if recording_starting {
        Some("Screen recording startup is already in progress...")
    } else {
        None
    }
}

/// Converts a read-only FFmpeg probe into an actionable recording readiness message.
pub(super) fn recording_support_status(
    result: Result<&crate::recording::FfmpegCapabilities, &std::io::Error>,
) -> String {
    match result {
        Ok(capabilities) if capabilities.supports_display_capture() => format!(
            "Recording ready: FFmpeg {} supports {}",
            capabilities.version(),
            if capabilities.supports_input("ddagrab") {
                "Desktop Duplication"
            } else {
                "GDI capture"
            }
        ),
        Ok(capabilities) => format!(
            "FFmpeg {} is installed but cannot capture the desktop. Use a build with ddagrab or gdigrab.",
            capabilities.version()
        ),
        Err(error) => recording_start_failure_status(error),
    }
}
