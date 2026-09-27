//! Real Windows mouse and keyboard acceptance for the settings navigation shell.

use std::{fs, io, path::PathBuf, process, time::Duration};
#[cfg(windows)]
use std::{thread, time::Instant};

use flash_shot::{
    SettingsInteractionAcceptanceOptions, history::ScreenshotHistory, i18n::Locale,
    performance::PerformanceRecorder, settings::UserSettings, theme::ThemeMode,
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
#[cfg(windows)]
const INPUT_SETTLE_DELAY: Duration = Duration::from_millis(120);

#[derive(Debug)]
struct Options {
    allow_input: bool,
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
        Ok(options)
    }
}

fn usage() -> String {
    "usage: settings-interaction-acceptance --allow-input [--output-dir <path>] [--width <px>] [--height <px>] [--timeout-ms <3000-60000>] [--settle-ms <100-5000>] [--locale <en|zh-CN>] [--theme <dark|light>]".to_owned()
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
    let history_dir = session_dir.join("history");
    fs::create_dir_all(&history_dir)?;
    let history = ScreenshotHistory::open_with_limit(&history_dir, 30)?;
    let performance = PerformanceRecorder::new(session_dir.join("metrics"))?;
    let mut settings = UserSettings::default();
    settings.locale = options.locale;
    settings.theme_mode = options.theme;
    let settings_path = session_dir.join("settings.json");
    let (command_tx, command_rx) = async_channel::bounded(1);
    let worker_options = input::WorkerOptions {
        output_dir: session_dir.clone(),
        width: options.width,
        height: options.height,
        timeout: options.timeout,
        settle: options.settle,
        locale: options.locale,
        theme: options.theme,
        commands: command_tx,
    };
    thread::spawn(move || {
        if let Err(error) = input::run_input_probe(worker_options) {
            eprintln!("settings input probe failed: {error}");
            process::exit(1);
        }
    });
    flash_shot::run_settings_interaction_acceptance(
        Instant::now(),
        performance,
        history,
        settings,
        settings_path,
        SettingsInteractionAcceptanceOptions {
            width: options.width as f32,
            height: options.height as f32,
            commands: command_rx,
        },
    )?;
    Ok(())
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
    }
}
