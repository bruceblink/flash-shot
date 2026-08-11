//! Capture, selection, and clipboard workflow orchestration.

mod annotation;
mod capture;
mod exporting;
mod file_io;
mod images;
mod pinning;
mod recognition;
mod recording;
mod scrolling;
mod settings;
mod support;
mod windowing;

use file_io::*;
pub(super) use settings::shortcut_option_label;
pub(super) use support::*;
use windowing::*;

#[cfg(test)]
pub(in crate::app::workflow) use capture::{
    capture_session_can_restart, capture_start_conflict_status,
};
#[cfg(test)]
pub(super) use recognition::recognition_start_conflict_status;
#[cfg(test)]
use recording::{
    format_recording_progress, format_recording_stopping, next_recording_audio_selection,
    next_recording_display_selection, recording_directory_candidates,
    recording_discovery_conflict_status, recording_discovery_result_is_applicable,
    recording_output_path_from_candidates, recording_start_cancellation_generation,
    recording_start_conflict_status, recording_start_failure_status,
    recording_start_result_is_applicable, recording_support_check_conflict_status,
    recording_support_status, recording_target_label,
};
pub(super) use recording::{
    recording_audio_selection_label, recording_directory_for_display,
    recording_display_selection_label,
};

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
    FlashShotApp, HistoryFilter, RecognitionResult, RecognitionRetry, RecordingAudioSelection,
    RecordingDisplaySelection, SettingsSection,
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
        if self.history_retention_target.is_some()
            || self.history_clear_in_flight
            || self.history_clear_confirmation
            || !self.history_deletions_in_flight.is_empty()
        {
            return;
        }
        let next_limit = next_history_limit(self.settings.history_limit);
        self.history_retention_target = Some(next_limit);
        self.status = format!("Updating screenshot history retention to {next_limit}...");
        cx.notify();
        self.continue_history_retention(cx);
    }

    fn continue_history_retention(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.history_retention_target else {
            return;
        };
        let candidates = self.history.retention_candidates(usize::from(target));
        if candidates.is_empty() {
            self.finish_history_retention(cx);
            return;
        }
        let snapshot = self.history.clone();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let deletion = cx
                    .background_executor()
                    .spawn(async move { snapshot.delete_managed_paths(candidates) })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.finish_history_retention_deletion(deletion, cx)
                    });
                }
            }
        })
        .detach();
    }

    fn finish_history_retention_deletion(
        &mut self,
        deletion: crate::history::HistoryFileDeletion,
        cx: &mut Context<Self>,
    ) {
        if self.history_retention_target.is_none() {
            return;
        }
        for (path, error) in &deletion.failures {
            log::warn!(target: "flash_shot::history", "history_retention_delete_failed path={} error={error}", path.display());
        }
        if let Err(error) = self.history.forget_deleted(&deletion.deleted) {
            self.history_retention_target = None;
            self.status = format!("Could not update screenshot history index: {error}");
            cx.notify();
            return;
        }
        self.synchronize_history_preview_cache();
        if !deletion.failures.is_empty() {
            let failure_count = deletion.failures.len();
            self.history_retention_target = None;
            self.status = format!(
                "Could not remove {failure_count} capture(s); history retention was unchanged"
            );
            cx.notify();
            return;
        }
        self.continue_history_retention(cx);
    }

    fn finish_history_retention(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.history_retention_target.take() else {
            return;
        };
        if let Err(error) = self.history.set_limit_after_prune(usize::from(target)) {
            self.status = format!("Could not update screenshot history retention: {error}");
            cx.notify();
            return;
        }
        self.settings.history_limit = target;
        self.status = match self.settings.save(&self.settings_path) {
            Ok(()) => format!("Screenshot history retains the latest {target} captures"),
            Err(error) => format!(
                "History retention is {target} captures for this session but could not be saved: {error}"
            ),
        };
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

    /// Cycles the suggested extension for interactive screenshot exports and persists it locally.
    pub(super) fn cycle_export_format(&mut self, cx: &mut Context<Self>) {
        let previous = self.settings.export_format;
        let next = UserSettings::next_export_format(previous);
        self.settings.export_format = next;
        if let Err(error) = self.settings.save(&self.settings_path) {
            self.settings.export_format = previous;
            self.status = format!("Could not save export format preference: {error}");
            cx.notify();
            return;
        }
        self.status = format!("Default export format: {}", self.export_format_label());
        cx.notify();
    }

    pub(super) fn export_format_label(&self) -> &'static str {
        match self.settings.export_format {
            1 => "JPEG",
            2 => "WebP",
            _ => "PNG",
        }
    }

    pub(super) fn clear_recognition_result(&mut self, cx: &mut Context<Self>) {
        self.recognition_result = None;
        self.recognition_retry = None;
        cx.notify();
    }

    /// Repeats the last failed OCR or translation request while the original selection is intact.
    pub(super) fn retry_recognition(&mut self, retry: RecognitionRetry, cx: &mut Context<Self>) {
        self.recognition_retry = None;
        match retry {
            RecognitionRetry::Ocr => self.recognize_text_selection(cx),
            RecognitionRetry::Translation => self.translate_selection(cx),
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
        let path = self.history.root().to_owned();
        self.status = match directory::open(&path).map(|()| path) {
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

#[cfg(test)]
mod tests;
