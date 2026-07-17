//! Command-line entry point for repeatable capture resource and latency testing.

use std::{io, path::PathBuf};

use flash_shot::capture_stress::{StressConfig, run};

fn main() {
    match execute() {
        Ok(true) => {}
        Ok(false) => std::process::exit(2),
        Err(error) => {
            eprintln!("capture stress failed: {error}");
            std::process::exit(1);
        }
    }
}

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

fn parse_args(args: impl IntoIterator<Item = String>) -> io::Result<StressConfig> {
    let mut config = StressConfig::default();
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
            "--iterations" => config.iterations = parse_usize(value()?, &argument)?,
            "--output" => config.output = Some(PathBuf::from(value()?)),
            "--max-handle-growth" => config.max_handle_growth = parse_i64(value()?, &argument)?,
            "--max-thread-growth" => config.max_thread_growth = parse_i64(value()?, &argument)?,
            "--max-working-set-growth-mib" => {
                config.max_working_set_growth = parse_i64(value()?, &argument)?
                    .checked_mul(1024 * 1024)
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidInput, "memory limit overflow")
                    })?;
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

fn parse_usize(value: String, argument: &str) -> io::Result<usize> {
    value.parse().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid value for {argument}: {value}"),
        )
    })
}

fn parse_i64(value: String, argument: &str) -> io::Result<i64> {
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
    fn parses_stress_gate_overrides() {
        let config = parse_args([
            "--iterations".to_owned(),
            "12".to_owned(),
            "--output".to_owned(),
            "report.json".to_owned(),
            "--max-working-set-growth-mib".to_owned(),
            "32".to_owned(),
        ])
        .unwrap();

        assert_eq!(config.iterations, 12);
        assert_eq!(
            config.output.unwrap(),
            std::path::PathBuf::from("report.json")
        );
        assert_eq!(config.max_working_set_growth, 32 * 1024 * 1024);
    }
}
