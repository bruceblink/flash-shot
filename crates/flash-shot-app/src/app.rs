//! GPUI capture workspace state and module boundaries.

mod history_search;
mod overlay;
mod pin_acceptance;
mod pinned;
mod render_image;
mod scroll_control;
mod view;
mod workflow;

use std::{
    collections::{HashMap, HashSet, VecDeque},
    ops::Range,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::Instant,
};

use gpui::{
    App, AsyncApp, Context, EntityInputHandler, FocusHandle, Focusable, RenderImage, Subscription,
    UTF16Selection, WeakEntity, Window, WindowHandle,
};

use crate::{
    domain::{
        annotation::{
            AnnotationDocument, AnnotationEditor, AnnotationId, AnnotationStyle, AnnotationTool,
            CommandHistory,
        },
        geometry::PhysicalPoint,
        selection::SelectionDrag,
        session::CaptureSession,
    },
    history::ScreenshotHistory,
    i18n::UiText,
    performance::PerformanceRecorder,
    platform::{
        autostart::{AutoStartService, AutoStartState, SystemAutoStart},
        capture::CaptureFrame,
        clipboard::{ClipboardService, SystemClipboard},
        shortcut::{
            CaptureShortcut, GlobalShortcutService, ShortcutAction, ShortcutBinding, ShortcutEvent,
        },
        tray::{
            TrayAutoStartState, TrayEvent, TrayNotification, TrayRecordingState,
            TrayRecordingTarget, TrayService,
        },
        window_inspector::InspectionTarget,
    },
    settings::UserSettings,
    theme::ThemeColors,
};

/// Opens the capture-overlay acceptance surface while keeping the overlay implementation private.
pub(crate) fn open_overlay_ui_acceptance(
    started_at: Instant,
    performance: PerformanceRecorder,
    history: ScreenshotHistory,
    settings: UserSettings,
    settings_path: PathBuf,
    acceptance: crate::OverlayUiAcceptanceOptions,
    cx: &mut App,
) -> Result<(), Box<dyn std::error::Error>> {
    overlay::open_ui_acceptance(
        started_at,
        performance,
        history,
        settings,
        settings_path,
        acceptance,
        cx,
    )
}

/// Opens the isolated multi-Pin lifecycle while keeping Pin internals inside the app module.
pub(crate) fn open_pin_lifecycle_acceptance(
    performance: PerformanceRecorder,
    history: ScreenshotHistory,
    settings: UserSettings,
    settings_path: PathBuf,
    acceptance: crate::PinLifecycleAcceptanceOptions,
    cx: &mut App,
) -> Result<(), Box<dyn std::error::Error>> {
    pin_acceptance::open(
        performance,
        history,
        settings,
        settings_path,
        acceptance,
        cx,
    )
}

pub struct FlashShotApp {
    colors: ThemeColors,
    session: CaptureSession,
    frame: Option<CaptureFrame>,
    annotation_document: Option<AnnotationDocument>,
    annotation_history: CommandHistory,
    annotation_editor: AnnotationEditor,
    annotation_tool: Option<AnnotationTool>,
    annotation_style: AnnotationStyle,
    selected_annotation: Option<AnnotationId>,
    next_annotation_id: u64,
    next_sequence_number: u32,
    text_edit: Option<TextEdit>,
    text_edit_annotation: Option<AnnotationId>,
    preview: Option<Arc<RenderImage>>,
    selection_drag: SelectionDrag,
    hover_pixel: Option<PhysicalPoint>,
    inspection_target: Option<InspectionTarget>,
    pending_click_target: Option<InspectionTarget>,
    inspection_request: Option<PhysicalPoint>,
    inspection_in_flight: bool,
    manual_scroll: crate::scroll::ManualScrollCapture,
    manual_scroll_selection: Option<crate::domain::geometry::PhysicalRect>,
    manual_scroll_capture_in_flight: bool,
    manual_scroll_finish_in_flight: bool,
    manual_scroll_auto_capture_generation: Option<u64>,
    recording_control: Option<crate::recording::RecordingControl>,
    // Acceptance-only flag that renders live Record controls without starting an FFmpeg worker.
    recording_acceptance_active: bool,
    recording_progress: crate::recording::RecordingProgress,
    recording_start_in_flight: bool,
    recording_stopping: bool,
    recording_paused: bool,
    recording_support_check_in_flight: bool,
    recording_support_check_generation: u64,
    recording_audio: RecordingAudioSelection,
    recording_audio_discovery_in_flight: bool,
    recording_display: RecordingDisplaySelection,
    recording_display_discovery_in_flight: bool,
    recording_directory_check_in_flight: bool,
    update_check_in_flight: bool,
    update_check_generation: u64,
    auto_start_enabled: bool,
    capture_delay_seconds: u8,
    delayed_capture_generation: Option<u64>,
    delayed_capture_remaining_seconds: Option<u8>,
    full_screen_copy_generation: Option<u64>,
    // A selection Copy owns the image-clipboard slot until its background task has stopped or
    // committed. Its id is independent from capture generations so normal tools may continue.
    selection_copy: Option<SelectionCopyLease>,
    // A single shared lease prevents independent UI surfaces from racing to replace clipboard
    // contents while their heavy image encoding remains off the UI thread.
    clipboard_write_sequence: u64,
    clipboard_write_lease: Option<ClipboardWriteLease>,
    full_screen_save_generation: Option<u64>,
    full_screen_pin_generation: Option<u64>,
    clipboard_pin_generation: Option<u64>,
    // One foreground history reader owns its file until the background task finishes.
    history_reader: Option<HistoryReaderLease>,
    // A generic file picker may still select a managed history image, so it reserves the history
    // file set until its async result returns to the UI for completion handling.
    generic_open_generation: Option<u64>,
    // One managed save owns the history index while recording it may prune an older PNG.
    history_write_generation: Option<u64>,
    history_write_sequence: u64,
    // A folder picker owns the history root until its result has either been applied or discarded.
    history_root_change_in_flight: bool,
    pinned_save_in_flight: bool,
    include_cursor: bool,
    recognition_result: Option<RecognitionResult>,
    recognition_retry: Option<RecognitionRetry>,
    recognition_in_flight: bool,
    recognition_generation: u64,
    translation_service_test_in_flight: bool,
    translation_service_test_generation: u64,
    ocr_support_check_in_flight: bool,
    ocr_support_check_generation: u64,
    overlay_more_actions: bool,
    overlay_annotation_controls: bool,
    operation_generation: u64,
    overlay_windows: Vec<WindowHandle<overlay::CaptureOverlay>>,
    // Deferred native window removal remains observable until the close callback has run.
    capture_teardown_pending: bool,
    capture_teardown_windows: HashSet<gpui::WindowId>,
    pinned_windows: Vec<WindowHandle<pinned::PinnedImage>>,
    scroll_window: Option<WindowHandle<scroll_control::ManualScrollControl>>,
    settings_window_handle: Option<isize>,
    focus_handle: FocusHandle,
    settings_navigation_focus: [FocusHandle; 4],
    capture_shortcut: String,
    capture_shortcut_enabled: bool,
    settings_section: SettingsSection,
    settings: UserSettings,
    settings_path: PathBuf,
    status: String,
    performance: PerformanceRecorder,
    history: ScreenshotHistory,
    // Tracks the workflow that produced the current editable frame so managed quick saves keep
    // scrolling screenshots distinguishable from ordinary selections in history.
    history_source: crate::history::HistorySource,
    history_expanded: bool,
    history_filter: HistoryFilter,
    history_search: HistorySearch,
    history_clear_confirmation: bool,
    history_clear_scope: HistoryClearScope,
    history_clear_count: usize,
    history_clear_paths: Vec<PathBuf>,
    quick_save_directory_check_in_flight: bool,
    history_selected_paths: HashSet<PathBuf>,
    history_keyboard_focus: Option<PathBuf>,
    history_clear_in_flight: bool,
    history_retention_target: Option<u16>,
    history_deletions_in_flight: HashSet<PathBuf>,
    history_thumbnails: HashMap<PathBuf, Arc<RenderImage>>,
    history_thumbnail_loading: HashSet<PathBuf>,
    // Keeps a large Library expansion from queuing one full PNG decode per saved capture.
    history_thumbnail_pending: VecDeque<PathBuf>,
    history_thumbnail_failed: HashSet<PathBuf>,
    // Invalidates best-effort thumbnail reads when a file mutation starts or completes.
    history_thumbnail_revision: u64,
    image_clipboard: Arc<dyn ClipboardService + Send + Sync>,
    system_services: SystemServices,
    _shutdown: Subscription,
    _window_closed: Subscription,
    _shortcut: Option<GlobalShortcutService>,
    _tray: Option<TrayService>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SystemServices {
    Production,
    DisabledForAcceptance,
}

/// Owns one selection Copy while its background preparation and clipboard commit are active.
///
/// The cancellation state crosses the UI/background boundary so Escape can stop a copy before
/// it reaches the irreversible native clipboard write.
pub(super) struct SelectionCopyLease {
    pub(super) id: u64,
    pub(super) status_generation: u64,
    pub(super) recognition_generation: u64,
    pub(super) cancellation: Arc<SelectionCopyCancellation>,
    pub(super) cancel_requested: bool,
}

impl SelectionCopyLease {
    /// Lets only the first Escape belong to Copy; a later Escape closes the editor normally.
    pub(super) const fn owns_escape(&self) -> bool {
        !self.cancel_requested
    }
}

/// Identifies the one asynchronous task that currently owns an image clipboard write.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ClipboardWriteLease {
    pub(super) id: u64,
}

/// Coordinates the last cancellable point before a selection Copy changes the system clipboard.
pub(super) struct SelectionCopyCancellation {
    state: AtomicU8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SelectionCopyCancelRequest {
    CancelledBeforeCommit,
    AlreadyCancelled,
    ClipboardCommitStarted,
}

const SELECTION_COPY_PREPARING: u8 = 0;
const SELECTION_COPY_CANCELLED: u8 = 1;
const SELECTION_COPY_COMMITTING: u8 = 2;
const SELECTION_COPY_COMMITTED: u8 = 3;

impl Default for SelectionCopyCancellation {
    fn default() -> Self {
        Self {
            state: AtomicU8::new(SELECTION_COPY_PREPARING),
        }
    }
}

impl SelectionCopyCancellation {
    /// Claims the native clipboard write only if cancellation has not already won the race.
    pub(super) fn begin_clipboard_commit(&self) -> bool {
        self.state
            .compare_exchange(
                SELECTION_COPY_PREPARING,
                SELECTION_COPY_COMMITTING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    /// Marks the clipboard call as finished, including a failed call that may have emptied it.
    pub(super) fn finish_clipboard_commit(&self) {
        self.state
            .store(SELECTION_COPY_COMMITTED, Ordering::Release);
    }

    /// Requests cancellation while pixel preparation is still reversible.
    pub(super) fn request_cancel(&self) -> SelectionCopyCancelRequest {
        loop {
            match self.state.load(Ordering::Acquire) {
                SELECTION_COPY_PREPARING => {
                    if self
                        .state
                        .compare_exchange(
                            SELECTION_COPY_PREPARING,
                            SELECTION_COPY_CANCELLED,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_ok()
                    {
                        return SelectionCopyCancelRequest::CancelledBeforeCommit;
                    }
                }
                SELECTION_COPY_CANCELLED => return SelectionCopyCancelRequest::AlreadyCancelled,
                SELECTION_COPY_COMMITTING | SELECTION_COPY_COMMITTED => {
                    return SelectionCopyCancelRequest::ClipboardCommitStarted;
                }
                _ => unreachable!("selection Copy cancellation state is valid"),
            }
        }
    }

    /// Checks whether Escape cancelled the task before it begins expensive pixel preparation.
    pub(super) fn is_cancelled(&self) -> bool {
        self.state.load(Ordering::Acquire) == SELECTION_COPY_CANCELLED
    }
}

impl crate::platform::clipboard::ClipboardCommitGate for SelectionCopyCancellation {
    fn is_cancelled(&self) -> bool {
        self.is_cancelled()
    }

    fn begin_clipboard_commit(&self) -> bool {
        self.begin_clipboard_commit()
    }

    fn finish_clipboard_commit(&self) {
        self.finish_clipboard_commit();
    }
}

/// The settings window is intentionally segmented so the capture service has
/// no always-visible command surface and each configuration task stays small.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum SettingsSection {
    #[default]
    Capture,
    Files,
    Recording,
    System,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum HistoryFilter {
    #[default]
    All,
    Selection,
    Scrolling,
    FullScreen,
    Pinned,
}

/// Names which part of the history list a destructive clear request targets.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum HistoryClearScope {
    #[default]
    All,
    Filtered,
    Selected,
}

/// Identifies the user-visible history action that currently owns one saved PNG.
///
/// Open, Copy, and Pin all decode the same managed file asynchronously, so they share one lease
/// instead of relying on unrelated session state or clipboard flags.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum HistoryReaderKind {
    Open,
    Copy,
    Pin,
}

/// Keeps the active history reader tied to the exact request that started it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct HistoryReaderLease {
    pub(super) kind: HistoryReaderKind,
    pub(super) generation: u64,
    pub(super) path: PathBuf,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct HistorySearch {
    content: String,
    selected_range: Range<usize>,
    marked_range: Option<Range<usize>>,
    active: bool,
}

impl HistoryFilter {
    pub(super) const ALL: [Self; 5] = [
        Self::All,
        Self::Selection,
        Self::Scrolling,
        Self::FullScreen,
        Self::Pinned,
    ];

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Selection => "Selections",
            Self::Scrolling => "Scrolling",
            Self::FullScreen => "Full screen",
            Self::Pinned => "Pinned",
        }
    }

    pub(super) const fn matches(self, source: crate::history::HistorySource) -> bool {
        match self {
            Self::All => true,
            Self::Selection => matches!(source, crate::history::HistorySource::Selection),
            Self::Scrolling => matches!(source, crate::history::HistorySource::Scrolling),
            Self::FullScreen => matches!(source, crate::history::HistorySource::FullScreen),
            Self::Pinned => matches!(source, crate::history::HistorySource::Pinned),
        }
    }
}

/// Applies the same source and filename rules to rendering and filtered deletion.
pub(super) fn history_entry_matches(
    entry: &crate::history::HistoryEntry,
    filter: HistoryFilter,
    query: &str,
) -> bool {
    if !filter.matches(entry.source) {
        return false;
    }
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    entry
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_lowercase().contains(&query))
        || entry.source.label().to_lowercase().contains(&query)
}

/// Returns selected history paths that still belong to the current filtered list.
/// Keeping this snapshot derived from the history store prevents a stale selection from
/// widening a later batch deletion after files or filters have changed.
pub(super) fn selected_history_paths(
    entries: &std::collections::VecDeque<crate::history::HistoryEntry>,
    selected: &HashSet<PathBuf>,
    filter: HistoryFilter,
    query: &str,
) -> Vec<PathBuf> {
    entries
        .iter()
        .filter(|entry| selected.contains(&entry.path))
        .filter(|entry| history_entry_matches(entry, filter, query))
        .map(|entry| entry.path.clone())
        .collect()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TextEdit {
    pub(super) origin: PhysicalPoint,
    pub(super) content: String,
    pub(super) selected_range: Range<usize>,
    pub(super) marked_range: Option<Range<usize>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RecognitionResult {
    pub(super) title: String,
    pub(super) text: String,
}

/// Identifies the failed selection workflow that can be rerun without losing the overlay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RecognitionRetry {
    Ocr,
    Translation,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) enum RecordingAudioSelection {
    #[default]
    Automatic,
    Disabled,
    Source(crate::recording::AudioSource),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) enum RecordingDisplaySelection {
    #[default]
    Primary,
    Display {
        id: String,
        label: String,
    },
}

impl TextEdit {
    pub(super) fn new(origin: PhysicalPoint) -> Self {
        Self::with_content(origin, String::new(), false)
    }

    pub(super) fn with_content(origin: PhysicalPoint, content: String, select_all: bool) -> Self {
        let selected_range = if select_all { 0..content.len() } else { 0..0 };
        Self {
            origin,
            content,
            selected_range,
            marked_range: None,
        }
    }
}

impl FlashShotApp {
    pub(crate) fn set_settings_window_handle(&mut self, handle: isize) {
        self.settings_window_handle = Some(handle);
    }

    pub(super) fn notify_user(&self, title: &str, body: &str) {
        let Some(tray) = self._tray.as_ref() else {
            return;
        };
        if let Err(error) = tray.notify(TrayNotification::new(title, body)) {
            log::warn!(target: "flash_shot::tray", "user_notification_failed error={error}");
        }
    }

    /// Mirrors the recording lifecycle into the independent Windows tray thread.
    ///
    /// A missing tray is harmless: recording continues, while notification-area commands remain
    /// unavailable on platforms that do not support them.
    pub(super) fn set_tray_recording_state(&self, state: TrayRecordingState) {
        if let Some(tray) = self._tray.as_ref() {
            tray.set_recording_state(state);
        }
    }

    /// Mirrors the active recording target into the tray's pause and stop labels.
    pub(super) fn set_tray_recording_target(&self, target: TrayRecordingTarget) {
        if let Some(tray) = self._tray.as_ref() {
            tray.set_recording_target(target);
        }
    }

    /// Returns tray controls to their display-recording idle state without relabeling an active
    /// region or window recording during the state transition.
    pub(super) fn reset_tray_recording_to_idle(&self) {
        self.set_tray_recording_state(TrayRecordingState::Idle);
        self.set_tray_recording_target(TrayRecordingTarget::Display);
    }

    /// Mirrors Windows sign-in ownership into the tray so unsafe entry replacement is impossible.
    pub(super) fn set_tray_auto_start_state(&self, state: AutoStartState) {
        let state = match state {
            AutoStartState::Enabled => TrayAutoStartState::Enabled,
            AutoStartState::Disabled => TrayAutoStartState::Disabled,
            AutoStartState::ManagedByAnotherExecutable => {
                TrayAutoStartState::ManagedByAnotherExecutable
            }
        };
        if let Some(tray) = self._tray.as_ref() {
            tray.set_auto_start_state(state);
        }
    }

    /// Mirrors the saved cursor preference into the tray check mark for capture commands.
    pub(super) fn set_tray_capture_cursor_enabled(&self, enabled: bool) {
        if let Some(tray) = self._tray.as_ref() {
            tray.set_capture_cursor_enabled(enabled);
        }
    }

    /// Mirrors the live global-hotkey registration into the native tray check mark.
    pub(super) fn set_tray_capture_shortcut_enabled(&self, enabled: bool) {
        if let Some(tray) = self._tray.as_ref() {
            tray.set_capture_shortcut_enabled(enabled);
        }
    }

    /// Lets the isolated input runner refuse before sending keys when F24 registration failed.
    pub(crate) const fn capture_shortcut_active_for_acceptance(&self) -> bool {
        self.capture_shortcut_enabled
    }

    /// Verifies that an acceptance app owns no process-wide shortcut or tray integration.
    pub(crate) fn system_services_disabled_for_acceptance(&self) -> bool {
        self.system_services == SystemServices::DisabledForAcceptance
            && self._shortcut.is_none()
            && self._tray.is_none()
            && !self.capture_shortcut_enabled
    }

    pub fn new(
        performance: PerformanceRecorder,
        history: ScreenshotHistory,
        settings: UserSettings,
        settings_path: PathBuf,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_with_system_services(
            performance,
            history,
            settings,
            settings_path,
            Arc::new(SystemClipboard),
            SystemServices::Production,
            cx,
        )
    }

    /// Builds an isolated acceptance app without registering hotkeys, tray icons, or autostart.
    pub(crate) fn new_for_acceptance(
        performance: PerformanceRecorder,
        history: ScreenshotHistory,
        settings: UserSettings,
        settings_path: PathBuf,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_with_system_services(
            performance,
            history,
            settings,
            settings_path,
            Arc::new(SystemClipboard),
            SystemServices::DisabledForAcceptance,
            cx,
        )
    }

    /// Builds the production shortcut workflow with a disposable selection-copy destination.
    ///
    /// Native input acceptance still exercises the real global shortcut and overlay lifecycle,
    /// while the injected sink prevents its Copy action from replacing the user's clipboard.
    pub(crate) fn new_for_overlay_interaction(
        performance: PerformanceRecorder,
        history: ScreenshotHistory,
        settings: UserSettings,
        settings_path: PathBuf,
        image_clipboard: Arc<dyn ClipboardService + Send + Sync>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_with_system_services(
            performance,
            history,
            settings,
            settings_path,
            image_clipboard,
            SystemServices::Production,
            cx,
        )
    }

    /// Initializes shared workflow state while making system-wide services an explicit boundary.
    fn new_with_system_services(
        performance: PerformanceRecorder,
        history: ScreenshotHistory,
        settings: UserSettings,
        settings_path: PathBuf,
        image_clipboard: Arc<dyn ClipboardService + Send + Sync>,
        system_services: SystemServices,
        cx: &mut Context<Self>,
    ) -> Self {
        let shutdown = cx.on_app_quit(|this, cx| {
            this.shutdown(cx);
            async {}
        });
        let app = cx.entity().downgrade();
        let window_closed = cx.on_window_closed(move |cx, window_id| {
            if let Some(app) = app.upgrade() {
                app.update(cx, |app, cx| {
                    app.unregister_capture_overlay(window_id, cx);
                    app.unregister_pinned_window(window_id, cx);
                });
            }
        });
        let production_services = system_services == SystemServices::Production;
        let capture_shortcut = if production_services {
            match settings
                .capture_shortcut
                .as_deref()
                .map(str::parse)
                .transpose()
            {
                Ok(Some(shortcut)) => shortcut,
                Ok(None) => match CaptureShortcut::from_environment() {
                    Ok(shortcut) => shortcut,
                    Err(error) => {
                        log::warn!(target: "flash_shot::shortcut", "capture_hotkey_config_invalid error={error}");
                        CaptureShortcut::default()
                    }
                },
                Err(error) => {
                    log::warn!(target: "flash_shot::shortcut", "saved_capture_hotkey_invalid error={error}");
                    CaptureShortcut::default()
                }
            }
        } else {
            CaptureShortcut::default()
        };
        let capture_shortcut_label = capture_shortcut.to_string();
        let shortcut = if production_services && settings.capture_shortcut_enabled {
            match register_global_shortcuts(capture_shortcut, &settings) {
                Ok((service, events)) => {
                    Self::listen_for_shortcut(events, cx);
                    Some(service)
                }
                Err(error) => {
                    log::warn!(target: "flash_shot::shortcut", "capture_hotkey_unavailable error={error}");
                    None
                }
            }
        } else {
            None
        };
        let capture_shortcut_enabled = shortcut.is_some();
        let locale = settings.locale;
        let status = if !production_services {
            locale
                .text(UiText::ReadySystemServicesDisabledForAcceptance)
                .to_owned()
        } else if capture_shortcut_enabled {
            locale.ready_with_shortcut(&capture_shortcut_label)
        } else if settings.capture_shortcut_enabled {
            locale
                .text(UiText::ReadyGlobalShortcutUnavailable)
                .to_owned()
        } else {
            locale.text(UiText::ReadyGlobalShortcutDisabled).to_owned()
        };
        let tray = if production_services {
            match TrayService::start() {
                Ok((service, events)) => {
                    Self::listen_for_tray(events, cx);
                    Some(service)
                }
                Err(error) => {
                    log::warn!(target: "flash_shot::tray", "tray_unavailable error={error}");
                    None
                }
            }
        } else {
            None
        };
        let auto_start_enabled = if production_services {
            match std::env::current_exe()
                .ok()
                .map(|executable| SystemAutoStart.state(&executable))
            {
                Some(Ok(AutoStartState::Enabled)) => true,
                Some(Ok(AutoStartState::Disabled | AutoStartState::ManagedByAnotherExecutable)) => {
                    false
                }
                Some(Err(error)) => {
                    log::warn!(target: "flash_shot::autostart", "auto_start_state_read_failed error={error}");
                    false
                }
                None => false,
            }
        } else {
            false
        };
        if let (Some(tray), Ok(executable)) = (tray.as_ref(), std::env::current_exe())
            && let Ok(state) = SystemAutoStart.state(&executable)
        {
            let state = match state {
                AutoStartState::Enabled => TrayAutoStartState::Enabled,
                AutoStartState::Disabled => TrayAutoStartState::Disabled,
                AutoStartState::ManagedByAnotherExecutable => {
                    TrayAutoStartState::ManagedByAnotherExecutable
                }
            };
            tray.set_auto_start_state(state);
        }
        if let Some(tray) = tray.as_ref() {
            tray.set_capture_cursor_enabled(settings.include_cursor);
            tray.set_capture_shortcut_enabled(capture_shortcut_enabled);
        }

        Self {
            colors: ThemeColors::for_mode(settings.theme_mode),
            session: CaptureSession::default(),
            frame: None,
            annotation_document: None,
            annotation_history: CommandHistory::default(),
            annotation_editor: AnnotationEditor::default(),
            annotation_tool: None,
            annotation_style: AnnotationStyle::default(),
            selected_annotation: None,
            next_annotation_id: 1,
            next_sequence_number: 1,
            text_edit: None,
            text_edit_annotation: None,
            preview: None,
            selection_drag: SelectionDrag::default(),
            hover_pixel: None,
            inspection_target: None,
            pending_click_target: None,
            inspection_request: None,
            inspection_in_flight: false,
            manual_scroll: crate::scroll::ManualScrollCapture::default(),
            manual_scroll_selection: None,
            manual_scroll_capture_in_flight: false,
            manual_scroll_finish_in_flight: false,
            manual_scroll_auto_capture_generation: None,
            recording_control: None,
            recording_acceptance_active: false,
            recording_progress: Default::default(),
            recording_start_in_flight: false,
            recording_stopping: false,
            recording_paused: false,
            recording_support_check_in_flight: false,
            recording_support_check_generation: 0,
            recording_audio: RecordingAudioSelection::Automatic,
            recording_audio_discovery_in_flight: false,
            recording_display: RecordingDisplaySelection::Primary,
            recording_display_discovery_in_flight: false,
            recording_directory_check_in_flight: false,
            update_check_in_flight: false,
            update_check_generation: 0,
            auto_start_enabled,
            capture_delay_seconds: settings.capture_delay_seconds,
            delayed_capture_generation: None,
            delayed_capture_remaining_seconds: None,
            full_screen_copy_generation: None,
            selection_copy: None,
            clipboard_write_sequence: 0,
            clipboard_write_lease: None,
            full_screen_save_generation: None,
            full_screen_pin_generation: None,
            clipboard_pin_generation: None,
            history_reader: None,
            generic_open_generation: None,
            history_write_generation: None,
            history_write_sequence: 0,
            history_root_change_in_flight: false,
            pinned_save_in_flight: false,
            include_cursor: settings.include_cursor,
            recognition_result: None,
            recognition_retry: None,
            recognition_in_flight: false,
            recognition_generation: 0,
            translation_service_test_in_flight: false,
            translation_service_test_generation: 0,
            ocr_support_check_in_flight: false,
            ocr_support_check_generation: 0,
            overlay_more_actions: false,
            overlay_annotation_controls: false,
            operation_generation: 0,
            overlay_windows: Vec::new(),
            capture_teardown_pending: false,
            capture_teardown_windows: HashSet::new(),
            pinned_windows: Vec::new(),
            scroll_window: None,
            settings_window_handle: None,
            focus_handle: cx.focus_handle(),
            settings_navigation_focus: std::array::from_fn(|_| cx.focus_handle()),
            capture_shortcut: capture_shortcut_label,
            capture_shortcut_enabled,
            settings_section: SettingsSection::default(),
            settings,
            settings_path,
            status,
            performance,
            history,
            history_source: crate::history::HistorySource::Selection,
            history_expanded: false,
            history_filter: HistoryFilter::All,
            history_search: HistorySearch::default(),
            history_clear_confirmation: false,
            history_clear_scope: HistoryClearScope::default(),
            history_clear_count: 0,
            history_clear_paths: Vec::new(),
            quick_save_directory_check_in_flight: false,
            history_selected_paths: HashSet::new(),
            history_keyboard_focus: None,
            history_clear_in_flight: false,
            history_retention_target: None,
            history_deletions_in_flight: HashSet::new(),
            history_thumbnails: HashMap::new(),
            history_thumbnail_loading: HashSet::new(),
            history_thumbnail_pending: VecDeque::new(),
            history_thumbnail_failed: HashSet::new(),
            history_thumbnail_revision: 0,
            image_clipboard,
            system_services,
            _shutdown: shutdown,
            _window_closed: window_closed,
            _shortcut: shortcut,
            _tray: tray,
        }
    }

    /// Bridges the isolated input runner to observable state without bypassing product UI actions.
    ///
    /// Snapshot replies are process-local and bounded. The two mutating commands only restore the
    /// hidden settings controller; capture, export, Pin, and recording actions still use input.
    pub(crate) fn listen_for_overlay_interaction_commands(
        commands: async_channel::Receiver<crate::OverlayInteractionAcceptanceCommand>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                while let Ok(command) = commands.recv().await {
                    let Some(this) = this.upgrade() else {
                        break;
                    };
                    this.update(&mut cx, |this, cx| match command {
                        crate::OverlayInteractionAcceptanceCommand::Snapshot(reply) => {
                            let target = this.recording_control.as_ref().map(|control| {
                                match control.target() {
                                    crate::recording::RecordingTarget::Display { .. } => "display",
                                    crate::recording::RecordingTarget::Window { .. } => "window",
                                    crate::recording::RecordingTarget::Region { .. } => {
                                        "selected area"
                                    }
                                }
                                .to_owned()
                            });
                            let target_bounds = this.recording_control.as_ref().map(|control| {
                                match control.target() {
                                    crate::recording::RecordingTarget::Display { bounds }
                                    | crate::recording::RecordingTarget::Window {
                                        bounds, ..
                                    }
                                    | crate::recording::RecordingTarget::Region { bounds } => {
                                        *bounds
                                    }
                                }
                            });
                            let _ = reply.send(crate::OverlayInteractionRecordingState {
                                active: this.recording_control.is_some(),
                                starting: this.recording_start_in_flight,
                                stopping: this.recording_stopping,
                                paused: this.recording_paused,
                                target,
                                target_bounds,
                                progress_frame: this.recording_progress.frame.unwrap_or_default(),
                                progress_time_us: this
                                    .recording_progress
                                    .output_time_us
                                    .unwrap_or_default(),
                                status: this.status.clone(),
                            });
                        }
                        crate::OverlayInteractionAcceptanceCommand::CaptureSnapshot(reply) => {
                            let pinned_source_bounds = this.pinned_windows.last().and_then(|pin| {
                                pin.update(cx, |pin, _, _| pin.source_bounds_for_acceptance())
                                    .ok()
                            });
                            let session_state = match this.session.state() {
                                crate::domain::session::CaptureSessionState::Idle => "idle",
                                crate::domain::session::CaptureSessionState::Capturing => {
                                    "capturing"
                                }
                                crate::domain::session::CaptureSessionState::Selecting => {
                                    "selecting"
                                }
                                crate::domain::session::CaptureSessionState::Exporting => {
                                    "exporting"
                                }
                                crate::domain::session::CaptureSessionState::Completed => {
                                    "completed"
                                }
                                crate::domain::session::CaptureSessionState::Cancelled => {
                                    "cancelled"
                                }
                                crate::domain::session::CaptureSessionState::Failed => "failed",
                            };
                            let _ = reply.send(crate::OverlayInteractionCaptureState {
                                session_state: session_state.to_owned(),
                                // Report the committed session rectangle used by Save, Pin, and
                                // Copy, never an in-flight mouse-move preview awaiting mouse-up.
                                selection: this.session.selection(),
                                selection_copy_active: this.selection_copy_is_active(),
                                clipboard_write_active: this.clipboard_write_lease.is_some(),
                                manual_scroll_state: manual_scroll_state_label(
                                    this.manual_scroll.state(),
                                )
                                .to_owned(),
                                manual_scroll_frame_count: this.manual_scroll.frame_count(),
                                manual_scroll_can_finish: this.manual_scroll.can_finish(),
                                manual_scroll_capture_in_flight: this
                                    .manual_scroll_capture_in_flight,
                                manual_scroll_finish_in_flight: this.manual_scroll_finish_in_flight,
                                manual_scroll_auto_capture_pending: this
                                    .manual_scroll_auto_capture_generation
                                    .is_some(),
                                manual_scroll_selection: this.manual_scroll_selection,
                                overlay_count: this.overlay_windows.len(),
                                more_actions_visible: this.overlay_more_actions,
                                annotation_controls_visible: this.overlay_annotation_controls,
                                pinned_count: this.pinned_windows.len(),
                                pinned_source_bounds,
                                capture_teardown_pending: this.capture_teardown_pending,
                                operation_generation: this.operation_generation,
                                background_tasks_idle: this.capture_background_tasks_idle(),
                                capture_preflight_ready: this.capture_preflight_ready(),
                                status: this.status.clone(),
                            });
                        }
                        crate::OverlayInteractionAcceptanceCommand::CaptureContent(reply) => {
                            let selection = this.session.selection().and_then(|selection| {
                                this.frame
                                    .as_ref()
                                    .and_then(|frame| frame.crop(selection).ok())
                            });
                            let pins = this
                                .pinned_windows
                                .iter()
                                .filter_map(|pin| {
                                    pin.update(cx, |pin, _, _| pin.frame_for_acceptance()).ok()
                                })
                                .collect();
                            let _ = reply
                                .send(crate::OverlayInteractionCaptureContent { selection, pins });
                        }
                        crate::OverlayInteractionAcceptanceCommand::ShowCaptureSettings => {
                            this.select_settings_section(SettingsSection::Capture, cx);
                            this.show_settings_window(cx);
                        }
                        crate::OverlayInteractionAcceptanceCommand::ShowRecordingSettings => {
                            this.select_settings_section(SettingsSection::Recording, cx);
                            this.show_settings_window(cx);
                        }
                    });
                }
            }
        })
        .detach();
    }

    /// Bridges no-input Library resource probes to the real thumbnail queue without exposing
    /// mutable production state. Every reply is a bounded snapshot taken on the GPUI thread.
    pub(crate) fn listen_for_history_resource_commands(
        commands: async_channel::Receiver<crate::HistoryResourceAcceptanceCommand>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                while let Ok(command) = commands.recv().await {
                    let Some(this) = this.upgrade() else {
                        break;
                    };
                    this.update(&mut cx, |this, cx| match command {
                        crate::HistoryResourceAcceptanceCommand::Snapshot(reply) => {
                            let _ = reply.send(this.history_resource_acceptance_state());
                        }
                        crate::HistoryResourceAcceptanceCommand::SetExpanded {
                            expanded,
                            reply,
                        } => {
                            this.history_expanded = expanded;
                            cx.notify();
                            let _ = reply.send(this.history_resource_acceptance_state());
                        }
                        crate::HistoryResourceAcceptanceCommand::Quit(reply) => {
                            let _ = reply.send(());
                            cx.quit();
                        }
                    });
                }
            }
        })
        .detach();
    }

    /// Reports the visible history count and thumbnail queue ownership for resource evidence.
    fn history_resource_acceptance_state(&self) -> crate::HistoryResourceAcceptanceState {
        let filtered_entries = self
            .history
            .entries()
            .iter()
            .filter(|entry| {
                history_entry_matches(entry, self.history_filter, self.history_search_query())
            })
            .count();
        crate::HistoryResourceAcceptanceState {
            total_entries: self.history.entries().len(),
            visible_entries: if self.history_expanded {
                filtered_entries
            } else {
                filtered_entries.min(5)
            },
            expanded: self.history_expanded,
            thumbnails_cached: self.history_thumbnails.len(),
            thumbnails_loading: self.history_thumbnail_loading.len(),
            thumbnails_pending: self.history_thumbnail_pending.len(),
        }
    }

    fn listen_for_shortcut(events: async_channel::Receiver<ShortcutEvent>, cx: &mut Context<Self>) {
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                while let Ok(event) = events.recv().await {
                    if let Some(this) = this.upgrade() {
                        this.update(&mut cx, |this, cx| match event {
                            ShortcutEvent::CaptureRequested => this.start_capture(cx),
                            ShortcutEvent::FullScreenRequested => {
                                this.start_full_screen_capture(cx)
                            }
                            ShortcutEvent::FocusedWindowRequested => {
                                this.start_focused_window_capture(cx)
                            }
                        });
                    } else {
                        break;
                    }
                }
            }
        })
        .detach();
    }

    fn listen_for_tray(events: async_channel::Receiver<TrayEvent>, cx: &mut Context<Self>) {
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                while let Ok(event) = events.recv().await {
                    let Some(this) = this.upgrade() else {
                        break;
                    };
                    match event {
                        TrayEvent::CaptureRequested => {
                            this.update(&mut cx, |this, cx| this.start_capture(cx));
                        }
                        TrayEvent::FullScreenCaptureRequested => {
                            this.update(&mut cx, |this, cx| this.start_full_screen_capture(cx));
                        }
                        TrayEvent::FullScreenCopyRequested => {
                            this.update(&mut cx, |this, cx| this.copy_full_screen(cx));
                        }
                        TrayEvent::FullScreenSaveRequested => {
                            this.update(&mut cx, |this, cx| this.quick_save_full_screen(cx));
                        }
                        TrayEvent::FullScreenPinRequested => {
                            this.update(&mut cx, |this, cx| this.pin_full_screen(cx));
                        }
                        TrayEvent::PinClipboardImageRequested => {
                            this.update(&mut cx, |this, cx| this.pin_clipboard_image(cx));
                        }
                        TrayEvent::DelayedCaptureRequested(seconds) => {
                            this.update(&mut cx, |this, cx| {
                                this.start_delayed_capture(seconds, cx)
                            });
                        }
                        TrayEvent::ToggleDisplayRecordingRequested => {
                            this.update(&mut cx, |this, cx| this.toggle_display_recording(cx));
                        }
                        TrayEvent::ToggleRecordingPauseRequested => {
                            this.update(&mut cx, |this, cx| this.toggle_recording_pause(cx));
                        }
                        TrayEvent::ToggleAutoStartRequested => {
                            this.update(&mut cx, |this, cx| this.toggle_auto_start(cx));
                        }
                        TrayEvent::ToggleCaptureCursorRequested => {
                            this.update(&mut cx, |this, cx| this.toggle_capture_cursor(cx));
                        }
                        TrayEvent::ToggleCaptureShortcutRequested => {
                            this.update(&mut cx, |this, cx| this.toggle_capture_shortcut(cx));
                        }
                        TrayEvent::OpenHistoryDirectoryRequested => {
                            this.update(&mut cx, |this, cx| this.open_history_directory(cx));
                        }
                        TrayEvent::OpenImageRequested => {
                            this.update(&mut cx, |this, cx| this.open_image(cx));
                        }
                        TrayEvent::OpenProjectRequested => {
                            this.update(&mut cx, |this, cx| this.open_editable_project(cx));
                        }
                        TrayEvent::HistoryRequested => {
                            this.update(&mut cx, |this, cx| this.show_history_window(cx));
                        }
                        TrayEvent::SettingsRequested => {
                            this.update(&mut cx, |this, cx| this.show_settings_window(cx));
                        }
                        TrayEvent::CheckUpdatesRequested => {
                            this.update(&mut cx, |this, cx| this.check_for_updates(cx));
                        }
                        TrayEvent::QuitRequested => {
                            cx.update(|cx| cx.quit());
                            break;
                        }
                    }
                }
            }
        })
        .detach();
    }
}

/// Converts persisted optional action keys into one atomic native registration request.
fn register_global_shortcuts(
    capture: CaptureShortcut,
    settings: &UserSettings,
) -> std::io::Result<(
    GlobalShortcutService,
    async_channel::Receiver<ShortcutEvent>,
)> {
    let mut bindings = vec![ShortcutBinding {
        action: ShortcutAction::Capture,
        shortcut: capture,
    }];
    for (action, configured) in [
        (
            ShortcutAction::FullScreen,
            settings.full_screen_shortcut.as_deref(),
        ),
        (
            ShortcutAction::FocusedWindow,
            settings.focused_window_shortcut.as_deref(),
        ),
    ] {
        if let Some(configured) = configured {
            let shortcut = configured.parse().map_err(std::io::Error::other)?;
            bindings.push(ShortcutBinding { action, shortcut });
        }
    }
    GlobalShortcutService::register(&bindings)
}

impl EntityInputHandler for FlashShotApp {
    fn text_for_range(
        &mut self,
        range_utf16: std::ops::Range<usize>,
        actual_range: &mut Option<std::ops::Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let content = self
            .text_edit
            .as_ref()
            .map(|edit| edit.content.as_str())
            .or_else(|| {
                self.history_search
                    .active
                    .then_some(self.history_search.content.as_str())
            })?;
        let range = utf16_range_to_byte_range(content, &range_utf16);
        *actual_range = Some(byte_range_to_utf16_range(content, &range));
        Some(content[range].to_owned())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let (content, selected_range) = if let Some(edit) = self.text_edit.as_ref() {
            (&edit.content, &edit.selected_range)
        } else if self.history_search.active {
            (
                &self.history_search.content,
                &self.history_search.selected_range,
            )
        } else {
            return None;
        };
        Some(UTF16Selection {
            range: byte_range_to_utf16_range(content, selected_range),
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<std::ops::Range<usize>> {
        let (content, marked_range) = if let Some(edit) = self.text_edit.as_ref() {
            (&edit.content, edit.marked_range.as_ref())
        } else if self.history_search.active {
            (
                &self.history_search.content,
                self.history_search.marked_range.as_ref(),
            )
        } else {
            return None;
        };
        marked_range
            .as_ref()
            .map(|range| byte_range_to_utf16_range(content, range))
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.unmark_text_edit(cx) {
            self.unmark_history_search(cx);
        }
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<std::ops::Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.replace_text_edit(range_utf16.clone(), text, None, cx) {
            self.replace_history_search(range_utf16, text, None, cx);
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<std::ops::Range<usize>>,
        text: &str,
        selected_range_utf16: Option<std::ops::Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.replace_text_edit(range_utf16.clone(), text, selected_range_utf16.clone(), cx) {
            self.replace_history_search(range_utf16, text, selected_range_utf16, cx);
        }
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: std::ops::Range<usize>,
        bounds: gpui::Bounds<gpui::Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<gpui::Bounds<gpui::Pixels>> {
        (self.text_edit.is_some() || self.history_search.active).then_some(bounds)
    }

    fn character_index_for_point(
        &mut self,
        _point: gpui::Point<gpui::Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        self.text_edit
            .as_ref()
            .map(|edit| edit.content.as_str())
            .or_else(|| {
                self.history_search
                    .active
                    .then_some(self.history_search.content.as_str())
            })
            .map(|content| content.chars().map(char::len_utf16).sum::<usize>())
    }

    fn accepts_text_input(&self, _window: &mut Window, _cx: &mut Context<Self>) -> bool {
        self.text_edit.is_some() || self.history_search.active
    }
}

fn byte_range_to_utf16_range(text: &str, range: &std::ops::Range<usize>) -> std::ops::Range<usize> {
    let utf16_offset = |offset| text[..offset].chars().map(char::len_utf16).sum();
    utf16_offset(range.start)..utf16_offset(range.end)
}

fn utf16_range_to_byte_range(text: &str, range: &std::ops::Range<usize>) -> std::ops::Range<usize> {
    let byte_offset = |target| {
        let mut units = 0;
        let mut bytes = 0;
        for character in text.chars() {
            if units >= target {
                break;
            }
            units += character.len_utf16();
            bytes += character.len_utf8();
        }
        bytes
    };
    byte_offset(range.start)..byte_offset(range.end)
}

/// Converts the internal manual-scroll lifecycle into a stable acceptance-facing label.
fn manual_scroll_state_label(state: crate::scroll::ManualScrollState) -> &'static str {
    match state {
        crate::scroll::ManualScrollState::Idle => "idle",
        crate::scroll::ManualScrollState::Collecting => "collecting",
        crate::scroll::ManualScrollState::Finishing => "finishing",
        crate::scroll::ManualScrollState::Completed => "completed",
        crate::scroll::ManualScrollState::Cancelled => "cancelled",
        crate::scroll::ManualScrollState::Failed => "failed",
    }
}

impl Focusable for FlashShotApp {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        HistoryFilter, byte_range_to_utf16_range, history_entry_matches, manual_scroll_state_label,
        selected_history_paths, utf16_range_to_byte_range,
    };
    use crate::history::{HistoryEntry, HistorySource};
    use crate::scroll::ManualScrollState;
    use std::{
        collections::{HashSet, VecDeque},
        path::PathBuf,
    };

    #[test]
    fn utf16_ranges_round_trip_for_mixed_language_and_surrogate_pair_text() {
        let text = "Hello, 中文 👋";
        let chinese_start = text.find('中').unwrap();
        let emoji_start = text.find('👋').unwrap();
        let range = chinese_start..emoji_start;

        let utf16 = byte_range_to_utf16_range(text, &range);
        assert_eq!(&text[utf16_range_to_byte_range(text, &utf16)], "中文 ");

        let emoji_utf16 = byte_range_to_utf16_range(text, &(emoji_start..text.len()));
        assert_eq!(emoji_utf16.end - emoji_utf16.start, 2);
        assert_eq!(
            utf16_range_to_byte_range(text, &emoji_utf16),
            emoji_start..text.len()
        );
    }

    #[test]
    fn selected_history_paths_keep_only_current_matches_in_history_order() {
        let first = PathBuf::from("F:/captures/invoice.png");
        let second = PathBuf::from("F:/captures/pinned.png");
        let stale = PathBuf::from("F:/captures/removed.png");
        let entries = VecDeque::from([
            HistoryEntry {
                path: first.clone(),
                created_at_ms: 3,
                source: HistorySource::Selection,
            },
            HistoryEntry {
                path: second,
                created_at_ms: 2,
                source: HistorySource::Pinned,
            },
        ]);
        let selected = HashSet::from([first.clone(), stale]);

        assert_eq!(
            selected_history_paths(&entries, &selected, HistoryFilter::All, "invoice"),
            vec![first]
        );
    }

    #[test]
    fn scrolling_history_filter_matches_only_scrolling_captures() {
        let entry = HistoryEntry {
            path: PathBuf::from("F:/captures/long-page.png"),
            created_at_ms: 1,
            source: HistorySource::Scrolling,
        };

        assert!(history_entry_matches(&entry, HistoryFilter::Scrolling, ""));
        assert!(!history_entry_matches(&entry, HistoryFilter::Selection, ""));
        assert_eq!(HistorySource::Scrolling.label(), "Scrolling screenshot");
    }

    #[test]
    fn manual_scroll_state_labels_are_stable_for_acceptance_reports() {
        assert_eq!(manual_scroll_state_label(ManualScrollState::Idle), "idle");
        assert_eq!(
            manual_scroll_state_label(ManualScrollState::Collecting),
            "collecting"
        );
        assert_eq!(
            manual_scroll_state_label(ManualScrollState::Completed),
            "completed"
        );
        assert_eq!(
            manual_scroll_state_label(ManualScrollState::Cancelled),
            "cancelled"
        );
        assert_eq!(
            manual_scroll_state_label(ManualScrollState::Failed),
            "failed"
        );
    }
}
