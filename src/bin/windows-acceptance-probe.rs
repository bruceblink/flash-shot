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
    let output = parse_output(args)?;
    let displays = SystemDisplayProvider.displays()?;
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
        "display_count": displays.len(),
        "displays": displays.iter().map(display_json).collect::<Vec<_>>(),
        "ffmpeg": ffmpeg,
    });
    let rendered = serde_json::to_string_pretty(&report).map_err(io::Error::other)?;
    if let Some(path) = output {
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

fn parse_output(args: impl IntoIterator<Item = String>) -> io::Result<Option<PathBuf>> {
    let mut output = None;
    let mut args = args.into_iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--output" => {
                let value = args.next().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "missing value for --output")
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
    use super::{parse_output, rotation_label};
    use flash_shot::platform::display::DisplayRotation;

    #[test]
    fn accepts_an_optional_report_path() {
        assert_eq!(
            parse_output(["--output".to_owned(), "target/environment.json".to_owned(),]).unwrap(),
            Some(std::path::PathBuf::from("target/environment.json"))
        );
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
