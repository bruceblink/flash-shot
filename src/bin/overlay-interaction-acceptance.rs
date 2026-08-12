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
    domain::selection::{ResizeHandle, SelectionDrag},
    history::ScreenshotHistory,
    performance::PerformanceRecorder,
    platform::display::{DisplayInfo, DisplayProvider, SystemDisplayProvider},
    settings::UserSettings,
};
#[cfg(windows)]
use flash_shot::{
    platform::{
        clipboard::SystemClipboard,
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
use flash_shot::platform::capture::{
    CaptureBackend, CaptureFrame, PixelFormat, SystemCaptureBackend,
};
#[cfg(windows)]
use std::{
    ffi::c_void,
    mem::size_of,
    os::windows::process::CommandExt,
    panic::AssertUnwindSafe,
    ptr,
    sync::{
        atomic::{AtomicI32, AtomicUsize, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError},
    },
};
#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::{
        BeginPaint, ClientToScreen, CreateSolidBrush, DeleteObject, EndPaint, FillRect,
        InvalidateRect, PAINTSTRUCT, UpdateWindow,
    },
    System::{
        DataExchange::{
            CloseClipboard, GetClipboardData, GetClipboardSequenceNumber,
            IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW,
        },
        LibraryLoader::GetModuleHandleW,
        Memory::{GlobalLock, GlobalSize, GlobalUnlock},
        Ole::CF_DIB,
        Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
        Threading::GetCurrentProcessId,
    },
    UI::{
        HiDpi::{
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow,
            SetProcessDpiAwarenessContext,
        },
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT,
            KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN,
            MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT, SendInput,
            VK_A, VK_CONTROL, VK_ESCAPE, VK_F24, VK_LBUTTON, VK_MENU, VK_RETURN, VK_RIGHT, VK_S,
            VK_SHIFT, VK_SPACE,
        },
        WindowsAndMessaging::{
            BringWindowToTop, CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW,
            DestroyWindow, DispatchMessageW, EnumChildWindows, EnumWindows, FindWindowW,
            GUITHREADINFO, GW_OWNER, GWLP_USERDATA, GetClassNameW, GetClientRect, GetCursorPos,
            GetForegroundWindow, GetGUIThreadInfo, GetMessageW, GetSystemMetrics, GetWindow,
            GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
            GetWindowThreadProcessId, HTCAPTION, HWND_TOP, IsChild, IsIconic, IsWindow,
            IsWindowVisible, MOUSEWHEEL_ROUTING_MOUSE_POS, MSG, PostMessageW, PostQuitMessage,
            RegisterClassW, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
            SM_YVIRTUALSCREEN, SPI_GETMOUSEWHEELROUTING, SW_HIDE, SW_MINIMIZE, SW_RESTORE,
            SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, SendMessageW, SetCursorPos,
            SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowWindow, SwitchToThisWindow,
            SystemParametersInfoW, TranslateMessage, WM_CLOSE, WM_DESTROY, WM_ERASEBKGND,
            WM_MOUSEWHEEL, WM_NCHITTEST, WM_PAINT, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
            WS_POPUP, WS_VISIBLE, WindowFromPoint,
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
#[cfg(windows)]
const FOCUS_TITLEBAR_FALLBACK_DELAY: Duration = Duration::from_millis(250);
const WINDOWS_BASE_DPI: f32 = 96.0;
const SELECTION_EDGE_TOLERANCE: i32 = 1;
const PAUSE_STABILITY_INTERVAL: Duration = Duration::from_millis(300);
// Give DWM and GPUI one quiet interval after native windows close before the next desktop sample.
const DESKTOP_QUIESCENCE_SETTLE: Duration = Duration::from_millis(300);
const MAX_RECORDING_GRID_MAE: f64 = 18.0;
const WINDOW_TARGET_CHILD_MODE: &str = "--window-target-child";
const SCROLL_TARGET_CHILD_MODE: &str = "--scroll-target-child";
const CLIPBOARD_CONSUMER_CHILD_MODE: &str = "--clipboard-consumer-child";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const WINDOW_FIXTURE_CLASS: &str = "FlashShotRecordingWindowFixture";
const SCROLL_FIXTURE_CLASS: &str = "FlashShotScrollWindowFixture";
const WINDOW_PHASE_SETTLE_US: u64 = 300_000;
const WINDOW_PHASE_HOLD_US: u64 = 300_000;
const WINDOW_TIMELINE_FRAMES_PER_SECOND: u16 = 5;
const WINDOW_TIMELINE_STABLE_FRAMES: usize = 2;
const MIN_WINDOW_FIXTURE_WIDTH: i64 = 240;
const MIN_WINDOW_FIXTURE_HEIGHT: i64 = 120;
const NARROW_EDGE_SELECTION_WIDTH: f32 = 160.0;
const NARROW_EDGE_SELECTION_HEIGHT: f32 = 96.0;
const NARROW_EDGE_RIGHT_INSET: f32 = 18.0;
const NARROW_EDGE_BOTTOM_INSET: f32 = 12.0;
const NARROW_EDGE_ANNOTATION_WIDTH: f32 = 900.0;
const NARROW_EDGE_ANNOTATION_HEIGHT: f32 = 186.0;
const PIN_COEXIST_COUNT: usize = 3;
const PIN_COEXIST_SELECTION_WIDTH: f32 = 360.0;
const PIN_COEXIST_SELECTION_HEIGHT: f32 = 240.0;
const PIN_COEXIST_SELECTION_GAP: f32 = 80.0;
const PIN_COEXIST_SELECTION_TOP: f32 = 160.0;
const PIN_COEXIST_LAYOUT_GAP: i32 = 100;
const PIN_COEXIST_LAYOUT_MARGIN: i32 = 80;
const SELECTION_MOVE_DELTA: PhysicalPoint = PhysicalPoint { x: 120, y: 72 };
const SELECTION_RESIZE_DELTA: PhysicalPoint = PhysicalPoint { x: 144, y: -80 };
const SELECTION_SHIFT_RESIZE_DELTA: PhysicalPoint = PhysicalPoint { x: 120, y: -24 };
const SELECTION_ALT_RESIZE_DELTA: PhysicalPoint = PhysicalPoint { x: 80, y: -56 };
const SCROLL_SECONDARY_MENU_HEIGHT: f32 = 218.0;
const SCROLL_SECONDARY_MENU_ROW: f32 = 90.0;
const SCROLL_SECONDARY_MENU_SCROLL_ROW_WIDTH: f32 = 323.0;
const SCROLL_SECONDARY_MENU_CONTENT_RIGHT_INSET: f32 = 7.0;
const SCROLL_FIXTURE_SCROLL_STEP: i32 = 96;
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
#[cfg(windows)]
static SCROLL_FIXTURE_OFFSET: AtomicI32 = AtomicI32::new(0);

#[derive(Debug, Eq, PartialEq)]
struct Options {
    allow_input: bool,
    allow_system_clipboard: bool,
    copy_trigger: CopyTriggerOption,
    output_dir: PathBuf,
    timeout: Duration,
    settle_delay: Duration,
    capture_scenario: CaptureScenarioOption,
    scroll_export: ScrollExportOption,
    record_target: Option<RecordTargetOption>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum CaptureScenarioOption {
    #[default]
    Standard,
    NarrowEdge,
    PinsCoexist,
    SelectionTransform,
    ScrollRoundtrip,
}

impl CaptureScenarioOption {
    const fn workflow(self) -> &'static str {
        match self {
            Self::Standard => "capture",
            Self::NarrowEdge => "capture_narrow_edge",
            Self::PinsCoexist => "capture_pins_coexist",
            Self::SelectionTransform => "capture_selection_transform",
            Self::ScrollRoundtrip => "capture_scroll_roundtrip",
        }
    }

    const fn requires_100_percent_display(self) -> bool {
        matches!(
            self,
            Self::NarrowEdge | Self::PinsCoexist | Self::SelectionTransform | Self::ScrollRoundtrip
        )
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum ScrollExportOption {
    #[default]
    Cancel,
    Copy,
    Save,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum CopyTriggerOption {
    #[default]
    Toolbar,
    Enter,
}

impl CopyTriggerOption {
    /// Returns the stable report/CLI label for the input gesture used to finish Copy.
    const fn label(self) -> &'static str {
        match self {
            Self::Toolbar => "toolbar",
            Self::Enter => "enter",
        }
    }
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
            allow_system_clipboard: false,
            copy_trigger: CopyTriggerOption::Toolbar,
            output_dir: PathBuf::from(DEFAULT_OUTPUT_DIR),
            timeout: DEFAULT_TIMEOUT,
            settle_delay: DEFAULT_SETTLE_DELAY,
            capture_scenario: CaptureScenarioOption::Standard,
            scroll_export: ScrollExportOption::Cancel,
            record_target: None,
        };
        let mut arguments = arguments.into_iter();
        let mut output_seen = false;
        let mut timeout_seen = false;
        let mut settle_seen = false;
        let mut capture_scenario_seen = false;
        let mut scroll_export_seen = false;
        let mut record_target_seen = false;
        let mut copy_trigger_seen = false;
        while let Some(argument) = arguments.next() {
            let argument = argument
                .into_string()
                .map_err(|_| "acceptance options must be valid Unicode".to_owned())?;
            match argument.as_str() {
                "--allow-input" if !options.allow_input => options.allow_input = true,
                "--allow-input" => return Err("--allow-input may only be supplied once".to_owned()),
                "--allow-system-clipboard" if !options.allow_system_clipboard => {
                    options.allow_system_clipboard = true
                }
                "--allow-system-clipboard" => {
                    return Err("--allow-system-clipboard may only be supplied once".to_owned());
                }
                "--copy-trigger" if !copy_trigger_seen => {
                    let trigger = arguments
                        .next()
                        .ok_or_else(usage)?
                        .into_string()
                        .map_err(|_| "copy trigger must be valid Unicode".to_owned())?;
                    options.copy_trigger = match trigger.as_str() {
                        "toolbar" => CopyTriggerOption::Toolbar,
                        "enter" => CopyTriggerOption::Enter,
                        _ => return Err("copy trigger must be 'toolbar' or 'enter'".to_owned()),
                    };
                    copy_trigger_seen = true;
                }
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
                "--capture-scenario" if !capture_scenario_seen => {
                    let scenario = arguments
                        .next()
                        .ok_or_else(usage)?
                        .into_string()
                        .map_err(|_| "capture scenario must be valid Unicode".to_owned())?;
                    options.capture_scenario = match scenario.as_str() {
                        "narrow-edge" => CaptureScenarioOption::NarrowEdge,
                        "pins-coexist" => CaptureScenarioOption::PinsCoexist,
                        "selection-transform" => CaptureScenarioOption::SelectionTransform,
                        "scroll-roundtrip" => CaptureScenarioOption::ScrollRoundtrip,
                        _ => {
                            return Err(
                                "capture scenario must be 'narrow-edge', 'pins-coexist', 'selection-transform', or 'scroll-roundtrip'"
                                    .to_owned(),
                            );
                        }
                    };
                    capture_scenario_seen = true;
                }
                "--scroll-export" if !scroll_export_seen => {
                    let export = arguments
                        .next()
                        .ok_or_else(usage)?
                        .into_string()
                        .map_err(|_| "scroll export must be valid Unicode".to_owned())?;
                    options.scroll_export = match export.as_str() {
                        "cancel" => ScrollExportOption::Cancel,
                        "copy" => ScrollExportOption::Copy,
                        "save" => ScrollExportOption::Save,
                        _ => {
                            return Err(
                                "scroll export must be 'cancel', 'copy', or 'save'".to_owned()
                            );
                        }
                    };
                    scroll_export_seen = true;
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
                "--output-dir" | "--timeout-ms" | "--settle-ms" | "--capture-scenario"
                | "--scroll-export" | "--record-target" | "--copy-trigger" => {
                    return Err(format!("{argument} may only be supplied once"));
                }
                _ => return Err(usage()),
            }
        }
        if options.output_dir.as_os_str().is_empty() {
            return Err("output directory must not be empty".to_owned());
        }
        if options.capture_scenario != CaptureScenarioOption::Standard
            && options.record_target.is_some()
        {
            return Err("--capture-scenario cannot be combined with --record-target".to_owned());
        }
        if scroll_export_seen && options.capture_scenario != CaptureScenarioOption::ScrollRoundtrip
        {
            return Err(
                "--scroll-export may only be used with --capture-scenario scroll-roundtrip"
                    .to_owned(),
            );
        }
        if options.scroll_export == ScrollExportOption::Copy && !options.allow_system_clipboard {
            return Err(
                "scroll Copy changes the Windows clipboard; rerun with --allow-system-clipboard"
                    .to_owned(),
            );
        }
        let standard_capture = options.capture_scenario == CaptureScenarioOption::Standard
            && options.record_target.is_none()
            && !scroll_export_seen;
        if copy_trigger_seen && !standard_capture {
            return Err("--copy-trigger is only valid with standard capture".to_owned());
        }
        let standard_system_copy = options.capture_scenario == CaptureScenarioOption::Standard
            && options.record_target.is_none()
            && !scroll_export_seen;
        let scroll_system_copy = options.capture_scenario == CaptureScenarioOption::ScrollRoundtrip
            && options.scroll_export == ScrollExportOption::Copy;
        if options.allow_system_clipboard && !standard_system_copy && !scroll_system_copy {
            return Err(
                "--allow-system-clipboard is only valid with standard capture or scroll-roundtrip Copy"
                    .to_owned(),
            );
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
    "usage: overlay-interaction-acceptance --allow-input [--allow-system-clipboard] [--copy-trigger <toolbar|enter>] [--capture-scenario <narrow-edge|pins-coexist|selection-transform|scroll-roundtrip> [--scroll-export <cancel|copy|save> [--allow-system-clipboard]] | --record-target <area|window>] [--output-dir <path>] [--timeout-ms <3000-60000>] [--settle-ms <100-5000>]".to_owned()
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
    mark: PhysicalPoint,
    pin: PhysicalPoint,
    copy: PhysicalPoint,
    save: PhysicalPoint,
    more: PhysicalPoint,
    cancel: PhysicalPoint,
    record_area: PhysicalPoint,
    record_window: PhysicalPoint,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NarrowEdgeInteractionPlan {
    base: InteractionPlan,
    expanded_mark: PhysicalPoint,
    evidence_rest: PhysicalPoint,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SelectionTransformKind {
    Move,
    CornerResize,
    ShiftResize,
    AltResize,
}

impl SelectionTransformKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Move => "move",
            Self::CornerResize => "corner_resize",
            Self::ShiftResize => "shift_resize",
            Self::AltResize => "alt_resize",
        }
    }

    const fn modifiers(self) -> DragModifiers {
        match self {
            Self::Move | Self::CornerResize => DragModifiers::NONE,
            Self::ShiftResize => DragModifiers::SHIFT,
            Self::AltResize => DragModifiers::ALT,
        }
    }

    const fn delta(self) -> PhysicalPoint {
        match self {
            Self::Move => SELECTION_MOVE_DELTA,
            Self::CornerResize => SELECTION_RESIZE_DELTA,
            Self::ShiftResize => SELECTION_SHIFT_RESIZE_DELTA,
            Self::AltResize => SELECTION_ALT_RESIZE_DELTA,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct DragModifiers {
    shift: bool,
    alt: bool,
}

impl DragModifiers {
    const NONE: Self = Self {
        shift: false,
        alt: false,
    };
    const SHIFT: Self = Self {
        shift: true,
        alt: false,
    };
    const ALT: Self = Self {
        shift: false,
        alt: true,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SelectionTransformGesture {
    kind: SelectionTransformKind,
    start: PhysicalPoint,
    end: PhysicalPoint,
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

/// Validates an overlay client and returns its logical dimensions for deterministic input plans.
fn overlay_logical_size(bounds: PhysicalRect, scale: f32) -> io::Result<(f32, f32)> {
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
    Ok((width, height))
}

/// Maps a committed logical selection and its production toolbar into physical screen points.
fn interaction_plan_for_logical_selection(
    bounds: PhysicalRect,
    scale: f32,
    width: f32,
    height: f32,
    start: (f32, f32),
    end: (f32, f32),
) -> io::Result<InteractionPlan> {
    if start.0 < 0.0
        || start.1 < 0.0
        || end.0 <= start.0
        || end.1 <= start.1
        || end.0 > width
        || end.1 > height
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "interaction selection must be increasing and inside the overlay client",
        ));
    }

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
        mark: screen_point((toolbar_left + 33.0, toolbar_top + 25.0)),
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

/// Converts the standard proportional selection into physical screen points for SendInput.
fn interaction_plan(bounds: PhysicalRect, scale: f32) -> io::Result<InteractionPlan> {
    let (width, height) = overlay_logical_size(bounds, scale)?;
    interaction_plan_for_logical_selection(
        bounds,
        scale,
        width,
        height,
        (width * 0.22, height * 0.20),
        (width * 0.68, height * 0.50),
    )
}

/// Locates the production Scroll shot item in the expanded More menu using its fixed width rows.
fn scroll_shot_point_for_logical_selection(
    bounds: PhysicalRect,
    scale: f32,
    width: f32,
    height: f32,
    start: (f32, f32),
    end: (f32, f32),
) -> io::Result<PhysicalPoint> {
    if start.0 < 0.0
        || start.1 < 0.0
        || end.0 <= start.0
        || end.1 <= start.1
        || end.0 > width
        || end.1 > height
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "scroll-shot selection must be increasing and inside the overlay client",
        ));
    }
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

    // The 11 production secondary actions wrap into five natural-width rows at the 342px
    // toolbar width; Scroll shot is the leftmost item in the 323px third row (65px wide).
    let menu_above = toolbar_top - 8.0 - SCROLL_SECONDARY_MENU_HEIGHT >= 18.0
        || toolbar_top + toolbar_height + 8.0 + SCROLL_SECONDARY_MENU_HEIGHT > height - 96.0;
    let menu_top = if menu_above {
        toolbar_top - 8.0 - SCROLL_SECONDARY_MENU_HEIGHT
    } else {
        toolbar_top + toolbar_height + 8.0
    };
    let scroll_center = (
        toolbar_left + toolbar_width
            - SCROLL_SECONDARY_MENU_CONTENT_RIGHT_INSET
            - SCROLL_SECONDARY_MENU_SCROLL_ROW_WIDTH
            + 32.5,
        menu_top + SCROLL_SECONDARY_MENU_ROW + 18.0,
    );
    Ok(PhysicalPoint {
        x: bounds.left + (scroll_center.0 * scale).round() as i32,
        y: bounds.top + (scroll_center.1 * scale).round() as i32,
    })
}

/// Chooses a tall viewport that stays inside the fixture and above the scrolling controller.
fn scroll_roundtrip_interaction_plan(
    bounds: PhysicalRect,
    scale: f32,
) -> io::Result<InteractionPlan> {
    let (width, height) = overlay_logical_size(bounds, scale)?;
    if height < 740.0 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "scroll roundtrip acceptance requires at least 740 logical pixels of height",
        ));
    }
    let start = (width * 0.16, (height * 0.12).max(120.0));
    let end = (width * 0.74, (start.1 + 380.0).min(height - 180.0));
    interaction_plan_for_logical_selection(bounds, scale, width, height, start, end)
}

const fn rect_contains_rect(outer: PhysicalRect, inner: PhysicalRect) -> bool {
    outer.left <= inner.left
        && outer.top <= inner.top
        && outer.right >= inner.right
        && outer.bottom >= inner.bottom
}

const fn rects_overlap(first: PhysicalRect, second: PhysicalRect) -> bool {
    first.left < second.right
        && first.right > second.left
        && first.top < second.bottom
        && first.bottom > second.top
}

/// Accepts cleanup only after every manual-scroll worker flag and overlay surface is idle.
fn scroll_roundtrip_cleanup_complete(state: &OverlayInteractionCaptureState) -> bool {
    state.overlay_count == 0
        && state.pinned_count == 0
        && !state.more_actions_visible
        && !state.annotation_controls_visible
        && state.manual_scroll_state == "idle"
        && state.manual_scroll_frame_count == 0
        && !state.manual_scroll_can_finish
        && !state.manual_scroll_capture_in_flight
        && !state.manual_scroll_auto_capture_pending
        && state.manual_scroll_selection.is_none()
        && !state.capture_teardown_pending
        && state.capture_preflight_ready
        && matches!(
            state.session_state.as_str(),
            "idle" | "completed" | "cancelled"
        )
}

/// Places a 160x96 selection at the bottom-right edge and predicts the relocated Mark control.
fn narrow_edge_interaction_plan(
    bounds: PhysicalRect,
    scale: f32,
) -> io::Result<NarrowEdgeInteractionPlan> {
    if (scale - 1.0).abs() > 0.001 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "narrow-edge acceptance requires a 100%-scaled overlay",
        ));
    }
    let (width, height) = overlay_logical_size(bounds, scale)?;
    if width < NARROW_EDGE_ANNOTATION_WIDTH + NARROW_EDGE_RIGHT_INSET * 2.0 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "narrow-edge acceptance requires the wide annotation dock layout",
        ));
    }
    let end = (
        width - NARROW_EDGE_RIGHT_INSET,
        height - NARROW_EDGE_BOTTOM_INSET,
    );
    let start = (
        end.0 - NARROW_EDGE_SELECTION_WIDTH,
        end.1 - NARROW_EDGE_SELECTION_HEIGHT,
    );
    let base = interaction_plan_for_logical_selection(bounds, scale, width, height, start, end)?;

    // The production wide dock is 900x186 with the 342px action row above its tools.
    let annotation_left_min = NARROW_EDGE_RIGHT_INSET;
    let annotation_left_limit =
        (width - NARROW_EDGE_RIGHT_INSET - NARROW_EDGE_ANNOTATION_WIDTH).max(annotation_left_min);
    let annotation_left =
        (end.0 - NARROW_EDGE_ANNOTATION_WIDTH).clamp(annotation_left_min, annotation_left_limit);
    let annotation_top = start.1 - 12.0 - NARROW_EDGE_ANNOTATION_HEIGHT;
    if annotation_top < NARROW_EDGE_RIGHT_INSET {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "narrow-edge annotation dock does not fit above the selection",
        ));
    }
    let screen_point = |point: (f32, f32)| PhysicalPoint {
        x: bounds.left + (point.0 * scale).round() as i32,
        y: bounds.top + (point.1 * scale).round() as i32,
    };
    let expanded_action_left = annotation_left + NARROW_EDGE_ANNOTATION_WIDTH - 342.0;
    Ok(NarrowEdgeInteractionPlan {
        base,
        expanded_mark: screen_point((expanded_action_left + 33.0, annotation_top + 25.0)),
        // Keep the always-visible magnifier away from the edge toolbars being reviewed.
        evidence_rest: screen_point((24.0, 24.0)),
    })
}

/// Places three compact source selections across the upper desktop so their Pins fit side by side.
fn pin_coexist_interaction_plan(
    bounds: PhysicalRect,
    scale: f32,
    index: usize,
) -> io::Result<InteractionPlan> {
    if (scale - 1.0).abs() > 0.001 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Pin coexistence acceptance requires a 100%-scaled overlay",
        ));
    }
    if index >= PIN_COEXIST_COUNT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Pin coexistence selection index is out of range",
        ));
    }
    let (width, height) = overlay_logical_size(bounds, scale)?;
    let group_width = PIN_COEXIST_SELECTION_WIDTH * PIN_COEXIST_COUNT as f32
        + PIN_COEXIST_SELECTION_GAP * (PIN_COEXIST_COUNT - 1) as f32;
    let group_left = (width - group_width) / 2.0;
    let start = (
        group_left + index as f32 * (PIN_COEXIST_SELECTION_WIDTH + PIN_COEXIST_SELECTION_GAP),
        PIN_COEXIST_SELECTION_TOP,
    );
    let end = (
        start.0 + PIN_COEXIST_SELECTION_WIDTH,
        start.1 + PIN_COEXIST_SELECTION_HEIGHT,
    );
    if group_left < 18.0 || end.1 + 96.0 > height {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "display is too small for three compact Pin selections",
        ));
    }
    interaction_plan_for_logical_selection(bounds, scale, width, height, start, end)
}

/// Builds one physical-pixel gesture with enough room to avoid desktop-edge clamping.
fn selection_transform_gesture(
    selection: PhysicalRect,
    capture_bounds: PhysicalRect,
    kind: SelectionTransformKind,
) -> io::Result<SelectionTransformGesture> {
    let start = if kind == SelectionTransformKind::Move {
        PhysicalPoint {
            x: selection.left + selection.width() as i32 / 2,
            y: selection.top + selection.height() as i32 / 2,
        }
    } else {
        PhysicalPoint {
            x: selection.right,
            y: selection.bottom,
        }
    };
    let delta = kind.delta();
    let end = PhysicalPoint {
        x: start
            .x
            .checked_add(delta.x)
            .ok_or_else(|| io::Error::other("selection gesture X overflowed"))?,
        y: start
            .y
            .checked_add(delta.y)
            .ok_or_else(|| io::Error::other("selection gesture Y overflowed"))?,
    };
    if !capture_bounds.contains(start) || !capture_bounds.contains(end) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "{} gesture from {start:?} to {end:?} does not fit inside {capture_bounds:?}",
                kind.label()
            ),
        ));
    }
    Ok(SelectionTransformGesture { kind, start, end })
}

/// Applies the production selection model to the actual cursor endpoints observed after SendInput.
fn expected_selection_transform(
    selection: PhysicalRect,
    start: PhysicalPoint,
    end: PhysicalPoint,
    capture_bounds: PhysicalRect,
    kind: SelectionTransformKind,
) -> io::Result<PhysicalRect> {
    let mut drag = SelectionDrag::default();
    match kind {
        SelectionTransformKind::Move => {
            drag.begin_move(selection, start);
            drag.update_move(end, capture_bounds);
        }
        SelectionTransformKind::CornerResize => {
            drag.begin_resize(selection, ResizeHandle::BottomRight);
            drag.update(end);
        }
        SelectionTransformKind::ShiftResize => {
            drag.begin_resize(selection, ResizeHandle::BottomRight);
            drag.update_with_aspect_ratio(end, capture_bounds);
        }
        SelectionTransformKind::AltResize => {
            drag.begin_resize(selection, ResizeHandle::BottomRight);
            drag.update_from_center(end, capture_bounds, false);
        }
    }
    drag.selection().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} gesture did not produce a selection", kind.label()),
        )
    })
}

/// Checks the production Shift-resize ratio with a one-pixel allowance for integer rounding.
fn selection_aspect_ratio_preserved(before: PhysicalRect, after: PhysicalRect) -> bool {
    if before.width() == 0 || before.height() == 0 || after.width() == 0 || after.height() == 0 {
        return false;
    }
    let expected_height =
        u64::from(after.width()) * u64::from(before.height()) / u64::from(before.width());
    (i64::from(after.height()) - expected_height as i64).abs() <= 1
}

/// Checks the production Alt-resize center with the model's integer rounding tolerance.
fn selection_center_preserved(before: PhysicalRect, after: PhysicalRect) -> bool {
    let before_center = (
        i64::from(before.left) + i64::from(before.width() / 2),
        i64::from(before.top) + i64::from(before.height() / 2),
    );
    let after_center = (
        i64::from(after.left) + i64::from(after.width() / 2),
        i64::from(after.top) + i64::from(after.height() / 2),
    );
    (before_center.0 - after_center.0).abs() <= 1 && (before_center.1 - after_center.1).abs() <= 1
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

/// Maps one actual cursor position without normalizing away its drag direction.
fn map_screen_point_to_capture(
    point: PhysicalPoint,
    client_bounds: PhysicalRect,
    capture_bounds: PhysicalRect,
) -> io::Result<PhysicalPoint> {
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
    Ok(PhysicalPoint {
        x: point
            .x
            .checked_add(
                capture_bounds
                    .left
                    .checked_sub(client_bounds.left)
                    .ok_or_else(|| {
                        io::Error::other("overlay client-to-display X offset overflowed")
                    })?,
            )
            .ok_or_else(|| io::Error::other("screen-to-capture X mapping overflowed"))?,
        y: point
            .y
            .checked_add(
                capture_bounds
                    .top
                    .checked_sub(client_bounds.top)
                    .ok_or_else(|| {
                        io::Error::other("overlay client-to-display Y offset overflowed")
                    })?,
            )
            .ok_or_else(|| io::Error::other("screen-to-capture Y mapping overflowed"))?,
    })
}

/// Converts a capture pixel back to the screen coordinate consumed by SendInput.
fn map_capture_point_to_screen(
    point: PhysicalPoint,
    client_bounds: PhysicalRect,
    capture_bounds: PhysicalRect,
) -> io::Result<PhysicalPoint> {
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
    Ok(PhysicalPoint {
        x: point
            .x
            .checked_add(
                client_bounds
                    .left
                    .checked_sub(capture_bounds.left)
                    .ok_or_else(|| {
                        io::Error::other("display-to-overlay client X offset overflowed")
                    })?,
            )
            .ok_or_else(|| io::Error::other("capture-to-screen X mapping overflowed"))?,
        y: point
            .y
            .checked_add(
                client_bounds
                    .top
                    .checked_sub(capture_bounds.top)
                    .ok_or_else(|| {
                        io::Error::other("display-to-overlay client Y offset overflowed")
                    })?,
            )
            .ok_or_else(|| io::Error::other("capture-to-screen Y mapping overflowed"))?,
    })
}

/// Rejects SendInput coordinates that drift beyond the measured physical-pixel tolerance.
fn validate_point_geometry(
    requested: PhysicalPoint,
    actual: PhysicalPoint,
    label: &str,
) -> io::Result<()> {
    let delta_x = (i64::from(requested.x) - i64::from(actual.x)).abs();
    let delta_y = (i64::from(requested.y) - i64::from(actual.y)).abs();
    if delta_x <= i64::from(SELECTION_EDGE_TOLERANCE)
        && delta_y <= i64::from(SELECTION_EDGE_TOLERANCE)
    {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{label} reached {actual:?}, requested {requested:?} (maximum tolerance: {SELECTION_EDGE_TOLERANCE}px)"
            ),
        ))
    }
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
    narrow_edge: Option<NarrowEdgeReport>,
    pins_coexist: Option<PinsCoexistReport>,
    selection_transform: Option<SelectionTransformReport>,
    scroll_roundtrip: Option<ScrollRoundtripReport>,
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

#[derive(Clone, Copy, serde::Serialize)]
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
struct NarrowEdgeReport {
    controller_client_bounds: PhysicalRect,
    controller_logical_width: u32,
    controller_logical_height: u32,
    requested_selection: PhysicalRect,
    committed_selection: PhysicalRect,
    selection_content: NarrowEdgeContentReport,
    more_opened: bool,
    more_closed: bool,
    annotation_opened: bool,
    annotation_closed: bool,
    cleanup: CleanupReport,
}

#[derive(serde::Serialize)]
struct NarrowEdgeContentReport {
    bounds: PhysicalRect,
    width: u32,
    height: u32,
    fingerprint: String,
    luma_min: u8,
    luma_max: u8,
}

#[derive(serde::Serialize)]
struct PinsCoexistReport {
    pins: Vec<PinReport>,
    arranged_windows: Vec<WindowReport>,
    pointer_drag: WindowDragReport,
    requested_capture_selection: PhysicalRect,
    committed_capture_selection: PhysicalRect,
    windows_during_capture: Vec<WindowReport>,
    windows_after_cancel: Vec<WindowReport>,
    sources_unchanged_during_capture: bool,
    sources_unchanged_after_cancel: bool,
    pins_during_capture: usize,
    pins_after_cancel: usize,
    closed_with_escape: usize,
    cleanup: CleanupReport,
}

#[derive(serde::Serialize)]
struct SelectionTransformReport {
    initial_requested_selection: PhysicalRect,
    initial_selection: PhysicalRect,
    gestures: Vec<SelectionTransformGestureReport>,
    cleanup: CleanupReport,
}

#[derive(serde::Serialize)]
struct SelectionTransformGestureReport {
    gesture: &'static str,
    shift: bool,
    alt: bool,
    before: PhysicalRect,
    pointer_start: PhysicalPoint,
    pointer_end: PhysicalPoint,
    expected: PhysicalRect,
    committed: PhysicalRect,
    geometry_matches: bool,
    size_preserved: Option<bool>,
    opposite_corner_fixed: Option<bool>,
    aspect_ratio_preserved: Option<bool>,
    center_preserved: Option<bool>,
}

#[derive(serde::Serialize)]
struct ScrollRoundtripReport {
    fixture_process_id: u32,
    fixture_window: WindowReport,
    wheel_routing: u32,
    requested_selection: PhysicalRect,
    initial_selection: PhysicalRect,
    scroll_control: WindowReport,
    ready_status: String,
    initial_frame: ScrollFrameReport,
    auto_capture_status: String,
    second_frame: ScrollFrameReport,
    finish_status: String,
    stitched_selection: PhysicalRect,
    stitched_height_increased: bool,
    export: ScrollExportReport,
    manual_scroll_cleanup: ManualScrollCleanupReport,
    cleanup: CleanupReport,
}

#[derive(serde::Serialize)]
struct ManualScrollCleanupReport {
    state: String,
    frame_count: usize,
    can_finish: bool,
    capture_in_flight: bool,
    auto_capture_pending: bool,
    selection: Option<PhysicalRect>,
    more_actions_visible: bool,
    annotation_controls_visible: bool,
}

#[derive(serde::Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum ScrollExportReport {
    Cancel,
    Copy {
        clipboard_sequence_before: u32,
        clipboard_sequence_after: u32,
        clipboard_sequence_changed: bool,
        png_format_available: bool,
        dib_format_available: bool,
        copied_bounds: PhysicalRect,
        width: u32,
        height: u32,
        png_path: String,
        png_bytes: usize,
        dib_bytes: usize,
        png_content: ExactPixelMatchReport,
        dib_content: ExactPixelMatchReport,
        consumer_image_content: Box<ExactPixelMatchReport>,
        timing_clock: &'static str,
        timing_boundary: &'static str,
        input_to_consumer_readable_ms: f64,
        consumer_result_path: String,
    },
    Save {
        path: String,
        width: u32,
        height: u32,
        bytes: u64,
        content: ExactPixelMatchReport,
    },
}

#[derive(serde::Serialize)]
struct ScrollFrameReport {
    bounds: PhysicalRect,
    width: u32,
    height: u32,
    screenshot: String,
    fingerprint: String,
}

#[derive(serde::Serialize)]
struct WindowDragReport {
    handle: usize,
    before: PhysicalRect,
    after: PhysicalRect,
    pointer_start: PhysicalPoint,
    pointer_end: PhysicalPoint,
    expected_delta_x: i32,
    expected_delta_y: i32,
    actual_delta_x: i32,
    actual_delta_y: i32,
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
    cancelled_dialog_verified: bool,
    selection_restored_after_cancel: bool,
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
    trigger: &'static str,
    action: &'static str,
    read_mechanism: &'static str,
    requested_selection: PhysicalRect,
    selection: PhysicalRect,
    copied_bounds: PhysicalRect,
    width: u32,
    height: u32,
    clipboard_sequence_before: u32,
    clipboard_sequence_after: u32,
    clipboard_sequence_changed: bool,
    timing_clock: &'static str,
    timing_boundary: &'static str,
    input_to_consumer_readable_ms: Option<f64>,
    sink: &'static str,
    png_format_available: bool,
    dib_format_available: bool,
    png_path: Option<String>,
    dib_path: Option<String>,
    consumer_image_path: Option<String>,
    png_bytes: Option<usize>,
    dib_bytes: Option<usize>,
    png_content: Option<ExactPixelMatchReport>,
    dib_content: Option<ExactPixelMatchReport>,
    consumer_image_content: ExactPixelMatchReport,
    consumer_process_id: Option<u32>,
    consumer_result_path: Option<String>,
    consumer_ready_before_click: bool,
    consumer_observing_before_click: bool,
    consumer_cleaned_up: bool,
    single_export_verified: bool,
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
    capture_teardown_pending: bool,
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
#[derive(Clone, Copy)]
struct InjectedSelectionTransform {
    start: PhysicalPoint,
    end: PhysicalPoint,
}

#[cfg(windows)]
#[derive(Clone, Copy)]
struct InjectedPointerDrag {
    foreground: NativeWindow,
    start: PhysicalPoint,
    end: PhysicalPoint,
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
struct ScrollWindowFixture {
    child: process::Child,
    process_group: ProcessGroup,
    process_id: u32,
    target: HWND,
    target_bounds: PhysicalRect,
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
    copy_results: Option<Receiver<CaptureFrame>>,
    use_system_clipboard: bool,
    copy_trigger: CopyTriggerOption,
    timeout: Duration,
    settle_delay: Duration,
    capture_scenario: CaptureScenarioOption,
    scroll_export: ScrollExportOption,
    record_target: Option<RecordTargetOption>,
}

#[cfg(windows)]
struct ClipboardConsumer {
    child: process::Child,
    process_group: ProcessGroup,
    process_id: u32,
    ready_path: PathBuf,
    observing_path: PathBuf,
    start_path: PathBuf,
    result_path: PathBuf,
    stopped: bool,
}

#[cfg(windows)]
#[derive(serde::Deserialize, serde::Serialize)]
struct ClipboardConsumerResult {
    previous_sequence: u32,
    observed_sequence: u32,
    png_path: String,
    dib_path: String,
    consumer_image_path: String,
    png_bytes: usize,
    dib_bytes: usize,
    consumer_read_qpc_ticks: u64,
}

fn main() {
    #[cfg(windows)]
    if std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new(CLIPBOARD_CONSUMER_CHILD_MODE))
    {
        if let Err(error) = run_clipboard_consumer_child(std::env::args_os().skip(2)) {
            eprintln!("clipboard consumer failed: {error}");
            process::exit(1);
        }
        return;
    }
    #[cfg(windows)]
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new(WINDOW_TARGET_CHILD_MODE))
    {
        if let Err(error) = run_recording_window_fixture_child(std::env::args_os().skip(2)) {
            eprintln!("recording window fixture failed: {error}");
            process::exit(1);
        }
        return;
    }
    #[cfg(windows)]
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new(SCROLL_TARGET_CHILD_MODE))
    {
        if let Err(error) = run_scroll_fixture_child(std::env::args_os().skip(2)) {
            eprintln!("scroll fixture failed: {error}");
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
    if options.capture_scenario.requires_100_percent_display()
        && (display.dpi_x != 96
            || display.dpi_y != 96
            || (display.scale_factor - 1.0).abs() > 0.001)
    {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "{} acceptance requires one 100%-scaled display, found {}x{} DPI at scale {}",
                options.capture_scenario.workflow(),
                display.dpi_x,
                display.dpi_y,
                display.scale_factor
            ),
        )
        .into());
    }
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
    let uses_system_clipboard = options.allow_system_clipboard;
    let app_copy_results = (!uses_system_clipboard).then_some(copy_result_tx);
    let worker_copy_results = (!uses_system_clipboard).then_some(copy_result_rx);
    let (window_width, window_height) = match (options.record_target, options.capture_scenario) {
        (Some(_), _) => (980.0, 760.0),
        (None, CaptureScenarioOption::NarrowEdge) => (420.0, 420.0),
        (
            None,
            CaptureScenarioOption::Standard
            | CaptureScenarioOption::PinsCoexist
            | CaptureScenarioOption::SelectionTransform
            | CaptureScenarioOption::ScrollRoundtrip,
        ) => (520.0, 640.0),
    };

    let worker_context = WorkerContext {
        session_root,
        report_path,
        display,
        shortcut_readiness: shortcut_ready_rx,
        interaction_commands: interaction_tx,
        copy_results: worker_copy_results,
        use_system_clipboard: uses_system_clipboard,
        copy_trigger: options.copy_trigger,
        timeout: options.timeout,
        settle_delay: options.settle_delay,
        capture_scenario: options.capture_scenario,
        scroll_export: options.scroll_export,
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
            copy_results: app_copy_results,
        },
    )?;
    Err(io::Error::other("GPUI exited before the interaction worker completed").into())
}

#[cfg(windows)]
/// Creates the persisted report before the worker can inject input or panic.
fn initial_report(context: &WorkerContext) -> AcceptanceReport {
    AcceptanceReport {
        // Increment when the machine-readable report shape changes; cleanup now exposes
        // deferred native teardown so downstream evidence readers can distinguish hidden
        // windows from fully quiescent capture state.
        schema_version: 13,
        test: "overlay_interaction_acceptance",
        workflow: context.record_target.map_or_else(
            || context.capture_scenario.workflow(),
            RecordTargetOption::workflow,
        ),
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
        narrow_edge: None,
        pins_coexist: None,
        selection_transform: None,
        scroll_roundtrip: None,
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
/// Runs a single disposable window whose deterministic content advances on real wheel input.
fn run_scroll_fixture_child(arguments: impl IntoIterator<Item = OsString>) -> io::Result<()> {
    let (token, bounds) = parse_recording_window_fixture_arguments(arguments)?;
    if unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) } == 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(5) {
            return Err(error);
        }
    }
    LIVE_FIXTURE_WINDOWS.store(0, Ordering::Release);
    SCROLL_FIXTURE_OFFSET.store(0, Ordering::Release);
    let class_name = wide_null(SCROLL_FIXTURE_CLASS);
    let instance = unsafe { GetModuleHandleW(ptr::null()) };
    if instance.is_null() {
        return Err(io::Error::last_os_error());
    }
    let window_class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(scroll_fixture_window_proc),
        hInstance: instance,
        lpszClassName: class_name.as_ptr(),
        ..Default::default()
    };
    if unsafe { RegisterClassW(&window_class) } == 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(1410) {
            return Err(error);
        }
    }
    let title = wide_null(&scroll_fixture_title(&token));
    let window = unsafe {
        CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_POPUP | WS_VISIBLE,
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
    unsafe { UpdateWindow(window) };

    let mut message = MSG::default();
    loop {
        let result = unsafe { GetMessageW(&mut message, ptr::null_mut(), 0, 0) };
        if result == -1 {
            let error = io::Error::last_os_error();
            if unsafe { IsWindow(window) } != 0 {
                unsafe { DestroyWindow(window) };
            }
            return Err(error);
        }
        if result == 0 {
            break;
        }
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    if unsafe { IsWindow(window) } != 0 {
        unsafe { DestroyWindow(window) };
    }
    Ok(())
}

#[cfg(windows)]
/// Waits without creating windows, snapshots all production image formats, and publishes atomically.
fn run_clipboard_consumer_child(arguments: impl IntoIterator<Item = OsString>) -> io::Result<()> {
    let mut arguments = arguments.into_iter();
    let timeout_ms = arguments
        .next()
        .and_then(|value| value.into_string().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid clipboard timeout"))?;
    let ready_path = PathBuf::from(arguments.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "clipboard ready path is missing",
        )
    })?);
    let observing_path = PathBuf::from(arguments.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "clipboard observing path is missing",
        )
    })?);
    let start_path = PathBuf::from(arguments.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "clipboard start path is missing",
        )
    })?);
    let result_path = PathBuf::from(arguments.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "clipboard result path is missing",
        )
    })?);
    let artifact_dir = PathBuf::from(arguments.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "clipboard artifact directory is missing",
        )
    })?);
    if arguments.next().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "clipboard consumer received unexpected arguments",
        ));
    }
    fs::create_dir_all(&artifact_dir)?;
    fs::write(&ready_path, b"ready")?;
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let previous_sequence = loop {
        if start_path.is_file() {
            let marker = fs::read_to_string(&start_path)?;
            match marker.trim().parse::<u32>() {
                Ok(sequence) => break sequence,
                Err(_) if marker.trim().is_empty() && Instant::now() < deadline => {}
                Err(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "clipboard start marker did not contain a valid sequence",
                    ));
                }
            }
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "clipboard consumer was ready but never armed",
            ));
        }
        thread::sleep(Duration::from_millis(5));
    };
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "clipboard consumer arm deadline expired",
        ));
    }
    // This marker is written only after the baseline is parsed and immediately before the
    // monotonic clipboard wait begins. The parent uses it to distinguish "process started"
    // from "consumer is actually observing the next export".
    fs::write(&observing_path, b"observing")?;
    let (observed_sequence, consumer_image, png, dib) =
        wait_for_system_clipboard_image_change(previous_sequence, remaining)?;
    let consumer_read_qpc_ticks = qpc_ticks()?;
    let png_path = artifact_dir.join("registered-png.png");
    let dib_path = artifact_dir.join("cf-dib.bin");
    let consumer_image_path = artifact_dir.join("consumer-image.png");
    fs::write(&png_path, &png)?;
    fs::write(&dib_path, &dib)?;
    consumer_image.save_png(&consumer_image_path)?;
    let result = ClipboardConsumerResult {
        previous_sequence,
        observed_sequence,
        png_path: png_path.to_string_lossy().into_owned(),
        dib_path: dib_path.to_string_lossy().into_owned(),
        consumer_image_path: consumer_image_path.to_string_lossy().into_owned(),
        png_bytes: png.len(),
        dib_bytes: dib.len(),
        consumer_read_qpc_ticks,
    };
    let temporary = result_path.with_extension("json.tmp");
    fs::write(
        &temporary,
        serde_json::to_vec_pretty(&result).map_err(io::Error::other)?,
    )?;
    fs::rename(temporary, result_path)
}

#[cfg(windows)]
/// Publishes the clipboard baseline atomically so the child never observes a partial marker.
fn write_clipboard_start_marker(path: &Path, sequence: u32) -> io::Result<()> {
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, sequence.to_string().as_bytes())?;
    fs::rename(temporary, path)
}

#[cfg(windows)]
fn scroll_fixture_title(token: &str) -> String {
    format!("Flash Shot Scroll Fixture {token}")
}

#[cfg(windows)]
unsafe extern "system" fn scroll_fixture_window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_PAINT => {
            paint_scroll_fixture_window(window);
            0
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam >> 16) & 0xFFFF) as u16 as i16 as i32;
            let steps = delta / 120;
            if steps != 0 {
                // Negative wheel notches move the document down so the next viewport overlaps
                // the lower portion of the previous one, matching the production scroll helper.
                let offset = -steps * SCROLL_FIXTURE_SCROLL_STEP;
                SCROLL_FIXTURE_OFFSET.fetch_add(offset, Ordering::AcqRel);
                unsafe {
                    InvalidateRect(window, ptr::null(), 1);
                    UpdateWindow(window);
                }
            }
            0
        }
        WM_ERASEBKGND => 1,
        WM_CLOSE => {
            unsafe { DestroyWindow(window) };
            0
        }
        WM_DESTROY => {
            if LIVE_FIXTURE_WINDOWS.fetch_sub(1, Ordering::AcqRel) == 1 {
                unsafe { PostQuitMessage(0) };
            }
            0
        }
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

#[cfg(windows)]
fn paint_scroll_fixture_window(window: HWND) {
    let mut paint = PAINTSTRUCT::default();
    let device = unsafe { BeginPaint(window, &mut paint) };
    if device.is_null() {
        return;
    }
    let mut client = RECT::default();
    if unsafe { GetClientRect(window, &mut client) } != 0 {
        const ROW_HEIGHT: i32 = 48;
        const COLUMN_WIDTH: i32 = 128;
        let offset = SCROLL_FIXTURE_OFFSET.load(Ordering::Acquire);
        let row_count = (client.bottom - client.top + ROW_HEIGHT - 1) / ROW_HEIGHT;
        let column_count = (client.right - client.left + COLUMN_WIDTH - 1) / COLUMN_WIDTH;
        for row in 0..row_count {
            let content_row = (row * ROW_HEIGHT + offset).div_euclid(ROW_HEIGHT);
            for column in 0..column_count {
                let cell = RECT {
                    left: client.left + column * COLUMN_WIDTH,
                    top: client.top + row * ROW_HEIGHT,
                    right: (client.left + (column + 1) * COLUMN_WIDTH).min(client.right),
                    bottom: (client.top + (row + 1) * ROW_HEIGHT).min(client.bottom),
                };
                let brush = unsafe { CreateSolidBrush(scroll_fixture_color(content_row, column)) };
                if !brush.is_null() {
                    unsafe {
                        FillRect(device, &cell, brush);
                        DeleteObject(brush);
                    }
                }
            }
        }
    }
    unsafe { EndPaint(window, &paint) };
}

#[cfg(windows)]
fn scroll_fixture_color(row: i32, column: i32) -> u32 {
    const COLORS: [u32; 12] = [
        0x001C4E80, 0x00B23A48, 0x002C8A63, 0x00C47B2D, 0x005C3FA6, 0x008E5A2A, 0x002D728F,
        0x00A53F72, 0x003E8C4A, 0x00C04F2A, 0x004D5D9A, 0x009B6A24,
    ];
    let index = row
        .wrapping_mul(7)
        .wrapping_add(column.wrapping_mul(3))
        .rem_euclid(COLORS.len() as i32) as usize;
    COLORS[index]
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
            let cleanup = terminate_process_group_bounded(
                &process_group,
                &mut child,
                Duration::from_millis(500),
            );
            return Err(io::Error::other(format!(
                "recording fixture could not join its Job Object ({error}); cleanup={cleanup:?}"
            )));
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
                let cleanup = terminate_process_group_bounded(
                    &process_group,
                    &mut child,
                    Duration::from_millis(500),
                );
                return Err(io::Error::other(format!(
                    "recording fixture did not become ready ({error}); cleanup={cleanup:?}"
                )));
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
                let cleanup = terminate_process_group_bounded(
                    &process_group,
                    &mut child,
                    Duration::from_millis(500),
                );
                return Err(io::Error::other(format!(
                    "fixture {label} bounds are {observed:?}, expected {initial_bounds:?}; cleanup={cleanup:?}"
                )));
            }
        }
        if unsafe { IsWindowVisible(target) } == 0
            || unsafe { IsWindowVisible(backdrop) } == 0
            || unsafe { IsWindowVisible(occluder) } != 0
        {
            let cleanup = terminate_process_group_bounded(
                &process_group,
                &mut child,
                Duration::from_millis(500),
            );
            return Err(io::Error::other(format!(
                "fixture windows did not start in target/backdrop visible, occluder hidden state; cleanup={cleanup:?}"
            )));
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
                let cleanup = terminate_process_group_bounded(
                    &self.process_group,
                    &mut self.child,
                    Duration::from_millis(500),
                );
                self.stopped = true;
                return Err(io::Error::other(format!(
                    "recording fixture required forced process cleanup ({cleanup:?})"
                )));
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
}

#[cfg(windows)]
impl Drop for RecordingWindowFixture {
    fn drop(&mut self) {
        if !self.stopped {
            let _ = terminate_process_group_bounded(
                &self.process_group,
                &mut self.child,
                Duration::from_millis(500),
            );
            self.stopped = true;
        }
    }
}

#[cfg(windows)]
impl ScrollWindowFixture {
    /// Starts the deterministic wheel target in a separate process so the capture area contains
    /// no acceptance-process controls when the production auto-capture hides its own controller.
    fn launch(target_bounds: PhysicalRect, timeout: Duration) -> io::Result<Self> {
        validate_recording_fixture_bounds(target_bounds)?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let token = format!("{}-{timestamp}", process::id());
        let title = scroll_fixture_title(&token);
        let process_group = ProcessGroup::create()?;
        let mut child = process::Command::new(std::env::current_exe()?)
            .arg(SCROLL_TARGET_CHILD_MODE)
            .arg(&token)
            .arg(target_bounds.left.to_string())
            .arg(target_bounds.top.to_string())
            .arg(target_bounds.right.to_string())
            .arg(target_bounds.bottom.to_string())
            .stdin(process::Stdio::null())
            .stdout(process::Stdio::null())
            .stderr(process::Stdio::inherit())
            .spawn()?;
        if let Err(error) = process_group.assign(&child) {
            let cleanup = terminate_process_group_bounded(
                &process_group,
                &mut child,
                Duration::from_millis(500),
            );
            return Err(io::Error::other(format!(
                "scroll fixture could not join its Job Object ({error}); cleanup={cleanup:?}"
            )));
        }
        let process_id = child.id();
        let target = wait_for_scroll_fixture_window(&mut child, process_id, &title, timeout)?;
        let observed = external_window_bounds(target, process_id)?;
        if observed != target_bounds || unsafe { IsWindowVisible(target) } == 0 {
            let cleanup = terminate_process_group_bounded(
                &process_group,
                &mut child,
                Duration::from_millis(500),
            );
            return Err(io::Error::other(format!(
                "scroll fixture target is bounds={observed:?}, visible={}, expected {target_bounds:?}; cleanup={cleanup:?}",
                unsafe { IsWindowVisible(target) } != 0
            )));
        }
        Ok(Self {
            child,
            process_group,
            process_id,
            target,
            target_bounds,
            stopped: false,
        })
    }

    fn report(&self) -> io::Result<WindowReport> {
        let bounds = external_window_bounds(self.target, self.process_id)?;
        if bounds != self.target_bounds {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "scroll fixture moved to {bounds:?}, expected {:?}",
                    self.target_bounds
                ),
            ));
        }
        let dpi = unsafe { GetDpiForWindow(self.target) };
        if dpi == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(WindowReport {
            handle: self.target as usize,
            bounds,
            dpi,
        })
    }

    /// Closes the fixture on its own GUI thread and bounds forced cleanup if it stops responding.
    fn shutdown(&mut self, timeout: Duration) -> io::Result<()> {
        if self.stopped {
            return Ok(());
        }
        if !self.target.is_null() && unsafe { IsWindow(self.target) } != 0 {
            unsafe { PostMessageW(self.target, WM_CLOSE, 0, 0) };
        }
        let deadline = Instant::now() + timeout;
        loop {
            if self.child.try_wait()?.is_some() {
                self.stopped = true;
                return Ok(());
            }
            if Instant::now() >= deadline {
                let cleanup = terminate_process_group_bounded(
                    &self.process_group,
                    &mut self.child,
                    Duration::from_millis(500),
                );
                self.stopped = true;
                return Err(io::Error::other(format!(
                    "scroll fixture required forced process cleanup ({cleanup:?})"
                )));
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
}

#[cfg(windows)]
impl Drop for ScrollWindowFixture {
    fn drop(&mut self) {
        if !self.stopped {
            let _ = terminate_process_group_bounded(
                &self.process_group,
                &mut self.child,
                Duration::from_millis(500),
            );
            self.stopped = true;
        }
    }
}

#[cfg(windows)]
/// Terminates an acceptance child within a fixed budget and reaps it before returning.
fn terminate_process_group_bounded(
    process_group: &ProcessGroup,
    child: &mut process::Child,
    timeout: Duration,
) -> io::Result<()> {
    let terminate_error = process_group.terminate().err();
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait()? {
            Some(_) => return Ok(()),
            None if Instant::now() >= deadline => break,
            None => thread::sleep(Duration::from_millis(10)),
        }
    }
    let kill_error = child.kill().err();
    let reap_deadline = Instant::now() + Duration::from_millis(250);
    while Instant::now() < reap_deadline {
        if child.try_wait()?.is_some() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(10));
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!(
            "acceptance child could not be reaped (terminate={terminate_error:?}, kill={kill_error:?})"
        ),
    ))
}

#[cfg(windows)]
impl ClipboardConsumer {
    /// Starts a no-window child in a Job Object and proves it reached the wait loop before input.
    fn launch(session_root: &Path, timeout: Duration) -> io::Result<Self> {
        let consumer_root = session_root.join("clipboard-consumer");
        fs::create_dir_all(&consumer_root)?;
        let ready_path = consumer_root.join("ready");
        let observing_path = consumer_root.join("observing");
        let result_path = consumer_root.join("result.json");
        let start_path = consumer_root.join("start");
        let artifact_dir = consumer_root.join("artifacts");
        for path in [&ready_path, &observing_path, &start_path, &result_path] {
            if path.exists() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("clipboard consumer path already exists: {}", path.display()),
                ));
            }
        }
        let process_group = ProcessGroup::create()?;
        let mut child = process::Command::new(std::env::current_exe()?)
            .arg(CLIPBOARD_CONSUMER_CHILD_MODE)
            .arg(timeout.as_millis().to_string())
            .arg(&ready_path)
            .arg(&observing_path)
            .arg(&start_path)
            .arg(&result_path)
            .arg(&artifact_dir)
            .stdin(process::Stdio::null())
            .stdout(process::Stdio::null())
            .stderr(process::Stdio::inherit())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()?;
        if let Err(error) = process_group.assign(&child) {
            let cleanup = terminate_process_group_bounded(
                &process_group,
                &mut child,
                Duration::from_millis(500),
            );
            return Err(io::Error::other(format!(
                "clipboard consumer could not join its Job Object ({error}); cleanup={cleanup:?}"
            )));
        }
        let process_id = child.id();
        let deadline = Instant::now() + timeout;
        loop {
            if ready_path.is_file() {
                break;
            }
            if let Some(status) = child.try_wait()? {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    format!("clipboard consumer exited before ready: {status}"),
                ));
            }
            if Instant::now() >= deadline {
                let cleanup = terminate_process_group_bounded(
                    &process_group,
                    &mut child,
                    Duration::from_millis(500),
                );
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("clipboard consumer did not become ready (cleanup={cleanup:?})"),
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
        Ok(Self {
            child,
            process_group,
            process_id,
            ready_path,
            observing_path,
            start_path,
            result_path,
            stopped: false,
        })
    }

    /// Captures the post-ready baseline and arms the child with an atomic marker.
    fn arm(&self) -> io::Result<u32> {
        // SAFETY: this call only reads the process-global clipboard change counter.
        let sequence = unsafe { GetClipboardSequenceNumber() };
        write_clipboard_start_marker(&self.start_path, sequence)?;
        // Reject a mutation that happened while the marker was being published; callers then
        // retry the whole consumer setup instead of attributing an unrelated change to Copy.
        // SAFETY: this call only reads the process-global clipboard change counter.
        let confirmed = unsafe { GetClipboardSequenceNumber() };
        if confirmed != sequence {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "system clipboard changed while arming consumer ({sequence} -> {confirmed})"
                ),
            ));
        }
        Ok(sequence)
    }

    /// Terminates and polls the child without an unbounded `wait`, preserving the runner watchdog.
    fn terminate_bounded(&mut self, timeout: Duration) -> io::Result<()> {
        let terminate_error = self.process_group.terminate().err();
        let deadline = Instant::now() + timeout;
        loop {
            match self.child.try_wait()? {
                Some(_) => {
                    self.stopped = true;
                    return Ok(());
                }
                None if Instant::now() >= deadline => break,
                None => thread::sleep(Duration::from_millis(10)),
            }
        }
        let kill_error = self.child.kill().err();
        let reap_deadline = Instant::now() + Duration::from_millis(250);
        while Instant::now() < reap_deadline {
            if self.child.try_wait()?.is_some() {
                self.stopped = true;
                return Ok(());
            }
            thread::sleep(Duration::from_millis(10));
        }
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!(
                "clipboard consumer could not be reaped (terminate={terminate_error:?}, kill={kill_error:?})"
            ),
        ))
    }

    /// Waits for the isolated result, reaps the child, and rejects partial or duplicate output.
    fn wait_result(&mut self, timeout: Duration) -> io::Result<ClipboardConsumerResult> {
        let deadline = Instant::now() + timeout;
        loop {
            if self.result_path.is_file() {
                let result = serde_json::from_slice(&fs::read(&self.result_path)?)
                    .map_err(io::Error::other)?;
                // The result is written before process exit. Reap with the same deadline so a
                // child that hangs after publishing cannot suspend the entire acceptance run.
                loop {
                    if let Some(status) = self.child.try_wait()? {
                        self.stopped = true;
                        if !status.success() {
                            return Err(io::Error::other(format!(
                                "clipboard consumer exited with {status}"
                            )));
                        }
                        return Ok(result);
                    }
                    if Instant::now() >= deadline {
                        let cleanup = self.terminate_bounded(Duration::from_millis(500));
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            format!(
                                "clipboard consumer did not exit after publishing a result (cleanup={cleanup:?})"
                            ),
                        ));
                    }
                    thread::sleep(Duration::from_millis(10));
                }
            }
            if let Some(status) = self.child.try_wait()? {
                self.stopped = true;
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    format!("clipboard consumer exited without a result: {status}"),
                ));
            }
            if Instant::now() >= deadline {
                let cleanup = self.terminate_bounded(Duration::from_millis(500));
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("clipboard consumer did not publish a result (cleanup={cleanup:?})"),
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(windows)]
impl Drop for ClipboardConsumer {
    fn drop(&mut self) {
        if !self.stopped {
            let _ = self.terminate_bounded(Duration::from_millis(500));
        }
    }
}

#[cfg(windows)]
fn wait_for_scroll_fixture_window(
    child: &mut process::Child,
    process_id: u32,
    title: &str,
    timeout: Duration,
) -> io::Result<HWND> {
    let wide_title = wide_null(title);
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Err(io::Error::other(format!(
                "scroll fixture exited before its window was ready: {status}"
            )));
        }
        let window = unsafe { FindWindowW(ptr::null(), wide_title.as_ptr()) };
        if !window.is_null() && external_window_bounds(window, process_id).is_ok() {
            return Ok(window);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "scroll fixture window did not become ready",
            ));
        }
        thread::sleep(Duration::from_millis(25));
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

    let outcome = match (context.record_target, context.capture_scenario) {
        (Some(_), _) => execute_recording_interactions(context, report),
        (None, CaptureScenarioOption::NarrowEdge) => {
            execute_narrow_edge_interactions(context, report)
        }
        (None, CaptureScenarioOption::PinsCoexist) => {
            execute_pins_coexist_interactions(context, report)
        }
        (None, CaptureScenarioOption::SelectionTransform) => {
            execute_selection_transform_interactions(context, report)
        }
        (None, CaptureScenarioOption::ScrollRoundtrip) => {
            execute_scroll_roundtrip_interactions(context, report)
        }
        (None, CaptureScenarioOption::Standard) => execute_capture_interactions(context, report),
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
/// Exercises the real minimum Settings window and the edge-placement More/Mark toolbars.
fn execute_narrow_edge_interactions(
    context: &WorkerContext,
    report: &mut AcceptanceReport,
) -> io::Result<()> {
    let controller = wait_for_controller(context.timeout)?;
    focus_owned_window(controller, context.timeout)?;
    let controller_client = client_bounds_for_window(controller.handle)?;
    let controller_scale = controller.dpi as f32 / WINDOWS_BASE_DPI;
    let controller_logical_width =
        (controller_client.width() as f32 / controller_scale).round() as u32;
    let controller_logical_height =
        (controller_client.height() as f32 / controller_scale).round() as u32;
    if controller.dpi != 96 || controller_logical_width != 420 || controller_logical_height != 420 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "minimum Settings client is {controller_logical_width}x{controller_logical_height} at {} DPI, expected 420x420 at 96 DPI",
                controller.dpi
            ),
        ));
    }
    report.controller_window = Some(controller.report());
    let controller_rest = PhysicalPoint {
        x: controller_client.left + controller_client.width() as i32 / 2,
        y: controller_client.top + 20,
    };
    let foreground = inject_mouse_move(controller.handle, controller_rest)?;
    thread::sleep(context.settle_delay);
    let minimum_settings = capture_evidence(context, "00-min-settings.png", controller)?;
    record_step(
        report,
        &context.report_path,
        "minimum_settings_ready",
        foreground,
        Some(&minimum_settings),
    )?;

    let foreground = inject_capture_shortcut(controller.handle)?;
    record_step(
        report,
        &context.report_path,
        "narrow_capture_shortcut",
        foreground,
        None,
    )?;
    let overlay = wait_for_overlay(
        controller.handle,
        context.display.physical_bounds,
        context.timeout,
    )?;
    // Settings is hidden by an asynchronous production callback. Do not drag until it is gone,
    // otherwise the source frame can contain the controller or a stale native dialog.
    wait_for_window_gone(
        controller.handle,
        context.timeout,
        "capture overlay hides Settings",
    )?;
    focus_owned_window(overlay, context.timeout)?;
    thread::sleep(context.settle_delay);
    let plan = narrow_edge_interaction_plan_for_window(overlay.handle)?;
    let drag = inject_mouse_drag(
        overlay.handle,
        plan.base.drag_start,
        plan.base.drag_end,
        context.display.physical_bounds,
    )?;
    let selected_state = wait_for_capture_state(context, "narrow edge selection", |state| {
        state.session_state == "selecting"
            && state.selection.is_some()
            && state.overlay_count == 1
            && !state.more_actions_visible
            && !state.annotation_controls_visible
    })?;
    let committed_selection = selected_state
        .selection
        .ok_or_else(|| io::Error::other("narrow edge selection disappeared"))?;
    validate_selection_geometry(
        drag.selection,
        committed_selection,
        "narrow edge overlay selection",
    )?;
    if committed_selection.width() != NARROW_EDGE_SELECTION_WIDTH as u32
        || committed_selection.height() != NARROW_EDGE_SELECTION_HEIGHT as u32
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "narrow edge selection is {}x{}, expected {}x{}",
                committed_selection.width(),
                committed_selection.height(),
                NARROW_EDGE_SELECTION_WIDTH as u32,
                NARROW_EDGE_SELECTION_HEIGHT as u32
            ),
        ));
    }
    let selection_frame =
        query_capture_content(context, context.timeout.min(Duration::from_secs(1)))?
            .selection
            .ok_or_else(|| {
                io::Error::other("narrow edge selection did not expose source pixels")
            })?;
    validate_frame_dimensions(
        &selection_frame,
        committed_selection,
        "narrow edge selection frame",
    )?;
    if selection_frame.bounds != committed_selection {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "narrow edge frame bounds {:?} do not match selection {committed_selection:?}",
                selection_frame.bounds
            ),
        ));
    }
    let selection_metrics = frame_content_metrics(&selection_frame)?;
    let foreground = inject_mouse_move(overlay.handle, plan.evidence_rest)?;
    thread::sleep(context.settle_delay);
    let selected = capture_evidence(context, "01-edge-selected.png", overlay)?;
    record_step(
        report,
        &context.report_path,
        "narrow_selection_drag",
        foreground,
        Some(&selected),
    )?;

    inject_mouse_click(overlay.handle, plan.base.more)?;
    let more_open = wait_for_capture_state(context, "narrow More open", |state| {
        state.selection == Some(committed_selection)
            && state.overlay_count == 1
            && state.more_actions_visible
            && !state.annotation_controls_visible
    })?;
    let foreground = inject_mouse_move(overlay.handle, plan.evidence_rest)?;
    thread::sleep(context.settle_delay);
    let more = capture_evidence(context, "02-edge-more.png", overlay)?;
    ensure_evidence_changed(&selected, &more, "narrow More did not change the overlay")?;
    record_step(
        report,
        &context.report_path,
        "narrow_more_open",
        foreground,
        Some(&more),
    )?;

    inject_mouse_click(overlay.handle, plan.base.more)?;
    let more_closed = wait_for_capture_state(context, "narrow More close", |state| {
        state.selection == Some(committed_selection)
            && state.overlay_count == 1
            && !state.more_actions_visible
            && !state.annotation_controls_visible
    })?;
    let foreground = inject_mouse_move(overlay.handle, plan.evidence_rest)?;
    thread::sleep(context.settle_delay);
    let less = capture_evidence(context, "03-edge-less.png", overlay)?;
    ensure_evidence_changed(&more, &less, "narrow Less did not close the menu")?;
    record_step(
        report,
        &context.report_path,
        "narrow_more_closed",
        foreground,
        Some(&less),
    )?;

    inject_mouse_click(overlay.handle, plan.base.mark)?;
    let annotation_open = wait_for_capture_state(context, "narrow Mark open", |state| {
        state.selection == Some(committed_selection)
            && state.overlay_count == 1
            && !state.more_actions_visible
            && state.annotation_controls_visible
    })?;
    let foreground = inject_mouse_move(overlay.handle, plan.evidence_rest)?;
    thread::sleep(context.settle_delay);
    let marking = capture_evidence(context, "04-edge-mark.png", overlay)?;
    ensure_evidence_changed(&less, &marking, "narrow Mark did not open its controls")?;
    record_step(
        report,
        &context.report_path,
        "narrow_mark_open",
        foreground,
        Some(&marking),
    )?;

    inject_mouse_click(overlay.handle, plan.expanded_mark)?;
    let annotation_closed = wait_for_capture_state(context, "narrow Mark close", |state| {
        state.selection == Some(committed_selection)
            && state.overlay_count == 1
            && !state.more_actions_visible
            && !state.annotation_controls_visible
    })?;
    let foreground = inject_mouse_move(overlay.handle, plan.evidence_rest)?;
    thread::sleep(context.settle_delay);
    let marking_closed = capture_evidence(context, "05-edge-mark-closed.png", overlay)?;
    ensure_evidence_changed(
        &marking,
        &marking_closed,
        "narrow Mark did not close its controls",
    )?;
    record_step(
        report,
        &context.report_path,
        "narrow_mark_closed",
        foreground,
        Some(&marking_closed),
    )?;

    let foreground = inject_mouse_click(overlay.handle, plan.base.cancel)?;
    wait_for_window_gone(overlay.handle, context.timeout, "narrow Cancel")?;
    record_step(
        report,
        &context.report_path,
        "narrow_cancel",
        foreground,
        None,
    )?;
    let final_state = wait_for_capture_state(context, "narrow edge cleanup", |state| {
        state.overlay_count == 0
            && state.pinned_count == 0
            && !state.more_actions_visible
            && !state.annotation_controls_visible
            && state.capture_preflight_ready
            && matches!(
                state.session_state.as_str(),
                "idle" | "completed" | "cancelled"
            )
    })?;
    let visible_process_windows = process_windows()?.len();
    if visible_process_windows != 0 {
        return Err(io::Error::other(format!(
            "narrow edge cleanup left {visible_process_windows} visible process window(s)"
        )));
    }
    report.narrow_edge = Some(NarrowEdgeReport {
        controller_client_bounds: controller_client,
        controller_logical_width,
        controller_logical_height,
        requested_selection: drag.selection,
        committed_selection,
        selection_content: NarrowEdgeContentReport {
            bounds: selection_frame.bounds,
            width: selection_frame.width,
            height: selection_frame.height,
            fingerprint: format!("{:016x}", selection_metrics.fingerprint),
            luma_min: selection_metrics.luma_min,
            luma_max: selection_metrics.luma_max,
        },
        more_opened: more_open.more_actions_visible,
        more_closed: !more_closed.more_actions_visible,
        annotation_opened: annotation_open.annotation_controls_visible,
        annotation_closed: !annotation_closed.annotation_controls_visible,
        cleanup: CleanupReport {
            session_state: final_state.session_state,
            overlay_count: final_state.overlay_count,
            pinned_count: final_state.pinned_count,
            capture_teardown_pending: final_state.capture_teardown_pending,
            visible_process_windows,
            capture_preflight_ready: final_state.capture_preflight_ready,
        },
    });
    write_report(&context.report_path, report)
}

#[cfg(windows)]
/// Creates three Pins through the real toolbar, drags one, then captures while all three coexist.
fn execute_pins_coexist_interactions(
    context: &WorkerContext,
    report: &mut AcceptanceReport,
) -> io::Result<()> {
    let controller = wait_for_controller(context.timeout)?;
    focus_owned_window(controller, context.timeout)?;
    report.controller_window = Some(controller.report());
    record_step(
        report,
        &context.report_path,
        "pins_controller_ready",
        controller,
        None,
    )?;

    let creation_actions = ["pin_one_created", "pin_two_created", "pin_three_created"];
    let selection_actions = [
        "pin_one_selection_ready",
        "pin_two_selection_ready",
        "pin_three_selection_ready",
    ];
    let selection_files = [
        "00-pin-one-selection.png",
        "01-pin-two-selection.png",
        "02-pin-three-selection.png",
    ];
    let mut handles = Vec::with_capacity(PIN_COEXIST_COUNT);
    let mut sources = Vec::with_capacity(PIN_COEXIST_COUNT);
    let mut pin_reports = Vec::with_capacity(PIN_COEXIST_COUNT);
    for (index, action) in creation_actions.into_iter().enumerate() {
        let (overlay, plan, selection, requested_selection, source) =
            begin_selected_overlay_with_plan(context, controller, |handle| {
                pin_coexist_interaction_plan_for_window(handle, index)
            })?;
        thread::sleep(context.settle_delay);
        let selected = capture_evidence(context, selection_files[index], overlay)?;
        record_step(
            report,
            &context.report_path,
            selection_actions[index],
            guard_foreground(overlay.handle)?,
            Some(&selected),
        )?;
        let foreground = inject_mouse_click(overlay.handle, plan.pin)?;
        let state = wait_for_capture_state(context, action, |state| {
            state.session_state == "idle"
                && state.overlay_count == 0
                && state.pinned_count == index + 1
                && state.pinned_source_bounds == Some(selection)
                && state.capture_preflight_ready
        })?;
        wait_for_overlay_teardown(
            overlay.handle,
            context.display.physical_bounds,
            context.timeout,
            "Pin creation",
        )?;
        let content = query_capture_content(context, context.timeout.min(Duration::from_secs(1)))?;
        if content.pins.len() != index + 1 {
            return Err(io::Error::other(format!(
                "{action} exposed {} Pin source frame(s), expected {}",
                content.pins.len(),
                index + 1
            )));
        }
        let pinned = content
            .pins
            .last()
            .ok_or_else(|| io::Error::other("new Pin did not expose its source frame"))?
            .clone();
        let pixel_match = validate_same_pixel_content(&source, &pinned, action)?;
        let pin = wait_for_new_pin(controller.handle, &handles, context.timeout)?;
        pin_reports.push(PinReport {
            requested_selection,
            selection,
            source_bounds: state
                .pinned_source_bounds
                .ok_or_else(|| io::Error::other("new Pin source bounds disappeared"))?,
            window: pin.report(),
            content: pixel_match,
        });
        handles.push(pin.handle);
        sources.push(pinned);
        record_step(report, &context.report_path, action, foreground, None)?;
    }

    let initial_windows = owned_windows(&handles)?;
    let layout = horizontal_pin_layout(context.display.physical_bounds, &initial_windows)?;
    for (window, bounds) in initial_windows.iter().zip(&layout) {
        move_owned_window(window.handle, *bounds)?;
    }
    thread::sleep(context.settle_delay);
    let arranged_windows = owned_windows(&handles)?;
    validate_exact_window_bounds(&arranged_windows, &layout, "arranged Pin windows")?;
    focus_owned_window(arranged_windows[0], context.timeout)?;
    validate_pin_sources(context, &sources, "arranged Pins")?;
    let before_region =
        window_union_with_margin(&arranged_windows, context.display.physical_bounds, 20)?;
    let before = capture_region_evidence(
        context,
        "03-pins-before-capture.png",
        arranged_windows[0],
        before_region,
    )?;
    record_step(
        report,
        &context.report_path,
        "pins_arranged",
        arranged_windows[0],
        Some(&before),
    )?;

    let drag_before = arranged_windows[0].bounds;
    let drag_start = PhysicalPoint {
        x: drag_before.left + drag_before.width() as i32 / 2,
        y: drag_before.bottom - 24,
    };
    let drag_end = PhysicalPoint {
        x: drag_start.x + 48,
        y: drag_start.y - 40,
    };
    let injected_drag = inject_native_window_drag(handles[0], drag_start, drag_end)?;
    let drag_after = wait_for_window_drag(
        handles[0],
        drag_before,
        injected_drag.start,
        injected_drag.end,
        context.timeout,
    )?;
    let dragged_windows = owned_windows(&handles)?;
    validate_non_overlapping_windows(&dragged_windows, context.display.physical_bounds)?;
    let dragged_region =
        window_union_with_margin(&dragged_windows, context.display.physical_bounds, 20)?;
    let dragged = capture_region_evidence(
        context,
        "04-pin-pointer-drag.png",
        dragged_windows[0],
        dragged_region,
    )?;
    record_step(
        report,
        &context.report_path,
        "pin_pointer_drag",
        injected_drag.foreground,
        Some(&dragged),
    )?;
    let expected_delta_x = injected_drag.end.x - injected_drag.start.x;
    let expected_delta_y = injected_drag.end.y - injected_drag.start.y;
    let pointer_drag = WindowDragReport {
        handle: handles[0] as usize,
        before: drag_before,
        after: drag_after,
        pointer_start: injected_drag.start,
        pointer_end: injected_drag.end,
        expected_delta_x,
        expected_delta_y,
        actual_delta_x: drag_after.left - drag_before.left,
        actual_delta_y: drag_after.top - drag_before.top,
    };

    validate_pin_sources(context, &sources, "Pins before coexistence capture")?;
    let baseline_windows = owned_windows(&handles)?;
    focus_owned_window(baseline_windows[0], context.timeout)?;
    let foreground = inject_capture_shortcut(baseline_windows[0].handle)?;
    record_step(
        report,
        &context.report_path,
        "pins_capture_shortcut",
        foreground,
        None,
    )?;
    let overlay = wait_for_overlay(
        controller.handle,
        context.display.physical_bounds,
        context.timeout,
    )?;
    // Settings is hidden by an asynchronous production callback. Do not drag until it is gone,
    // otherwise the source frame can contain the controller or a stale native dialog.
    wait_for_window_gone(
        controller.handle,
        context.timeout,
        "capture overlay hides Settings",
    )?;
    focus_owned_window(overlay, context.timeout)?;
    thread::sleep(context.settle_delay);
    let plan = interaction_plan_for_window(overlay.handle)?;
    let capture_drag = inject_mouse_drag(
        overlay.handle,
        plan.drag_start,
        plan.drag_end,
        context.display.physical_bounds,
    )?;
    let active = wait_for_capture_state(context, "capture with three Pins", |state| {
        state.session_state == "selecting"
            && state.selection.is_some()
            && state.overlay_count == 1
            && state.pinned_count == PIN_COEXIST_COUNT
    })?;
    let committed_capture_selection = active
        .selection
        .ok_or_else(|| io::Error::other("coexistence capture selection disappeared"))?;
    validate_selection_geometry(
        capture_drag.selection,
        committed_capture_selection,
        "capture with three Pins",
    )?;
    validate_pin_sources(context, &sources, "Pins during coexistence capture")?;
    let windows_during_capture = owned_windows(&handles)?;
    validate_same_windows(
        &baseline_windows,
        &windows_during_capture,
        "active coexistence overlay",
    )?;
    thread::sleep(context.settle_delay);
    let overlay_evidence = capture_evidence(context, "05-overlay-with-pins.png", overlay)?;
    record_step(
        report,
        &context.report_path,
        "pins_overlay_selection",
        capture_drag.foreground,
        Some(&overlay_evidence),
    )?;

    let foreground = inject_mouse_click(overlay.handle, plan.cancel)?;
    wait_for_window_gone(overlay.handle, context.timeout, "Pin coexistence Cancel")?;
    record_step(
        report,
        &context.report_path,
        "pins_overlay_cancel",
        foreground,
        None,
    )?;
    let after_cancel = wait_for_capture_state(context, "Pin coexistence Cancel", |state| {
        state.overlay_count == 0
            && state.pinned_count == PIN_COEXIST_COUNT
            && state.capture_preflight_ready
            && matches!(
                state.session_state.as_str(),
                "idle" | "completed" | "cancelled"
            )
    })?;
    validate_pin_sources(context, &sources, "Pins after coexistence Cancel")?;
    let windows_after_cancel = owned_windows(&handles)?;
    validate_same_windows(
        &baseline_windows,
        &windows_after_cancel,
        "coexistence Cancel",
    )?;
    focus_owned_window(windows_after_cancel[0], context.timeout)?;
    let after_region =
        window_union_with_margin(&windows_after_cancel, context.display.physical_bounds, 20)?;
    let after = capture_region_evidence(
        context,
        "06-pins-after-cancel.png",
        windows_after_cancel[0],
        after_region,
    )?;
    record_step(
        report,
        &context.report_path,
        "pins_survived_cancel",
        windows_after_cancel[0],
        Some(&after),
    )?;

    let close_actions = ["pin_one_escape", "pin_two_escape", "pin_three_escape"];
    for (index, (handle, action)) in handles.iter().zip(close_actions).enumerate() {
        let pin = owned_window(*handle)?;
        focus_owned_window(pin, context.timeout)?;
        let foreground = inject_key(pin.handle, VK_ESCAPE)?;
        wait_for_window_gone(pin.handle, context.timeout, action)?;
        let remaining = PIN_COEXIST_COUNT - index - 1;
        wait_for_capture_state(context, action, |state| {
            state.overlay_count == 0
                && state.pinned_count == remaining
                && state.capture_preflight_ready
        })?;
        record_step(report, &context.report_path, action, foreground, None)?;
    }

    let final_state = wait_for_capture_state(context, "Pin coexistence cleanup", |state| {
        state.overlay_count == 0 && state.pinned_count == 0 && state.capture_preflight_ready
    })?;
    let visible_process_windows = process_windows()?.len();
    if visible_process_windows != 0 {
        return Err(io::Error::other(format!(
            "Pin coexistence cleanup left {visible_process_windows} visible process window(s)"
        )));
    }
    report.pins_coexist = Some(PinsCoexistReport {
        pins: pin_reports,
        arranged_windows: arranged_windows
            .into_iter()
            .map(NativeWindow::report)
            .collect(),
        pointer_drag,
        requested_capture_selection: capture_drag.selection,
        committed_capture_selection,
        windows_during_capture: windows_during_capture
            .into_iter()
            .map(NativeWindow::report)
            .collect(),
        windows_after_cancel: windows_after_cancel
            .into_iter()
            .map(NativeWindow::report)
            .collect(),
        sources_unchanged_during_capture: true,
        sources_unchanged_after_cancel: true,
        pins_during_capture: active.pinned_count,
        pins_after_cancel: after_cancel.pinned_count,
        closed_with_escape: PIN_COEXIST_COUNT,
        cleanup: CleanupReport {
            session_state: final_state.session_state,
            overlay_count: final_state.overlay_count,
            pinned_count: final_state.pinned_count,
            capture_teardown_pending: final_state.capture_teardown_pending,
            visible_process_windows,
            capture_preflight_ready: final_state.capture_preflight_ready,
        },
    });
    write_report(&context.report_path, report)
}

#[cfg(windows)]
/// Cancels an active transform overlay before reporting a deterministic geometry preflight error.
fn cancel_selection_transform_overlay(
    overlay: NativeWindow,
    capture_bounds: PhysicalRect,
    selection: PhysicalRect,
    timeout: Duration,
) -> io::Result<()> {
    let plan = interaction_plan_for_capture_selection(overlay.handle, capture_bounds, selection)?;
    inject_mouse_click(overlay.handle, plan.cancel)?;
    wait_for_window_gone(
        overlay.handle,
        timeout,
        "selection transform preflight Cancel",
    )
}

#[cfg(windows)]
/// Verifies real move and corner-resize gestures, including Shift and Alt modifier semantics.
fn execute_selection_transform_interactions(
    context: &WorkerContext,
    report: &mut AcceptanceReport,
) -> io::Result<()> {
    let controller = wait_for_controller(context.timeout)?;
    focus_owned_window(controller, context.timeout)?;
    report.controller_window = Some(controller.report());
    record_step(
        report,
        &context.report_path,
        "selection_transform_controller_ready",
        controller,
        None,
    )?;

    let foreground = inject_capture_shortcut(controller.handle)?;
    record_step(
        report,
        &context.report_path,
        "selection_transform_capture_shortcut",
        foreground,
        None,
    )?;
    let overlay = wait_for_overlay(
        controller.handle,
        context.display.physical_bounds,
        context.timeout,
    )?;
    wait_for_window_gone(
        controller.handle,
        context.timeout,
        "capture overlay hides Settings",
    )?;
    focus_owned_window(overlay, context.timeout)?;
    thread::sleep(context.settle_delay);
    let plan = interaction_plan_for_window(overlay.handle)?;
    let initial_drag = inject_mouse_drag(
        overlay.handle,
        plan.drag_start,
        plan.drag_end,
        context.display.physical_bounds,
    )?;
    let initial_state =
        wait_for_capture_state(context, "selection transform initial drag", |state| {
            state.session_state == "selecting"
                && state.selection.is_some()
                && state.overlay_count == 1
                && !state.more_actions_visible
                && !state.annotation_controls_visible
        })?;
    let initial_selection = initial_state
        .selection
        .ok_or_else(|| io::Error::other("selection transform initial selection disappeared"))?;
    validate_selection_geometry(
        initial_drag.selection,
        initial_selection,
        "selection transform initial drag",
    )?;
    let evidence_rest = map_capture_point_to_screen(
        PhysicalPoint {
            x: context.display.physical_bounds.left + 24,
            y: context.display.physical_bounds.top + 24,
        },
        client_bounds_for_window(overlay.handle)?,
        context.display.physical_bounds,
    )?;
    let foreground = inject_mouse_move(overlay.handle, evidence_rest)?;
    thread::sleep(context.settle_delay);
    let mut previous_evidence = capture_evidence(context, "00-transform-selected.png", overlay)?;
    record_step(
        report,
        &context.report_path,
        "selection_transform_initial",
        foreground,
        Some(&previous_evidence),
    )?;

    let kinds = [
        SelectionTransformKind::Move,
        SelectionTransformKind::CornerResize,
        SelectionTransformKind::ShiftResize,
        SelectionTransformKind::AltResize,
    ];
    let screenshots = [
        "01-transform-moved.png",
        "02-transform-resized.png",
        "03-transform-shift-resized.png",
        "04-transform-alt-resized.png",
    ];
    let mut current = initial_selection;

    // Check the complete fixed gesture sequence before the first transform so a small desktop
    // fails with a clean Cancel instead of leaving an active overlay after a partial run. Keep
    // model errors on this same cleanup path because they also happen while the overlay is live.
    let preflight_result = (|| -> io::Result<()> {
        let mut preview = current;
        for kind in kinds {
            let requested =
                selection_transform_gesture(preview, context.display.physical_bounds, kind)?;
            preview = expected_selection_transform(
                preview,
                requested.start,
                requested.end,
                context.display.physical_bounds,
                kind,
            )?;
        }
        Ok(())
    })();
    if let Err(error) = preflight_result {
        let cleanup = cancel_selection_transform_overlay(
            overlay,
            context.display.physical_bounds,
            current,
            context.timeout,
        );
        return match cleanup {
            Ok(()) => Err(error),
            Err(cleanup_error) => Err(io::Error::other(format!(
                "{error}; transform preflight cleanup failed: {cleanup_error}"
            ))),
        };
    }

    let mut gestures = Vec::with_capacity(kinds.len());
    let transform_result = (|| -> io::Result<()> {
        for (kind, screenshot) in kinds.into_iter().zip(screenshots) {
            let requested =
                selection_transform_gesture(current, context.display.physical_bounds, kind)?;
            let injected = inject_selection_transform_drag(
                overlay.handle,
                requested,
                context.display.physical_bounds,
            )?;
            validate_point_geometry(
                requested.start,
                injected.start,
                &format!("{} pointer start", kind.label()),
            )?;
            validate_point_geometry(
                requested.end,
                injected.end,
                &format!("{} pointer end", kind.label()),
            )?;
            let expected = expected_selection_transform(
                current,
                injected.start,
                injected.end,
                context.display.physical_bounds,
                kind,
            )?;
            let transformed = wait_for_capture_state(context, kind.label(), |state| {
                state.session_state == "selecting"
                    && state
                        .selection
                        .is_some_and(|selection| selection != current)
                    && state.overlay_count == 1
                    && !state.more_actions_visible
                    && !state.annotation_controls_visible
            })?
            .selection
            .ok_or_else(|| io::Error::other(format!("{} selection disappeared", kind.label())))?;
            validate_selection_geometry(expected, transformed, kind.label())?;

            let size_preserved = (kind == SelectionTransformKind::Move).then_some(
                current.width() == transformed.width() && current.height() == transformed.height(),
            );
            let opposite_corner_fixed = matches!(
                kind,
                SelectionTransformKind::CornerResize | SelectionTransformKind::ShiftResize
            )
            .then_some(current.left == transformed.left && current.top == transformed.top);
            let aspect_ratio_preserved = matches!(kind, SelectionTransformKind::ShiftResize)
                .then_some(selection_aspect_ratio_preserved(current, transformed));
            let center_preserved = matches!(kind, SelectionTransformKind::AltResize)
                .then_some(selection_center_preserved(current, transformed));
            if size_preserved == Some(false)
                || opposite_corner_fixed == Some(false)
                || aspect_ratio_preserved == Some(false)
                || center_preserved == Some(false)
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{} violated its selection invariant", kind.label()),
                ));
            }

            let foreground = inject_mouse_move(overlay.handle, evidence_rest)?;
            thread::sleep(context.settle_delay);
            let evidence = capture_evidence(context, screenshot, overlay)?;
            ensure_evidence_changed(
                &previous_evidence,
                &evidence,
                &format!("{} did not change the overlay", kind.label()),
            )?;
            record_step(
                report,
                &context.report_path,
                kind.label(),
                foreground,
                Some(&evidence),
            )?;
            previous_evidence = evidence;
            gestures.push(SelectionTransformGestureReport {
                gesture: kind.label(),
                shift: kind.modifiers().shift,
                alt: kind.modifiers().alt,
                before: current,
                pointer_start: injected.start,
                pointer_end: injected.end,
                expected,
                committed: transformed,
                geometry_matches: true,
                size_preserved,
                opposite_corner_fixed,
                aspect_ratio_preserved,
                center_preserved,
            });
            current = transformed;
        }
        Ok(())
    })();
    if let Err(error) = transform_result {
        let cleanup = cancel_selection_transform_overlay(
            overlay,
            context.display.physical_bounds,
            current,
            context.timeout,
        );
        return match cleanup {
            Ok(()) => Err(error),
            Err(cleanup_error) => Err(io::Error::other(format!(
                "{error}; transform cleanup failed: {cleanup_error}"
            ))),
        };
    }

    let final_plan = interaction_plan_for_capture_selection(
        overlay.handle,
        context.display.physical_bounds,
        current,
    )?;
    let foreground = inject_mouse_click(overlay.handle, final_plan.cancel)?;
    wait_for_window_gone(
        overlay.handle,
        context.timeout,
        "selection transform Cancel",
    )?;
    record_step(
        report,
        &context.report_path,
        "selection_transform_cancel",
        foreground,
        None,
    )?;
    let final_state = wait_for_capture_state(context, "selection transform cleanup", |state| {
        state.overlay_count == 0
            && state.pinned_count == 0
            && state.capture_preflight_ready
            && matches!(
                state.session_state.as_str(),
                "idle" | "completed" | "cancelled"
            )
    })?;
    // Cancel restores the Settings controller for normal users; hide this probe-owned window
    // before asserting that the isolated process has no visible native windows left.
    unsafe { ShowWindow(controller.handle, SW_HIDE) };
    wait_for_window_gone(
        controller.handle,
        context.timeout,
        "selection transform controller hide",
    )?;
    let visible_process_windows = process_windows()?.len();
    if visible_process_windows != 0 {
        return Err(io::Error::other(format!(
            "selection transform cleanup left {visible_process_windows} visible process window(s)"
        )));
    }
    report.selection_transform = Some(SelectionTransformReport {
        initial_requested_selection: initial_drag.selection,
        initial_selection,
        gestures,
        cleanup: CleanupReport {
            session_state: final_state.session_state,
            overlay_count: final_state.overlay_count,
            pinned_count: final_state.pinned_count,
            capture_teardown_pending: final_state.capture_teardown_pending,
            visible_process_windows,
            capture_preflight_ready: final_state.capture_preflight_ready,
        },
    });
    write_report(&context.report_path, report)
}

#[cfg(windows)]
/// Drives More -> Scroll shot -> real auto-scroll capture -> Finish, then proves teardown.
fn execute_scroll_roundtrip_interactions(
    context: &WorkerContext,
    report: &mut AcceptanceReport,
) -> io::Result<()> {
    let display = context.display.physical_bounds;
    let fixture_bounds = PhysicalRect {
        left: display.left.saturating_add(120),
        top: display.top.saturating_add(100),
        right: display.right.saturating_sub(120),
        bottom: display.bottom.saturating_sub(100),
    };
    let mut controller = wait_for_controller(context.timeout)?;
    let controller_right = display.right.saturating_sub(20);
    let controller_bottom = display.bottom.saturating_sub(20);
    move_owned_window(
        controller.handle,
        PhysicalRect {
            left: controller_right.saturating_sub(controller.bounds.width() as i32),
            top: controller_bottom.saturating_sub(controller.bounds.height() as i32),
            right: controller_right,
            bottom: controller_bottom,
        },
    )?;
    controller = owned_window(controller.handle)?;
    focus_owned_window(controller, context.timeout)?;
    report.controller_window = Some(controller.report());
    record_step(
        report,
        &context.report_path,
        "scroll_roundtrip_controller_ready",
        controller,
        None,
    )?;
    // Launch the external target only after the acceptance controller owns foreground input; the
    // no-activate fixture style keeps that ownership while still receiving wheel hit testing.
    let mut fixture = ScrollWindowFixture::launch(fixture_bounds, context.timeout)?;
    let fixture_window = fixture.report()?;
    focus_owned_window(controller, context.timeout)?;

    let foreground = inject_capture_shortcut(controller.handle)?;
    record_step(
        report,
        &context.report_path,
        "scroll_roundtrip_capture_shortcut",
        foreground,
        None,
    )?;
    let overlay = wait_for_overlay(controller.handle, display, context.timeout)?;
    focus_owned_window(overlay, context.timeout)?;
    thread::sleep(context.settle_delay);
    let plan = scroll_roundtrip_interaction_plan_for_window(overlay.handle)?;
    let drag = inject_mouse_drag(overlay.handle, plan.drag_start, plan.drag_end, display)?;
    let selected = capture_evidence(context, "00-scroll-selected.png", drag.foreground)?;
    record_step(
        report,
        &context.report_path,
        "scroll_roundtrip_selection",
        drag.foreground,
        Some(&selected),
    )?;
    let initial_state = wait_for_capture_state(context, "scroll roundtrip selection", |state| {
        state.session_state == "selecting" && state.selection.is_some() && state.overlay_count == 1
    })?;
    let initial_selection = initial_state
        .selection
        .ok_or_else(|| io::Error::other("scroll roundtrip selection disappeared"))?;
    validate_selection_geometry(
        drag.selection,
        initial_selection,
        "scroll roundtrip selection",
    )?;
    if !rect_contains_rect(fixture.target_bounds, initial_selection) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "scroll fixture {:?} does not contain the full selection {initial_selection:?}",
                fixture.target_bounds
            ),
        ));
    }

    let initial_frame = query_capture_content(context, context.timeout)?
        .selection
        .ok_or_else(|| {
            io::Error::other("scroll roundtrip did not expose the initial selection frame")
        })?;
    validate_scroll_fixture_frame(&initial_frame, fixture.target_bounds, 0)?;
    let initial_frame_report =
        save_scroll_frame_report(context, "01-scroll-initial-frame.png", initial_frame)?;

    let foreground = inject_mouse_click(overlay.handle, plan.more)?;
    let _more_state = wait_for_capture_state(context, "scroll roundtrip More", |state| {
        state.session_state == "selecting" && state.more_actions_visible && state.overlay_count == 1
    })?;
    let more = capture_evidence(context, "02-scroll-more.png", overlay)?;
    ensure_evidence_changed(
        &selected,
        &more,
        "Scroll roundtrip More did not change the overlay",
    )?;
    record_step(
        report,
        &context.report_path,
        "scroll_roundtrip_more",
        foreground,
        Some(&more),
    )?;
    let scroll_point =
        scroll_shot_point_for_capture_selection(overlay.handle, display, initial_selection)?;
    if !overlay.bounds.contains(scroll_point) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("computed Scroll shot point {scroll_point:?} escaped overlay"),
        ));
    }
    let foreground = inject_mouse_click(overlay.handle, scroll_point)?;
    record_step(
        report,
        &context.report_path,
        "scroll_roundtrip_scroll_shot",
        foreground,
        None,
    )?;
    wait_for_window_gone(overlay.handle, context.timeout, "Scroll shot")?;
    let ready = wait_for_capture_state(context, "scroll roundtrip ready", |state| {
        state.overlay_count == 0
            && !state.more_actions_visible
            && !state.annotation_controls_visible
            && state.status == "Scrolling screenshot ready. One frame captured."
    })?;
    let mut scroll_control = wait_for_scroll_control(controller.handle, context.timeout)?;
    let preferred_top = initial_selection.bottom.saturating_add(12);
    let preferred_bottom = preferred_top.saturating_add(scroll_control.bounds.height() as i32);
    if preferred_bottom <= display.bottom.saturating_sub(12) {
        move_owned_window(
            scroll_control.handle,
            PhysicalRect {
                left: scroll_control.bounds.left,
                top: preferred_top,
                right: scroll_control.bounds.right,
                bottom: preferred_bottom,
            },
        )?;
        scroll_control = owned_window(scroll_control.handle)?;
    }
    if rects_overlap(initial_selection, scroll_control.bounds) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "scroll control {:?} overlaps capture selection {initial_selection:?}",
                scroll_control.bounds
            ),
        ));
    }
    focus_owned_window(scroll_control, context.timeout)?;
    let scroll_control_report = scroll_control.report();
    set_fixture_window_bounds(fixture.target, fixture.target_bounds, true)?;
    let wheel_routing = preflight_scroll_input(initial_selection, fixture.target)?;

    let foreground = inject_scroll_auto_capture(scroll_control.handle)?;
    record_step(
        report,
        &context.report_path,
        "scroll_roundtrip_auto_capture",
        foreground,
        None,
    )?;
    let auto = wait_for_capture_state(context, "scroll roundtrip auto capture", |state| {
        state.overlay_count == 0 && state.status.starts_with("Captured scroll frame 2 (")
    })?;
    thread::sleep(context.settle_delay);
    let second_frame_report = capture_scroll_region_evidence(
        context,
        "03-scroll-second-frame.png",
        initial_selection,
        fixture.target,
        fixture.target_bounds,
        SCROLL_FIXTURE_SCROLL_STEP * 3,
    )?;
    if second_frame_report.fingerprint == initial_frame_report.fingerprint {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "auto scroll completed but the fixture viewport did not change",
        ));
    }

    focus_owned_window(scroll_control, context.timeout)?;
    thread::sleep(context.settle_delay);
    scroll_control = owned_window(scroll_control.handle)?;
    let control_ready = capture_evidence(context, "03-scroll-control-ready.png", scroll_control)?;
    record_step(
        report,
        &context.report_path,
        "scroll_roundtrip_control_ready",
        scroll_control,
        Some(&control_ready),
    )?;
    let foreground = inject_key(scroll_control.handle, VK_RETURN)?;
    record_step(
        report,
        &context.report_path,
        "scroll_roundtrip_finish",
        foreground,
        None,
    )?;
    let finished = wait_for_capture_state(context, "scroll roundtrip Finish", |state| {
        state.overlay_count == 1
            && state.selection.is_some()
            && !state.more_actions_visible
            && !state.annotation_controls_visible
            && state
                .status
                .starts_with("Scrolling screenshot stitched 2 frames")
    })?;
    let stitched_selection = finished
        .selection
        .ok_or_else(|| io::Error::other("finished scrolling selection disappeared"))?;
    if stitched_selection.height() <= initial_selection.height() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "stitched selection {stitched_selection:?} did not grow beyond viewport {initial_selection:?}"
            ),
        ));
    }
    wait_for_window_gone(
        scroll_control.handle,
        context.timeout,
        "scroll Finish control close",
    )?;
    let stitched_overlay =
        wait_for_finished_image_overlay(controller.handle, scroll_control.handle, context.timeout)?;
    focus_owned_window(stitched_overlay, context.timeout)?;
    let stitched_evidence =
        capture_evidence(context, "04-scroll-stitched-overlay.png", stitched_overlay)?;
    record_step(
        report,
        &context.report_path,
        "scroll_roundtrip_stitched_overlay",
        stitched_overlay,
        Some(&stitched_evidence),
    )?;
    let stitched_source = query_capture_content(context, context.timeout)?
        .selection
        .ok_or_else(|| io::Error::other("finished scroll editor did not expose stitched pixels"))?;
    validate_frame_dimensions(
        &stitched_source,
        stitched_selection,
        "stitched scroll editor source",
    )?;
    if stitched_source.bounds != stitched_selection {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "stitched source bounds {:?} do not match editor selection {stitched_selection:?}",
                stitched_source.bounds
            ),
        ));
    }
    let export_plan = interaction_plan_for_capture_selection(
        stitched_overlay.handle,
        stitched_source.bounds,
        stitched_selection,
    )?;
    let export = execute_scroll_export(
        context,
        report,
        controller,
        stitched_overlay,
        export_plan,
        stitched_selection,
        &stitched_source,
    )?;
    let final_state = wait_for_capture_state(
        context,
        "scroll roundtrip cleanup",
        scroll_roundtrip_cleanup_complete,
    )?;
    ensure_input_keys_released(&[
        (VK_CONTROL, "Control"),
        (VK_MENU, "Alt"),
        (VK_F24, "F24"),
        (VK_SHIFT, "Shift"),
        (VK_SPACE, "Space"),
        (VK_RETURN, "Enter"),
        (VK_ESCAPE, "Escape"),
        (VK_LBUTTON, "left mouse button"),
    ])?;
    unsafe { ShowWindow(controller.handle, SW_HIDE) };
    wait_for_window_gone(
        controller.handle,
        context.timeout,
        "scroll roundtrip controller hide",
    )?;
    fixture.shutdown(context.timeout.min(Duration::from_secs(3)))?;
    let visible_process_windows = process_windows()?.len();
    if visible_process_windows != 0 {
        return Err(io::Error::other(format!(
            "scroll roundtrip cleanup left {visible_process_windows} visible process window(s)"
        )));
    }
    report.scroll_roundtrip = Some(ScrollRoundtripReport {
        fixture_process_id: fixture.process_id,
        fixture_window,
        wheel_routing,
        requested_selection: drag.selection,
        initial_selection,
        scroll_control: scroll_control_report,
        ready_status: ready.status,
        initial_frame: initial_frame_report,
        auto_capture_status: auto.status,
        second_frame: second_frame_report,
        finish_status: finished.status,
        stitched_selection,
        stitched_height_increased: stitched_selection.height() > initial_selection.height(),
        export,
        manual_scroll_cleanup: ManualScrollCleanupReport {
            state: final_state.manual_scroll_state.clone(),
            frame_count: final_state.manual_scroll_frame_count,
            can_finish: final_state.manual_scroll_can_finish,
            capture_in_flight: final_state.manual_scroll_capture_in_flight,
            auto_capture_pending: final_state.manual_scroll_auto_capture_pending,
            selection: final_state.manual_scroll_selection,
            more_actions_visible: final_state.more_actions_visible,
            annotation_controls_visible: final_state.annotation_controls_visible,
        },
        cleanup: CleanupReport {
            session_state: final_state.session_state,
            overlay_count: final_state.overlay_count,
            pinned_count: final_state.pinned_count,
            capture_teardown_pending: final_state.capture_teardown_pending,
            visible_process_windows,
            capture_preflight_ready: final_state.capture_preflight_ready,
        },
    });
    write_report(&context.report_path, report)
}

#[cfg(windows)]
/// Completes the already-open stitched editor through the requested production exit.
fn execute_scroll_export(
    context: &WorkerContext,
    report: &mut AcceptanceReport,
    controller: NativeWindow,
    overlay: NativeWindow,
    plan: InteractionPlan,
    selection: PhysicalRect,
    source: &CaptureFrame,
) -> io::Result<ScrollExportReport> {
    match context.scroll_export {
        ScrollExportOption::Cancel => {
            let foreground = inject_key(overlay.handle, VK_ESCAPE)?;
            wait_for_window_gone(overlay.handle, context.timeout, "scroll Finish Cancel")?;
            record_step(
                report,
                &context.report_path,
                "scroll_roundtrip_cancel",
                foreground,
                None,
            )?;
            Ok(ScrollExportReport::Cancel)
        }
        ScrollExportOption::Copy => {
            execute_scroll_copy_export(context, report, overlay, plan, selection, source)
        }
        ScrollExportOption::Save => execute_scroll_save_export(
            context, controller, report, overlay, plan, selection, source,
        ),
    }
}

#[cfg(windows)]
/// Clicks Copy in the stitched editor and proves the real Windows clipboard has exact pixels.
fn execute_scroll_copy_export(
    context: &WorkerContext,
    report: &mut AcceptanceReport,
    overlay: NativeWindow,
    plan: InteractionPlan,
    selection: PhysicalRect,
    source: &CaptureFrame,
) -> io::Result<ScrollExportReport> {
    if context.copy_results.is_some() {
        return Err(io::Error::other(
            "scroll Copy must use the production system clipboard, not the injected sink",
        ));
    }
    let mut consumer = ClipboardConsumer::launch(&context.session_root, context.timeout)?;
    let consumer_ready_before_click = consumer.ready_path.is_file();
    let clipboard_sequence_before = consumer.arm()?;
    let consumer_observing_before_click = wait_for_path(
        &consumer.observing_path,
        context.timeout,
        "scroll clipboard consumer observing marker",
    )?;
    // Keep the report boundary immediately adjacent to the injected gesture. A change after
    // arming means another process won the disposable clipboard and this run must be rejected.
    // SAFETY: this call only reads the monotonic user32 clipboard change counter.
    let before_input_sequence = unsafe { GetClipboardSequenceNumber() };
    if before_input_sequence != clipboard_sequence_before {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "system clipboard changed before scroll Copy input ({clipboard_sequence_before} -> {before_input_sequence})"
            ),
        ));
    }
    let (foreground, copy_started_qpc) =
        inject_copy_trigger(overlay.handle, plan.copy, context.copy_trigger)?;
    let consumer_result = consumer.wait_result(context.timeout)?;
    if consumer_result.previous_sequence != clipboard_sequence_before {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "scroll clipboard consumer observed a mismatched launch sequence",
        ));
    }
    for path in [
        Path::new(&consumer_result.png_path),
        Path::new(&consumer_result.dib_path),
        Path::new(&consumer_result.consumer_image_path),
    ] {
        ensure_path_within(path, &context.session_root)?;
    }
    let copied = CaptureFrame::open_png(&consumer_result.consumer_image_path)?;
    let clipboard_png = fs::read(&consumer_result.png_path)?;
    let clipboard_dib = fs::read(&consumer_result.dib_path)?;
    wait_for_window_gone(overlay.handle, context.timeout, "scroll Copy")?;
    let state = wait_for_capture_state(context, "scroll Copy completion", |state| {
        state.session_state == "completed"
            && state.selection == Some(selection)
            && state.overlay_count == 0
            && state.capture_preflight_ready
            && state.status == "Selection copied to clipboard"
    })?;
    // SAFETY: this call only reads the monotonic user32 clipboard change counter.
    let clipboard_sequence_after = unsafe { GetClipboardSequenceNumber() };
    if clipboard_sequence_after != consumer_result.observed_sequence {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "system clipboard changed after consumer read ({:?} -> {clipboard_sequence_after})",
                consumer_result.observed_sequence
            ),
        ));
    }
    validate_frame_dimensions(&copied, selection, "system clipboard image")?;
    let consumer_image_content =
        validate_same_pixel_content(source, &copied, "system clipboard image")?;
    let png_path = PathBuf::from(&consumer_result.png_path);
    let decoded_png = CaptureFrame::open_png(&png_path)?;
    validate_frame_dimensions(&decoded_png, selection, "clipboard PNG format")?;
    let png_content = validate_same_pixel_content(source, &decoded_png, "clipboard PNG format")?;
    let decoded_dib = decode_clipboard_dib(&clipboard_dib)?;
    validate_frame_dimensions(&decoded_dib, selection, "clipboard CF_DIB format")?;
    let dib_content = validate_same_pixel_content(source, &decoded_dib, "clipboard CF_DIB format")?;
    let input_to_consumer_readable_ms =
        qpc_elapsed_ms(copy_started_qpc, consumer_result.consumer_read_qpc_ticks)?;
    if clipboard_sequence_after == clipboard_sequence_before
        || !consumer_ready_before_click
        || !consumer_observing_before_click
        || !consumer.stopped
    {
        return Err(io::Error::other(
            "scroll clipboard consumer was not ready, did not observe a sequence change, or was not reaped",
        ));
    }
    ensure_path_within(&png_path, &context.session_root)?;
    record_step(
        report,
        &context.report_path,
        "scroll_roundtrip_copy",
        foreground,
        None,
    )?;
    debug_assert_eq!(state.selection, Some(selection));
    Ok(ScrollExportReport::Copy {
        clipboard_sequence_before,
        clipboard_sequence_after,
        clipboard_sequence_changed: clipboard_sequence_after != clipboard_sequence_before,
        png_format_available: true,
        dib_format_available: true,
        copied_bounds: copied.bounds,
        width: copied.width,
        height: copied.height,
        png_path: png_path
            .strip_prefix(&context.session_root)
            .unwrap_or(&png_path)
            .to_string_lossy()
            .into_owned(),
        png_bytes: clipboard_png.len(),
        dib_bytes: clipboard_dib.len(),
        png_content,
        dib_content,
        consumer_image_content: Box::new(consumer_image_content),
        timing_clock: "windows_qpc",
        timing_boundary: "button_down_batch_to_consumer_decoded_image",
        input_to_consumer_readable_ms,
        consumer_result_path: consumer
            .result_path
            .strip_prefix(&context.session_root)
            .unwrap_or(&consumer.result_path)
            .to_string_lossy()
            .into_owned(),
    })
}

#[cfg(windows)]
/// Waits for Copy to replace the clipboard, then retries image decoding through transient locks.
fn wait_for_system_clipboard_image_change(
    previous_sequence: u32,
    timeout: Duration,
) -> io::Result<(u32, CaptureFrame, Vec<u8>, Vec<u8>)> {
    let deadline = Instant::now() + timeout;
    let mut last_error = "clipboard sequence has not changed".to_owned();
    loop {
        // SAFETY: this call only reads the monotonic user32 clipboard change counter.
        let sequence = unsafe { GetClipboardSequenceNumber() };
        if sequence != previous_sequence {
            match read_system_clipboard_formats() {
                Ok((png, dib)) => match SystemClipboard.read_image() {
                    Ok(frame) => {
                        // SAFETY: this call only reads the clipboard change counter.
                        let after_read = unsafe { GetClipboardSequenceNumber() };
                        if after_read == sequence {
                            return Ok((sequence, frame, png, dib));
                        }
                        last_error = format!(
                            "clipboard changed again from sequence {sequence} to {after_read} while reading"
                        );
                    }
                    Err(error) => last_error = error.to_string(),
                },
                Err(error) => last_error = error.to_string(),
            }
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "system clipboard did not expose the copied image after sequence {previous_sequence}: {last_error}"
                ),
            ));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(windows)]
/// Reads the Windows monotonic performance counter used for cross-process latency evidence.
fn qpc_ticks() -> io::Result<u64> {
    let mut ticks = 0_i64;
    // SAFETY: both pointers refer to writable local storage owned for this call.
    if unsafe { QueryPerformanceCounter(&mut ticks) } == 0 {
        return Err(io::Error::last_os_error());
    }
    u64::try_from(ticks).map_err(|_| io::Error::other("QPC returned a negative tick count"))
}

#[cfg(windows)]
/// Converts a non-negative QPC delta to milliseconds using the machine's stable frequency.
fn qpc_elapsed_ms(start_ticks: u64, end_ticks: u64) -> io::Result<f64> {
    if end_ticks < start_ticks {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("QPC moved backwards from {start_ticks} to {end_ticks}"),
        ));
    }
    let mut frequency = 0_i64;
    // SAFETY: `frequency` is writable local storage and the API writes one integer value.
    if unsafe { QueryPerformanceFrequency(&mut frequency) } == 0 || frequency <= 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((end_ticks - start_ticks) as f64 * 1000.0 / frequency as f64)
}

#[cfg(windows)]
/// Copies the registered PNG and CF_DIB bytes while one clipboard snapshot is locked.
fn read_system_clipboard_formats() -> io::Result<(Vec<u8>, Vec<u8>)> {
    let png_name = "PNG".encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    // SAFETY: the clipboard format name is NUL terminated and remains alive for this call.
    let png_format = unsafe { RegisterClipboardFormatW(png_name.as_ptr()) };
    if png_format == 0 {
        return Err(io::Error::last_os_error());
    }

    let mut opened = false;
    for attempt in 0..8 {
        // SAFETY: a null owner is valid for this short synchronous clipboard read.
        if unsafe { OpenClipboard(ptr::null_mut()) } != 0 {
            opened = true;
            break;
        }
        if attempt + 1 < 8 {
            thread::sleep(Duration::from_millis(5));
        }
    }
    if !opened {
        return Err(io::Error::last_os_error());
    }

    let result = (|| {
        // SAFETY: format availability is a read-only query while the clipboard remains open.
        if unsafe { IsClipboardFormatAvailable(png_format) } == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "system clipboard does not expose the registered PNG format",
            ));
        }
        // SAFETY: CF_DIB is the production compatibility format written alongside PNG.
        if unsafe { IsClipboardFormatAvailable(CF_DIB as u32) } == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "system clipboard does not expose the CF_DIB compatibility format",
            ));
        }
        let png = copy_open_clipboard_bytes(png_format, "registered PNG")?;
        let dib = copy_open_clipboard_bytes(CF_DIB as u32, "CF_DIB")?;
        Ok((png, dib))
    })();
    // SAFETY: balances the successful OpenClipboard call on this thread.
    unsafe { CloseClipboard() };
    result
}

#[cfg(windows)]
/// Copies one global-memory clipboard payload without taking ownership of its Windows handle.
fn copy_open_clipboard_bytes(format: u32, label: &str) -> io::Result<Vec<u8>> {
    // SAFETY: the caller keeps the clipboard open and the returned handle remains clipboard-owned.
    let handle = unsafe { GetClipboardData(format) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the clipboard format stores bytes in a global-memory handle.
    let size = unsafe { GlobalSize(handle) };
    if size == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("clipboard {label} payload is empty"),
        ));
    }
    // SAFETY: locking the live clipboard-owned global handle yields at least `size` bytes.
    let data = unsafe { GlobalLock(handle) };
    if data.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: data points to the locked allocation whose size was returned by GlobalSize.
    let bytes = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), size) }.to_vec();
    // SAFETY: balances the successful GlobalLock; clipboard ownership is unchanged.
    unsafe { GlobalUnlock(handle) };
    Ok(bytes)
}

#[cfg(windows)]
/// Decodes the 32-bit BI_RGB DIB emitted by Flash Shot into its top-down BGRA frame model.
fn decode_clipboard_dib(bytes: &[u8]) -> io::Result<CaptureFrame> {
    const BITMAP_INFO_HEADER_SIZE: usize = 40;
    if bytes.len() < BITMAP_INFO_HEADER_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "clipboard CF_DIB header is truncated",
        ));
    }
    let header_size = usize::try_from(u32::from_le_bytes(bytes[0..4].try_into().unwrap()))
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "CF_DIB header is too large"))?;
    let width = i32::from_le_bytes(bytes[4..8].try_into().unwrap());
    let signed_height = i32::from_le_bytes(bytes[8..12].try_into().unwrap());
    let planes = u16::from_le_bytes(bytes[12..14].try_into().unwrap());
    let bits_per_pixel = u16::from_le_bytes(bytes[14..16].try_into().unwrap());
    let compression = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
    if header_size < BITMAP_INFO_HEADER_SIZE || header_size > bytes.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "clipboard CF_DIB has an invalid header size",
        ));
    }
    let height = signed_height.checked_abs().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "clipboard CF_DIB height overflowed",
        )
    })?;
    if width <= 0 || height == 0 || planes != 1 || bits_per_pixel != 32 || compression != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "unsupported clipboard CF_DIB geometry/format: {width}x{signed_height}, planes {planes}, {bits_per_pixel} bpp, compression {compression}"
            ),
        ));
    }
    let width_usize = usize::try_from(width)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "CF_DIB width is too large"))?;
    let height_usize = usize::try_from(height)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "CF_DIB height is too large"))?;
    let stride = width_usize.checked_mul(4).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "clipboard CF_DIB stride overflowed",
        )
    })?;
    let pixel_bytes = stride.checked_mul(height_usize).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "clipboard CF_DIB pixels overflowed",
        )
    })?;
    let pixel_end = header_size.checked_add(pixel_bytes).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "clipboard CF_DIB size overflowed",
        )
    })?;
    if bytes.len() < pixel_end {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "clipboard CF_DIB pixels are truncated",
        ));
    }

    let mut pixels = vec![0_u8; pixel_bytes];
    for target_row in 0..height_usize {
        let source_row = if signed_height > 0 {
            height_usize - target_row - 1
        } else {
            target_row
        };
        let source_start = header_size + source_row * stride;
        let target_start = target_row * stride;
        pixels[target_start..target_start + stride]
            .copy_from_slice(&bytes[source_start..source_start + stride]);
    }
    let frame = CaptureFrame {
        bounds: PhysicalRect {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        },
        width: width as u32,
        height: height as u32,
        stride,
        format: PixelFormat::Bgra8,
        pixels: pixels.into(),
        capture_duration: Duration::ZERO,
        cpu_copy_count: 1,
    };
    frame.validate()?;
    Ok(frame)
}

#[cfg(windows)]
/// Drives the stitched editor's native Save dialog and compares the decoded PNG pixel for pixel.
fn execute_scroll_save_export(
    context: &WorkerContext,
    controller: NativeWindow,
    report: &mut AcceptanceReport,
    overlay: NativeWindow,
    plan: InteractionPlan,
    selection: PhysicalRect,
    source: &CaptureFrame,
) -> io::Result<ScrollExportReport> {
    let export_directory = context.session_root.join("exports");
    fs::create_dir_all(&export_directory)?;
    let target = export_directory.join("scroll-roundtrip.png");
    if target.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "isolated scroll Save target already exists: {}",
                target.display()
            ),
        ));
    }

    let dialogs_before_save = visible_common_dialogs()?;
    let foreground = inject_mouse_click(overlay.handle, plan.save)?;
    record_step(
        report,
        &context.report_path,
        "scroll_roundtrip_save_click",
        foreground,
        None,
    )?;
    let dialog = wait_for_save_dialog(
        overlay.handle,
        controller.handle,
        &dialogs_before_save,
        context.timeout,
    )?;
    thread::sleep(context.settle_delay);
    let dialog_evidence = capture_evidence(context, "05-scroll-save-dialog.png", dialog)?;
    record_step(
        report,
        &context.report_path,
        "scroll_roundtrip_save_dialog",
        dialog,
        Some(&dialog_evidence),
    )?;

    set_save_dialog_path(&dialog, &target, context.timeout)?;
    let path_evidence = capture_evidence(context, "06-scroll-save-path.png", dialog)?;
    record_step(
        report,
        &context.report_path,
        "scroll_roundtrip_save_path",
        dialog,
        Some(&path_evidence),
    )?;
    inject_key(dialog.handle, VK_RETURN)?;
    wait_for_window_gone(dialog.handle, context.timeout, "scroll Save confirmation")?;
    wait_for_no_visible_save_dialogs(context.timeout, "scroll Save confirmation")?;
    wait_for_window_gone(overlay.handle, context.timeout, "scroll Save completion")?;
    wait_for_capture_state(context, "scroll Save completion", |state| {
        state.session_state == "completed"
            && state.selection == Some(selection)
            && state.overlay_count == 0
            && state.capture_preflight_ready
            && state.status.starts_with("Scrolling screenshot saved to ")
    })?;
    let (saved, bytes) = wait_for_saved_png(&target, context.timeout)?;
    validate_frame_dimensions(&saved, selection, "saved scroll PNG")?;
    let content = validate_same_pixel_content(source, &saved, "saved scroll PNG")?;
    ensure_path_within(&target, &context.session_root)?;
    Ok(ScrollExportReport::Save {
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
    wait_for_capture_state(context, "Capture restart teardown", |state| {
        !state.capture_teardown_pending && state.capture_preflight_ready
    })?;
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
            capture_teardown_pending: final_state.capture_teardown_pending,
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
    begin_selected_overlay_with_plan(context, controller, interaction_plan_for_window)
}

#[cfg(windows)]
/// Opens a fresh production overlay and commits the selection supplied by one measured plan.
fn begin_selected_overlay_with_plan(
    context: &WorkerContext,
    controller: NativeWindow,
    plan_for_window: impl FnOnce(*mut c_void) -> io::Result<InteractionPlan>,
) -> io::Result<(
    NativeWindow,
    InteractionPlan,
    PhysicalRect,
    PhysicalRect,
    CaptureFrame,
)> {
    // A previous native Save dialog can leave a compositor frame behind for a short interval.
    // Sample the whole desktop until it is quiet before asking production to start a new capture;
    // otherwise the stale dialog becomes part of the next screenshot's source pixels.
    wait_for_desktop_quiescence(context, "fresh capture setup", None)?;
    context
        .interaction_commands
        .send_blocking(OverlayInteractionAcceptanceCommand::ShowCaptureSettings)
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "capture command channel closed"))?;
    let controller = wait_for_owned_window_visible(controller.handle, context.timeout)?;
    wait_for_no_visible_save_dialogs(context.timeout, "fresh capture setup")?;
    focus_owned_window(controller, context.timeout)?;
    inject_capture_shortcut(controller.handle)?;
    let overlay = wait_for_overlay(
        controller.handle,
        context.display.physical_bounds,
        context.timeout,
    )?;
    // Settings is hidden by an asynchronous production callback. Do not drag until it is gone,
    // otherwise the source frame can contain the controller or a stale native dialog.
    wait_for_window_gone(
        controller.handle,
        context.timeout,
        "capture overlay hides Settings",
    )?;
    focus_owned_window(overlay, context.timeout)?;
    thread::sleep(context.settle_delay);
    let plan = plan_for_window(overlay.handle)?;
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
/// Opens the production Save dialog with Ctrl+S and verifies cancellation preserves the selection.
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

    let (overlay, _plan, selection, requested_selection, source) =
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

    let dialogs_before_cancel = visible_common_dialogs()?;
    let foreground = inject_ctrl_s(overlay.handle)?;
    record_step(
        report,
        &context.report_path,
        "save_shortcut",
        foreground,
        None,
    )?;
    let cancelled_dialog = wait_for_save_dialog(
        overlay.handle,
        controller.handle,
        &dialogs_before_cancel,
        context.timeout,
    )?;
    // The shell dialog becomes visible before its first paint and focus transition complete.
    thread::sleep(context.settle_delay);
    save_file_name_edit(cancelled_dialog.handle, "flash-shot.png")?;
    let dialog_evidence = capture_evidence(context, "07-save-dialog.png", cancelled_dialog)?;
    record_step(
        report,
        &context.report_path,
        "save_dialog_ready",
        cancelled_dialog,
        Some(&dialog_evidence),
    )?;

    inject_key(cancelled_dialog.handle, VK_ESCAPE)?;
    wait_for_window_gone(
        cancelled_dialog.handle,
        context.timeout,
        "Save dialog cancellation",
    )?;
    wait_for_no_visible_save_dialogs(context.timeout, "Save dialog cancellation")?;
    let cancelled = wait_for_capture_state(context, "cancelled Save dialog", |state| {
        state.session_state == "selecting"
            && state.selection == Some(selection)
            && state.overlay_count == 1
            && state.capture_preflight_ready
            && state.status
                == format!(
                    "Selection: {} x {} physical pixels",
                    selection.width(),
                    selection.height()
                )
    })?;
    if cancelled.selection != Some(selection) {
        return Err(io::Error::other(
            "cancelled Ctrl+S dialog did not preserve the committed selection",
        ));
    }
    focus_owned_window(overlay, context.timeout)?;
    thread::sleep(context.settle_delay);
    let restored = capture_evidence(context, "08-save-cancel-restored.png", overlay)?;
    record_step(
        report,
        &context.report_path,
        "save_cancelled_selection_restored",
        guard_foreground(overlay.handle)?,
        Some(&restored),
    )?;

    let dialogs_before_retry = visible_common_dialogs()?;
    let foreground = inject_ctrl_s(overlay.handle)?;
    record_step(
        report,
        &context.report_path,
        "save_shortcut_retry",
        foreground,
        None,
    )?;
    let dialog = wait_for_save_dialog(
        overlay.handle,
        controller.handle,
        &dialogs_before_retry,
        context.timeout,
    )?;
    thread::sleep(context.settle_delay);

    set_save_dialog_path(&dialog, &target, context.timeout)?;
    let path_evidence = capture_evidence(context, "09-save-path.png", dialog)?;
    record_step(
        report,
        &context.report_path,
        "save_path_verified",
        dialog,
        Some(&path_evidence),
    )?;
    inject_key(dialog.handle, VK_RETURN)?;
    wait_for_window_gone(dialog.handle, context.timeout, "Save confirmation")?;
    wait_for_no_visible_save_dialogs(context.timeout, "Save confirmation")?;
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
    let clean = wait_for_desktop_quiescence(
        context,
        "Save completion before Pin",
        Some("save-complete-clean.png"),
    )?;
    record_desktop_step(
        report,
        &context.report_path,
        "save_complete_clean",
        "save-complete-clean.png",
        &clean,
    )?;
    let (saved, bytes) = wait_for_saved_png(&target, context.timeout)?;
    validate_frame_dimensions(&saved, selection, "saved PNG")?;
    let content = validate_same_pixel_content(&source, &saved, "saved PNG")?;
    ensure_path_within(&target, &context.session_root)?;
    Ok(SaveReport {
        requested_selection,
        selection,
        cancelled_dialog_verified: true,
        selection_restored_after_cancel: true,
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
    let selected = capture_evidence(context, "10-pin-selection.png", overlay)?;
    record_step(
        report,
        &context.report_path,
        "pin_selection_ready",
        guard_foreground(overlay.handle)?,
        Some(&selected),
    )?;

    let visible_before_pin = process_windows()?;
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
        .pins
        .into_iter()
        .last()
        .ok_or_else(|| io::Error::other("selected Pin did not expose its source pixels"))?;
    let content = validate_same_pixel_content(&source, &pinned, "pinned frame")?;
    let pin = wait_for_new_pin_after_click(
        controller.handle,
        overlay.handle,
        &visible_before_pin,
        context.display.physical_bounds,
        context.timeout,
    )?;
    focus_owned_window(pin, context.timeout)?;
    thread::sleep(context.settle_delay);
    let pin_evidence = capture_evidence(context, "11-pin.png", pin)?;
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
/// Routes one real Copy trigger to either the isolated sink or the explicitly authorized clipboard.
fn execute_copy_interaction(
    context: &WorkerContext,
    report: &mut AcceptanceReport,
    controller: NativeWindow,
) -> io::Result<CopyReport> {
    if let Some(copy_results) = &context.copy_results {
        match copy_results.try_recv() {
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
    } else if !context.use_system_clipboard {
        return Err(io::Error::other(
            "standard Copy acceptance has neither an isolated sink nor clipboard authorization",
        ));
    }
    let (overlay, plan, selection, requested_selection, source) =
        begin_selected_overlay(context, controller)?;
    thread::sleep(context.settle_delay);
    let selected = capture_evidence(context, "12-copy-selection.png", overlay)?;
    record_step(
        report,
        &context.report_path,
        "copy_selection_ready",
        guard_foreground(overlay.handle)?,
        Some(&selected),
    )?;

    let mut consumer = context
        .use_system_clipboard
        .then(|| ClipboardConsumer::launch(&context.session_root, context.timeout))
        .transpose()?;
    let consumer_ready_before_click = consumer
        .as_ref()
        .is_some_and(|consumer| consumer.ready_path.is_file());
    let clipboard_sequence_before = if let Some(consumer) = consumer.as_ref() {
        consumer.arm()?
    } else {
        // SAFETY: this call only reads the monotonic user32 clipboard change counter.
        unsafe { GetClipboardSequenceNumber() }
    };
    let consumer_observing_before_click = if let Some(consumer) = consumer.as_ref() {
        wait_for_path(
            &consumer.observing_path,
            context.timeout,
            "clipboard consumer observing marker",
        )?
    } else {
        false
    };
    // Check after the consumer has entered its wait path and immediately before SendInput. A
    // change anywhere in the ready/start/observing handshake invalidates this isolated sample.
    // SAFETY: this call only reads the monotonic user32 clipboard change counter.
    let before_input_sequence = unsafe { GetClipboardSequenceNumber() };
    if before_input_sequence != clipboard_sequence_before {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "system clipboard changed before Copy input ({clipboard_sequence_before} -> {before_input_sequence})"
            ),
        ));
    }
    let (foreground, copy_started_qpc) =
        inject_copy_trigger(overlay.handle, plan.copy, context.copy_trigger)?;
    let (
        copied,
        clipboard_png,
        clipboard_dib,
        clipboard_sequence_after,
        consumer_result_path,
        consumer_read_qpc_ticks,
        consumer_png_path,
        consumer_dib_path,
        consumer_image_path,
    ) = if let Some(consumer) = consumer.as_mut() {
        let result = consumer.wait_result(context.timeout)?;
        let png_path = PathBuf::from(&result.png_path);
        let dib_path = PathBuf::from(&result.dib_path);
        let consumer_image_path = PathBuf::from(&result.consumer_image_path);
        for path in [&png_path, &dib_path, &consumer_image_path] {
            ensure_path_within(path, &context.session_root)?;
        }
        let copied = CaptureFrame::open_png(&consumer_image_path)?;
        let png = fs::read(&png_path)?;
        let dib = fs::read(&dib_path)?;
        if result.previous_sequence != clipboard_sequence_before
            || result.png_bytes != png.len()
            || result.dib_bytes != dib.len()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "clipboard consumer result does not match its launch sequence or artifact sizes",
            ));
        }
        ensure_path_within(&consumer.result_path, &context.session_root)?;
        let result_path = consumer
            .result_path
            .strip_prefix(&context.session_root)
            .unwrap_or(&consumer.result_path)
            .to_string_lossy()
            .into_owned();
        (
            copied,
            Some(png),
            Some(dib),
            {
                // SAFETY: this call only reads the monotonic user32 clipboard change counter.
                let current = unsafe { GetClipboardSequenceNumber() };
                if current != result.observed_sequence {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "system clipboard changed after consumer read ({} -> {current})",
                            result.observed_sequence
                        ),
                    ));
                }
                current
            },
            Some(result_path),
            Some(result.consumer_read_qpc_ticks),
            Some(png_path),
            Some(dib_path),
            Some(consumer_image_path),
        )
    } else {
        let copy_results = context
            .copy_results
            .as_ref()
            .expect("isolated Copy sink was checked above");
        let copied = copy_results
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
        // SAFETY: this call only reads the monotonic user32 clipboard change counter.
        let sequence = unsafe { GetClipboardSequenceNumber() };
        (copied, None, None, sequence, None, None, None, None, None)
    };
    let input_to_consumer_readable_ms = consumer_read_qpc_ticks
        .map(|read_ticks| qpc_elapsed_ms(copy_started_qpc, read_ticks))
        .transpose()?;
    if context.use_system_clipboard
        && (input_to_consumer_readable_ms.is_none()
            || !consumer_ready_before_click
            || !consumer.as_ref().is_some_and(|consumer| consumer.stopped))
    {
        return Err(io::Error::other(
            "clipboard consumer was not ready before input or was not reaped after reading",
        ));
    }
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
    if context.use_system_clipboard {
        // Re-read after the production state transition, not only after the child snapshot, so
        // a duplicate or late clipboard write cannot be hidden by an earlier observed counter.
        let final_sequence = unsafe { GetClipboardSequenceNumber() };
        if final_sequence != clipboard_sequence_after {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "system clipboard changed during Copy cleanup ({} -> {final_sequence})",
                    clipboard_sequence_after
                ),
            ));
        }
    }
    if context
        .copy_results
        .as_ref()
        .is_some_and(|copy_results| copy_results.try_recv().is_ok())
    {
        return Err(io::Error::other(
            "one Copy click produced more than one selection frame",
        ));
    }
    validate_frame_dimensions(&copied, selection, "copied frame")?;
    if !context.use_system_clipboard && copied.bounds != selection {
        return Err(io::Error::other(format!(
            "copied frame bounds {:?} do not match selection {:?}",
            copied.bounds, selection
        )));
    }
    let consumer_image_content = validate_same_pixel_content(&source, &copied, "copied frame")?;
    if context.use_system_clipboard && clipboard_sequence_after == clipboard_sequence_before {
        return Err(io::Error::other(
            "system clipboard sequence did not change after the production Copy click",
        ));
    }
    if !context.use_system_clipboard && clipboard_sequence_after != clipboard_sequence_before {
        return Err(io::Error::other(format!(
            "system clipboard sequence changed from {clipboard_sequence_before} to {clipboard_sequence_after}"
        )));
    }
    let (png_path, dib_path, consumer_image_path, png_bytes, dib_bytes, png_content, dib_content) =
        if let (Some(png), Some(dib)) = (clipboard_png, clipboard_dib) {
            let png_artifact = consumer_png_path.ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "clipboard PNG path is missing")
            })?;
            let dib_artifact = consumer_dib_path.ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "clipboard DIB path is missing")
            })?;
            let image_artifact = consumer_image_path.ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "consumer image path is missing")
            })?;
            let decoded_png = CaptureFrame::open_png(&png_artifact)?;
            validate_frame_dimensions(&decoded_png, selection, "clipboard PNG format")?;
            let png_content =
                validate_same_pixel_content(&source, &decoded_png, "clipboard PNG format")?;
            let decoded_dib = decode_clipboard_dib(&dib)?;
            validate_frame_dimensions(&decoded_dib, selection, "clipboard CF_DIB format")?;
            let dib_content =
                validate_same_pixel_content(&source, &decoded_dib, "clipboard CF_DIB format")?;
            let relative = |path: &Path| {
                path.strip_prefix(&context.session_root)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .into_owned()
            };
            (
                Some(relative(&png_artifact)),
                Some(relative(&dib_artifact)),
                Some(relative(&image_artifact)),
                Some(png.len()),
                Some(dib.len()),
                Some(png_content),
                Some(dib_content),
            )
        } else {
            (None, None, None, None, None, None, None)
        };
    record_step(
        report,
        &context.report_path,
        match (context.use_system_clipboard, context.copy_trigger) {
            (true, CopyTriggerOption::Toolbar) => "copy_system_clipboard_toolbar",
            (true, CopyTriggerOption::Enter) => "copy_system_clipboard_enter",
            (false, CopyTriggerOption::Toolbar) => "copy_click",
            (false, CopyTriggerOption::Enter) => "copy_enter",
        },
        foreground,
        None,
    )?;
    Ok(CopyReport {
        trigger: context.copy_trigger.label(),
        action: match context.copy_trigger {
            CopyTriggerOption::Toolbar => "toolbar_click",
            CopyTriggerOption::Enter => "enter_key",
        },
        read_mechanism: if context.use_system_clipboard {
            "independent_process_png_cf_dib_and_arboard"
        } else {
            "process_local_capture_frame_channel"
        },
        requested_selection,
        selection,
        copied_bounds: copied.bounds,
        width: copied.width,
        height: copied.height,
        clipboard_sequence_before,
        clipboard_sequence_after,
        clipboard_sequence_changed: clipboard_sequence_after != clipboard_sequence_before,
        timing_clock: "windows_qpc",
        timing_boundary: "button_down_batch_to_consumer_decoded_image",
        input_to_consumer_readable_ms,
        sink: if context.use_system_clipboard {
            "system_clipboard"
        } else {
            "isolated_observer"
        },
        png_format_available: context.use_system_clipboard,
        dib_format_available: context.use_system_clipboard,
        png_path,
        dib_path,
        consumer_image_path,
        png_bytes,
        dib_bytes,
        png_content,
        dib_content,
        consumer_image_content,
        consumer_process_id: consumer.as_ref().map(|consumer| consumer.process_id),
        consumer_result_path,
        consumer_ready_before_click,
        consumer_observing_before_click,
        consumer_cleaned_up: consumer.as_ref().is_none_or(|consumer| consumer.stopped),
        single_export_verified: state.overlay_count == 0
            && state.session_state == "completed"
            && context.copy_results.as_ref().is_none_or(|copy_results| {
                matches!(copy_results.try_recv(), Err(mpsc::TryRecvError::Empty))
            }),
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
/// Captures only the verified foreground native window and returns a change fingerprint.
fn capture_evidence(
    context: &WorkerContext,
    file_name: &str,
    window: NativeWindow,
) -> io::Result<Evidence> {
    capture_region_evidence(context, file_name, window, window.bounds)
}

#[cfg(windows)]
/// Captures a bounded desktop region only while the expected process window owns foreground input.
fn capture_region_evidence(
    context: &WorkerContext,
    file_name: &str,
    foreground: NativeWindow,
    bounds: PhysicalRect,
) -> io::Result<Evidence> {
    guard_foreground(foreground.handle)?;
    let frame = SystemCaptureBackend.capture(bounds)?;
    let path = context.session_root.join("screenshots").join(file_name);
    frame.save_png(&path)?;
    Ok(Evidence {
        file_name: format!("screenshots/{file_name}"),
        fingerprint: pixel_fingerprint(&frame.pixels),
    })
}

#[cfg(windows)]
/// Refuses assisted scrolling unless Windows routes the wheel to the verified hovered fixture.
fn preflight_scroll_input(bounds: PhysicalRect, target: HWND) -> io::Result<u32> {
    let mut routing = 0_u32;
    // SAFETY: `routing` is a writable u32 output for SPI_GETMOUSEWHEELROUTING.
    if unsafe {
        SystemParametersInfoW(
            SPI_GETMOUSEWHEELROUTING,
            0,
            (&mut routing as *mut u32).cast::<c_void>(),
            0,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if routing != MOUSEWHEEL_ROUTING_MOUSE_POS {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "scroll roundtrip requires mouse-position wheel routing; observed mode {routing}"
            ),
        ));
    }
    let center = POINT {
        x: bounds.left + bounds.width() as i32 / 2,
        y: bounds.top + bounds.height() as i32 / 2,
    };
    let hit = unsafe { WindowFromPoint(center) };
    if hit != target && unsafe { IsChild(target, hit) } == 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("scroll input point is under HWND {hit:?}, expected fixture target {target:?}"),
        ));
    }
    Ok(routing)
}

#[cfg(windows)]
/// Captures the selected viewport while proving the wheel target is the disposable fixture.
fn capture_scroll_region_evidence(
    context: &WorkerContext,
    file_name: &str,
    bounds: PhysicalRect,
    target: HWND,
    fixture_bounds: PhysicalRect,
    expected_offset: i32,
) -> io::Result<ScrollFrameReport> {
    let center = POINT {
        x: bounds.left + bounds.width() as i32 / 2,
        y: bounds.top + bounds.height() as i32 / 2,
    };
    let hit = unsafe { WindowFromPoint(center) };
    if hit != target && unsafe { IsChild(target, hit) } == 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "scroll viewport center is under HWND {hit:?}, expected fixture target {target:?}"
            ),
        ));
    }
    let frame = SystemCaptureBackend.capture(bounds)?;
    validate_scroll_fixture_frame(&frame, fixture_bounds, expected_offset)?;
    let path = context.session_root.join("screenshots").join(file_name);
    frame.save_png(&path)?;
    Ok(ScrollFrameReport {
        bounds: frame.bounds,
        width: frame.width,
        height: frame.height,
        screenshot: format!("screenshots/{file_name}"),
        fingerprint: format!("{:016x}", pixel_fingerprint(&frame.pixels)),
    })
}

#[cfg(windows)]
/// Proves every captured BGR pixel came from the deterministic scrolling fixture, not a window on top.
fn validate_scroll_fixture_frame(
    frame: &CaptureFrame,
    fixture_bounds: PhysicalRect,
    offset: i32,
) -> io::Result<()> {
    const ROW_HEIGHT: i32 = 48;
    const COLUMN_WIDTH: i32 = 128;
    // A repainted GDI fixture can round capture channels by at most two levels on this path.
    // Larger differences indicate that another window or control contributed the pixel.
    const GDI_CHANNEL_TOLERANCE: u8 = 2;
    frame.validate()?;
    if frame.format != PixelFormat::Bgra8 || !rect_contains_rect(fixture_bounds, frame.bounds) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "scroll fixture {:?} cannot provide frame {:?} in {:?}",
                fixture_bounds, frame.bounds, frame.format
            ),
        ));
    }

    for y in 0..frame.height as usize {
        let screen_y = frame.bounds.top.saturating_add(y as i32);
        let fixture_y = screen_y.saturating_sub(fixture_bounds.top);
        let content_row =
            (fixture_y.div_euclid(ROW_HEIGHT) * ROW_HEIGHT + offset).div_euclid(ROW_HEIGHT);
        for x in 0..frame.width as usize {
            let screen_x = frame.bounds.left.saturating_add(x as i32);
            let fixture_x = screen_x.saturating_sub(fixture_bounds.left);
            let column = fixture_x.div_euclid(COLUMN_WIDTH);
            let color = scroll_fixture_color(content_row, column);
            let expected = [
                ((color >> 16) & 0xff) as u8,
                ((color >> 8) & 0xff) as u8,
                (color & 0xff) as u8,
            ];
            let index = y * frame.stride + x * 4;
            let actual = &frame.pixels[index..index + 3];
            let differs = actual
                .iter()
                .zip(expected)
                .any(|(observed, expected)| observed.abs_diff(expected) > GDI_CHANNEL_TOLERANCE);
            if differs {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "scroll fixture pixel mismatch at ({screen_x}, {screen_y}): expected BGR {expected:?}, observed {actual:?}"
                    ),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
fn save_scroll_frame_report(
    context: &WorkerContext,
    file_name: &str,
    frame: CaptureFrame,
) -> io::Result<ScrollFrameReport> {
    let path = context.session_root.join("screenshots").join(file_name);
    frame.save_png(&path)?;
    Ok(ScrollFrameReport {
        bounds: frame.bounds,
        width: frame.width,
        height: frame.height,
        screenshot: format!("screenshots/{file_name}"),
        fingerprint: format!("{:016x}", pixel_fingerprint(&frame.pixels)),
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

#[cfg(windows)]
#[derive(Clone, Copy, Debug)]
struct DesktopEvidence {
    fingerprint: u64,
    bounds: PhysicalRect,
}

#[cfg(windows)]
/// Captures a full-display sample and rechecks the foreground after capture to reject races.
fn capture_desktop_sample(context: &WorkerContext) -> io::Result<DesktopEvidence> {
    let bounds = context.display.physical_bounds;
    let before = unsafe { GetForegroundWindow() };
    let frame = SystemCaptureBackend.capture(bounds)?;
    let after = unsafe { GetForegroundWindow() };
    if before != after {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("foreground changed while sampling desktop ({before:?} -> {after:?})"),
        ));
    }
    Ok(DesktopEvidence {
        fingerprint: pixel_fingerprint(&frame.pixels),
        bounds,
    })
}

#[cfg(windows)]
/// Waits for two consecutive equal desktop samples after Save/Pin teardown settles.
fn wait_for_desktop_quiescence(
    context: &WorkerContext,
    stage: &str,
    screenshot_name: Option<&str>,
) -> io::Result<DesktopEvidence> {
    let deadline = Instant::now() + context.timeout;
    let mut previous = None;
    let mut stable_samples = 0_u8;
    let mut quiet_since = None;
    loop {
        let state = query_capture_state(
            context,
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_secs(1)),
        )?;
        let dialogs = visible_common_dialogs()?;
        if state.capture_teardown_pending
            || state.overlay_count != 0
            || state.pinned_count != 0
            || !dialogs.is_empty()
        {
            stable_samples = 0;
            previous = None;
            quiet_since = None;
        } else {
            let quiet_start = *quiet_since.get_or_insert_with(Instant::now);
            if quiet_start.elapsed() >= context.settle_delay.max(DESKTOP_QUIESCENCE_SETTLE) {
                let sample = capture_desktop_sample(context)?;
                let stable = previous.is_some_and(|old: DesktopEvidence| {
                    old.bounds == sample.bounds && old.fingerprint == sample.fingerprint
                });
                stable_samples = if stable {
                    stable_samples.saturating_add(1)
                } else {
                    0
                };
                previous = Some(sample);
                if stable_samples >= 2 {
                    if let Some(file_name) = screenshot_name {
                        let frame =
                            SystemCaptureBackend.capture(context.display.physical_bounds)?;
                        let path = context.session_root.join("screenshots").join(file_name);
                        frame.save_png(&path)?;
                    }
                    return Ok(sample);
                }
            }
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("timed out waiting for {stage} desktop quiescence"),
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(windows)]
fn record_desktop_step(
    report: &mut AcceptanceReport,
    report_path: &Path,
    action: &'static str,
    file_name: &str,
    evidence: &DesktopEvidence,
) -> io::Result<()> {
    report.steps.push(StepReport {
        action,
        foreground_window: unsafe { GetForegroundWindow() } as usize,
        screenshot: Some(format!("screenshots/{file_name}")),
        pixel_fingerprint: Some(format!("{:016x}", evidence.fingerprint)),
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
/// Finds the one visible process window created by the manual scrolling workflow.
fn wait_for_scroll_control(controller: *mut c_void, timeout: Duration) -> io::Result<NativeWindow> {
    let deadline = Instant::now() + timeout;
    loop {
        let candidates = process_windows()?
            .into_iter()
            .filter(|window| window.handle != controller)
            .collect::<Vec<_>>();
        if candidates.len() == 1 {
            return Ok(candidates[0]);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "scroll controller did not become the only visible acceptance window; found {}",
                    candidates.len()
                ),
            ));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(windows)]
/// Finds the centered image editor produced after the scrolling stitch completes.
fn wait_for_finished_image_overlay(
    controller: *mut c_void,
    closed_scroll_control: *mut c_void,
    timeout: Duration,
) -> io::Result<NativeWindow> {
    let deadline = Instant::now() + timeout;
    loop {
        let candidates = process_windows()?
            .into_iter()
            .filter(|window| window.handle != controller && window.handle != closed_scroll_control)
            .collect::<Vec<_>>();
        if candidates.len() == 1 {
            return Ok(candidates[0]);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "finished scroll image editor did not become the only visible workflow window; found {}",
                    candidates.len()
                ),
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
/// Accepts a closed overlay HWND even when Windows immediately reuses it for the resulting Pin.
fn wait_for_overlay_teardown(
    handle: *mut c_void,
    display_bounds: PhysicalRect,
    timeout: Duration,
    completed_action: &str,
) -> io::Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        // SAFETY: both calls only query the borrowed HWND value.
        if unsafe { IsWindow(handle) } == 0 || unsafe { IsWindowVisible(handle) } == 0 {
            return Ok(());
        }
        if let Ok(window) = owned_window(handle)
            && !overlay_covers_display(window, display_bounds)
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("full-display overlay remained visible after {completed_action}"),
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
/// Waits for exactly one newly opened process Pin, excluding Settings and all known Pin handles.
fn wait_for_new_pin(
    controller: *mut c_void,
    known: &[*mut c_void],
    timeout: Duration,
) -> io::Result<NativeWindow> {
    let deadline = Instant::now() + timeout;
    loop {
        let candidates = process_windows()?
            .into_iter()
            .filter(|window| window.handle != controller && !known.contains(&window.handle))
            .collect::<Vec<_>>();
        if candidates.len() > 1 {
            return Err(io::Error::other(
                "multiple new process windows appeared while creating one Pin",
            ));
        }
        if let Some(pin) = candidates.into_iter().next() {
            return Ok(pin);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "new Pin window did not appear",
            ));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(windows)]
/// Re-reads native bounds for a stable ordered HWND list and fails if any Pin disappeared.
fn owned_windows(handles: &[*mut c_void]) -> io::Result<Vec<NativeWindow>> {
    handles.iter().copied().map(owned_window).collect()
}

/// Computes a centered bottom row while preserving every measured native Pin size.
#[cfg(windows)]
fn horizontal_pin_layout(
    display: PhysicalRect,
    windows: &[NativeWindow],
) -> io::Result<Vec<PhysicalRect>> {
    if windows.len() != PIN_COEXIST_COUNT
        || display.right <= display.left
        || display.bottom <= display.top
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Pin row requires three windows and an increasing display rectangle",
        ));
    }
    let widths = windows
        .iter()
        .map(|window| i64::from(window.bounds.width()))
        .collect::<Vec<_>>();
    let heights = windows
        .iter()
        .map(|window| i64::from(window.bounds.height()))
        .collect::<Vec<_>>();
    if widths.iter().any(|width| *width <= 0) || heights.iter().any(|height| *height <= 0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Pin row contains an empty native window",
        ));
    }
    let total_width =
        widths.iter().sum::<i64>() + i64::from(PIN_COEXIST_LAYOUT_GAP) * (windows.len() as i64 - 1);
    let available_width = i64::from(display.width()) - i64::from(PIN_COEXIST_LAYOUT_MARGIN) * 2;
    let maximum_height = heights.iter().copied().max().unwrap_or_default();
    let available_height = i64::from(display.height()) - i64::from(PIN_COEXIST_LAYOUT_MARGIN) * 2;
    if total_width > available_width || maximum_height > available_height {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "display is too small for a non-overlapping three-Pin row",
        ));
    }

    let mut left = i64::from(display.left) + (i64::from(display.width()) - total_width) / 2;
    let bottom = i64::from(display.bottom) - i64::from(PIN_COEXIST_LAYOUT_MARGIN);
    let mut layout = Vec::with_capacity(windows.len());
    for (width, height) in widths.into_iter().zip(heights) {
        let right = left + width;
        let top = bottom - height;
        layout.push(PhysicalRect {
            left: i32::try_from(left)
                .map_err(|_| io::Error::other("Pin row left coordinate overflowed"))?,
            top: i32::try_from(top)
                .map_err(|_| io::Error::other("Pin row top coordinate overflowed"))?,
            right: i32::try_from(right)
                .map_err(|_| io::Error::other("Pin row right coordinate overflowed"))?,
            bottom: i32::try_from(bottom)
                .map_err(|_| io::Error::other("Pin row bottom coordinate overflowed"))?,
        });
        left = right + i64::from(PIN_COEXIST_LAYOUT_GAP);
    }
    Ok(layout)
}

#[cfg(windows)]
/// Moves a verified process-owned Pin without changing its size, z-order, or activation state.
fn move_owned_window(handle: *mut c_void, bounds: PhysicalRect) -> io::Result<()> {
    owned_window(handle)?;
    // SAFETY: the verified process-owned HWND remains borrowed; only its origin changes.
    if unsafe {
        SetWindowPos(
            handle,
            ptr::null_mut(),
            bounds.left,
            bounds.top,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(windows)]
fn validate_exact_window_bounds(
    windows: &[NativeWindow],
    expected: &[PhysicalRect],
    stage: &str,
) -> io::Result<()> {
    if windows.len() == expected.len()
        && windows
            .iter()
            .zip(expected)
            .all(|(window, bounds)| window.bounds == *bounds)
    {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{stage} did not retain the requested native bounds"),
        ))
    }
}

#[cfg(windows)]
fn validate_non_overlapping_windows(
    windows: &[NativeWindow],
    display: PhysicalRect,
) -> io::Result<()> {
    for (index, window) in windows.iter().enumerate() {
        let bounds = window.bounds;
        if bounds.left < display.left
            || bounds.top < display.top
            || bounds.right > display.right
            || bounds.bottom > display.bottom
        {
            return Err(io::Error::other(
                "Pin window moved outside the acceptance display",
            ));
        }
        for other in &windows[index + 1..] {
            if bounds.left < other.bounds.right
                && bounds.right > other.bounds.left
                && bounds.top < other.bounds.bottom
                && bounds.bottom > other.bounds.top
            {
                return Err(io::Error::other(
                    "Pin windows overlap after pointer movement",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
fn window_union_with_margin(
    windows: &[NativeWindow],
    display: PhysicalRect,
    margin: i32,
) -> io::Result<PhysicalRect> {
    let first = windows
        .first()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no Pin windows to capture"))?;
    let union = windows
        .iter()
        .skip(1)
        .fold(first.bounds, |union, window| PhysicalRect {
            left: union.left.min(window.bounds.left),
            top: union.top.min(window.bounds.top),
            right: union.right.max(window.bounds.right),
            bottom: union.bottom.max(window.bounds.bottom),
        });
    let bounds = PhysicalRect {
        left: union.left.saturating_sub(margin).max(display.left),
        top: union.top.saturating_sub(margin).max(display.top),
        right: union.right.saturating_add(margin).min(display.right),
        bottom: union.bottom.saturating_add(margin).min(display.bottom),
    };
    if bounds.right <= bounds.left || bounds.bottom <= bounds.top {
        return Err(io::Error::other("Pin screenshot union is empty"));
    }
    Ok(bounds)
}

#[cfg(windows)]
/// Compares every registered Pin source with its creation-time frame in the same stable order.
fn validate_pin_sources(
    context: &WorkerContext,
    expected: &[CaptureFrame],
    stage: &str,
) -> io::Result<()> {
    let observed =
        query_capture_content(context, context.timeout.min(Duration::from_secs(1)))?.pins;
    if observed.len() != expected.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{stage} exposed {} Pin frame(s), expected {}",
                observed.len(),
                expected.len()
            ),
        ));
    }
    for (index, (expected, observed)) in expected.iter().zip(&observed).enumerate() {
        validate_same_pixel_content(expected, observed, &format!("{stage} Pin {}", index + 1))?;
    }
    Ok(())
}

#[cfg(windows)]
fn validate_same_windows(
    expected: &[NativeWindow],
    observed: &[NativeWindow],
    stage: &str,
) -> io::Result<()> {
    if expected.len() == observed.len()
        && expected.iter().zip(observed).all(|(expected, observed)| {
            expected.handle == observed.handle && expected.bounds == observed.bounds
        })
    {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{stage} changed the Pin HWND set or native bounds"),
        ))
    }
}

#[cfg(windows)]
/// Waits until the HWND translation matches the actual cursor delta while preserving Pin size.
fn wait_for_window_drag(
    handle: *mut c_void,
    before: PhysicalRect,
    pointer_start: PhysicalPoint,
    pointer_end: PhysicalPoint,
    timeout: Duration,
) -> io::Result<PhysicalRect> {
    let deadline = Instant::now() + timeout;
    loop {
        let after = owned_window(handle)?.bounds;
        if window_drag_matches(before, after, pointer_start, pointer_end, 2) {
            return Ok(after);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("Pin pointer drag ended at {after:?}, starting from {before:?}"),
            ));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

/// Allows only small native rounding around the requested pointer translation.
fn window_drag_matches(
    before: PhysicalRect,
    after: PhysicalRect,
    pointer_start: PhysicalPoint,
    pointer_end: PhysicalPoint,
    tolerance: i32,
) -> bool {
    let expected_x = pointer_end.x - pointer_start.x;
    let expected_y = pointer_end.y - pointer_start.y;
    before.width() == after.width()
        && before.height() == after.height()
        && ((after.left - before.left) - expected_x).abs() <= tolerance
        && ((after.top - before.top) - expected_y).abs() <= tolerance
}

#[cfg(windows)]
/// Waits for the unambiguous process-owned common Save dialog opened by the overlay.
fn wait_for_save_dialog(
    overlay: *mut c_void,
    controller: *mut c_void,
    known_dialogs: &[*mut c_void],
    timeout: Duration,
) -> io::Result<NativeWindow> {
    let deadline = Instant::now() + timeout;
    loop {
        let candidates = process_windows()?
            .into_iter()
            .filter(|window| window.handle != overlay && window.handle != controller)
            .filter(|window| !known_dialogs.contains(&window.handle))
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
/// Waits until every visible process-owned common dialog is gone before the next UI evidence step.
fn wait_for_no_visible_save_dialogs(timeout: Duration, action: &str) -> io::Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        let dialogs = visible_common_dialogs()?;
        if dialogs.is_empty() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let handles = dialogs
                .iter()
                .map(|window| format!("{window:?}"))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("visible Save dialog(s) remained after {action}: {handles}"),
            ));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(windows)]
fn visible_common_dialogs() -> io::Result<Vec<*mut c_void>> {
    Ok(process_windows()?
        .into_iter()
        .filter(|window| window_class_name(window.handle).is_ok_and(|class| class == "#32770"))
        .map(|window| window.handle)
        .collect())
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
/// Replaces the native Save dialog filename without borrowing the clipboard, then verifies it.
fn set_save_dialog_path(dialog: &NativeWindow, target: &Path, timeout: Duration) -> io::Result<()> {
    let edit = save_file_name_edit(dialog.handle, "flash-shot.png")?;
    let edit_center = PhysicalPoint {
        x: edit.bounds.left + edit.bounds.width() as i32 / 2,
        y: edit.bounds.top + edit.bounds.height() as i32 / 2,
    };
    inject_mouse_click(dialog.handle, edit_center)?;
    wait_for_window_focus(dialog.handle, edit.handle, timeout)?;
    inject_select_all(dialog.handle)?;
    let target_text = target.to_string_lossy().into_owned();
    inject_unicode_text(dialog.handle, &target_text)?;
    wait_for_window_text(edit.handle, &target_text, timeout)
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
/// Waits for a new foreground process window after Pin, excluding the pre-click overlay/dialogs.
fn wait_for_new_pin_after_click(
    controller: *mut c_void,
    closed_overlay: *mut c_void,
    before: &[NativeWindow],
    display: PhysicalRect,
    timeout: Duration,
) -> io::Result<NativeWindow> {
    let mut known = before
        .iter()
        .map(|window| window.handle)
        .collect::<Vec<_>>();
    known.push(closed_overlay);
    let deadline = Instant::now() + timeout;
    loop {
        let candidates = process_windows()?
            .into_iter()
            .filter(|window| window.handle != controller)
            .filter(|window| !known.contains(&window.handle))
            .filter(|window| window_class_name(window.handle).is_ok_and(|class| class != "#32770"))
            .filter(|window| rect_inside_display(window.bounds, display))
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
fn rect_inside_display(bounds: PhysicalRect, display: PhysicalRect) -> bool {
    bounds.left >= display.left
        && bounds.top >= display.top
        && bounds.right <= display.right
        && bounds.bottom <= display.bottom
        && bounds.width() > 0
        && bounds.height() > 0
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
fn scroll_roundtrip_interaction_plan_for_window(
    handle: *mut c_void,
) -> io::Result<InteractionPlan> {
    let window = owned_window(handle)?;
    scroll_roundtrip_interaction_plan(
        client_bounds_for_window(handle)?,
        window.dpi as f32 / WINDOWS_BASE_DPI,
    )
}

#[cfg(windows)]
/// Recomputes the production toolbar from the latest committed physical selection.
fn interaction_plan_for_capture_selection(
    handle: *mut c_void,
    capture_bounds: PhysicalRect,
    selection: PhysicalRect,
) -> io::Result<InteractionPlan> {
    let window = owned_window(handle)?;
    let client = client_bounds_for_window(handle)?;
    let scale = window.dpi as f32 / WINDOWS_BASE_DPI;
    let (width, height) = overlay_logical_size(client, scale)?;
    let top_left = map_capture_point_to_screen(
        PhysicalPoint {
            x: selection.left,
            y: selection.top,
        },
        client,
        capture_bounds,
    )?;
    let bottom_right = map_capture_point_to_screen(
        PhysicalPoint {
            x: selection.right,
            y: selection.bottom,
        },
        client,
        capture_bounds,
    )?;
    let logical = |point: PhysicalPoint| {
        (
            (point.x - client.left) as f32 / scale,
            (point.y - client.top) as f32 / scale,
        )
    };
    interaction_plan_for_logical_selection(
        client,
        scale,
        width,
        height,
        logical(top_left),
        logical(bottom_right),
    )
}

#[cfg(windows)]
fn scroll_shot_point_for_capture_selection(
    handle: *mut c_void,
    capture_bounds: PhysicalRect,
    selection: PhysicalRect,
) -> io::Result<PhysicalPoint> {
    let window = owned_window(handle)?;
    let client = client_bounds_for_window(handle)?;
    let scale = window.dpi as f32 / WINDOWS_BASE_DPI;
    let (width, height) = overlay_logical_size(client, scale)?;
    let top_left = map_capture_point_to_screen(
        PhysicalPoint {
            x: selection.left,
            y: selection.top,
        },
        client,
        capture_bounds,
    )?;
    let bottom_right = map_capture_point_to_screen(
        PhysicalPoint {
            x: selection.right,
            y: selection.bottom,
        },
        client,
        capture_bounds,
    )?;
    let logical = |point: PhysicalPoint| {
        (
            (point.x - client.left) as f32 / scale,
            (point.y - client.top) as f32 / scale,
        )
    };
    scroll_shot_point_for_logical_selection(
        client,
        scale,
        width,
        height,
        logical(top_left),
        logical(bottom_right),
    )
}

#[cfg(windows)]
/// Re-measures the overlay before placing the real bottom-right narrow-selection controls.
fn narrow_edge_interaction_plan_for_window(
    handle: *mut c_void,
) -> io::Result<NarrowEdgeInteractionPlan> {
    let window = owned_window(handle)?;
    narrow_edge_interaction_plan(
        client_bounds_for_window(handle)?,
        window.dpi as f32 / WINDOWS_BASE_DPI,
    )
}

#[cfg(windows)]
fn pin_coexist_interaction_plan_for_window(
    handle: *mut c_void,
    index: usize,
) -> io::Result<InteractionPlan> {
    let window = owned_window(handle)?;
    pin_coexist_interaction_plan(
        client_bounds_for_window(handle)?,
        window.dpi as f32 / WINDOWS_BASE_DPI,
        index,
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
/// Confirms that acceptance-owned key and button presses left no global held-input state behind.
fn ensure_input_keys_released(keys: &[(u16, &str)]) -> io::Result<()> {
    let held = keys
        .iter()
        .filter_map(|(virtual_key, name)| {
            ((unsafe { GetAsyncKeyState(*virtual_key as i32) } as u16 & 0x8000) != 0)
                .then_some(*name)
        })
        .collect::<Vec<_>>();
    if held.is_empty() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "scroll roundtrip left injected input held: {}",
                held.join(", ")
            ),
        ))
    }
}

#[cfg(windows)]
/// Polls briefly because Windows may expose the final key-up one scheduler turn after SendInput.
fn wait_for_input_keys_released(keys: &[(u16, &str)], timeout: Duration) -> io::Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        match ensure_input_keys_released(keys) {
            Ok(()) => return Ok(()),
            Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            Err(error) => return Err(error),
        }
    }
}

#[cfg(windows)]
/// Waits for a child marker while keeping the acceptance deadline finite and observable.
fn wait_for_path(path: &Path, timeout: Duration, label: &str) -> io::Result<bool> {
    let deadline = Instant::now() + timeout;
    loop {
        if path.is_file() {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("{label} did not appear: {}", path.display()),
            ));
        }
        thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(windows)]
/// Tries a verified window's non-client title bar and reports when borderless geometry has none.
fn activate_owned_window_via_titlebar(window: NativeWindow) -> io::Result<bool> {
    if left_button_held() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "left mouse button is already held; title-bar activation was aborted",
        ));
    }
    let client = client_bounds_for_window(window.handle)?;
    if !has_safe_titlebar_band(window.bounds, client) {
        return Ok(false);
    }
    let Some(point) = find_visible_titlebar_point(window, client)? else {
        // Borderless GPUI windows have no safe non-client point. Keep trying the
        // verified API activation path instead of moving the user's cursor or failing.
        return Ok(false);
    };
    let cursor = CursorRestore::capture()?;
    let desktop = virtual_desktop()?;
    let activation = (|| -> io::Result<()> {
        send_input_unchecked(&[absolute_mouse_input(point, MOUSEEVENTF_MOVE, desktop)])?;
        send_titlebar_click(window.handle)
    })();
    let cursor_restore = cursor.restore();
    match (activation, cursor_restore) {
        (Ok(()), Ok(())) => Ok(true),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(io::Error::other(format!(
            "window activation succeeded, but restoring the cursor failed: {error}"
        ))),
        (Err(error), Err(restore_error)) => Err(io::Error::new(
            error.kind(),
            format!("{error}; restoring the cursor also failed: {restore_error}"),
        )),
    }
}

#[cfg(windows)]
/// Keeps borderless overlays on the API-only retry path instead of treating no title bar as fatal.
fn has_safe_titlebar_band(window: PhysicalRect, client: PhysicalRect) -> bool {
    client.top > window.top.saturating_add(4)
}

#[cfg(windows)]
/// Defers cursor-moving activation so GPUI's own asynchronous focus request can settle first.
fn titlebar_fallback_ready(elapsed: Duration, attempted: bool) -> bool {
    !attempted && elapsed >= FOCUS_TITLEBAR_FALLBACK_DELAY
}

#[cfg(windows)]
/// Finds an uncovered point in the middle half of a title bar, away from its icon and buttons.
fn find_visible_titlebar_point(
    window: NativeWindow,
    client: PhysicalRect,
) -> io::Result<Option<PhysicalPoint>> {
    let width = window.bounds.width() as i32;
    let left = window.bounds.left.saturating_add(width / 4);
    let right = window.bounds.right.saturating_sub(width / 4);
    let center = window.bounds.left.saturating_add(width / 2);
    let y = window.bounds.top + (client.top - window.bounds.top) / 2;
    let step = 8_i32;
    let max_steps = ((right - left) / step).max(0);

    for index in 0..=max_steps {
        let offset = ((index + 1) / 2) * step;
        let x = if index % 2 == 0 {
            center.saturating_add(offset)
        } else {
            center.saturating_sub(offset)
        };
        if x < left || x > right {
            continue;
        }
        let point = PhysicalPoint { x, y };
        if is_caption_point(window.handle, point)? {
            return Ok(Some(PhysicalPoint { x, y }));
        }
    }

    Ok(None)
}

#[cfg(windows)]
fn left_button_held() -> bool {
    (unsafe { GetAsyncKeyState(VK_LBUTTON as i32) } as u16 & 0x8000) != 0
}

#[cfg(windows)]
/// Checks both z-order ownership and non-client hit testing for one physical screen point.
fn is_caption_point(expected: HWND, point: PhysicalPoint) -> io::Result<bool> {
    let native_point = POINT {
        x: point.x,
        y: point.y,
    };
    let hit = unsafe { WindowFromPoint(native_point) };
    if hit != expected {
        return Ok(false);
    }
    owned_window(expected)?;
    let packed_point = ((point.y as u32 & 0xffff) << 16) | (point.x as u32 & 0xffff);
    let hit_test = unsafe { SendMessageW(expected, WM_NCHITTEST, 0, packed_point as isize) };
    Ok(hit_test == HTCAPTION as isize)
}

#[cfg(windows)]
/// Reads the rounded cursor position and verifies it is still a safe caption on the owned window.
fn guard_current_caption_target(expected: HWND) -> io::Result<PhysicalPoint> {
    owned_window(expected)?;
    let mut point = POINT::default();
    if unsafe { GetCursorPos(&mut point) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let point = PhysicalPoint {
        x: point.x,
        y: point.y,
    };
    if !is_caption_point(expected, point)? {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("actual cursor point {point:?} is not the expected process window's caption"),
        ));
    }
    Ok(point)
}

#[cfg(windows)]
/// Clicks once and releases only when Windows confirms this batch inserted the button-down event.
fn send_titlebar_click(expected: HWND) -> io::Result<()> {
    guard_current_caption_target(expected)?;
    if left_button_held() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "left mouse button became held before title-bar activation",
        ));
    }
    let inputs = [
        mouse_button_input(MOUSEEVENTF_LEFTDOWN),
        mouse_button_input(MOUSEEVENTF_LEFTUP),
    ];
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    };
    if sent == inputs.len() as u32 {
        return if left_button_held() {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "left mouse button remained held after title-bar activation",
            ))
        } else {
            Ok(())
        };
    }

    let click_error = io::Error::last_os_error();
    let release_error = if sent == 1 {
        send_input_unchecked(&[mouse_button_input(MOUSEEVENTF_LEFTUP)]).err()
    } else {
        None
    };
    let still_held = left_button_held();
    let detail = match (release_error, still_held) {
        (Some(error), true) => format!(
            "title-bar click inserted {sent}/2 events ({click_error}); releasing the accepted button-down failed ({error}) and the button is still held"
        ),
        (Some(error), false) => format!(
            "title-bar click inserted {sent}/2 events ({click_error}); defensive release reported {error}"
        ),
        (None, true) => format!(
            "title-bar click inserted {sent}/2 events ({click_error}); the left button is still held"
        ),
        (None, false) => {
            format!("title-bar click inserted {sent}/2 events: {click_error}")
        }
    };
    Err(io::Error::other(detail))
}

#[cfg(windows)]
/// Raises a verified window, then waits until Windows confirms foreground ownership.
fn focus_owned_window(window: NativeWindow, timeout: Duration) -> io::Result<()> {
    owned_window(window.handle)?;
    let started = Instant::now();
    let deadline = started + timeout.min(Duration::from_secs(3));
    let mut titlebar_fallback_attempted = false;
    // SAFETY: both calls borrow a verified process-owned HWND without transferring ownership.
    unsafe {
        ShowWindow(window.handle, SW_RESTORE);
        BringWindowToTop(window.handle);
        // Reassert activation with the measured rectangle; unlike SWP_NOACTIVATE this also
        // works when a fresh GPUI popup was created by a background acceptance worker thread.
        SetWindowPos(
            window.handle,
            HWND_TOP,
            window.bounds.left,
            window.bounds.top,
            window.bounds.width() as i32,
            window.bounds.height() as i32,
            SWP_SHOWWINDOW,
        );
        SetForegroundWindow(window.handle);
    }
    if unsafe { GetForegroundWindow() } != window.handle {
        // Switch only to the process-owned HWND already validated above. Unlike an Alt tap, this
        // cannot leak modifier state into whichever unrelated window currently owns input.
        unsafe { SwitchToThisWindow(window.handle, 1) };
    }
    loop {
        if guard_foreground(window.handle).is_ok() {
            return Ok(());
        }
        let now = Instant::now();
        if now >= deadline {
            let foreground = unsafe { GetForegroundWindow() };
            let mut foreground_process = 0;
            unsafe { GetWindowThreadProcessId(foreground, &mut foreground_process) };
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "acceptance window could not become foreground (expected {:?}, observed {:?}, process {foreground_process}); input injection was aborted",
                    window.handle, foreground
                ),
            ));
        }
        if titlebar_fallback_ready(now.duration_since(started), titlebar_fallback_attempted) {
            titlebar_fallback_attempted = true;
            // GPUI popups are intentionally borderless. Continue the process-owned API retry
            // loop when there is no non-client band instead of failing before GPUI settles.
            if activate_owned_window_via_titlebar(window)? {
                continue;
            }
        }
        // Foreground policy may briefly defer activation while a popup is being created. Retry
        // only the verified process-owned HWND; never send a modifier to the unrelated window
        // that currently owns global input.
        unsafe {
            BringWindowToTop(window.handle);
            SetForegroundWindow(window.handle);
            SwitchToThisWindow(window.handle, 1);
        }
        thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(windows)]
fn inject_capture_shortcut(expected: *mut c_void) -> io::Result<NativeWindow> {
    let foreground = guard_foreground(expected)?;
    // Keep the global shortcut independent from a user's current modifier state, then verify
    // every injected key is released so a partial SendInput batch cannot poison the next step.
    ensure_input_keys_released(&[(VK_CONTROL, "Control"), (VK_MENU, "Alt"), (VK_F24, "F24")])?;
    let inputs = [
        keyboard_input(VK_CONTROL, false),
        keyboard_input(VK_MENU, false),
        keyboard_input(VK_F24, false),
        keyboard_input(VK_F24, true),
        keyboard_input(VK_MENU, true),
        keyboard_input(VK_CONTROL, true),
    ];
    send_input_batch_with_cleanup(
        expected,
        &inputs,
        &[
            keyboard_input(VK_F24, true),
            keyboard_input(VK_MENU, true),
            keyboard_input(VK_CONTROL, true),
        ],
    )?;
    wait_for_input_keys_released(
        &[(VK_CONTROL, "Control"), (VK_MENU, "Alt"), (VK_F24, "F24")],
        Duration::from_millis(250),
    )?;
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
/// Sends Ctrl+S only from a neutral key state and waits until both injected keys are released.
fn inject_ctrl_s(expected: *mut c_void) -> io::Result<NativeWindow> {
    let foreground = guard_foreground(expected)?;
    ensure_input_keys_released(&[(VK_S, "S"), (VK_CONTROL, "Control")])?;
    let inputs = [
        keyboard_input(VK_CONTROL, false),
        keyboard_input(VK_S, false),
        keyboard_input(VK_S, true),
        keyboard_input(VK_CONTROL, true),
    ];
    let cleanup = [keyboard_input(VK_S, true), keyboard_input(VK_CONTROL, true)];
    send_input_batch_with_cleanup(expected, &inputs, &cleanup)?;
    wait_for_input_keys_released(
        &[(VK_S, "S"), (VK_CONTROL, "Control")],
        Duration::from_millis(250),
    )?;
    Ok(foreground)
}

#[cfg(windows)]
/// Activates the scroll controller's explicit Shift+Space auto-capture command.
fn inject_scroll_auto_capture(expected: *mut c_void) -> io::Result<NativeWindow> {
    let foreground = guard_foreground(expected)?;
    let inputs = [
        keyboard_input(VK_SHIFT, false),
        keyboard_input(VK_SPACE, false),
        keyboard_input(VK_SPACE, true),
        keyboard_input(VK_SHIFT, true),
    ];
    let cleanup = [
        keyboard_input(VK_SPACE, true),
        keyboard_input(VK_SHIFT, true),
    ];
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
/// Moves away from a clicked control while preserving the same foreground and hit-test guards.
fn inject_mouse_move(expected: *mut c_void, point: PhysicalPoint) -> io::Result<NativeWindow> {
    let foreground = guard_foreground(expected)?;
    let desktop = virtual_desktop()?;
    send_input_batch(
        expected,
        &[absolute_mouse_input(point, MOUSEEVENTF_MOVE, desktop)],
    )?;
    guard_current_pointer_target(expected)?;
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
/// Performs the same guarded toolbar click but records time immediately before button-down input.
fn inject_copy_click(
    expected: *mut c_void,
    point: PhysicalPoint,
) -> io::Result<(NativeWindow, u64)> {
    let foreground = guard_foreground(expected)?;
    let desktop = virtual_desktop()?;
    send_input_batch(
        expected,
        &[absolute_mouse_input(point, MOUSEEVENTF_MOVE, desktop)],
    )?;
    guard_current_pointer_target(expected)?;
    let input_started_at = qpc_ticks()?;
    send_input_batch_with_cleanup(
        expected,
        &[
            mouse_button_input(MOUSEEVENTF_LEFTDOWN),
            mouse_button_input(MOUSEEVENTF_LEFTUP),
        ],
        &[mouse_button_input(MOUSEEVENTF_LEFTUP)],
    )?;
    Ok((foreground, input_started_at))
}

#[cfg(windows)]
/// Injects the selected Copy gesture and returns the guarded foreground plus its QPC start tick.
/// Enter reuses the same foreground and partial-input cleanup checks as every other key action.
fn inject_copy_trigger(
    expected: *mut c_void,
    toolbar_point: PhysicalPoint,
    trigger: CopyTriggerOption,
) -> io::Result<(NativeWindow, u64)> {
    match trigger {
        CopyTriggerOption::Toolbar => inject_copy_click(expected, toolbar_point),
        CopyTriggerOption::Enter => {
            let foreground = guard_foreground(expected)?;
            ensure_input_keys_released(&[(VK_RETURN, "Enter")])?;
            let input_started_at = qpc_ticks()?;
            let inputs = [
                keyboard_input(VK_RETURN, false),
                keyboard_input(VK_RETURN, true),
            ];
            let cleanup = [keyboard_input(VK_RETURN, true)];
            send_input_batch_with_cleanup(expected, &inputs, &cleanup)?;
            if let Err(error) =
                wait_for_input_keys_released(&[(VK_RETURN, "Enter")], Duration::from_millis(250))
            {
                let cleanup_error = send_input_unchecked(&cleanup).err();
                return Err(match cleanup_error {
                    Some(cleanup_error) => io::Error::new(
                        error.kind(),
                        format!(
                            "Enter remained held after Copy input ({error}); defensive release also failed: {cleanup_error}"
                        ),
                    ),
                    None => error,
                });
            }
            Ok((foreground, input_started_at))
        }
    }
}

#[cfg(windows)]
/// Releases only the modifiers owned by one guarded drag, even when SendInput or focus fails.
struct ModifierReleaseGuard {
    modifiers: DragModifiers,
    modifiers_armed: bool,
    mouse_button_armed: bool,
}

#[cfg(windows)]
impl ModifierReleaseGuard {
    fn new(modifiers: DragModifiers) -> Self {
        Self {
            modifiers,
            // These flags are armed only after their corresponding down input is accepted. If
            // foreground validation fails before injection, cleanup must not release user input.
            modifiers_armed: false,
            mouse_button_armed: false,
        }
    }

    fn modifier_cleanup_inputs(&self) -> Vec<INPUT> {
        let mut inputs = Vec::new();
        if self.modifiers.alt {
            inputs.push(keyboard_input(VK_MENU, true));
        }
        if self.modifiers.shift {
            inputs.push(keyboard_input(VK_SHIFT, true));
        }
        inputs
    }

    fn action_cleanup_inputs(&self) -> Vec<INPUT> {
        let mut inputs = vec![mouse_button_input(MOUSEEVENTF_LEFTUP)];
        inputs.extend(self.modifier_cleanup_inputs());
        inputs
    }

    fn armed_cleanup_inputs(&self) -> Vec<INPUT> {
        let mut inputs = Vec::new();
        if self.mouse_button_armed {
            inputs.push(mouse_button_input(MOUSEEVENTF_LEFTUP));
        }
        if self.modifiers_armed {
            inputs.extend(self.modifier_cleanup_inputs());
        }
        inputs
    }

    fn arm_modifiers(&mut self) {
        self.modifiers_armed = self.modifiers.alt || self.modifiers.shift;
    }

    fn arm_mouse_button(&mut self) {
        self.mouse_button_armed = true;
    }

    fn disarm(&mut self) {
        self.modifiers_armed = false;
        self.mouse_button_armed = false;
    }
}

#[cfg(windows)]
impl Drop for ModifierReleaseGuard {
    fn drop(&mut self) {
        let cleanup = self.armed_cleanup_inputs();
        if !cleanup.is_empty() {
            let _ = send_input_unchecked(&cleanup);
        }
    }
}

#[cfg(windows)]
/// Performs one committed-selection drag with real modifier key state and release-safe cleanup.
fn inject_selection_transform_drag(
    expected: *mut c_void,
    gesture: SelectionTransformGesture,
    capture_bounds: PhysicalRect,
) -> io::Result<InjectedSelectionTransform> {
    guard_foreground(expected)?;
    let desktop = virtual_desktop()?;
    let client_bounds = client_bounds_for_window(expected)?;
    let screen_start = map_capture_point_to_screen(gesture.start, client_bounds, capture_bounds)?;
    let screen_end = map_capture_point_to_screen(gesture.end, client_bounds, capture_bounds)?;

    send_input_batch(
        expected,
        &[absolute_mouse_input(
            screen_start,
            MOUSEEVENTF_MOVE,
            desktop,
        )],
    )?;
    let actual_start = guard_current_pointer_target(expected)?;
    validate_point_geometry(
        screen_start,
        actual_start,
        &format!("{} pointer start", gesture.kind.label()),
    )?;

    let mut modifiers = ModifierReleaseGuard::new(gesture.kind.modifiers());
    let mut inputs = Vec::with_capacity(3);
    if gesture.kind.modifiers().shift {
        inputs.push(keyboard_input(VK_SHIFT, false));
    }
    if gesture.kind.modifiers().alt {
        inputs.push(keyboard_input(VK_MENU, false));
    }
    let modifier_cleanup = modifiers.modifier_cleanup_inputs();
    if !inputs.is_empty() {
        // Keep modifier-down separate from the mouse button so a menu/focus transition cannot
        // turn the following click into an input delivered to an unexpected window.
        send_modifier_batch_with_cleanup(expected, &inputs, &modifier_cleanup)?;
        modifiers.arm_modifiers();
        guard_current_pointer_target(expected)?;
    }
    let action_cleanup = modifiers.action_cleanup_inputs();
    send_input_batch_with_cleanup(
        expected,
        &[mouse_button_input(MOUSEEVENTF_LEFTDOWN)],
        &action_cleanup,
    )?;
    modifiers.arm_mouse_button();

    let movement_result = (|| {
        for step in 1..=8 {
            guard_current_pointer_target(expected)?;
            let fraction = step as i64;
            let point = PhysicalPoint {
                x: (i64::from(screen_start.x)
                    + (i64::from(screen_end.x) - i64::from(screen_start.x)) * fraction / 8)
                    as i32,
                y: (i64::from(screen_start.y)
                    + (i64::from(screen_end.y) - i64::from(screen_start.y)) * fraction / 8)
                    as i32,
            };
            send_input_batch(
                expected,
                &[absolute_mouse_input(point, MOUSEEVENTF_MOVE, desktop)],
            )?;
        }
        guard_current_pointer_target(expected)
    })();
    let release_result = send_input_unchecked(&modifiers.action_cleanup_inputs());
    if release_result.is_ok() {
        modifiers.disarm();
    }
    let actual_end = match (movement_result, release_result) {
        (Ok(point), Ok(())) => point,
        (Err(error), _) => return Err(error),
        (Ok(_), Err(error)) => return Err(error),
    };

    let final_client_bounds = client_bounds_for_window(expected)?;
    if final_client_bounds != client_bounds {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "overlay client bounds changed during {} from {client_bounds:?} to {final_client_bounds:?}",
                gesture.kind.label()
            ),
        ));
    }
    let capture_start = map_screen_point_to_capture(actual_start, client_bounds, capture_bounds)?;
    let capture_end = map_screen_point_to_capture(actual_end, client_bounds, capture_bounds)?;
    Ok(InjectedSelectionTransform {
        start: capture_start,
        end: capture_end,
    })
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
/// Drags one verified Pin image through native hit testing and returns the actual cursor endpoints.
fn inject_native_window_drag(
    expected: *mut c_void,
    start: PhysicalPoint,
    end: PhysicalPoint,
) -> io::Result<InjectedPointerDrag> {
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
            let point = PhysicalPoint {
                x: start.x + (end.x - start.x) * step / 8,
                y: start.y + (end.y - start.y) * step / 8,
            };
            send_input_batch(
                expected,
                &[absolute_mouse_input(point, MOUSEEVENTF_MOVE, desktop)],
            )?;
            // Native caption dragging moves the HWND on its GUI thread. Give that loop one short
            // dispatch interval before requiring the window to remain beneath the cursor.
            thread::sleep(Duration::from_millis(15));
            guard_current_pointer_target(expected)?;
        }
        guard_current_pointer_target(expected)
    })();
    let release_result = send_input_unchecked(&[mouse_button_input(MOUSEEVENTF_LEFTUP)]);
    let actual_end = match (movement_result, release_result) {
        (Ok(point), Ok(())) => point,
        (Err(error), _) => return Err(error),
        (Ok(_), Err(error)) => return Err(error),
    };
    Ok(InjectedPointerDrag {
        foreground,
        start: actual_start,
        end: actual_end,
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
/// Submits one guarded batch and reports any failure to release action-specific inputs.
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
    let cleanup_result = send_input_unchecked(&cleanup);
    match (result, cleanup_result) {
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(cleanup_error)) => Err(io::Error::new(
            error.kind(),
            format!("{error}; defensive input cleanup also failed: {cleanup_error}"),
        )),
        (Ok(()), _) => unreachable!("successful batches return before defensive cleanup"),
    }
}

#[cfg(windows)]
/// Sends modifier-only input without releasing unrelated global buttons or hotkey keys on error.
fn send_modifier_batch_with_cleanup(
    expected: *mut c_void,
    inputs: &[INPUT],
    modifier_cleanup: &[INPUT],
) -> io::Result<()> {
    guard_foreground(expected)?;
    let result = send_input_unchecked(inputs);
    if result.is_ok() {
        return Ok(());
    }
    let _ = send_input_unchecked(modifier_cleanup);
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
        if left_button_held() {
            // Moving while a title-bar click is still held would drag a native window. Leave the
            // pointer in place and make the unsafe cleanup state explicit to the caller instead.
            self.restored = true;
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "cursor was not restored because the left mouse button is still held",
            ));
        }
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
        if !self.restored && !left_button_held() {
            // SAFETY: the coordinates came from GetCursorPos; Drop is a best-effort fallback.
            unsafe { SetCursorPos(self.original.x, self.original.y) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CaptureScenarioOption, CopyTriggerOption, DEFAULT_OUTPUT_DIR, Options, RecordTargetOption,
        ScrollExportOption, SelectionTransformKind, ensure_input_authorized,
        expected_selection_transform, first_stable_recording_match, interaction_command_channel,
        interaction_plan, map_capture_point_to_screen, map_screen_point_to_capture,
        map_screen_selection_to_capture, narrow_edge_interaction_plan, normalize_axis,
        pin_coexist_interaction_plan, recording_control_plan, recording_failed, recording_saved,
        rect_contains_rect, scroll_roundtrip_cleanup_complete, scroll_roundtrip_interaction_plan,
        scroll_shot_point_for_logical_selection, selection_aspect_ratio_preserved,
        selection_center_preserved, selection_transform_gesture, translated_rect,
        validate_distinct_recording_phase_fingerprints, validate_paused_progress,
        validate_recorded_media, validate_recording_target_bounds, validate_selection_geometry,
        window_drag_matches,
    };
    #[cfg(windows)]
    use super::{
        FixturePhaseState, NativeWindow, decode_clipboard_dib, has_safe_titlebar_band,
        horizontal_pin_layout, panic_payload_message, parse_recording_window_fixture_arguments,
        recording_fixture_dynamic_bounds, request_recording_state, titlebar_fallback_ready,
        validate_fixture_phase_state, validate_recording_frame_content,
        validate_same_pixel_content, validate_scroll_fixture_frame,
    };
    use super::{MediaMetadata, OverlayInteractionCaptureState, OverlayInteractionRecordingState};
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

    #[cfg(windows)]
    #[test]
    fn borderless_focus_uses_api_retries_without_requiring_a_titlebar() {
        let window = PhysicalRect {
            left: -1920,
            top: 0,
            right: 0,
            bottom: 1080,
        };
        let borderless_client = window;
        let framed_client = PhysicalRect { top: 31, ..window };

        assert!(!has_safe_titlebar_band(window, borderless_client));
        assert!(has_safe_titlebar_band(window, framed_client));
        assert!(!titlebar_fallback_ready(Duration::from_millis(249), false));
        assert!(titlebar_fallback_ready(Duration::from_millis(250), false));
        assert!(!titlebar_fallback_ready(Duration::from_secs(1), true));
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
        assert_eq!(options.capture_scenario, CaptureScenarioOption::Standard);
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
    fn parser_accepts_only_isolated_capture_scenarios() {
        let options = Options::parse_from(arguments(&[
            "--allow-input",
            "--capture-scenario",
            "narrow-edge",
        ]))
        .unwrap();
        assert_eq!(options.capture_scenario, CaptureScenarioOption::NarrowEdge);
        assert_eq!(options.record_target, None);

        let pins = Options::parse_from(arguments(&[
            "--allow-input",
            "--capture-scenario",
            "pins-coexist",
        ]))
        .unwrap();
        assert_eq!(pins.capture_scenario, CaptureScenarioOption::PinsCoexist);
        assert_eq!(pins.capture_scenario.workflow(), "capture_pins_coexist");

        assert!(
            Options::parse_from(arguments(&[
                "--capture-scenario",
                "narrow-edge",
                "--capture-scenario",
                "narrow-edge",
            ]))
            .is_err()
        );
        assert!(Options::parse_from(arguments(&["--capture-scenario", "wide"])).is_err());
        assert!(
            Options::parse_from(arguments(&[
                "--capture-scenario",
                "narrow-edge",
                "--record-target",
                "area",
            ]))
            .is_err()
        );
    }

    #[test]
    fn parser_accepts_selection_transform_scenario() {
        let options = Options::parse_from(arguments(&[
            "--allow-input",
            "--capture-scenario",
            "selection-transform",
        ]))
        .unwrap();

        assert_eq!(
            options.capture_scenario,
            CaptureScenarioOption::SelectionTransform
        );
        assert_eq!(
            options.capture_scenario.workflow(),
            "capture_selection_transform"
        );
    }

    #[test]
    fn parser_accepts_scroll_roundtrip_scenario() {
        let options = Options::parse_from(arguments(&[
            "--allow-input",
            "--capture-scenario",
            "scroll-roundtrip",
        ]))
        .unwrap();

        assert_eq!(
            options.capture_scenario,
            CaptureScenarioOption::ScrollRoundtrip
        );
        assert_eq!(
            options.capture_scenario.workflow(),
            "capture_scroll_roundtrip"
        );
        assert!(options.capture_scenario.requires_100_percent_display());
        assert_eq!(options.scroll_export, ScrollExportOption::Cancel);
        assert!(!options.allow_system_clipboard);
    }

    #[test]
    fn parser_gates_standard_system_clipboard_and_copy_trigger() {
        let options =
            Options::parse_from(arguments(&["--allow-input", "--allow-system-clipboard"])).unwrap();

        assert_eq!(options.capture_scenario, CaptureScenarioOption::Standard);
        assert!(options.allow_system_clipboard);
        assert_eq!(options.copy_trigger, CopyTriggerOption::Toolbar);

        let enter = Options::parse_from(arguments(&[
            "--allow-input",
            "--allow-system-clipboard",
            "--copy-trigger",
            "enter",
        ]))
        .unwrap();
        assert_eq!(enter.copy_trigger, CopyTriggerOption::Enter);
        assert_eq!(enter.copy_trigger.label(), "enter");

        let toolbar =
            Options::parse_from(arguments(&["--allow-input", "--copy-trigger", "toolbar"]))
                .unwrap();
        assert_eq!(toolbar.copy_trigger, CopyTriggerOption::Toolbar);

        for forbidden in [
            vec![
                "--capture-scenario",
                "narrow-edge",
                "--allow-system-clipboard",
            ],
            vec![
                "--capture-scenario",
                "pins-coexist",
                "--allow-system-clipboard",
            ],
            vec![
                "--capture-scenario",
                "selection-transform",
                "--allow-system-clipboard",
            ],
            vec!["--record-target", "area", "--allow-system-clipboard"],
            vec!["--record-target", "window", "--allow-system-clipboard"],
        ] {
            let mut args = vec!["--allow-input"];
            args.extend(forbidden);
            assert!(Options::parse_from(arguments(&args)).is_err());
        }
        assert!(Options::parse_from(arguments(&["--copy-trigger", "space"])).is_err());
        assert!(
            Options::parse_from(arguments(&[
                "--copy-trigger",
                "enter",
                "--copy-trigger",
                "toolbar"
            ]))
            .is_err()
        );
        assert!(
            Options::parse_from(arguments(&[
                "--capture-scenario",
                "scroll-roundtrip",
                "--copy-trigger",
                "enter",
            ]))
            .is_err()
        );
    }

    #[test]
    fn parser_gates_scroll_exports_and_system_clipboard_mutation() {
        let copy = Options::parse_from(arguments(&[
            "--allow-input",
            "--capture-scenario",
            "scroll-roundtrip",
            "--scroll-export",
            "copy",
            "--allow-system-clipboard",
        ]))
        .unwrap();
        assert_eq!(copy.scroll_export, ScrollExportOption::Copy);
        assert!(copy.allow_system_clipboard);

        let save = Options::parse_from(arguments(&[
            "--capture-scenario",
            "scroll-roundtrip",
            "--scroll-export",
            "save",
        ]))
        .unwrap();
        assert_eq!(save.scroll_export, ScrollExportOption::Save);
        assert!(!save.allow_system_clipboard);

        assert!(
            Options::parse_from(arguments(&[
                "--capture-scenario",
                "scroll-roundtrip",
                "--scroll-export",
                "copy",
            ]))
            .is_err()
        );
        assert!(Options::parse_from(arguments(&["--allow-system-clipboard"])).is_ok());
        assert!(Options::parse_from(arguments(&["--scroll-export", "cancel"])).is_err());
        assert!(
            Options::parse_from(arguments(&[
                "--capture-scenario",
                "scroll-roundtrip",
                "--scroll-export",
                "save",
                "--allow-system-clipboard",
            ]))
            .is_err()
        );
        assert!(
            Options::parse_from(arguments(&[
                "--capture-scenario",
                "scroll-roundtrip",
                "--scroll-export",
                "save",
                "--scroll-export",
                "cancel",
            ]))
            .is_err()
        );
    }

    #[test]
    fn scroll_cleanup_requires_every_background_and_overlay_flag_to_be_idle() {
        let selection = PhysicalRect {
            left: 10,
            top: 20,
            right: 110,
            bottom: 220,
        };
        let clean = OverlayInteractionCaptureState {
            session_state: "completed".to_owned(),
            selection: Some(selection),
            manual_scroll_state: "idle".to_owned(),
            manual_scroll_frame_count: 0,
            manual_scroll_can_finish: false,
            manual_scroll_capture_in_flight: false,
            manual_scroll_auto_capture_pending: false,
            manual_scroll_selection: None,
            overlay_count: 0,
            more_actions_visible: false,
            annotation_controls_visible: false,
            pinned_count: 0,
            pinned_source_bounds: None,
            capture_teardown_pending: false,
            capture_preflight_ready: true,
            status: "Selection copied to clipboard".to_owned(),
        };
        assert!(scroll_roundtrip_cleanup_complete(&clean));

        let mut stale = clean.clone();
        stale.manual_scroll_state = "collecting".to_owned();
        assert!(!scroll_roundtrip_cleanup_complete(&stale));
        stale = clean.clone();
        stale.manual_scroll_frame_count = 2;
        assert!(!scroll_roundtrip_cleanup_complete(&stale));
        stale = clean.clone();
        stale.manual_scroll_can_finish = true;
        assert!(!scroll_roundtrip_cleanup_complete(&stale));
        stale = clean.clone();
        stale.manual_scroll_capture_in_flight = true;
        assert!(!scroll_roundtrip_cleanup_complete(&stale));
        stale = clean.clone();
        stale.manual_scroll_auto_capture_pending = true;
        assert!(!scroll_roundtrip_cleanup_complete(&stale));
        stale = clean.clone();
        stale.manual_scroll_selection = Some(selection);
        assert!(!scroll_roundtrip_cleanup_complete(&stale));
        stale = clean.clone();
        stale.more_actions_visible = true;
        assert!(!scroll_roundtrip_cleanup_complete(&stale));
        stale = clean.clone();
        stale.annotation_controls_visible = true;
        assert!(!scroll_roundtrip_cleanup_complete(&stale));
        stale = clean;
        stale.capture_teardown_pending = true;
        assert!(!scroll_roundtrip_cleanup_complete(&stale));
    }

    #[test]
    fn scroll_roundtrip_plan_keeps_scroll_shot_inside_the_expanded_menu() {
        let bounds = PhysicalRect {
            left: 0,
            top: -4,
            right: 2560,
            bottom: 1436,
        };
        let plan = scroll_roundtrip_interaction_plan(bounds, 1.0).unwrap();
        let point = scroll_shot_point_for_logical_selection(
            bounds,
            1.0,
            2560.0,
            1440.0,
            (2560.0 * 0.16, (1440.0_f32 * 0.12).max(120.0)),
            (2560.0 * 0.74, (1440.0_f32 * 0.12).max(120.0) + 380.0),
        )
        .unwrap();
        assert_eq!(
            PhysicalRect::new(plan.drag_start, plan.drag_end),
            PhysicalRect {
                left: 410,
                top: 169,
                right: 1894,
                bottom: 549,
            }
        );
        assert_eq!(point, PhysicalPoint { x: 1597, y: 443 });
        assert!(bounds.contains(point));
    }

    #[test]
    fn scroll_roundtrip_minimum_height_keeps_the_full_selection_inside_the_fixture() {
        let bounds = PhysicalRect {
            left: 0,
            top: 0,
            right: 1280,
            bottom: 740,
        };
        let fixture = PhysicalRect {
            left: 120,
            top: 100,
            right: 1160,
            bottom: 640,
        };
        let plan = scroll_roundtrip_interaction_plan(bounds, 1.0).unwrap();
        let selection = PhysicalRect::new(plan.drag_start, plan.drag_end);

        assert!(rect_contains_rect(fixture, selection));
        assert!(selection.height() >= 16);
    }

    #[cfg(windows)]
    #[test]
    fn scroll_fixture_oracle_rejects_one_foreign_pixel() {
        let fixture = PhysicalRect {
            left: 100,
            top: 100,
            right: 500,
            bottom: 500,
        };
        let bounds = PhysicalRect {
            left: 100,
            top: 100,
            right: 102,
            bottom: 102,
        };
        let mut pixels = [0x1c, 0x4e, 0x80, 0xff].repeat(4);
        let frame = |pixels: Vec<u8>| CaptureFrame {
            bounds,
            width: 2,
            height: 2,
            stride: 8,
            format: PixelFormat::Bgra8,
            pixels: Arc::from(pixels),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 1,
        };

        assert!(validate_scroll_fixture_frame(&frame(pixels.clone()), fixture, 0).is_ok());
        pixels[5] ^= 0xff;
        let error = validate_scroll_fixture_frame(&frame(pixels), fixture, 0).unwrap_err();
        assert!(error.to_string().contains("pixel mismatch"));
    }

    #[test]
    fn selection_transform_model_matches_move_resize_and_modifier_invariants() {
        let capture_bounds = PhysicalRect {
            left: -1920,
            top: -200,
            right: 0,
            bottom: 880,
        };
        let selection = PhysicalRect {
            left: -1500,
            top: 100,
            right: -900,
            bottom: 400,
        };

        let move_gesture =
            selection_transform_gesture(selection, capture_bounds, SelectionTransformKind::Move)
                .unwrap();
        let moved = expected_selection_transform(
            selection,
            move_gesture.start,
            move_gesture.end,
            capture_bounds,
            SelectionTransformKind::Move,
        )
        .unwrap();
        assert_eq!(
            moved,
            PhysicalRect {
                left: -1380,
                top: 172,
                right: -780,
                bottom: 472,
            }
        );
        assert_eq!(moved.width(), selection.width());
        assert_eq!(moved.height(), selection.height());

        let corner = selection_transform_gesture(
            selection,
            capture_bounds,
            SelectionTransformKind::CornerResize,
        )
        .unwrap();
        let resized = expected_selection_transform(
            selection,
            corner.start,
            corner.end,
            capture_bounds,
            SelectionTransformKind::CornerResize,
        )
        .unwrap();
        assert_eq!(resized.left, selection.left);
        assert_eq!(resized.top, selection.top);
        assert!(resized.width() > selection.width());

        let shift = selection_transform_gesture(
            selection,
            capture_bounds,
            SelectionTransformKind::ShiftResize,
        )
        .unwrap();
        let shift_resized = expected_selection_transform(
            selection,
            shift.start,
            shift.end,
            capture_bounds,
            SelectionTransformKind::ShiftResize,
        )
        .unwrap();
        assert!(selection_aspect_ratio_preserved(selection, shift_resized));

        let alt = selection_transform_gesture(
            selection,
            capture_bounds,
            SelectionTransformKind::AltResize,
        )
        .unwrap();
        let alt_resized = expected_selection_transform(
            selection,
            alt.start,
            alt.end,
            capture_bounds,
            SelectionTransformKind::AltResize,
        )
        .unwrap();
        assert!(selection_center_preserved(selection, alt_resized));
    }

    #[test]
    fn selection_transform_point_mapping_round_trips_negative_offset_coordinates() {
        let client = PhysicalRect {
            left: -300,
            top: -200,
            right: 700,
            bottom: 600,
        };
        let capture = PhysicalRect {
            left: -1920,
            top: -1080,
            right: -920,
            bottom: -280,
        };
        let capture_point = PhysicalPoint { x: -1440, y: -720 };
        let screen_point = map_capture_point_to_screen(capture_point, client, capture).unwrap();
        assert_eq!(screen_point, PhysicalPoint { x: 180, y: 160 });
        assert_eq!(
            map_screen_point_to_capture(screen_point, client, capture).unwrap(),
            capture_point
        );
    }

    #[test]
    fn stitched_editor_points_map_from_the_image_bounds_instead_of_the_display() {
        let client = PhysicalRect {
            left: 538,
            top: 388,
            right: 2022,
            bottom: 1051,
        };
        let stitched = PhysicalRect {
            left: 410,
            top: 169,
            right: 1894,
            bottom: 832,
        };
        let display = PhysicalRect {
            left: 0,
            top: 0,
            right: 2560,
            bottom: 1440,
        };

        assert_eq!(
            map_capture_point_to_screen(
                PhysicalPoint {
                    x: stitched.right,
                    y: stitched.bottom,
                },
                client,
                stitched,
            )
            .unwrap(),
            PhysicalPoint {
                x: client.right,
                y: client.bottom,
            }
        );
        assert!(
            map_capture_point_to_screen(
                PhysicalPoint {
                    x: stitched.right,
                    y: stitched.bottom,
                },
                client,
                display,
            )
            .is_err()
        );
    }

    #[test]
    fn pin_coexist_plan_spreads_three_compact_selections_across_the_upper_overlay() {
        let client = PhysicalRect {
            left: 0,
            top: -4,
            right: 2560,
            bottom: 1436,
        };
        let expected = [
            (
                PhysicalPoint { x: 660, y: 156 },
                PhysicalPoint { x: 1020, y: 396 },
            ),
            (
                PhysicalPoint { x: 1100, y: 156 },
                PhysicalPoint { x: 1460, y: 396 },
            ),
            (
                PhysicalPoint { x: 1540, y: 156 },
                PhysicalPoint { x: 1900, y: 396 },
            ),
        ];

        for (index, (start, end)) in expected.into_iter().enumerate() {
            let plan = pin_coexist_interaction_plan(client, 1.0, index).unwrap();
            assert_eq!(plan.drag_start, start);
            assert_eq!(plan.drag_end, end);
            assert!(client.contains(plan.pin));
        }
        assert!(pin_coexist_interaction_plan(client, 1.5, 0).is_err());
        assert!(pin_coexist_interaction_plan(client, 1.0, 3).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn pin_row_layout_preserves_sizes_and_window_drag_follows_pointer_delta() {
        let display = PhysicalRect {
            left: 0,
            top: 0,
            right: 2560,
            bottom: 1440,
        };
        let windows = (0..3)
            .map(|index| NativeWindow {
                handle: (index + 1) as *mut std::ffi::c_void,
                bounds: PhysicalRect {
                    left: 0,
                    top: 0,
                    right: 360,
                    bottom: 240,
                },
                dpi: 96,
            })
            .collect::<Vec<_>>();
        let layout = horizontal_pin_layout(display, &windows).unwrap();
        assert_eq!(
            layout,
            vec![
                PhysicalRect {
                    left: 640,
                    top: 1120,
                    right: 1000,
                    bottom: 1360,
                },
                PhysicalRect {
                    left: 1100,
                    top: 1120,
                    right: 1460,
                    bottom: 1360,
                },
                PhysicalRect {
                    left: 1560,
                    top: 1120,
                    right: 1920,
                    bottom: 1360,
                },
            ]
        );
        let before = layout[0];
        let after = PhysicalRect {
            left: before.left + 48,
            top: before.top - 40,
            right: before.right + 48,
            bottom: before.bottom - 40,
        };
        assert!(window_drag_matches(
            before,
            after,
            PhysicalPoint { x: 820, y: 1336 },
            PhysicalPoint { x: 868, y: 1296 },
            2,
        ));
        assert!(!window_drag_matches(
            before,
            after,
            PhysicalPoint { x: 820, y: 1336 },
            PhysicalPoint { x: 870, y: 1296 },
            1,
        ));
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
            plan.mark,
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
        assert!(plan.mark.x < plan.pin.x);
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
    fn narrow_edge_plan_matches_real_borderless_client_and_relocated_mark() {
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
        let plan = narrow_edge_interaction_plan(client, 1.0).unwrap();

        assert_eq!(plan.base.drag_start, PhysicalPoint { x: 2382, y: 1328 });
        assert_eq!(plan.base.drag_end, PhysicalPoint { x: 2542, y: 1424 });
        assert_eq!(plan.base.mark, PhysicalPoint { x: 2233, y: 1291 });
        assert_eq!(plan.base.more, PhysicalPoint { x: 2447, y: 1291 });
        assert_eq!(plan.base.cancel, PhysicalPoint { x: 2512, y: 1291 });
        assert_eq!(plan.expanded_mark, PhysicalPoint { x: 2233, y: 1155 });
        assert_eq!(plan.evidence_rest, PhysicalPoint { x: 24, y: 20 });
        assert_eq!(
            map_screen_selection_to_capture(
                PhysicalRect::new(plan.base.drag_start, plan.base.drag_end),
                client,
                display,
            )
            .unwrap(),
            PhysicalRect {
                left: 2382,
                top: 1332,
                right: 2542,
                bottom: 1428,
            }
        );
        assert!(narrow_edge_interaction_plan(client, 1.5).is_err());
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
    fn clipboard_dib_decoder_restores_bottom_up_bgra_rows() {
        let mut dib = vec![0_u8; 40 + 16];
        dib[0..4].copy_from_slice(&40_u32.to_le_bytes());
        dib[4..8].copy_from_slice(&2_i32.to_le_bytes());
        dib[8..12].copy_from_slice(&2_i32.to_le_bytes());
        dib[12..14].copy_from_slice(&1_u16.to_le_bytes());
        dib[14..16].copy_from_slice(&32_u16.to_le_bytes());
        dib[20..24].copy_from_slice(&16_u32.to_le_bytes());
        dib[40..56].copy_from_slice(&[7, 8, 9, 255, 10, 11, 12, 255, 1, 2, 3, 255, 4, 5, 6, 255]);

        let frame = decode_clipboard_dib(&dib).unwrap();

        assert_eq!((frame.width, frame.height, frame.stride), (2, 2, 8));
        assert_eq!(
            frame.pixels.as_ref(),
            &[1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255]
        );
        dib[14..16].copy_from_slice(&24_u16.to_le_bytes());
        assert_eq!(
            decode_clipboard_dib(&dib).unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
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
