//! Explicit, isolated Windows input probe for the real capture-overlay workflow.

use std::{
    any::Any,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process, thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use flash_shot::{
    OverlayInteractionAcceptanceCommand, OverlayInteractionAcceptanceOptions,
    OverlayInteractionCaptureContent, OverlayInteractionCaptureState,
    OverlayInteractionRecordingState,
    domain::geometry::{PhysicalPoint, PhysicalRect},
    history::ScreenshotHistory,
    performance::PerformanceRecorder,
    platform::display::{DisplayInfo, DisplayProvider, SystemDisplayProvider},
    settings::UserSettings,
};
#[cfg(windows)]
use flash_shot::{
    platform::{
        process_group::ProcessGroup,
        window_inspector::{SystemWindowInspector, WindowInspector},
    },
    recording::discover,
};

#[path = "support/recording_probe.rs"]
mod recording_probe;

use recording_probe::MediaMetadata;
#[cfg(windows)]
use recording_probe::{extract_video_frame, extract_video_frame_series, probe_media};

#[cfg(windows)]
use flash_shot::platform::capture::{CaptureBackend, CaptureFrame, SystemCaptureBackend};
#[cfg(windows)]
use std::{
    ffi::c_void,
    mem::size_of,
    panic::AssertUnwindSafe,
    ptr,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError},
    },
};
#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::{
        BeginPaint, ClientToScreen, CreateSolidBrush, DeleteObject, EndPaint, FillRect,
        PAINTSTRUCT, UpdateWindow,
    },
    System::{
        DataExchange::GetClipboardSequenceNumber, LibraryLoader::GetModuleHandleW,
        Threading::GetCurrentProcessId,
    },
    UI::{
        HiDpi::{
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow,
            SetProcessDpiAwarenessContext,
        },
        Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP,
            KEYEVENTF_UNICODE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
            MOUSEEVENTF_MOVE, MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT, SendInput, VK_A, VK_CONTROL,
            VK_ESCAPE, VK_F24, VK_MENU, VK_RETURN, VK_RIGHT,
        },
        WindowsAndMessaging::{
            BringWindowToTop, CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW,
            DestroyWindow, DispatchMessageW, EnumChildWindows, EnumWindows, FindWindowW,
            GUITHREADINFO, GW_OWNER, GWLP_USERDATA, GetClassNameW, GetClientRect, GetCursorPos,
            GetForegroundWindow, GetGUIThreadInfo, GetMessageW, GetSystemMetrics, GetWindow,
            GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
            GetWindowThreadProcessId, HWND_TOP, IsChild, IsIconic, IsWindow, IsWindowVisible, MSG,
            PostMessageW, PostQuitMessage, RegisterClassW, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
            SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_HIDE, SW_MINIMIZE, SW_RESTORE, SWP_NOACTIVATE,
            SWP_SHOWWINDOW, SetCursorPos, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos,
            ShowWindow, TranslateMessage, WM_CLOSE, WM_DESTROY, WM_ERASEBKGND, WM_PAINT, WNDCLASSW,
            WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP, WS_VISIBLE, WindowFromPoint,
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
const SELECTION_EDGE_TOLERANCE: i32 = 1;
const PAUSE_STABILITY_INTERVAL: Duration = Duration::from_millis(300);
const MAX_RECORDING_GRID_MAE: f64 = 18.0;
const WINDOW_TARGET_CHILD_MODE: &str = "--window-target-child";
const WINDOW_FIXTURE_CLASS: &str = "FlashShotRecordingWindowFixture";
const WINDOW_PHASE_SETTLE_US: u64 = 300_000;
const WINDOW_PHASE_HOLD_US: u64 = 300_000;
const WINDOW_TIMELINE_FRAMES_PER_SECOND: u16 = 5;
const WINDOW_TIMELINE_STABLE_FRAMES: usize = 2;
const MIN_WINDOW_FIXTURE_WIDTH: i64 = 240;
const MIN_WINDOW_FIXTURE_HEIGHT: i64 = 120;
#[cfg(windows)]
const PROFILE_DIRECTORY_ENV: &str = "FLASH_SHOT_PROFILE_DIR";
#[cfg(windows)]
const RECORDING_DIRECTORY_ENV: &str = "FLASH_SHOT_RECORDING_DIRECTORY";
#[cfg(windows)]
const RECORDING_MICROPHONE_ENV: &str = "FLASH_SHOT_RECORDING_MICROPHONE";
#[cfg(windows)]
const RECORDING_SYSTEM_AUDIO_ENV: &str = "FLASH_SHOT_RECORDING_SYSTEM_AUDIO";

#[cfg(windows)]
static LIVE_FIXTURE_WINDOWS: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Eq, PartialEq)]
struct Options {
    allow_input: bool,
    output_dir: PathBuf,
    timeout: Duration,
    settle_delay: Duration,
    record_target: Option<RecordTargetOption>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecordTargetOption {
    Area,
    Window,
}

impl RecordTargetOption {
    const fn label(self) -> &'static str {
        match self {
            Self::Area => "selected area",
            Self::Window => "window",
        }
    }

    const fn workflow(self) -> &'static str {
        match self {
            Self::Area => "record_area",
            Self::Window => "record_window",
        }
    }
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
            record_target: None,
        };
        let mut arguments = arguments.into_iter();
        let mut output_seen = false;
        let mut timeout_seen = false;
        let mut settle_seen = false;
        let mut record_target_seen = false;
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
                "--record-target" if !record_target_seen => {
                    let target = arguments
                        .next()
                        .ok_or_else(usage)?
                        .into_string()
                        .map_err(|_| "record target must be valid Unicode".to_owned())?;
                    options.record_target = Some(match target.as_str() {
                        "area" => RecordTargetOption::Area,
                        "window" => RecordTargetOption::Window,
                        _ => return Err("record target must be 'area' or 'window'".to_owned()),
                    });
                    record_target_seen = true;
                }
                "--output-dir" | "--timeout-ms" | "--settle-ms" | "--record-target" => {
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
    "usage: overlay-interaction-acceptance --allow-input [--record-target <area|window>] [--output-dir <path>] [--timeout-ms <3000-60000>] [--settle-ms <100-5000>]".to_owned()
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

/// Creates the process-local command bridge without a capacity wait that could defeat the runner
/// deadline. Only the single acceptance worker owns a sender, so the overall timeout also bounds
/// the maximum number of queued snapshots.
fn interaction_command_channel() -> (
    async_channel::Sender<OverlayInteractionAcceptanceCommand>,
    async_channel::Receiver<OverlayInteractionAcceptanceCommand>,
) {
    async_channel::unbounded()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct InteractionPlan {
    drag_start: PhysicalPoint,
    drag_end: PhysicalPoint,
    pin: PhysicalPoint,
    copy: PhysicalPoint,
    save: PhysicalPoint,
    more: PhysicalPoint,
    cancel: PhysicalPoint,
    record_area: PhysicalPoint,
    record_window: PhysicalPoint,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RecordingControlPlan {
    stop: PhysicalPoint,
    pause_or_resume: PhysicalPoint,
}

/// Converts stable logical Record-page controls through the measured client-area DPI.
fn recording_control_plan(
    client_origin: PhysicalPoint,
    client_width: u32,
    client_height: u32,
    scale: f32,
) -> io::Result<RecordingControlPlan> {
    if !scale.is_finite() || !(1.0..=4.0).contains(&scale) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "recording controller scale must be between 1.0 and 4.0",
        ));
    }
    let logical_width = client_width as f32 / scale;
    let logical_height = client_height as f32 / scale;
    if logical_width < 640.0 || logical_height < 500.0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "recording controller did not retain the wide Record-page layout",
        ));
    }
    let point = |x: f32, y: f32| PhysicalPoint {
        x: client_origin.x + (x * scale).round() as i32,
        y: client_origin.y + (y * scale).round() as i32,
    };
    Ok(RecordingControlPlan {
        stop: point(386.0, 426.0),
        // Pause changes to the wider Resume label in place; this point stays inside both buttons.
        pause_or_resume: point(493.0, 426.0),
    })
}

/// Converts the overlay client area's logical geometry into physical screen points for SendInput.
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
        pin: screen_point((toolbar_left + 87.0, toolbar_top + 25.0)),
        copy: screen_point((toolbar_left + 139.0, toolbar_top + 25.0)),
        save: screen_point((toolbar_left + 193.0, toolbar_top + 25.0)),
        more: screen_point((toolbar_left + 247.0, toolbar_top + 25.0)),
        cancel: screen_point((toolbar_left + 312.0, toolbar_top + 25.0)),
        // The expanded 342 px menu wraps into five right-aligned rows. Recording occupies the
        // final item of row four and the sole item of row five above this toolbar.
        record_area: screen_point((toolbar_left + toolbar_width - 31.0, toolbar_top - 75.0)),
        record_window: screen_point((toolbar_left + toolbar_width - 31.0, toolbar_top - 33.0)),
    })
}

/// Verifies that the application's committed selection follows the actual injected pointer path.
fn validate_selection_geometry(
    requested: PhysicalRect,
    committed: PhysicalRect,
    label: &str,
) -> io::Result<()> {
    let edge_delta = |left: i32, right: i32| (i64::from(left) - i64::from(right)).abs();
    let tolerance = i64::from(SELECTION_EDGE_TOLERANCE);
    let matches = edge_delta(requested.left, committed.left) <= tolerance
        && edge_delta(requested.top, committed.top) <= tolerance
        && edge_delta(requested.right, committed.right) <= tolerance
        && edge_delta(requested.bottom, committed.bottom) <= tolerance;
    if matches {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{label} committed {committed:?}, requested {requested:?} (maximum edge tolerance: {SELECTION_EDGE_TOLERANCE}px)"
            ),
        ))
    }
}

/// Translates the actual screen pointer path into the full-display pixels shown by the overlay.
fn map_screen_selection_to_capture(
    screen_selection: PhysicalRect,
    client_bounds: PhysicalRect,
    capture_bounds: PhysicalRect,
) -> io::Result<PhysicalRect> {
    if client_bounds.width() != capture_bounds.width()
        || client_bounds.height() != capture_bounds.height()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "overlay client {:?} does not match capture display {:?}",
                client_bounds, capture_bounds
            ),
        ));
    }
    let delta_x = capture_bounds
        .left
        .checked_sub(client_bounds.left)
        .ok_or_else(|| io::Error::other("overlay client-to-display X offset overflowed"))?;
    let delta_y = capture_bounds
        .top
        .checked_sub(client_bounds.top)
        .ok_or_else(|| io::Error::other("overlay client-to-display Y offset overflowed"))?;
    translated_rect(screen_selection, delta_x, delta_y)
}

/// Requires the production recorder to expose the independently resolved source rectangle.
fn validate_recording_target_bounds(
    target: RecordTargetOption,
    expected: PhysicalRect,
    reported: PhysicalRect,
) -> io::Result<()> {
    if reported == expected {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{} recording reported source {reported:?}, independently expected {expected:?}",
                target.label()
            ),
        ))
    }
}

fn validate_paused_progress(
    before: &OverlayInteractionRecordingState,
    after: &OverlayInteractionRecordingState,
) -> io::Result<()> {
    if !before.active || !before.paused || !after.active || !after.paused {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recording left the active paused state during the stability interval",
        ));
    }
    if before.progress_frame == after.progress_frame
        && before.progress_time_us == after.progress_time_us
    {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "paused recording advanced from frame {} / {}us to frame {} / {}us",
                before.progress_frame,
                before.progress_time_us,
                after.progress_frame,
                after.progress_time_us
            ),
        ))
    }
}

#[derive(serde::Serialize)]
struct AcceptanceReport {
    schema_version: u32,
    test: &'static str,
    workflow: &'static str,
    status: String,
    process_id: u32,
    shortcut: &'static str,
    shortcut_registered: bool,
    isolated_profile: String,
    display: DisplayReport,
    controller_window: Option<WindowReport>,
    steps: Vec<StepReport>,
    recording_states: Vec<RecordingStateReport>,
    recording: Option<RecordingReport>,
    capture_actions: Option<CaptureActionReport>,
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

#[derive(serde::Serialize)]
struct RecordingStateReport {
    stage: &'static str,
    active: bool,
    starting: bool,
    stopping: bool,
    paused: bool,
    target: Option<String>,
    target_bounds: Option<PhysicalRect>,
    progress_frame: u64,
    progress_time_us: u64,
    status: String,
}

impl RecordingStateReport {
    fn from_state(stage: &'static str, state: OverlayInteractionRecordingState) -> Self {
        Self {
            stage,
            active: state.active,
            starting: state.starting,
            stopping: state.stopping,
            paused: state.paused,
            target: state.target,
            target_bounds: state.target_bounds,
            progress_frame: state.progress_frame,
            progress_time_us: state.progress_time_us,
            status: state.status,
        }
    }
}

#[derive(serde::Serialize)]
struct RecordingReport {
    target: &'static str,
    requested_selection: PhysicalRect,
    source_bounds: PhysicalRect,
    reported_source_bounds: PhysicalRect,
    window_title: Option<String>,
    output: String,
    output_bytes: u64,
    ffmpeg_version: String,
    codec_name: String,
    width: u32,
    height: u32,
    duration_seconds: f64,
    pause_observed: bool,
    resume_observed: bool,
    maximum_progress_frame: u64,
    content: RecordingContentReport,
    window_dynamics: Option<RecordingWindowDynamicsReport>,
}

#[derive(serde::Serialize)]
struct RecordingContentReport {
    reference: String,
    decoded_frame: String,
    timestamp_seconds: f64,
    reference_fingerprint: String,
    decoded_fingerprint: String,
    reference_luma_min: u8,
    reference_luma_max: u8,
    decoded_luma_min: u8,
    decoded_luma_max: u8,
    grid_mean_absolute_error: f64,
    maximum_allowed_error: f64,
}

#[derive(serde::Serialize)]
struct RecordingWindowDynamicsReport {
    fixture_process_id: u32,
    target_title: String,
    initial_target_bounds: PhysicalRect,
    moved_target_bounds: PhysicalRect,
    resized_target_bounds: PhysicalRect,
    source_bounds_fixed: bool,
    fixture_cleaned_up: bool,
    timeline_frames_per_second: u16,
    timeline_sample_count: usize,
    phases: Vec<RecordingWindowPhaseReport>,
}

#[derive(serde::Serialize)]
struct RecordingWindowPhaseReport {
    stage: &'static str,
    progress_timestamp_seconds: f64,
    target_bounds: PhysicalRect,
    target_visible: bool,
    target_minimized: bool,
    backdrop_visible: bool,
    occluder_visible: bool,
    reported_source_bounds: PhysicalRect,
    content: RecordingContentReport,
}

#[derive(serde::Serialize)]
struct CaptureActionReport {
    initial_requested_selection: PhysicalRect,
    recapture_requested_selection: PhysicalRect,
    nudge: NudgeReport,
    save: SaveReport,
    pin: PinReport,
    copy: CopyReport,
    cleanup: CleanupReport,
}

#[derive(serde::Serialize)]
struct NudgeReport {
    before: PhysicalRect,
    after: PhysicalRect,
    delta_x: i32,
    delta_y: i32,
}

#[derive(serde::Serialize)]
struct SaveReport {
    requested_selection: PhysicalRect,
    selection: PhysicalRect,
    path: String,
    width: u32,
    height: u32,
    bytes: u64,
    content: ExactPixelMatchReport,
}

#[derive(serde::Serialize)]
struct PinReport {
    requested_selection: PhysicalRect,
    selection: PhysicalRect,
    source_bounds: PhysicalRect,
    window: WindowReport,
    content: ExactPixelMatchReport,
}

#[derive(serde::Serialize)]
struct CopyReport {
    requested_selection: PhysicalRect,
    selection: PhysicalRect,
    copied_bounds: PhysicalRect,
    width: u32,
    height: u32,
    clipboard_sequence_before: u32,
    clipboard_sequence_after: u32,
    clipboard_unchanged: bool,
    content: ExactPixelMatchReport,
}

#[derive(serde::Serialize)]
struct ExactPixelMatchReport {
    source_fingerprint: String,
    result_fingerprint: String,
    source_luma_min: u8,
    source_luma_max: u8,
    exact_match: bool,
}

#[derive(serde::Serialize)]
struct CleanupReport {
    session_state: String,
    overlay_count: usize,
    pinned_count: usize,
    visible_process_windows: usize,
    capture_preflight_ready: bool,
}

#[cfg(windows)]
#[derive(Clone, Copy)]
struct NativeWindow {
    handle: *mut c_void,
    bounds: PhysicalRect,
    dpi: u32,
}

#[cfg(windows)]
#[derive(Clone, Copy)]
struct InjectedDrag {
    foreground: NativeWindow,
    selection: PhysicalRect,
}

#[cfg(windows)]
struct RecordingWindowFixture {
    child: process::Child,
    process_group: ProcessGroup,
    process_id: u32,
    target_title: String,
    target: HWND,
    backdrop: HWND,
    occluder: HWND,
    initial_bounds: PhysicalRect,
    moved_bounds: PhysicalRect,
    resized_bounds: PhysicalRect,
    stopped: bool,
}

#[cfg(windows)]
#[derive(Clone, Copy)]
struct FixturePhaseState {
    target_bounds: PhysicalRect,
    target_visible: bool,
    target_minimized: bool,
    backdrop_visible: bool,
    occluder_visible: bool,
}

#[cfg(windows)]
struct PendingRecordingWindowPhase {
    stage: &'static str,
    reference_file: String,
    decoded_file: String,
    timestamp_seconds: f64,
    reference: CaptureFrame,
    fixture: FixturePhaseState,
    reported_source_bounds: PhysicalRect,
    maximum_progress_frame: u64,
}

#[cfg(windows)]
struct RecordingTimelineCandidate {
    timestamp_seconds: f64,
    path: PathBuf,
}

#[cfg(windows)]
struct StableRecordingFrameMatch {
    candidate_index: usize,
    comparison: RecordingFrameContentComparison,
}

#[cfg(windows)]
struct RecordingWindowFixtureReportSeed {
    process_id: u32,
    target_title: String,
    initial_bounds: PhysicalRect,
    moved_bounds: PhysicalRect,
    resized_bounds: PhysicalRect,
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
    interaction_commands: async_channel::Sender<OverlayInteractionAcceptanceCommand>,
    copy_results: Receiver<CaptureFrame>,
    timeout: Duration,
    settle_delay: Duration,
    record_target: Option<RecordTargetOption>,
}

fn main() {
    #[cfg(windows)]
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new(WINDOW_TARGET_CHILD_MODE))
    {
        if let Err(error) = run_recording_window_fixture_child(std::env::args_os().skip(2)) {
            eprintln!("recording window fixture failed: {error}");
            process::exit(1);
        }
        return;
    }
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

    isolate_process_environment(&session_root);
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
    let (interaction_tx, interaction_rx) = interaction_command_channel();
    let (copy_result_tx, copy_result_rx) = mpsc::channel();
    let (window_width, window_height) = if options.record_target.is_some() {
        (980.0, 760.0)
    } else {
        (520.0, 640.0)
    };

    let worker_context = WorkerContext {
        session_root,
        report_path,
        display,
        shortcut_readiness: shortcut_ready_rx,
        interaction_commands: interaction_tx,
        copy_results: copy_result_rx,
        timeout: options.timeout,
        settle_delay: options.settle_delay,
        record_target: options.record_target,
    };
    let mut report = initial_report(&worker_context);
    write_report(&worker_context.report_path, &report)?;
    thread::Builder::new()
        .name("overlay-interaction-acceptance".to_owned())
        .spawn(move || {
            let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
                run_interaction_sequence(&worker_context, &mut report)
            }));
            let exit_code = match result {
                Ok(Ok(())) => 0,
                Ok(Err(error)) => {
                    set_failed_report(&mut report, error.to_string());
                    if let Err(report_error) = write_report(&worker_context.report_path, &report) {
                        eprintln!("could not persist failed acceptance report: {report_error}");
                    }
                    eprintln!("overlay interaction worker failed: {error}");
                    1
                }
                Err(payload) => {
                    let message = format!(
                        "overlay interaction worker panicked: {}",
                        panic_payload_message(payload.as_ref())
                    );
                    set_failed_report(&mut report, message.clone());
                    if let Err(report_error) = write_report(&worker_context.report_path, &report) {
                        eprintln!("could not persist panicked acceptance report: {report_error}");
                    }
                    eprintln!("{message}");
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
        OverlayInteractionAcceptanceOptions {
            window_width,
            window_height,
            shortcut_readiness: shortcut_ready_tx,
            commands: interaction_rx,
            copy_results: copy_result_tx,
        },
    )?;
    Err(io::Error::other("GPUI exited before the interaction worker completed").into())
}

#[cfg(windows)]
/// Creates the persisted report before the worker can inject input or panic.
fn initial_report(context: &WorkerContext) -> AcceptanceReport {
    AcceptanceReport {
        schema_version: 5,
        test: "overlay_interaction_acceptance",
        workflow: context
            .record_target
            .map_or("capture", RecordTargetOption::workflow),
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
        recording_states: Vec::new(),
        recording: None,
        capture_actions: None,
        error: None,
    }
}

#[cfg(windows)]
fn set_failed_report(report: &mut AcceptanceReport, message: impl Into<String>) {
    report.status = "failed".to_owned();
    report.error = Some(message.into());
}

#[cfg(windows)]
fn panic_payload_message(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string panic payload")
}

#[cfg(windows)]
/// Pins all writable recording state to this session and disables inherited audio capture.
fn isolate_process_environment(session_root: &Path) {
    let recording_directory = session_root.join("recordings");
    // SAFETY: this runs before GPUI and the acceptance worker start, so no other thread can read
    // a partially updated process environment. The process exits after this one isolated run.
    unsafe {
        std::env::set_var(PROFILE_DIRECTORY_ENV, session_root);
        std::env::set_var(RECORDING_DIRECTORY_ENV, recording_directory);
        std::env::remove_var(RECORDING_MICROPHONE_ENV);
        std::env::remove_var(RECORDING_SYSTEM_AUDIO_ENV);
    }
}

#[cfg(windows)]
/// Runs the private child mode that paints deterministic windows for recording verification.
fn run_recording_window_fixture_child(
    arguments: impl IntoIterator<Item = OsString>,
) -> io::Result<()> {
    let (token, bounds) = parse_recording_window_fixture_arguments(arguments)?;
    // A manifest may have set this already; AccessDenied means the process is already locked in.
    if unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) } == 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(5) {
            return Err(error);
        }
    }
    LIVE_FIXTURE_WINDOWS.store(0, Ordering::Release);
    let class_name = wide_null(WINDOW_FIXTURE_CLASS);
    // SAFETY: a null module name requests the executable module for this child process.
    let instance = unsafe { GetModuleHandleW(ptr::null()) };
    if instance.is_null() {
        return Err(io::Error::last_os_error());
    }
    let window_class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(recording_fixture_window_proc),
        hInstance: instance,
        lpszClassName: class_name.as_ptr(),
        ..Default::default()
    };
    // SAFETY: the class strings and callback remain valid for this process lifetime.
    if unsafe { RegisterClassW(&window_class) } == 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(1410) {
            return Err(error);
        }
    }

    let mut windows = Vec::with_capacity(3);
    let create_result = (|| {
        let backdrop = create_recording_fixture_window(
            instance,
            &class_name,
            &recording_fixture_title(&token, "Backdrop"),
            bounds,
            2,
            true,
        )?;
        windows.push(backdrop);
        let target = create_recording_fixture_window(
            instance,
            &class_name,
            &recording_fixture_title(&token, "Target"),
            bounds,
            1,
            true,
        )?;
        windows.push(target);
        let occluder = create_recording_fixture_window(
            instance,
            &class_name,
            &recording_fixture_title(&token, "Occluder"),
            bounds,
            3,
            false,
        )?;
        windows.push(occluder);
        set_fixture_window_bounds(backdrop, bounds, true)?;
        set_fixture_window_bounds(target, bounds, true)?;
        Ok::<(), io::Error>(())
    })();
    if let Err(error) = create_result {
        destroy_recording_fixture_windows(&windows);
        return Err(error);
    }

    let mut message = MSG::default();
    loop {
        // SAFETY: message is writable and this thread owns every fixture window.
        let result = unsafe { GetMessageW(&mut message, ptr::null_mut(), 0, 0) };
        if result == -1 {
            let error = io::Error::last_os_error();
            destroy_recording_fixture_windows(&windows);
            return Err(error);
        }
        if result == 0 {
            break;
        }
        // SAFETY: GetMessageW initialized this message for the current GUI thread.
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    destroy_recording_fixture_windows(&windows);
    Ok(())
}

#[cfg(windows)]
fn parse_recording_window_fixture_arguments(
    arguments: impl IntoIterator<Item = OsString>,
) -> io::Result<(String, PhysicalRect)> {
    let arguments = arguments.into_iter().collect::<Vec<_>>();
    if arguments.len() != 5 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "window fixture child requires TOKEN LEFT TOP RIGHT BOTTOM",
        ));
    }
    let token = arguments[0]
        .to_str()
        .filter(|value| !value.is_empty() && !value.contains('\0'))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid fixture token"))?
        .to_owned();
    let coordinate = |index: usize, label: &str| {
        arguments[index]
            .to_str()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid coordinate"))?
            .parse::<i32>()
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("fixture {label} must be a 32-bit integer"),
                )
            })
    };
    let bounds = PhysicalRect {
        left: coordinate(1, "left")?,
        top: coordinate(2, "top")?,
        right: coordinate(3, "right")?,
        bottom: coordinate(4, "bottom")?,
    };
    validate_recording_fixture_bounds(bounds)?;
    Ok((token, bounds))
}

#[cfg(windows)]
fn validate_recording_fixture_bounds(bounds: PhysicalRect) -> io::Result<()> {
    let width = i64::from(bounds.right) - i64::from(bounds.left);
    let height = i64::from(bounds.bottom) - i64::from(bounds.top);
    if width < MIN_WINDOW_FIXTURE_WIDTH || height < MIN_WINDOW_FIXTURE_HEIGHT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "window fixture must be at least {MIN_WINDOW_FIXTURE_WIDTH}x{MIN_WINDOW_FIXTURE_HEIGHT} physical pixels"
            ),
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn recording_fixture_title(token: &str, role: &str) -> String {
    format!("Flash Shot Recording Fixture {token} {role}")
}

#[cfg(windows)]
fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn create_recording_fixture_window(
    instance: *mut c_void,
    class_name: &[u16],
    title: &str,
    bounds: PhysicalRect,
    pattern: isize,
    visible: bool,
) -> io::Result<HWND> {
    let title = wide_null(title);
    let style = WS_POPUP | if visible { WS_VISIBLE } else { 0 };
    // SAFETY: all string pointers are NUL-terminated and valid for the duration of this call.
    let window = unsafe {
        CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            class_name.as_ptr(),
            title.as_ptr(),
            style,
            bounds.left,
            bounds.top,
            bounds.width() as i32,
            bounds.height() as i32,
            ptr::null_mut(),
            ptr::null_mut(),
            instance,
            ptr::null(),
        )
    };
    if window.is_null() {
        return Err(io::Error::last_os_error());
    }
    LIVE_FIXTURE_WINDOWS.fetch_add(1, Ordering::AcqRel);
    // SAFETY: this child owns the window; the small role value is read only by its window proc.
    unsafe {
        SetWindowLongPtrW(window, GWLP_USERDATA, pattern);
        UpdateWindow(window);
    }
    Ok(window)
}

#[cfg(windows)]
unsafe extern "system" fn recording_fixture_window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_PAINT => {
            paint_recording_fixture_window(window);
            0
        }
        WM_ERASEBKGND => 1,
        WM_CLOSE => {
            // SAFETY: this callback runs on the GUI thread that created the fixture window.
            unsafe { DestroyWindow(window) };
            0
        }
        WM_DESTROY => {
            if LIVE_FIXTURE_WINDOWS.fetch_sub(1, Ordering::AcqRel) == 1 {
                // SAFETY: ending the child message loop after its final window closes is intended.
                unsafe { PostQuitMessage(0) };
            }
            0
        }
        _ => {
            // SAFETY: unhandled messages use the standard top-level window behavior.
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
    }
}

#[cfg(windows)]
fn paint_recording_fixture_window(window: HWND) {
    let mut paint = PAINTSTRUCT::default();
    // SAFETY: paint is writable and this call is paired with EndPaint below.
    let device = unsafe { BeginPaint(window, &mut paint) };
    if device.is_null() {
        return;
    }
    let mut client = RECT::default();
    // SAFETY: both calls borrow the live window and initialized paint data.
    if unsafe { GetClientRect(window, &mut client) } != 0 {
        // SAFETY: the role was installed immediately after CreateWindowExW returned.
        let pattern = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as usize;
        for row in 0..4 {
            for column in 0..4 {
                let cell = RECT {
                    left: client.left + (client.right - client.left) * column / 4,
                    top: client.top + (client.bottom - client.top) * row / 4,
                    right: client.left + (client.right - client.left) * (column + 1) / 4,
                    bottom: client.top + (client.bottom - client.top) * (row + 1) / 4,
                };
                // SAFETY: the brush is deleted after FillRect and the rectangle is initialized.
                let brush = unsafe {
                    CreateSolidBrush(recording_fixture_color(
                        pattern,
                        row as usize,
                        column as usize,
                    ))
                };
                if !brush.is_null() {
                    unsafe {
                        FillRect(device, &cell, brush);
                        DeleteObject(brush);
                    }
                }
            }
        }
    }
    // SAFETY: BeginPaint succeeded and paint remains initialized.
    unsafe { EndPaint(window, &paint) };
}

#[cfg(windows)]
fn recording_fixture_color(pattern: usize, row: usize, column: usize) -> u32 {
    const TARGET: [u32; 4] = [0x002040f0, 0x00e0c020, 0x0030c060, 0x00f0f0f0];
    const BACKDROP: [u32; 4] = [0x00181818, 0x00d040b0, 0x00e0a030, 0x0040d8e8];
    const OCCLUDER: [u32; 4] = [0x000020f0, 0x00080808, 0x00f0e040, 0x00e8e8e8];
    let palette = match pattern {
        2 => BACKDROP,
        3 => OCCLUDER,
        _ => TARGET,
    };
    palette[(row * 3 + column) % palette.len()]
}

#[cfg(windows)]
fn destroy_recording_fixture_windows(windows: &[HWND]) {
    for window in windows.iter().rev().copied() {
        // SAFETY: this helper is called only on the creating GUI thread.
        if !window.is_null() && unsafe { IsWindow(window) } != 0 {
            unsafe { DestroyWindow(window) };
        }
    }
}

#[cfg(windows)]
/// Derives one pure move and one pure resize from the committed recording selection.
fn recording_fixture_dynamic_bounds(
    initial_bounds: PhysicalRect,
) -> io::Result<(PhysicalRect, PhysicalRect)> {
    validate_recording_fixture_bounds(initial_bounds)?;
    let moved = translated_rect(
        initial_bounds,
        (initial_bounds.width() as i32 / 10).max(32),
        (initial_bounds.height() as i32 / 10).max(24),
    )?;
    let resized = PhysicalRect {
        left: initial_bounds.left,
        top: initial_bounds.top,
        right: initial_bounds.left + (initial_bounds.width() * 3 / 4) as i32,
        bottom: initial_bounds.top + (initial_bounds.height() * 3 / 4) as i32,
    };
    validate_recording_fixture_bounds(resized)?;
    Ok((moved, resized))
}

#[cfg(windows)]
/// Rejects a helper transition unless visibility, minimization, and bounds match its contract.
fn validate_fixture_phase_state(
    stage: &str,
    state: FixturePhaseState,
    expected_bounds: Option<PhysicalRect>,
    expected_minimized: bool,
    expected_occluder: bool,
) -> io::Result<()> {
    if !state.target_visible
        || !state.backdrop_visible
        || state.target_minimized != expected_minimized
        || state.occluder_visible != expected_occluder
        || expected_bounds.is_some_and(|bounds| state.target_bounds != bounds)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "fixture {stage} state is bounds={:?}, visible={}, minimized={}, backdrop={}, occluder={}",
                state.target_bounds,
                state.target_visible,
                state.target_minimized,
                state.backdrop_visible,
                state.occluder_visible
            ),
        ));
    }
    Ok(())
}

#[cfg(windows)]
impl RecordingWindowFixture {
    /// Starts the same executable in child mode and verifies all three cross-process HWNDs.
    fn launch(initial_bounds: PhysicalRect, timeout: Duration) -> io::Result<Self> {
        validate_recording_fixture_bounds(initial_bounds)?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let token = format!("{}-{timestamp}", process::id());
        let target_title = recording_fixture_title(&token, "Target");
        let backdrop_title = recording_fixture_title(&token, "Backdrop");
        let occluder_title = recording_fixture_title(&token, "Occluder");
        let process_group = ProcessGroup::create()?;
        let mut child = process::Command::new(std::env::current_exe()?)
            .arg(WINDOW_TARGET_CHILD_MODE)
            .arg(&token)
            .arg(initial_bounds.left.to_string())
            .arg(initial_bounds.top.to_string())
            .arg(initial_bounds.right.to_string())
            .arg(initial_bounds.bottom.to_string())
            .stdin(process::Stdio::null())
            .stdout(process::Stdio::null())
            .stderr(process::Stdio::inherit())
            .spawn()?;
        if let Err(error) = process_group.assign(&child) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        let process_id = child.id();
        let handles = match wait_for_recording_fixture_windows(
            &mut child,
            process_id,
            [&target_title, &backdrop_title, &occluder_title],
            timeout,
        ) {
            Ok(handles) => handles,
            Err(error) => {
                let _ = process_group.terminate();
                let _ = child.wait();
                return Err(error);
            }
        };
        let target = handles[0];
        let backdrop = handles[1];
        let occluder = handles[2];
        for (label, window) in [
            ("target", target),
            ("backdrop", backdrop),
            ("occluder", occluder),
        ] {
            let observed = external_window_bounds(window, process_id)?;
            if observed != initial_bounds {
                let _ = process_group.terminate();
                let _ = child.wait();
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("fixture {label} bounds are {observed:?}, expected {initial_bounds:?}"),
                ));
            }
        }
        if unsafe { IsWindowVisible(target) } == 0
            || unsafe { IsWindowVisible(backdrop) } == 0
            || unsafe { IsWindowVisible(occluder) } != 0
        {
            let _ = process_group.terminate();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "fixture windows did not start in target/backdrop visible, occluder hidden state",
            ));
        }
        let (moved_bounds, resized_bounds) = recording_fixture_dynamic_bounds(initial_bounds)?;
        Ok(Self {
            child,
            process_group,
            process_id,
            target_title,
            target,
            backdrop,
            occluder,
            initial_bounds,
            moved_bounds,
            resized_bounds,
            stopped: false,
        })
    }

    fn move_target(&self) -> io::Result<FixturePhaseState> {
        self.set_target_bounds(self.moved_bounds)?;
        let state = self.phase_state()?;
        validate_fixture_phase_state("moved", state, Some(self.moved_bounds), false, false)?;
        Ok(state)
    }

    fn resize_target(&self) -> io::Result<FixturePhaseState> {
        self.set_target_bounds(self.resized_bounds)?;
        let state = self.phase_state()?;
        validate_fixture_phase_state("resized", state, Some(self.resized_bounds), false, false)?;
        Ok(state)
    }

    fn occlude_target(&self) -> io::Result<FixturePhaseState> {
        self.set_target_bounds(self.initial_bounds)?;
        self.ensure_handle(self.occluder)?;
        set_fixture_window_bounds(self.occluder, self.initial_bounds, true)?;
        let state = self.phase_state()?;
        validate_fixture_phase_state("occluded", state, Some(self.initial_bounds), false, true)?;
        Ok(state)
    }

    fn minimize_target(&self, timeout: Duration) -> io::Result<FixturePhaseState> {
        self.ensure_handle(self.occluder)?;
        // ShowWindow reports prior visibility, so the postcondition is checked separately.
        unsafe { ShowWindow(self.occluder, SW_HIDE) };
        self.set_target_bounds(self.initial_bounds)?;
        unsafe { ShowWindow(self.target, SW_MINIMIZE) };
        let deadline = Instant::now() + timeout;
        while unsafe { IsIconic(self.target) } == 0 {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "fixture target did not become minimized",
                ));
            }
            thread::sleep(Duration::from_millis(25));
        }
        let state = self.phase_state()?;
        validate_fixture_phase_state("minimized", state, None, true, false)?;
        Ok(state)
    }

    fn hide_all(&self) -> io::Result<()> {
        for window in [self.occluder, self.target, self.backdrop] {
            self.ensure_handle(window)?;
            unsafe { ShowWindow(window, SW_HIDE) };
        }
        Ok(())
    }

    fn set_target_bounds(&self, bounds: PhysicalRect) -> io::Result<()> {
        self.ensure_handle(self.backdrop)?;
        self.ensure_handle(self.target)?;
        // Re-raise the backdrop before the target so exposed pixels never depend on user windows.
        set_fixture_window_bounds(self.backdrop, self.initial_bounds, true)?;
        unsafe { ShowWindow(self.target, SW_RESTORE) };
        set_fixture_window_bounds(self.target, bounds, true)?;
        let observed = external_window_bounds(self.target, self.process_id)?;
        if observed != bounds {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("fixture target bounds are {observed:?}, expected {bounds:?}"),
            ));
        }
        Ok(())
    }

    fn phase_state(&self) -> io::Result<FixturePhaseState> {
        Ok(FixturePhaseState {
            target_bounds: external_window_bounds(self.target, self.process_id)?,
            target_visible: unsafe { IsWindowVisible(self.target) } != 0,
            target_minimized: unsafe { IsIconic(self.target) } != 0,
            backdrop_visible: unsafe { IsWindowVisible(self.backdrop) } != 0,
            occluder_visible: unsafe { IsWindowVisible(self.occluder) } != 0,
        })
    }

    fn ensure_handle(&self, window: HWND) -> io::Result<()> {
        external_window_bounds(window, self.process_id).map(|_| ())
    }

    /// Closes the helper gracefully and uses its Job Object only as a bounded fallback.
    fn shutdown(&mut self, timeout: Duration) -> io::Result<()> {
        if self.stopped {
            return Ok(());
        }
        for window in [self.occluder, self.target, self.backdrop] {
            if !window.is_null() && unsafe { IsWindow(window) } != 0 {
                unsafe { PostMessageW(window, WM_CLOSE, 0, 0) };
            }
        }
        let deadline = Instant::now() + timeout;
        loop {
            if self.child.try_wait()?.is_some() {
                self.stopped = true;
                return Ok(());
            }
            if Instant::now() >= deadline {
                let _ = self.process_group.terminate();
                let _ = self.child.wait();
                self.stopped = true;
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "recording fixture required forced process cleanup",
                ));
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
}

#[cfg(windows)]
impl Drop for RecordingWindowFixture {
    fn drop(&mut self) {
        if !self.stopped {
            let _ = self.process_group.terminate();
            let _ = self.child.wait();
            self.stopped = true;
        }
    }
}

#[cfg(windows)]
fn wait_for_recording_fixture_windows(
    child: &mut process::Child,
    process_id: u32,
    titles: [&str; 3],
    timeout: Duration,
) -> io::Result<[HWND; 3]> {
    let wide_titles = titles.map(wide_null);
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Err(io::Error::other(format!(
                "recording fixture exited before its windows were ready: {status}"
            )));
        }
        let handles = [
            unsafe { FindWindowW(ptr::null(), wide_titles[0].as_ptr()) },
            unsafe { FindWindowW(ptr::null(), wide_titles[1].as_ptr()) },
            unsafe { FindWindowW(ptr::null(), wide_titles[2].as_ptr()) },
        ];
        if handles.iter().all(|window| !window.is_null())
            && handles
                .iter()
                .all(|window| external_window_bounds(*window, process_id).is_ok())
        {
            return Ok(handles);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "recording fixture windows did not become ready",
            ));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(windows)]
fn external_window_bounds(window: HWND, expected_process_id: u32) -> io::Result<PhysicalRect> {
    if window.is_null() || unsafe { IsWindow(window) } == 0 {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "recording fixture window is unavailable",
        ));
    }
    let mut process_id = 0;
    unsafe { GetWindowThreadProcessId(window, &mut process_id) };
    if process_id != expected_process_id {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "recording fixture HWND belongs to an unexpected process",
        ));
    }
    let mut rect = RECT::default();
    if unsafe { GetWindowRect(window, &mut rect) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let bounds = PhysicalRect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    };
    if bounds.left >= bounds.right || bounds.top >= bounds.bottom {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recording fixture window has invalid bounds",
        ));
    }
    Ok(bounds)
}

#[cfg(windows)]
fn set_fixture_window_bounds(window: HWND, bounds: PhysicalRect, show: bool) -> io::Result<()> {
    let flags = SWP_NOACTIVATE | if show { SWP_SHOWWINDOW } else { 0 };
    // SAFETY: the caller validated the HWND and supplies increasing physical bounds.
    if unsafe {
        SetWindowPos(
            window,
            HWND_TOP,
            bounds.left,
            bounds.top,
            bounds.width() as i32,
            bounds.height() as i32,
            flags,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(windows)]
/// Waits for shortcut registration, owns cursor restoration, and finalizes one truthful report.
fn run_interaction_sequence(
    context: &WorkerContext,
    report: &mut AcceptanceReport,
) -> io::Result<()> {
    let shortcut_registered = match context.shortcut_readiness.recv_timeout(context.timeout) {
        Ok(registered) => registered,
        Err(_) => {
            let error = io::Error::new(
                io::ErrorKind::TimedOut,
                "shortcut readiness was not reported; no input was injected",
            );
            set_failed_report(report, error.to_string());
            write_report(&context.report_path, report)?;
            return Err(error);
        }
    };
    if !shortcut_registered {
        let error = io::Error::new(
            io::ErrorKind::AddrInUse,
            "Ctrl+Alt+F24 could not be registered; no input was injected",
        );
        set_failed_report(report, error.to_string());
        write_report(&context.report_path, report)?;
        return Err(error);
    }
    report.shortcut_registered = true;
    write_report(&context.report_path, report)?;
    let cursor = match CursorRestore::capture() {
        Ok(cursor) => cursor,
        Err(error) => {
            set_failed_report(
                report,
                format!("cursor snapshot failed before input: {error}"),
            );
            write_report(&context.report_path, report)?;
            return Err(error);
        }
    };

    let outcome = if context.record_target.is_some() {
        execute_recording_interactions(context, report)
    } else {
        execute_capture_interactions(context, report)
    };
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
            set_failed_report(report, error.to_string());
        }
    }
    let report_result = write_report(&context.report_path, report);
    match (final_result, report_result) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

#[cfg(windows)]
/// Drives real selection, toolbar, restart, export, Pin, Copy, and cleanup interactions.
fn execute_capture_interactions(
    context: &WorkerContext,
    report: &mut AcceptanceReport,
) -> io::Result<()> {
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
    let plan = interaction_plan_for_window(first_overlay.handle)?;

    let first_drag = inject_mouse_drag(
        first_overlay.handle,
        plan.drag_start,
        plan.drag_end,
        context.display.physical_bounds,
    )?;
    let foreground = first_drag.foreground;
    thread::sleep(context.settle_delay);
    let selected = capture_evidence(context, "01-selected.png", first_overlay)?;
    record_step(
        report,
        &context.report_path,
        "selection_drag",
        foreground,
        Some(&selected),
    )?;

    let before_nudge = wait_for_capture_state(context, "initial selection state", |state| {
        state.session_state == "selecting" && state.selection.is_some() && state.overlay_count == 1
    })?
    .selection
    .ok_or_else(|| io::Error::other("selection disappeared before keyboard nudge"))?;
    validate_selection_geometry(
        first_drag.selection,
        before_nudge,
        "initial overlay selection",
    )?;
    let expected_after_nudge = translated_rect(before_nudge, 1, 0)?;
    let foreground = inject_key(first_overlay.handle, VK_RIGHT)?;
    let after_nudge = wait_for_capture_state(context, "one-pixel keyboard nudge", |state| {
        state.selection == Some(expected_after_nudge)
    })?
    .selection
    .ok_or_else(|| io::Error::other("selection disappeared after keyboard nudge"))?;
    thread::sleep(context.settle_delay);
    let nudged = capture_evidence(context, "02-nudged.png", first_overlay)?;
    ensure_evidence_changed(&selected, &nudged, "Right did not move the selection")?;
    record_step(
        report,
        &context.report_path,
        "selection_nudge_right",
        foreground,
        Some(&nudged),
    )?;

    let foreground = inject_mouse_click(first_overlay.handle, plan.more)?;
    thread::sleep(context.settle_delay);
    let more = capture_evidence(context, "03-more.png", first_overlay)?;
    ensure_evidence_changed(&nudged, &more, "More did not change the overlay")?;
    record_step(
        report,
        &context.report_path,
        "more",
        foreground,
        Some(&more),
    )?;

    let foreground = inject_mouse_click(first_overlay.handle, plan.more)?;
    thread::sleep(context.settle_delay);
    let less = capture_evidence(context, "04-less.png", first_overlay)?;
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
    wait_for_window_gone(first_overlay.handle, context.timeout, "Capture restart")?;
    let second_overlay = wait_for_overlay(
        controller.handle,
        context.display.physical_bounds,
        context.timeout,
    )?;
    focus_owned_window(second_overlay, context.timeout)?;
    thread::sleep(context.settle_delay);
    let second_plan = interaction_plan_for_window(second_overlay.handle)?;
    let second_drag = inject_mouse_drag(
        second_overlay.handle,
        second_plan.drag_start,
        second_plan.drag_end,
        context.display.physical_bounds,
    )?;
    let foreground = second_drag.foreground;
    thread::sleep(context.settle_delay);
    let restarted = capture_evidence(context, "05-recapture.png", second_overlay)?;
    record_step(
        report,
        &context.report_path,
        "recapture_overlay_ready",
        foreground,
        Some(&restarted),
    )?;
    let recapture_selection = wait_for_capture_state(context, "recapture selection", |state| {
        state.session_state == "selecting" && state.selection.is_some() && state.overlay_count == 1
    })?
    .selection
    .ok_or_else(|| io::Error::other("recapture selection disappeared"))?;
    validate_selection_geometry(
        second_drag.selection,
        recapture_selection,
        "recapture overlay selection",
    )?;

    let foreground = inject_mouse_click(second_overlay.handle, second_plan.cancel)?;
    wait_for_window_gone(second_overlay.handle, context.timeout, "Cancel")?;
    record_step(
        report,
        &context.report_path,
        "second_cancel",
        foreground,
        None,
    )?;

    let nudge = NudgeReport {
        before: before_nudge,
        after: after_nudge,
        delta_x: 1,
        delta_y: 0,
    };
    let save = execute_save_interaction(context, report, controller)?;
    let pin = execute_pin_interaction(context, report, controller)?;
    let copy = execute_copy_interaction(context, report, controller)?;
    let final_state = wait_for_capture_state(context, "final capture cleanup", |state| {
        state.overlay_count == 0
            && state.pinned_count == 0
            && state.capture_preflight_ready
            && matches!(
                state.session_state.as_str(),
                "idle" | "completed" | "cancelled"
            )
    })?;
    let visible_process_windows = process_windows()?.len();
    if visible_process_windows != 0 {
        return Err(io::Error::other(format!(
            "capture action cleanup left {visible_process_windows} visible process window(s)"
        )));
    }
    report.capture_actions = Some(CaptureActionReport {
        initial_requested_selection: first_drag.selection,
        recapture_requested_selection: second_drag.selection,
        nudge,
        save,
        pin,
        copy,
        cleanup: CleanupReport {
            session_state: final_state.session_state,
            overlay_count: final_state.overlay_count,
            pinned_count: final_state.pinned_count,
            visible_process_windows,
            capture_preflight_ready: final_state.capture_preflight_ready,
        },
    });
    write_report(&context.report_path, report)
}

#[cfg(windows)]
/// Restores the hidden controller, opens a fresh overlay, and returns its measured selection.
fn begin_selected_overlay(
    context: &WorkerContext,
    controller: NativeWindow,
) -> io::Result<(
    NativeWindow,
    InteractionPlan,
    PhysicalRect,
    PhysicalRect,
    CaptureFrame,
)> {
    context
        .interaction_commands
        .send_blocking(OverlayInteractionAcceptanceCommand::ShowCaptureSettings)
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "capture command channel closed"))?;
    let controller = wait_for_owned_window_visible(controller.handle, context.timeout)?;
    focus_owned_window(controller, context.timeout)?;
    inject_capture_shortcut(controller.handle)?;
    let overlay = wait_for_overlay(
        controller.handle,
        context.display.physical_bounds,
        context.timeout,
    )?;
    focus_owned_window(overlay, context.timeout)?;
    thread::sleep(context.settle_delay);
    let plan = interaction_plan_for_window(overlay.handle)?;
    let drag = inject_mouse_drag(
        overlay.handle,
        plan.drag_start,
        plan.drag_end,
        context.display.physical_bounds,
    )?;
    let state = wait_for_capture_state(context, "fresh overlay selection", |state| {
        state.session_state == "selecting" && state.selection.is_some() && state.overlay_count == 1
    })?;
    let selection = state
        .selection
        .ok_or_else(|| io::Error::other("fresh overlay did not retain its selection"))?;
    validate_selection_geometry(drag.selection, selection, "fresh overlay selection")?;
    let content = query_capture_content(context, context.timeout.min(Duration::from_secs(1)))?;
    let source = content
        .selection
        .ok_or_else(|| io::Error::other("fresh overlay did not expose selected source pixels"))?;
    validate_frame_dimensions(&source, selection, "selected source frame")?;
    if source.bounds != selection {
        return Err(io::Error::other(format!(
            "selected source bounds {:?} do not match committed selection {selection:?}",
            source.bounds
        )));
    }
    Ok((overlay, plan, selection, drag.selection, source))
}

#[cfg(windows)]
/// Clicks the production Save action and drives only the uniquely owned common-file dialog.
fn execute_save_interaction(
    context: &WorkerContext,
    report: &mut AcceptanceReport,
    controller: NativeWindow,
) -> io::Result<SaveReport> {
    let export_directory = context.session_root.join("exports");
    fs::create_dir_all(&export_directory)?;
    let target = export_directory.join("selection.png");
    if target.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("isolated Save target already exists: {}", target.display()),
        ));
    }

    let (overlay, plan, selection, requested_selection, source) =
        begin_selected_overlay(context, controller)?;
    thread::sleep(context.settle_delay);
    let selected = capture_evidence(context, "06-save-selection.png", overlay)?;
    record_step(
        report,
        &context.report_path,
        "save_selection_ready",
        guard_foreground(overlay.handle)?,
        Some(&selected),
    )?;

    let foreground = inject_mouse_click(overlay.handle, plan.save)?;
    record_step(report, &context.report_path, "save_click", foreground, None)?;
    let dialog = wait_for_save_dialog(overlay.handle, controller.handle, context.timeout)?;
    // The shell dialog becomes visible before its first paint and focus transition complete.
    thread::sleep(context.settle_delay);
    let dialog_evidence = capture_evidence(context, "07-save-dialog.png", dialog)?;
    record_step(
        report,
        &context.report_path,
        "save_dialog_ready",
        dialog,
        Some(&dialog_evidence),
    )?;

    let edit = save_file_name_edit(dialog.handle, "flash-shot.png")?;
    let edit_center = PhysicalPoint {
        x: edit.bounds.left + edit.bounds.width() as i32 / 2,
        y: edit.bounds.top + edit.bounds.height() as i32 / 2,
    };
    inject_mouse_click(dialog.handle, edit_center)?;
    wait_for_window_focus(dialog.handle, edit.handle, context.timeout)?;
    inject_select_all(dialog.handle)?;
    let target_text = target.to_string_lossy().into_owned();
    inject_unicode_text(dialog.handle, &target_text)?;
    wait_for_window_text(edit.handle, &target_text, context.timeout)?;
    let path_evidence = capture_evidence(context, "08-save-path.png", dialog)?;
    record_step(
        report,
        &context.report_path,
        "save_path_verified",
        dialog,
        Some(&path_evidence),
    )?;
    inject_key(dialog.handle, VK_RETURN)?;
    wait_for_window_gone(dialog.handle, context.timeout, "Save confirmation")?;
    wait_for_window_gone(overlay.handle, context.timeout, "Save completion")?;
    let state = wait_for_capture_state(context, "selection Save completion", |state| {
        state.session_state == "completed"
            && state.overlay_count == 0
            && state.capture_preflight_ready
            && state.status.starts_with("Selection saved to ")
    })?;
    if state.selection != Some(selection) {
        return Err(io::Error::other(
            "completed Save no longer reports the exported selection",
        ));
    }
    let (saved, bytes) = wait_for_saved_png(&target, context.timeout)?;
    validate_frame_dimensions(&saved, selection, "saved PNG")?;
    let content = validate_same_pixel_content(&source, &saved, "saved PNG")?;
    ensure_path_within(&target, &context.session_root)?;
    Ok(SaveReport {
        requested_selection,
        selection,
        path: target
            .strip_prefix(&context.session_root)
            .unwrap_or(&target)
            .to_string_lossy()
            .into_owned(),
        width: saved.width,
        height: saved.height,
        bytes,
        content,
    })
}

#[cfg(windows)]
/// Opens a Pin through the real toolbar, verifies its source, then closes it with Escape.
fn execute_pin_interaction(
    context: &WorkerContext,
    report: &mut AcceptanceReport,
    controller: NativeWindow,
) -> io::Result<PinReport> {
    let (overlay, plan, selection, requested_selection, source) =
        begin_selected_overlay(context, controller)?;
    thread::sleep(context.settle_delay);
    let selected = capture_evidence(context, "09-pin-selection.png", overlay)?;
    record_step(
        report,
        &context.report_path,
        "pin_selection_ready",
        guard_foreground(overlay.handle)?,
        Some(&selected),
    )?;

    let foreground = inject_mouse_click(overlay.handle, plan.pin)?;
    record_step(report, &context.report_path, "pin_click", foreground, None)?;
    wait_for_window_gone(overlay.handle, context.timeout, "Pin")?;
    let state = wait_for_capture_state(context, "selected Pin window", |state| {
        state.session_state == "idle"
            && state.overlay_count == 0
            && state.pinned_count == 1
            && state.pinned_source_bounds == Some(selection)
            && state.capture_preflight_ready
    })?;
    let source_bounds = state
        .pinned_source_bounds
        .ok_or_else(|| io::Error::other("Pin source bounds were not reported"))?;
    let pinned = query_capture_content(context, context.timeout.min(Duration::from_secs(1)))?
        .pin
        .ok_or_else(|| io::Error::other("selected Pin did not expose its source pixels"))?;
    let content = validate_same_pixel_content(&source, &pinned, "pinned frame")?;
    let pin = wait_for_single_visible_pin(controller.handle, context.timeout)?;
    focus_owned_window(pin, context.timeout)?;
    thread::sleep(context.settle_delay);
    let pin_evidence = capture_evidence(context, "10-pin.png", pin)?;
    record_step(
        report,
        &context.report_path,
        "pin_visible",
        pin,
        Some(&pin_evidence),
    )?;
    let pin_report = PinReport {
        requested_selection,
        selection,
        source_bounds,
        window: pin.report(),
        content,
    };

    let foreground = inject_key(pin.handle, VK_ESCAPE)?;
    wait_for_window_gone(pin.handle, context.timeout, "Pin Escape")?;
    wait_for_capture_state(context, "Pin cleanup", |state| {
        state.pinned_count == 0 && state.overlay_count == 0 && state.capture_preflight_ready
    })?;
    record_step(report, &context.report_path, "pin_escape", foreground, None)?;
    Ok(pin_report)
}

#[cfg(windows)]
/// Routes a real Copy click into the injected sink and proves Windows clipboard state is unchanged.
fn execute_copy_interaction(
    context: &WorkerContext,
    report: &mut AcceptanceReport,
    controller: NativeWindow,
) -> io::Result<CopyReport> {
    match context.copy_results.try_recv() {
        Err(mpsc::TryRecvError::Empty) => {}
        Err(mpsc::TryRecvError::Disconnected) => {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "selection Copy result channel disconnected",
            ));
        }
        Ok(_) => {
            return Err(io::Error::other(
                "selection Copy sink contained an unexpected earlier frame",
            ));
        }
    }
    let (overlay, plan, selection, requested_selection, source) =
        begin_selected_overlay(context, controller)?;
    thread::sleep(context.settle_delay);
    let selected = capture_evidence(context, "11-copy-selection.png", overlay)?;
    record_step(
        report,
        &context.report_path,
        "copy_selection_ready",
        guard_foreground(overlay.handle)?,
        Some(&selected),
    )?;

    // SAFETY: this call only reads the monotonic user32 clipboard change counter.
    let clipboard_sequence_before = unsafe { GetClipboardSequenceNumber() };
    let foreground = inject_mouse_click(overlay.handle, plan.copy)?;
    let copied =
        context
            .copy_results
            .recv_timeout(context.timeout)
            .map_err(|error| match error {
                RecvTimeoutError::Timeout => io::Error::new(
                    io::ErrorKind::TimedOut,
                    "selection Copy did not reach the injected sink",
                ),
                RecvTimeoutError::Disconnected => io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "selection Copy result channel disconnected",
                ),
            })?;
    wait_for_window_gone(overlay.handle, context.timeout, "Copy")?;
    let state = wait_for_capture_state(context, "selection Copy completion", |state| {
        state.session_state == "completed"
            && state.overlay_count == 0
            && state.capture_preflight_ready
            && state.status == "Selection copied to clipboard"
    })?;
    if state.selection != Some(selection) {
        return Err(io::Error::other(
            "completed Copy no longer reports the exported selection",
        ));
    }
    if context.copy_results.try_recv().is_ok() {
        return Err(io::Error::other(
            "one Copy click produced more than one selection frame",
        ));
    }
    validate_frame_dimensions(&copied, selection, "copied frame")?;
    if copied.bounds != selection {
        return Err(io::Error::other(format!(
            "copied frame bounds {:?} do not match selection {:?}",
            copied.bounds, selection
        )));
    }
    let content = validate_same_pixel_content(&source, &copied, "copied frame")?;
    // SAFETY: this call only reads the monotonic user32 clipboard change counter.
    let clipboard_sequence_after = unsafe { GetClipboardSequenceNumber() };
    if clipboard_sequence_after != clipboard_sequence_before {
        return Err(io::Error::other(format!(
            "system clipboard sequence changed from {clipboard_sequence_before} to {clipboard_sequence_after}"
        )));
    }
    record_step(report, &context.report_path, "copy_click", foreground, None)?;
    Ok(CopyReport {
        requested_selection,
        selection,
        copied_bounds: copied.bounds,
        width: copied.width,
        height: copied.height,
        clipboard_sequence_before,
        clipboard_sequence_after,
        clipboard_unchanged: true,
        content,
    })
}

fn translated_rect(rect: PhysicalRect, delta_x: i32, delta_y: i32) -> io::Result<PhysicalRect> {
    Ok(PhysicalRect {
        left: rect
            .left
            .checked_add(delta_x)
            .ok_or_else(|| io::Error::other("selection nudge overflowed left"))?,
        top: rect
            .top
            .checked_add(delta_y)
            .ok_or_else(|| io::Error::other("selection nudge overflowed top"))?,
        right: rect
            .right
            .checked_add(delta_x)
            .ok_or_else(|| io::Error::other("selection nudge overflowed right"))?,
        bottom: rect
            .bottom
            .checked_add(delta_y)
            .ok_or_else(|| io::Error::other("selection nudge overflowed bottom"))?,
    })
}

#[cfg(windows)]
fn wait_for_saved_png(path: &Path, timeout: Duration) -> io::Result<(CaptureFrame, u64)> {
    let deadline = Instant::now() + timeout;
    loop {
        let last_error = match fs::metadata(path) {
            Ok(metadata) if metadata.len() > 0 => match CaptureFrame::open_png(path) {
                Ok(frame) => return Ok((frame, metadata.len())),
                Err(error) => error.to_string(),
            },
            Ok(_) => "saved file is empty".to_owned(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => error.to_string(),
            Err(error) => return Err(error),
        };
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("saved PNG did not become readable: {last_error}"),
            ));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn validate_frame_dimensions(
    frame: &CaptureFrame,
    selection: PhysicalRect,
    label: &str,
) -> io::Result<()> {
    if frame.width != selection.width() || frame.height != selection.height() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{label} is {}x{}, expected {}x{}",
                frame.width,
                frame.height,
                selection.width(),
                selection.height()
            ),
        ));
    }
    Ok(())
}

#[cfg(windows)]
#[derive(Clone, Copy)]
struct FrameContentMetrics {
    fingerprint: u64,
    luma_min: u8,
    luma_max: u8,
}

#[cfg(windows)]
#[derive(Clone, Copy)]
struct RecordingFrameContentComparison {
    mean_absolute_error: f64,
    reference: FrameContentMetrics,
    decoded: FrameContentMetrics,
}

#[cfg(windows)]
/// Compares meaningful BGRA rows exactly while ignoring origin and stride padding differences.
fn validate_same_pixel_content(
    source: &CaptureFrame,
    result: &CaptureFrame,
    label: &str,
) -> io::Result<ExactPixelMatchReport> {
    source.validate()?;
    result.validate()?;
    if source.format != result.format
        || source.width != result.width
        || source.height != result.height
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{label} format or dimensions differ: source {:?} {}x{}, result {:?} {}x{}",
                source.format,
                source.width,
                source.height,
                result.format,
                result.width,
                result.height
            ),
        ));
    }
    let source_metrics = frame_content_metrics(source)?;
    let result_metrics = frame_content_metrics(result)?;
    if source_metrics
        .luma_max
        .saturating_sub(source_metrics.luma_min)
        < 8
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{label} source frame is visually flat (luma {}..{})",
                source_metrics.luma_min, source_metrics.luma_max
            ),
        ));
    }
    let row_bytes = source.width as usize * 4;
    let exact_match = (0..source.height as usize).all(|row| {
        let source_start = row * source.stride;
        let result_start = row * result.stride;
        source.pixels[source_start..source_start + row_bytes]
            == result.pixels[result_start..result_start + row_bytes]
    });
    if !exact_match {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{label} pixels differ from source ({:016x} != {:016x})",
                source_metrics.fingerprint, result_metrics.fingerprint
            ),
        ));
    }
    Ok(ExactPixelMatchReport {
        source_fingerprint: format!("{:016x}", source_metrics.fingerprint),
        result_fingerprint: format!("{:016x}", result_metrics.fingerprint),
        source_luma_min: source_metrics.luma_min,
        source_luma_max: source_metrics.luma_max,
        exact_match,
    })
}

#[cfg(windows)]
/// Hashes visible pixels and records their luminance range for reportable nonblank evidence.
fn frame_content_metrics(frame: &CaptureFrame) -> io::Result<FrameContentMetrics> {
    frame.validate()?;
    let row_bytes = frame.width as usize * 4;
    let mut fingerprint = 0xcbf29ce484222325_u64;
    let mut luma_min = u8::MAX;
    let mut luma_max = u8::MIN;
    for row in 0..frame.height as usize {
        let start = row * frame.stride;
        for pixel in frame.pixels[start..start + row_bytes].chunks_exact(4) {
            for byte in pixel {
                fingerprint = (fingerprint ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
            }
            let luma = ((77_u16 * u16::from(pixel[2])
                + 150_u16 * u16::from(pixel[1])
                + 29_u16 * u16::from(pixel[0]))
                >> 8) as u8;
            luma_min = luma_min.min(luma);
            luma_max = luma_max.max(luma);
        }
    }
    Ok(FrameContentMetrics {
        fingerprint,
        luma_min,
        luma_max,
    })
}

#[cfg(windows)]
/// Measures a lossy H.264 frame against its desktop reference using stable RGB tile means.
fn compare_recording_frame_content(
    reference: &CaptureFrame,
    decoded: &CaptureFrame,
) -> io::Result<RecordingFrameContentComparison> {
    reference.validate()?;
    decoded.validate()?;
    if reference.format != decoded.format
        || decoded.width < reference.width
        || decoded.height < reference.height
        || decoded.width > reference.width.saturating_add(1)
        || decoded.height > reference.height.saturating_add(1)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "decoded frame {:?} {}x{} cannot represent reference {:?} {}x{}",
                decoded.format,
                decoded.width,
                decoded.height,
                reference.format,
                reference.width,
                reference.height
            ),
        ));
    }
    let reference_metrics = frame_content_metrics(reference)?;
    let decoded_metrics = frame_content_metrics(decoded)?;
    if reference_metrics
        .luma_max
        .saturating_sub(reference_metrics.luma_min)
        < 8
        || decoded_metrics
            .luma_max
            .saturating_sub(decoded_metrics.luma_min)
            < 8
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recording reference or decoded frame is visually flat",
        ));
    }
    let reference_grid = frame_rgb_grid(reference, reference.width, reference.height)?;
    let decoded_grid = frame_rgb_grid(decoded, reference.width, reference.height)?;
    let absolute_error = reference_grid
        .iter()
        .zip(&decoded_grid)
        .flat_map(|(left, right)| left.iter().zip(right))
        .map(|(left, right)| (f64::from(*left) - f64::from(*right)).abs())
        .sum::<f64>();
    let mean_absolute_error = absolute_error / (reference_grid.len() * 3) as f64;
    Ok(RecordingFrameContentComparison {
        mean_absolute_error,
        reference: reference_metrics,
        decoded: decoded_metrics,
    })
}

#[cfg(windows)]
fn validate_recording_frame_content(
    reference: &CaptureFrame,
    decoded: &CaptureFrame,
) -> io::Result<RecordingFrameContentComparison> {
    let comparison = compare_recording_frame_content(reference, decoded)?;
    if comparison.mean_absolute_error > MAX_RECORDING_GRID_MAE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "decoded recording frame differs from its desktop reference: grid MAE {:.3} > {MAX_RECORDING_GRID_MAE:.3}",
                comparison.mean_absolute_error
            ),
        ));
    }
    Ok(comparison)
}

#[cfg(windows)]
/// Captures one stable desktop-composition phase while independently rechecking source bounds.
fn capture_recording_window_phase(
    context: &WorkerContext,
    report: &mut AcceptanceReport,
    stage: &'static str,
    file_stem: &str,
    expected_source_bounds: PhysicalRect,
    fixture: FixturePhaseState,
) -> io::Result<PendingRecordingWindowPhase> {
    let baseline = wait_for_recording_state(context, stage, |state| {
        state.active && !state.paused && !state.starting && !state.stopping
    })?;
    let baseline_bounds = baseline.target_bounds.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{stage} did not report physical source bounds"),
        )
    })?;
    validate_recording_target_bounds(
        RecordTargetOption::Window,
        expected_source_bounds,
        baseline_bounds,
    )?;
    let stable_after = baseline
        .progress_time_us
        .checked_add(WINDOW_PHASE_SETTLE_US)
        .ok_or_else(|| io::Error::other("recording phase timestamp overflow"))?;
    let stable = wait_for_recording_state(context, stage, |state| {
        state.active && !state.paused && !state.stopping && state.progress_time_us >= stable_after
    })?;
    let reported_source_bounds = stable.target_bounds.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{stage} lost its physical source bounds"),
        )
    })?;
    validate_recording_target_bounds(
        RecordTargetOption::Window,
        expected_source_bounds,
        reported_source_bounds,
    )?;
    let reference = SystemCaptureBackend.capture(expected_source_bounds)?;
    if reference.bounds != expected_source_bounds {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{stage} reference captured {:?}, expected {expected_source_bounds:?}",
                reference.bounds
            ),
        ));
    }
    let reference_file = format!("screenshots/{file_stem}-reference.png");
    reference.save_png(context.session_root.join(&reference_file))?;
    let timestamp_seconds = stable.progress_time_us as f64 / 1_000_000.0;
    record_recording_state(report, &context.report_path, stage, stable.clone())?;
    let held_after = stable
        .progress_time_us
        .checked_add(WINDOW_PHASE_HOLD_US)
        .ok_or_else(|| io::Error::other("recording phase hold timestamp overflow"))?;
    let held = wait_for_recording_state(context, stage, |state| {
        state.active && !state.paused && !state.stopping && state.progress_time_us >= held_after
    })?;
    let held_bounds = held.target_bounds.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{stage} lost source bounds during its stable hold"),
        )
    })?;
    validate_recording_target_bounds(
        RecordTargetOption::Window,
        expected_source_bounds,
        held_bounds,
    )?;
    Ok(PendingRecordingWindowPhase {
        stage,
        reference_file,
        decoded_file: format!("screenshots/{file_stem}-decoded.png"),
        timestamp_seconds,
        reference,
        fixture,
        reported_source_bounds,
        maximum_progress_frame: held.progress_frame.max(stable.progress_frame),
    })
}

#[cfg(windows)]
/// Matches every dynamic reference to two consecutive low-rate frames in chronological order.
fn finalize_recording_window_timeline(
    ffmpeg: &Path,
    output: &Path,
    session_root: &Path,
    initial_reference: &CaptureFrame,
    phases: Vec<PendingRecordingWindowPhase>,
) -> io::Result<(
    RecordingContentReport,
    Vec<RecordingWindowPhaseReport>,
    usize,
)> {
    let candidates = extract_recording_timeline(ffmpeg, output, session_root)?;
    let (initial_match, mut next_candidate) =
        match_stable_recording_frame(initial_reference, &candidates, 0, "initial")?;
    let initial_decoded = "screenshots/recording-decoded-frame.png".to_owned();
    fs::copy(
        &candidates[initial_match.candidate_index].path,
        session_root.join(&initial_decoded),
    )?;
    let initial_content = recording_content_report(
        "screenshots/recording-source-reference.png".to_owned(),
        initial_decoded,
        candidates[initial_match.candidate_index].timestamp_seconds,
        initial_match.comparison,
    );
    let mut reports = Vec::with_capacity(phases.len());
    for phase in phases {
        let (matched, next) = match_stable_recording_frame(
            &phase.reference,
            &candidates,
            next_candidate,
            phase.stage,
        )?;
        next_candidate = next;
        fs::copy(
            &candidates[matched.candidate_index].path,
            session_root.join(&phase.decoded_file),
        )?;
        reports.push(RecordingWindowPhaseReport {
            stage: phase.stage,
            progress_timestamp_seconds: phase.timestamp_seconds,
            target_bounds: phase.fixture.target_bounds,
            target_visible: phase.fixture.target_visible,
            target_minimized: phase.fixture.target_minimized,
            backdrop_visible: phase.fixture.backdrop_visible,
            occluder_visible: phase.fixture.occluder_visible,
            reported_source_bounds: phase.reported_source_bounds,
            content: recording_content_report(
                phase.reference_file,
                phase.decoded_file,
                candidates[matched.candidate_index].timestamp_seconds,
                matched.comparison,
            ),
        });
    }
    Ok((initial_content, reports, candidates.len()))
}

#[cfg(windows)]
fn extract_recording_timeline(
    ffmpeg: &Path,
    output: &Path,
    session_root: &Path,
) -> io::Result<Vec<RecordingTimelineCandidate>> {
    let timeline_directory = session_root
        .join("screenshots")
        .join("recording-window-timeline");
    fs::create_dir_all(&timeline_directory)?;
    let pattern = timeline_directory.join("frame-%05d.png");
    extract_video_frame_series(ffmpeg, output, WINDOW_TIMELINE_FRAMES_PER_SECOND, &pattern)?;
    let mut indexed = fs::read_dir(&timeline_directory)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let index = path
                .file_stem()?
                .to_str()?
                .strip_prefix("frame-")?
                .parse::<usize>()
                .ok()?;
            Some((index, path))
        })
        .collect::<Vec<_>>();
    indexed.sort_by_key(|(index, _)| *index);
    if indexed.is_empty()
        || indexed
            .iter()
            .enumerate()
            .any(|(expected, (observed, _))| expected != *observed)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "FFmpeg recording timeline is empty or has non-contiguous frame names",
        ));
    }
    Ok(indexed
        .into_iter()
        .map(|(index, path)| RecordingTimelineCandidate {
            timestamp_seconds: index as f64 / f64::from(WINDOW_TIMELINE_FRAMES_PER_SECOND),
            path,
        })
        .collect())
}

#[cfg(windows)]
/// Finds the first stable matching run so a transient repaint cannot satisfy a phase.
fn match_stable_recording_frame(
    reference: &CaptureFrame,
    candidates: &[RecordingTimelineCandidate],
    start_index: usize,
    stage: &str,
) -> io::Result<(StableRecordingFrameMatch, usize)> {
    let mut comparisons = vec![None; candidates.len()];
    let mut errors = vec![f64::INFINITY; candidates.len()];
    let mut best_error = f64::INFINITY;
    for (index, candidate) in candidates.iter().enumerate().skip(start_index) {
        let decoded = CaptureFrame::open_png(&candidate.path)?;
        if let Ok(comparison) = compare_recording_frame_content(reference, &decoded) {
            best_error = best_error.min(comparison.mean_absolute_error);
            errors[index] = comparison.mean_absolute_error;
            comparisons[index] = Some(comparison);
        }
    }
    if let Some((candidate_index, next_candidate)) = first_stable_recording_match(
        &errors,
        start_index,
        MAX_RECORDING_GRID_MAE,
        WINDOW_TIMELINE_STABLE_FRAMES,
    ) {
        return Ok((
            StableRecordingFrameMatch {
                candidate_index,
                comparison: comparisons[candidate_index]
                    .expect("a passing timeline error has a comparison"),
            },
            next_candidate,
        ));
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "recording timeline has no stable {stage} match after sample {start_index}; best grid MAE {best_error:.3}"
        ),
    ))
}

/// Selects the lowest-error sample in the first consecutive passing run.
fn first_stable_recording_match(
    errors: &[f64],
    start_index: usize,
    maximum_error: f64,
    stable_frames: usize,
) -> Option<(usize, usize)> {
    if stable_frames == 0 || start_index >= errors.len() || stable_frames > errors.len() {
        return None;
    }
    let last_start = errors.len().checked_sub(stable_frames)?;
    for run_start in start_index..=last_start {
        let run = &errors[run_start..run_start + stable_frames];
        if run
            .iter()
            .all(|error| error.is_finite() && *error <= maximum_error)
        {
            let best_offset = run
                .iter()
                .enumerate()
                .min_by(|left, right| left.1.total_cmp(right.1))?
                .0;
            return Some((run_start + best_offset, run_start + stable_frames));
        }
    }
    None
}

#[cfg(windows)]
fn recording_content_report(
    reference: String,
    decoded_frame: String,
    timestamp_seconds: f64,
    comparison: RecordingFrameContentComparison,
) -> RecordingContentReport {
    RecordingContentReport {
        reference,
        decoded_frame,
        timestamp_seconds,
        reference_fingerprint: format!("{:016x}", comparison.reference.fingerprint),
        decoded_fingerprint: format!("{:016x}", comparison.decoded.fingerprint),
        reference_luma_min: comparison.reference.luma_min,
        reference_luma_max: comparison.reference.luma_max,
        decoded_luma_min: comparison.decoded.luma_min,
        decoded_luma_max: comparison.decoded.luma_max,
        grid_mean_absolute_error: comparison.mean_absolute_error,
        maximum_allowed_error: MAX_RECORDING_GRID_MAE,
    }
}

/// Ensures every dynamic phase actually presented a different desktop composition.
fn validate_distinct_recording_phase_fingerprints(fingerprints: &[&str]) -> io::Result<()> {
    for (index, fingerprint) in fingerprints.iter().enumerate() {
        if fingerprints[..index].contains(fingerprint) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("recording phase fingerprint {fingerprint} was repeated"),
            ));
        }
    }
    Ok(())
}

#[cfg(windows)]
/// Reduces a frame to spatial RGB averages so codec noise passes but blank/wrong regions do not.
fn frame_rgb_grid(frame: &CaptureFrame, width: u32, height: u32) -> io::Result<Vec<[u8; 3]>> {
    if width == 0 || height == 0 || width > frame.width || height > frame.height {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "RGB grid bounds must fit inside the frame",
        ));
    }
    let columns = width.min(16) as usize;
    let rows = height.min(16) as usize;
    let mut grid = Vec::with_capacity(columns * rows);
    for grid_y in 0..rows {
        let top = grid_y * height as usize / rows;
        let bottom = (grid_y + 1) * height as usize / rows;
        for grid_x in 0..columns {
            let left = grid_x * width as usize / columns;
            let right = (grid_x + 1) * width as usize / columns;
            let mut blue = 0_u64;
            let mut green = 0_u64;
            let mut red = 0_u64;
            let mut count = 0_u64;
            for y in top..bottom {
                let row_start = y * frame.stride;
                for x in left..right {
                    let offset = row_start + x * 4;
                    blue += u64::from(frame.pixels[offset]);
                    green += u64::from(frame.pixels[offset + 1]);
                    red += u64::from(frame.pixels[offset + 2]);
                    count += 1;
                }
            }
            grid.push([
                (red / count) as u8,
                (green / count) as u8,
                (blue / count) as u8,
            ]);
        }
    }
    Ok(grid)
}

fn ensure_path_within(path: &Path, root: &Path) -> io::Result<()> {
    let path = fs::canonicalize(path)?;
    let root = fs::canonicalize(root)?;
    if path.starts_with(&root) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("saved path {} escaped {}", path.display(), root.display()),
        ))
    }
}

#[cfg(windows)]
/// Drives the real overlay entry plus Record-page pause, resume, stop, and MP4 verification.
fn execute_recording_interactions(
    context: &WorkerContext,
    report: &mut AcceptanceReport,
) -> io::Result<()> {
    let target = context
        .record_target
        .ok_or_else(|| io::Error::other("recording workflow requires a target"))?;
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
        "capture_shortcut",
        foreground,
        None,
    )?;
    let overlay = wait_for_overlay(
        controller.handle,
        context.display.physical_bounds,
        context.timeout,
    )?;
    focus_owned_window(overlay, context.timeout)?;
    thread::sleep(context.settle_delay);
    let plan = interaction_plan_for_window(overlay.handle)?;

    let drag = inject_mouse_drag(
        overlay.handle,
        plan.drag_start,
        plan.drag_end,
        context.display.physical_bounds,
    )?;
    let foreground = drag.foreground;
    thread::sleep(context.settle_delay);
    let selected = capture_evidence(context, "01-selected.png", overlay)?;
    record_step(
        report,
        &context.report_path,
        "selection_drag",
        foreground,
        Some(&selected),
    )?;
    let committed_selection = wait_for_capture_state(context, "recording selection", |state| {
        state.session_state == "selecting" && state.selection.is_some() && state.overlay_count == 1
    })?
    .selection
    .ok_or_else(|| io::Error::other("recording selection disappeared"))?;
    validate_selection_geometry(
        drag.selection,
        committed_selection,
        "recording overlay selection",
    )?;
    let mut window_fixture = None;
    let (expected_source_bounds, window_title) = match target {
        RecordTargetOption::Area => (committed_selection, None),
        RecordTargetOption::Window => {
            let fixture = RecordingWindowFixture::launch(committed_selection, context.timeout)?;
            focus_owned_window(overlay, context.timeout)?;
            thread::sleep(context.settle_delay);
            let center = PhysicalPoint {
                x: committed_selection.left + committed_selection.width() as i32 / 2,
                y: committed_selection.top + committed_selection.height() as i32 / 2,
            };
            let window_target = SystemWindowInspector
                .window_capture_target_at(center)?
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        "no external titled window is visible beneath the selection center",
                    )
                })?;
            if window_target.title != fixture.target_title
                || window_target.bounds != fixture.initial_bounds
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "window inspector selected {:?} at {:?}, expected fixture {:?} at {:?}",
                        window_target.title,
                        window_target.bounds,
                        fixture.target_title,
                        fixture.initial_bounds
                    ),
                ));
            }
            let bounds = fixture.initial_bounds;
            let title = fixture.target_title.clone();
            window_fixture = Some(fixture);
            (bounds, Some(title))
        }
    };

    let foreground = inject_mouse_click(overlay.handle, plan.more)?;
    thread::sleep(context.settle_delay);
    let more = capture_evidence(context, "02-more.png", overlay)?;
    ensure_evidence_changed(&selected, &more, "More did not change the overlay")?;
    record_step(
        report,
        &context.report_path,
        "more",
        foreground,
        Some(&more),
    )?;

    let record_point = match target {
        RecordTargetOption::Area => plan.record_area,
        RecordTargetOption::Window => plan.record_window,
    };
    let foreground = inject_mouse_click(overlay.handle, record_point)?;
    record_step(
        report,
        &context.report_path,
        match target {
            RecordTargetOption::Area => "record_area_click",
            RecordTargetOption::Window => "record_window_click",
        },
        foreground,
        None,
    )?;
    wait_for_window_gone(
        overlay.handle,
        context.timeout,
        match target {
            RecordTargetOption::Area => "Record area",
            RecordTargetOption::Window => "Record window",
        },
    )?;

    let active = wait_for_recording_state(context, "active recording", |state| {
        state.active
            && !state.starting
            && !state.stopping
            && state.target.as_deref() == Some(target.label())
            && state.progress_frame >= 10
            && state.progress_time_us > 0
    })?;
    let reported_source_bounds = active.target_bounds.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "active recording did not report physical source bounds",
        )
    })?;
    validate_recording_target_bounds(target, expected_source_bounds, reported_source_bounds)?;
    let reference_timestamp_seconds = active.progress_time_us as f64 / 1_000_000.0;
    let recording_reference = SystemCaptureBackend.capture(expected_source_bounds)?;
    if recording_reference.bounds != expected_source_bounds {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "recording reference captured {:?}, expected {expected_source_bounds:?}",
                recording_reference.bounds
            ),
        ));
    }
    let reference_path = context
        .session_root
        .join("screenshots")
        .join("recording-source-reference.png");
    recording_reference.save_png(&reference_path)?;
    let mut maximum_progress_frame = active.progress_frame;
    record_recording_state(report, &context.report_path, "recording", active.clone())?;
    let mut pending_window_phases = Vec::new();
    let mut window_fixture_seed = None;
    if let Some(fixture) = window_fixture.as_ref() {
        let initial_hold_after = active
            .progress_time_us
            .checked_add(WINDOW_PHASE_HOLD_US)
            .ok_or_else(|| io::Error::other("initial recording timestamp overflow"))?;
        let initial_held = wait_for_recording_state(context, "initial window hold", |state| {
            state.active
                && !state.paused
                && !state.stopping
                && state.progress_time_us >= initial_hold_after
        })?;
        let initial_held_bounds = initial_held.target_bounds.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "initial window hold lost physical source bounds",
            )
        })?;
        validate_recording_target_bounds(
            RecordTargetOption::Window,
            expected_source_bounds,
            initial_held_bounds,
        )?;
        maximum_progress_frame = maximum_progress_frame.max(initial_held.progress_frame);

        let moved = fixture.move_target()?;
        pending_window_phases.push(capture_recording_window_phase(
            context,
            report,
            "window_moved",
            "recording-window-moved",
            expected_source_bounds,
            moved,
        )?);
        let resized = fixture.resize_target()?;
        pending_window_phases.push(capture_recording_window_phase(
            context,
            report,
            "window_resized",
            "recording-window-resized",
            expected_source_bounds,
            resized,
        )?);
        let occluded = fixture.occlude_target()?;
        pending_window_phases.push(capture_recording_window_phase(
            context,
            report,
            "window_occluded",
            "recording-window-occluded",
            expected_source_bounds,
            occluded,
        )?);
        let minimized = fixture.minimize_target(context.timeout)?;
        pending_window_phases.push(capture_recording_window_phase(
            context,
            report,
            "window_minimized",
            "recording-window-minimized",
            expected_source_bounds,
            minimized,
        )?);
        maximum_progress_frame = pending_window_phases
            .iter()
            .fold(maximum_progress_frame, |maximum, phase| {
                maximum.max(phase.maximum_progress_frame)
            });
        fixture.hide_all()?;
        window_fixture_seed = Some(RecordingWindowFixtureReportSeed {
            process_id: fixture.process_id,
            target_title: fixture.target_title.clone(),
            initial_bounds: fixture.initial_bounds,
            moved_bounds: fixture.moved_bounds,
            resized_bounds: fixture.resized_bounds,
        });
    }

    context
        .interaction_commands
        .send_blocking(OverlayInteractionAcceptanceCommand::ShowRecordingSettings)
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "Record page command closed"))?;
    let controller = wait_for_owned_window_visible(controller.handle, context.timeout)?;
    focus_owned_window(controller, context.timeout)?;
    thread::sleep(context.settle_delay);
    let controls = recording_control_plan_for_window(controller.handle)?;
    let recording = capture_evidence(context, "03-recording.png", controller)?;
    record_step(
        report,
        &context.report_path,
        "record_page_visible",
        controller,
        Some(&recording),
    )?;

    let foreground = inject_mouse_click(controller.handle, controls.pause_or_resume)?;
    let paused = wait_for_recording_state(context, "paused recording", |state| {
        state.active && state.paused && !state.stopping
    })?;
    maximum_progress_frame = maximum_progress_frame.max(paused.progress_frame);
    record_recording_state(report, &context.report_path, "paused", paused)?;
    thread::sleep(context.settle_delay.max(PAUSE_STABILITY_INTERVAL));
    let paused_before = wait_for_recording_state(context, "settled paused recording", |state| {
        state.active && state.paused && !state.stopping
    })?;
    maximum_progress_frame = maximum_progress_frame.max(paused_before.progress_frame);
    record_recording_state(
        report,
        &context.report_path,
        "paused_stability_start",
        paused_before.clone(),
    )?;
    thread::sleep(PAUSE_STABILITY_INTERVAL);
    let paused_after = query_recording_state(context, context.timeout.min(Duration::from_secs(1)))?;
    validate_paused_progress(&paused_before, &paused_after)?;
    maximum_progress_frame = maximum_progress_frame.max(paused_after.progress_frame);
    record_recording_state(
        report,
        &context.report_path,
        "paused_stability_end",
        paused_after.clone(),
    )?;
    let paused_evidence = capture_evidence(
        context,
        "04-paused.png",
        guard_foreground(controller.handle)?,
    )?;
    record_step(
        report,
        &context.report_path,
        "pause_click",
        foreground,
        Some(&paused_evidence),
    )?;

    let foreground = inject_mouse_click(controller.handle, controls.pause_or_resume)?;
    let resumed = wait_for_recording_state(context, "resumed recording", |state| {
        state.active
            && !state.paused
            && !state.stopping
            && state.progress_frame > paused_after.progress_frame
            && state.progress_time_us > paused_after.progress_time_us
    })?;
    maximum_progress_frame = maximum_progress_frame.max(resumed.progress_frame);
    record_recording_state(report, &context.report_path, "resumed", resumed)?;
    thread::sleep(context.settle_delay);
    let resumed_evidence = capture_evidence(
        context,
        "05-resumed.png",
        guard_foreground(controller.handle)?,
    )?;
    record_step(
        report,
        &context.report_path,
        "resume_click",
        foreground,
        Some(&resumed_evidence),
    )?;

    let foreground = inject_mouse_click(controller.handle, controls.stop)?;
    record_step(report, &context.report_path, "stop_click", foreground, None)?;
    let stopping = wait_for_recording_state(context, "stopping recording", |state| state.stopping)?;
    maximum_progress_frame = maximum_progress_frame.max(stopping.progress_frame);
    record_recording_state(report, &context.report_path, "stopping", stopping)?;

    // Stopping may finish between two frames, so it is recorded from production state rather than
    // attaching that label to a screenshot which might already contain the Saved UI.
    let saved = wait_for_recording_state(context, "saved recording", recording_saved)?;
    maximum_progress_frame = maximum_progress_frame.max(saved.progress_frame);
    record_recording_state(report, &context.report_path, "saved", saved)?;
    thread::sleep(context.settle_delay);
    let saved_evidence = capture_evidence(
        context,
        "06-saved.png",
        guard_foreground(controller.handle)?,
    )?;
    record_step(
        report,
        &context.report_path,
        "saved_visible",
        guard_foreground(controller.handle)?,
        Some(&saved_evidence),
    )?;

    let fixture_cleaned_up = if let Some(fixture) = window_fixture.as_mut() {
        fixture.shutdown(context.timeout.min(Duration::from_secs(3)))?;
        true
    } else {
        false
    };

    let output = single_recording_output(&context.session_root.join("recordings"))?;
    let output_bytes = fs::metadata(&output)?.len();
    if output_bytes == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recorded MP4 is empty",
        ));
    }
    let capabilities = discover()?;
    let media = probe_media(capabilities.executable(), &output)?;
    validate_recorded_media(expected_source_bounds, &media)?;
    let (content, window_phases, timeline_sample_count) = if window_fixture_seed.is_some() {
        finalize_recording_window_timeline(
            capabilities.executable(),
            &output,
            &context.session_root,
            &recording_reference,
            pending_window_phases,
        )?
    } else {
        if !pending_window_phases.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "area recording unexpectedly produced window dynamic phases",
            ));
        }
        let decoded_path = context
            .session_root
            .join("screenshots")
            .join("recording-decoded-frame.png");
        extract_video_frame(
            capabilities.executable(),
            &output,
            reference_timestamp_seconds,
            &decoded_path,
        )?;
        let decoded_frame = CaptureFrame::open_png(&decoded_path)?;
        let content_comparison =
            validate_recording_frame_content(&recording_reference, &decoded_frame)?;
        (
            recording_content_report(
                "screenshots/recording-source-reference.png".to_owned(),
                "screenshots/recording-decoded-frame.png".to_owned(),
                reference_timestamp_seconds,
                content_comparison,
            ),
            Vec::new(),
            0,
        )
    };
    let window_dynamics = match window_fixture_seed {
        Some(seed) => {
            if window_phases.len() != 4 || !fixture_cleaned_up {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "window recording did not finish four dynamic phases and helper cleanup",
                ));
            }
            let source_bounds_fixed = window_phases
                .iter()
                .all(|phase| phase.reported_source_bounds == expected_source_bounds);
            if !source_bounds_fixed {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "window recording source bounds changed during a dynamic phase",
                ));
            }
            let mut fingerprints = Vec::with_capacity(window_phases.len() + 1);
            fingerprints.push(content.reference_fingerprint.as_str());
            fingerprints.extend(
                window_phases
                    .iter()
                    .map(|phase| phase.content.reference_fingerprint.as_str()),
            );
            validate_distinct_recording_phase_fingerprints(&fingerprints)?;
            Some(RecordingWindowDynamicsReport {
                fixture_process_id: seed.process_id,
                target_title: seed.target_title,
                initial_target_bounds: seed.initial_bounds,
                moved_target_bounds: seed.moved_bounds,
                resized_target_bounds: seed.resized_bounds,
                source_bounds_fixed,
                fixture_cleaned_up,
                timeline_frames_per_second: WINDOW_TIMELINE_FRAMES_PER_SECOND,
                timeline_sample_count,
                phases: window_phases,
            })
        }
        None => None,
    };
    let relative_output = output
        .strip_prefix(&context.session_root)
        .unwrap_or(&output)
        .to_string_lossy()
        .into_owned();
    report.recording = Some(RecordingReport {
        target: target.label(),
        requested_selection: drag.selection,
        source_bounds: expected_source_bounds,
        reported_source_bounds,
        window_title,
        output: relative_output,
        output_bytes,
        ffmpeg_version: capabilities.version().to_owned(),
        codec_name: media.codec_name,
        width: media.width,
        height: media.height,
        duration_seconds: media.duration_seconds,
        pause_observed: true,
        resume_observed: true,
        maximum_progress_frame,
        content,
        window_dynamics,
    });
    write_report(&context.report_path, report)
}

#[cfg(windows)]
fn query_recording_state(
    context: &WorkerContext,
    timeout: Duration,
) -> io::Result<OverlayInteractionRecordingState> {
    request_recording_state(&context.interaction_commands, timeout)
}

#[cfg(windows)]
/// Sends one snapshot request without a capacity wait and bounds the corresponding reply.
fn request_recording_state(
    commands: &async_channel::Sender<OverlayInteractionAcceptanceCommand>,
    timeout: Duration,
) -> io::Result<OverlayInteractionRecordingState> {
    let (reply_tx, reply_rx) = mpsc::sync_channel(1);
    commands
        .send_blocking(OverlayInteractionAcceptanceCommand::Snapshot(reply_tx))
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "recording state channel closed"))?;
    reply_rx.recv_timeout(timeout).map_err(|error| match error {
        RecvTimeoutError::Timeout => io::Error::new(
            io::ErrorKind::TimedOut,
            "recording state reply did not arrive",
        ),
        RecvTimeoutError::Disconnected => io::Error::new(
            io::ErrorKind::BrokenPipe,
            "recording state reply channel disconnected",
        ),
    })
}

#[cfg(windows)]
/// Requests the selection and Pin state observed by the production app entity.
fn query_capture_state(
    context: &WorkerContext,
    timeout: Duration,
) -> io::Result<OverlayInteractionCaptureState> {
    let (reply_tx, reply_rx) = mpsc::sync_channel(1);
    context
        .interaction_commands
        .send_blocking(OverlayInteractionAcceptanceCommand::CaptureSnapshot(
            reply_tx,
        ))
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "capture state channel closed"))?;
    reply_rx.recv_timeout(timeout).map_err(|error| match error {
        RecvTimeoutError::Timeout => io::Error::new(
            io::ErrorKind::TimedOut,
            "capture state reply did not arrive",
        ),
        RecvTimeoutError::Disconnected => io::Error::new(
            io::ErrorKind::BrokenPipe,
            "capture state reply channel disconnected",
        ),
    })
}

#[cfg(windows)]
/// Requests exact selection and Pin frames only when an action needs a content oracle.
fn query_capture_content(
    context: &WorkerContext,
    timeout: Duration,
) -> io::Result<OverlayInteractionCaptureContent> {
    let (reply_tx, reply_rx) = mpsc::sync_channel(1);
    context
        .interaction_commands
        .send_blocking(OverlayInteractionAcceptanceCommand::CaptureContent(
            reply_tx,
        ))
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "capture content channel closed"))?;
    reply_rx.recv_timeout(timeout).map_err(|error| match error {
        RecvTimeoutError::Timeout => io::Error::new(
            io::ErrorKind::TimedOut,
            "capture content reply did not arrive",
        ),
        RecvTimeoutError::Disconnected => io::Error::new(
            io::ErrorKind::BrokenPipe,
            "capture content reply channel disconnected",
        ),
    })
}

#[cfg(windows)]
/// Polls capture state through asynchronous export and Pin teardown transitions.
fn wait_for_capture_state(
    context: &WorkerContext,
    stage: &str,
    expected: impl Fn(&OverlayInteractionCaptureState) -> bool,
) -> io::Result<OverlayInteractionCaptureState> {
    let deadline = Instant::now() + context.timeout;
    let mut last_state = None;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "timed out waiting for {stage}; last state: {}",
                    last_state.as_deref().unwrap_or("no state reply received")
                ),
            ));
        }
        let state = match query_capture_state(context, remaining.min(Duration::from_secs(1))) {
            Ok(state) => state,
            Err(error) if error.kind() == io::ErrorKind::TimedOut && Instant::now() < deadline => {
                continue;
            }
            Err(error) => return Err(error),
        };
        if expected(&state) {
            return Ok(state);
        }
        last_state = Some(format!("{state:?}"));
        thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(windows)]
/// Polls process-local state while failing fast on the product's explicit recording errors.
fn wait_for_recording_state(
    context: &WorkerContext,
    stage: &str,
    expected: impl Fn(&OverlayInteractionRecordingState) -> bool,
) -> io::Result<OverlayInteractionRecordingState> {
    let deadline = Instant::now() + context.timeout;
    let mut last_status = None;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "timed out waiting for {stage}; last status: {}",
                    last_status.as_deref().unwrap_or("no state reply received")
                ),
            ));
        }
        let state = match query_recording_state(context, remaining.min(Duration::from_secs(1))) {
            Ok(state) => state,
            Err(error) if error.kind() == io::ErrorKind::TimedOut && Instant::now() < deadline => {
                continue;
            }
            Err(error) => return Err(error),
        };
        if expected(&state) {
            return Ok(state);
        }
        if recording_failed(&state) {
            return Err(io::Error::other(state.status));
        }
        last_status = Some(state.status.clone());
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "timed out waiting for {stage}; last status: {}",
                    state.status
                ),
            ));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn recording_failed(state: &OverlayInteractionRecordingState) -> bool {
    [
        "Recording is unavailable",
        "This FFmpeg build cannot",
        "Could not start screen recording",
        "Could not stop screen recording",
        "Could not change recording pause state",
        "Screen recording failed",
    ]
    .iter()
    .any(|prefix| state.status.starts_with(prefix))
}

fn recording_saved(state: &OverlayInteractionRecordingState) -> bool {
    !state.active
        && !state.starting
        && !state.stopping
        && state.status.starts_with("Screen recording saved to ")
}

fn record_recording_state(
    report: &mut AcceptanceReport,
    report_path: &Path,
    stage: &'static str,
    state: OverlayInteractionRecordingState,
) -> io::Result<()> {
    report
        .recording_states
        .push(RecordingStateReport::from_state(stage, state));
    write_report(report_path, report)
}

fn single_recording_output(directory: &Path) -> io::Result<PathBuf> {
    let mut outputs = fs::read_dir(directory)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"))
        })
        .collect::<Vec<_>>();
    outputs.sort();
    match outputs.as_slice() {
        [output] => Ok(output.clone()),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "expected exactly one finalized MP4, found {}",
                outputs.len()
            ),
        )),
    }
}

fn validate_recorded_media(source_bounds: PhysicalRect, media: &MediaMetadata) -> io::Result<()> {
    if media.codec_name != "h264" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("expected H.264 video, found {}", media.codec_name),
        ));
    }
    let expected_width = (source_bounds.width() + 1) & !1;
    let expected_height = (source_bounds.height() + 1) & !1;
    if media.width != expected_width || media.height != expected_height {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "recording is {}x{}, expected {}x{} from the selected source",
                media.width, media.height, expected_width, expected_height
            ),
        ));
    }
    Ok(())
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
fn wait_for_owned_window_visible(
    handle: *mut c_void,
    timeout: Duration,
) -> io::Result<NativeWindow> {
    let deadline = Instant::now() + timeout;
    loop {
        if unsafe { IsWindowVisible(handle) } != 0
            && let Ok(window) = owned_window(handle)
        {
            return Ok(window);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Record page did not become visible",
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
/// Waits for one overlay transition and keeps the triggering action in timeout diagnostics.
fn wait_for_window_gone(
    window: *mut c_void,
    timeout: Duration,
    completed_action: &str,
) -> io::Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        // SAFETY: both functions only query the borrowed HWND value.
        if unsafe { IsWindow(window) } == 0 || unsafe { IsWindowVisible(window) } == 0 {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("process window did not close after {completed_action}"),
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
            let dpi = unsafe { GetDpiForWindow(window) };
            if dpi == 0 {
                return 1;
            }
            search.windows.push(NativeWindow {
                handle: window,
                bounds: PhysicalRect {
                    left: rect.left,
                    top: rect.top,
                    right: rect.right,
                    bottom: rect.bottom,
                },
                dpi,
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
/// Waits for the unambiguous process-owned common Save dialog opened by the overlay.
fn wait_for_save_dialog(
    overlay: *mut c_void,
    controller: *mut c_void,
    timeout: Duration,
) -> io::Result<NativeWindow> {
    let deadline = Instant::now() + timeout;
    loop {
        let candidates = process_windows()?
            .into_iter()
            .filter(|window| window.handle != overlay && window.handle != controller)
            .filter(|window| window_class_name(window.handle).is_ok_and(|class| class == "#32770"))
            .filter(|window| owner_chain_contains(window.handle, overlay))
            .collect::<Vec<_>>();
        if candidates.len() > 1 {
            return Err(io::Error::other(
                "multiple owned Save dialogs appeared; input injection was aborted",
            ));
        }
        if let Some(dialog) = candidates.into_iter().next()
            && unsafe { GetForegroundWindow() } == dialog.handle
        {
            return Ok(dialog);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the owned Save dialog did not become the foreground window",
            ));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(windows)]
/// Finds the one standard Edit child that still contains GPUI's suggested filename.
fn save_file_name_edit(dialog: *mut c_void, suggested_name: &str) -> io::Result<NativeWindow> {
    struct Search {
        handles: Vec<*mut c_void>,
    }

    unsafe extern "system" fn callback(window: *mut c_void, parameter: LPARAM) -> BOOL {
        // SAFETY: EnumChildWindows returns the pointer supplied for this synchronous traversal.
        let search = unsafe { &mut *(parameter as *mut Search) };
        search.handles.push(window);
        1
    }

    let mut search = Search {
        handles: Vec::new(),
    };
    // SAFETY: callback only borrows search for the duration of this recursive child enumeration.
    unsafe { EnumChildWindows(dialog, Some(callback), &mut search as *mut Search as LPARAM) };
    let mut matches = search
        .handles
        .into_iter()
        .filter(|handle| window_class_name(*handle).is_ok_and(|class| class == "Edit"))
        .filter(|handle| window_text(*handle).is_ok_and(|text| text == suggested_name))
        .map(owned_window)
        .collect::<io::Result<Vec<_>>>()?;
    if matches.len() != 1 {
        return Err(io::Error::other(format!(
            "expected one Save filename edit containing {suggested_name:?}, found {}",
            matches.len()
        )));
    }
    Ok(matches.remove(0))
}

#[cfg(windows)]
fn window_class_name(handle: *mut c_void) -> io::Result<String> {
    let mut buffer = [0_u16; 256];
    // SAFETY: buffer is writable and handle is only queried.
    let length = unsafe { GetClassNameW(handle, buffer.as_mut_ptr(), buffer.len() as i32) };
    if length <= 0 {
        return Err(io::Error::last_os_error());
    }
    String::from_utf16(&buffer[..length as usize])
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(windows)]
fn window_text(handle: *mut c_void) -> io::Result<String> {
    // SAFETY: both calls only query the borrowed HWND and the second writes into owned storage.
    let length = unsafe { GetWindowTextLengthW(handle) };
    if length < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut buffer = vec![0_u16; length as usize + 1];
    let copied = unsafe { GetWindowTextW(handle, buffer.as_mut_ptr(), buffer.len() as i32) };
    if copied < 0 {
        return Err(io::Error::last_os_error());
    }
    String::from_utf16(&buffer[..copied as usize])
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(windows)]
/// Waits for the shell dialog's GUI thread to finish focusing the filename edit after a click.
fn wait_for_window_focus(
    dialog: *mut c_void,
    expected: *mut c_void,
    timeout: Duration,
) -> io::Result<()> {
    // SAFETY: the dialog is a verified live process-owned HWND and no process handle is opened.
    let thread_id = unsafe { GetWindowThreadProcessId(dialog, std::ptr::null_mut()) };
    if thread_id == 0 {
        return Err(io::Error::last_os_error());
    }
    let deadline = Instant::now() + timeout;
    loop {
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        // SAFETY: info is initialized writable storage and thread_id owns the live dialog.
        if unsafe { GetGUIThreadInfo(thread_id, &mut info) } != 0 && info.hwndFocus == expected {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Save filename edit did not receive keyboard focus",
            ));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(windows)]
/// Waits for injected Unicode input to be consumed before validating the common dialog field.
fn wait_for_window_text(handle: *mut c_void, expected: &str, timeout: Duration) -> io::Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        let observed = window_text(handle)?;
        if observed == expected {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("Save filename edit contains {observed:?}, expected {expected:?}"),
            ));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(windows)]
fn owner_chain_contains(mut window: *mut c_void, expected_owner: *mut c_void) -> bool {
    for _ in 0..16 {
        // SAFETY: GetWindow borrows the HWND and returns its current owner, if any.
        window = unsafe { GetWindow(window, GW_OWNER) };
        if window.is_null() {
            return false;
        }
        if window == expected_owner {
            return true;
        }
    }
    false
}

#[cfg(windows)]
/// Waits until one Pin is the only visible process window besides the hidden controller.
fn wait_for_single_visible_pin(
    controller: *mut c_void,
    timeout: Duration,
) -> io::Result<NativeWindow> {
    let deadline = Instant::now() + timeout;
    loop {
        let candidates = process_windows()?
            .into_iter()
            .filter(|window| window.handle != controller)
            .collect::<Vec<_>>();
        let last_count = candidates.len();
        if let Some(pin) = candidates.into_iter().next()
            && last_count == 1
            && unsafe { GetForegroundWindow() } == pin.handle
        {
            return Ok(pin);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "the selected Pin did not become the only foreground process window; last visible count: {last_count}"
                ),
            ));
        }
        thread::sleep(Duration::from_millis(25));
    }
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
    let dpi = unsafe { GetDpiForWindow(handle) };
    if dpi == 0 {
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
        dpi,
    })
}

#[cfg(windows)]
/// Maps a verified HWND's client rectangle into physical desktop coordinates for GPUI input.
fn client_bounds_for_window(handle: *mut c_void) -> io::Result<PhysicalRect> {
    owned_window(handle)?;
    let mut client = RECT::default();
    if unsafe { GetClientRect(handle, &mut client) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if client.right <= client.left || client.bottom <= client.top {
        return Err(io::Error::other("acceptance window client area is empty"));
    }
    let mut top_left = POINT {
        x: client.left,
        y: client.top,
    };
    let mut bottom_right = POINT {
        x: client.right,
        y: client.bottom,
    };
    // SAFETY: both points are initialized and the HWND remains process-owned and live.
    if unsafe { ClientToScreen(handle, &mut top_left) } == 0
        || unsafe { ClientToScreen(handle, &mut bottom_right) } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if bottom_right.x <= top_left.x || bottom_right.y <= top_left.y {
        return Err(io::Error::other(
            "acceptance window client bounds became invalid on screen",
        ));
    }
    Ok(PhysicalRect {
        left: top_left.x,
        top: top_left.y,
        right: bottom_right.x,
        bottom: bottom_right.y,
    })
}

#[cfg(windows)]
/// Re-measures the overlay client area immediately before deriving drag and toolbar points.
fn interaction_plan_for_window(handle: *mut c_void) -> io::Result<InteractionPlan> {
    let window = owned_window(handle)?;
    interaction_plan(
        client_bounds_for_window(handle)?,
        window.dpi as f32 / WINDOWS_BASE_DPI,
    )
}

#[cfg(windows)]
/// Re-reads the client origin and DPI immediately before clicking Record-page controls.
fn recording_control_plan_for_window(handle: *mut c_void) -> io::Result<RecordingControlPlan> {
    let window = owned_window(handle)?;
    let client = client_bounds_for_window(handle)?;
    recording_control_plan(
        PhysicalPoint {
            x: client.left,
            y: client.top,
        },
        client.width(),
        client.height(),
        window.dpi as f32 / WINDOWS_BASE_DPI,
    )
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
/// Sends one ordinary key press and guarantees a defensive key-up on partial injection.
fn inject_key(expected: *mut c_void, virtual_key: u16) -> io::Result<NativeWindow> {
    let foreground = guard_foreground(expected)?;
    let inputs = [
        keyboard_input(virtual_key, false),
        keyboard_input(virtual_key, true),
    ];
    let cleanup = [keyboard_input(virtual_key, true)];
    send_input_batch_with_cleanup(expected, &inputs, &cleanup)?;
    Ok(foreground)
}

#[cfg(windows)]
/// Selects all text in the focused native edit without relying on clipboard paste.
fn inject_select_all(expected: *mut c_void) -> io::Result<NativeWindow> {
    let foreground = guard_foreground(expected)?;
    let inputs = [
        keyboard_input(VK_CONTROL, false),
        keyboard_input(VK_A, false),
        keyboard_input(VK_A, true),
        keyboard_input(VK_CONTROL, true),
    ];
    let cleanup = [keyboard_input(VK_A, true), keyboard_input(VK_CONTROL, true)];
    send_input_batch_with_cleanup(expected, &inputs, &cleanup)?;
    Ok(foreground)
}

#[cfg(windows)]
/// Types UTF-16 code units directly so Save acceptance never borrows the system clipboard.
fn inject_unicode_text(expected: *mut c_void, text: &str) -> io::Result<NativeWindow> {
    let foreground = guard_foreground(expected)?;
    let code_units = text.encode_utf16().collect::<Vec<_>>();
    if code_units.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "injected Save path must not be empty",
        ));
    }
    let mut inputs = Vec::with_capacity(code_units.len() * 2);
    let mut cleanup = Vec::with_capacity(code_units.len());
    for code_unit in code_units {
        inputs.push(unicode_keyboard_input(code_unit, false));
        inputs.push(unicode_keyboard_input(code_unit, true));
        cleanup.push(unicode_keyboard_input(code_unit, true));
    }
    send_input_batch_with_cleanup(expected, &inputs, &cleanup)?;
    Ok(foreground)
}

#[cfg(windows)]
fn inject_mouse_click(expected: *mut c_void, point: PhysicalPoint) -> io::Result<NativeWindow> {
    let foreground = guard_foreground(expected)?;
    let desktop = virtual_desktop()?;
    send_input_batch(
        expected,
        &[absolute_mouse_input(point, MOUSEEVENTF_MOVE, desktop)],
    )?;
    guard_current_pointer_target(expected)?;
    // Recheck foreground and hit testing after the move, immediately before button-down.
    send_input_batch_with_cleanup(
        expected,
        &[
            mouse_button_input(MOUSEEVENTF_LEFTDOWN),
            mouse_button_input(MOUSEEVENTF_LEFTUP),
        ],
        &[mouse_button_input(MOUSEEVENTF_LEFTUP)],
    )?;
    Ok(foreground)
}

#[cfg(windows)]
/// Splits a drag into guarded moves and always releases the global mouse button afterward.
fn inject_mouse_drag(
    expected: *mut c_void,
    start: PhysicalPoint,
    end: PhysicalPoint,
    capture_bounds: PhysicalRect,
) -> io::Result<InjectedDrag> {
    let foreground = guard_foreground(expected)?;
    let desktop = virtual_desktop()?;
    send_input_batch(
        expected,
        &[absolute_mouse_input(start, MOUSEEVENTF_MOVE, desktop)],
    )?;
    let actual_start = guard_current_pointer_target(expected)?;
    send_input_batch_with_cleanup(
        expected,
        &[mouse_button_input(MOUSEEVENTF_LEFTDOWN)],
        &[mouse_button_input(MOUSEEVENTF_LEFTUP)],
    )?;

    let movement_result = (|| {
        for step in 1..=8 {
            guard_current_pointer_target(expected)?;
            let point = PhysicalPoint {
                x: start.x + (end.x - start.x) * step / 8,
                y: start.y + (end.y - start.y) * step / 8,
            };
            send_input_batch(
                expected,
                &[absolute_mouse_input(point, MOUSEEVENTF_MOVE, desktop)],
            )?;
        }
        guard_current_pointer_target(expected)
    })();
    // Button-up is cleanup as well as the intended action, so it must not depend on foreground.
    let release_result = send_input_unchecked(&[mouse_button_input(MOUSEEVENTF_LEFTUP)]);
    let actual_end = match (movement_result, release_result) {
        (Ok(point), Ok(())) => point,
        (Err(error), _) => return Err(error),
        (Ok(_), Err(error)) => return Err(error),
    };
    let screen_selection = PhysicalRect::new(actual_start, actual_end);
    Ok(InjectedDrag {
        foreground,
        selection: map_screen_selection_to_capture(
            screen_selection,
            client_bounds_for_window(expected)?,
            capture_bounds,
        )?,
    })
}

#[cfg(windows)]
/// Refuses a physical pointer action when another native window visually covers its target.
fn guard_pointer_target(expected: *mut c_void, point: PhysicalPoint) -> io::Result<()> {
    // SAFETY: WindowFromPoint only borrows the returned HWND, and IsChild only compares handles.
    let hit = unsafe {
        WindowFromPoint(POINT {
            x: point.x,
            y: point.y,
        })
    };
    if hit.is_null() || (hit != expected && unsafe { IsChild(expected, hit) } == 0) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "pointer target is covered by another native window; input injection was aborted",
        ));
    }
    Ok(())
}

#[cfg(windows)]
/// Reads the actual cursor point after SendInput rounding and verifies the window beneath it.
fn guard_current_pointer_target(expected: *mut c_void) -> io::Result<PhysicalPoint> {
    guard_foreground(expected)?;
    let mut point = POINT::default();
    // SAFETY: point is initialized writable storage for the process-global cursor coordinate.
    if unsafe { GetCursorPos(&mut point) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let point = PhysicalPoint {
        x: point.x,
        y: point.y,
    };
    guard_pointer_target(expected, point)?;
    Ok(point)
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
fn unicode_keyboard_input(code_unit: u16, key_up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wScan: code_unit,
                dwFlags: KEYEVENTF_UNICODE | if key_up { KEYEVENTF_KEYUP } else { 0 },
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
    send_input_batch_with_cleanup(expected, inputs, &[])
}

#[cfg(windows)]
/// Submits one guarded batch and releases action-specific keys if Windows accepts it partially.
fn send_input_batch_with_cleanup(
    expected: *mut c_void,
    inputs: &[INPUT],
    action_cleanup: &[INPUT],
) -> io::Result<()> {
    guard_foreground(expected)?;
    let result = send_input_unchecked(inputs);
    if result.is_ok() {
        return Ok(());
    }
    // Release buttons and modifiers defensively if Windows accepted only part of a batch.
    let mut cleanup = action_cleanup.to_vec();
    cleanup.extend([
        mouse_button_input(MOUSEEVENTF_LEFTUP),
        keyboard_input(VK_F24, true),
        keyboard_input(VK_MENU, true),
        keyboard_input(VK_CONTROL, true),
    ]);
    // Key-up and button-up cleanup must not depend on focus: leaving an accepted modifier or
    // mouse press held would be a larger global side effect than releasing it after focus moved.
    let _ = send_input_unchecked(&cleanup);
    result
}

#[cfg(windows)]
/// Sends release-safe input without a focus guard; callers use this only for cleanup after down.
fn send_input_unchecked(inputs: &[INPUT]) -> io::Result<()> {
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
        DEFAULT_OUTPUT_DIR, Options, RecordTargetOption, ensure_input_authorized,
        first_stable_recording_match, interaction_command_channel, interaction_plan,
        map_screen_selection_to_capture, normalize_axis, recording_control_plan, recording_failed,
        recording_saved, translated_rect, validate_distinct_recording_phase_fingerprints,
        validate_paused_progress, validate_recorded_media, validate_recording_target_bounds,
        validate_selection_geometry,
    };
    #[cfg(windows)]
    use super::{
        FixturePhaseState, panic_payload_message, parse_recording_window_fixture_arguments,
        recording_fixture_dynamic_bounds, request_recording_state, validate_fixture_phase_state,
        validate_recording_frame_content, validate_same_pixel_content,
    };
    use super::{MediaMetadata, OverlayInteractionRecordingState};
    use flash_shot::domain::geometry::{PhysicalPoint, PhysicalRect};
    #[cfg(windows)]
    use flash_shot::platform::capture::{CaptureFrame, PixelFormat};
    use std::{ffi::OsString, path::PathBuf, time::Duration};
    #[cfg(windows)]
    use std::{sync::Arc, time::Instant};

    fn arguments(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    fn recording_state(status: &str) -> OverlayInteractionRecordingState {
        OverlayInteractionRecordingState {
            active: false,
            starting: false,
            stopping: false,
            paused: false,
            target: None,
            target_bounds: None,
            progress_frame: 0,
            progress_time_us: 0,
            status: status.to_owned(),
        }
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
    fn interaction_command_sends_never_wait_for_receiver_capacity() {
        let (commands, _receiver) = interaction_command_channel();
        assert_eq!(commands.capacity(), None);
    }

    #[cfg(windows)]
    #[test]
    fn undrained_recording_state_request_respects_its_reply_timeout() {
        let (commands, _receiver) = interaction_command_channel();
        let started = Instant::now();
        let error = request_recording_state(&commands, Duration::from_millis(20)).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(1));
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
            "--record-target",
            "window",
        ]))
        .unwrap();
        assert_eq!(options.output_dir, PathBuf::from("evidence"));
        assert_eq!(options.timeout, Duration::from_secs(12));
        assert_eq!(options.settle_delay, Duration::from_millis(350));
        assert_eq!(options.record_target, Some(RecordTargetOption::Window));
        assert!(Options::parse_from(arguments(&["--allow-input", "--allow-input"])).is_err());
        assert!(Options::parse_from(arguments(&["--timeout-ms", "2999"])).is_err());
        assert!(Options::parse_from(arguments(&["--settle-ms", "5001"])).is_err());
        assert!(Options::parse_from(arguments(&["--record-target", "display"])).is_err());
        assert_eq!(
            Options::parse_from(arguments(&["--record-target", "area"]))
                .unwrap()
                .record_target,
            Some(RecordTargetOption::Area)
        );
        assert!(
            Options::parse_from(arguments(&[
                "--record-target",
                "area",
                "--record-target",
                "window"
            ]))
            .is_err()
        );
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
        for point in [
            plan.drag_start,
            plan.drag_end,
            plan.pin,
            plan.copy,
            plan.save,
            plan.more,
            plan.cancel,
            plan.record_area,
            plan.record_window,
        ] {
            assert!(bounds.contains(point), "{point:?}");
        }
        assert!(plan.drag_start.x < plan.drag_end.x);
        assert!(plan.drag_start.y < plan.drag_end.y);
        assert!(plan.pin.x < plan.copy.x);
        assert!(plan.copy.x < plan.save.x);
        assert!(plan.save.x < plan.more.x);
        assert!(plan.more.x < plan.cancel.x);
        assert!(plan.more.y > plan.drag_end.y);
        assert_eq!(plan.record_area.x, plan.record_window.x);
        assert!(plan.record_area.y < plan.record_window.y);
        assert!(plan.record_window.y < plan.more.y);
    }

    #[test]
    fn interaction_plan_honors_offset_client_bounds_across_dpi_scales() {
        let cases = [
            (
                PhysicalRect {
                    left: 100,
                    top: 200,
                    right: 1380,
                    bottom: 920,
                },
                1.0,
                PhysicalRect {
                    left: 382,
                    top: 344,
                    right: 970,
                    bottom: 560,
                },
            ),
            (
                PhysicalRect {
                    left: -1920,
                    top: 0,
                    right: 0,
                    bottom: 1080,
                },
                1.5,
                PhysicalRect {
                    left: -1498,
                    top: 216,
                    right: -614,
                    bottom: 540,
                },
            ),
            (
                PhysicalRect {
                    left: 40,
                    top: -1440,
                    right: 2600,
                    bottom: 0,
                },
                2.0,
                PhysicalRect {
                    left: 603,
                    top: -1152,
                    right: 1781,
                    bottom: -720,
                },
            ),
        ];

        for (bounds, scale, expected) in cases {
            let plan = interaction_plan(bounds, scale).unwrap();
            assert_eq!(PhysicalRect::new(plan.drag_start, plan.drag_end), expected);
            assert!(bounds.contains(plan.cancel));
        }
    }

    #[test]
    fn selection_oracle_allows_one_physical_pixel_per_edge() {
        let requested = PhysicalRect {
            left: -400,
            top: 100,
            right: 600,
            bottom: 700,
        };
        let rounded = PhysicalRect {
            left: -399,
            top: 99,
            right: 601,
            bottom: 700,
        };
        let wrong = PhysicalRect {
            left: -398,
            ..rounded
        };

        assert!(validate_selection_geometry(requested, rounded, "selection").is_ok());
        assert!(validate_selection_geometry(requested, wrong, "selection").is_err());
    }

    #[test]
    fn screen_pointer_path_accounts_for_borderless_client_origin() {
        let screen = PhysicalRect {
            left: 563,
            top: 284,
            right: 1741,
            bottom: 716,
        };
        let client = PhysicalRect {
            left: 0,
            top: -4,
            right: 2560,
            bottom: 1436,
        };
        let display = PhysicalRect {
            left: 0,
            top: 0,
            right: 2560,
            bottom: 1440,
        };

        assert_eq!(
            map_screen_selection_to_capture(screen, client, display).unwrap(),
            PhysicalRect {
                left: 563,
                top: 288,
                right: 1741,
                bottom: 720,
            }
        );
        assert!(
            map_screen_selection_to_capture(
                screen,
                PhysicalRect {
                    bottom: 1435,
                    ..client
                },
                display
            )
            .is_err()
        );
    }

    #[cfg(windows)]
    #[test]
    fn exact_content_oracle_ignores_bounds_and_stride_padding() {
        let source = CaptureFrame {
            bounds: PhysicalRect {
                left: 50,
                top: 60,
                right: 52,
                bottom: 62,
            },
            width: 2,
            height: 2,
            stride: 12,
            format: PixelFormat::Bgra8,
            pixels: Arc::from([
                0, 0, 0, 255, 255, 255, 255, 255, 9, 9, 9, 9, 20, 30, 40, 255, 200, 210, 220, 255,
                8, 8, 8, 8,
            ]),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 0,
        };
        let result = CaptureFrame {
            bounds: PhysicalRect {
                left: 0,
                top: 0,
                right: 2,
                bottom: 2,
            },
            stride: 8,
            pixels: Arc::from([
                0, 0, 0, 255, 255, 255, 255, 255, 20, 30, 40, 255, 200, 210, 220, 255,
            ]),
            ..source.clone()
        };

        let report = validate_same_pixel_content(&source, &result, "fixture").unwrap();
        assert!(report.exact_match);
        assert_eq!(report.source_fingerprint, report.result_fingerprint);

        let mut changed_pixels = result.pixels.to_vec();
        changed_pixels[4] = 254;
        let changed = CaptureFrame {
            pixels: Arc::from(changed_pixels),
            ..result
        };
        assert!(validate_same_pixel_content(&source, &changed, "fixture").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn recording_content_oracle_accepts_codec_noise_and_rejects_blank_frames() {
        let reference = CaptureFrame {
            bounds: PhysicalRect {
                left: 0,
                top: 0,
                right: 2,
                bottom: 2,
            },
            width: 2,
            height: 2,
            stride: 8,
            format: PixelFormat::Bgra8,
            pixels: Arc::from([
                0, 0, 0, 255, 255, 255, 255, 255, 20, 30, 40, 255, 200, 210, 220, 255,
            ]),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 0,
        };
        let noisy = CaptureFrame {
            pixels: Arc::from([
                2, 1, 3, 255, 252, 254, 253, 255, 22, 29, 42, 255, 197, 212, 218, 255,
            ]),
            ..reference.clone()
        };
        let blank = CaptureFrame {
            pixels: Arc::from([0_u8; 16]),
            ..reference.clone()
        };

        assert!(validate_recording_frame_content(&reference, &noisy).is_ok());
        assert!(validate_recording_frame_content(&reference, &blank).is_err());
    }

    #[test]
    fn recording_target_oracle_rejects_area_and_window_mismatches() {
        let expected = PhysicalRect {
            left: 10,
            top: 20,
            right: 650,
            bottom: 380,
        };
        let wrong = PhysicalRect {
            right: 649,
            ..expected
        };

        assert!(
            validate_recording_target_bounds(RecordTargetOption::Area, expected, expected).is_ok()
        );
        assert!(
            validate_recording_target_bounds(RecordTargetOption::Area, expected, wrong).is_err()
        );
        assert!(
            validate_recording_target_bounds(RecordTargetOption::Window, expected, wrong).is_err()
        );
    }

    #[test]
    fn recording_phase_fingerprints_must_identify_distinct_compositions() {
        assert!(validate_distinct_recording_phase_fingerprints(&["a", "b", "c"]).is_ok());
        assert!(validate_distinct_recording_phase_fingerprints(&["a", "b", "a"]).is_err());
    }

    #[test]
    fn recording_timeline_requires_consecutive_ordered_matches() {
        let errors = [50.0, 4.0, 40.0, 3.0, 2.0, 1.0, 80.0];
        assert_eq!(
            first_stable_recording_match(&errors, 0, 18.0, 2),
            Some((4, 5))
        );
        assert_eq!(first_stable_recording_match(&errors, 5, 18.0, 2), None);
        assert_eq!(first_stable_recording_match(&errors, 0, 18.0, 0), None);
    }

    #[cfg(windows)]
    #[test]
    fn window_fixture_child_geometry_separates_move_resize_and_state_rules() {
        let (token, initial) = parse_recording_window_fixture_arguments(arguments(&[
            "probe", "100", "200", "1100", "700",
        ]))
        .unwrap();
        assert_eq!(token, "probe");
        let (moved, resized) = recording_fixture_dynamic_bounds(initial).unwrap();
        assert_eq!(moved.width(), initial.width());
        assert_eq!(moved.height(), initial.height());
        assert!(moved.left > initial.left && moved.top > initial.top);
        assert_eq!((resized.left, resized.top), (initial.left, initial.top));
        assert!(resized.width() < initial.width());
        assert!(resized.height() < initial.height());

        let compact = PhysicalRect {
            left: 293,
            top: 181,
            right: 881,
            bottom: 397,
        };
        let (_, compact_resized) = recording_fixture_dynamic_bounds(compact).unwrap();
        assert_eq!(
            (compact_resized.width(), compact_resized.height()),
            (441, 162)
        );

        let moved_state = FixturePhaseState {
            target_bounds: moved,
            target_visible: true,
            target_minimized: false,
            backdrop_visible: true,
            occluder_visible: false,
        };
        assert!(
            validate_fixture_phase_state("moved", moved_state, Some(moved), false, false).is_ok()
        );
        assert!(
            validate_fixture_phase_state("moved", moved_state, Some(initial), false, false)
                .is_err()
        );
        assert!(
            parse_recording_window_fixture_arguments(arguments(&[
                "probe", "100", "200", "300", "250",
            ]))
            .is_err()
        );
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
    fn one_pixel_nudge_translates_every_edge_without_resizing() {
        let before = PhysicalRect {
            left: -15,
            top: 20,
            right: 185,
            bottom: 140,
        };
        let after = translated_rect(before, 1, 0).unwrap();

        assert_eq!(
            after,
            PhysicalRect {
                left: -14,
                top: 20,
                right: 186,
                bottom: 140,
            }
        );
        assert_eq!(after.width(), before.width());
        assert_eq!(after.height(), before.height());
        assert!(translated_rect(before, i32::MAX, 0).is_err());
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

    #[test]
    fn record_page_control_points_use_client_coordinates_and_dpi() {
        let unscaled =
            recording_control_plan(PhysicalPoint { x: 10, y: 20 }, 980, 760, 1.0).unwrap();
        assert_eq!(unscaled.stop, PhysicalPoint { x: 396, y: 446 });
        assert_eq!(unscaled.pause_or_resume, PhysicalPoint { x: 503, y: 446 });

        let plan =
            recording_control_plan(PhysicalPoint { x: 100, y: 200 }, 1470, 1140, 1.5).unwrap();

        assert_eq!(plan.stop, PhysicalPoint { x: 679, y: 839 });
        assert_eq!(plan.pause_or_resume, PhysicalPoint { x: 840, y: 839 });
        assert!(recording_control_plan(PhysicalPoint::default(), 600, 480, 1.0).is_err());
        assert!(recording_control_plan(PhysicalPoint::default(), 980, 760, 0.0).is_err());
    }

    #[test]
    fn recording_media_validation_accepts_the_even_padding_contract() {
        let media = MediaMetadata {
            codec_name: "h264".to_owned(),
            width: 642,
            height: 362,
            duration_seconds: 1.25,
        };
        let source = PhysicalRect {
            left: -200,
            top: 40,
            right: 441,
            bottom: 401,
        };
        assert!(validate_recorded_media(source, &media).is_ok());

        let wrong_size = MediaMetadata {
            width: 640,
            ..media
        };
        assert!(validate_recorded_media(source, &wrong_size).is_err());
    }

    #[test]
    fn recording_state_checks_distinguish_saved_output_from_failures() {
        assert!(recording_saved(&recording_state(
            "Screen recording saved to C:\\recordings\\clip.mp4"
        )));
        assert!(!recording_failed(&recording_state(
            "Screen recording saved to C:\\recordings\\clip.mp4"
        )));
        assert!(recording_failed(&recording_state(
            "Could not start screen recording: missing encoder"
        )));
        let mut pause_error = recording_state("Could not change recording pause state: denied");
        pause_error.active = true;
        assert!(recording_failed(&pause_error));
        let mut stop_error = recording_state("Could not stop screen recording: pipe closed");
        stop_error.active = true;
        assert!(recording_failed(&stop_error));
    }

    #[test]
    fn paused_progress_must_remain_stable_before_resume() {
        let mut before = recording_state("Paused window recording");
        before.active = true;
        before.paused = true;
        before.progress_frame = 42;
        before.progress_time_us = 1_400_000;
        let stable = before.clone();
        let mut advancing = before.clone();
        advancing.progress_frame += 1;
        advancing.progress_time_us += 33_333;

        assert!(validate_paused_progress(&before, &stable).is_ok());
        assert!(validate_paused_progress(&before, &advancing).is_err());
        let mut resumed = stable;
        resumed.paused = false;
        assert!(validate_paused_progress(&before, &resumed).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn panic_payloads_are_persistable_strings() {
        let borrowed: Box<dyn std::any::Any + Send> = Box::new("borrowed panic");
        let owned: Box<dyn std::any::Any + Send> = Box::new("owned panic".to_owned());
        let opaque: Box<dyn std::any::Any + Send> = Box::new(42_u32);

        assert_eq!(panic_payload_message(borrowed.as_ref()), "borrowed panic");
        assert_eq!(panic_payload_message(owned.as_ref()), "owned panic");
        assert_eq!(
            panic_payload_message(opaque.as_ref()),
            "non-string panic payload"
        );
    }
}
