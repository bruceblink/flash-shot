//! Flash Shot application library.

pub mod annotation_stress;
pub mod app;
pub mod capture_stress;
pub mod diagnostics;
pub mod domain;
pub mod history;
pub mod image;
pub mod ocr;
pub mod performance;
pub mod performance_report;
pub mod platform;
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
use std::{path::PathBuf, time::Instant};

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
    SelectedRegion { show_more_actions: bool },
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
        if let Err(error) = cx.open_window(options, move |window, cx| {
            let performance = performance.clone();
            let startup_performance = performance.clone();
            let app =
                cx.new(|cx| FlashShotApp::new(performance, history, settings, settings_path, cx));
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
