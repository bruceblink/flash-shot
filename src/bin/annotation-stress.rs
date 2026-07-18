//! CLI entry point for repeatable 4K annotation composite measurements.

use std::io;

use flash_shot::annotation_stress::{AnnotationStressConfig, run};

fn main() {
    match execute() {
        Ok(true) => {}
        Ok(false) => std::process::exit(2),
        Err(error) => {
            eprintln!("annotation stress failed: {error}");
            std::process::exit(1);
        }
    }
}

fn execute() -> io::Result<bool> {
    let config = parse_args(std::env::args().skip(1))?;
    let report = run(config)?;
    println!("{}", report.to_pretty_json()?);
    Ok(report.passed())
}

fn parse_args(args: impl IntoIterator<Item = String>) -> io::Result<AnnotationStressConfig> {
    let mut config = AnnotationStressConfig::default();
    let mut args = args.into_iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--no-gate" => config.max_p95_ms = None,
            "--iterations" | "--max-p95-ms" => {
                let value = args.next().ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("missing value for {argument}"),
                    )
                })?;
                if argument == "--iterations" {
                    config.iterations = value.parse().map_err(|_| {
                        io::Error::new(io::ErrorKind::InvalidInput, "invalid iteration count")
                    })?;
                } else {
                    config.max_p95_ms = Some(value.parse().map_err(|_| {
                        io::Error::new(io::ErrorKind::InvalidInput, "invalid p95 limit")
                    })?);
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
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::parse_args;

    #[test]
    fn parses_an_optional_p95_gate() {
        let config = parse_args([
            "--iterations".to_owned(),
            "5".to_owned(),
            "--max-p95-ms".to_owned(),
            "80".to_owned(),
        ])
        .unwrap();
        assert_eq!(config.iterations, 5);
        assert_eq!(config.max_p95_ms, Some(80));

        let config = parse_args(["--no-gate".to_owned()]).unwrap();
        assert_eq!(config.max_p95_ms, None);
    }
}
