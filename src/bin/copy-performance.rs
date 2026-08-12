//! Opt-in CLI for the ordinary selection path's synthetic system-clipboard p95 gate.

use std::{io, path::PathBuf};

use flash_shot::copy_performance::{CopyPerformanceConfig, run};

fn main() {
    match execute() {
        Ok(true) => {}
        Ok(false) => std::process::exit(2),
        Err(error) => {
            eprintln!("copy performance failed: {error}");
            std::process::exit(1);
        }
    }
}

/// Runs the gated measurement only after parsing the explicit clipboard-mutation permission.
fn execute() -> io::Result<bool> {
    let config = parse_args(std::env::args().skip(1))?;
    let report = run(&config)?;
    let json = report.to_pretty_json()?;
    println!("{json}");
    if let Some(path) = &config.output {
        report.write(path)?;
    }
    Ok(report.passed())
}

/// Parses benchmark shape and gates without implicitly granting access to the system clipboard.
fn parse_args(args: impl IntoIterator<Item = String>) -> io::Result<CopyPerformanceConfig> {
    let mut config = CopyPerformanceConfig::default();
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
            "--allow-system-clipboard" if !config.allow_system_clipboard => {
                config.allow_system_clipboard = true
            }
            "--allow-system-clipboard" => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "--allow-system-clipboard may only be supplied once",
                ));
            }
            "--iterations" => config.iterations = parse_value(value()?, &argument)?,
            "--width" => config.width = parse_value(value()?, &argument)?,
            "--height" => config.height = parse_value(value()?, &argument)?,
            "--max-p95-ms" => config.max_p95_ms = Some(parse_value(value()?, &argument)?),
            "--no-gate" => config.max_p95_ms = None,
            "--output" => config.output = Some(PathBuf::from(value()?)),
            "--metrics-dir" => config.metrics_directory = Some(PathBuf::from(value()?)),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unknown argument: {argument}"),
                ));
            }
        }
    }
    if !config.allow_system_clipboard {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "--allow-system-clipboard is required because this benchmark replaces clipboard contents",
        ));
    }
    Ok(config)
}

fn parse_value<T>(value: String, argument: &str) -> io::Result<T>
where
    T: std::str::FromStr,
{
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
    fn parses_explicit_system_clipboard_measurement() {
        let config = parse_args([
            "--allow-system-clipboard".to_owned(),
            "--iterations".to_owned(),
            "25".to_owned(),
            "--width".to_owned(),
            "800".to_owned(),
            "--height".to_owned(),
            "600".to_owned(),
            "--max-p95-ms".to_owned(),
            "225".to_owned(),
            "--output".to_owned(),
            "target/copy-performance.json".to_owned(),
            "--metrics-dir".to_owned(),
            "target/copy-performance-metrics".to_owned(),
        ])
        .unwrap();

        assert!(config.allow_system_clipboard);
        assert_eq!(config.iterations, 25);
        assert_eq!((config.width, config.height), (800, 600));
        assert_eq!(config.max_p95_ms, Some(225));
        assert_eq!(
            config.output.unwrap(),
            std::path::PathBuf::from("target/copy-performance.json")
        );
        assert_eq!(
            config.metrics_directory.unwrap(),
            std::path::PathBuf::from("target/copy-performance-metrics")
        );
    }

    #[test]
    fn refuses_to_mutate_the_clipboard_without_explicit_permission() {
        let error = parse_args(["--iterations".to_owned(), "20".to_owned()]).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("--allow-system-clipboard"));
    }

    #[test]
    fn no_gate_clears_an_explicit_threshold() {
        let config = parse_args([
            "--allow-system-clipboard".to_owned(),
            "--max-p95-ms".to_owned(),
            "1".to_owned(),
            "--no-gate".to_owned(),
        ])
        .unwrap();

        assert_eq!(config.max_p95_ms, None);
    }

    #[test]
    fn rejects_duplicate_permission_and_unknown_options() {
        let duplicate = parse_args([
            "--allow-system-clipboard".to_owned(),
            "--allow-system-clipboard".to_owned(),
        ])
        .unwrap_err();
        assert!(duplicate.to_string().contains("only be supplied once"));

        let unknown =
            parse_args(["--allow-system-clipboard".to_owned(), "--wat".to_owned()]).unwrap_err();
        assert!(unknown.to_string().contains("unknown argument"));
    }
}
