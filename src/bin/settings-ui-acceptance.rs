//! Deterministic native screenshots of the real GPUI settings surface.

use std::{
    fs, io,
    path::PathBuf,
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
    UI::WindowsAndMessaging::{
        EnumWindows, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible,
    },
};
#[cfg(windows)]
use windows_sys::core::BOOL;

const RENDER_SETTLE_DELAY: Duration = Duration::from_millis(1_500);

#[derive(Debug)]
struct Options {
    theme: ThemeMode,
    width: f32,
    height: f32,
    output: PathBuf,
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
        })
    }
}

fn usage() -> String {
    "usage: settings-ui-acceptance <dark|light> <width> <height> <output.png>".to_owned()
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

    spawn_screenshot_worker(output);
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

/// Waits for GPUI to paint, captures this process's visible top-level window, then exits.
fn spawn_screenshot_worker(output: PathBuf) {
    thread::spawn(move || {
        thread::sleep(RENDER_SETTLE_DELAY);
        let result = visible_process_window_bounds().and_then(|bounds| {
            let frame = SystemCaptureBackend.capture(bounds)?;
            frame.save_png(output)
        });
        match result {
            Ok(()) => process::exit(0),
            Err(error) => {
                eprintln!("settings UI screenshot failed: {error}");
                process::exit(1);
            }
        }
    });
}

/// Finds the first visible top-level HWND owned by this acceptance process.
#[cfg(windows)]
fn visible_process_window_bounds() -> io::Result<flash_shot::domain::geometry::PhysicalRect> {
    struct Search {
        process_id: u32,
        rect: Option<RECT>,
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
            search.rect = Some(rect);
            return 0;
        }
        1
    }

    let mut search = Search {
        process_id: unsafe { GetCurrentProcessId() },
        rect: None,
    };
    unsafe { EnumWindows(Some(callback), &mut search as *mut Search as LPARAM) };
    let rect = search.rect.ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "visible settings window not found")
    })?;
    Ok(flash_shot::domain::geometry::PhysicalRect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    })
}

#[cfg(not(windows))]
fn visible_process_window_bounds() -> io::Result<flash_shot::domain::geometry::PhysicalRect> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "settings UI screenshots are currently Windows-only",
    ))
}
