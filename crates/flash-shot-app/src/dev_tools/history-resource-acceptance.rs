//! No-input Release resource evidence for the Library thumbnail queue.

use std::{
    fs, io,
    path::{Path, PathBuf},
    process,
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(windows)]
use std::sync::mpsc::RecvTimeoutError;

use flash_shot::{
    HistoryResourceAcceptanceCommand, HistoryResourceAcceptanceOptions,
    HistoryResourceAcceptanceState,
    domain::geometry::PhysicalRect,
    history::{HistorySource, ScreenshotHistory},
    performance::PerformanceRecorder,
    platform::capture::{CaptureFrame, PixelFormat},
    settings::UserSettings,
};

#[cfg(windows)]
use flash_shot::performance::build_profile;

#[cfg(windows)]
use flash_shot::platform::capture::{CaptureBackend, SystemCaptureBackend};

const DEFAULT_OUTPUT_DIR: &str = "target/history-resource-acceptance";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_SAMPLE_INTERVAL: Duration = Duration::from_millis(50);
const FIXTURE_COUNT: usize = 300;
const DEFAULT_VISIBLE_COUNT: usize = 5;
const FIXTURE_WIDTH: u32 = 320;
const FIXTURE_HEIGHT: u32 = 200;

#[derive(Clone, Debug)]
struct Options {
    output_dir: PathBuf,
    timeout: Duration,
    sample_interval: Duration,
}

#[derive(Clone, Debug, serde::Serialize)]
struct ResourceSample {
    phase: &'static str,
    elapsed_ms: f64,
    state: HistoryResourceAcceptanceState,
    working_set_bytes: u64,
    private_commit_bytes: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Serialize)]
struct ResourceSnapshot {
    working_set_bytes: u64,
    private_commit_bytes: u64,
}

#[derive(Clone, Debug)]
struct NativeWindow {
    bounds: PhysicalRect,
    dpi: u32,
    #[cfg(windows)]
    handle: *mut std::ffi::c_void,
}

pub(super) fn entrypoint() {
    if let Err(error) = run() {
        eprintln!("history resource acceptance failed: {error}");
        process::exit(1);
    }
}

/// Builds an isolated history fixture, drives the Library queue, and writes the final report
/// before GPUI shuts down so Windows process termination cannot discard cleanup evidence.
fn run() -> io::Result<serde_json::Value> {
    let options = parse_args(std::env::args().skip(1))?;
    let output_dir = fs::canonicalize(&options.output_dir).or_else(|_| {
        fs::create_dir_all(&options.output_dir)?;
        fs::canonicalize(&options.output_dir)
    })?;
    let session_root = create_session_root(&output_dir)?;
    let history_root = session_root.join("history");
    fs::create_dir_all(&history_root)?;
    fs::create_dir_all(session_root.join("screenshots"))?;
    let report_path = session_root.join("report.json");
    let fixture_paths = create_history_fixtures(&history_root)?;
    let mut history = ScreenshotHistory::open_with_limit(&history_root, FIXTURE_COUNT)?;
    for path in &fixture_paths {
        history.record_with_source(path.clone(), HistorySource::Selection)?;
    }
    let performance = PerformanceRecorder::new(session_root.join("metrics"))?;
    let settings_path = session_root.join("settings.json");
    let (command_tx, command_rx) = async_channel::unbounded();
    // GPUI owns the process lifetime on Windows, so this worker is intentionally detached: the
    // final report and fixture cleanup must be complete before its Quit command is sent.
    let _worker = thread::spawn(move || {
        let result = flash_shot::run_history_resource_acceptance(
            Instant::now(),
            performance,
            history,
            UserSettings::default(),
            settings_path,
            HistoryResourceAcceptanceOptions {
                window_width: 980.0,
                window_height: 760.0,
                commands: command_rx,
            },
        )
        .map_err(|error| error.to_string());
        let _ = result;
    });

    let result = (|| -> io::Result<serde_json::Value> {
        #[cfg(not(windows))]
        {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "history resource acceptance is currently Windows-only",
            ));
        }

        #[cfg(windows)]
        {
            let started = Instant::now();
            let window = wait_for_visible_process_window(options.timeout)?;
            focus_process_window(&window)?;
            let mut samples = Vec::new();
            let default_state = wait_for_default_preview(
                &command_tx,
                options.timeout,
                options.sample_interval,
                started,
                &mut samples,
            )?;
            let default_screenshot = session_root.join("screenshots/default-5-preview.png");
            capture_window(&window, &default_screenshot)?;
            let baseline = resource_snapshot()?;

            let expanded_state = set_expanded_and_wait(
                &command_tx,
                options.timeout,
                options.sample_interval,
                started,
                &mut samples,
            )?;
            let expanded_screenshot = session_root.join("screenshots/expanded-300-preview.png");
            capture_window(&window, &expanded_screenshot)?;
            let peak = peak_for_phase(baseline, &samples, "expanded_300");
            let peak_loading = samples
                .iter()
                .filter(|sample| sample.phase == "expanded_300")
                .map(|sample| sample.state.thumbnails_loading)
                .max()
                .unwrap_or(0);
            let peak_pending = samples
                .iter()
                .filter(|sample| sample.phase == "expanded_300")
                .map(|sample| sample.state.thumbnails_pending)
                .max()
                .unwrap_or(0);
            let first_thumbnail_elapsed_ms = samples
                .iter()
                .find(|sample| sample.state.thumbnails_cached > 0)
                .map(|sample| sample.elapsed_ms);
            let report = serde_json::json!({
                "schema_version": 1,
                "test": "history_thumbnail_resource_acceptance",
                "passed": build_profile() == "release"
                    && default_state.total_entries == FIXTURE_COUNT
                    && default_state.visible_entries == DEFAULT_VISIBLE_COUNT
                    && default_state.thumbnails_cached >= DEFAULT_VISIBLE_COUNT
                    && default_state.thumbnails_loading == 0
                    && default_state.thumbnails_pending == 0
                    && expanded_state.total_entries == FIXTURE_COUNT
                    && expanded_state.visible_entries == FIXTURE_COUNT
                    && expanded_state.thumbnails_cached >= FIXTURE_COUNT
                    && expanded_state.thumbnails_loading == 0
                    && expanded_state.thumbnails_pending == 0
                    && peak_loading <= 2
                    && first_thumbnail_elapsed_ms.is_some(),
                "measurement_mode": "release_resource",
                "build_profile": build_profile(),
                "session_root": session_root.to_string_lossy().into_owned(),
                "fixture_count": FIXTURE_COUNT,
                "default_preview_count": DEFAULT_VISIBLE_COUNT,
                "fixture_dimensions": { "width": FIXTURE_WIDTH, "height": FIXTURE_HEIGHT },
                "window": {
                    "left": window.bounds.left,
                    "top": window.bounds.top,
                    "right": window.bounds.right,
                    "bottom": window.bounds.bottom,
                    "dpi": window.dpi,
                    "scale_factor": window.dpi as f64 / 96.0,
                },
                "phases": {
                    "default_5": default_state,
                    "expanded_300": expanded_state,
                },
                "resources": {
                    "baseline_after_default_5": baseline,
                    "peak_during_expanded_300": peak,
                    "working_set_growth_bytes": peak.working_set_bytes.saturating_sub(baseline.working_set_bytes),
                    "private_commit_growth_bytes": peak
                        .private_commit_bytes
                        .saturating_sub(baseline.private_commit_bytes),
                },
                "queue": {
                    "peak_loading": peak_loading,
                    "peak_pending": peak_pending,
                    "max_in_flight_limit": 2,
                    "first_thumbnail_elapsed_ms": first_thumbnail_elapsed_ms,
                },
                "samples": samples,
                "screenshots": {
                    "default_5": "screenshots/default-5-preview.png",
                    "expanded_300": "screenshots/expanded-300-preview.png",
                },
                "cleanup": {
                    "fixture_count_created": fixture_paths.len(),
                    "fixture_files_removed": false,
                    "history_root_exists": true,
                    "history_root": history_root.to_string_lossy(),
                },
            });
            Ok(report)
        }
    })();

    // Allow the last decode callback to release its file handle, then remove the isolated tree
    // while the GPUI worker is still alive. Its Quit path calls ExitProcess(0), so cleanup after
    // request_quit would never run on the Windows release build.
    thread::sleep(Duration::from_millis(250));
    let cleanup_result = remove_history_root(&history_root);
    let mut report = match result {
        Ok(report) => report,
        Err(error) => serde_json::json!({
            "schema_version": 1,
            "test": "history_thumbnail_resource_acceptance",
            "passed": false,
            "error": error.to_string(),
            "session_root": session_root.to_string_lossy().into_owned(),
        }),
    };
    let history_root_exists = history_root.exists();
    if report.get("cleanup").is_none() {
        report["cleanup"] = serde_json::json!({
            "fixture_count_created": fixture_paths.len(),
            "history_root": history_root.to_string_lossy(),
        });
    }
    report["cleanup"]["fixture_files_removed"] = serde_json::Value::Bool(cleanup_result.is_ok());
    report["cleanup"]["history_root_exists"] = serde_json::Value::Bool(history_root_exists);
    report["cleanup"]["fixture_cleanup_error"] = cleanup_result
        .as_ref()
        .err()
        .map(|error| serde_json::Value::String(error.to_string()))
        .unwrap_or(serde_json::Value::Null);
    report["passed"] = serde_json::Value::Bool(
        report["passed"].as_bool().unwrap_or(false)
            && cleanup_result.is_ok()
            && !history_root_exists,
    );
    fs::write(
        &report_path,
        serde_json::to_vec_pretty(&report).map_err(io::Error::other)?,
    )?;
    println!(
        "{}",
        serde_json::to_string_pretty(&report).expect("resource report is serializable")
    );
    let _ = io::Write::flush(&mut io::stdout());

    if !report["passed"].as_bool().unwrap_or(false) {
        process::exit(2);
    }
    if let Err(error) = request_quit(&command_tx, options.timeout) {
        report["passed"] = serde_json::Value::Bool(false);
        report["shutdown_error"] = serde_json::Value::String(error.to_string());
        fs::write(
            &report_path,
            serde_json::to_vec_pretty(&report).map_err(io::Error::other)?,
        )?;
        eprintln!("history resource acceptance shutdown failed: {error}");
        process::exit(1);
    }
    Ok(report)
}

/// Creates one fresh evidence directory so retries and concurrent probes cannot overwrite output.
fn create_session_root(output_dir: &Path) -> io::Result<PathBuf> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let process_id = process::id();
    for attempt in 0_u16..=100 {
        let suffix = if attempt == 0 {
            String::new()
        } else {
            format!("-{attempt}")
        };
        let root = output_dir.join(format!("session-{timestamp}-{process_id}{suffix}"));
        match fs::create_dir(&root) {
            Ok(()) => return Ok(root),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique history resource acceptance session directory",
    ))
}

/// Removes the isolated fixture tree, retrying transient Windows file locks.
fn remove_history_root(root: &Path) -> io::Result<()> {
    let mut last_error = None;
    for attempt in 0..=10 {
        match fs::remove_dir_all(root) {
            Ok(()) => return Ok(()),
            Err(error) if attempt < 10 => {
                last_error = Some(error);
                thread::sleep(Duration::from_millis(100));
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_error.expect("cleanup retry loop always records an error"))
}

fn request_quit(
    commands: &async_channel::Sender<HistoryResourceAcceptanceCommand>,
    timeout: Duration,
) -> io::Result<()> {
    let (quit_tx, quit_rx) = mpsc::sync_channel(1);
    commands
        .try_send(HistoryResourceAcceptanceCommand::Quit(quit_tx))
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "resource app already closed"))?;
    quit_rx
        .recv_timeout(timeout)
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "resource app did not quit"))
}

fn parse_args(args: impl IntoIterator<Item = String>) -> io::Result<Options> {
    let mut options = Options {
        output_dir: PathBuf::from(DEFAULT_OUTPUT_DIR),
        timeout: DEFAULT_TIMEOUT,
        sample_interval: DEFAULT_SAMPLE_INTERVAL,
    };
    let mut args = args.into_iter();
    while let Some(argument) = args.next() {
        let mut value = || {
            args.next().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("missing value for {argument}"),
                )
            })
        };
        match argument.as_str() {
            "--output-dir" => options.output_dir = PathBuf::from(value()?),
            "--timeout-ms" => {
                let milliseconds = parse_u64(value()?, &argument)?;
                if !(3_000..=300_000).contains(&milliseconds) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "timeout-ms must be between 3000 and 300000",
                    ));
                }
                options.timeout = Duration::from_millis(milliseconds);
            }
            "--sample-interval-ms" => {
                let milliseconds = parse_u64(value()?, &argument)?;
                if !(10..=1_000).contains(&milliseconds) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "sample-interval-ms must be between 10 and 1000",
                    ));
                }
                options.sample_interval = Duration::from_millis(milliseconds);
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unknown argument: {argument}"),
                ));
            }
        }
    }
    Ok(options)
}

fn parse_u64(value: String, argument: &str) -> io::Result<u64> {
    value.parse().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid value for {argument}: {value}"),
        )
    })
}

/// Computes a phase-local resource peak while retaining the post-baseline floor.
fn peak_for_phase(
    baseline: ResourceSnapshot,
    samples: &[ResourceSample],
    phase: &str,
) -> ResourceSnapshot {
    samples
        .iter()
        .filter(|sample| sample.phase == phase)
        .fold(baseline, |peak, sample| ResourceSnapshot {
            working_set_bytes: peak.working_set_bytes.max(sample.working_set_bytes),
            private_commit_bytes: peak.private_commit_bytes.max(sample.private_commit_bytes),
        })
}

fn create_history_fixtures(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut paths = Vec::with_capacity(FIXTURE_COUNT);
    for index in 0..FIXTURE_COUNT {
        let path = root.join(format!("fixture-{index:03}.png"));
        fixture_frame(index).save_png(&path)?;
        paths.push(path);
    }
    Ok(paths)
}

fn fixture_frame(index: usize) -> CaptureFrame {
    let mut pixels = vec![0_u8; FIXTURE_WIDTH as usize * FIXTURE_HEIGHT as usize * 4];
    for y in 0..FIXTURE_HEIGHT as usize {
        for x in 0..FIXTURE_WIDTH as usize {
            let offset = (y * FIXTURE_WIDTH as usize + x) * 4;
            pixels[offset] = ((x + index) % 251) as u8;
            pixels[offset + 1] = ((y * 3 + index) % 251) as u8;
            pixels[offset + 2] = ((x + y + index * 7) % 251) as u8;
            pixels[offset + 3] = 255;
        }
    }
    CaptureFrame {
        bounds: PhysicalRect {
            left: 0,
            top: 0,
            right: FIXTURE_WIDTH as i32,
            bottom: FIXTURE_HEIGHT as i32,
        },
        width: FIXTURE_WIDTH,
        height: FIXTURE_HEIGHT,
        stride: FIXTURE_WIDTH as usize * 4,
        format: PixelFormat::Bgra8,
        pixels: Arc::from(pixels),
        capture_duration: Duration::ZERO,
        cpu_copy_count: 1,
    }
}

#[cfg(windows)]
fn wait_for_default_preview(
    commands: &async_channel::Sender<HistoryResourceAcceptanceCommand>,
    timeout: Duration,
    interval: Duration,
    started: Instant,
    samples: &mut Vec<ResourceSample>,
) -> io::Result<HistoryResourceAcceptanceState> {
    wait_for_state(
        commands,
        timeout,
        interval,
        started,
        samples,
        "default_5",
        |state| {
            state.total_entries == FIXTURE_COUNT
                && state.visible_entries == DEFAULT_VISIBLE_COUNT
                && state.thumbnails_cached >= DEFAULT_VISIBLE_COUNT
                && state.thumbnails_loading == 0
                && state.thumbnails_pending == 0
        },
    )
}

#[cfg(windows)]
fn set_expanded_and_wait(
    commands: &async_channel::Sender<HistoryResourceAcceptanceCommand>,
    timeout: Duration,
    interval: Duration,
    started: Instant,
    samples: &mut Vec<ResourceSample>,
) -> io::Result<HistoryResourceAcceptanceState> {
    let (reply_tx, reply_rx) = mpsc::sync_channel(1);
    commands
        .send_blocking(HistoryResourceAcceptanceCommand::SetExpanded {
            expanded: true,
            reply: reply_tx,
        })
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "resource app closed"))?;
    reply_rx.recv_timeout(timeout).map_err(|_| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            "expanded state reply did not arrive",
        )
    })?;
    wait_for_state(
        commands,
        timeout,
        interval,
        started,
        samples,
        "expanded_300",
        |state| {
            state.total_entries == FIXTURE_COUNT
                && state.visible_entries == FIXTURE_COUNT
                && state.thumbnails_cached >= FIXTURE_COUNT
                && state.thumbnails_loading == 0
                && state.thumbnails_pending == 0
        },
    )
}

#[cfg(windows)]
fn wait_for_state(
    commands: &async_channel::Sender<HistoryResourceAcceptanceCommand>,
    timeout: Duration,
    interval: Duration,
    started: Instant,
    samples: &mut Vec<ResourceSample>,
    phase: &'static str,
    ready: impl Fn(&HistoryResourceAcceptanceState) -> bool,
) -> io::Result<HistoryResourceAcceptanceState> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("{phase} thumbnails did not settle before timeout"),
            ));
        }
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        commands
            .try_send(HistoryResourceAcceptanceCommand::Snapshot(reply_tx))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "resource app closed"))?;
        let state = reply_rx
            .recv_timeout(remaining)
            .map_err(|error| match error {
                RecvTimeoutError::Timeout => {
                    io::Error::new(io::ErrorKind::TimedOut, "resource state reply timed out")
                }
                RecvTimeoutError::Disconnected => {
                    io::Error::new(io::ErrorKind::BrokenPipe, "resource state channel closed")
                }
            })?;
        let resources = resource_snapshot()?;
        samples.push(ResourceSample {
            phase,
            elapsed_ms: started.elapsed().as_secs_f64() * 1_000.0,
            state: state.clone(),
            working_set_bytes: resources.working_set_bytes,
            private_commit_bytes: resources.private_commit_bytes,
        });
        if ready(&state) {
            return Ok(state);
        }
        thread::sleep(interval.min(deadline.saturating_duration_since(Instant::now())));
    }
}

#[cfg(windows)]
fn wait_for_visible_process_window(timeout: Duration) -> io::Result<NativeWindow> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(window) = visible_process_window() {
            return Ok(window);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "visible history resource window did not appear",
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(windows)]
fn visible_process_window() -> io::Result<NativeWindow> {
    use std::ffi::c_void;
    use windows_sys::Win32::{
        Foundation::{LPARAM, RECT},
        UI::{
            HiDpi::GetDpiForWindow,
            WindowsAndMessaging::{
                EnumWindows, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible,
            },
        },
    };
    use windows_sys::core::BOOL;

    struct Search {
        process_id: u32,
        window: Option<NativeWindow>,
    }

    unsafe extern "system" fn callback(window: *mut c_void, parameter: LPARAM) -> BOOL {
        let search = unsafe { &mut *(parameter as *mut Search) };
        let mut process_id = 0;
        unsafe { GetWindowThreadProcessId(window, &mut process_id) };
        if process_id != search.process_id || unsafe { IsWindowVisible(window) } == 0 {
            return 1;
        }
        let mut rect = RECT::default();
        if unsafe { GetWindowRect(window, &mut rect) } == 0 {
            return 1;
        }
        search.window = Some(NativeWindow {
            bounds: PhysicalRect {
                left: rect.left,
                top: rect.top,
                right: rect.right,
                bottom: rect.bottom,
            },
            dpi: unsafe { GetDpiForWindow(window) }.max(96),
            handle: window,
        });
        0
    }

    let mut search = Search {
        process_id: unsafe { windows_sys::Win32::System::Threading::GetCurrentProcessId() },
        window: None,
    };
    unsafe { EnumWindows(Some(callback), &mut search as *mut Search as LPARAM) };
    search
        .window
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "visible process window not found"))
}

#[cfg(windows)]
fn focus_process_window(window: &NativeWindow) -> io::Result<()> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, SW_RESTORE, SetForegroundWindow, ShowWindow,
    };
    unsafe { ShowWindow(window.handle, SW_RESTORE) };
    let focused = unsafe { SetForegroundWindow(window.handle) } != 0;
    let raised = unsafe { BringWindowToTop(window.handle) } != 0;
    if focused || raised {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn capture_window(window: &NativeWindow, output: &Path) -> io::Result<()> {
    let frame = SystemCaptureBackend.capture(window.bounds)?;
    frame.save_png(output)
}

#[cfg(windows)]
fn resource_snapshot() -> io::Result<ResourceSnapshot> {
    use std::mem::size_of;
    use windows_sys::Win32::System::{
        ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
        Threading::GetCurrentProcess,
    };
    let mut memory = PROCESS_MEMORY_COUNTERS {
        cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    if unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut memory, memory.cb) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(ResourceSnapshot {
        working_set_bytes: memory.WorkingSetSize as u64,
        private_commit_bytes: memory.PagefileUsage as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        FIXTURE_COUNT, ResourceSample, ResourceSnapshot, create_session_root, parse_args,
        peak_for_phase,
    };
    use flash_shot::HistoryResourceAcceptanceState;
    use std::{
        fs,
        path::PathBuf,
        process,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn parser_defaults_to_bounded_release_resource_sampling() {
        let options = parse_args([]).unwrap();
        assert_eq!(
            options.output_dir,
            PathBuf::from("target/history-resource-acceptance")
        );
        assert_eq!(options.timeout.as_secs(), 60);
        assert_eq!(FIXTURE_COUNT, 300);
    }

    #[test]
    fn parser_rejects_unbounded_timeout_values() {
        assert!(parse_args(["--timeout-ms".to_owned(), "2999".to_owned()]).is_err());
        assert!(parse_args(["--sample-interval-ms".to_owned(), "9".to_owned()]).is_err());
    }

    #[test]
    fn peak_only_includes_samples_from_requested_phase() {
        let state = HistoryResourceAcceptanceState {
            total_entries: 300,
            visible_entries: 5,
            expanded: false,
            thumbnails_cached: 5,
            thumbnails_loading: 0,
            thumbnails_pending: 0,
        };
        let baseline = ResourceSnapshot {
            working_set_bytes: 100,
            private_commit_bytes: 200,
        };
        let samples = [
            ResourceSample {
                phase: "default_5",
                elapsed_ms: 1.0,
                state: state.clone(),
                working_set_bytes: 9_000,
                private_commit_bytes: 9_000,
            },
            ResourceSample {
                phase: "expanded_300",
                elapsed_ms: 2.0,
                state,
                working_set_bytes: 300,
                private_commit_bytes: 500,
            },
        ];

        assert_eq!(
            peak_for_phase(baseline, &samples, "expanded_300"),
            ResourceSnapshot {
                working_set_bytes: 300,
                private_commit_bytes: 500,
            }
        );
    }

    #[test]
    fn resource_snapshot_uses_private_commit_json_name() {
        let value = serde_json::to_value(ResourceSnapshot {
            working_set_bytes: 1,
            private_commit_bytes: 2,
        })
        .unwrap();

        assert_eq!(value["private_commit_bytes"], 2);
        assert!(value.get("private_bytes").is_none());
    }

    #[test]
    fn session_root_keeps_retries_from_reusing_evidence_paths() {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let output = std::env::temp_dir().join(format!(
            "flash-shot-history-resource-test-{}-{timestamp}",
            process::id()
        ));
        fs::create_dir_all(&output).unwrap();

        let first = create_session_root(&output).unwrap();
        let second = create_session_root(&output).unwrap();

        assert_ne!(first, second);
        assert!(first.starts_with(&output));
        assert!(second.starts_with(&output));
        fs::remove_dir_all(&output).unwrap();
    }
}
