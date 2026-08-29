//! Isolated Windows acceptance runner for three real Pin windows without global input injection.

use std::{
    ffi::OsString,
    fs, io,
    path::PathBuf,
    process,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use flash_shot::{
    PinLifecycleAcceptanceOptions,
    history::ScreenshotHistory,
    i18n::Locale,
    performance::PerformanceRecorder,
    platform::display::{DisplayProvider, SystemDisplayProvider},
    settings::UserSettings,
    theme::ThemeMode,
};

const DEFAULT_OUTPUT_DIR: &str = "target/pin-lifecycle-acceptance";
const DEFAULT_LOCALE: Locale = Locale::English;
const DEFAULT_THEME: ThemeMode = ThemeMode::Dark;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);
const DEFAULT_SETTLE_DELAY: Duration = Duration::from_millis(350);
const DEFAULT_SOAK_DURATION: Duration = Duration::ZERO;
const MIN_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const MIN_SETTLE_DELAY: Duration = Duration::from_millis(100);
const MAX_SETTLE_DELAY: Duration = Duration::from_secs(3);
const MIN_SOAK_DURATION: Duration = Duration::from_secs(10);
const MAX_SOAK_DURATION: Duration = Duration::from_secs(10 * 60);
const SOAK_WATCHDOG_OVERHEAD: Duration = Duration::from_secs(10);

#[derive(Debug, Eq, PartialEq)]
struct Options {
    output_dir: PathBuf,
    locale: Locale,
    theme_mode: ThemeMode,
    timeout: Duration,
    settle_delay: Duration,
    soak_duration: Duration,
}

impl Options {
    fn parse() -> Result<Self, String> {
        Self::parse_from(std::env::args_os().skip(1))
    }

    /// Parses bounded options before creating a profile, GPUI window, or evidence directory.
    fn parse_from(arguments: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let mut options = Self {
            output_dir: PathBuf::from(DEFAULT_OUTPUT_DIR),
            locale: DEFAULT_LOCALE,
            theme_mode: DEFAULT_THEME,
            timeout: DEFAULT_TIMEOUT,
            settle_delay: DEFAULT_SETTLE_DELAY,
            soak_duration: DEFAULT_SOAK_DURATION,
        };
        let mut arguments = arguments.into_iter();
        let mut output_seen = false;
        let mut locale_seen = false;
        let mut theme_seen = false;
        let mut timeout_seen = false;
        let mut settle_seen = false;
        let mut soak_seen = false;
        while let Some(argument) = arguments.next() {
            let argument = argument
                .into_string()
                .map_err(|_| "acceptance options must be valid Unicode".to_owned())?;
            match argument.as_str() {
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
                "--locale" if !locale_seen => {
                    options.locale = parse_locale(arguments.next())?;
                    locale_seen = true;
                }
                "--theme" if !theme_seen => {
                    options.theme_mode = parse_theme(arguments.next())?;
                    theme_seen = true;
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
                "--soak-ms" if !soak_seen => {
                    options.soak_duration = parse_duration(
                        arguments.next(),
                        "soak duration",
                        MIN_SOAK_DURATION,
                        MAX_SOAK_DURATION,
                    )?;
                    soak_seen = true;
                }
                "--output-dir" | "--locale" | "--theme" | "--timeout-ms" | "--settle-ms"
                | "--soak-ms" => {
                    return Err(format!("{argument} may only be supplied once"));
                }
                _ => return Err(usage()),
            }
        }
        if options.output_dir.as_os_str().is_empty() {
            return Err("output directory must not be empty".to_owned());
        }
        if options.soak_duration > Duration::ZERO
            && options.timeout < options.soak_duration + SOAK_WATCHDOG_OVERHEAD
        {
            return Err(format!(
                "timeout must allow at least {} milliseconds beyond the soak duration",
                SOAK_WATCHDOG_OVERHEAD.as_millis()
            ));
        }
        Ok(options)
    }
}

/// Parses the stable locale identifiers accepted by the Windows evidence scripts.
fn parse_locale(value: Option<OsString>) -> Result<Locale, String> {
    let value = value
        .and_then(|value| value.into_string().ok())
        .ok_or_else(usage)?;
    match value.as_str() {
        "en" => Ok(Locale::English),
        "zh-CN" => Ok(Locale::SimplifiedChinese),
        _ => Err("locale must be en or zh-CN".to_owned()),
    }
}

/// Parses the stable theme identifiers accepted by the Windows evidence scripts.
fn parse_theme(value: Option<OsString>) -> Result<ThemeMode, String> {
    let value = value
        .and_then(|value| value.into_string().ok())
        .ok_or_else(usage)?;
    match value.as_str() {
        "dark" => Ok(ThemeMode::Dark),
        "light" => Ok(ThemeMode::Light),
        _ => Err("theme must be dark or light".to_owned()),
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
    "usage: pin-lifecycle-acceptance [--output-dir <path>] [--locale <en|zh-CN>] [--theme <dark|light>] [--timeout-ms <3000-900000>] [--settle-ms <100-3000>] [--soak-ms <10000-600000>]".to_owned()
}

pub(super) fn entrypoint() {
    if let Err(error) = run() {
        eprintln!("pin lifecycle acceptance failed: {error}");
        process::exit(1);
    }
}

/// Creates a unique isolated profile, then delegates every Pin action to the production app code.
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let options = Options::parse().map_err(io::Error::other)?;
    #[cfg(not(windows))]
    {
        let _ = options;
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Pin lifecycle acceptance is currently Windows-only",
        )
        .into());
    }
    #[cfg(windows)]
    {
        let displays = SystemDisplayProvider.displays()?;
        if displays.len() != 1 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!(
                    "single-display Pin acceptance requires exactly one display, found {}",
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
            .map_err(io::Error::other)?
            .as_millis();
        let session_root = output_root.join(format!("session-{timestamp}-{}", process::id()));
        fs::create_dir_all(session_root.join("screenshots"))?;
        let session_root = fs::canonicalize(session_root)?;

        // This is process-local and happens before history or GPUI starts, so fallback paths remain
        // inside the disposable evidence session even when the user's real Flash Shot is running.
        unsafe { std::env::set_var("FLASH_SHOT_PROFILE_DIR", &session_root) };
        let settings_path = session_root.join("settings.json");
        let mut settings = UserSettings::default();
        settings.locale = options.locale;
        settings.theme_mode = options.theme_mode;
        settings.capture_shortcut_enabled = false;
        settings.full_screen_shortcut = None;
        settings.focused_window_shortcut = None;
        settings.quick_save_directory = Some(session_root.join("history"));
        settings.save(&settings_path)?;
        let history = ScreenshotHistory::open_with_limit(session_root.join("history"), 30)?;
        let performance = PerformanceRecorder::new(session_root.join("metrics"))?;
        let report_path = session_root.join("report.json");
        println!("pin lifecycle report: {}", report_path.display());

        flash_shot::run_pin_lifecycle_acceptance(
            performance,
            history,
            settings,
            settings_path,
            PinLifecycleAcceptanceOptions {
                session_root,
                display,
                timeout: options.timeout,
                settle_delay: options.settle_delay,
                soak_duration: options.soak_duration,
                locale: options.locale,
                theme_mode: options.theme_mode,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_LOCALE, DEFAULT_OUTPUT_DIR, DEFAULT_THEME, Options};
    use flash_shot::{i18n::Locale, theme::ThemeMode};
    use std::{ffi::OsString, path::PathBuf, time::Duration};

    fn arguments(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn parser_uses_safe_defaults_and_accepts_bounded_overrides() {
        let defaults = Options::parse_from(arguments(&[])).unwrap();
        assert_eq!(defaults.output_dir, PathBuf::from(DEFAULT_OUTPUT_DIR));
        assert_eq!(defaults.locale, DEFAULT_LOCALE);
        assert_eq!(defaults.theme_mode, DEFAULT_THEME);
        assert_eq!(defaults.timeout, Duration::from_secs(15));
        assert_eq!(defaults.soak_duration, Duration::ZERO);

        let options = Options::parse_from(arguments(&[
            "--output-dir",
            "evidence",
            "--locale",
            "zh-CN",
            "--theme",
            "light",
            "--timeout-ms",
            "20000",
            "--settle-ms",
            "500",
            "--soak-ms",
            "10000",
        ]))
        .unwrap();
        assert_eq!(options.output_dir, PathBuf::from("evidence"));
        assert_eq!(options.locale, Locale::SimplifiedChinese);
        assert_eq!(options.theme_mode, ThemeMode::Light);
        assert_eq!(options.timeout, Duration::from_secs(20));
        assert_eq!(options.settle_delay, Duration::from_millis(500));
        assert_eq!(options.soak_duration, Duration::from_secs(10));
    }

    #[test]
    fn parser_rejects_duplicate_and_out_of_range_options() {
        assert!(
            Options::parse_from(arguments(&["--output-dir", "a", "--output-dir", "b"])).is_err()
        );
        assert!(Options::parse_from(arguments(&["--locale", "en", "--locale", "zh-CN"])).is_err());
        assert!(Options::parse_from(arguments(&["--locale", "fr"])).is_err());
        assert!(Options::parse_from(arguments(&["--theme", "blue"])).is_err());
        assert!(Options::parse_from(arguments(&["--theme", "dark", "--theme", "light"])).is_err());
        assert!(Options::parse_from(arguments(&["--timeout-ms", "2999"])).is_err());
        assert!(Options::parse_from(arguments(&["--settle-ms", "3001"])).is_err());
        assert!(Options::parse_from(arguments(&["--soak-ms", "9999"])).is_err());
        assert!(
            Options::parse_from(arguments(&["--timeout-ms", "19000", "--soak-ms", "10000",]))
                .is_err()
        );
    }
}
