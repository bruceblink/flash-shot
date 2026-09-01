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
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::domain::geometry::PhysicalRect;
use crate::platform::process_group::ProcessGroup;
use crate::platform::process_pause::set_paused;

const FFMPEG_PATH_ENV: &str = "FLASH_SHOT_FFMPEG";
const MICROPHONE_DEVICE_ENV: &str = "FLASH_SHOT_RECORDING_MICROPHONE";
const SYSTEM_AUDIO_DEVICE_ENV: &str = "FLASH_SHOT_RECORDING_SYSTEM_AUDIO";
const VERSION_ARGUMENTS: &[&str] = &["-hide_banner", "-version"];
const FORMAT_ARGUMENTS: &[&str] = &["-hide_banner", "-formats"];
const DEVICE_ARGUMENTS: &[&str] = &["-hide_banner", "-devices"];
const DSHOW_AUDIO_DEVICE_ARGUMENTS: &[&str] = &[
    "-hide_banner",
    "-list_devices",
    "true",
    "-f",
    "dshow",
    "-i",
    "dummy",
];

/// Maximum time a recording process gets to finalize its container after receiving `q`.
pub const GRACEFUL_STOP_TIMEOUT: Duration = Duration::from_secs(10);
const FFMPEG_PROBE_TIMEOUT: Duration = Duration::from_secs(10);
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

    /// Returns a compact one-line version label for fixed-width settings and status surfaces.
    ///
    /// FFmpeg's raw banner may include a copyright suffix or line breaks; those details remain
    /// available through [`Self::version`] for reports, while this label keeps UI feedback readable.
    pub fn version_label(&self) -> String {
        compact_version_label(&self.version)
    }

    pub fn input_formats(&self) -> &[String] {
        &self.input_formats
    }

    /// Desktop Duplication is preferred; GDI capture is a compatible Windows fallback.
    pub fn supports_display_capture(&self) -> bool {
        self.supports_input("ddagrab") || self.supports_input("gdigrab")
    }

    /// A window is captured from the visible physical desktop bounds resolved by the overlay.
    pub fn supports_window_capture(&self) -> bool {
        self.supports_display_capture()
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

const MAX_VERSION_LABEL_CHARS: usize = 32;

/// Removes banner noise and bounds the version text before it reaches a fixed-width UI row.
fn compact_version_label(version: &str) -> String {
    let first_line = version.lines().next().unwrap_or_default().trim();
    let label = first_line
        .split("Copyright")
        .next()
        .unwrap_or(first_line)
        .trim();
    if label.is_empty() {
        return "unknown".to_owned();
    }
    let mut compact = label
        .chars()
        .take(MAX_VERSION_LABEL_CHARS)
        .collect::<String>();
    if label.chars().count() > MAX_VERSION_LABEL_CHARS {
        compact.push_str("...");
    }
    compact
}

/// A physical-pixel video source selected before an FFmpeg process is started.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecordingTarget {
    /// A complete display represented by its physical desktop bounds.
    Display { bounds: PhysicalRect },
    /// A visible top-level Windows window identified by title and captured through its desktop
    /// bounds. Keeping the bounds avoids black frames from title-based GDI capture of GPU-backed
    /// windows while retaining a stable label for lifecycle feedback.
    Window { title: String, bounds: PhysicalRect },
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

/// Lists local audio inputs that can be selected without opening a recording session.
///
/// FFmpeg reports DirectShow devices on stderr and exits unsuccessfully after listing them,
/// so this intentionally does not use the normal successful-probe path.
pub fn discover_audio_sources() -> io::Result<Vec<AudioSource>> {
    let capabilities = discover()?;
    let mut sources = Vec::new();
    if capabilities.supports_microphone_capture() {
        let output = run_listing_probe(capabilities.executable(), DSHOW_AUDIO_DEVICE_ARGUMENTS)?;
        sources.extend(
            parse_dshow_audio_devices(&output)
                .into_iter()
                .map(|device| AudioSource::Microphone { device }),
        );
    }
    if capabilities.supports_system_audio_capture() {
        sources.push(AudioSource::SystemAudio {
            device: "default".to_owned(),
        });
    }
    Ok(sources)
}

/// Explicit local audio selection loaded without probing or opening a device.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordingAudioConfig {
    source: Option<AudioSource>,
}

impl RecordingAudioConfig {
    /// Reads one optional audio source. Configuring both inputs is rejected instead of mixing
    /// unrelated capture backends without an explicit product decision.
    pub fn from_environment() -> io::Result<Self> {
        let microphone = non_empty_environment(MICROPHONE_DEVICE_ENV);
        let system_audio = non_empty_environment(SYSTEM_AUDIO_DEVICE_ENV);
        let source = audio_source_from_config(microphone, system_audio)?;
        Ok(Self { source })
    }

    pub fn source(&self) -> Option<&AudioSource> {
        self.source.as_ref()
    }
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

/// Events emitted by a live recording worker for a UI or other caller to observe.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecordingEvent {
    Started,
    Paused,
    Resumed,
    Progress(RecordingProgress),
    Finished { output: PathBuf },
    Failed { message: String },
}

/// Handle for an isolated recording worker. Dropping it requests a normal FFmpeg stop.
pub struct RecordingControl {
    commands: Arc<RecordingCommands>,
    events: async_channel::Receiver<RecordingEvent>,
    target: RecordingTarget,
}

impl RecordingControl {
    pub fn request_stop(&self) -> io::Result<()> {
        self.commands.request_stop()
    }

    pub fn set_paused(&self, paused: bool) -> io::Result<()> {
        self.commands.set_paused(paused)
    }

    pub fn events(&self) -> async_channel::Receiver<RecordingEvent> {
        self.events.clone()
    }

    /// The immutable capture target accepted when this worker was started.
    pub const fn target(&self) -> &RecordingTarget {
        &self.target
    }
}

impl Drop for RecordingControl {
    fn drop(&mut self) {
        let _ = self.commands.request_stop();
    }
}

const PAUSE_UNCHANGED: u8 = 0;
const PAUSE_RESUME: u8 = 1;
const PAUSE_PAUSE: u8 = 2;

/// Coalesces UI control changes into bounded shared state while keeping stop permanently visible.
struct RecordingCommands {
    running: AtomicBool,
    stop_requested: AtomicBool,
    pause_request: AtomicU8,
}

impl RecordingCommands {
    fn new() -> Self {
        Self {
            running: AtomicBool::new(true),
            stop_requested: AtomicBool::new(false),
            pause_request: AtomicU8::new(PAUSE_UNCHANGED),
        }
    }

    fn request_stop(&self) -> io::Result<()> {
        self.require_running()?;
        self.stop_requested.store(true, Ordering::Release);
        Ok(())
    }

    fn set_paused(&self, paused: bool) -> io::Result<()> {
        self.require_running()?;
        self.pause_request.store(
            if paused { PAUSE_PAUSE } else { PAUSE_RESUME },
            Ordering::Release,
        );
        Ok(())
    }

    fn stop_requested(&self) -> bool {
        self.stop_requested.load(Ordering::Acquire)
    }

    fn take_pause_request(&self) -> Option<bool> {
        match self.pause_request.swap(PAUSE_UNCHANGED, Ordering::AcqRel) {
            PAUSE_PAUSE => Some(true),
            PAUSE_RESUME => Some(false),
            _ => None,
        }
    }

    fn require_running(&self) -> io::Result<()> {
        if self.running.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "recording worker is no longer running",
            ))
        }
    }
}

struct RecordingWorkerLifecycle(Arc<RecordingCommands>);

impl Drop for RecordingWorkerLifecycle {
    fn drop(&mut self) {
        self.0.running.store(false, Ordering::Release);
    }
}

/// Launches the FFmpeg process on a dedicated worker and returns non-blocking lifecycle events.
pub fn start_recording(
    capabilities: FfmpegCapabilities,
    request: RecordingRequest,
) -> io::Result<RecordingControl> {
    let command = build_recording_command(&capabilities, &request)?;
    let target = request.target.clone();
    let commands = Arc::new(RecordingCommands::new());
    let (event_tx, event_rx) = async_channel::bounded(32);
    let worker_commands = Arc::clone(&commands);
    std::thread::Builder::new()
        .name("flash-shot-recording".to_owned())
        .spawn(move || recording_worker(command, request.output, worker_commands, event_tx))
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("could not start recording worker: {error}"),
            )
        })?;
    Ok(RecordingControl {
        commands,
        events: event_rx,
        target,
    })
}

/// Runs one recording process, turning process lifecycle changes into bounded UI events.
///
/// The worker owns all blocking waits and reports every terminal process outcome so the caller
/// can return to a retryable state without relying on a worker panic.
fn recording_worker(
    command: FfmpegCommand,
    output: PathBuf,
    commands: Arc<RecordingCommands>,
    events: async_channel::Sender<RecordingEvent>,
) {
    let _lifecycle = RecordingWorkerLifecycle(Arc::clone(&commands));
    let mut process = match RecordingProcess::start(command) {
        Ok(process) => process,
        Err(error) => {
            let _ = events.try_send(RecordingEvent::Failed {
                message: error.to_string(),
            });
            return;
        }
    };
    if events.try_send(RecordingEvent::Started).is_err() {
        return;
    }
    let mut last_progress = RecordingProgress::default();
    let mut paused = false;
    loop {
        if commands.stop_requested() {
            match process.stop_gracefully(GRACEFUL_STOP_TIMEOUT) {
                Ok(exit) if exit.success => {
                    if let Err(error) = emit_final_progress(&process, &mut last_progress, &events) {
                        let _ = events.try_send(RecordingEvent::Failed {
                            message: error.to_string(),
                        });
                        return;
                    }
                    let _ = events.try_send(RecordingEvent::Finished { output });
                }
                Ok(_) => {
                    unreachable!("successful recording exits are represented by RecordingExit")
                }
                Err(error) => {
                    let _ = events.try_send(RecordingEvent::Failed {
                        message: error.to_string(),
                    });
                }
            }
            return;
        }
        if let Some(requested_pause) = commands.take_pause_request()
            && requested_pause != paused
        {
            match process.set_paused(requested_pause) {
                Ok(()) => {
                    paused = requested_pause;
                    let _ = events.try_send(if paused {
                        RecordingEvent::Paused
                    } else {
                        RecordingEvent::Resumed
                    });
                }
                Err(error) => {
                    let _ = events.try_send(RecordingEvent::Failed {
                        message: error.to_string(),
                    });
                    return;
                }
            }
        }
        match process.try_wait_for_exit() {
            Ok(Some(exit)) if exit.success => {
                if let Err(error) = emit_final_progress(&process, &mut last_progress, &events) {
                    let _ = events.try_send(RecordingEvent::Failed {
                        message: error.to_string(),
                    });
                    return;
                }
                let _ = events.try_send(RecordingEvent::Finished { output });
                return;
            }
            Ok(Some(exit)) => {
                let message = if exit.diagnostic.is_empty() {
                    "FFmpeg exited unsuccessfully".to_owned()
                } else {
                    format!("FFmpeg exited unsuccessfully: {}", exit.diagnostic)
                };
                let _ = events.try_send(RecordingEvent::Failed { message });
                return;
            }
            Err(error) => {
                let _ = events.try_send(RecordingEvent::Failed {
                    message: error.to_string(),
                });
                return;
            }
            Ok(None) => {}
        }
        match process.progress() {
            Ok(progress) if progress != last_progress => {
                last_progress = progress;
                let _ = events.try_send(RecordingEvent::Progress(progress));
            }
            Ok(_) => {}
            Err(error) => {
                let _ = events.try_send(RecordingEvent::Failed {
                    message: error.to_string(),
                });
                return;
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// Publishes the last FFmpeg progress snapshot after the child has been fully reaped.
fn emit_final_progress(
    process: &RecordingProcess,
    last_progress: &mut RecordingProgress,
    events: &async_channel::Sender<RecordingEvent>,
) -> io::Result<()> {
    let progress = process.progress()?;
    if progress != *last_progress {
        *last_progress = progress;
        let _ = events.try_send(RecordingEvent::Progress(progress));
    }
    Ok(())
}

/// The latest machine-readable progress information emitted by FFmpeg's `-progress` pipe.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RecordingProgress {
    /// Encoded output timestamp in microseconds, when FFmpeg has reported one.
    pub output_time_us: Option<u64>,
    /// Total encoded video frames, when reported by FFmpeg.
    pub frame: Option<u64>,
    /// `true` only after FFmpeg emits `progress=end`.
    pub finished: bool,
}

/// Incrementally consumes line-oriented FFmpeg `-progress pipe:1` output.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProgressParser {
    pending: String,
    progress: RecordingProgress,
}

impl ProgressParser {
    pub fn progress(&self) -> RecordingProgress {
        self.progress
    }

    /// Pushes any bytes received from stdout and returns a snapshot when a progress block ends.
    pub fn push(&mut self, bytes: &[u8]) -> Option<RecordingProgress> {
        self.pending.push_str(&String::from_utf8_lossy(bytes));
        let mut completed = None;
        while let Some(newline) = self.pending.find('\n') {
            let line = self.pending[..newline].trim_end_matches('\r').to_owned();
            self.pending.drain(..=newline);
            if let Some(progress) = self.consume_line(&line) {
                completed = Some(progress);
            }
        }
        completed
    }

    /// Treats any partial final line as a complete line after stdout closes.
    pub fn finish(&mut self) -> Option<RecordingProgress> {
        if self.pending.is_empty() {
            return None;
        }
        let line = std::mem::take(&mut self.pending);
        self.consume_line(line.trim_end_matches('\r'))
    }

    fn consume_line(&mut self, line: &str) -> Option<RecordingProgress> {
        let (key, value) = line.split_once('=')?;
        let key = key.trim();
        let value = value.trim();
        match key {
            "out_time_us" => self.progress.output_time_us = value.parse().ok(),
            "frame" => self.progress.frame = value.parse().ok(),
            "progress" if value == "end" => {
                self.progress.finished = true;
                return Some(self.progress);
            }
            "progress" if value == "continue" => return Some(self.progress),
            _ => {}
        }
        None
    }
}

/// Owns a single FFmpeg child process and guarantees cleanup when the owner is dropped.
pub struct RecordingProcess {
    child: Option<Child>,
    process_group: ProcessGroup,
    stdin: Option<ChildStdin>,
    progress: Arc<Mutex<RecordingProgress>>,
    stdout_reader: Option<JoinHandle<io::Result<RecordingProgress>>>,
    stderr_reader: Option<JoinHandle<io::Result<Vec<u8>>>>,
}

/// Owns the child and Job Object while startup is still fallible.
///
/// Any setup error before the recording process is fully constructed drops this guard, which
/// terminates and reaps the child instead of leaving an untracked FFmpeg process behind.
struct StartupChildCleanup {
    child: Option<Child>,
    process_group: Option<ProcessGroup>,
}

impl StartupChildCleanup {
    fn new(child: Child, process_group: ProcessGroup) -> Self {
        Self {
            child: Some(child),
            process_group: Some(process_group),
        }
    }

    fn assign(&self) -> io::Result<()> {
        let process_group = self.process_group.as_ref().ok_or_else(|| {
            io::Error::other("recording Job Object is unavailable during startup")
        })?;
        let child = self
            .child
            .as_ref()
            .ok_or_else(|| io::Error::other("recording child is unavailable during startup"))?;
        process_group.assign(child)
    }

    /// Returns the child for taking its three pipes while this guard remains armed.
    fn child_mut(&mut self) -> &mut Child {
        self.child
            .as_mut()
            .expect("startup cleanup guard must own its child")
    }

    /// Disarms cleanup only after all startup-owned resources have been transferred successfully.
    fn into_parts(mut self) -> (Child, ProcessGroup) {
        (
            self.child
                .take()
                .expect("startup cleanup guard must own its child"),
            self.process_group
                .take()
                .expect("startup cleanup guard must own its Job Object"),
        )
    }

    /// Terminates and reaps the child before an early startup error is returned.
    fn terminate_and_reap(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        if let Some(process_group) = self.process_group.as_ref() {
            let _ = process_group.terminate();
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

impl Drop for StartupChildCleanup {
    fn drop(&mut self) {
        self.terminate_and_reap();
    }
}

impl RecordingProcess {
    /// Starts FFmpeg with piped control input and continuously drained stderr.
    pub fn start(command: FfmpegCommand) -> io::Result<Self> {
        let process_group = ProcessGroup::create()?;
        let child = command
            .into_command()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                io::Error::new(error.kind(), format!("could not start FFmpeg: {error}"))
            })?;
        let mut startup = StartupChildCleanup::new(child, process_group);
        startup.assign()?;
        let stdin = startup.child_mut().stdin.take().ok_or_else(|| {
            io::Error::other("FFmpeg control input pipe was not available after startup")
        })?;
        let stdout = startup.child_mut().stdout.take().ok_or_else(|| {
            io::Error::other("FFmpeg progress output pipe was not available after startup")
        })?;
        let stderr = startup.child_mut().stderr.take().ok_or_else(|| {
            io::Error::other("FFmpeg diagnostic pipe was not available after startup")
        })?;
        let progress = Arc::new(Mutex::new(RecordingProgress::default()));
        let progress_target = Arc::clone(&progress);
        let stdout_reader = match thread::Builder::new()
            .name("flash-shot-recording-progress".to_owned())
            .spawn(move || read_progress(stdout, progress_target))
        {
            Ok(reader) => reader,
            Err(error) => {
                startup.terminate_and_reap();
                return Err(io::Error::new(
                    error.kind(),
                    format!("could not start FFmpeg progress reader: {error}"),
                ));
            }
        };
        let stderr_reader = match thread::Builder::new()
            .name("flash-shot-recording-diagnostics".to_owned())
            .spawn(move || read_bounded_diagnostics(stderr))
        {
            Ok(reader) => reader,
            Err(error) => {
                startup.terminate_and_reap();
                let _ = stdout_reader.join();
                return Err(io::Error::new(
                    error.kind(),
                    format!("could not start FFmpeg diagnostic reader: {error}"),
                ));
            }
        };
        let (child, process_group) = startup.into_parts();
        Ok(Self {
            child: Some(child),
            process_group,
            stdin: Some(stdin),
            progress,
            stdout_reader: Some(stdout_reader),
            stderr_reader: Some(stderr_reader),
        })
    }

    /// Returns the latest parsed FFmpeg `-progress` snapshot without blocking on process output.
    pub fn progress(&self) -> io::Result<RecordingProgress> {
        self.progress
            .lock()
            .map(|progress| *progress)
            .map_err(|_| io::Error::other("FFmpeg progress state lock poisoned"))
    }

    /// Suspends or resumes all FFmpeg threads through the platform process boundary.
    pub fn set_paused(&self, paused: bool) -> io::Result<()> {
        let child = self
            .child
            .as_ref()
            .ok_or_else(|| io::Error::other("recording process has already been reaped"))?;
        set_paused(child.id(), paused)
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

    /// Non-blockingly observes a naturally exited recording process.
    pub fn try_wait_for_exit(&mut self) -> io::Result<Option<RecordingExit>> {
        let Some(child) = self.child.as_mut() else {
            return Err(io::Error::other(
                "recording process has already been reaped",
            ));
        };
        let Some(status) = child.try_wait()? else {
            return Ok(None);
        };
        self.child.take();
        self.stdin.take();
        self.complete(status).map(Some)
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
                let _ = self.process_group.terminate();
                let _ = child.kill();
                let _ = child.wait();
                self.join_progress()?;
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
        self.join_progress()?;
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

    fn join_progress(&mut self) -> io::Result<()> {
        let Some(reader) = self.stdout_reader.take() else {
            return Ok(());
        };
        let progress = reader
            .join()
            .map_err(|_| io::Error::other("FFmpeg progress reader panicked"))??;
        let mut current = self
            .progress
            .lock()
            .map_err(|_| io::Error::other("FFmpeg progress state lock poisoned"))?;
        *current = progress;
        Ok(())
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
            let _ = self.process_group.terminate();
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
        if let Some(reader) = self.stdout_reader.take() {
            let _ = reader.join();
        }
    }
}

fn read_progress(
    mut stdout: impl Read,
    progress: Arc<Mutex<RecordingProgress>>,
) -> io::Result<RecordingProgress> {
    let mut parser = ProgressParser::default();
    let mut buffer = [0_u8; 4096];
    loop {
        let read = stdout.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        if let Some(snapshot) = parser.push(&buffer[..read]) {
            update_progress(&progress, snapshot)?;
        }
    }
    if let Some(snapshot) = parser.finish() {
        update_progress(&progress, snapshot)?;
    }
    Ok(parser.progress())
}

fn update_progress(
    target: &Mutex<RecordingProgress>,
    progress: RecordingProgress,
) -> io::Result<()> {
    *target
        .lock()
        .map_err(|_| io::Error::other("FFmpeg progress state lock poisoned"))? = progress;
    Ok(())
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
        // Retain the latest stderr bytes because FFmpeg writes the actionable failure after
        // startup banners. Keeping only the prefix can hide the real reason a recording stopped.
        if read >= MAX_DIAGNOSTIC_BYTES {
            retained.clear();
            retained.extend_from_slice(&buffer[read - MAX_DIAGNOSTIC_BYTES..read]);
            continue;
        }
        let overflow = retained
            .len()
            .saturating_add(read)
            .saturating_sub(MAX_DIAGNOSTIC_BYTES);
        if overflow > 0 {
            retained.drain(..overflow);
        }
        retained.extend_from_slice(&buffer[..read]);
    }
    Ok(retained)
}

fn diagnostic_suffix(diagnostic: &str) -> String {
    last_diagnostic_line(diagnostic)
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
    let bounds = match &request.target {
        RecordingTarget::Display { bounds }
        | RecordingTarget::Window { bounds, .. }
        | RecordingTarget::Region { bounds } => *bounds,
    };
    let mut arguments = vec![
        OsString::from("-hide_banner"),
        OsString::from("-n"),
        OsString::from("-nostats"),
        OsString::from("-progress"),
        OsString::from("pipe:1"),
    ];
    match &request.target {
        RecordingTarget::Display { bounds } | RecordingTarget::Region { bounds } => {
            let input = desktop_input(capabilities, *bounds, request.frame_rate)?;
            arguments.extend(input);
        }
        RecordingTarget::Window { bounds, .. } => {
            let input = desktop_input(capabilities, *bounds, request.frame_rate)?;
            arguments.extend(input);
        }
    }
    let has_audio = if let Some(audio) = &request.audio {
        arguments.extend(audio_input(capabilities, audio)?);
        true
    } else {
        false
    };
    if h264_padding_required(bounds) {
        arguments.extend([
            OsString::from("-vf"),
            OsString::from("pad=ceil(iw/2)*2:ceil(ih/2)*2"),
        ]);
    }
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
    validate_capture_bounds(bounds)?;
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

/// Rejects invalid desktop rectangles before their signed coordinates reach FFmpeg arguments.
fn validate_capture_bounds(bounds: PhysicalRect) -> io::Result<()> {
    let width = i64::from(bounds.right) - i64::from(bounds.left);
    let height = i64::from(bounds.bottom) - i64::from(bounds.top);
    if width < 2 || height < 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "recording bounds must use increasing coordinates and be at least 2x2 physical pixels",
        ));
    }
    Ok(())
}

/// Pads odd desktop extents instead of dropping the user's final row or column for yuv420p.
fn h264_padding_required(bounds: PhysicalRect) -> bool {
    let width = i64::from(bounds.right) - i64::from(bounds.left);
    let height = i64::from(bounds.bottom) - i64::from(bounds.top);
    width % 2 != 0 || height % 2 != 0
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
        RecordingTarget::Display { bounds }
        | RecordingTarget::Region { bounds }
        | RecordingTarget::Window { bounds, .. } => {
            validate_capture_bounds(*bounds)?;
        }
    }
    if let RecordingTarget::Window { title, .. } = &request.target
        && title.trim().is_empty()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "recording window title must not be empty",
        ));
    }
    if request
        .audio
        .as_ref()
        .is_some_and(|audio| audio_device_name(audio).trim().is_empty())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "recording audio device name must not be empty",
        ));
    }
    Ok(())
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

fn non_empty_environment(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

fn audio_source_from_config(
    microphone: Option<String>,
    system_audio: Option<String>,
) -> io::Result<Option<AudioSource>> {
    match (microphone, system_audio) {
        (Some(_), Some(_)) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "configure either a recording microphone or system audio device, not both",
        )),
        (Some(device), None) => Ok(Some(AudioSource::Microphone { device })),
        (None, Some(device)) => Ok(Some(AudioSource::SystemAudio { device })),
        (None, None) => Ok(None),
    }
}

fn run_probe(executable: &OsStr, arguments: &[&str]) -> io::Result<Output> {
    let output = run_probe_output(executable, arguments)?;
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

fn run_listing_probe(executable: &Path, arguments: &[&str]) -> io::Result<String> {
    Ok(combined_output(&run_probe_output(
        executable.as_os_str(),
        arguments,
    )?))
}

fn run_probe_output(executable: &OsStr, arguments: &[&str]) -> io::Result<Output> {
    let mut child = Command::new(executable)
        .args(arguments)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "could not start FFmpeg '{}': {error}",
                    executable.to_string_lossy()
                ),
            )
        })?;
    wait_for_probe_child(&mut child, FFMPEG_PROBE_TIMEOUT)?;
    child.wait_with_output()
}

/// Polls a read-only FFmpeg probe and kills it if discovery exceeds its time budget.
fn wait_for_probe_child(child: &mut Child, timeout: Duration) -> io::Result<()> {
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
                        "FFmpeg probe exceeded {} ms and was terminated",
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
            .filter(|name| !name.is_empty() && *name != "=")
        {
            let input = input.to_ascii_lowercase();
            if !inputs.contains(&input) {
                inputs.push(input);
            }
        }
    }
    inputs
}

fn parse_dshow_audio_devices(output: &str) -> Vec<String> {
    let mut devices = Vec::new();
    let mut in_audio_section = false;
    for line in output.lines() {
        let line = line.trim();
        if line.contains("DirectShow audio devices") {
            in_audio_section = true;
            continue;
        }
        if line.contains("DirectShow video devices") {
            in_audio_section = false;
            continue;
        }
        if !in_audio_section || line.contains("Alternative name") {
            continue;
        }
        let Some((_, quoted)) = line.split_once('"') else {
            continue;
        };
        let Some((name, _)) = quoted.split_once('"') else {
            continue;
        };
        let name = name.trim();
        if !name.is_empty() && !devices.iter().any(|device| device == name) {
            devices.push(name.to_owned());
        }
    }
    devices
}

fn first_diagnostic_line(output: &str) -> Option<&str> {
    output.lines().map(str::trim).find(|line| !line.is_empty())
}

/// Selects the final non-empty FFmpeg line, where encoder and output failures are reported.
fn last_diagnostic_line(output: &str) -> Option<&str> {
    output
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
}

#[cfg(test)]
mod tests {
    use super::{
        AudioSource, DEVICE_ARGUMENTS, FFMPEG_PROBE_TIMEOUT, FORMAT_ARGUMENTS, FfmpegCapabilities,
        FfmpegCommand, GRACEFUL_STOP_TIMEOUT, MAX_DIAGNOSTIC_BYTES, ProgressParser,
        RecordingAudioConfig, RecordingCommands, RecordingEvent, RecordingProcess,
        RecordingProgress, RecordingRequest, RecordingSession, RecordingState, RecordingTarget,
        StartupChildCleanup, VERSION_ARGUMENTS, audio_source_from_config, build_recording_command,
        diagnostic_suffix, executable_from, first_diagnostic_line, graceful_stop_input,
        last_diagnostic_line, parse_dshow_audio_devices, parse_input_formats, parse_version,
        read_bounded_diagnostics, recording_worker, wait_for_probe_child,
    };
    use crate::domain::geometry::PhysicalRect;
    use std::{
        ffi::OsString,
        io::Cursor,
        path::PathBuf,
        process::{Command, Stdio},
        sync::Arc,
        time::Duration,
    };

    const FORMATS: &str = "\
 File formats:\n\
  D  =                 Demuxing supported\n\
  D  ddagrab          Windows Desktop Duplication API\n\
  D  gdigrab          GDI API Windows frame grabber\n\
  D  dshow            DirectShow capture\n\
 DE png_pipe          PNG pipe\n\
";

    const DSHOW_DEVICES: &str = "\
[dshow @ 000001] DirectShow video devices (some header)\n\
[dshow @ 000001]  \"Camera\"\n\
[dshow @ 000001] DirectShow audio devices\n\
[dshow @ 000001]  \"Microphone (USB Audio)\"\n\
[dshow @ 000001]     Alternative name \"@device_cm_{abc}\"\n\
[dshow @ 000001]  \"Microphone (USB Audio)\"\n\
[dshow @ 000001]  \"Line In\"\n\
";

    #[test]
    fn ffmpeg_probes_have_a_bounded_timeout() {
        assert_eq!(FFMPEG_PROBE_TIMEOUT, Duration::from_secs(10));
    }

    #[test]
    fn stuck_ffmpeg_probes_are_terminated_at_the_deadline() {
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

        let error = wait_for_probe_child(&mut child, Duration::from_millis(100)).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(child.try_wait().unwrap().is_some());
    }

    #[test]
    fn probe_arguments_are_read_only_and_hide_banner_noise() {
        assert_eq!(VERSION_ARGUMENTS, ["-hide_banner", "-version"]);
        assert_eq!(FORMAT_ARGUMENTS, ["-hide_banner", "-formats"]);
        assert_eq!(DEVICE_ARGUMENTS, ["-hide_banner", "-devices"]);
    }

    #[test]
    fn recording_controls_coalesce_pause_changes_without_losing_stop() {
        let commands = RecordingCommands::new();

        commands.set_paused(true).unwrap();
        commands.set_paused(false).unwrap();
        commands.request_stop().unwrap();

        assert_eq!(commands.take_pause_request(), Some(false));
        assert_eq!(commands.take_pause_request(), None);
        assert!(commands.stop_requested());
    }

    #[test]
    fn recording_controls_reject_changes_after_the_worker_finishes() {
        let commands = RecordingCommands::new();
        commands
            .running
            .store(false, std::sync::atomic::Ordering::Release);

        assert_eq!(
            commands.request_stop().unwrap_err().kind(),
            std::io::ErrorKind::BrokenPipe
        );
        assert_eq!(
            commands.set_paused(true).unwrap_err().kind(),
            std::io::ErrorKind::BrokenPipe
        );
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
    fn parser_extracts_unique_audio_device_names_from_dshow_listing() {
        assert_eq!(
            parse_dshow_audio_devices(DSHOW_DEVICES),
            ["Microphone (USB Audio)", "Line In"]
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
        assert_eq!(
            last_diagnostic_line("capturing desktop\n  encoder failed\n"),
            Some("encoder failed")
        );
    }

    #[test]
    fn version_label_removes_banner_suffix_and_limits_width() {
        let capabilities = FfmpegCapabilities {
            executable: PathBuf::from("ffmpeg"),
            version: "9.0-full_build-www.gyan.dev Copyright (c) 2000-2026 the FFmpeg developers\nmore diagnostics".to_owned(),
            input_formats: Vec::new(),
        };

        assert_eq!(capabilities.version_label(), "9.0-full_build-www.gyan.dev");

        let long = FfmpegCapabilities {
            executable: PathBuf::from("ffmpeg"),
            version: "123456789012345678901234567890123456".to_owned(),
            input_formats: Vec::new(),
        };
        assert_eq!(long.version_label(), "12345678901234567890123456789012...");

        let empty = FfmpegCapabilities {
            executable: PathBuf::from("ffmpeg"),
            version: "\nCopyright only".to_owned(),
            input_formats: Vec::new(),
        };
        assert_eq!(empty.version_label(), "unknown");
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
                "-n",
                "-nostats",
                "-progress",
                "pipe:1",
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
    fn recording_command_never_overwrites_an_existing_output_file() {
        let command = build_recording_command(&capabilities(), &region_request()).unwrap();

        assert!(command.arguments().contains(&OsString::from("-n")));
        assert!(!command.arguments().contains(&OsString::from("-y")));
    }

    #[test]
    fn window_recording_uses_visible_desktop_bounds_without_a_shell() {
        let command = build_recording_command(
            &capabilities(),
            &RecordingRequest {
                target: RecordingTarget::Window {
                    title: "Editor & terminal".to_owned(),
                    bounds: PhysicalRect {
                        left: 40,
                        top: 80,
                        right: 1640,
                        bottom: 980,
                    },
                },
                audio: None,
                frame_rate: 30,
                output: PathBuf::from("window.mp4"),
            },
        )
        .unwrap();

        assert!(
            command
                .arguments()
                .windows(2)
                .any(|pair| { pair == [OsString::from("-i"), OsString::from("desktop")] })
        );
        assert!(
            command
                .arguments()
                .windows(2)
                .any(|pair| { pair == [OsString::from("-offset_x"), OsString::from("40")] })
        );
        assert!(
            command.arguments().windows(2).any(|pair| {
                pair == [OsString::from("-video_size"), OsString::from("1600x900")]
            })
        );
        assert!(
            !command
                .arguments()
                .iter()
                .any(|argument| argument == &OsString::from("title=Editor & terminal"))
        );
    }

    #[test]
    fn recording_command_pads_odd_bounds_without_dropping_selected_pixels() {
        let command = build_recording_command(
            &capabilities(),
            &RecordingRequest {
                target: RecordingTarget::Region {
                    bounds: PhysicalRect {
                        left: 10,
                        top: 20,
                        right: 651,
                        bottom: 381,
                    },
                },
                audio: None,
                frame_rate: 30,
                output: PathBuf::from("odd-region.mp4"),
            },
        )
        .unwrap();

        assert!(
            command
                .arguments()
                .windows(2)
                .any(|pair| { pair == [OsString::from("-video_size"), OsString::from("641x361")] })
        );
        assert!(command.arguments().windows(2).any(|pair| {
            pair == [
                OsString::from("-vf"),
                OsString::from("pad=ceil(iw/2)*2:ceil(ih/2)*2"),
            ]
        }));
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
                    RecordingTarget::Region {
                        bounds: PhysicalRect {
                            left: 640,
                            top: 360,
                            right: 0,
                            bottom: 0,
                        },
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
                        bounds: PhysicalRect {
                            left: 0,
                            top: 0,
                            right: 640,
                            bottom: 360,
                        },
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
    fn audio_configuration_is_opt_in_and_accepts_one_explicit_source() {
        assert_eq!(
            audio_source_from_config(None, None).unwrap(),
            RecordingAudioConfig { source: None }.source().cloned()
        );
        assert_eq!(
            audio_source_from_config(Some("USB Mic".to_owned()), None).unwrap(),
            Some(AudioSource::Microphone {
                device: "USB Mic".to_owned(),
            })
        );
        assert_eq!(
            audio_source_from_config(None, Some("default".to_owned())).unwrap(),
            Some(AudioSource::SystemAudio {
                device: "default".to_owned(),
            })
        );
        assert!(
            audio_source_from_config(Some("mic".to_owned()), Some("default".to_owned())).is_err()
        );
    }

    #[test]
    fn progress_parser_combines_fragmented_ffmpeg_progress_blocks() {
        let mut parser = ProgressParser::default();

        assert_eq!(parser.push(b"frame=12\nout_time_us=50"), None);
        assert_eq!(
            parser.push(b"0000\nprogress=continue\n"),
            Some(super::RecordingProgress {
                frame: Some(12),
                output_time_us: Some(500_000),
                finished: false,
            })
        );
        assert_eq!(
            parser.push(b"progress=end\n"),
            Some(super::RecordingProgress {
                frame: Some(12),
                output_time_us: Some(500_000),
                finished: true,
            })
        );
    }

    #[test]
    fn progress_parser_ignores_invalid_values_and_flushes_a_final_partial_line() {
        let mut parser = ProgressParser::default();

        assert_eq!(
            parser.push(b"frame=unknown\nprogress=continue\n"),
            Some(Default::default())
        );
        assert_eq!(parser.push(b"out_time_us=10"), None);
        assert_eq!(
            parser.finish(),
            None,
            "a final metric alone is not a complete progress block"
        );
        assert_eq!(parser.progress().output_time_us, Some(10));
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
    fn diagnostics_keep_the_latest_actionable_failure_line() {
        let diagnostics = read_bounded_diagnostics(Cursor::new(
            b"capturing whole desktop\nfailed to open output file",
        ))
        .unwrap();
        let diagnostics = String::from_utf8(diagnostics).unwrap();

        assert_eq!(
            diagnostic_suffix(&diagnostics),
            ": failed to open output file"
        );
    }

    #[test]
    fn diagnostic_buffer_discards_old_banner_bytes_before_recent_errors() {
        let mut output = vec![b'a'; MAX_DIAGNOSTIC_BYTES + 16];
        output.extend_from_slice(b"\nlatest error");

        let diagnostics = read_bounded_diagnostics(Cursor::new(output)).unwrap();

        assert_eq!(diagnostics.len(), MAX_DIAGNOSTIC_BYTES);
        assert!(String::from_utf8_lossy(&diagnostics).ends_with("latest error"));
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

    #[cfg(windows)]
    #[test]
    fn recording_worker_reports_an_abnormal_exit_without_panicking() {
        let command = FfmpegCommand {
            executable: PathBuf::from("cmd.exe"),
            arguments: ["/C", "echo encoder failed 1>&2 & exit /b 7"]
                .map(OsString::from)
                .into(),
        };
        let commands = Arc::new(RecordingCommands::new());
        let (events, received) = async_channel::bounded(4);

        recording_worker(
            command,
            PathBuf::from("failed.mp4"),
            Arc::clone(&commands),
            events,
        );

        assert!(matches!(received.try_recv(), Ok(RecordingEvent::Started)));
        let failure = received.try_recv().unwrap();
        match failure {
            RecordingEvent::Failed { message } => assert!(message.contains("encoder failed")),
            other => panic!("expected a failure event, got {other:?}"),
        }
        assert!(!commands.running.load(std::sync::atomic::Ordering::Acquire));
    }

    #[test]
    fn recording_worker_reports_start_failure_and_releases_its_lifecycle() {
        // A process-start error must become a bounded failure event instead of leaving the
        // recording workflow marked as running without a child that can be stopped.
        let command = FfmpegCommand {
            executable: PathBuf::from("missing\0ffmpeg"),
            arguments: Vec::new(),
        };
        let commands = Arc::new(RecordingCommands::new());
        let (events, received) = async_channel::bounded(1);

        recording_worker(
            command,
            PathBuf::from("failed-start.mp4"),
            Arc::clone(&commands),
            events,
        );

        match received.try_recv().unwrap() {
            RecordingEvent::Failed { message } => assert!(!message.is_empty()),
            other => panic!("expected a start failure event, got {other:?}"),
        }
        assert!(received.try_recv().is_err());
        assert!(!commands.running.load(std::sync::atomic::Ordering::Acquire));
    }

    #[cfg(windows)]
    #[test]
    fn process_stop_timeout_terminates_and_reaps_the_child() {
        let command = FfmpegCommand {
            executable: PathBuf::from("cmd.exe"),
            arguments: ["/C", "ping -n 10 127.0.0.1 > nul"]
                .map(OsString::from)
                .into(),
        };
        let mut process = RecordingProcess::start(command).unwrap();

        let error = process
            .stop_gracefully(Duration::from_millis(100))
            .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(error.to_string().contains("was terminated"));
        assert!(process.child.is_none());
        assert!(process.stdin.is_none());
        assert!(process.stdout_reader.is_none());
        assert!(process.stderr_reader.is_none());
        assert!(process.try_wait_for_exit().is_err());
    }

    #[cfg(windows)]
    #[test]
    fn startup_cleanup_terminates_a_child_before_process_construction() {
        let marker = std::env::temp_dir().join(format!(
            "flash-shot-recording-startup-{}.txt",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&marker);
        let process_group = super::ProcessGroup::create().unwrap();
        let command_line = format!(
            r#"ping -n 10 127.0.0.1 > nul & echo leaked > "{}""#,
            marker.display()
        );
        let child = Command::new("cmd.exe")
            .args(["/C", command_line.as_str()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let startup = StartupChildCleanup::new(child, process_group);
        startup.assign().unwrap();

        drop(startup);
        std::thread::sleep(Duration::from_millis(500));
        assert!(!marker.exists(), "startup cleanup left a child running");
        let _ = std::fs::remove_file(marker);
    }

    #[cfg(windows)]
    #[test]
    fn process_parses_progress_while_reaping_stdout() {
        let command = FfmpegCommand {
            executable: PathBuf::from("cmd.exe"),
            arguments: [
                "/C",
                "echo frame=5 & echo out_time_us=125000 & echo progress=end",
            ]
            .map(OsString::from)
            .into(),
        };
        let mut process = RecordingProcess::start(command).unwrap();
        process.wait_for_exit().unwrap();

        assert_eq!(
            process.progress().unwrap(),
            RecordingProgress {
                frame: Some(5),
                output_time_us: Some(125_000),
                finished: true,
            }
        );
    }

    #[cfg(windows)]
    #[test]
    fn process_can_pause_and_resume_its_ffmpeg_threads() {
        let command = FfmpegCommand {
            executable: PathBuf::from("cmd.exe"),
            arguments: ["/C", "ping -n 3 127.0.0.1 > nul"]
                .map(OsString::from)
                .into(),
        };
        let mut process = RecordingProcess::start(command).unwrap();

        process.set_paused(true).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        assert!(process.try_wait_for_exit().unwrap().is_none());
        process.set_paused(false).unwrap();
        assert!(process.wait_for_exit().unwrap().success);
    }

    #[cfg(windows)]
    #[test]
    fn dropping_a_process_terminates_its_background_child_tree() {
        let marker = std::env::temp_dir().join(format!(
            "flash-shot-recording-orphan-{}-{}.txt",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let command = FfmpegCommand {
            executable: PathBuf::from("cmd.exe"),
            arguments: [
                "/C".to_owned(),
                format!(
                    r#"start "" /B cmd.exe /C "ping -n 3 127.0.0.1 > nul & echo orphan > \"{}\"" & more > nul"#,
                    marker.display()
                ),
            ]
            .map(OsString::from)
            .into(),
        };
        let process = RecordingProcess::start(command).unwrap();
        drop(process);
        std::thread::sleep(Duration::from_millis(700));

        assert!(
            !marker.exists(),
            "a child that outlived the recording Job Object wrote {marker:?}"
        );
        let _ = std::fs::remove_file(marker);
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
