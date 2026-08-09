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

/// Describes optional readiness requirements without making the default probe network-bound.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct ProbeOptions {
    output: Option<PathBuf>,
    require_ocr: bool,
    require_translation: bool,
}

#[derive(Serialize)]
struct AcceptanceReport {
    schema_version: u32,
    test: &'static str,
    timestamp_unix_ms: u128,
    ocr: OcrReadiness,
    translation: TranslationReadiness,
    require_ocr: bool,
    require_translation: bool,
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
    let options = parse_options(args)?;
    let ocr = probe_ocr();
    let translation = probe_translation();
    let passed = (!options.require_ocr || ocr.available)
        && (!options.require_translation || translation.configured);
    let report = AcceptanceReport {
        schema_version: 2,
        test: "recognition_readiness",
        timestamp_unix_ms: unix_timestamp_ms(),
        ocr,
        translation,
        require_ocr: options.require_ocr,
        require_translation: options.require_translation,
        passed,
    };
    let encoded = serde_json::to_vec_pretty(&report).map_err(io::Error::other)?;
    println!("{}", String::from_utf8_lossy(&encoded));
    if let Some(output) = options.output {
        if let Some(parent) = output
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(output, encoded)?;
    }
    if passed {
        Ok(())
    } else {
        let mut missing = Vec::new();
        if options.require_ocr && !report.ocr.available {
            missing.push("OCR");
        }
        if options.require_translation && !report.translation.configured {
            missing.push("translation");
        }
        Err(io::Error::other(format!(
            "required recognition dependencies are not ready: {}",
            missing.join(", ")
        )))
    }
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

/// Parses output and optional readiness gates while rejecting duplicate flags early.
fn parse_options(args: impl IntoIterator<Item = String>) -> io::Result<ProbeOptions> {
    let mut options = ProbeOptions::default();
    let mut arguments = args.into_iter();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--output" => {
                if options.output.is_some() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "--output may only be provided once",
                    ));
                }
                let value = arguments.next().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "--output requires a path")
                })?;
                options.output = Some(PathBuf::from(value));
            }
            "--require-ocr" => {
                if options.require_ocr {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "--require-ocr may only be provided once",
                    ));
                }
                options.require_ocr = true;
            }
            "--require-translation" => {
                if options.require_translation {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "--require-translation may only be provided once",
                    ));
                }
                options.require_translation = true;
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unknown argument: {argument}"),
                ));
            }
        }
    }
    Ok(options)
}

fn unix_timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::{ProbeOptions, parse_options};
    use std::path::PathBuf;

    #[test]
    fn readiness_requirements_and_output_argument_are_bounded() {
        assert_eq!(
            parse_options(std::iter::empty()).unwrap(),
            ProbeOptions::default()
        );
        assert_eq!(
            parse_options(
                [
                    "--output".to_owned(),
                    "report.json".to_owned(),
                    "--require-ocr".to_owned(),
                    "--require-translation".to_owned(),
                ]
                .into_iter()
            )
            .unwrap(),
            ProbeOptions {
                output: Some(PathBuf::from("report.json")),
                require_ocr: true,
                require_translation: true,
            }
        );
        assert!(parse_options(["--output".to_owned()].into_iter()).is_err());
        assert!(parse_options(["--unknown".to_owned()].into_iter()).is_err());
        assert!(
            parse_options(
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
        assert!(
            parse_options(["--require-ocr".to_owned(), "--require-ocr".to_owned()].into_iter())
                .is_err()
        );
        assert!(
            parse_options(
                [
                    "--require-translation".to_owned(),
                    "--require-translation".to_owned(),
                ]
                .into_iter()
            )
            .is_err()
        );
    }
}
