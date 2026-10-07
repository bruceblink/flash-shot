//! Real Windows mouse and keyboard acceptance for the settings navigation shell.

use std::{
    fs, io,
    path::PathBuf,
    process,
    sync::{Arc, mpsc},
    time::Duration,
};
#[cfg(windows)]
use std::{thread, time::Instant};

use flash_shot::{
    SettingsInteractionAcceptanceOptions,
    domain::geometry::PhysicalRect,
    history::{HistorySource, ScreenshotHistory},
    i18n::Locale,
    performance::PerformanceRecorder,
    platform::capture::{CaptureFrame, PixelFormat},
    settings::UserSettings,
    theme::ThemeMode,
};
#[cfg(windows)]
use serde::Serialize;

#[cfg(windows)]
#[path = "settings-interaction-acceptance/input.rs"]
mod input;
#[cfg(windows)]
#[path = "settings-interaction-acceptance/native.rs"]
mod native;

const DEFAULT_OUTPUT_DIR: &str = "target/settings-interaction-acceptance";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);
const DEFAULT_SETTLE: Duration = Duration::from_millis(700);
const ACCEPTANCE_RECORDING_DIRECTORY: &str = r"C:\FlashShotAcceptance";
const ACCEPTANCE_LIBRARY_ROOT: &str = r"C:\FSA";
#[cfg(windows)]
const INPUT_SETTLE_DELAY: Duration = Duration::from_millis(120);

#[derive(Debug)]
struct Options {
    allow_input: bool,
    exercise_app_update: bool,
    exercise_record_support: bool,
    exercise_record_start: bool,
    exercise_record_success: bool,
    exercise_library_format: bool,
    exercise_library_search_selection: bool,
    exercise_library_single_delete: bool,
    exercise_library_open_copy: bool,
    exercise_library_retention: bool,
    exercise_pin_appearance: bool,
    output_dir: PathBuf,
    width: i32,
    height: i32,
    timeout: Duration,
    settle: Duration,
    locale: Locale,
    theme: ThemeMode,
}

#[cfg(windows)]
#[derive(Serialize)]
struct Report {
    schema: u32,
    status: &'static str,
    input_authorized: bool,
    locale: &'static str,
    theme: &'static str,
    window_width: i32,
    window_height: i32,
    window_bounds: WindowBounds,
    dpi: u32,
    scale_factor: f32,
    click_steps: Vec<StepReport>,
    keyboard_steps: Vec<StepReport>,
    action_steps: Vec<ActionStepReport>,
    recording_success: Option<RecordingSuccessReport>,
    library_search_selection: Option<LibrarySearchSelectionReport>,
    library_single_delete: Option<LibrarySingleDeleteReport>,
    library_open_copy: Option<LibraryOpenCopyReport>,
    library_retention: Option<LibraryRetentionReport>,
    pin_appearance: Option<PinAppearanceReport>,
    cleanup: CleanupReport,
}

#[cfg(windows)]
#[derive(Serialize)]
struct WindowBounds {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[cfg(windows)]
#[derive(Serialize)]
struct StepReport {
    action: String,
    expected_section: String,
    observed_section: String,
    observed_locale: String,
    observed_theme: String,
    passed: bool,
    screenshot: String,
}

#[cfg(windows)]
#[derive(Serialize)]
struct ActionStepReport {
    action: String,
    expected_section: String,
    observed_section: String,
    expected_busy: bool,
    observed_busy: bool,
    expected_status: String,
    observed_status: String,
    passed: bool,
    screenshot: String,
}

#[cfg(windows)]
#[derive(Serialize)]
struct RecordingSuccessReport {
    target: String,
    active_observed: bool,
    stopping_observed: bool,
    progress_frames: u64,
    output_path: String,
    output_exists: bool,
    output_bytes: u64,
    screenshots: Vec<String>,
}

#[cfg(windows)]
#[derive(Serialize)]
struct LibrarySingleDeleteReport {
    initial_entries: usize,
    single_confirmation_observed: bool,
    cancel_preserved_entries: bool,
    confirmation_reopened: bool,
    target_removed: bool,
    neighbor_preserved: bool,
    remaining_entries: usize,
    screenshots: Vec<String>,
}

#[cfg(windows)]
#[derive(Serialize)]
struct LibrarySearchSelectionReport {
    initial_entries: usize,
    selection_filter_entries: usize,
    query_entries: usize,
    selected_after_query: usize,
    entries_after_clearing_query: usize,
    selection_preserved_after_clearing_query: bool,
    selection_cleared: bool,
    all_filter_entries: usize,
    screenshots: Vec<String>,
}

#[cfg(windows)]
#[derive(Serialize)]
struct LibraryOpenCopyReport {
    initial_entries: usize,
    copy_status_observed: bool,
    clipboard_width: u32,
    clipboard_height: u32,
    clipboard_pixels_match: bool,
    open_result_observed: bool,
    editor_window_observed: bool,
    selection_matches_fixture: bool,
    cancel_returned_to_idle: bool,
    editor_window_closed: bool,
    history_file_preserved: bool,
    screenshots: Vec<String>,
}

#[cfg(windows)]
#[derive(Serialize)]
struct LibraryRetentionReport {
    initial_entries: usize,
    initial_limit: u16,
    applied_limits: Vec<u16>,
    entry_counts_after_updates: Vec<usize>,
    oldest_entries_removed: bool,
    newest_entries_preserved: bool,
    final_limit: u16,
    final_entries: usize,
    screenshots: Vec<String>,
}

#[cfg(windows)]
#[derive(Serialize)]
struct PinAppearanceReport {
    initial_locale: String,
    initial_theme: String,
    after_theme_locale: String,
    after_theme: String,
    after_locale: String,
    final_theme: String,
    pin_count: usize,
    all_pins_updated: bool,
    screenshots: Vec<String>,
}

#[cfg(windows)]
#[derive(Serialize)]
struct CleanupReport {
    cursor_restored: bool,
    input_released: bool,
    window_demoted: bool,
    process_exit_requested: bool,
}

impl Options {
    /// Parses only explicit, bounded runner options and refuses global input without opt-in.
    fn parse() -> Result<Self, String> {
        Self::parse_args(std::env::args_os().skip(1))
    }

    /// Parses supplied process arguments so authorization and bounds stay directly testable.
    fn parse_args(mut args: impl Iterator<Item = std::ffi::OsString>) -> Result<Self, String> {
        let mut options = Self {
            allow_input: false,
            exercise_app_update: false,
            exercise_record_support: false,
            exercise_record_start: false,
            exercise_record_success: false,
            exercise_library_format: false,
            exercise_library_search_selection: false,
            exercise_library_single_delete: false,
            exercise_library_open_copy: false,
            exercise_library_retention: false,
            exercise_pin_appearance: false,
            output_dir: PathBuf::from(DEFAULT_OUTPUT_DIR),
            width: 520,
            height: 640,
            timeout: DEFAULT_TIMEOUT,
            settle: DEFAULT_SETTLE,
            locale: Locale::English,
            theme: ThemeMode::Dark,
        };
        while let Some(argument) = args.next() {
            let argument = argument
                .into_string()
                .map_err(|_| "arguments must be valid UTF-8".to_owned())?;
            match argument.as_str() {
                "--allow-input" => options.allow_input = true,
                "--exercise-app-update" => options.exercise_app_update = true,
                "--exercise-record-support" => options.exercise_record_support = true,
                "--exercise-record-start" => options.exercise_record_start = true,
                "--exercise-record-success" => options.exercise_record_success = true,
                "--exercise-library-format" => options.exercise_library_format = true,
                "--exercise-library-search-selection" => {
                    options.exercise_library_search_selection = true
                }
                "--exercise-library-single-delete" => options.exercise_library_single_delete = true,
                "--exercise-library-open-copy" => options.exercise_library_open_copy = true,
                "--exercise-library-retention" => options.exercise_library_retention = true,
                "--exercise-pin-appearance" => options.exercise_pin_appearance = true,
                "--output-dir" => {
                    options.output_dir = args.next().map(PathBuf::from).ok_or_else(usage)?;
                }
                "--width" => options.width = parse_extent(args.next(), "width")?,
                "--height" => options.height = parse_extent(args.next(), "height")?,
                "--timeout-ms" => {
                    options.timeout = parse_duration(args.next(), "timeout-ms", 3_000, 60_000)?
                }
                "--settle-ms" => {
                    options.settle = parse_duration(args.next(), "settle-ms", 100, 5_000)?
                }
                "--locale" => {
                    options.locale = parse_locale(args.next())?;
                }
                "--theme" => {
                    options.theme = match required_value(args.next(), "theme")?.as_str() {
                        "dark" => ThemeMode::Dark,
                        "light" => ThemeMode::Light,
                        _ => return Err("theme must be dark or light".to_owned()),
                    };
                }
                _ => return Err(usage()),
            }
        }
        if !options.allow_input {
            return Err("settings-interaction-acceptance requires --allow-input".to_owned());
        }
        if (options.exercise_library_single_delete
            || options.exercise_library_search_selection
            || options.exercise_library_open_copy
            || options.exercise_library_retention)
            && (options.width < 900 || options.height < 1_000)
        {
            return Err(
                "Library history interaction acceptance requires --width >= 900 and --height >= 1000"
                .to_owned(),
            );
        }
        let library_exercise_count = [
            options.exercise_library_single_delete,
            options.exercise_library_search_selection,
            options.exercise_library_open_copy,
            options.exercise_library_retention,
        ]
        .into_iter()
        .filter(|enabled| *enabled)
        .count();
        if library_exercise_count > 1 {
            return Err("Library interaction exercises must run separately".to_owned());
        }
        Ok(options)
    }
}

fn usage() -> String {
    "usage: settings-interaction-acceptance --allow-input [--exercise-app-update] [--exercise-record-support] [--exercise-record-start] [--exercise-record-success] [--exercise-library-format] [--exercise-library-search-selection] [--exercise-library-single-delete] [--exercise-library-open-copy] [--exercise-library-retention] [--exercise-pin-appearance] [--output-dir <path>] [--width <px>] [--height <px>] [--timeout-ms <3000-60000>] [--settle-ms <100-5000>] [--locale <en|zh-CN>] [--theme <dark|light>]".to_owned()
}

fn required_value(value: Option<std::ffi::OsString>, name: &str) -> Result<String, String> {
    value
        .ok_or_else(|| format!("{name} requires a value"))?
        .into_string()
        .map_err(|_| format!("{name} must be valid UTF-8"))
}

/// Parses a logical window extent and keeps the test target inside documented desktop sizes.
fn parse_extent(value: Option<std::ffi::OsString>, name: &str) -> Result<i32, String> {
    let value = required_value(value, name)?;
    let value = value
        .parse::<i32>()
        .map_err(|_| format!("{name} must be a number"))?;
    if !(420..=2_000).contains(&value) {
        return Err(format!("{name} must be between 420 and 2000"));
    }
    Ok(value)
}

/// Parses a bounded timeout so an unresponsive native window cannot hold the runner indefinitely.
fn parse_duration(
    value: Option<std::ffi::OsString>,
    name: &str,
    minimum: u64,
    maximum: u64,
) -> Result<Duration, String> {
    let value = required_value(value, name)?
        .parse::<u64>()
        .map_err(|_| format!("{name} must be a number"))?;
    if !(minimum..=maximum).contains(&value) {
        return Err(format!("{name} must be between {minimum} and {maximum}"));
    }
    Ok(Duration::from_millis(value))
}

/// Resolves one supported UI catalog for an isolated real-window run.
fn parse_locale(value: Option<std::ffi::OsString>) -> Result<Locale, String> {
    match required_value(value, "locale")?.as_str() {
        "en" => Ok(Locale::English),
        "zh-CN" => Ok(Locale::SimplifiedChinese),
        _ => Err("locale must be en or zh-CN".to_owned()),
    }
}

pub(super) fn entrypoint() {
    if let Err(error) = run() {
        eprintln!("settings interaction acceptance failed: {error}");
        process::exit(1);
    }
}

/// Starts the native probe only on Windows, where real desktop input is available.
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let options = Options::parse().map_err(io::Error::other)?;
    #[cfg(not(windows))]
    {
        let _ = options;
        return Err(io::Error::other("settings interaction acceptance requires Windows").into());
    }
    #[cfg(windows)]
    {
        run_windows(options)
    }
}

#[cfg(windows)]
/// Creates an isolated GPUI settings process and the worker that sends authorized desktop input.
fn run_windows(options: Options) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(&options.output_dir)?;
    let output_dir = std::path::absolute(&options.output_dir)?;
    let session_dir = output_dir.join(format!("session-{}", std::process::id()));
    fs::create_dir_all(&session_dir)?;
    let history_dir = if options.exercise_library_format {
        PathBuf::from(format!(r"{ACCEPTANCE_LIBRARY_ROOT}\h-{}", process::id()))
    } else {
        session_dir.join("history")
    };
    fs::create_dir_all(&history_dir)?;
    let mut history = ScreenshotHistory::open_with_limit(&history_dir, 30)?;
    let library_remove_paths = if options.exercise_library_single_delete {
        Some(create_single_delete_fixtures(&mut history, &history_dir)?)
    } else {
        None
    };
    let library_open_path = if options.exercise_library_open_copy {
        Some(create_library_open_fixture(&mut history, &history_dir)?)
    } else {
        None
    };
    let library_retention_paths = if options.exercise_library_retention {
        Some(create_library_retention_fixtures(
            &mut history,
            &history_dir,
        )?)
    } else {
        None
    };
    if options.exercise_library_search_selection {
        create_library_search_fixtures(&mut history, &history_dir)?;
    }
    let performance = PerformanceRecorder::new(session_dir.join("metrics"))?;
    let mut settings = UserSettings::default();
    settings.locale = options.locale;
    settings.theme_mode = options.theme;
    if options.exercise_record_success {
        let recording_dir = session_dir.join("recordings");
        fs::create_dir_all(&recording_dir)?;
        settings.recording_directory = Some(recording_dir);
    }
    let settings_path = session_dir.join("settings.json");
    if options.exercise_record_support || options.exercise_record_start {
        // SAFETY: the isolated runner sets this before starting GPUI or the input worker, and
        // the process exits after the single acceptance session completes.
        unsafe {
            std::env::set_var(
                "FLASH_SHOT_RECORDING_DIRECTORY",
                ACCEPTANCE_RECORDING_DIRECTORY,
            );
        }
    }
    let (command_tx, command_rx) = async_channel::bounded(1);
    let quit_commands = command_tx.clone();
    let worker_options = input::WorkerOptions {
        output_dir: session_dir.clone(),
        width: options.width,
        height: options.height,
        timeout: options.timeout,
        settle: options.settle,
        locale: options.locale,
        theme: options.theme,
        exercise_app_update: options.exercise_app_update,
        exercise_record_support: options.exercise_record_support,
        exercise_record_start: options.exercise_record_start,
        exercise_record_success: options.exercise_record_success,
        exercise_library_format: options.exercise_library_format,
        exercise_library_search_selection: options.exercise_library_search_selection,
        exercise_library_single_delete: options.exercise_library_single_delete,
        exercise_library_open_copy: options.exercise_library_open_copy,
        exercise_library_retention: options.exercise_library_retention,
        library_remove_paths,
        library_open_path,
        library_retention_paths,
        exercise_pin_appearance: options.exercise_pin_appearance,
        commands: command_tx,
    };
    let (input_error_tx, input_error_rx) = mpsc::channel();
    let input_worker = thread::spawn(move || {
        if let Err(error) = input::run_input_probe(worker_options) {
            let error = error.to_string();
            eprintln!("settings input probe failed: {error}");
            let _ = input_error_tx.send(error);
            let (quit_tx, quit_rx) = mpsc::sync_channel(1);
            if quit_commands
                .send_blocking(flash_shot::SettingsInteractionAcceptanceCommand::Quit(
                    quit_tx,
                ))
                .is_ok()
            {
                let _ = quit_rx.recv_timeout(Duration::from_secs(2));
            }
        }
    });
    let application_result = flash_shot::run_settings_interaction_acceptance(
        Instant::now(),
        performance,
        history,
        settings,
        settings_path,
        SettingsInteractionAcceptanceOptions {
            width: options.width as f32,
            height: options.height as f32,
            update_check_state: if options.exercise_app_update {
                flash_shot::UpdateUiAcceptanceState::Checking
            } else {
                flash_shot::UpdateUiAcceptanceState::Idle
            },
            recording_support_check_state: if options.exercise_record_support {
                flash_shot::RecordingSupportUiAcceptanceState::Checking
            } else {
                flash_shot::RecordingSupportUiAcceptanceState::Idle
            },
            recording_state: if options.exercise_record_start {
                flash_shot::RecordingUiAcceptanceState::Starting
            } else {
                flash_shot::RecordingUiAcceptanceState::Idle
            },
            exercise_pin_appearance: options.exercise_pin_appearance,
            commands: command_rx,
        },
    );
    input_worker
        .join()
        .map_err(|_| io::Error::other("settings input acceptance worker panicked"))?;
    if options.exercise_library_format
        || options.exercise_library_search_selection
        || options.exercise_library_single_delete
        || options.exercise_library_open_copy
        || options.exercise_library_retention
    {
        remove_acceptance_history_root(&history_dir)?;
    }
    application_result?;
    if let Ok(error) = input_error_rx.try_recv() {
        return Err(io::Error::other(error).into());
    }
    Ok(())
}

#[cfg(windows)]
/// Seeds two managed captures so the real-input probe can prove one removal preserves its neighbor.
fn create_single_delete_fixtures(
    history: &mut ScreenshotHistory,
    root: &std::path::Path,
) -> io::Result<(PathBuf, PathBuf)> {
    for index in 0..2 {
        let path = root.join(format!("library-delete-{index}.png"));
        acceptance_fixture_frame(index).save_png(&path)?;
        history.record_with_source(path, HistorySource::Selection)?;
        thread::sleep(Duration::from_millis(5));
    }
    let entries = history.entries();
    let target = entries
        .front()
        .map(|entry| entry.path.clone())
        .ok_or_else(|| io::Error::other("single-delete fixture has no first entry"))?;
    let neighbor = entries
        .get(1)
        .map(|entry| entry.path.clone())
        .ok_or_else(|| io::Error::other("single-delete fixture has no neighboring entry"))?;
    Ok((target, neighbor))
}

#[cfg(windows)]
/// Seeds source- and filename-distinct rows for real Library filter, search, and selection input.
fn create_library_search_fixtures(
    history: &mut ScreenshotHistory,
    root: &std::path::Path,
) -> io::Result<()> {
    let fixtures = [
        ("library-needle-only.png", HistorySource::Selection),
        ("library-selection-other.png", HistorySource::Selection),
        ("library-scroll-other.png", HistorySource::Scrolling),
    ];
    for (index, (name, source)) in fixtures.into_iter().enumerate() {
        let path = root.join(name);
        acceptance_fixture_frame(index).save_png(&path)?;
        history.record_with_source(path, source)?;
        thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}

#[cfg(windows)]
/// Seeds one retained image whose exact pixels and file contents can be checked after Library use.
fn create_library_open_fixture(
    history: &mut ScreenshotHistory,
    root: &std::path::Path,
) -> io::Result<PathBuf> {
    let path = root.join("library-open-copy.png");
    acceptance_fixture_frame(0).save_png(&path)?;
    history.record_with_source(path.clone(), HistorySource::Selection)?;
    Ok(path)
}

#[cfg(windows)]
/// Seeds ordered managed captures so retention proves which files it prunes and preserves.
fn create_library_retention_fixtures(
    history: &mut ScreenshotHistory,
    root: &std::path::Path,
) -> io::Result<Vec<PathBuf>> {
    let mut paths = Vec::with_capacity(12);
    for index in 0..12 {
        let path = root.join(format!("library-retention-{index:02}.png"));
        acceptance_fixture_frame(index).save_png(&path)?;
        history.record_with_source(path.clone(), HistorySource::Selection)?;
        paths.push(path);
        thread::sleep(Duration::from_millis(5));
    }
    Ok(paths)
}

#[cfg(windows)]
/// Creates a small, fully decodable PNG for Library preview and deletion acceptance.
fn acceptance_fixture_frame(index: usize) -> CaptureFrame {
    const WIDTH: u32 = 96;
    const HEIGHT: u32 = 64;
    let mut pixels = vec![0_u8; WIDTH as usize * HEIGHT as usize * 4];
    for (offset, pixel) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        pixel[0] = ((offset + index * 17) % 251) as u8;
        pixel[1] = ((offset / WIDTH as usize + index * 31) % 251) as u8;
        pixel[2] = ((offset / 4 + index * 47) % 251) as u8;
        pixel[3] = 255;
    }
    CaptureFrame {
        bounds: PhysicalRect {
            left: 0,
            top: 0,
            right: WIDTH as i32,
            bottom: HEIGHT as i32,
        },
        width: WIDTH,
        height: HEIGHT,
        stride: WIDTH as usize * 4,
        format: PixelFormat::Bgra8,
        pixels: Arc::from(pixels),
        capture_duration: Duration::ZERO,
        cpu_copy_count: 1,
    }
}

#[cfg(windows)]
/// Removes the short Library acceptance root after GPUI releases its history handles.
fn remove_acceptance_history_root(path: &std::path::Path) -> io::Result<()> {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        if !path.exists() {
            return Ok(());
        }
        match fs::remove_dir_all(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) if std::time::Instant::now() >= deadline => return Err(error),
            Err(_) => {}
        }
        if std::time::Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "acceptance history root remains after cleanup: {}",
                    path.display()
                ),
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::{Options, parse_duration, parse_extent};
    use std::ffi::OsString;

    #[test]
    fn dimensions_and_timeouts_stay_inside_native_probe_limits() {
        assert_eq!(parse_extent(Some("520".into()), "width").unwrap(), 520);
        assert!(parse_extent(Some("419".into()), "width").is_err());
        assert_eq!(
            parse_duration(Some("500".into()), "settle-ms", 100, 5_000)
                .unwrap()
                .as_millis(),
            500
        );
        assert!(parse_duration(Some("99".into()), "settle-ms", 100, 5_000).is_err());
    }

    #[test]
    fn input_requires_the_explicit_opt_in_argument() {
        assert!(Options::parse_args(std::iter::empty()).is_err());
        let options = Options::parse_args([OsString::from("--allow-input")].into_iter()).unwrap();
        assert!(options.allow_input);
        assert!(!options.exercise_app_update);
        assert!(!options.exercise_record_support);
        assert!(!options.exercise_record_start);
        assert!(!options.exercise_record_success);
        assert!(!options.exercise_library_format);
        assert!(!options.exercise_library_search_selection);
        assert!(!options.exercise_library_single_delete);
        assert!(!options.exercise_library_open_copy);
        assert!(!options.exercise_library_retention);
        assert!(!options.exercise_pin_appearance);
        let options = Options::parse_args(
            [
                OsString::from("--allow-input"),
                OsString::from("--exercise-app-update"),
            ]
            .into_iter(),
        )
        .unwrap();
        assert!(options.exercise_app_update);
        let options = Options::parse_args(
            [
                OsString::from("--allow-input"),
                OsString::from("--exercise-record-support"),
            ]
            .into_iter(),
        )
        .unwrap();
        assert!(options.exercise_record_support);
        let options = Options::parse_args(
            [
                OsString::from("--allow-input"),
                OsString::from("--exercise-record-start"),
            ]
            .into_iter(),
        )
        .unwrap();
        assert!(options.exercise_record_start);
        let options = Options::parse_args(
            [
                OsString::from("--allow-input"),
                OsString::from("--exercise-record-success"),
            ]
            .into_iter(),
        )
        .unwrap();
        assert!(options.exercise_record_success);
        let options = Options::parse_args(
            [
                OsString::from("--allow-input"),
                OsString::from("--exercise-library-format"),
            ]
            .into_iter(),
        )
        .unwrap();
        assert!(options.exercise_library_format);
        assert!(
            Options::parse_args(
                [
                    OsString::from("--allow-input"),
                    OsString::from("--exercise-library-single-delete"),
                    OsString::from("--width"),
                    OsString::from("980"),
                    OsString::from("--height"),
                    OsString::from("1400"),
                ]
                .into_iter()
            )
            .unwrap()
            .exercise_library_single_delete
        );
        assert!(
            Options::parse_args(
                [
                    OsString::from("--allow-input"),
                    OsString::from("--exercise-library-open-copy"),
                    OsString::from("--width"),
                    OsString::from("980"),
                    OsString::from("--height"),
                    OsString::from("1400"),
                ]
                .into_iter()
            )
            .unwrap()
            .exercise_library_open_copy
        );
        assert!(
            Options::parse_args(
                [
                    OsString::from("--allow-input"),
                    OsString::from("--exercise-library-retention"),
                    OsString::from("--width"),
                    OsString::from("980"),
                    OsString::from("--height"),
                    OsString::from("1400"),
                ]
                .into_iter()
            )
            .unwrap()
            .exercise_library_retention
        );
        assert!(
            Options::parse_args(
                [
                    OsString::from("--allow-input"),
                    OsString::from("--exercise-library-search-selection"),
                    OsString::from("--width"),
                    OsString::from("980"),
                    OsString::from("--height"),
                    OsString::from("1400"),
                ]
                .into_iter()
            )
            .unwrap()
            .exercise_library_search_selection
        );
        assert!(
            Options::parse_args(
                [
                    OsString::from("--allow-input"),
                    OsString::from("--exercise-library-open-copy"),
                ]
                .into_iter()
            )
            .is_err()
        );
        assert!(
            Options::parse_args(
                [
                    OsString::from("--allow-input"),
                    OsString::from("--exercise-library-retention"),
                    OsString::from("--exercise-library-open-copy"),
                    OsString::from("--width"),
                    OsString::from("980"),
                    OsString::from("--height"),
                    OsString::from("1400"),
                ]
                .into_iter()
            )
            .is_err()
        );
        assert!(
            Options::parse_args(
                [
                    OsString::from("--allow-input"),
                    OsString::from("--exercise-library-open-copy"),
                    OsString::from("--exercise-library-single-delete"),
                    OsString::from("--width"),
                    OsString::from("980"),
                    OsString::from("--height"),
                    OsString::from("1400"),
                ]
                .into_iter()
            )
            .is_err()
        );
        assert!(
            Options::parse_args(
                [
                    OsString::from("--allow-input"),
                    OsString::from("--exercise-library-search-selection"),
                ]
                .into_iter()
            )
            .is_err()
        );
        assert!(
            Options::parse_args(
                [
                    OsString::from("--allow-input"),
                    OsString::from("--exercise-library-single-delete"),
                    OsString::from("--width"),
                    OsString::from("980"),
                ]
                .into_iter()
            )
            .is_err()
        );
        let options = Options::parse_args(
            [
                OsString::from("--allow-input"),
                OsString::from("--exercise-pin-appearance"),
            ]
            .into_iter(),
        )
        .unwrap();
        assert!(options.exercise_pin_appearance);
    }
}
