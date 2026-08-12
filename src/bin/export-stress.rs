//! CLI entry point for repeatable full-frame long-image export preparation measurements.

use std::{io, path::PathBuf};

use flash_shot::export_stress::{ExportStressConfig, run};

fn main() {
    match execute() {
        Ok(true) => {}
        Ok(false) => std::process::exit(2),
        Err(error) => {
            eprintln!("export stress failed: {error}");
            std::process::exit(1);
        }
    }
}

/// Runs the benchmark, prints its JSON, optionally writes it to disk, and returns its gate result.
fn execute() -> io::Result<bool> {
    let config = parse_args(std::env::args().skip(1))?;
    let report = run(&config)?;
    println!("{}", report.to_pretty_json()?);
    if let Some(path) = &config.output {
        report.write(path)?;
    }
    Ok(report.passed())
}

/// Parses explicit benchmark sizes and gates without hiding platform-specific defaults.
fn parse_args(args: impl IntoIterator<Item = String>) -> io::Result<ExportStressConfig> {
    let mut config = ExportStressConfig::default();
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
            "--output" => config.output = Some(PathBuf::from(value()?)),
            "--iterations" => config.iterations = parse_value(value()?, &argument)?,
            "--width" => config.width = parse_value(value()?, &argument)?,
            "--height" => config.height = parse_value(value()?, &argument)?,
            "--max-p95-ms" => config.max_p95_ms = Some(parse_value(value()?, &argument)?),
            "--max-additional-copies" => {
                config.max_additional_copies = parse_value(value()?, &argument)?
            }
            "--require-pixel-reuse" => config.require_pixel_reuse = true,
            "--no-latency-gate" => config.max_p95_ms = None,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unknown argument: {argument}"),
                ));
            }
        }
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
    fn parses_dimensions_output_and_export_gates() {
        let config = parse_args([
            "--output".to_owned(),
            "target/export-stress.json".to_owned(),
            "--iterations".to_owned(),
            "5".to_owned(),
            "--width".to_owned(),
            "320".to_owned(),
            "--height".to_owned(),
            "900".to_owned(),
            "--max-p95-ms".to_owned(),
            "80".to_owned(),
            "--max-additional-copies".to_owned(),
            "1".to_owned(),
            "--require-pixel-reuse".to_owned(),
        ])
        .unwrap();

        assert_eq!(
            config.output.unwrap(),
            std::path::PathBuf::from("target/export-stress.json")
        );
        assert_eq!(config.iterations, 5);
        assert_eq!(config.width, 320);
        assert_eq!(config.height, 900);
        assert_eq!(config.max_p95_ms, Some(80));
        assert_eq!(config.max_additional_copies, 1);
        assert!(config.require_pixel_reuse);
    }

    #[test]
    fn no_latency_gate_clears_an_explicit_limit() {
        let config = parse_args([
            "--max-p95-ms".to_owned(),
            "1".to_owned(),
            "--no-latency-gate".to_owned(),
        ])
        .unwrap();

        assert_eq!(config.max_p95_ms, None);
    }
}
