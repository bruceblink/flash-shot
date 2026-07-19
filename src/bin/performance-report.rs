//! Summarizes recorded Flash Shot performance samples and applies optional p95 gates.

use std::{io, path::PathBuf};

use flash_shot::performance_report::{PerformanceThresholds, summarize_file};

const PERFORMANCE_REPORT_PROTOCOL_VERSION: &str = "performance-report-v3";

fn main() {
    match execute() {
        Ok(true) => {}
        Ok(false) => std::process::exit(2),
        Err(error) => {
            eprintln!("performance report failed: {error}");
            std::process::exit(1);
        }
    }
}

fn execute() -> io::Result<bool> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() == 1 && args[0] == "--protocol-version" {
        println!("{PERFORMANCE_REPORT_PROTOCOL_VERSION}");
        return Ok(true);
    }
    let (input, thresholds, output) = parse_args(args)?;
    let report = summarize_file(input, &thresholds)?;
    let json = report.to_pretty_json()?;
    println!("{json}");
    if let Some(output) = output {
        std::fs::write(output, json)?;
    }
    Ok(report.passed)
}

fn parse_args(
    args: impl IntoIterator<Item = String>,
) -> io::Result<(PathBuf, PerformanceThresholds, Option<PathBuf>)> {
    let mut input = None;
    let mut output = None;
    let mut thresholds = PerformanceThresholds::default();
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
            "--input" => input = Some(PathBuf::from(value()?)),
            "--output" => output = Some(PathBuf::from(value()?)),
            "--max-startup-p95-ms" => {
                thresholds.startup_p95_ms = Some(parse_u64(value()?, &argument)?)
            }
            "--max-frame-ready-p95-ms" => {
                thresholds.shortcut_to_frame_ready_p95_ms = Some(parse_u64(value()?, &argument)?)
            }
            "--max-overlay-p95-ms" => {
                thresholds.shortcut_to_overlay_frame_p95_ms = Some(parse_u64(value()?, &argument)?)
            }
            "--minimum-samples" => thresholds.minimum_samples = parse_usize(value()?, &argument)?,
            "--since-ms" => thresholds.since_timestamp_ms = Some(parse_u128(value()?, &argument)?),
            "--include-nonrelease" => thresholds.require_release_profile = false,
            "--startup-only" => {
                thresholds.shortcut_to_frame_ready_p95_ms = None;
                thresholds.shortcut_to_overlay_frame_p95_ms = None;
            }
            "--capture-only" => thresholds.startup_p95_ms = None,
            "--no-gate" => {
                thresholds = PerformanceThresholds {
                    minimum_samples: 0,
                    since_timestamp_ms: thresholds.since_timestamp_ms,
                    require_release_profile: thresholds.require_release_profile,
                    startup_p95_ms: None,
                    shortcut_to_frame_ready_p95_ms: None,
                    shortcut_to_overlay_frame_p95_ms: None,
                }
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unknown argument: {argument}"),
                ));
            }
        }
    }
    let input =
        input.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "--input is required"))?;
    Ok((input, thresholds, output))
}

fn parse_u64(value: String, argument: &str) -> io::Result<u64> {
    value.parse().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid value for {argument}: {value}"),
        )
    })
}

fn parse_usize(value: String, argument: &str) -> io::Result<usize> {
    value.parse().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid value for {argument}: {value}"),
        )
    })
}

fn parse_u128(value: String, argument: &str) -> io::Result<u128> {
    value.parse().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid value for {argument}: {value}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::parse_args;

    #[test]
    fn parses_explicit_performance_gates() {
        let (input, thresholds, output) = parse_args([
            "--input".to_owned(),
            "metrics.jsonl".to_owned(),
            "--max-overlay-p95-ms".to_owned(),
            "125".to_owned(),
            "--output".to_owned(),
            "summary.json".to_owned(),
        ])
        .unwrap();

        assert_eq!(input, std::path::PathBuf::from("metrics.jsonl"));
        assert_eq!(thresholds.shortcut_to_overlay_frame_p95_ms, Some(125));
        assert_eq!(thresholds.minimum_samples, 10);
        assert_eq!(output, Some(std::path::PathBuf::from("summary.json")));
    }

    #[test]
    fn rejects_an_unknown_option() {
        let error = parse_args(["--wat".to_owned()]).unwrap_err();
        assert!(error.to_string().contains("unknown argument"));
    }

    #[test]
    fn includes_nonrelease_samples_only_when_requested() {
        let (_, thresholds, _) = parse_args([
            "--input".to_owned(),
            "metrics.jsonl".to_owned(),
            "--include-nonrelease".to_owned(),
        ])
        .unwrap();
        assert!(!thresholds.require_release_profile);
    }

    #[test]
    fn startup_only_keeps_the_release_startup_gate() {
        let (_, thresholds, _) = parse_args([
            "--input".to_owned(),
            "metrics.jsonl".to_owned(),
            "--startup-only".to_owned(),
        ])
        .unwrap();
        assert_eq!(thresholds.startup_p95_ms, Some(500));
        assert_eq!(thresholds.shortcut_to_frame_ready_p95_ms, None);
        assert_eq!(thresholds.shortcut_to_overlay_frame_p95_ms, None);
        assert!(thresholds.require_release_profile);
    }

    #[test]
    fn capture_only_keeps_the_release_capture_gates() {
        let (_, thresholds, _) = parse_args([
            "--input".to_owned(),
            "metrics.jsonl".to_owned(),
            "--capture-only".to_owned(),
        ])
        .unwrap();
        assert_eq!(thresholds.startup_p95_ms, None);
        assert_eq!(thresholds.shortcut_to_frame_ready_p95_ms, Some(100));
        assert_eq!(thresholds.shortcut_to_overlay_frame_p95_ms, Some(100));
        assert!(thresholds.require_release_profile);
    }

    #[test]
    fn protocol_version_is_stable_for_measurement_scripts() {
        assert_eq!(
            super::PERFORMANCE_REPORT_PROTOCOL_VERSION,
            "performance-report-v3"
        );
    }
}
