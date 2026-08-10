//! Real FFmpeg recording lifecycle probe for Windows acceptance evidence.

use std::{
    fs, io,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

use flash_shot::{
    domain::geometry::PhysicalRect,
    platform::display::{DisplayProvider, SystemDisplayProvider},
    recording::{RecordingEvent, RecordingRequest, RecordingTarget, discover, start_recording},
};
use serde::{Deserialize, Serialize};

const EVENT_TIMEOUT: Duration = Duration::from_secs(20);
const FFPROBE_TIMEOUT: Duration = Duration::from_secs(10);
const ACTIVE_CAPTURE_DELAY: Duration = Duration::from_millis(900);
const PAUSE_DELAY: Duration = Duration::from_millis(350);

#[derive(Debug)]
struct Options {
    target: TargetOption,
    output: PathBuf,
    report: PathBuf,
}

#[derive(Debug)]
enum TargetOption {
    Display,
    Region,
    Window(String),
}

impl Options {
    /// Parses a stable positional command for display, region, or named-window acceptance.
    fn parse() -> Result<Self, String> {
        let mut arguments = std::env::args_os().skip(1);
        let target = arguments
            .next()
            .and_then(|value| value.into_string().ok())
            .ok_or_else(usage)?;
        let output = arguments.next().map(PathBuf::from).ok_or_else(usage)?;
        let report = arguments.next().map(PathBuf::from).ok_or_else(usage)?;
        let target = match target.as_str() {
            "display" => TargetOption::Display,
            "region" => TargetOption::Region,
            "window" => {
                let title = arguments
                    .next()
                    .and_then(|value| value.into_string().ok())
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| "window recording requires a non-empty title".to_owned())?;
                TargetOption::Window(title)
            }
            _ => return Err("target must be 'display', 'region', or 'window'".to_owned()),
        };
        if arguments.next().is_some() {
            return Err(usage());
        }
        Ok(Self {
            target,
            output,
            report,
        })
    }
}

#[derive(Debug, Serialize)]
struct AcceptanceReport {
    schema_version: u32,
    target: &'static str,
    ffmpeg_version: String,
    output: PathBuf,
    output_bytes: u64,
    codec_name: String,
    width: u32,
    height: u32,
    duration_seconds: f64,
    pause_observed: bool,
    resume_observed: bool,
    maximum_progress_frame: u64,
}

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
struct MediaMetadata {
    codec_name: String,
    width: u32,
    height: u32,
    duration_seconds: f64,
}

#[derive(Default)]
struct ObservedEvents {
    pause: bool,
    resume: bool,
    maximum_progress_frame: u64,
}

fn usage() -> String {
    "usage: recording-acceptance <display|region|window> <output.mp4> <report.json> [window title]"
        .to_owned()
}

fn main() {
    if let Err(error) = run() {
        eprintln!("recording acceptance failed: {error}");
        std::process::exit(1);
    }
}

/// Runs the production recording worker through start, pause, resume, and graceful stop.
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let options = Options::parse().map_err(io::Error::other)?;
    let output = std::path::absolute(&options.output)?;
    let report_path = std::path::absolute(&options.report)?;
    create_parent(&output)?;
    create_parent(&report_path)?;
    prepare_reusable_output(&output)?;

    let capabilities = discover()?;
    if !capabilities.supports_display_capture() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "installed FFmpeg cannot capture the Windows desktop",
        )
        .into());
    }
    let (target_name, target) = resolve_target(options.target)?;
    let control = start_recording(
        capabilities.clone(),
        RecordingRequest {
            target,
            audio: None,
            frame_rate: 15,
            output: output.clone(),
        },
    )?;
    let events = control.events();
    let mut observed = ObservedEvents::default();

    wait_for_event(&events, "start", &mut observed, |event| {
        matches!(event, RecordingEvent::Started)
    })?;
    thread::sleep(ACTIVE_CAPTURE_DELAY);
    control.set_paused(true)?;
    wait_for_event(&events, "pause", &mut observed, |event| {
        matches!(event, RecordingEvent::Paused)
    })?;
    thread::sleep(PAUSE_DELAY);
    control.set_paused(false)?;
    wait_for_event(&events, "resume", &mut observed, |event| {
        matches!(event, RecordingEvent::Resumed)
    })?;
    thread::sleep(ACTIVE_CAPTURE_DELAY);
    control.request_stop()?;
    wait_for_event(&events, "finish", &mut observed, |event| {
        matches!(event, RecordingEvent::Finished { .. })
    })?;

    let metadata = fs::metadata(&output)?;
    if metadata.len() == 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "recorded MP4 is empty").into());
    }
    let media = probe_media(capabilities.executable(), &output)?;
    let report = AcceptanceReport {
        schema_version: 2,
        target: target_name,
        ffmpeg_version: capabilities.version().to_owned(),
        output,
        output_bytes: metadata.len(),
        codec_name: media.codec_name,
        width: media.width,
        height: media.height,
        duration_seconds: media.duration_seconds,
        pause_observed: observed.pause,
        resume_observed: observed.resume,
        maximum_progress_frame: observed.maximum_progress_frame,
    };
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn create_parent(path: &Path) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("acceptance output requires a parent directory"))?;
    fs::create_dir_all(parent)
}

/// Removes only the explicitly named probe artifact so repeated acceptance runs remain reusable.
///
/// Production recordings use FFmpeg's no-overwrite mode. This executable is different: its
/// command-line output path intentionally names disposable test evidence, so replacing that exact
/// artifact preserves the long-standing rerun behavior without weakening product safety.
fn prepare_reusable_output(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Resolves a CLI target into the same physical-pixel request used by the product UI.
fn resolve_target(option: TargetOption) -> io::Result<(&'static str, RecordingTarget)> {
    match option {
        TargetOption::Window(title) => Ok(("window", RecordingTarget::Window { title })),
        TargetOption::Display | TargetOption::Region => {
            let display = SystemDisplayProvider
                .displays()?
                .into_iter()
                .find(|display| display.primary)
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, "primary display missing")
                })?;
            match option {
                TargetOption::Display => Ok((
                    "display",
                    RecordingTarget::Display {
                        bounds: display.physical_bounds,
                    },
                )),
                TargetOption::Region => Ok((
                    "region",
                    RecordingTarget::Region {
                        bounds: centered_region(display.physical_bounds, 640, 360),
                    },
                )),
                TargetOption::Window(_) => unreachable!(),
            }
        }
    }
}

/// Centers an even-sized region inside a display so H.264 yuv420p encoding remains valid.
fn centered_region(
    display: PhysicalRect,
    requested_width: i32,
    requested_height: i32,
) -> PhysicalRect {
    let width = requested_width.min(display.width() as i32).max(2) & !1;
    let height = requested_height.min(display.height() as i32).max(2) & !1;
    let left = display.left + (display.width() as i32 - width) / 2;
    let top = display.top + (display.height() as i32 - height) / 2;
    PhysicalRect {
        left,
        top,
        right: left + width,
        bottom: top + height,
    }
}

/// Drains progress while waiting for one lifecycle event, failing fast on worker errors.
fn wait_for_event(
    events: &async_channel::Receiver<RecordingEvent>,
    stage: &str,
    observed: &mut ObservedEvents,
    expected: impl Fn(&RecordingEvent) -> bool,
) -> io::Result<()> {
    let deadline = Instant::now() + EVENT_TIMEOUT;
    loop {
        match events.try_recv() {
            Ok(event) => {
                match &event {
                    RecordingEvent::Paused => observed.pause = true,
                    RecordingEvent::Resumed => observed.resume = true,
                    RecordingEvent::Progress(progress) => {
                        observed.maximum_progress_frame = observed
                            .maximum_progress_frame
                            .max(progress.frame.unwrap_or_default());
                    }
                    RecordingEvent::Failed { message } => {
                        return Err(io::Error::other(message.clone()));
                    }
                    _ => {}
                }
                if expected(&event) {
                    return Ok(());
                }
            }
            Err(async_channel::TryRecvError::Empty) => {
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!("timed out waiting for recording {stage}"),
                    ));
                }
                thread::sleep(Duration::from_millis(25));
            }
            Err(async_channel::TryRecvError::Closed) => {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    format!("recording worker closed before {stage}"),
                ));
            }
        }
    }
}

/// Uses the FFprobe installed beside FFmpeg to prove the finalized MP4's media metadata.
fn probe_media(ffmpeg: &Path, output: &Path) -> io::Result<MediaMetadata> {
    let ffprobe = ffmpeg.with_file_name(if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    });
    let result = run_ffprobe(&ffprobe, output)?;
    if !result.status.success() {
        return Err(io::Error::other(format!(
            "FFprobe rejected the MP4: {}",
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
fn wait_for_ffprobe_child(child: &mut Child, timeout: Duration) -> io::Result<()> {
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
fn parse_media_probe(stdout: &[u8]) -> io::Result<MediaMetadata> {
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
    use super::{
        FFPROBE_TIMEOUT, centered_region, parse_media_probe, prepare_reusable_output,
        wait_for_ffprobe_child,
    };
    use flash_shot::domain::geometry::PhysicalRect;
    use std::{
        process::{Command, Stdio},
        time::Duration,
    };

    #[test]
    fn ffprobe_checks_have_a_bounded_timeout() {
        assert_eq!(FFPROBE_TIMEOUT, Duration::from_secs(10));
    }

    #[test]
    fn stuck_ffprobe_checks_are_terminated_at_the_deadline() {
        let mut command = if cfg!(windows) {
            let mut command = Command::new("cmd");
            command.args(["/C", "ping -n 5 127.0.0.1 >NUL"]);
            command
        } else {
            let mut command = Command::new("sh");
            command.args(["-c", "sleep 2"]);
            command
        };
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        let error = wait_for_ffprobe_child(&mut child, Duration::from_millis(100)).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(child.try_wait().unwrap().is_some());
    }

    #[test]
    fn acceptance_region_is_centered_even_and_clamped_to_the_display() {
        assert_eq!(
            centered_region(
                PhysicalRect {
                    left: -800,
                    top: 100,
                    right: 800,
                    bottom: 1000,
                },
                641,
                361,
            ),
            PhysicalRect {
                left: -320,
                top: 370,
                right: 320,
                bottom: 730,
            }
        );
    }

    #[test]
    fn reusable_probe_output_clears_only_its_explicit_file() {
        let root = std::env::temp_dir().join(format!(
            "flash-shot-recording-probe-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let output = root.join("recording.mp4");
        let retained = root.join("keep.mp4");
        std::fs::write(&output, b"old test artifact").unwrap();
        std::fs::write(&retained, b"keep this artifact").unwrap();

        prepare_reusable_output(&output).unwrap();
        assert!(!output.exists());
        assert_eq!(std::fs::read(retained).unwrap(), b"keep this artifact");
        prepare_reusable_output(&output).unwrap();

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn media_probe_requires_a_valid_video_stream_and_duration() {
        let metadata = parse_media_probe(
            br#"{"streams":[{"codec_name":"h264","width":520,"height":640}],"format":{"duration":"2.8"}}"#,
        )
        .unwrap();
        assert_eq!(
            metadata,
            super::MediaMetadata {
                codec_name: "h264".to_owned(),
                width: 520,
                height: 640,
                duration_seconds: 2.8,
            }
        );
        assert!(parse_media_probe(br#"{"streams":[],"format":{"duration":"2.8"}}"#).is_err());
        assert!(parse_media_probe(
            br#"{"streams":[{"codec_name":"h264","width":520,"height":640}],"format":{"duration":"0"}}"#
        )
        .is_err());
    }
}
