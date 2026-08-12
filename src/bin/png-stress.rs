//! CLI entry point for repeatable long-image PNG encoding measurements.

use std::{io, path::PathBuf};

use flash_shot::png_stress::{PngStressConfig, run};

fn main() {
    match execute() {
        Ok(true) => {}
        Ok(false) => std::process::exit(2),
        Err(error) => {
            eprintln!("PNG stress failed: {error}");
            std::process::exit(1);
        }
    }
}

fn execute() -> io::Result<bool> {
    let config = parse_args(std::env::args().skip(1))?;
    let report = run(&config)?;
    println!("{}", report.to_pretty_json()?);
    if let Some(path) = &config.output {
        report.write(path)?;
    }
    Ok(report.passed())
}

/// Parses explicit benchmark dimensions and an optional fixed-machine p95 gate.
fn parse_args(args: impl IntoIterator<Item = String>) -> io::Result<PngStressConfig> {
    let mut config = PngStressConfig::default();
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
            "--no-gate" => config.max_p95_ms = None,
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
    fn parses_dimensions_output_and_latency_gate() {
        let config = parse_args([
            "--output".to_owned(),
            "target/png-stress.json".to_owned(),
            "--iterations".to_owned(),
            "5".to_owned(),
            "--width".to_owned(),
            "320".to_owned(),
            "--height".to_owned(),
            "900".to_owned(),
            "--max-p95-ms".to_owned(),
            "80".to_owned(),
        ])
        .unwrap();

        assert_eq!(
            config.output.unwrap(),
            std::path::PathBuf::from("target/png-stress.json")
        );
        assert_eq!(config.iterations, 5);
        assert_eq!(config.width, 320);
        assert_eq!(config.height, 900);
        assert_eq!(config.max_p95_ms, Some(80));
    }

    #[test]
    fn no_gate_clears_an_explicit_limit() {
        let config = parse_args([
            "--max-p95-ms".to_owned(),
            "1".to_owned(),
            "--no-gate".to_owned(),
        ])
        .unwrap();

        assert_eq!(config.max_p95_ms, None);
    }
}
