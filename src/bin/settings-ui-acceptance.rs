//! Deterministic native screenshots of the real GPUI settings surface.

use std::{
    fs, io,
    path::{Path, PathBuf},
    process, thread,
    time::{Duration, Instant},
};

use flash_shot::{
    OcrSupportUiAcceptanceState, RecordingSupportUiAcceptanceState, RecordingUiAcceptanceState,
    TranslationServiceUiAcceptanceState, UpdateUiAcceptanceState,
    history::ScreenshotHistory,
    performance::PerformanceRecorder,
    platform::capture::{CaptureBackend, SystemCaptureBackend},
    settings::UserSettings,
    theme::ThemeMode,
};
#[cfg(windows)]
use std::ffi::c_void;
#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{LPARAM, RECT},
    System::Threading::GetCurrentProcessId,
    UI::{
        HiDpi::GetDpiForWindow,
        WindowsAndMessaging::{
            BringWindowToTop, EnumWindows, GetWindowRect, GetWindowThreadProcessId, HWND_TOPMOST,
            IsWindowVisible, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetForegroundWindow,
            SetWindowPos,
        },
    },
};
#[cfg(windows)]
use windows_sys::core::BOOL;

const DEFAULT_RENDER_SETTLE_DELAY: Duration = Duration::from_millis(1_500);
const DEFAULT_LINGER_DELAY: Duration = Duration::ZERO;
const DEFAULT_WINDOWS_DPI: u32 = 96;
const SCALE_TOLERANCE: f32 = 0.01;

#[derive(serde::Serialize)]
struct ScreenshotMetadata {
    screenshot: String,
    physical_bounds: ScreenshotBounds,
    dpi: u32,
    scale_factor: f32,
    expected_scale: Option<f32>,
    scale_match: Option<bool>,
}

#[derive(serde::Serialize)]
struct ScreenshotBounds {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

struct VisibleProcessWindow {
    bounds: flash_shot::domain::geometry::PhysicalRect,
    dpi: u32,
    #[cfg(windows)]
    handle: *mut c_void,
}

#[derive(Debug)]
struct Options {
    theme: ThemeMode,
    width: f32,
    height: f32,
    output: PathBuf,
    settle_delay: Duration,
    linger_delay: Duration,
    expected_scale: Option<f32>,
    section: String,
    display_index: Option<usize>,
    recording_state: RecordingUiAcceptanceState,
    recording_support_check_state: RecordingSupportUiAcceptanceState,
    translation_service_test_state: TranslationServiceUiAcceptanceState,
    ocr_support_check_state: OcrSupportUiAcceptanceState,
    update_check_state: UpdateUiAcceptanceState,
    pinned_saved_feedback_preview: bool,
}

impl Options {
    /// Parses one stable positional command used by local and CI acceptance scripts.
    fn parse() -> Result<Self, String> {
        let mut arguments = std::env::args_os().skip(1);
        let theme = arguments
            .next()
            .and_then(|value| value.into_string().ok())
            .ok_or_else(usage)?;
        let width = parse_extent(arguments.next(), "width")?;
        let height = parse_extent(arguments.next(), "height")?;
        let output = arguments.next().map(PathBuf::from).ok_or_else(usage)?;
        let settle_delay = arguments
            .next()
            .map(parse_settle_delay)
            .transpose()?
            .unwrap_or(DEFAULT_RENDER_SETTLE_DELAY);
        let linger_delay = arguments
            .next()
            .map(parse_linger_delay)
            .transpose()?
            .unwrap_or(DEFAULT_LINGER_DELAY);
        let expected_scale = arguments.next().map(parse_expected_scale).transpose()?;
        let section = arguments
            .next()
            .map(parse_section)
            .transpose()?
            .unwrap_or_else(|| "capture".to_owned());
        let display_index = arguments.next().map(parse_display_index).transpose()?;
        let recording_state = arguments
            .next()
            .map(parse_recording_state)
            .transpose()?
            .unwrap_or_default();
        let translation_service_test_state = arguments
            .next()
            .map(parse_translation_service_test_state)
            .transpose()?
            .unwrap_or_default();
        let ocr_support_check_state = arguments
            .next()
            .map(parse_ocr_support_check_state)
            .transpose()?
            .unwrap_or_default();
        let recording_support_check_state = arguments
            .next()
            .map(parse_recording_support_check_state)
            .transpose()?
            .unwrap_or_default();
        let update_check_state = arguments
            .next()
            .map(parse_update_check_state)
            .transpose()?
            .unwrap_or_default();
        let pinned_saved_feedback_preview = arguments
            .next()
            .map(parse_pinned_saved_feedback_preview)
            .transpose()?
            .unwrap_or(false);
        if arguments.next().is_some() {
            return Err(usage());
        }
        let theme = match theme.as_str() {
            "dark" => ThemeMode::Dark,
            "light" => ThemeMode::Light,
            _ => return Err("theme must be 'dark' or 'light'".to_owned()),
        };
        Ok(Self {
            theme,
            width,
            height,
            output,
            settle_delay,
            linger_delay,
            expected_scale,
            section,
            display_index,
            recording_state,
            recording_support_check_state,
            translation_service_test_state,
            ocr_support_check_state,
            update_check_state,
            pinned_saved_feedback_preview,
        })
    }
}

fn usage() -> String {
    "usage: settings-ui-acceptance <dark|light> <width> <height> <output.png> [settle-ms] [linger-ms] [expected-scale] [capture|library|record|app] [display-index] [idle|starting|recording|paused|stopping|cancelled] [translation-idle|translation-testing|translation-ready] [ocr-idle|ocr-checking] [recording-support-idle|recording-support-checking] [update-idle|update-checking] [settings|pin-saved-feedback]"
        .to_owned()
}

fn parse_extent(value: Option<std::ffi::OsString>, name: &str) -> Result<f32, String> {
    let value = value
        .and_then(|value| value.into_string().ok())
        .ok_or_else(usage)?;
    let extent = value
        .parse::<f32>()
        .map_err(|_| format!("{name} must be a number"))?;
    if !extent.is_finite() || extent < 420.0 {
        return Err(format!("{name} must be at least 420"));
    }
    Ok(extent)
}

fn parse_settle_delay(value: std::ffi::OsString) -> Result<Duration, String> {
    let milliseconds = value
        .into_string()
        .map_err(|_| "settle-ms must be a number".to_owned())?
        .parse::<u64>()
        .map_err(|_| "settle-ms must be a number".to_owned())?;
    if !(500..=60_000).contains(&milliseconds) {
        return Err("settle-ms must be between 500 and 60000".to_owned());
    }
    Ok(Duration::from_millis(milliseconds))
}

/// Parses an optional post-screenshot lifetime for acceptance windows used as recording targets.
fn parse_linger_delay(value: std::ffi::OsString) -> Result<Duration, String> {
    let milliseconds = value
        .into_string()
        .map_err(|_| "linger-ms must be a number".to_owned())?
        .parse::<u64>()
        .map_err(|_| "linger-ms must be a number".to_owned())?;
    if milliseconds > 300_000 {
        return Err("linger-ms must be between 0 and 300000".to_owned());
    }
    Ok(Duration::from_millis(milliseconds))
}

/// Parses the optional scale required by a high-DPI acceptance run.
/// Keeping it explicit prevents a 100% screenshot from being mistaken for 150% or 200% evidence.
fn parse_expected_scale(value: std::ffi::OsString) -> Result<f32, String> {
    let value = value
        .into_string()
        .map_err(|_| "expected-scale must be a number".to_owned())?;
    let scale = value
        .parse::<f32>()
        .map_err(|_| "expected-scale must be a number".to_owned())?;
    if !scale.is_finite() || !(1.0..=4.0).contains(&scale) {
        return Err("expected-scale must be between 1.0 and 4.0".to_owned());
    }
    Ok(scale)
}

/// Parses the settings workflow that a disposable screenshot should open first.
fn parse_section(value: std::ffi::OsString) -> Result<String, String> {
    let section = value
        .into_string()
        .map_err(|_| "section must be capture, library, record, or app".to_owned())?;
    if matches!(section.as_str(), "capture" | "library" | "record" | "app") {
        Ok(section)
    } else {
        Err("section must be capture, library, record, or app".to_owned())
    }
}

/// Parses a zero-based display index so DPI evidence can target a specific monitor.
fn parse_display_index(value: std::ffi::OsString) -> Result<usize, String> {
    value
        .into_string()
        .map_err(|_| "display-index must be a non-negative integer".to_owned())?
        .parse::<usize>()
        .map_err(|_| "display-index must be a non-negative integer".to_owned())
}

/// Parses a synthetic Record page state used for deterministic lifecycle screenshots.
fn parse_recording_state(value: std::ffi::OsString) -> Result<RecordingUiAcceptanceState, String> {
    match value
        .into_string()
        .map_err(|_| {
            "recording-state must be idle, starting, recording, paused, stopping, or cancelled"
                .to_owned()
        })?
        .as_str()
    {
        "idle" => Ok(RecordingUiAcceptanceState::Idle),
        "starting" => Ok(RecordingUiAcceptanceState::Starting),
        "recording" => Ok(RecordingUiAcceptanceState::Recording),
        "paused" => Ok(RecordingUiAcceptanceState::Paused),
        "stopping" => Ok(RecordingUiAcceptanceState::Stopping),
        "cancelled" => Ok(RecordingUiAcceptanceState::Cancelled),
        _ => Err(
            "recording-state must be idle, starting, recording, paused, stopping, or cancelled"
                .to_owned(),
        ),
    }
}

/// Parses a synthetic translation-service test state for deterministic settings screenshots.
fn parse_translation_service_test_state(
    value: std::ffi::OsString,
) -> Result<TranslationServiceUiAcceptanceState, String> {
    match value
        .into_string()
        .map_err(|_| {
            "translation-state must be translation-idle, translation-testing, or translation-ready"
                .to_owned()
        })?
        .as_str()
    {
        "translation-idle" => Ok(TranslationServiceUiAcceptanceState::Idle),
        "translation-testing" => Ok(TranslationServiceUiAcceptanceState::Testing),
        "translation-ready" => Ok(TranslationServiceUiAcceptanceState::Ready),
        _ => Err(
            "translation-state must be translation-idle, translation-testing, or translation-ready"
                .to_owned(),
        ),
    }
}

/// Parses a synthetic local-OCR support state for deterministic settings screenshots.
fn parse_ocr_support_check_state(
    value: std::ffi::OsString,
) -> Result<OcrSupportUiAcceptanceState, String> {
    match value
        .into_string()
        .map_err(|_| "ocr-state must be ocr-idle or ocr-checking".to_owned())?
        .as_str()
    {
        "ocr-idle" => Ok(OcrSupportUiAcceptanceState::Idle),
        "ocr-checking" => Ok(OcrSupportUiAcceptanceState::Checking),
        _ => Err("ocr-state must be ocr-idle or ocr-checking".to_owned()),
    }
}

/// Parses a synthetic FFmpeg support-check state for deterministic settings screenshots.
fn parse_recording_support_check_state(
    value: std::ffi::OsString,
) -> Result<RecordingSupportUiAcceptanceState, String> {
    match value
        .into_string()
        .map_err(|_| {
            "recording-support-state must be recording-support-idle or recording-support-checking"
                .to_owned()
        })?
        .as_str()
    {
        "recording-support-idle" => Ok(RecordingSupportUiAcceptanceState::Idle),
        "recording-support-checking" => Ok(RecordingSupportUiAcceptanceState::Checking),
        _ => Err(
            "recording-support-state must be recording-support-idle or recording-support-checking"
                .to_owned(),
        ),
    }
}

/// Parses a synthetic update-check state for deterministic settings screenshots.
fn parse_update_check_state(value: std::ffi::OsString) -> Result<UpdateUiAcceptanceState, String> {
    match value
        .into_string()
        .map_err(|_| "update-state must be update-idle or update-checking".to_owned())?
        .as_str()
    {
        "update-idle" => Ok(UpdateUiAcceptanceState::Idle),
        "update-checking" => Ok(UpdateUiAcceptanceState::Checking),
        _ => Err("update-state must be update-idle or update-checking".to_owned()),
    }
}

/// Selects the ordinary settings page or a Pin window after its saved-state feedback appears.
fn parse_pinned_saved_feedback_preview(value: std::ffi::OsString) -> Result<bool, String> {
    match value
        .into_string()
        .map_err(|_| "surface must be settings or pin-saved-feedback".to_owned())?
        .as_str()
    {
        "settings" => Ok(false),
        "pin-saved-feedback" => Ok(true),
        _ => Err("surface must be settings or pin-saved-feedback".to_owned()),
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("settings UI acceptance failed: {error}");
        process::exit(1);
    }
}

/// Starts a disposable app instance while a worker captures its visible native window.
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let options = Options::parse().map_err(io::Error::other)?;
    let output = std::path::absolute(&options.output)?;
    let evidence_root = output
        .parent()
        .ok_or_else(|| io::Error::other("screenshot output requires a parent directory"))?;
    fs::create_dir_all(evidence_root)?;
    let session_root = evidence_root.join(format!("settings-ui-{}", process::id()));
    let history_root = session_root.join("history");
    fs::create_dir_all(&history_root)?;

    let history = ScreenshotHistory::open_with_limit(&history_root, 30)?;
    let performance = PerformanceRecorder::new(session_root.join("metrics"))?;
    let mut settings = UserSettings::default();
    settings.theme_mode = options.theme;

    spawn_screenshot_worker(
        output,
        options.settle_delay,
        options.linger_delay,
        options.expected_scale,
    );
    flash_shot::run_settings_ui_acceptance(
        Instant::now(),
        performance,
        history,
        settings,
        session_root.join("settings.json"),
        flash_shot::SettingsUiAcceptanceOptions {
            width: options.width,
            height: options.height,
            section: options.section,
            display_index: options.display_index,
            recording_state: options.recording_state,
            recording_support_check_state: options.recording_support_check_state,
            translation_service_test_state: options.translation_service_test_state,
            ocr_support_check_state: options.ocr_support_check_state,
            update_check_state: options.update_check_state,
            pinned_saved_feedback_preview: options.pinned_saved_feedback_preview,
        },
    )
}

/// Waits for GPUI to paint, captures the window, optionally keeps it alive, then exits.
fn spawn_screenshot_worker(
    output: PathBuf,
    settle_delay: Duration,
    linger_delay: Duration,
    expected_scale: Option<f32>,
) {
    thread::spawn(move || {
        thread::sleep(settle_delay);
        let result = visible_process_window().and_then(|window| {
            focus_process_window(&window)?;
            thread::sleep(Duration::from_millis(100));
            let frame = SystemCaptureBackend.capture(window.bounds)?;
            frame.save_png(&output)?;
            write_screenshot_metadata(&output, &window, expected_scale)?;
            if let Some(expected_scale) = expected_scale {
                let actual_scale = scale_factor_for_dpi(window.dpi);
                if !scale_matches(actual_scale, expected_scale) {
                    return Err(io::Error::other(format!(
                        "expected Windows scale {expected_scale:.2}, observed {actual_scale:.2}"
                    )));
                }
            }
            Ok(())
        });
        match result {
            Ok(()) => {
                thread::sleep(linger_delay);
                process::exit(0);
            }
            Err(error) => {
                eprintln!("settings UI screenshot failed: {error}");
                process::exit(1);
            }
        }
    });
}

/// Finds this process's visible native window together with its active DPI scaling.
#[cfg(windows)]
fn visible_process_window() -> io::Result<VisibleProcessWindow> {
    struct Search {
        process_id: u32,
        window: Option<VisibleProcessWindow>,
    }

    unsafe extern "system" fn callback(window: *mut c_void, parameter: LPARAM) -> BOOL {
        let search = unsafe { &mut *(parameter as *mut Search) };
        let mut process_id = 0;
        unsafe { GetWindowThreadProcessId(window, &mut process_id) };
        if process_id != search.process_id || unsafe { IsWindowVisible(window) } == 0 {
            return 1;
        }
        let mut rect = RECT::default();
        if unsafe { GetWindowRect(window, &mut rect) } != 0 {
            // A visible top-level window should always report its monitor DPI. Keep 96 as a
            // conservative fallback for older or transient Windows window handles.
            let dpi = unsafe { GetDpiForWindow(window) }.max(DEFAULT_WINDOWS_DPI);
            search.window = Some(VisibleProcessWindow {
                bounds: flash_shot::domain::geometry::PhysicalRect {
                    left: rect.left,
                    top: rect.top,
                    right: rect.right,
                    bottom: rect.bottom,
                },
                dpi,
                handle: window,
            });
            return 0;
        }
        1
    }

    let mut search = Search {
        process_id: unsafe { GetCurrentProcessId() },
        window: None,
    };
    unsafe { EnumWindows(Some(callback), &mut search as *mut Search as LPARAM) };
    search
        .window
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "visible settings window not found"))
}

/// Brings the disposable settings window above every normal desktop window before capture.
///
/// Windows may deny a foreground request from a background acceptance process. Temporarily making
/// this short-lived process window topmost gives the desktop-region capture the same pixels a
/// reviewer sees instead of silently recording an occluding terminal or File Explorer window.
#[cfg(windows)]
fn focus_process_window(window: &VisibleProcessWindow) -> io::Result<()> {
    // SAFETY: the handle was returned by EnumWindows for this live process and remains valid
    // until the acceptance process exits after the screenshot is written.
    let topmost = unsafe {
        SetWindowPos(
            window.handle,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )
    } != 0;
    let focused = unsafe { SetForegroundWindow(window.handle) } != 0;
    let raised = unsafe { BringWindowToTop(window.handle) } != 0;
    if topmost || focused || raised {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(windows))]
fn focus_process_window(_window: &VisibleProcessWindow) -> io::Result<()> {
    Ok(())
}

/// Writes machine-readable scale evidence beside a native screenshot without changing its pixels.
fn write_screenshot_metadata(
    output: &Path,
    window: &VisibleProcessWindow,
    expected_scale: Option<f32>,
) -> io::Result<()> {
    let scale_factor = scale_factor_for_dpi(window.dpi);
    let metadata = ScreenshotMetadata {
        screenshot: output
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("settings-ui.png")
            .to_owned(),
        physical_bounds: ScreenshotBounds {
            left: window.bounds.left,
            top: window.bounds.top,
            right: window.bounds.right,
            bottom: window.bounds.bottom,
        },
        dpi: window.dpi,
        scale_factor,
        expected_scale,
        scale_match: expected_scale.map(|expected| scale_matches(scale_factor, expected)),
    };
    let encoded = serde_json::to_vec_pretty(&metadata).map_err(io::Error::other)?;
    fs::write(screenshot_metadata_path(output), encoded)
}

/// Maps Windows DPI values to the logical-to-physical scale used by the screenshot window.
fn scale_factor_for_dpi(dpi: u32) -> f32 {
    dpi.max(DEFAULT_WINDOWS_DPI) as f32 / DEFAULT_WINDOWS_DPI as f32
}

/// Compares observed and requested scale with a small tolerance for fractional DPI rounding.
fn scale_matches(observed: f32, expected: f32) -> bool {
    (observed - expected).abs() <= SCALE_TOLERANCE
}

/// Keeps evidence pairs easy to find by using the screenshot's filename with a JSON extension.
fn screenshot_metadata_path(output: &Path) -> PathBuf {
    output.with_extension("json")
}

#[cfg(test)]
mod tests {
    use super::{
        parse_display_index, parse_expected_scale, parse_linger_delay,
        parse_ocr_support_check_state, parse_pinned_saved_feedback_preview, parse_recording_state,
        parse_recording_support_check_state, parse_section, parse_settle_delay,
        parse_translation_service_test_state, parse_update_check_state, scale_factor_for_dpi,
        scale_matches, screenshot_metadata_path,
    };
    use flash_shot::{
        OcrSupportUiAcceptanceState, RecordingSupportUiAcceptanceState, RecordingUiAcceptanceState,
        TranslationServiceUiAcceptanceState, UpdateUiAcceptanceState,
    };
    use std::{ffi::OsString, path::Path, time::Duration};

    #[test]
    fn acceptance_linger_delay_is_bounded_and_optional() {
        assert_eq!(
            parse_linger_delay(OsString::from("120000")).unwrap(),
            Duration::from_secs(120)
        );
        assert!(parse_linger_delay(OsString::from("300001")).is_err());
    }

    #[test]
    fn acceptance_settle_delay_rejects_an_unstable_short_wait() {
        assert!(parse_settle_delay(OsString::from("499")).is_err());
        assert_eq!(
            parse_settle_delay(OsString::from("1500")).unwrap(),
            Duration::from_millis(1500)
        );
    }

    #[test]
    fn expected_scale_accepts_standard_dpi_values_and_rejects_invalid_values() {
        assert_eq!(parse_expected_scale(OsString::from("1.5")).unwrap(), 1.5);
        assert_eq!(parse_expected_scale(OsString::from("2.0")).unwrap(), 2.0);
        assert!(parse_expected_scale(OsString::from("0.9")).is_err());
        assert!(parse_expected_scale(OsString::from("not-a-scale")).is_err());
    }

    #[test]
    fn display_index_accepts_zero_based_monitor_selection() {
        assert_eq!(parse_display_index(OsString::from("0")).unwrap(), 0);
        assert_eq!(parse_display_index(OsString::from("12")).unwrap(), 12);
        assert!(parse_display_index(OsString::from("-1")).is_err());
        assert!(parse_display_index(OsString::from("monitor")).is_err());
    }

    #[test]
    fn recording_state_parser_accepts_lifecycle_states() {
        assert_eq!(
            parse_recording_state(OsString::from("paused")).unwrap(),
            RecordingUiAcceptanceState::Paused
        );
        assert_eq!(
            parse_recording_state(OsString::from("cancelled")).unwrap(),
            RecordingUiAcceptanceState::Cancelled
        );
        assert!(parse_recording_state(OsString::from("running")).is_err());
    }

    #[test]
    fn translation_service_test_state_parser_accepts_the_busy_state() {
        assert_eq!(
            parse_translation_service_test_state(OsString::from("translation-testing")).unwrap(),
            TranslationServiceUiAcceptanceState::Testing
        );
        assert!(parse_translation_service_test_state(OsString::from("testing")).is_err());
    }

    #[test]
    fn translation_service_test_state_parser_accepts_the_ready_state() {
        assert_eq!(
            parse_translation_service_test_state(OsString::from("translation-ready")).unwrap(),
            TranslationServiceUiAcceptanceState::Ready
        );
    }

    #[test]
    fn ocr_support_state_parser_accepts_the_busy_state() {
        assert_eq!(
            parse_ocr_support_check_state(OsString::from("ocr-checking")).unwrap(),
            OcrSupportUiAcceptanceState::Checking
        );
        assert!(parse_ocr_support_check_state(OsString::from("checking")).is_err());
    }

    #[test]
    fn recording_support_state_parser_accepts_the_busy_state() {
        assert_eq!(
            parse_recording_support_check_state(OsString::from("recording-support-checking"))
                .unwrap(),
            RecordingSupportUiAcceptanceState::Checking
        );
        assert!(parse_recording_support_check_state(OsString::from("checking")).is_err());
    }

    #[test]
    fn update_state_parser_accepts_the_busy_state() {
        assert_eq!(
            parse_update_check_state(OsString::from("update-checking")).unwrap(),
            UpdateUiAcceptanceState::Checking
        );
        assert!(parse_update_check_state(OsString::from("checking")).is_err());
    }

    #[test]
    fn acceptance_surface_parser_enables_the_saved_pin_preview() {
        assert!(parse_pinned_saved_feedback_preview(OsString::from("pin-saved-feedback")).unwrap());
        assert!(!parse_pinned_saved_feedback_preview(OsString::from("settings")).unwrap());
        assert!(parse_pinned_saved_feedback_preview(OsString::from("pin")).is_err());
    }

    #[test]
    fn section_parser_accepts_each_settings_workflow() {
        for section in ["capture", "library", "record", "app"] {
            assert_eq!(parse_section(section.into()).unwrap(), section);
        }
        assert!(parse_section("unknown".into()).is_err());
    }

    #[test]
    fn dpi_metadata_uses_standard_windows_scale_factors() {
        assert_eq!(scale_factor_for_dpi(96), 1.0);
        assert_eq!(scale_factor_for_dpi(144), 1.5);
        assert_eq!(scale_factor_for_dpi(192), 2.0);
        assert_eq!(scale_factor_for_dpi(0), 1.0);
    }

    #[test]
    fn expected_scale_guard_allows_only_small_dpi_rounding() {
        assert!(scale_matches(1.5, 1.5));
        assert!(scale_matches(1.509, 1.5));
        assert!(!scale_matches(1.52, 1.5));
    }

    #[test]
    fn screenshot_metadata_is_written_beside_the_png() {
        assert_eq!(
            screenshot_metadata_path(Path::new("target/ui-acceptance/settings.png")),
            Path::new("target/ui-acceptance/settings.json")
        );
    }
}

#[cfg(not(windows))]
fn visible_process_window() -> io::Result<VisibleProcessWindow> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "settings UI screenshots are currently Windows-only",
    ))
}
