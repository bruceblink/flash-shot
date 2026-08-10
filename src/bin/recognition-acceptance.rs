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

/// Records whether an explicitly requested OCR fixture produced any text without retaining its
/// contents. The report is safe to commit because it exposes only bounded metadata.
#[derive(Serialize)]
struct OcrExercise {
    passed: bool,
    text_length: Option<usize>,
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
    ocr_image: Option<PathBuf>,
}

#[derive(Serialize)]
struct AcceptanceReport {
    schema_version: u32,
    test: &'static str,
    timestamp_unix_ms: u128,
    ocr: OcrReadiness,
    ocr_exercise: Option<OcrExercise>,
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

/// Probes optional dependencies without capturing the screen or making a translation request.
/// An explicit `--ocr-image` exercises the full PNG-to-Tesseract path against a supplied fixture.
fn execute(args: impl IntoIterator<Item = String>) -> io::Result<()> {
    let options = parse_options(args)?;
    let ocr = probe_ocr();
    let ocr_exercise = options.ocr_image.as_deref().map(exercise_ocr);
    let translation = probe_translation();
    let passed = (!options.require_ocr || ocr.available)
        && (!options.require_translation || translation.configured)
        && ocr_exercise.as_ref().is_none_or(|exercise| exercise.passed);
    let report = AcceptanceReport {
        schema_version: 3,
        test: "recognition_readiness",
        timestamp_unix_ms: unix_timestamp_ms(),
        ocr,
        ocr_exercise,
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
        if report
            .ocr_exercise
            .as_ref()
            .is_some_and(|exercise| !exercise.passed)
        {
            missing.push("OCR fixture");
        }
        Err(io::Error::other(format!(
            "required recognition dependencies are not ready: {}",
            missing.join(", ")
        )))
    }
}

/// Runs the same PNG-to-Tesseract path used by the app and keeps the report content-free.
fn exercise_ocr(path: &std::path::Path) -> OcrExercise {
    let result = flash_shot::platform::capture::CaptureFrame::open_png(path)
        .and_then(|frame| flash_shot::ocr::recognize_with_language(&frame, None));
    match result {
        Ok(text) if !text.trim().is_empty() => OcrExercise {
            passed: true,
            text_length: Some(text.trim().chars().count()),
            error: None,
        },
        Ok(_) => OcrExercise {
            passed: false,
            text_length: Some(0),
            error: Some("OCR returned no text".to_owned()),
        },
        Err(error) => OcrExercise {
            passed: false,
            text_length: None,
            error: Some(error.to_string()),
        },
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
            "--ocr-image" => {
                if options.ocr_image.is_some() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "--ocr-image may only be provided once",
                    ));
                }
                let value = arguments.next().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "--ocr-image requires a path")
                })?;
                options.ocr_image = Some(PathBuf::from(value));
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
                ocr_image: None,
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
        assert_eq!(
            parse_options(["--ocr-image".to_owned(), "fixture.png".to_owned()].into_iter())
                .unwrap()
                .ocr_image,
            Some(PathBuf::from("fixture.png"))
        );
        assert!(
            parse_options(
                [
                    "--ocr-image".to_owned(),
                    "one.png".to_owned(),
                    "--ocr-image".to_owned(),
                    "two.png".to_owned(),
                ]
                .into_iter()
            )
            .is_err()
        );
    }
}
