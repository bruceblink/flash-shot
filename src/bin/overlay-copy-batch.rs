//! Runs isolated real Windows Copy acceptance sessions and aggregates their timing evidence.

use std::{
    env,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process::{self, Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use sha2::Digest;

#[cfg(windows)]
use flash_shot::platform::process_group::ProcessGroup;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const DEFAULT_ITERATIONS: usize = 30;
const MIN_ITERATIONS: usize = 30;
const MAX_ITERATIONS: usize = 200;
const DEFAULT_WARMUP: usize = 2;
const MAX_WARMUP: usize = 20;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const MIN_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_SETTLE_DELAY: Duration = Duration::from_millis(500);
const MIN_SETTLE_DELAY: Duration = Duration::from_millis(100);
const MAX_SETTLE_DELAY: Duration = Duration::from_secs(5);
const DEFAULT_MAX_P95_MS: f64 = 250.0;
const REPORT_SCHEMA_VERSION: u32 = 1;
// The runner owns its own workflow deadline and report write. Give it a small tail after the
// requested limit so the batch can preserve that report instead of killing it at the same tick.
const RUNNER_REPORT_GRACE: Duration = Duration::from_secs(2);
// Windows releases a process-scoped RegisterHotKey registration during process teardown. Keep
// consecutive isolated samples apart long enough that the next child does not race that release.
const HOTKEY_RELEASE_SETTLE: Duration = Duration::from_millis(150);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CopyTrigger {
    Toolbar,
    Enter,
}

impl CopyTrigger {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "toolbar" => Ok(Self::Toolbar),
            "enter" => Ok(Self::Enter),
            _ => Err("copy trigger must be 'toolbar' or 'enter'".to_owned()),
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Toolbar => "toolbar",
            Self::Enter => "enter",
        }
    }
}

#[derive(Debug, PartialEq)]
struct Options {
    allow_input: bool,
    allow_system_clipboard: bool,
    iterations: usize,
    warmup: usize,
    runner: Option<PathBuf>,
    output_dir: PathBuf,
    copy_trigger: CopyTrigger,
    timeout: Duration,
    settle_delay: Duration,
    max_p95_ms: f64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            allow_input: false,
            allow_system_clipboard: false,
            iterations: DEFAULT_ITERATIONS,
            warmup: DEFAULT_WARMUP,
            runner: None,
            output_dir: PathBuf::from("target/overlay-copy-batch"),
            copy_trigger: CopyTrigger::Toolbar,
            timeout: DEFAULT_TIMEOUT,
            settle_delay: DEFAULT_SETTLE_DELAY,
            max_p95_ms: DEFAULT_MAX_P95_MS,
        }
    }
}

impl Options {
    fn parse() -> Result<Self, String> {
        Self::parse_from(env::args_os().skip(1))
    }

    /// Parses the batch contract without launching a child or touching global input.
    fn parse_from(arguments: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let mut options = Self::default();
        let mut arguments = arguments.into_iter();
        let mut iterations_seen = false;
        let mut warmup_seen = false;
        let mut runner_seen = false;
        let mut output_seen = false;
        let mut trigger_seen = false;
        let mut timeout_seen = false;
        let mut settle_seen = false;
        let mut max_p95_seen = false;
        while let Some(argument) = arguments.next() {
            let argument = argument
                .into_string()
                .map_err(|_| "batch options must be valid Unicode".to_owned())?;
            match argument.as_str() {
                "--allow-input" if !options.allow_input => options.allow_input = true,
                "--allow-input" => return Err("--allow-input may only be supplied once".to_owned()),
                "--allow-system-clipboard" if !options.allow_system_clipboard => {
                    options.allow_system_clipboard = true;
                }
                "--allow-system-clipboard" => {
                    return Err("--allow-system-clipboard may only be supplied once".to_owned());
                }
                "--copy-iterations" | "--iterations" if !iterations_seen => {
                    options.iterations = parse_usize(
                        arguments.next(),
                        "copy iterations",
                        MIN_ITERATIONS,
                        MAX_ITERATIONS,
                    )?;
                    iterations_seen = true;
                }
                "--copy-iterations" | "--iterations" => {
                    return Err("--copy-iterations may only be supplied once".to_owned());
                }
                "--warmup" if !warmup_seen => {
                    options.warmup = parse_usize(arguments.next(), "warmup", 0, MAX_WARMUP)?;
                    warmup_seen = true;
                }
                "--warmup" => return Err("--warmup may only be supplied once".to_owned()),
                "--runner" if !runner_seen => {
                    options.runner = Some(PathBuf::from(
                        arguments
                            .next()
                            .ok_or_else(usage)?
                            .into_string()
                            .map_err(|_| "runner path must be valid Unicode".to_owned())?,
                    ));
                    runner_seen = true;
                }
                "--runner" => return Err("--runner may only be supplied once".to_owned()),
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
                "--output-dir" => {
                    return Err("--output-dir may only be supplied once".to_owned());
                }
                "--copy-trigger" if !trigger_seen => {
                    let trigger = arguments
                        .next()
                        .ok_or_else(usage)?
                        .into_string()
                        .map_err(|_| "copy trigger must be valid Unicode".to_owned())?;
                    options.copy_trigger = CopyTrigger::parse(&trigger)?;
                    trigger_seen = true;
                }
                "--copy-trigger" => {
                    return Err("--copy-trigger may only be supplied once".to_owned());
                }
                "--timeout-ms" if !timeout_seen => {
                    options.timeout =
                        parse_duration(arguments.next(), "timeout", MIN_TIMEOUT, MAX_TIMEOUT)?;
                    timeout_seen = true;
                }
                "--timeout-ms" => return Err("--timeout-ms may only be supplied once".to_owned()),
                "--settle-ms" if !settle_seen => {
                    options.settle_delay = parse_duration(
                        arguments.next(),
                        "settle delay",
                        MIN_SETTLE_DELAY,
                        MAX_SETTLE_DELAY,
                    )?;
                    settle_seen = true;
                }
                "--settle-ms" => return Err("--settle-ms may only be supplied once".to_owned()),
                "--max-p95-ms" if !max_p95_seen => {
                    options.max_p95_ms = arguments
                        .next()
                        .and_then(|value| value.into_string().ok())
                        .ok_or_else(usage)?
                        .parse::<f64>()
                        .map_err(|_| "max p95 must be a finite positive number".to_owned())?;
                    if !options.max_p95_ms.is_finite() || options.max_p95_ms <= 0.0 {
                        return Err("max p95 must be a finite positive number".to_owned());
                    }
                    max_p95_seen = true;
                }
                "--max-p95-ms" => {
                    return Err("--max-p95-ms may only be supplied once".to_owned());
                }
                _ => return Err(usage()),
            }
        }
        if options.output_dir.as_os_str().is_empty() {
            return Err("output directory must not be empty".to_owned());
        }
        if options
            .runner
            .as_ref()
            .is_some_and(|path| path.as_os_str().is_empty())
        {
            return Err("runner path must not be empty".to_owned());
        }
        Ok(options)
    }
}

fn parse_usize(
    value: Option<OsString>,
    label: &str,
    minimum: usize,
    maximum: usize,
) -> Result<usize, String> {
    let value = value
        .and_then(|value| value.into_string().ok())
        .ok_or_else(usage)?;
    let parsed = value
        .parse::<usize>()
        .map_err(|_| format!("{label} must be a whole number"))?;
    if !(minimum..=maximum).contains(&parsed) {
        return Err(format!("{label} must be between {minimum} and {maximum}"));
    }
    Ok(parsed)
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
    "usage: overlay-copy-batch --allow-input --allow-system-clipboard [--copy-iterations <30-200>] [--warmup <0-20>] [--runner <path>] [--output-dir <path>] [--copy-trigger <toolbar|enter>] [--timeout-ms <3000-60000>] [--settle-ms <100-5000>] [--max-p95-ms <positive-ms>]".to_owned()
}

fn ensure_authorized(options: &Options) -> io::Result<()> {
    if !options.allow_input {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "real UI batch requires explicit --allow-input",
        ));
    }
    if !options.allow_system_clipboard {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "real UI batch changes the Windows clipboard; rerun with --allow-system-clipboard",
        ));
    }
    Ok(())
}

#[derive(Debug, serde::Deserialize)]
struct AcceptanceSnapshot {
    status: Option<String>,
    display: Option<DisplaySnapshot>,
    capture_actions: Option<CaptureActionsSnapshot>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
struct DisplaySnapshot {
    id: Option<String>,
    bounds: Option<serde_json::Value>,
    dpi_x: Option<u32>,
    dpi_y: Option<u32>,
    scale_factor: Option<f32>,
}

#[derive(Debug, serde::Deserialize)]
struct CaptureActionsSnapshot {
    copy: Option<CopySnapshot>,
    cleanup: Option<CleanupSnapshot>,
}

#[derive(Debug, serde::Deserialize)]
struct CopySnapshot {
    read_mechanism: Option<String>,
    timing_clock: Option<String>,
    timing_boundary: Option<String>,
    input_to_consumer_readable_ms: Option<f64>,
    sink: Option<String>,
    consumer_image_content: Option<ExactPixelSnapshot>,
    consumer_cleaned_up: Option<bool>,
    editor_retained_after_copy: Option<bool>,
    cleanup_after_escape: Option<bool>,
    single_export_verified: Option<bool>,
}

#[derive(Debug, serde::Deserialize)]
struct ExactPixelSnapshot {
    exact_match: Option<bool>,
}

#[derive(Debug, serde::Deserialize)]
struct CleanupSnapshot {
    overlay_count: Option<usize>,
    pinned_count: Option<usize>,
    capture_teardown_pending: Option<bool>,
    visible_process_windows: Option<usize>,
    capture_preflight_ready: Option<bool>,
}

#[derive(Debug)]
struct ParsedAcceptance {
    status: String,
    display: DisplaySnapshot,
    copy_latency_ms: f64,
    timing_clock: String,
    timing_boundary: String,
    exact_match: bool,
    cleanup_safe: bool,
    production_clipboard: bool,
}

impl ParsedAcceptance {
    /// Extracts only production system-clipboard evidence that can enter the batch percentile.
    fn from_report(path: &Path) -> io::Result<Self> {
        let report = fs::read_to_string(path)?;
        let report: AcceptanceSnapshot = serde_json::from_str(&report).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "could not parse acceptance report {}: {error}",
                    path.display()
                ),
            )
        })?;
        let display = report
            .display
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "report has no display"))?;
        let actions = report.capture_actions.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "report has no capture action report",
            )
        })?;
        let copy = actions.copy.ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "report has no Copy report")
        })?;
        let cleanup = actions.cleanup.ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "report has no cleanup report")
        })?;
        let copy_latency_ms = copy.input_to_consumer_readable_ms.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Copy report has no external consumer timing",
            )
        })?;
        if !copy_latency_ms.is_finite() || copy_latency_ms < 0.0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Copy report has an invalid external consumer timing",
            ));
        }
        let exact_match = copy
            .consumer_image_content
            .and_then(|content| content.exact_match)
            .unwrap_or(false);
        let cleanup_safe = cleanup.overlay_count == Some(0)
            && cleanup.pinned_count == Some(0)
            && cleanup.capture_teardown_pending == Some(false)
            && cleanup.visible_process_windows == Some(0)
            && cleanup.capture_preflight_ready == Some(true);
        let production_clipboard = copy.read_mechanism.as_deref()
            == Some("independent_process_png_cf_dib_and_arboard")
            && copy.sink.as_deref() == Some("system_clipboard")
            && copy.consumer_cleaned_up == Some(true)
            && copy.editor_retained_after_copy == Some(true)
            && copy.cleanup_after_escape == Some(true)
            && copy.single_export_verified == Some(true);
        Ok(Self {
            status: report.status.unwrap_or_default(),
            display,
            copy_latency_ms,
            timing_clock: copy.timing_clock.unwrap_or_default(),
            timing_boundary: copy.timing_boundary.unwrap_or_default(),
            exact_match,
            cleanup_safe,
            production_clipboard,
        })
    }
}

#[derive(Clone, Debug, serde::Serialize)]
struct LatencySample {
    iteration: usize,
    latency_ms: f64,
}

#[derive(Debug, serde::Serialize)]
struct LatencySummary {
    samples: Vec<LatencySample>,
    p50_ms: Option<f64>,
    p95_ms: Option<f64>,
    min_ms: Option<f64>,
    max_ms: Option<f64>,
}

#[derive(Debug, serde::Serialize)]
struct IterationReport {
    phase: &'static str,
    iteration: usize,
    status: &'static str,
    report_path: Option<String>,
    child_exit_code: Option<i32>,
    elapsed_ms: f64,
    copy_latency_ms: Option<f64>,
    exact_match: Option<bool>,
    cleanup_safe: Option<bool>,
    production_clipboard: Option<bool>,
    timing_clock: Option<String>,
    timing_boundary: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct FailureReport {
    phase: &'static str,
    iteration: usize,
    reason: String,
    report_path: Option<String>,
    child_exit_code: Option<i32>,
}

#[derive(Debug, serde::Serialize)]
struct BuildReport {
    batch_profile: String,
    runner_profile: String,
    runner_path: String,
    runner_sha256: Option<String>,
    runner_modified_unix_ms: Option<u128>,
    package_version: &'static str,
}

#[derive(Debug, serde::Serialize)]
struct TimeBoundaryReport {
    started_unix_ms: u128,
    finished_unix_ms: u128,
    elapsed_ms: f64,
    sample_clock: String,
    sample_boundary: String,
}

#[derive(Debug, serde::Serialize)]
struct EnvironmentReport {
    platform: String,
    display: Option<DisplaySnapshot>,
    consistent_display: bool,
}

#[derive(Debug, serde::Serialize)]
struct BatchReport {
    schema_version: u32,
    test: &'static str,
    workflow: &'static str,
    status: &'static str,
    measurement_mode: &'static str,
    real_ui: bool,
    copy_trigger: &'static str,
    requested_iterations: usize,
    warmup_iterations: usize,
    completed_iterations: usize,
    valid_samples: usize,
    failure_count: usize,
    warmup_failure_count: usize,
    total_failure_count: usize,
    max_p95_ms: f64,
    p95_passed: bool,
    sample_count_passed: bool,
    failures_zero: bool,
    latency_ms: LatencySummary,
    environment: EnvironmentReport,
    build: BuildReport,
    time: TimeBoundaryReport,
    iterations: Vec<IterationReport>,
    failures: Vec<FailureReport>,
}

#[derive(Debug)]
struct IterationOutcome {
    report: IterationReport,
    failure: Option<FailureReport>,
    parsed: Option<ParsedAcceptance>,
    safe_to_continue: bool,
}

fn main() {
    let result = Options::parse()
        .map_err(io::Error::other)
        .and_then(|options| ensure_authorized(&options).and_then(|()| run_batch(options)));
    if let Err(error) = result {
        eprintln!("overlay Copy batch failed: {error}");
        process::exit(1);
    }
}

fn run_batch(options: Options) -> io::Result<()> {
    #[cfg(not(windows))]
    {
        let _ = options;
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "overlay Copy batch is currently Windows-only",
        ));
    }
    #[cfg(windows)]
    {
        let runner = resolve_runner(options.runner.as_deref())?;
        if !runner.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "overlay interaction runner does not exist: {}",
                    runner.display()
                ),
            ));
        }
        let requested_output_root = if options.output_dir.is_absolute() {
            options.output_dir.clone()
        } else {
            std::path::absolute(&options.output_dir)?
        };
        fs::create_dir_all(&requested_output_root)?;
        let output_root = requested_output_root.join(format!(
            "session-{}-{}",
            unix_timestamp_ms(),
            process::id()
        ));
        fs::create_dir_all(&output_root)?;
        let started = Instant::now();
        let started_unix_ms = unix_timestamp_ms();
        let mut iterations = Vec::with_capacity(options.warmup + options.iterations);
        let mut failures = Vec::new();
        let mut samples = Vec::new();
        let mut display = None;
        let mut consistent_display = true;
        let mut sample_clock = String::new();
        let mut sample_boundary = String::new();
        let mut stopped_early = false;

        for index in 0..options.warmup {
            let outcome = run_iteration(&runner, &output_root, "warmup", index + 1, &options)?;
            if let Some(parsed) = outcome.parsed.as_ref() {
                consistent_display &= merge_display(&mut display, &parsed.display);
            }
            if let Some(failure) = outcome.failure {
                failures.push(failure);
            }
            iterations.push(outcome.report);
            if !outcome.safe_to_continue {
                stopped_early = true;
                break;
            }
        }
        if !stopped_early {
            for index in 0..options.iterations {
                let outcome = run_iteration(&runner, &output_root, "sample", index + 1, &options)?;
                if let Some(parsed) = outcome.parsed.as_ref() {
                    let display_matches = merge_display(&mut display, &parsed.display);
                    consistent_display &= display_matches;
                    if parsed.status == "passed"
                        && parsed.cleanup_safe
                        && parsed.exact_match
                        && parsed.production_clipboard
                        && display_matches
                    {
                        samples.push(LatencySample {
                            iteration: index + 1,
                            latency_ms: parsed.copy_latency_ms,
                        });
                        if sample_clock.is_empty() {
                            sample_clock.clone_from(&parsed.timing_clock);
                            sample_boundary.clone_from(&parsed.timing_boundary);
                        }
                    }
                }
                if let Some(failure) = outcome.failure {
                    failures.push(failure);
                }
                iterations.push(outcome.report);
                if !outcome.safe_to_continue {
                    stopped_early = true;
                    break;
                }
            }
        }

        let valid_samples = samples.len();
        let summary = latency_summary(samples);
        let completed_iterations = iterations
            .iter()
            .filter(|iteration| iteration.phase == "sample")
            .count();
        let warmup_failure_count = failures
            .iter()
            .filter(|failure| failure.phase == "warmup")
            .count();
        let failure_count = failures
            .iter()
            .filter(|failure| failure.phase == "sample")
            .count();
        let p95_passed = summary.p95_ms.is_some_and(|p95| p95 <= options.max_p95_ms);
        let sample_count_passed = completed_iterations == options.iterations
            && valid_samples == options.iterations
            && !stopped_early;
        let failures_zero = failure_count == 0 && warmup_failure_count == 0;
        let status = if sample_count_passed && p95_passed && failures_zero && consistent_display {
            "passed"
        } else {
            "failed"
        };
        let report = BatchReport {
            schema_version: REPORT_SCHEMA_VERSION,
            test: "overlay_copy_batch",
            workflow: "real_ui_copy_batch",
            status,
            measurement_mode: "real_ui",
            real_ui: true,
            copy_trigger: options.copy_trigger.label(),
            requested_iterations: options.iterations,
            warmup_iterations: options.warmup,
            completed_iterations,
            valid_samples,
            failure_count,
            warmup_failure_count,
            total_failure_count: failures.len(),
            max_p95_ms: options.max_p95_ms,
            p95_passed,
            sample_count_passed,
            failures_zero,
            latency_ms: summary,
            environment: EnvironmentReport {
                platform: format!("{}-{}", env::consts::OS, env::consts::ARCH),
                display,
                consistent_display,
            },
            build: build_report(&runner),
            time: TimeBoundaryReport {
                started_unix_ms,
                finished_unix_ms: unix_timestamp_ms(),
                elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
                sample_clock,
                sample_boundary,
            },
            iterations,
            failures,
        };
        let report_path = output_root.join("batch-report.json");
        write_json(&report_path, &report)?;
        println!("overlay Copy batch report: {}", report_path.display());
        if status == "failed" {
            return Err(io::Error::other(format!(
                "real UI batch did not pass ({} valid samples, {failure_count} sample failures)",
                report.valid_samples
            )));
        }
        Ok(())
    }
}

#[cfg(windows)]
/// Launches one isolated acceptance process and refuses to continue after unverifiable cleanup.
fn run_iteration(
    runner: &Path,
    output_root: &Path,
    phase: &'static str,
    iteration: usize,
    options: &Options,
) -> io::Result<IterationOutcome> {
    let iteration_dir = output_root.join(format!("{phase}-{iteration:03}"));
    fs::create_dir_all(&iteration_dir)?;
    let started = Instant::now();
    let mut command = Command::new(runner);
    command
        .arg("--allow-input")
        .arg("--allow-system-clipboard")
        .arg("--copy-trigger")
        .arg(options.copy_trigger.label())
        .arg("--output-dir")
        .arg(&iteration_dir)
        .arg("--timeout-ms")
        .arg(options.timeout.as_millis().to_string())
        .arg("--settle-ms")
        .arg(options.settle_delay.as_millis().to_string())
        .stdout(Stdio::null())
        // The child report is the source of truth. Avoid inheriting a pipe from descendants,
        // because collecting descendant output could otherwise outlive the root process.
        .stderr(Stdio::null());
    command.creation_flags(CREATE_NO_WINDOW);
    // The child runner has its own workflow deadline, but the batch process also needs a hard
    // outer bound so one wedged desktop session cannot consume the entire sample run. The small
    // grace interval belongs to report persistence, not the measured Copy latency.
    let output = spawn_and_wait(&mut command, runner_timeout(options.timeout));
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    let report_path = find_report(&iteration_dir).ok();
    let relative_report_path = report_path.as_deref().and_then(|path| {
        path.strip_prefix(output_root)
            .ok()
            .map(|path| path.to_string_lossy().into_owned())
    });
    let (exit_code, timed_out) = match output {
        Ok((exit_code, timed_out)) => (exit_code, timed_out),
        Err(error) => {
            let failure = failure(
                phase,
                iteration,
                error.to_string(),
                relative_report_path,
                None,
            );
            return Ok(IterationOutcome {
                report: IterationReport {
                    phase,
                    iteration,
                    status: "failed",
                    report_path: failure.report_path.clone(),
                    child_exit_code: None,
                    elapsed_ms,
                    copy_latency_ms: None,
                    exact_match: None,
                    cleanup_safe: None,
                    production_clipboard: None,
                    timing_clock: None,
                    timing_boundary: None,
                },
                failure: Some(failure),
                parsed: None,
                safe_to_continue: false,
            });
        }
    };
    let parsed_result = report_path
        .as_deref()
        .map(ParsedAcceptance::from_report)
        .transpose();
    let parse_failure = parsed_result.as_ref().err().map(ToString::to_string);
    let parsed = parsed_result.ok().flatten();
    let cleanup_safe = parsed.as_ref().is_some_and(|parsed| parsed.cleanup_safe);
    let valid = parsed.as_ref().is_some_and(|parsed| {
        parsed.status == "passed"
            && parsed.cleanup_safe
            && parsed.exact_match
            && parsed.production_clipboard
    });
    let child_failed = exit_code != Some(0);
    let reason = if timed_out {
        Some(format!(
            "acceptance runner exceeded {}ms plus {}ms report grace and was terminated",
            options.timeout.as_millis(),
            RUNNER_REPORT_GRACE.as_millis()
        ))
    } else if child_failed {
        Some(format!("acceptance runner exited with {:?}", exit_code))
    } else if parsed.is_none() {
        Some(
            parse_failure
                .unwrap_or_else(|| "acceptance runner did not produce report.json".to_owned()),
        )
    } else if !valid {
        Some("report failed production Copy, pixel, or cleanup validation".to_owned())
    } else {
        None
    };
    let report = IterationReport {
        phase,
        iteration,
        status: if valid { "passed" } else { "failed" },
        report_path: relative_report_path.clone(),
        child_exit_code: exit_code,
        elapsed_ms,
        copy_latency_ms: parsed.as_ref().map(|parsed| parsed.copy_latency_ms),
        exact_match: parsed.as_ref().map(|parsed| parsed.exact_match),
        cleanup_safe: parsed.as_ref().map(|parsed| parsed.cleanup_safe),
        production_clipboard: parsed.as_ref().map(|parsed| parsed.production_clipboard),
        timing_clock: parsed.as_ref().map(|parsed| parsed.timing_clock.clone()),
        timing_boundary: parsed.as_ref().map(|parsed| parsed.timing_boundary.clone()),
    };
    let failure =
        reason.map(|reason| failure(phase, iteration, reason, relative_report_path, exit_code));
    // A fresh runner must register the same process-wide hotkey. Wait after reaping this runner,
    // rather than allowing the next isolated child to race Windows' hotkey teardown.
    std::thread::sleep(HOTKEY_RELEASE_SETTLE);
    Ok(IterationOutcome {
        report,
        failure,
        parsed,
        safe_to_continue: cleanup_safe,
    })
}

/// Extends only the wrapper's kill deadline so a child can write its own final report.
fn runner_timeout(timeout: Duration) -> Duration {
    timeout.saturating_add(RUNNER_REPORT_GRACE)
}

#[cfg(windows)]
/// Runs one child under a kill-on-close Job Object and enforces the per-session timeout.
fn spawn_and_wait(command: &mut Command, timeout: Duration) -> io::Result<(Option<i32>, bool)> {
    let process_group = ProcessGroup::create()?;
    let mut child = command.spawn()?;
    if let Err(error) = process_group.assign(&child) {
        return Err(with_cleanup_error(
            error,
            terminate_and_reap_bounded(&process_group, &mut child, Duration::from_secs(1)).err(),
        ));
    }
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok((status.code(), false)),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                let status =
                    terminate_and_reap_bounded(&process_group, &mut child, Duration::from_secs(1))?;
                return Ok((status.code(), true));
            }
            Err(error) => {
                return Err(with_cleanup_error(
                    error,
                    terminate_and_reap_bounded(&process_group, &mut child, Duration::from_secs(1))
                        .err(),
                ));
            }
        }
    }
}

/// Terminates a child tree and reaps it within a fixed budget, preserving cleanup failures.
#[cfg(windows)]
fn terminate_and_reap_bounded(
    process_group: &ProcessGroup,
    child: &mut process::Child,
    timeout: Duration,
) -> io::Result<process::ExitStatus> {
    let terminate_error = process_group.terminate().err();
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait()? {
            Some(status) => {
                return terminate_error.map_or(Ok(status), |error| {
                    Err(io::Error::new(
                        error.kind(),
                        format!("job termination failed after child exit: {error}"),
                    ))
                });
            }
            None if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            None => break,
        }
    }

    let kill_error = child.kill().err();
    let reap_deadline = Instant::now() + Duration::from_millis(250);
    while Instant::now() < reap_deadline {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!(
            "acceptance child could not be reaped (terminate={terminate_error:?}, kill={kill_error:?})"
        ),
    ))
}

fn with_cleanup_error(original: io::Error, cleanup: Option<io::Error>) -> io::Error {
    match cleanup {
        Some(cleanup) => io::Error::new(
            original.kind(),
            format!("{original}; bounded child cleanup also failed: {cleanup}"),
        ),
        None => original,
    }
}

#[cfg(windows)]
fn resolve_runner(explicit: Option<&Path>) -> io::Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }
    let mut path = env::current_exe()?;
    path.set_file_name(format!(
        "overlay-interaction-acceptance{}",
        env::consts::EXE_SUFFIX
    ));
    Ok(path)
}

#[cfg(windows)]
fn find_report(root: &Path) -> io::Result<PathBuf> {
    let mut reports = Vec::new();
    collect_reports(root, &mut reports)?;
    match reports.as_slice() {
        [report] => Ok(report.clone()),
        [] => Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no report.json found under {}", root.display()),
        )),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("multiple report.json files found under {}", root.display()),
        )),
    }
}

#[cfg(windows)]
fn collect_reports(root: &Path, reports: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_reports(&path, reports)?;
        } else if path.file_name().is_some_and(|name| name == "report.json") {
            reports.push(path);
        }
    }
    Ok(())
}

fn merge_display(display: &mut Option<DisplaySnapshot>, candidate: &DisplaySnapshot) -> bool {
    match display {
        None => {
            *display = Some(candidate.clone());
            true
        }
        Some(current) => {
            current.id == candidate.id
                && current.bounds == candidate.bounds
                && current.dpi_x == candidate.dpi_x
                && current.dpi_y == candidate.dpi_y
                && current.scale_factor == candidate.scale_factor
        }
    }
}

fn latency_summary(samples: Vec<LatencySample>) -> LatencySummary {
    let mut values = samples
        .iter()
        .map(|sample| sample.latency_ms)
        .collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    LatencySummary {
        p50_ms: (!values.is_empty()).then(|| percentile(&values, 50)),
        p95_ms: (!values.is_empty()).then(|| percentile(&values, 95)),
        min_ms: values.first().copied(),
        max_ms: values.last().copied(),
        samples,
    }
}

fn percentile(sorted: &[f64], percentile: usize) -> f64 {
    let index = (sorted.len() * percentile).div_ceil(100).saturating_sub(1);
    sorted[index]
}

fn failure(
    phase: &'static str,
    iteration: usize,
    reason: String,
    report_path: Option<String>,
    child_exit_code: Option<i32>,
) -> FailureReport {
    FailureReport {
        phase,
        iteration,
        reason,
        report_path,
        child_exit_code,
    }
}

fn build_report(runner: &Path) -> BuildReport {
    BuildReport {
        batch_profile: infer_profile(&env::current_exe().unwrap_or_default()),
        runner_profile: infer_profile(runner),
        runner_path: runner.to_string_lossy().into_owned(),
        runner_sha256: file_sha256(runner),
        runner_modified_unix_ms: fs::metadata(runner)
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis()),
        package_version: env!("CARGO_PKG_VERSION"),
    }
}

fn file_sha256(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let digest = sha2::Sha256::digest(bytes);
    Some(format!("{digest:x}"))
}

fn infer_profile(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().to_ascii_lowercase())
        .find(|component| component == "release" || component == "debug")
        .unwrap_or_else(|| "unknown".to_owned())
}

fn unix_timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn write_json(path: &Path, value: &impl serde::Serialize) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::{CopyTrigger, DEFAULT_ITERATIONS, Options, percentile, usage, with_cleanup_error};
    use std::{ffi::OsString, path::PathBuf, time::Duration};

    fn arguments(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn parser_requires_explicit_side_effect_authorization_at_execution_boundary() {
        let options = Options::parse_from(arguments(&[])).unwrap();
        assert!(!options.allow_input);
        assert!(!options.allow_system_clipboard);
        assert_eq!(options.iterations, DEFAULT_ITERATIONS);
        assert_eq!(options.warmup, 2);
    }

    #[test]
    fn parser_accepts_real_batch_contract_and_alias() {
        let options = Options::parse_from(arguments(&[
            "--allow-input",
            "--allow-system-clipboard",
            "--copy-iterations",
            "35",
            "--warmup",
            "3",
            "--runner",
            "target/release/overlay-interaction-acceptance.exe",
            "--output-dir",
            "evidence",
            "--copy-trigger",
            "enter",
            "--timeout-ms",
            "12000",
            "--settle-ms",
            "350",
            "--max-p95-ms",
            "225",
        ]))
        .unwrap();
        assert_eq!(options.iterations, 35);
        assert_eq!(options.warmup, 3);
        assert_eq!(options.copy_trigger, CopyTrigger::Enter);
        assert_eq!(
            options.runner,
            Some(PathBuf::from(
                "target/release/overlay-interaction-acceptance.exe"
            ))
        );
        assert_eq!(options.timeout, Duration::from_secs(12));
        assert_eq!(options.settle_delay, Duration::from_millis(350));
        assert_eq!(options.max_p95_ms, 225.0);

        let alias = Options::parse_from(arguments(&[
            "--allow-input",
            "--allow-system-clipboard",
            "--iterations",
            "30",
        ]))
        .unwrap();
        assert_eq!(alias.iterations, 30);
    }

    #[test]
    fn parser_rejects_short_or_duplicate_batches() {
        assert!(Options::parse_from(arguments(&["--copy-iterations", "29"])).is_err());
        assert!(Options::parse_from(arguments(&["--copy-iterations", "201"])).is_err());
        assert!(
            Options::parse_from(arguments(&[
                "--copy-iterations",
                "30",
                "--iterations",
                "30",
            ]))
            .is_err()
        );
        assert!(Options::parse_from(arguments(&["--warmup", "21"])).is_err());
        assert!(Options::parse_from(arguments(&["--copy-trigger", "space"])).is_err());
        assert_eq!(usage().split_whitespace().next(), Some("usage:"));
    }

    #[test]
    fn percentile_uses_nearest_rank() {
        let values = [10.0, 20.0, 30.0, 40.0];
        assert_eq!(percentile(&values, 50), 20.0);
        assert_eq!(percentile(&values, 95), 40.0);
    }

    #[test]
    fn runner_timeout_reserves_only_the_bounded_report_grace() {
        assert_eq!(
            super::runner_timeout(Duration::from_secs(30)),
            Duration::from_secs(32)
        );
        assert_eq!(
            super::runner_timeout(Duration::MAX),
            Duration::MAX,
            "the outer watchdog must remain finite even for a synthetic maximum input"
        );
    }

    #[test]
    fn cleanup_error_keeps_the_original_kind_and_appends_bounded_reap_context() {
        let original = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "spawn failed");
        let cleanup = std::io::Error::new(std::io::ErrorKind::TimedOut, "child did not reap");
        let combined = with_cleanup_error(original, Some(cleanup));
        assert_eq!(combined.kind(), std::io::ErrorKind::BrokenPipe);
        assert_eq!(
            combined.to_string(),
            "spawn failed; bounded child cleanup also failed: child did not reap"
        );
        let original = std::io::Error::new(std::io::ErrorKind::NotFound, "runner missing");
        let unchanged = with_cleanup_error(original, None);
        assert_eq!(unchanged.kind(), std::io::ErrorKind::NotFound);
        assert_eq!(unchanged.to_string(), "runner missing");
    }
}
