//! Emits a machine-readable snapshot of the Windows environment needed by the manual matrix.

use std::{
    io,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use flash_shot::platform::display::{DisplayProvider, DisplayRotation, SystemDisplayProvider};
use serde_json::json;

fn main() {
    if let Err(error) = execute(std::env::args().skip(1)) {
        eprintln!("windows acceptance probe failed: {error}");
        std::process::exit(1);
    }
}

/// Collects display geometry and FFmpeg readiness without opening a capture or recording.
fn execute(args: impl IntoIterator<Item = String>) -> io::Result<()> {
    let options = parse_output(args)?;
    let displays = SystemDisplayProvider.displays()?;
    validate_display_scope(displays.len(), options.require_single_display)?;
    let ffmpeg = match flash_shot::recording::discover() {
        Ok(capabilities) => json!({
            "available": true,
            "executable": capabilities.executable(),
            "version": capabilities.version(),
            "input_formats": capabilities.input_formats(),
            "supports_display_capture": capabilities.supports_display_capture(),
            "supports_window_capture": capabilities.supports_window_capture(),
            "supports_region_capture": capabilities.supports_region_capture(),
        }),
        Err(error) => json!({
            "available": false,
            "error": error.to_string(),
        }),
    };
    let report = json!({
        "schema_version": 1,
        "test": "windows_acceptance_environment",
        "timestamp_unix_ms": unix_timestamp_ms(),
        "single_display_required": options.require_single_display,
        "display_count": displays.len(),
        "displays": displays.iter().map(display_json).collect::<Vec<_>>(),
        "ffmpeg": ffmpeg,
    });
    let rendered = serde_json::to_string_pretty(&report).map_err(io::Error::other)?;
    if let Some(path) = options.output {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, rendered.as_bytes())?;
    }
    println!("{rendered}");
    Ok(())
}

#[derive(Debug, Default, Eq, PartialEq)]
struct ProbeOptions {
    output: Option<PathBuf>,
    require_single_display: bool,
}

/// Parses the report destination and optional single-display scope guard.
fn parse_output(args: impl IntoIterator<Item = String>) -> io::Result<ProbeOptions> {
    let mut options = ProbeOptions::default();
    let mut args = args.into_iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--output" => {
                let value = args.next().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "missing value for --output")
                })?;
                options.output = Some(PathBuf::from(value));
            }
            "--single-display" => {
                options.require_single_display = true;
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

/// Enforces the active acceptance scope before collecting environment evidence.
fn validate_display_scope(display_count: usize, require_single_display: bool) -> io::Result<()> {
    if require_single_display && display_count != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "single-display acceptance requires exactly one display, found {display_count}"
            ),
        ));
    }
    Ok(())
}

/// Converts one native display record into stable JSON fields used by the acceptance checklist.
fn display_json(display: &flash_shot::platform::display::DisplayInfo) -> serde_json::Value {
    json!({
        "id": display.id,
        "physical_bounds": rect_json(display.physical_bounds),
        "work_area": rect_json(display.work_area),
        "dpi": { "x": display.dpi_x, "y": display.dpi_y },
        "scale_factor": display.scale_factor,
        "rotation": rotation_label(display.rotation),
        "bits_per_pixel": display.bits_per_pixel,
        "primary": display.primary,
    })
}

fn rect_json(rect: flash_shot::domain::geometry::PhysicalRect) -> serde_json::Value {
    json!({
        "left": rect.left,
        "top": rect.top,
        "right": rect.right,
        "bottom": rect.bottom,
        "width": rect.width(),
        "height": rect.height(),
    })
}

fn rotation_label(rotation: DisplayRotation) -> &'static str {
    match rotation {
        DisplayRotation::Landscape => "landscape",
        DisplayRotation::Portrait => "portrait",
        DisplayRotation::LandscapeFlipped => "landscape_flipped",
        DisplayRotation::PortraitFlipped => "portrait_flipped",
        DisplayRotation::Unknown => "unknown",
    }
}

fn unix_timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::{ProbeOptions, parse_output, rotation_label, validate_display_scope};
    use flash_shot::platform::display::DisplayRotation;

    #[test]
    fn accepts_an_optional_report_path() {
        assert_eq!(
            parse_output(["--output".to_owned(), "target/environment.json".to_owned(),]).unwrap(),
            ProbeOptions {
                output: Some(std::path::PathBuf::from("target/environment.json")),
                require_single_display: false,
            }
        );
    }

    #[test]
    fn accepts_the_single_display_scope_guard() {
        assert_eq!(
            parse_output(["--single-display".to_owned()]).unwrap(),
            ProbeOptions {
                output: None,
                require_single_display: true,
            }
        );
    }

    #[test]
    fn single_display_scope_rejects_missing_or_extra_displays() {
        assert!(validate_display_scope(1, true).is_ok());
        assert!(validate_display_scope(0, true).is_err());
        assert!(validate_display_scope(2, true).is_err());
        assert!(validate_display_scope(2, false).is_ok());
    }

    #[test]
    fn rejects_unknown_probe_options() {
        assert!(parse_output(["--wat".to_owned()]).is_err());
    }

    #[test]
    fn rotation_labels_are_stable_for_acceptance_reports() {
        assert_eq!(rotation_label(DisplayRotation::Landscape), "landscape");
        assert_eq!(
            rotation_label(DisplayRotation::PortraitFlipped),
            "portrait_flipped"
        );
    }
}
