//! Isolated FFmpeg discovery and capture capability probing.
//!
//! This module deliberately owns only executable discovery and read-only probing. Recording
//! sessions will build on these stable data types without leaking process details into the UI.

use std::{
    env,
    ffi::{OsStr, OsString},
    io,
    path::PathBuf,
    process::{Command, Output},
};

const FFMPEG_PATH_ENV: &str = "FLASH_SHOT_FFMPEG";
const VERSION_ARGUMENTS: &[&str] = &["-hide_banner", "-version"];
const FORMAT_ARGUMENTS: &[&str] = &["-hide_banner", "-formats"];
const DEVICE_ARGUMENTS: &[&str] = &["-hide_banner", "-devices"];

/// Read-only capabilities exposed by an installed FFmpeg executable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FfmpegCapabilities {
    executable: PathBuf,
    version: String,
    input_formats: Vec<String>,
}

impl FfmpegCapabilities {
    pub fn executable(&self) -> &std::path::Path {
        &self.executable
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn input_formats(&self) -> &[String] {
        &self.input_formats
    }

    /// Desktop Duplication is preferred; GDI capture is a compatible Windows fallback.
    pub fn supports_display_capture(&self) -> bool {
        self.supports_input("ddagrab") || self.supports_input("gdigrab")
    }

    /// A window is captured by a Windows screen input selected by title or bounds.
    pub fn supports_window_capture(&self) -> bool {
        self.supports_input("gdigrab")
    }

    pub fn supports_region_capture(&self) -> bool {
        self.supports_display_capture()
    }

    pub fn supports_microphone_capture(&self) -> bool {
        self.supports_input("dshow")
    }

    pub fn supports_system_audio_capture(&self) -> bool {
        self.supports_input("wasapi") || self.supports_input("dshow")
    }

    pub fn supports_input(&self, name: &str) -> bool {
        self.input_formats.iter().any(|input| input == name)
    }
}

/// Locates FFmpeg from an explicit environment override or the process PATH, then probes it.
pub fn discover() -> io::Result<FfmpegCapabilities> {
    let executable = executable_from(env::var_os(FFMPEG_PATH_ENV));
    let version_output = run_probe(&executable, VERSION_ARGUMENTS)?;
    let format_output = run_probe(&executable, FORMAT_ARGUMENTS)?;
    let device_output = run_probe(&executable, DEVICE_ARGUMENTS)?;

    let version = parse_version(&combined_output(&version_output)).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "FFmpeg did not report a recognizable version",
        )
    })?;
    let mut input_formats = parse_input_formats(&combined_output(&format_output));
    for device in parse_input_formats(&combined_output(&device_output)) {
        if !input_formats.contains(&device) {
            input_formats.push(device);
        }
    }
    input_formats.sort_unstable();

    Ok(FfmpegCapabilities {
        executable: PathBuf::from(executable),
        version,
        input_formats,
    })
}

fn executable_from(configured: Option<OsString>) -> OsString {
    configured
        .filter(|path| !path.is_empty())
        .unwrap_or_else(|| OsString::from("ffmpeg"))
}

fn run_probe(executable: &OsStr, arguments: &[&str]) -> io::Result<Output> {
    let output = Command::new(executable)
        .args(arguments)
        .output()
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "could not start FFmpeg '{}': {error}",
                    executable.to_string_lossy()
                ),
            )
        })?;
    if output.status.success() {
        return Ok(output);
    }

    Err(io::Error::other(format!(
        "FFmpeg probe {} exited with {}{}",
        arguments.join(" "),
        output.status,
        first_diagnostic_line(&combined_output(&output))
            .map(|line| format!(": {line}"))
            .unwrap_or_default(),
    )))
}

fn combined_output(output: &Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text
}

fn parse_version(output: &str) -> Option<String> {
    output
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("ffmpeg version "))
        .map(str::to_owned)
        .filter(|version| !version.is_empty())
}

fn parse_input_formats(output: &str) -> Vec<String> {
    let mut inputs = Vec::new();
    for line in output.lines() {
        let mut fields = line.split_whitespace();
        let Some(flags) = fields.next() else {
            continue;
        };
        if !flags.contains('D') {
            continue;
        }
        let Some(name) = fields.next() else {
            continue;
        };
        for input in name
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            let input = input.to_ascii_lowercase();
            if !inputs.contains(&input) {
                inputs.push(input);
            }
        }
    }
    inputs
}

fn first_diagnostic_line(output: &str) -> Option<&str> {
    output.lines().map(str::trim).find(|line| !line.is_empty())
}

#[cfg(test)]
mod tests {
    use super::{
        DEVICE_ARGUMENTS, FORMAT_ARGUMENTS, FfmpegCapabilities, VERSION_ARGUMENTS, executable_from,
        first_diagnostic_line, parse_input_formats, parse_version,
    };
    use std::{ffi::OsString, path::PathBuf};

    const FORMATS: &str = "\
 File formats:\n\
  D  ddagrab          Windows Desktop Duplication API\n\
  D  gdigrab          GDI API Windows frame grabber\n\
  D  dshow            DirectShow capture\n\
 DE png_pipe          PNG pipe\n\
";

    #[test]
    fn probe_arguments_are_read_only_and_hide_banner_noise() {
        assert_eq!(VERSION_ARGUMENTS, ["-hide_banner", "-version"]);
        assert_eq!(FORMAT_ARGUMENTS, ["-hide_banner", "-formats"]);
        assert_eq!(DEVICE_ARGUMENTS, ["-hide_banner", "-devices"]);
    }

    #[test]
    fn configured_executable_overrides_path_lookup() {
        assert_eq!(
            executable_from(Some(OsString::from(r"C:\\tools\\ffmpeg.exe"))),
            OsString::from(r"C:\\tools\\ffmpeg.exe")
        );
        assert_eq!(
            executable_from(Some(OsString::new())),
            OsString::from("ffmpeg")
        );
    }

    #[test]
    fn parser_keeps_only_demotion_input_formats_and_deduplicates() {
        assert_eq!(
            parse_input_formats(FORMATS),
            ["ddagrab", "gdigrab", "dshow", "png_pipe"]
        );
    }

    #[test]
    fn version_and_diagnostics_are_bounded_to_useful_output() {
        assert_eq!(
            parse_version("ffmpeg version 7.1-full_build Copyright"),
            Some("7.1-full_build Copyright".to_owned())
        );
        assert_eq!(
            first_diagnostic_line("\n  access denied\ntrace"),
            Some("access denied")
        );
    }

    #[test]
    fn windows_capture_capabilities_are_derived_from_detected_inputs() {
        let capabilities = FfmpegCapabilities {
            executable: PathBuf::from("ffmpeg"),
            version: "7.1".to_owned(),
            input_formats: parse_input_formats(FORMATS),
        };

        assert!(capabilities.supports_display_capture());
        assert!(capabilities.supports_window_capture());
        assert!(capabilities.supports_region_capture());
        assert!(capabilities.supports_microphone_capture());
        assert!(capabilities.supports_system_audio_capture());
        assert!(!capabilities.supports_input("avfoundation"));
    }
}
