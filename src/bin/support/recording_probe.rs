//! Shared FFprobe validation used by native recording acceptance executables.

use std::{
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

/// Polls FFprobe until it exits; a stuck metadata check is terminated and reaped.
pub(crate) fn wait_for_ffprobe_child(child: &mut Child, timeout: Duration) -> io::Result<()> {
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
                        "FFprobe exceeded {} ms and was terminated",
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
