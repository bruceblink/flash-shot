//! Read-only acceptance probe for the optional OCR and translation integrations.

use std::{
    io,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;

#[derive(Serialize)]
struct OcrReadiness {
    available: bool,
    version: Option<String>,
    language: Option<String>,
    language_available: Option<bool>,
    error: Option<String>,
}

#[derive(Serialize)]
struct TranslationReadiness {
    configured: bool,
    target_language: Option<String>,
    token_configured: bool,
    error: Option<String>,
}

#[derive(Serialize)]
struct AcceptanceReport {
    schema_version: u32,
    test: &'static str,
    timestamp_unix_ms: u128,
    ocr: OcrReadiness,
    translation: TranslationReadiness,
    passed: bool,
}

fn main() {
    if let Err(error) = execute(std::env::args().skip(1)) {
        eprintln!("recognition acceptance failed: {error}");
        std::process::exit(1);
    }
}

/// Probes optional dependencies without creating screenshots or making a translation request.
fn execute(args: impl IntoIterator<Item = String>) -> io::Result<()> {
    let output = parse_output(args)?;
    let report = AcceptanceReport {
        schema_version: 1,
        test: "recognition_readiness",
        timestamp_unix_ms: unix_timestamp_ms(),
        ocr: probe_ocr(),
        translation: probe_translation(),
        passed: true,
    };
    let encoded = serde_json::to_vec_pretty(&report).map_err(io::Error::other)?;
    println!("{}", String::from_utf8_lossy(&encoded));
    if let Some(output) = output {
        if let Some(parent) = output
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(output, encoded)?;
    }
    Ok(())
}

/// Converts an OCR support result into a report that never exposes a local executable path.
fn probe_ocr() -> OcrReadiness {
    match flash_shot::ocr::check_support(None) {
        Ok(support) => OcrReadiness {
            available: support.language_available(),
            version: Some(support.version().to_owned()),
            language: Some(support.language().to_owned()),
            language_available: Some(support.language_available()),
            error: None,
        },
        Err(error) => OcrReadiness {
            available: false,
            version: None,
            language: None,
            language_available: None,
            error: Some(error.to_string()),
        },
    }
}

/// Reports translation configuration only; the probe deliberately performs no network I/O.
fn probe_translation() -> TranslationReadiness {
    match flash_shot::translation::TranslationConfig::from_environment() {
        Ok(Some(config)) => TranslationReadiness {
            configured: true,
            target_language: Some(config.target_language().to_owned()),
            token_configured: config.token().is_some(),
            error: None,
        },
        Ok(None) => TranslationReadiness {
            configured: false,
            target_language: None,
            token_configured: false,
            error: None,
        },
        Err(error) => TranslationReadiness {
            configured: false,
            target_language: None,
            token_configured: false,
            error: Some(error.to_string()),
        },
    }
}

fn parse_output(args: impl IntoIterator<Item = String>) -> io::Result<Option<PathBuf>> {
    let mut output = None;
    let mut arguments = args.into_iter();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--output" => {
                if output.is_some() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "--output may only be provided once",
                    ));
                }
                let value = arguments.next().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "--output requires a path")
                })?;
                output = Some(PathBuf::from(value));
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unknown argument: {argument}"),
                ));
            }
        }
    }
    Ok(output)
}

fn unix_timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::parse_output;
    use std::path::PathBuf;

    #[test]
    fn output_argument_is_optional_and_bounded() {
        assert!(parse_output(std::iter::empty()).unwrap().is_none());
        assert_eq!(
            parse_output(["--output".to_owned(), "report.json".to_owned()].into_iter()).unwrap(),
            Some(PathBuf::from("report.json"))
        );
        assert!(parse_output(["--output".to_owned()].into_iter()).is_err());
        assert!(parse_output(["--unknown".to_owned()].into_iter()).is_err());
        assert!(
            parse_output(
                [
                    "--output".to_owned(),
                    "one.json".to_owned(),
                    "--output".to_owned(),
                    "two.json".to_owned(),
                ]
                .into_iter()
            )
            .is_err()
        );
    }
}
