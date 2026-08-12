//! Flash Shot application library.

pub mod annotation_stress;
pub mod app;
pub mod capture_stress;
pub mod copy_performance;
pub mod diagnostics;
pub mod domain;
pub mod export_stress;
pub mod history;
pub mod image;
pub mod ocr;
pub mod performance;
pub mod performance_report;
pub mod platform;
pub mod png_stress;
pub mod recording;
pub mod scroll;
pub mod settings;
pub mod single_instance;
pub mod theme;
pub mod translation;
pub mod update;

use app::FlashShotApp;
use gpui::*;
use history::ScreenshotHistory;
use performance::PerformanceRecorder;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use settings::UserSettings;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{Sender, SyncSender},
    },
    time::{Duration, Instant},
};

actions!(flash_shot, [Quit]);

fn build_menus() -> Vec<Menu> {
    vec![Menu {
        name: "Flash Shot".into(),
        items: vec![MenuItem::action("Quit Flash Shot", Quit)],
        disabled: false,
    }]
}

/// Starts the native GPUI application.
pub fn run(
    started_at: Instant,
    performance: PerformanceRecorder,
    history: ScreenshotHistory,
    settings: UserSettings,
    settings_path: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    run_with_settings_window(
        started_at,
        performance,
        history,
        settings,
        settings_path,
        SettingsWindowOptions::default(),
    )
}

/// Starts the real settings surface visibly at a deterministic size for native screenshot QA.
///
/// Production startup continues to use [`run`] and remains hidden in the tray. This entry point
/// exists so the repository's acceptance probe can render both themes without UI automation.
pub fn run_settings_ui_acceptance(
    started_at: Instant,
    performance: PerformanceRecorder,
    history: ScreenshotHistory,
    settings: UserSettings,
    settings_path: PathBuf,
    acceptance: SettingsUiAcceptanceOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    run_with_settings_window(
        started_at,
        performance,
        history,
        settings,
        settings_path,
        SettingsWindowOptions {
            width: acceptance.width.max(420.0),
            height: acceptance.height.max(420.0),
            show: true,
            section: acceptance.section,
            recording_state: acceptance.recording_state,
            recording_support_check_state: acceptance.recording_support_check_state,
            translation_service_test_state: acceptance.translation_service_test_state,
            ocr_support_check_state: acceptance.ocr_support_check_state,
            update_check_state: acceptance.update_check_state,
            display_index: acceptance.display_index,
            pinned_saved_feedback_preview: acceptance.pinned_saved_feedback_preview,
            interaction_shortcut_readiness: None,
            interaction_commands: None,
            interaction_copy_results: None,
        },
    )
}

/// Starts a disposable capture overlay for native screenshot acceptance without using the
/// production process entry point or its single-instance mutex.
pub fn run_overlay_ui_acceptance(
    started_at: Instant,
    performance: PerformanceRecorder,
    history: ScreenshotHistory,
    settings: UserSettings,
    settings_path: PathBuf,
    acceptance: OverlayUiAcceptanceOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    run_native_application(move |cx| {
        if let Err(error) = app::open_overlay_ui_acceptance(
            started_at,
            performance,
            history,
            settings,
            settings_path,
            acceptance,
            cx,
        ) {
            log::error!(target: "flash_shot::acceptance", "overlay_ui_open_failed error={error}");
            cx.quit();
        }
    })
}

/// Commands that let the input probe observe recording state and restore the real Record page.
#[derive(Debug)]
pub enum OverlayInteractionAcceptanceCommand {
    Snapshot(SyncSender<OverlayInteractionRecordingState>),
    CaptureSnapshot(SyncSender<OverlayInteractionCaptureState>),
    CaptureContent(SyncSender<OverlayInteractionCaptureContent>),
    ShowCaptureSettings,
    ShowRecordingSettings,
}

/// Minimal production recording state returned to the isolated input probe.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OverlayInteractionRecordingState {
    pub active: bool,
    pub starting: bool,
    pub stopping: bool,
    pub paused: bool,
    pub target: Option<String>,
    pub target_bounds: Option<domain::geometry::PhysicalRect>,
    pub progress_frame: u64,
    pub progress_time_us: u64,
    pub status: String,
}

/// Minimal capture and Pin state returned to the isolated real-input probe.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OverlayInteractionCaptureState {
    pub session_state: String,
    pub selection: Option<domain::geometry::PhysicalRect>,
    /// Current production manual-scroll lifecycle label (for example, `collecting`).
    pub manual_scroll_state: String,
    /// Number of viewport frames accepted by the active manual-scroll session.
    pub manual_scroll_frame_count: usize,
    /// Whether the current manual-scroll session has enough frames to stitch and finish.
    pub manual_scroll_can_finish: bool,
    /// Whether a native frame capture is currently being appended asynchronously.
    pub manual_scroll_capture_in_flight: bool,
    /// Whether an assisted scroll has been requested and is waiting for its delayed capture.
    pub manual_scroll_auto_capture_pending: bool,
    /// Selection used as the fixed viewport for the active manual-scroll session.
    pub manual_scroll_selection: Option<domain::geometry::PhysicalRect>,
    pub overlay_count: usize,
    pub more_actions_visible: bool,
    pub annotation_controls_visible: bool,
    pub pinned_count: usize,
    pub pinned_source_bounds: Option<domain::geometry::PhysicalRect>,
    pub capture_preflight_ready: bool,
    pub status: String,
}

/// Exact process-local frames used to prove Save, Copy, and Pin content without global state.
#[derive(Clone, Debug)]
pub struct OverlayInteractionCaptureContent {
    pub selection: Option<platform::capture::CaptureFrame>,
    pub pins: Vec<platform::capture::CaptureFrame>,
}

/// Process-local controls for one isolated real-input overlay acceptance session.
pub struct OverlayInteractionAcceptanceOptions {
    pub window_width: f32,
    pub window_height: f32,
    pub shortcut_readiness: SyncSender<bool>,
    pub commands: async_channel::Receiver<OverlayInteractionAcceptanceCommand>,
    /// `Some` redirects Copy into a process-local observer; `None` exercises `SystemClipboard`.
    pub copy_results: Option<Sender<platform::capture::CaptureFrame>>,
}

impl OverlayInteractionAcceptanceOptions {
    /// Converts public runner inputs into the private window setup while preserving clipboard mode.
    fn into_settings_window_options(self) -> SettingsWindowOptions {
        SettingsWindowOptions {
            width: self.window_width.max(420.0),
            height: self.window_height.max(420.0),
            show: true,
            section: "capture".to_owned(),
            interaction_shortcut_readiness: Some(self.shortcut_readiness),
            interaction_commands: Some(self.commands),
            interaction_copy_results: self.copy_results,
            ..SettingsWindowOptions::default()
        }
    }
}

/// Process-local clipboard used only by the real-input acceptance entry point.
struct OverlayInteractionClipboard(Sender<platform::capture::CaptureFrame>);

impl platform::clipboard::ClipboardService for OverlayInteractionClipboard {
    fn copy_image(&self, frame: &platform::capture::CaptureFrame) -> std::io::Result<()> {
        self.0.send(frame.clone()).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "overlay interaction copy receiver was dropped",
            )
        })
    }

    fn copy_text(&self, _text: &str) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "overlay interaction acceptance only records image copies",
        ))
    }
}

#[cfg(test)]
mod overlay_interaction_clipboard_tests {
    use super::{OverlayInteractionAcceptanceOptions, OverlayInteractionClipboard};
    use crate::platform::{
        capture::{CaptureFrame, PixelFormat},
        clipboard::ClipboardService,
    };
    use std::{
        sync::{Arc, mpsc, mpsc::sync_channel},
        time::Duration,
    };

    fn frame() -> CaptureFrame {
        CaptureFrame {
            bounds: crate::domain::geometry::PhysicalRect {
                left: -4,
                top: 7,
                right: -2,
                bottom: 8,
            },
            width: 2,
            height: 1,
            stride: 8,
            format: PixelFormat::Bgra8,
            pixels: Arc::from([1, 2, 3, 255, 4, 5, 6, 255]),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 2,
        }
    }

    #[test]
    fn acceptance_clipboard_sends_the_exact_cropped_frame() {
        let (sender, receiver) = mpsc::channel();
        let clipboard = OverlayInteractionClipboard(sender);
        let expected = frame();

        clipboard.copy_image(&expected).unwrap();
        let copied = receiver.recv().unwrap();

        assert_eq!(copied.bounds, expected.bounds);
        assert_eq!((copied.width, copied.height), (2, 1));
        assert_eq!(copied.pixels, expected.pixels);
    }

    #[test]
    fn acceptance_clipboard_fails_when_the_observer_is_gone() {
        let (sender, receiver) = mpsc::channel();
        drop(receiver);
        let error = OverlayInteractionClipboard(sender)
            .copy_image(&frame())
            .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn acceptance_options_preserve_injected_clipboard_mode() {
        let (readiness, _readiness_results) = sync_channel(1);
        let (_commands, command_results) = async_channel::unbounded();
        let (copy_results, _copied_frames) = mpsc::channel();

        let options = OverlayInteractionAcceptanceOptions {
            window_width: 800.0,
            window_height: 600.0,
            shortcut_readiness: readiness,
            commands: command_results,
            copy_results: Some(copy_results),
        }
        .into_settings_window_options();

        assert!(options.interaction_copy_results.is_some());
    }

    #[test]
    fn acceptance_options_preserve_system_clipboard_mode() {
        let (readiness, _readiness_results) = sync_channel(1);
        let (_commands, command_results) = async_channel::unbounded();

        let options = OverlayInteractionAcceptanceOptions {
            window_width: 800.0,
            window_height: 600.0,
            shortcut_readiness: readiness,
            commands: command_results,
            copy_results: None,
        }
        .into_settings_window_options();

        assert!(options.interaction_copy_results.is_none());
    }
}

/// Starts the real capture service with a visible, disposable settings window for input-driven QA.
///
/// The caller supplies isolated settings and history paths. Unlike the production binary, this
/// entry point does not acquire the single-instance mutex, so an acceptance process can exercise
/// the normal global-shortcut and overlay workflow without reading the user's profile.
pub fn run_overlay_interaction_acceptance(
    started_at: Instant,
    performance: PerformanceRecorder,
    history: ScreenshotHistory,
    settings: UserSettings,
    settings_path: PathBuf,
    acceptance: OverlayInteractionAcceptanceOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    run_with_settings_window(
        started_at,
        performance,
        history,
        settings,
        settings_path,
        acceptance.into_settings_window_options(),
    )
}

/// Runtime inputs for one isolated, no-input multi-Pin lifecycle acceptance session.
#[derive(Clone, Debug)]
pub struct PinLifecycleAcceptanceOptions {
    pub session_root: PathBuf,
    pub display: crate::platform::display::DisplayInfo,
    pub timeout: Duration,
    pub settle_delay: Duration,
}

/// Runs three real Pin windows without the production tray, hotkeys, or single-instance mutex.
pub fn run_pin_lifecycle_acceptance(
    performance: PerformanceRecorder,
    history: ScreenshotHistory,
    settings: UserSettings,
    settings_path: PathBuf,
    acceptance: PinLifecycleAcceptanceOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    run_native_application(move |cx| {
        if let Err(error) = app::open_pin_lifecycle_acceptance(
            performance,
            history,
            settings,
            settings_path,
            acceptance,
            cx,
        ) {
            log::error!(target: "flash_shot::acceptance", "pin_lifecycle_open_failed error={error}");
            std::process::exit(1);
        }
    })?;
    Err(std::io::Error::other("GPUI exited before Pin lifecycle acceptance completed").into())
}

/// Describes the disposable settings window rendered by the native screenshot acceptance probe.
#[derive(Clone, Debug)]
pub struct SettingsUiAcceptanceOptions {
    pub width: f32,
    pub height: f32,
    pub section: String,
    /// Seeds a deterministic Record page state without launching FFmpeg.
    pub recording_state: RecordingUiAcceptanceState,
    /// Seeds the FFmpeg support-check button without probing the installed executable.
    pub recording_support_check_state: RecordingSupportUiAcceptanceState,
    /// Seeds the explicit translation-service test button without making a network request.
    pub translation_service_test_state: TranslationServiceUiAcceptanceState,
    /// Seeds the local OCR support button without probing the installed OCR executable.
    pub ocr_support_check_state: OcrSupportUiAcceptanceState,
    /// Seeds the update check button without contacting a release endpoint.
    pub update_check_state: UpdateUiAcceptanceState,
    /// Selects a zero-based Windows display index for multi-monitor DPI acceptance runs.
    pub display_index: Option<usize>,
    /// Opens a disposable Pin window that displays the same saved-state feedback as production.
    pub pinned_saved_feedback_preview: bool,
}

/// Describes a synthetic but fully rendered capture overlay used for native screenshot QA.
#[derive(Clone, Copy, Debug)]
pub struct OverlayUiAcceptanceOptions {
    pub width: f32,
    pub height: f32,
    pub scenario: OverlayUiAcceptanceScenario,
}

/// Selects the fixed overlay state shown by the native screenshot acceptance probe.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OverlayUiAcceptanceScenario {
    /// Shows an uncommitted smart target under the current pointer.
    SmartTarget {
        kind: platform::window_inspector::InspectionKind,
    },
    /// Shows a committed region and optionally opens the progressive action menu.
    SelectedRegion {
        placement: OverlayUiAcceptanceSelectionPlacement,
        show_more_actions: bool,
    },
}

/// Positions the fixed region used to exercise an overlay layout branch in a screenshot probe.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OverlayUiAcceptanceSelectionPlacement {
    /// Keeps the compact region in the middle of the overlay for dense-toolbar review.
    #[default]
    Centered,
    /// Forces the toolbar and More menu to use their bottom-right edge avoidance paths.
    BottomRight,
}

/// Synthetic Record page states used only by the native screenshot acceptance probe.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RecordingUiAcceptanceState {
    #[default]
    Idle,
    Starting,
    Recording,
    Paused,
    Stopping,
    Cancelled,
    Failed,
}

/// Synthetic FFmpeg support-check states used only by the native screenshot acceptance probe.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RecordingSupportUiAcceptanceState {
    #[default]
    Idle,
    Checking,
}

/// Synthetic translation-service button states used only by the native screenshot acceptance probe.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TranslationServiceUiAcceptanceState {
    #[default]
    Idle,
    Testing,
    Ready,
}

/// Synthetic local-OCR support button states used only by the native screenshot acceptance probe.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OcrSupportUiAcceptanceState {
    #[default]
    Idle,
    Checking,
}

/// Synthetic update-check states used only by the native screenshot acceptance probe.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum UpdateUiAcceptanceState {
    #[default]
    Idle,
    Checking,
}

#[derive(Clone, Debug)]
struct SettingsWindowOptions {
    width: f32,
    height: f32,
    show: bool,
    section: String,
    recording_state: RecordingUiAcceptanceState,
    recording_support_check_state: RecordingSupportUiAcceptanceState,
    translation_service_test_state: TranslationServiceUiAcceptanceState,
    ocr_support_check_state: OcrSupportUiAcceptanceState,
    update_check_state: UpdateUiAcceptanceState,
    display_index: Option<usize>,
    pinned_saved_feedback_preview: bool,
    /// Reports whether the isolated acceptance shortcut was registered before input starts.
    interaction_shortcut_readiness: Option<SyncSender<bool>>,
    /// Receives process-local recording observations and Record-page restore requests.
    interaction_commands: Option<async_channel::Receiver<OverlayInteractionAcceptanceCommand>>,
    /// `Some` installs the acceptance sink; `None` leaves the production system clipboard active.
    interaction_copy_results: Option<Sender<platform::capture::CaptureFrame>>,
}

impl Default for SettingsWindowOptions {
    fn default() -> Self {
        Self {
            width: 520.0,
            height: 640.0,
            show: false,
            section: "capture".to_owned(),
            recording_state: RecordingUiAcceptanceState::Idle,
            recording_support_check_state: RecordingSupportUiAcceptanceState::Idle,
            translation_service_test_state: TranslationServiceUiAcceptanceState::Idle,
            ocr_support_check_state: OcrSupportUiAcceptanceState::Idle,
            update_check_state: UpdateUiAcceptanceState::Idle,
            display_index: None,
            pinned_saved_feedback_preview: false,
            interaction_shortcut_readiness: None,
            interaction_commands: None,
            interaction_copy_results: None,
        }
    }
}

/// Runs the shared GPUI application with only the initial settings-window presentation varied.
fn run_with_settings_window(
    started_at: Instant,
    performance: PerformanceRecorder,
    history: ScreenshotHistory,
    settings: UserSettings,
    settings_path: PathBuf,
    window_options: SettingsWindowOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    let display_id = window_options
        .display_index
        .map(|index| {
            use crate::platform::display::DisplayProvider;

            let displays = crate::platform::display::SystemDisplayProvider.displays()?;
            displays
                .get(index)
                .map(|display| DisplayId::new(display.platform_id))
                .ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!(
                            "display index {index} is unavailable ({} display(s) detected)",
                            displays.len()
                        ),
                    )
                })
        })
        .transpose()?;

    run_native_application(move |cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(
                size(px(window_options.width), px(window_options.height)),
                cx,
            )),
            display_id,
            window_min_size: Some(size(px(420.), px(420.))),
            // Flash Shot runs from its tray icon. The settings surface is restored only
            // when requested, keeping app launch out of the capture workflow.
            show: window_options.show,
            titlebar: Some(TitlebarOptions {
                title: Some("Flash Shot".into()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let initial_section = window_options.section.clone();
        let recording_state = window_options.recording_state;
        let recording_support_check_state = window_options.recording_support_check_state;
        let translation_service_test_state = window_options.translation_service_test_state;
        let ocr_support_check_state = window_options.ocr_support_check_state;
        let update_check_state = window_options.update_check_state;
        let pinned_saved_feedback_preview = window_options.pinned_saved_feedback_preview;
        let interaction_shortcut_readiness = window_options.interaction_shortcut_readiness;
        let interaction_commands = window_options.interaction_commands;
        let interaction_copy_results = window_options.interaction_copy_results;
        if let Err(error) = cx.open_window(options, move |window, cx| {
            let performance = performance.clone();
            let startup_performance = performance.clone();
            let app = if let Some(copy_results) = interaction_copy_results {
                cx.new(|cx| {
                    FlashShotApp::new_for_overlay_interaction(
                        performance,
                        history,
                        settings,
                        settings_path,
                        Arc::new(OverlayInteractionClipboard(copy_results)),
                        cx,
                    )
                })
            } else {
                cx.new(|cx| FlashShotApp::new(performance, history, settings, settings_path, cx))
            };
            if let Some(readiness) = interaction_shortcut_readiness {
                let _ = readiness.send(app.read(cx).capture_shortcut_active_for_acceptance());
            }
            if let Some(commands) = interaction_commands {
                app.update(cx, |_, cx| {
                    FlashShotApp::listen_for_overlay_interaction_commands(commands, cx)
                });
            }
            app.update(cx, |app, _| {
                app.set_settings_section_for_acceptance(&initial_section);
                app.set_recording_state_for_acceptance(recording_state);
                app.set_recording_support_check_for_acceptance(recording_support_check_state);
                app.set_translation_service_test_for_acceptance(translation_service_test_state);
                app.set_ocr_support_check_for_acceptance(ocr_support_check_state);
                app.set_update_check_for_acceptance(update_check_state);
            });
            if pinned_saved_feedback_preview {
                let preview_app = app.clone();
                cx.defer(move |cx| {
                    preview_app.update(cx, |app, cx| app.open_pinned_saved_feedback_preview(cx));
                });
            }
            if let Ok(handle) = window.window_handle()
                && let RawWindowHandle::Win32(handle) = handle.as_raw()
            {
                app.update(cx, |app, _| {
                    app.set_settings_window_handle(handle.hwnd.get())
                });
            }
            // Flash Shot starts as a tray service with its settings window hidden.
            // Measuring readiness after the app installs its shortcut and tray is
            // meaningful here; a hidden settings window may never paint a frame.
            startup_performance.record_duration("startup_to_service_ready", started_at.elapsed());
            // The settings surface is an on-demand control panel, not the
            // application's lifetime. Closing it returns Flash Shot to the tray.
            let close_app = app.clone();
            window.on_window_should_close(cx, move |_, cx| {
                close_app.update(cx, |app, _| app.hide_settings_window());
                false
            });
            app
        }) {
            log::error!(target: "flash_shot::lifecycle", "main_window_open_failed error={error}");
            cx.quit();
        }
    })
}

/// Configures one disposable GPUI process with the shared menu and shutdown conventions.
fn run_native_application(
    startup: impl FnOnce(&mut App) + 'static,
) -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let _guard = runtime.enter();
    gpui_platform::application().run(move |cx| {
        cx.set_menus(build_menus());
        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("ctrl-q", Quit, None),
            KeyBinding::new("alt-f4", Quit, None),
        ]);
        startup(cx);
    });
    Ok(())
}
