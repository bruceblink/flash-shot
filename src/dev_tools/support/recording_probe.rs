//! Shared FFprobe validation used by native recording acceptance modules.

use std::{
    ffi::OsString,
    io,
    path::Path,
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde::Deserialize;

pub(crate) const FFPROBE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Deserialize)]
struct ProbeOutput {
    #[serde(default)]
    streams: Vec<ProbeStream>,
    format: Option<ProbeFormat>,
}

#[derive(Debug, Deserialize)]
struct ProbeStream {
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct ProbeFormat {
    duration: Option<String>,
}

#[derive(Debug, PartialEq)]
pub(crate) struct MediaMetadata {
    pub(crate) codec_name: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) duration_seconds: f64,
}

/// Uses the FFprobe installed beside FFmpeg to prove a finalized recording's media metadata.
pub(crate) fn probe_media(ffmpeg: &Path, output: &Path) -> io::Result<MediaMetadata> {
    let ffprobe = ffmpeg.with_file_name(if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    });
    let result = run_ffprobe(&ffprobe, output)?;
    if !result.status.success() {
        return Err(io::Error::other(format!(
            "FFprobe rejected the recording: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        )));
    }
    parse_media_probe(&result.stdout)
}

fn run_ffprobe(ffprobe: &Path, output: &Path) -> io::Result<Output> {
    let mut child = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=codec_name,width,height:format=duration",
            "-of",
            "json",
        ])
        .arg(output)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    wait_for_ffprobe_child(&mut child, FFPROBE_TIMEOUT)?;
    child.wait_with_output()
}

/// Decodes one bounded video frame into PNG for native recording content verification.
pub(crate) fn extract_video_frame(
    ffmpeg: &Path,
    input: &Path,
    timestamp_seconds: f64,
    output: &Path,
) -> io::Result<()> {
    let arguments = video_frame_arguments(input, timestamp_seconds, output)?;
    let mut child = Command::new(ffmpeg)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    wait_for_child(&mut child, FFPROBE_TIMEOUT, "FFmpeg frame extraction")?;
    let result = child.wait_with_output()?;
    if !result.status.success() {
        return Err(io::Error::other(format!(
            "FFmpeg could not extract a recording frame: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        )));
    }
    if !output.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "FFmpeg reported success without writing the extracted frame",
        ));
    }
    Ok(())
}

/// Decodes a low-rate PNG timeline so dynamic recording phases can be matched in order.
pub(crate) fn extract_video_frame_series(
    ffmpeg: &Path,
    input: &Path,
    frames_per_second: u16,
    output_pattern: &Path,
) -> io::Result<()> {
    let arguments = video_frame_series_arguments(input, frames_per_second, output_pattern)?;
    let mut child = Command::new(ffmpeg)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    wait_for_child(
        &mut child,
        FFPROBE_TIMEOUT,
        "FFmpeg frame timeline extraction",
    )?;
    let result = child.wait_with_output()?;
    if !result.status.success() {
        return Err(io::Error::other(format!(
            "FFmpeg could not extract the recording timeline: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        )));
    }
    Ok(())
}

/// Builds the bounded timeline command independently for focused argument tests.
pub(crate) fn video_frame_series_arguments(
    input: &Path,
    frames_per_second: u16,
    output_pattern: &Path,
) -> io::Result<Vec<OsString>> {
    if !(1..=30).contains(&frames_per_second) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "recording timeline rate must be between 1 and 30 frames per second",
        ));
    }
    Ok(vec![
        OsString::from("-hide_banner"),
        OsString::from("-loglevel"),
        OsString::from("error"),
        OsString::from("-i"),
        input.as_os_str().to_owned(),
        OsString::from("-map"),
        OsString::from("0:v:0"),
        OsString::from("-vf"),
        OsString::from(format!("fps={frames_per_second}:start_time=0:round=near")),
        OsString::from("-an"),
        OsString::from("-c:v"),
        OsString::from("png"),
        OsString::from("-start_number"),
        OsString::from("0"),
        OsString::from("-y"),
        output_pattern.as_os_str().to_owned(),
    ])
}

/// Builds the one-frame decode contract separately so tests do not need to launch FFmpeg.
pub(crate) fn video_frame_arguments(
    input: &Path,
    timestamp_seconds: f64,
    output: &Path,
) -> io::Result<Vec<OsString>> {
    if !timestamp_seconds.is_finite() || timestamp_seconds < 0.0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "video frame timestamp must be finite and non-negative",
        ));
    }
    Ok(vec![
        OsString::from("-hide_banner"),
        OsString::from("-loglevel"),
        OsString::from("error"),
        OsString::from("-ss"),
        OsString::from(format!("{timestamp_seconds:.6}")),
        OsString::from("-i"),
        input.as_os_str().to_owned(),
        OsString::from("-map"),
        OsString::from("0:v:0"),
        OsString::from("-frames:v"),
        OsString::from("1"),
        OsString::from("-an"),
        OsString::from("-c:v"),
        OsString::from("png"),
        OsString::from("-y"),
        output.as_os_str().to_owned(),
    ])
}

/// Polls FFprobe until it exits; a stuck metadata check is terminated and reaped.
pub(crate) fn wait_for_ffprobe_child(child: &mut Child, timeout: Duration) -> io::Result<()> {
    wait_for_child(child, timeout, "FFprobe")
}

/// Polls one probe process until exit and always reaps it after timeout or I/O failure.
fn wait_for_child(child: &mut Child, timeout: Duration, label: &str) -> io::Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "{label} exceeded {} ms and was terminated",
                        timeout.as_millis()
                    ),
                ));
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        }
    }
}

/// Parses and validates the small FFprobe JSON contract written into acceptance reports.
pub(crate) fn parse_media_probe(stdout: &[u8]) -> io::Result<MediaMetadata> {
    let probe: ProbeOutput = serde_json::from_slice(stdout).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid FFprobe JSON: {error}"),
        )
    })?;
    let stream = probe.streams.into_iter().next().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "FFprobe found no video stream")
    })?;
    let codec_name = stream
        .codec_name
        .filter(|codec| !codec.trim().is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "FFprobe reported no codec"))?;
    let width = stream.width.filter(|width| *width > 0).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "FFprobe reported no video width",
        )
    })?;
    let height = stream.height.filter(|height| *height > 0).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "FFprobe reported no video height",
        )
    })?;
    let duration = probe
        .format
        .and_then(|format| format.duration)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "FFprobe reported no duration"))?
        .parse::<f64>()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid FFprobe duration"))?;
    if !duration.is_finite() || duration <= 0.0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "FFprobe reported an empty recording",
        ));
    }
    Ok(MediaMetadata {
        codec_name,
        width,
        height,
        duration_seconds: duration,
    })
}

#[cfg(test)]
mod tests {
    use super::{video_frame_arguments, video_frame_series_arguments};
    use std::{ffi::OsString, path::Path};

    #[test]
    fn frame_extraction_decodes_one_silent_png_and_overwrites_only_its_target() {
        let arguments =
            video_frame_arguments(Path::new("recording.mp4"), 1.25, Path::new("evidence.png"))
                .unwrap();

        assert_eq!(
            arguments,
            [
                "-hide_banner",
                "-loglevel",
                "error",
                "-ss",
                "1.250000",
                "-i",
                "recording.mp4",
                "-map",
                "0:v:0",
                "-frames:v",
                "1",
                "-an",
                "-c:v",
                "png",
                "-y",
                "evidence.png",
            ]
            .map(OsString::from)
        );
        assert!(
            video_frame_arguments(
                Path::new("recording.mp4"),
                f64::NAN,
                Path::new("evidence.png")
            )
            .is_err()
        );
    }

    #[test]
    fn frame_timeline_uses_a_bounded_rate_and_zero_based_output_names() {
        let arguments = video_frame_series_arguments(
            Path::new("recording.mp4"),
            5,
            Path::new("timeline/frame-%05d.png"),
        )
        .unwrap();

        assert_eq!(
            arguments,
            [
                "-hide_banner",
                "-loglevel",
                "error",
                "-i",
                "recording.mp4",
                "-map",
                "0:v:0",
                "-vf",
                "fps=5:start_time=0:round=near",
                "-an",
                "-c:v",
                "png",
                "-start_number",
                "0",
                "-y",
                "timeline/frame-%05d.png",
            ]
            .map(OsString::from)
        );
        assert!(
            video_frame_series_arguments(Path::new("recording.mp4"), 0, Path::new("frame-%d.png"))
                .is_err()
        );
    }
}
