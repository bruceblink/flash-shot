//! Deterministic native screenshots of the real GPUI settings surface.

use std::{
    fs, io,
    path::{Path, PathBuf},
    process, thread,
    time::{Duration, Instant},
};

use flash_shot::{
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
            EnumWindows, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible,
        },
    },
};
#[cfg(windows)]
use windows_sys::core::BOOL;

const DEFAULT_RENDER_SETTLE_DELAY: Duration = Duration::from_millis(1_500);
const DEFAULT_LINGER_DELAY: Duration = Duration::ZERO;
const DEFAULT_WINDOWS_DPI: u32 = 96;

#[derive(serde::Serialize)]
struct ScreenshotMetadata {
    screenshot: String,
    physical_bounds: ScreenshotBounds,
    dpi: u32,
    scale_factor: f32,
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
}

#[derive(Debug)]
struct Options {
    theme: ThemeMode,
    width: f32,
    height: f32,
    output: PathBuf,
    settle_delay: Duration,
    linger_delay: Duration,
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
        })
    }
}

fn usage() -> String {
    "usage: settings-ui-acceptance <dark|light> <width> <height> <output.png> [settle-ms] [linger-ms]"
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

    spawn_screenshot_worker(output, options.settle_delay, options.linger_delay);
    flash_shot::run_settings_ui_acceptance(
        Instant::now(),
        performance,
        history,
        settings,
        session_root.join("settings.json"),
        options.width,
        options.height,
    )
}

/// Waits for GPUI to paint, captures the window, optionally keeps it alive, then exits.
fn spawn_screenshot_worker(output: PathBuf, settle_delay: Duration, linger_delay: Duration) {
    thread::spawn(move || {
        thread::sleep(settle_delay);
        let result = visible_process_window().and_then(|window| {
            let frame = SystemCaptureBackend.capture(window.bounds)?;
            frame.save_png(&output)?;
            write_screenshot_metadata(&output, &window)
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

/// Writes machine-readable scale evidence beside a native screenshot without changing its pixels.
fn write_screenshot_metadata(output: &Path, window: &VisibleProcessWindow) -> io::Result<()> {
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
        scale_factor: scale_factor_for_dpi(window.dpi),
    };
    let encoded = serde_json::to_vec_pretty(&metadata).map_err(io::Error::other)?;
    fs::write(screenshot_metadata_path(output), encoded)
}

/// Maps Windows DPI values to the logical-to-physical scale used by the screenshot window.
fn scale_factor_for_dpi(dpi: u32) -> f32 {
    dpi.max(DEFAULT_WINDOWS_DPI) as f32 / DEFAULT_WINDOWS_DPI as f32
}

/// Keeps evidence pairs easy to find by using the screenshot's filename with a JSON extension.
fn screenshot_metadata_path(output: &Path) -> PathBuf {
    output.with_extension("json")
}

#[cfg(test)]
mod tests {
    use super::{
        parse_linger_delay, parse_settle_delay, scale_factor_for_dpi, screenshot_metadata_path,
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
    fn dpi_metadata_uses_standard_windows_scale_factors() {
        assert_eq!(scale_factor_for_dpi(96), 1.0);
        assert_eq!(scale_factor_for_dpi(144), 1.5);
        assert_eq!(scale_factor_for_dpi(192), 2.0);
        assert_eq!(scale_factor_for_dpi(0), 1.0);
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
