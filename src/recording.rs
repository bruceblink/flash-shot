//! Isolated FFmpeg discovery and capture capability probing.
//!
//! This module deliberately owns only executable discovery and read-only probing. Recording
//! sessions will build on these stable data types without leaking process details into the UI.

use std::{
    env,
    ffi::{OsStr, OsString},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, ExitStatus, Output, Stdio},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::domain::geometry::PhysicalRect;

const FFMPEG_PATH_ENV: &str = "FLASH_SHOT_FFMPEG";
const VERSION_ARGUMENTS: &[&str] = &["-hide_banner", "-version"];
const FORMAT_ARGUMENTS: &[&str] = &["-hide_banner", "-formats"];
const DEVICE_ARGUMENTS: &[&str] = &["-hide_banner", "-devices"];

/// Maximum time a recording process gets to finalize its container after receiving `q`.
pub const GRACEFUL_STOP_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_DIAGNOSTIC_BYTES: usize = 64 * 1024;

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
        self.supports_input("wasapi")
    }

    pub fn supports_input(&self, name: &str) -> bool {
        self.input_formats.iter().any(|input| input == name)
    }
}

/// A physical-pixel video source selected before an FFmpeg process is started.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecordingTarget {
    /// A complete display represented by its physical desktop bounds.
    Display { bounds: PhysicalRect },
    /// A top-level Windows window addressed by its visible title.
    Window { title: String },
    /// A user-selected physical-pixel rectangle in virtual desktop coordinates.
    Region { bounds: PhysicalRect },
}

/// An explicitly selected local audio input for a recording.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AudioSource {
    /// A DirectShow microphone device name as reported by FFmpeg.
    Microphone { device: String },
    /// A WASAPI loopback or output device name as reported by FFmpeg.
    SystemAudio { device: String },
}

/// A validated first-pass MP4 recording request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordingRequest {
    pub target: RecordingTarget,
    pub audio: Option<AudioSource>,
    pub frame_rate: u16,
    pub output: PathBuf,
}

/// An argument-vector command ready to launch without a shell.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FfmpegCommand {
    executable: PathBuf,
    arguments: Vec<OsString>,
}

/// Observable lifecycle for one FFmpeg recording process.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RecordingState {
    #[default]
    Idle,
    Starting,
    Recording,
    Paused,
    Stopping,
    Failed,
}

/// Process-independent recording lifecycle with legal transition checks.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecordingSession {
    state: RecordingState,
    request: Option<RecordingRequest>,
    failure: Option<String>,
}

impl RecordingSession {
    pub const fn state(&self) -> RecordingState {
        self.state
    }

    pub fn request(&self) -> Option<&RecordingRequest> {
        self.request.as_ref()
    }

    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    /// Starts a session before the external process has confirmed that capture is live.
    pub fn begin(&mut self, request: RecordingRequest) -> io::Result<()> {
        self.require(RecordingState::Idle, "begin")?;
        validate_request(&request)?;
        self.request = Some(request);
        self.failure = None;
        self.state = RecordingState::Starting;
        Ok(())
    }

    /// Marks the process as producing a recording after FFmpeg starts successfully.
    pub fn mark_recording(&mut self) -> io::Result<()> {
        self.require(RecordingState::Starting, "mark recording")?;
        self.state = RecordingState::Recording;
        Ok(())
    }

    pub fn pause(&mut self) -> io::Result<()> {
        self.require(RecordingState::Recording, "pause")?;
        self.state = RecordingState::Paused;
        Ok(())
    }

    pub fn resume(&mut self) -> io::Result<()> {
        self.require(RecordingState::Paused, "resume")?;
        self.state = RecordingState::Recording;
        Ok(())
    }

    /// Enters the finalization state. The process owner should write [`graceful_stop_input`].
    pub fn request_stop(&mut self) -> io::Result<()> {
        if !matches!(
            self.state,
            RecordingState::Recording | RecordingState::Paused
        ) {
            return Err(invalid_recording_transition(self.state, "request stop"));
        }
        self.state = RecordingState::Stopping;
        Ok(())
    }

    /// Finalizes a normally stopped process and releases its request data.
    pub fn finish(&mut self) -> io::Result<()> {
        self.require(RecordingState::Stopping, "finish")?;
        *self = Self::default();
        Ok(())
    }

    /// Records a recoverable process failure without panicking the application.
    pub fn fail(&mut self, error: impl std::fmt::Display) -> io::Result<()> {
        if matches!(self.state, RecordingState::Idle | RecordingState::Failed) {
            return Err(invalid_recording_transition(self.state, "fail"));
        }
        self.failure = Some(error.to_string());
        self.state = RecordingState::Failed;
        Ok(())
    }

    /// Clears a completed failure before a new recording is started.
    pub fn reset(&mut self) -> io::Result<()> {
        self.require(RecordingState::Failed, "reset")?;
        *self = Self::default();
        Ok(())
    }

    fn require(&self, expected: RecordingState, operation: &'static str) -> io::Result<()> {
        if self.state == expected {
            Ok(())
        } else {
            Err(invalid_recording_transition(self.state, operation))
        }
    }
}

/// FFmpeg's documented interactive command for a normal, container-safe stop.
pub const fn graceful_stop_input() -> &'static [u8] {
    b"q\n"
}

impl FfmpegCommand {
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    pub fn arguments(&self) -> &[OsString] {
        &self.arguments
    }

    pub fn into_command(self) -> Command {
        let mut command = Command::new(self.executable);
        command.args(self.arguments);
        command
    }
}

/// Completion data retained after an FFmpeg process has exited.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordingExit {
    pub success: bool,
    pub diagnostic: String,
}

/// Owns a single FFmpeg child process and guarantees cleanup when the owner is dropped.
pub struct RecordingProcess {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stderr_reader: Option<JoinHandle<io::Result<Vec<u8>>>>,
}

impl RecordingProcess {
    /// Starts FFmpeg with piped control input and continuously drained stderr.
    pub fn start(command: FfmpegCommand) -> io::Result<Self> {
        let mut child = command
            .into_command()
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                io::Error::new(error.kind(), format!("could not start FFmpeg: {error}"))
            })?;
        let stdin = child.stdin.take().ok_or_else(|| {
            io::Error::other("FFmpeg control input pipe was not available after startup")
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            io::Error::other("FFmpeg diagnostic pipe was not available after startup")
        })?;
        let stderr_reader = thread::spawn(move || read_bounded_diagnostics(stderr));
        Ok(Self {
            child: Some(child),
            stdin: Some(stdin),
            stderr_reader: Some(stderr_reader),
        })
    }

    /// Waits for natural completion and returns the bounded FFmpeg diagnostic output.
    pub fn wait_for_exit(&mut self) -> io::Result<RecordingExit> {
        let status = self
            .child
            .take()
            .ok_or_else(|| io::Error::other("recording process has already been reaped"))?
            .wait()?;
        self.stdin.take();
        self.complete(status)
    }

    /// Requests a container-safe FFmpeg stop, then kills only after the timeout expires.
    pub fn stop_gracefully(&mut self, timeout: Duration) -> io::Result<RecordingExit> {
        let stdin = self
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("recording process control input is unavailable"))?;
        write_graceful_stop(stdin)?;
        let deadline = Instant::now() + timeout;
        loop {
            let child = self
                .child
                .as_mut()
                .ok_or_else(|| io::Error::other("recording process has already been reaped"))?;
            if let Some(status) = child.try_wait()? {
                self.child.take();
                return self.complete(status);
            }
            if Instant::now() >= deadline {
                let mut child = self.child.take().expect("checked above");
                let _ = child.kill();
                let _ = child.wait();
                let diagnostic = self.join_diagnostics()?;
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "FFmpeg did not stop within {} ms and was terminated{}",
                        timeout.as_millis(),
                        diagnostic_suffix(&diagnostic),
                    ),
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn complete(&mut self, status: ExitStatus) -> io::Result<RecordingExit> {
        let diagnostic = self.join_diagnostics()?;
        let exit = RecordingExit {
            success: status.success(),
            diagnostic,
        };
        if exit.success {
            Ok(exit)
        } else {
            Err(io::Error::other(format!(
                "FFmpeg exited with {status}{}",
                diagnostic_suffix(&exit.diagnostic),
            )))
        }
    }

    fn join_diagnostics(&mut self) -> io::Result<String> {
        let Some(reader) = self.stderr_reader.take() else {
            return Ok(String::new());
        };
        let bytes = reader
            .join()
            .map_err(|_| io::Error::other("FFmpeg diagnostic reader panicked"))??;
        Ok(String::from_utf8_lossy(&bytes).trim().to_owned())
    }
}

impl Drop for RecordingProcess {
    fn drop(&mut self) {
        self.stdin.take();
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
    }
}

fn write_graceful_stop(mut stdin: ChildStdin) -> io::Result<()> {
    stdin.write_all(graceful_stop_input())?;
    stdin.flush()
}

fn read_bounded_diagnostics(mut stderr: impl Read) -> io::Result<Vec<u8>> {
    let mut retained = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let read = stderr.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let remaining = MAX_DIAGNOSTIC_BYTES.saturating_sub(retained.len());
        retained.extend_from_slice(&buffer[..read.min(remaining)]);
    }
    Ok(retained)
}

fn diagnostic_suffix(diagnostic: &str) -> String {
    first_diagnostic_line(diagnostic)
        .map(|line| format!(": {line}"))
        .unwrap_or_default()
}

/// Builds a shell-free FFmpeg command for a display, window, or region recording.
///
/// This only validates intent and arguments. Process lifecycle, audio selection, progress, and
/// cleanup stay in the upcoming recording-session boundary.
pub fn build_recording_command(
    capabilities: &FfmpegCapabilities,
    request: &RecordingRequest,
) -> io::Result<FfmpegCommand> {
    validate_request(request)?;
    let mut arguments = vec![OsString::from("-hide_banner"), OsString::from("-y")];
    match &request.target {
        RecordingTarget::Display { bounds } | RecordingTarget::Region { bounds } => {
            let input = desktop_input(capabilities, *bounds, request.frame_rate)?;
            arguments.extend(input);
        }
        RecordingTarget::Window { title } => {
            if !capabilities.supports_window_capture() {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "FFmpeg does not support Windows window capture (gdigrab unavailable)",
                ));
            }
            arguments.extend([
                OsString::from("-f"),
                OsString::from("gdigrab"),
                OsString::from("-framerate"),
                OsString::from(request.frame_rate.to_string()),
                OsString::from("-i"),
                OsString::from(format!("title={title}")),
            ]);
        }
    }
    let has_audio = if let Some(audio) = &request.audio {
        arguments.extend(audio_input(capabilities, audio)?);
        true
    } else {
        false
    };
    arguments.extend([
        OsString::from("-c:v"),
        OsString::from("libx264"),
        OsString::from("-pix_fmt"),
        OsString::from("yuv420p"),
        OsString::from("-movflags"),
        OsString::from("+faststart"),
    ]);
    if has_audio {
        arguments.extend([OsString::from("-c:a"), OsString::from("aac")]);
    }
    arguments.push(request.output.as_os_str().to_owned());
    Ok(FfmpegCommand {
        executable: capabilities.executable.clone(),
        arguments,
    })
}

fn audio_input(
    capabilities: &FfmpegCapabilities,
    audio: &AudioSource,
) -> io::Result<Vec<OsString>> {
    match audio {
        AudioSource::Microphone { device } => {
            if !capabilities.supports_microphone_capture() {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "FFmpeg does not support microphone capture (dshow unavailable)",
                ));
            }
            Ok(vec![
                OsString::from("-f"),
                OsString::from("dshow"),
                OsString::from("-i"),
                OsString::from(format!("audio={device}")),
            ])
        }
        AudioSource::SystemAudio { device } => {
            if !capabilities.supports_system_audio_capture() {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "FFmpeg does not support Windows system audio capture (wasapi unavailable)",
                ));
            }
            Ok(vec![
                OsString::from("-f"),
                OsString::from("wasapi"),
                OsString::from("-i"),
                OsString::from(device),
            ])
        }
    }
}

fn desktop_input(
    capabilities: &FfmpegCapabilities,
    bounds: PhysicalRect,
    frame_rate: u16,
) -> io::Result<Vec<OsString>> {
    let input = if capabilities.supports_input("ddagrab") {
        "ddagrab"
    } else if capabilities.supports_input("gdigrab") {
        "gdigrab"
    } else {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "FFmpeg does not support Windows display capture (ddagrab or gdigrab unavailable)",
        ));
    };
    Ok(vec![
        OsString::from("-f"),
        OsString::from(input),
        OsString::from("-framerate"),
        OsString::from(frame_rate.to_string()),
        OsString::from("-offset_x"),
        OsString::from(bounds.left.to_string()),
        OsString::from("-offset_y"),
        OsString::from(bounds.top.to_string()),
        OsString::from("-video_size"),
        OsString::from(format!("{}x{}", bounds.width(), bounds.height())),
        OsString::from("-i"),
        OsString::from("desktop"),
    ])
}

fn validate_request(request: &RecordingRequest) -> io::Result<()> {
    if !(1..=240).contains(&request.frame_rate) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "recording frame rate must be between 1 and 240",
        ));
    }
    if request
        .output
        .extension()
        .is_none_or(|extension| !extension.eq_ignore_ascii_case("mp4"))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "recording output must use an .mp4 extension",
        ));
    }
    match &request.target {
        RecordingTarget::Display { bounds } | RecordingTarget::Region { bounds }
            if bounds.width() == 0 || bounds.height() == 0 =>
        {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "recording bounds must have a positive width and height",
            ))
        }
        RecordingTarget::Window { title } if title.trim().is_empty() => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "recording window title must not be empty",
        )),
        _ if request
            .audio
            .as_ref()
            .is_some_and(|audio| audio_device_name(audio).trim().is_empty()) =>
        {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "recording audio device name must not be empty",
            ))
        }
        _ => Ok(()),
    }
}

fn audio_device_name(audio: &AudioSource) -> &str {
    match audio {
        AudioSource::Microphone { device } | AudioSource::SystemAudio { device } => device,
    }
}

fn invalid_recording_transition(state: RecordingState, operation: &'static str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("cannot {operation} recording while session is {state:?}"),
    )
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
        AudioSource, DEVICE_ARGUMENTS, FORMAT_ARGUMENTS, FfmpegCapabilities, FfmpegCommand,
        GRACEFUL_STOP_TIMEOUT, RecordingProcess, RecordingRequest, RecordingSession,
        RecordingState, RecordingTarget, VERSION_ARGUMENTS, build_recording_command,
        diagnostic_suffix, executable_from, first_diagnostic_line, graceful_stop_input,
        parse_input_formats, parse_version, read_bounded_diagnostics,
    };
    use crate::domain::geometry::PhysicalRect;
    use std::{ffi::OsString, io::Cursor, path::PathBuf, time::Duration};

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
        assert!(!capabilities.supports_system_audio_capture());
        assert!(!capabilities.supports_input("avfoundation"));
    }

    #[test]
    fn command_uses_desktop_duplication_for_a_negative_coordinate_display() {
        let command = build_recording_command(
            &capabilities(),
            &RecordingRequest {
                target: RecordingTarget::Display {
                    bounds: PhysicalRect {
                        left: -1920,
                        top: 40,
                        right: 0,
                        bottom: 1120,
                    },
                },
                audio: None,
                frame_rate: 60,
                output: PathBuf::from("recording.mp4"),
            },
        )
        .unwrap();

        assert_eq!(command.executable(), PathBuf::from("ffmpeg"));
        assert_eq!(
            command.arguments(),
            [
                "-hide_banner",
                "-y",
                "-f",
                "ddagrab",
                "-framerate",
                "60",
                "-offset_x",
                "-1920",
                "-offset_y",
                "40",
                "-video_size",
                "1920x1080",
                "-i",
                "desktop",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-movflags",
                "+faststart",
                "recording.mp4",
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn window_recording_uses_gdigrab_title_input_without_a_shell() {
        let command = build_recording_command(
            &capabilities(),
            &RecordingRequest {
                target: RecordingTarget::Window {
                    title: "Editor & terminal".to_owned(),
                },
                audio: None,
                frame_rate: 30,
                output: PathBuf::from("window.mp4"),
            },
        )
        .unwrap();

        assert!(command.arguments().windows(2).any(|pair| pair
            == [
                OsString::from("-i"),
                OsString::from("title=Editor & terminal")
            ]));
    }

    #[test]
    fn recording_requests_reject_invalid_rates_extensions_and_targets() {
        let capabilities = capabilities();
        let invalid = |target, frame_rate, output| RecordingRequest {
            target,
            audio: None,
            frame_rate,
            output: PathBuf::from(output),
        };

        assert!(
            build_recording_command(
                &capabilities,
                &invalid(
                    RecordingTarget::Region {
                        bounds: PhysicalRect::default(),
                    },
                    60,
                    "recording.mp4",
                ),
            )
            .is_err()
        );
        assert!(
            build_recording_command(
                &capabilities,
                &invalid(
                    RecordingTarget::Window {
                        title: " ".to_owned(),
                    },
                    0,
                    "recording.webm",
                ),
            )
            .is_err()
        );
    }

    #[test]
    fn recording_session_follows_the_normal_start_pause_stop_lifecycle() {
        let mut session = RecordingSession::default();
        let request = region_request();

        session.begin(request.clone()).unwrap();
        assert_eq!(session.state(), RecordingState::Starting);
        assert_eq!(session.request(), Some(&request));
        session.mark_recording().unwrap();
        session.pause().unwrap();
        assert_eq!(session.state(), RecordingState::Paused);
        session.resume().unwrap();
        session.request_stop().unwrap();
        assert_eq!(graceful_stop_input(), b"q\n");
        assert_eq!(GRACEFUL_STOP_TIMEOUT, std::time::Duration::from_secs(10));
        session.finish().unwrap();
        assert_eq!(session.state(), RecordingState::Idle);
        assert!(session.request().is_none());
    }

    #[test]
    fn microphone_and_system_audio_use_their_explicit_ffmpeg_inputs() {
        let microphone = RecordingRequest {
            audio: Some(AudioSource::Microphone {
                device: "Microphone (USB)".to_owned(),
            }),
            ..region_request()
        };
        let microphone = build_recording_command(&capabilities(), &microphone).unwrap();
        assert!(
            microphone
                .arguments()
                .windows(2)
                .any(|pair| { pair == [OsString::from("-f"), OsString::from("dshow")] })
        );
        assert!(microphone.arguments().windows(2).any(|pair| {
            pair == [
                OsString::from("-i"),
                OsString::from("audio=Microphone (USB)"),
            ]
        }));

        let system = RecordingRequest {
            audio: Some(AudioSource::SystemAudio {
                device: "default".to_owned(),
            }),
            ..region_request()
        };
        let system = build_recording_command(&wasapi_capabilities(), &system).unwrap();
        assert!(
            system
                .arguments()
                .windows(2)
                .any(|pair| { pair == [OsString::from("-f"), OsString::from("wasapi")] })
        );
    }

    #[test]
    fn audio_requires_a_supported_backend_and_non_empty_device_name() {
        let request = RecordingRequest {
            audio: Some(AudioSource::SystemAudio {
                device: "default".to_owned(),
            }),
            ..region_request()
        };
        assert!(build_recording_command(&capabilities(), &request).is_err());

        let request = RecordingRequest {
            audio: Some(AudioSource::Microphone {
                device: " ".to_owned(),
            }),
            ..region_request()
        };
        assert!(build_recording_command(&capabilities(), &request).is_err());
    }

    #[test]
    fn recording_session_makes_failures_observable_and_recoverable() {
        let mut session = RecordingSession::default();
        session.begin(region_request()).unwrap();
        session.fail("FFmpeg exited with code 1").unwrap();

        assert_eq!(session.state(), RecordingState::Failed);
        assert_eq!(session.failure(), Some("FFmpeg exited with code 1"));
        assert!(session.request_stop().is_err());
        session.reset().unwrap();
        assert_eq!(session.state(), RecordingState::Idle);
        assert!(session.failure().is_none());
    }

    #[test]
    fn recording_session_rejects_out_of_order_lifecycle_operations() {
        let mut session = RecordingSession::default();

        assert!(session.pause().is_err());
        assert!(session.finish().is_err());
        assert!(session.reset().is_err());
        session.begin(region_request()).unwrap();
        assert!(session.resume().is_err());
        assert!(session.request_stop().is_err());
    }

    #[test]
    fn diagnostics_are_bounded_and_include_only_the_first_line_in_errors() {
        let diagnostics =
            read_bounded_diagnostics(Cursor::new(b"failed to initialize\nverbose")).unwrap();
        let diagnostics = String::from_utf8(diagnostics).unwrap();

        assert_eq!(diagnostic_suffix(&diagnostics), ": failed to initialize");
    }

    #[cfg(windows)]
    #[test]
    fn process_stop_sends_ffmpeg_control_input_and_reaps_the_child() {
        let command = FfmpegCommand {
            executable: PathBuf::from("cmd.exe"),
            arguments: ["/C", "more > nul & echo finalized 1>&2"]
                .map(OsString::from)
                .into(),
        };
        let mut process = RecordingProcess::start(command).unwrap();
        let exit = process.stop_gracefully(Duration::from_secs(2)).unwrap();

        assert!(exit.success);
        assert_eq!(exit.diagnostic, "finalized");
    }

    #[cfg(windows)]
    #[test]
    fn process_exit_errors_include_bounded_ffmpeg_diagnostics() {
        let command = FfmpegCommand {
            executable: PathBuf::from("cmd.exe"),
            arguments: ["/C", "echo encoder failed 1>&2 & exit /b 7"]
                .map(OsString::from)
                .into(),
        };
        let mut process = RecordingProcess::start(command).unwrap();
        let error = process.wait_for_exit().unwrap_err();

        assert!(error.to_string().contains("encoder failed"));
    }

    fn capabilities() -> FfmpegCapabilities {
        FfmpegCapabilities {
            executable: PathBuf::from("ffmpeg"),
            version: "7.1".to_owned(),
            input_formats: parse_input_formats(FORMATS),
        }
    }

    fn wasapi_capabilities() -> FfmpegCapabilities {
        let mut capabilities = capabilities();
        capabilities.input_formats.push("wasapi".to_owned());
        capabilities
    }

    fn region_request() -> RecordingRequest {
        RecordingRequest {
            target: RecordingTarget::Region {
                bounds: PhysicalRect {
                    left: 5,
                    top: 10,
                    right: 805,
                    bottom: 610,
                },
            },
            audio: None,
            frame_rate: 30,
            output: PathBuf::from("region.mp4"),
        }
    }
}
