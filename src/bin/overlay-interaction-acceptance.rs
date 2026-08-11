//! Explicit, isolated Windows input probe for the real capture-overlay workflow.

use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process, thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use flash_shot::{
    domain::geometry::{PhysicalPoint, PhysicalRect},
    history::ScreenshotHistory,
    performance::PerformanceRecorder,
    platform::display::{DisplayInfo, DisplayProvider, SystemDisplayProvider},
    settings::UserSettings,
};

#[cfg(windows)]
use flash_shot::platform::capture::{CaptureBackend, SystemCaptureBackend};
#[cfg(windows)]
use std::{
    ffi::c_void,
    mem::size_of,
    panic::AssertUnwindSafe,
    sync::mpsc::{self, Receiver},
};
#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{LPARAM, POINT, RECT},
    System::Threading::GetCurrentProcessId,
    UI::{
        HiDpi::GetDpiForWindow,
        Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP,
            MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE,
            MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT, SendInput, VK_CONTROL, VK_F24, VK_MENU,
        },
        WindowsAndMessaging::{
            BringWindowToTop, EnumWindows, GetCursorPos, GetForegroundWindow, GetSystemMetrics,
            GetWindowRect, GetWindowThreadProcessId, IsWindow, IsWindowVisible, SM_CXVIRTUALSCREEN,
            SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SetCursorPos,
            SetForegroundWindow,
        },
    },
};
#[cfg(windows)]
use windows_sys::core::BOOL;

const CAPTURE_SHORTCUT: &str = "Ctrl+Alt+F24";
const DEFAULT_OUTPUT_DIR: &str = "target/overlay-interaction-acceptance";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);
const DEFAULT_SETTLE_DELAY: Duration = Duration::from_millis(500);
const MIN_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_TIMEOUT: Duration = Duration::from_secs(60);
const MIN_SETTLE_DELAY: Duration = Duration::from_millis(100);
const MAX_SETTLE_DELAY: Duration = Duration::from_secs(5);
const WINDOWS_BASE_DPI: f32 = 96.0;

#[derive(Debug, Eq, PartialEq)]
struct Options {
    allow_input: bool,
    output_dir: PathBuf,
    timeout: Duration,
    settle_delay: Duration,
}

impl Options {
    fn parse() -> Result<Self, String> {
        Self::parse_from(std::env::args_os().skip(1))
    }

    /// Parses named options without performing filesystem, GPUI, or input side effects.
    fn parse_from(arguments: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let mut options = Self {
            allow_input: false,
            output_dir: PathBuf::from(DEFAULT_OUTPUT_DIR),
            timeout: DEFAULT_TIMEOUT,
            settle_delay: DEFAULT_SETTLE_DELAY,
        };
        let mut arguments = arguments.into_iter();
        let mut output_seen = false;
        let mut timeout_seen = false;
        let mut settle_seen = false;
        while let Some(argument) = arguments.next() {
            let argument = argument
                .into_string()
                .map_err(|_| "acceptance options must be valid Unicode".to_owned())?;
            match argument.as_str() {
                "--allow-input" if !options.allow_input => options.allow_input = true,
                "--allow-input" => return Err("--allow-input may only be supplied once".to_owned()),
                "--output-dir" if !output_seen => {
                    options.output_dir = PathBuf::from(
                        arguments
                            .next()
                            .ok_or_else(usage)?
                            .into_string()
                            .map_err(|_| "output directory must be valid Unicode".to_owned())?,
                    );
                    output_seen = true;
                }
                "--timeout-ms" if !timeout_seen => {
                    options.timeout =
                        parse_duration(arguments.next(), "timeout", MIN_TIMEOUT, MAX_TIMEOUT)?;
                    timeout_seen = true;
                }
                "--settle-ms" if !settle_seen => {
                    options.settle_delay = parse_duration(
                        arguments.next(),
                        "settle delay",
                        MIN_SETTLE_DELAY,
                        MAX_SETTLE_DELAY,
                    )?;
                    settle_seen = true;
                }
                "--output-dir" | "--timeout-ms" | "--settle-ms" => {
                    return Err(format!("{argument} may only be supplied once"));
                }
                _ => return Err(usage()),
            }
        }
        if options.output_dir.as_os_str().is_empty() {
            return Err("output directory must not be empty".to_owned());
        }
        Ok(options)
    }
}

fn parse_duration(
    value: Option<OsString>,
    label: &str,
    minimum: Duration,
    maximum: Duration,
) -> Result<Duration, String> {
    let milliseconds = value
        .and_then(|value| value.into_string().ok())
        .ok_or_else(usage)?
        .parse::<u64>()
        .map_err(|_| format!("{label} must be a whole number of milliseconds"))?;
    let duration = Duration::from_millis(milliseconds);
    if !(minimum..=maximum).contains(&duration) {
        return Err(format!(
            "{label} must be between {} and {} milliseconds",
            minimum.as_millis(),
            maximum.as_millis()
        ));
    }
    Ok(duration)
}

fn usage() -> String {
    "usage: overlay-interaction-acceptance --allow-input [--output-dir <path>] [--timeout-ms <3000-60000>] [--settle-ms <100-5000>]".to_owned()
}

/// Refuses before GPUI starts unless the caller explicitly authorizes global input injection.
fn ensure_input_authorized(options: &Options) -> io::Result<()> {
    if options.allow_input {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "real input injection is disabled; rerun with --allow-input on a disposable desktop session",
        ))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct InteractionPlan {
    drag_start: PhysicalPoint,
    drag_end: PhysicalPoint,
    more: PhysicalPoint,
    cancel: PhysicalPoint,
}

/// Converts the overlay's logical toolbar geometry into physical screen points for SendInput.
fn interaction_plan(bounds: PhysicalRect, scale: f32) -> io::Result<InteractionPlan> {
    if !scale.is_finite() || !(1.0..=4.0).contains(&scale) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "overlay scale must be between 1.0 and 4.0",
        ));
    }
    let width = bounds.width() as f32 / scale;
    let height = bounds.height() as f32 / scale;
    if width < 700.0 || height < 500.0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "overlay must be at least 700 x 500 logical pixels for interaction acceptance",
        ));
    }

    let start = (width * 0.22, height * 0.20);
    let end = (width * 0.68, height * 0.50);
    let toolbar_width = 342.0_f32.min(width - 36.0).min(620.0);
    let toolbar_height = 50.0;
    let left_min = 18.0;
    let left_limit = (width - 18.0 - toolbar_width).max(left_min);
    let toolbar_left = (end.0 - toolbar_width).clamp(left_min, left_limit);
    let lowest_top = (height - 96.0 - toolbar_height).max(18.0);
    let below = end.1 + 12.0;
    let above = start.1 - toolbar_height - 12.0;
    let toolbar_top = if below <= lowest_top {
        below
    } else {
        above.max(18.0).min(lowest_top)
    };

    let screen_point = |point: (f32, f32)| PhysicalPoint {
        x: bounds.left + (point.0 * scale).round() as i32,
        y: bounds.top + (point.1 * scale).round() as i32,
    };
    Ok(InteractionPlan {
        drag_start: screen_point(start),
        drag_end: screen_point(end),
        // The primary row is fixed: Mark, Pin, Copy, Save, More, then Cancel.
        more: screen_point((toolbar_left + 247.0, toolbar_top + 25.0)),
        cancel: screen_point((toolbar_left + 312.0, toolbar_top + 25.0)),
    })
}

#[derive(serde::Serialize)]
struct AcceptanceReport {
    schema_version: u32,
    test: &'static str,
    status: String,
    process_id: u32,
    shortcut: &'static str,
    shortcut_registered: bool,
    isolated_profile: String,
    display: DisplayReport,
    controller_window: Option<WindowReport>,
    steps: Vec<StepReport>,
    error: Option<String>,
}

#[derive(serde::Serialize)]
struct DisplayReport {
    id: String,
    bounds: PhysicalRect,
    dpi_x: u32,
    dpi_y: u32,
    scale_factor: f32,
}

#[derive(serde::Serialize)]
struct WindowReport {
    handle: usize,
    bounds: PhysicalRect,
    dpi: u32,
}

#[derive(serde::Serialize)]
struct StepReport {
    action: &'static str,
    foreground_window: usize,
    screenshot: Option<String>,
    pixel_fingerprint: Option<String>,
}

#[cfg(windows)]
#[derive(Clone, Copy)]
struct NativeWindow {
    handle: *mut c_void,
    bounds: PhysicalRect,
    dpi: u32,
}

#[cfg(windows)]
impl NativeWindow {
    fn report(self) -> WindowReport {
        WindowReport {
            handle: self.handle as usize,
            bounds: self.bounds,
            dpi: self.dpi,
        }
    }
}

#[cfg(windows)]
struct WorkerContext {
    session_root: PathBuf,
    report_path: PathBuf,
    display: DisplayInfo,
    shortcut_readiness: Receiver<bool>,
    timeout: Duration,
    settle_delay: Duration,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("overlay interaction acceptance failed: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let options = Options::parse().map_err(io::Error::other)?;
    ensure_input_authorized(&options)?;
    #[cfg(windows)]
    {
        run_windows(options)
    }
    #[cfg(not(windows))]
    {
        let _ = options;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "overlay interaction acceptance is currently Windows-only",
        )
        .into())
    }
}

#[cfg(windows)]
/// Builds a disposable profile, starts the worker, then hands control to the real GPUI app.
fn run_windows(options: Options) -> Result<(), Box<dyn std::error::Error>> {
    let displays = SystemDisplayProvider.displays()?;
    if displays.len() != 1 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "single-display acceptance requires exactly one display, found {}",
                displays.len()
            ),
        )
        .into());
    }
    let display = displays
        .into_iter()
        .next()
        .expect("one display was checked");
    let output_root = std::path::absolute(options.output_dir)?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let session_root = output_root.join(format!("session-{timestamp}-{}", process::id()));
    fs::create_dir_all(&session_root)?;
    fs::create_dir_all(session_root.join("screenshots"))?;

    // This happens before GPUI or worker threads start, so every later path lookup is isolated.
    unsafe { std::env::set_var("FLASH_SHOT_PROFILE_DIR", &session_root) };
    let settings_path = session_root.join("settings.json");
    let mut settings = UserSettings::default();
    settings.capture_shortcut = Some(CAPTURE_SHORTCUT.to_owned());
    settings.full_screen_shortcut = None;
    settings.focused_window_shortcut = None;
    settings.capture_shortcut_enabled = true;
    settings.include_cursor = false;
    settings.quick_save_directory = Some(session_root.join("history"));
    settings.recording_directory = Some(session_root.join("recordings"));
    settings.save(&settings_path)?;
    let history = ScreenshotHistory::open_with_limit(session_root.join("history"), 30)?;
    let performance = PerformanceRecorder::new(session_root.join("metrics"))?;
    let report_path = session_root.join("report.json");
    println!("overlay interaction report: {}", report_path.display());
    let (shortcut_ready_tx, shortcut_ready_rx) = mpsc::sync_channel(1);

    let worker_context = WorkerContext {
        session_root,
        report_path,
        display,
        shortcut_readiness: shortcut_ready_rx,
        timeout: options.timeout,
        settle_delay: options.settle_delay,
    };
    thread::Builder::new()
        .name("overlay-interaction-acceptance".to_owned())
        .spawn(move || {
            let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
                run_interaction_sequence(worker_context)
            }));
            let exit_code = match result {
                Ok(Ok(())) => 0,
                Ok(Err(error)) => {
                    eprintln!("overlay interaction worker failed: {error}");
                    1
                }
                Err(_) => {
                    eprintln!("overlay interaction worker panicked");
                    1
                }
            };
            process::exit(exit_code);
        })?;

    flash_shot::run_overlay_interaction_acceptance(
        Instant::now(),
        performance,
        history,
        settings,
        settings_path,
        shortcut_ready_tx,
    )?;
    Err(io::Error::other("GPUI exited before the interaction worker completed").into())
}

#[cfg(windows)]
/// Waits for shortcut registration, owns cursor restoration, and finalizes one truthful report.
fn run_interaction_sequence(context: WorkerContext) -> io::Result<()> {
    let mut report = AcceptanceReport {
        schema_version: 1,
        test: "overlay_interaction_acceptance",
        status: "running".to_owned(),
        process_id: unsafe { GetCurrentProcessId() },
        shortcut: CAPTURE_SHORTCUT,
        shortcut_registered: false,
        isolated_profile: context.session_root.to_string_lossy().into_owned(),
        display: DisplayReport {
            id: context.display.id.clone(),
            bounds: context.display.physical_bounds,
            dpi_x: context.display.dpi_x,
            dpi_y: context.display.dpi_y,
            scale_factor: context.display.scale_factor,
        },
        controller_window: None,
        steps: Vec::new(),
        error: None,
    };
    write_report(&context.report_path, &report)?;

    let shortcut_registered = match context.shortcut_readiness.recv_timeout(context.timeout) {
        Ok(registered) => registered,
        Err(_) => {
            let error = io::Error::new(
                io::ErrorKind::TimedOut,
                "shortcut readiness was not reported; no input was injected",
            );
            report.status = "failed".to_owned();
            report.error = Some(error.to_string());
            write_report(&context.report_path, &report)?;
            return Err(error);
        }
    };
    if !shortcut_registered {
        let error = io::Error::new(
            io::ErrorKind::AddrInUse,
            "Ctrl+Alt+F24 could not be registered; no input was injected",
        );
        report.status = "failed".to_owned();
        report.error = Some(error.to_string());
        write_report(&context.report_path, &report)?;
        return Err(error);
    }
    report.shortcut_registered = true;
    write_report(&context.report_path, &report)?;
    let cursor = match CursorRestore::capture() {
        Ok(cursor) => cursor,
        Err(error) => {
            report.status = "failed".to_owned();
            report.error = Some(format!("cursor snapshot failed before input: {error}"));
            write_report(&context.report_path, &report)?;
            return Err(error);
        }
    };

    let outcome = execute_interactions(&context, &mut report);
    let cursor_result = cursor.restore();
    let final_result = match (outcome, cursor_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(io::Error::other(format!(
            "interaction steps passed but cursor restore failed: {error}"
        ))),
        (Err(error), Err(cursor_error)) => Err(io::Error::other(format!(
            "{error}; cursor restore also failed: {cursor_error}"
        ))),
    };
    match &final_result {
        Ok(()) => report.status = "passed".to_owned(),
        Err(error) => {
            report.status = "failed".to_owned();
            report.error = Some(error.to_string());
        }
    }
    let report_result = write_report(&context.report_path, &report);
    match (final_result, report_result) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

#[cfg(windows)]
/// Drives the two capture lifecycles without invoking Copy, Save, Pin, or recording commands.
fn execute_interactions(context: &WorkerContext, report: &mut AcceptanceReport) -> io::Result<()> {
    let controller = wait_for_controller(context.timeout)?;
    focus_owned_window(controller, context.timeout)?;
    report.controller_window = Some(controller.report());
    record_step(
        report,
        &context.report_path,
        "controller_ready",
        controller,
        None,
    )?;

    let foreground = inject_capture_shortcut(controller.handle)?;
    record_step(
        report,
        &context.report_path,
        "first_capture_shortcut",
        foreground,
        None,
    )?;
    let first_overlay = wait_for_overlay(
        controller.handle,
        context.display.physical_bounds,
        context.timeout,
    )?;
    focus_owned_window(first_overlay, context.timeout)?;
    thread::sleep(context.settle_delay);
    let scale = (first_overlay.dpi as f32 / WINDOWS_BASE_DPI).max(1.0);
    let plan = interaction_plan(first_overlay.bounds, scale)?;

    let foreground = inject_mouse_drag(first_overlay.handle, plan.drag_start, plan.drag_end)?;
    thread::sleep(context.settle_delay);
    let selected = capture_evidence(context, "01-selected.png", first_overlay)?;
    record_step(
        report,
        &context.report_path,
        "selection_drag",
        foreground,
        Some(&selected),
    )?;

    let foreground = inject_mouse_click(first_overlay.handle, plan.more)?;
    thread::sleep(context.settle_delay);
    let more = capture_evidence(context, "02-more.png", first_overlay)?;
    ensure_evidence_changed(&selected, &more, "More did not change the overlay")?;
    record_step(
        report,
        &context.report_path,
        "more",
        foreground,
        Some(&more),
    )?;

    let foreground = inject_mouse_click(first_overlay.handle, plan.more)?;
    thread::sleep(context.settle_delay);
    let less = capture_evidence(context, "03-less.png", first_overlay)?;
    ensure_evidence_changed(&more, &less, "Less did not close the expanded actions")?;
    record_step(
        report,
        &context.report_path,
        "less",
        foreground,
        Some(&less),
    )?;

    // Trigger Capture while the first overlay still owns focus. This is the production race the
    // probe exists to cover: the old overlay must close before the replacement accepts input.
    let foreground = inject_capture_shortcut(first_overlay.handle)?;
    record_step(
        report,
        &context.report_path,
        "recapture_shortcut",
        foreground,
        None,
    )?;
    wait_for_window_gone(first_overlay.handle, context.timeout)?;
    let second_overlay = wait_for_overlay(
        controller.handle,
        context.display.physical_bounds,
        context.timeout,
    )?;
    focus_owned_window(second_overlay, context.timeout)?;
    thread::sleep(context.settle_delay);
    let second_scale = (second_overlay.dpi as f32 / WINDOWS_BASE_DPI).max(1.0);
    let second_plan = interaction_plan(second_overlay.bounds, second_scale)?;
    let foreground = inject_mouse_drag(
        second_overlay.handle,
        second_plan.drag_start,
        second_plan.drag_end,
    )?;
    thread::sleep(context.settle_delay);
    let restarted = capture_evidence(context, "04-recapture.png", second_overlay)?;
    record_step(
        report,
        &context.report_path,
        "recapture_overlay_ready",
        foreground,
        Some(&restarted),
    )?;

    let foreground = inject_mouse_click(second_overlay.handle, second_plan.cancel)?;
    wait_for_window_gone(second_overlay.handle, context.timeout)?;
    record_step(
        report,
        &context.report_path,
        "second_cancel",
        foreground,
        None,
    )
}

#[cfg(windows)]
struct Evidence {
    file_name: String,
    fingerprint: u64,
}

#[cfg(windows)]
/// Captures only the verified foreground overlay and returns a lightweight change fingerprint.
fn capture_evidence(
    context: &WorkerContext,
    file_name: &str,
    window: NativeWindow,
) -> io::Result<Evidence> {
    guard_foreground(window.handle)?;
    let frame = SystemCaptureBackend.capture(window.bounds)?;
    let path = context.session_root.join("screenshots").join(file_name);
    frame.save_png(&path)?;
    Ok(Evidence {
        file_name: format!("screenshots/{file_name}"),
        fingerprint: pixel_fingerprint(&frame.pixels),
    })
}

#[cfg(windows)]
fn pixel_fingerprint(pixels: &[u8]) -> u64 {
    pixels.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

#[cfg(windows)]
fn ensure_evidence_changed(left: &Evidence, right: &Evidence, message: &str) -> io::Result<()> {
    if left.fingerprint == right.fingerprint {
        Err(io::Error::other(message))
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn record_step(
    report: &mut AcceptanceReport,
    report_path: &Path,
    action: &'static str,
    foreground: NativeWindow,
    evidence: Option<&Evidence>,
) -> io::Result<()> {
    report.steps.push(StepReport {
        action,
        foreground_window: foreground.handle as usize,
        screenshot: evidence.map(|evidence| evidence.file_name.clone()),
        pixel_fingerprint: evidence.map(|evidence| format!("{:016x}", evidence.fingerprint)),
    });
    write_report(report_path, report)
}

fn write_report(path: &Path, report: &AcceptanceReport) -> io::Result<()> {
    let encoded = serde_json::to_vec_pretty(report).map_err(io::Error::other)?;
    fs::write(path, encoded)
}

#[cfg(windows)]
fn wait_for_controller(timeout: Duration) -> io::Result<NativeWindow> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(window) = process_windows()?.into_iter().max_by_key(window_area) {
            return Ok(window);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "visible acceptance controller did not appear",
            ));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(windows)]
/// Waits for a process-owned window large enough to be the single-display capture overlay.
fn wait_for_overlay(
    controller: *mut c_void,
    display_bounds: PhysicalRect,
    timeout: Duration,
) -> io::Result<NativeWindow> {
    let deadline = Instant::now() + timeout;
    loop {
        let overlay = process_windows()?
            .into_iter()
            .filter(|window| window.handle != controller)
            .filter(|window| overlay_covers_display(*window, display_bounds))
            .max_by_key(window_area);
        if let Some(overlay) = overlay {
            return Ok(overlay);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "capture shortcut did not open a full-display overlay",
            ));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(windows)]
fn overlay_covers_display(window: NativeWindow, display_bounds: PhysicalRect) -> bool {
    let scale = (window.dpi as f32 / WINDOWS_BASE_DPI).max(1.0);
    let observed_width = window.bounds.width() as f32;
    let observed_height = window.bounds.height() as f32;
    let required_width = display_bounds.width() as f32 * 0.75;
    let required_height = display_bounds.height() as f32 * 0.75;
    (observed_width >= required_width || observed_width * scale >= required_width)
        && (observed_height >= required_height || observed_height * scale >= required_height)
}

#[cfg(windows)]
fn wait_for_window_gone(window: *mut c_void, timeout: Duration) -> io::Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        // SAFETY: both functions only query the borrowed HWND value.
        if unsafe { IsWindow(window) } == 0 || unsafe { IsWindowVisible(window) } == 0 {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "capture overlay did not close after Cancel",
            ));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(windows)]
/// Enumerates visible top-level windows owned by this acceptance process only.
fn process_windows() -> io::Result<Vec<NativeWindow>> {
    struct Search {
        process_id: u32,
        windows: Vec<NativeWindow>,
    }

    unsafe extern "system" fn callback(window: *mut c_void, parameter: LPARAM) -> BOOL {
        // SAFETY: EnumWindows passes back the Search pointer supplied for this synchronous call.
        let search = unsafe { &mut *(parameter as *mut Search) };
        let mut process_id = 0;
        unsafe { GetWindowThreadProcessId(window, &mut process_id) };
        if process_id != search.process_id || unsafe { IsWindowVisible(window) } == 0 {
            return 1;
        }
        let mut rect = RECT::default();
        if unsafe { GetWindowRect(window, &mut rect) } != 0
            && rect.right > rect.left
            && rect.bottom > rect.top
        {
            search.windows.push(NativeWindow {
                handle: window,
                bounds: PhysicalRect {
                    left: rect.left,
                    top: rect.top,
                    right: rect.right,
                    bottom: rect.bottom,
                },
                dpi: unsafe { GetDpiForWindow(window) }.max(WINDOWS_BASE_DPI as u32),
            });
        }
        1
    }

    let mut search = Search {
        process_id: unsafe { GetCurrentProcessId() },
        windows: Vec::new(),
    };
    // SAFETY: callback borrows search only for the duration of EnumWindows.
    if unsafe { EnumWindows(Some(callback), &mut search as *mut Search as LPARAM) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(search.windows)
}

#[cfg(windows)]
fn window_area(window: &NativeWindow) -> u64 {
    u64::from(window.bounds.width()) * u64::from(window.bounds.height())
}

#[cfg(windows)]
fn owned_window(handle: *mut c_void) -> io::Result<NativeWindow> {
    // The settings controller is intentionally hidden during capture, so ownership must be
    // checked directly instead of relying on the visible-window enumeration.
    if handle.is_null() || unsafe { IsWindow(handle) } == 0 {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "acceptance window is unavailable",
        ));
    }
    let mut process_id = 0;
    unsafe { GetWindowThreadProcessId(handle, &mut process_id) };
    if process_id != unsafe { GetCurrentProcessId() } {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "acceptance window belongs to another process",
        ));
    }
    let mut rect = RECT::default();
    if unsafe { GetWindowRect(handle, &mut rect) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(NativeWindow {
        handle,
        bounds: PhysicalRect {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        },
        dpi: unsafe { GetDpiForWindow(handle) }.max(WINDOWS_BASE_DPI as u32),
    })
}

#[cfg(windows)]
/// Refuses an input action unless the expected process-owned HWND is still in the foreground.
fn guard_foreground(expected: *mut c_void) -> io::Result<NativeWindow> {
    // SAFETY: GetForegroundWindow returns a borrowed HWND that is only queried below.
    let foreground = unsafe { GetForegroundWindow() };
    if foreground.is_null() {
        return Err(io::Error::other(
            "no foreground window owns the next input action",
        ));
    }
    let mut process_id = 0;
    unsafe { GetWindowThreadProcessId(foreground, &mut process_id) };
    if process_id != unsafe { GetCurrentProcessId() } || foreground != expected {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "foreground window changed; input injection was aborted",
        ));
    }
    owned_window(foreground)
}

#[cfg(windows)]
/// Raises a verified window, then waits until Windows confirms foreground ownership.
fn focus_owned_window(window: NativeWindow, timeout: Duration) -> io::Result<()> {
    owned_window(window.handle)?;
    // SAFETY: both calls borrow a verified process-owned HWND without transferring ownership.
    unsafe {
        BringWindowToTop(window.handle);
        SetForegroundWindow(window.handle);
    }
    let deadline = Instant::now() + timeout.min(Duration::from_secs(3));
    loop {
        if guard_foreground(window.handle).is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "acceptance window could not become foreground; input injection was aborted",
            ));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(windows)]
fn inject_capture_shortcut(expected: *mut c_void) -> io::Result<NativeWindow> {
    let foreground = guard_foreground(expected)?;
    let inputs = [
        keyboard_input(VK_CONTROL, false),
        keyboard_input(VK_MENU, false),
        keyboard_input(VK_F24, false),
        keyboard_input(VK_F24, true),
        keyboard_input(VK_MENU, true),
        keyboard_input(VK_CONTROL, true),
    ];
    send_input_batch(expected, &inputs)?;
    Ok(foreground)
}

#[cfg(windows)]
fn inject_mouse_click(expected: *mut c_void, point: PhysicalPoint) -> io::Result<NativeWindow> {
    let foreground = guard_foreground(expected)?;
    let desktop = virtual_desktop()?;
    send_input_batch(
        expected,
        &[
            absolute_mouse_input(point, MOUSEEVENTF_MOVE, desktop),
            mouse_button_input(MOUSEEVENTF_LEFTDOWN),
            mouse_button_input(MOUSEEVENTF_LEFTUP),
        ],
    )?;
    Ok(foreground)
}

#[cfg(windows)]
/// Sends one left-button drag as a single ordered batch after checking foreground ownership.
fn inject_mouse_drag(
    expected: *mut c_void,
    start: PhysicalPoint,
    end: PhysicalPoint,
) -> io::Result<NativeWindow> {
    let foreground = guard_foreground(expected)?;
    let desktop = virtual_desktop()?;
    let mut inputs = Vec::with_capacity(11);
    inputs.push(absolute_mouse_input(start, MOUSEEVENTF_MOVE, desktop));
    inputs.push(mouse_button_input(MOUSEEVENTF_LEFTDOWN));
    for step in 1..=8 {
        let point = PhysicalPoint {
            x: start.x + (end.x - start.x) * step / 8,
            y: start.y + (end.y - start.y) * step / 8,
        };
        inputs.push(absolute_mouse_input(point, MOUSEEVENTF_MOVE, desktop));
    }
    inputs.push(mouse_button_input(MOUSEEVENTF_LEFTUP));
    send_input_batch(expected, &inputs)?;
    Ok(foreground)
}

#[cfg(windows)]
fn keyboard_input(virtual_key: u16, key_up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: virtual_key,
                dwFlags: if key_up { KEYEVENTF_KEYUP } else { 0 },
                ..Default::default()
            },
        },
    }
}

#[cfg(windows)]
fn mouse_button_input(flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dwFlags: flags,
                ..Default::default()
            },
        },
    }
}

#[cfg(windows)]
fn absolute_mouse_input(point: PhysicalPoint, flags: u32, desktop: PhysicalRect) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: normalize_axis(point.x, desktop.left, desktop.width() as i32),
                dy: normalize_axis(point.y, desktop.top, desktop.height() as i32),
                dwFlags: flags | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                ..Default::default()
            },
        },
    }
}

fn normalize_axis(value: i32, origin: i32, extent: i32) -> i32 {
    if extent <= 1 {
        return 0;
    }
    let offset = value.saturating_sub(origin).clamp(0, extent - 1);
    (i64::from(offset) * 65_535 / i64::from(extent - 1)) as i32
}

#[cfg(windows)]
fn virtual_desktop() -> io::Result<PhysicalRect> {
    // SAFETY: these calls query immutable virtual-desktop metrics.
    let left = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let top = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    if width <= 0 || height <= 0 {
        return Err(io::Error::other("virtual desktop metrics are invalid"));
    }
    Ok(PhysicalRect {
        left,
        top,
        right: left.saturating_add(width),
        bottom: top.saturating_add(height),
    })
}

#[cfg(windows)]
/// Submits one guarded SendInput batch and releases held inputs if Windows accepts it partially.
fn send_input_batch(expected: *mut c_void, inputs: &[INPUT]) -> io::Result<()> {
    guard_foreground(expected)?;
    // SAFETY: every item is initialized according to its INPUT type for this synchronous call.
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    };
    if sent == inputs.len() as u32 {
        Ok(())
    } else {
        // Release buttons and modifiers defensively if Windows accepted only part of a batch.
        let cleanup = [
            mouse_button_input(MOUSEEVENTF_LEFTUP),
            keyboard_input(VK_F24, true),
            keyboard_input(VK_MENU, true),
            keyboard_input(VK_CONTROL, true),
        ];
        // Key-up and button-up cleanup must not depend on focus: leaving an accepted modifier or
        // mouse press held would be a larger global side effect than releasing it after focus moved.
        unsafe {
            SendInput(
                cleanup.len() as u32,
                cleanup.as_ptr(),
                size_of::<INPUT>() as i32,
            )
        };
        Err(io::Error::last_os_error())
    }
}

#[cfg(windows)]
struct CursorRestore {
    original: POINT,
    restored: bool,
}

#[cfg(windows)]
impl CursorRestore {
    fn capture() -> io::Result<Self> {
        let mut original = POINT { x: 0, y: 0 };
        // SAFETY: original is valid writable storage for this synchronous call.
        if unsafe { GetCursorPos(&mut original) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            original,
            restored: false,
        })
    }

    /// Restores eagerly so a successful report also proves the cursor side effect was removed.
    fn restore(mut self) -> io::Result<()> {
        // SAFETY: the coordinates came from GetCursorPos in this process.
        if unsafe { SetCursorPos(self.original.x, self.original.y) } == 0 {
            return Err(io::Error::last_os_error());
        }
        self.restored = true;
        Ok(())
    }
}

#[cfg(windows)]
impl Drop for CursorRestore {
    fn drop(&mut self) {
        if !self.restored {
            // SAFETY: the coordinates came from GetCursorPos; Drop is a best-effort fallback.
            unsafe { SetCursorPos(self.original.x, self.original.y) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_OUTPUT_DIR, Options, ensure_input_authorized, interaction_plan, normalize_axis,
    };
    use flash_shot::domain::geometry::{PhysicalPoint, PhysicalRect};
    use std::{ffi::OsString, path::PathBuf, time::Duration};

    fn arguments(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn input_injection_requires_the_explicit_opt_in_flag() {
        let options = Options::parse_from(arguments(&[])).unwrap();
        assert_eq!(options.output_dir, PathBuf::from(DEFAULT_OUTPUT_DIR));
        assert_eq!(
            ensure_input_authorized(&options).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );

        let allowed = Options::parse_from(arguments(&["--allow-input"])).unwrap();
        assert!(ensure_input_authorized(&allowed).is_ok());
    }

    #[test]
    fn parser_accepts_bounded_delays_and_rejects_duplicate_authorization() {
        let options = Options::parse_from(arguments(&[
            "--allow-input",
            "--output-dir",
            "evidence",
            "--timeout-ms",
            "12000",
            "--settle-ms",
            "350",
        ]))
        .unwrap();
        assert_eq!(options.output_dir, PathBuf::from("evidence"));
        assert_eq!(options.timeout, Duration::from_secs(12));
        assert_eq!(options.settle_delay, Duration::from_millis(350));
        assert!(Options::parse_from(arguments(&["--allow-input", "--allow-input"])).is_err());
        assert!(Options::parse_from(arguments(&["--timeout-ms", "2999"])).is_err());
        assert!(Options::parse_from(arguments(&["--settle-ms", "5001"])).is_err());
    }

    #[test]
    fn interaction_plan_keeps_drag_and_toolbar_clicks_inside_scaled_overlay() {
        let bounds = PhysicalRect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        let plan = interaction_plan(bounds, 1.5).unwrap();
        for point in [plan.drag_start, plan.drag_end, plan.more, plan.cancel] {
            assert!(bounds.contains(point), "{point:?}");
        }
        assert!(plan.drag_start.x < plan.drag_end.x);
        assert!(plan.drag_start.y < plan.drag_end.y);
        assert!(plan.more.x < plan.cancel.x);
        assert!(plan.more.y > plan.drag_end.y);
    }

    #[test]
    fn interaction_plan_rejects_unsafe_geometry_and_scale() {
        let small = PhysicalRect {
            left: 0,
            top: 0,
            right: 640,
            bottom: 480,
        };
        assert!(interaction_plan(small, 1.0).is_err());
        assert!(interaction_plan(small, 0.0).is_err());
    }

    #[test]
    fn absolute_input_normalization_clamps_virtual_desktop_edges() {
        assert_eq!(normalize_axis(-1200, -1000, 2000), 0);
        assert_eq!(normalize_axis(999, -1000, 2000), 65_535);
        assert_eq!(normalize_axis(5_000, -1000, 2000), 65_535);
        assert_eq!(normalize_axis(10, 10, 1), 0);
        let midpoint = normalize_axis(0, -1000, 2000);
        assert!((32_750..=32_800).contains(&midpoint));
    }

    #[test]
    fn plan_selection_matches_the_expected_physical_drag() {
        let bounds = PhysicalRect {
            left: -1920,
            top: 0,
            right: 0,
            bottom: 1080,
        };
        let plan = interaction_plan(bounds, 1.0).unwrap();
        assert_eq!(
            PhysicalRect::new(plan.drag_start, plan.drag_end),
            PhysicalRect {
                left: -1498,
                top: 216,
                right: -614,
                bottom: 540,
            }
        );
        assert_ne!(plan.more, PhysicalPoint::default());
    }
}
