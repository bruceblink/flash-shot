//! Lazily invoked local OCR through a separately installed Tesseract executable.

use std::{
    ffi::OsString,
    fs::{self, OpenOptions},
    io,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::platform::capture::CaptureFrame;

const TESSERACT_PATH_ENV: &str = "FLASH_SHOT_TESSERACT";
const TESSERACT_LANGUAGE_ENV: &str = "FLASH_SHOT_OCR_LANGUAGE";
const OCR_COMMAND_TIMEOUT: Duration = Duration::from_secs(20);

/// Read-only local OCR readiness information used before the user begins a capture workflow.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OcrSupport {
    version: String,
    language: String,
    language_available: bool,
}

impl OcrSupport {
    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn language(&self) -> &str {
        &self.language
    }

    pub const fn language_available(&self) -> bool {
        self.language_available
    }
}

/// Checks the configured Tesseract executable and selected language without creating an image.
pub fn check_support(configured_language: Option<&str>) -> io::Result<OcrSupport> {
    let executable = executable_path();
    let version_output = Command::new(&executable).arg("--version").output()?;
    if !version_output.status.success() {
        return Err(ocr_command_error("--version", &version_output));
    }
    let languages_output = Command::new(&executable).arg("--list-langs").output()?;
    if !languages_output.status.success() {
        return Err(ocr_command_error("--list-langs", &languages_output));
    }
    let language = configured_language.unwrap_or(&language()).to_owned();
    let languages = listed_languages(&languages_output.stdout);
    Ok(OcrSupport {
        version: tesseract_version(&version_output.stdout),
        language_available: language
            .split('+')
            .all(|requested| languages.iter().any(|available| available == requested)),
        language,
    })
}

/// Runs the local OCR executable only when the user explicitly requests text recognition.
pub fn recognize(frame: &CaptureFrame) -> io::Result<String> {
    recognize_with_language(frame, None)
}

/// Runs local OCR with a saved language preset, falling back to the environment for legacy setups.
pub fn recognize_with_language(
    frame: &CaptureFrame,
    configured_language: Option<&str>,
) -> io::Result<String> {
    let image_path = temporary_image_path()?;
    let temporary = TemporaryImage::create(image_path)?;
    frame.save_png(temporary.path())?;

    let output = run_tesseract_command(command_arguments(
        temporary.path(),
        configured_language.unwrap_or(&language()),
    ))?;
    if !output.status.success() {
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        return Err(io::Error::other(format!(
            "local OCR exited with {}{}",
            output.status,
            diagnostic
                .lines()
                .next()
                .map(|line| format!(": {line}"))
                .unwrap_or_default()
        )));
    }
    String::from_utf8(output.stdout)
        .map(|text| text.trim().to_owned())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// Waits for Tesseract without letting a stuck local process block recognition forever.
fn run_tesseract_command(arguments: Vec<OsString>) -> io::Result<Output> {
    let mut child = Command::new(executable_path())
        .args(arguments)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    wait_for_child(&mut child, OCR_COMMAND_TIMEOUT)?;
    child.wait_with_output()
}

/// Polls a child until it exits; on timeout or wait failure, kills and reaps it before returning.
fn wait_for_child(child: &mut Child, timeout: Duration) -> io::Result<()> {
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
                        "local OCR exceeded {} ms and was terminated",
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

fn executable_path() -> OsString {
    std::env::var_os(TESSERACT_PATH_ENV).unwrap_or_else(|| OsString::from("tesseract"))
}

fn language() -> String {
    std::env::var(TESSERACT_LANGUAGE_ENV).unwrap_or_else(|_| "eng".to_owned())
}

fn ocr_command_error(arguments: &str, output: &std::process::Output) -> io::Error {
    io::Error::other(format!(
        "Tesseract {arguments} exited with {}{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
            .lines()
            .next()
            .map(|line| format!(": {line}"))
            .unwrap_or_default()
    ))
}

fn tesseract_version(output: &[u8]) -> String {
    String::from_utf8_lossy(output)
        .lines()
        .next()
        .unwrap_or("unknown version")
        .trim()
        .to_owned()
}

fn listed_languages(output: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(output)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("List of available languages"))
        .map(str::to_owned)
        .collect()
}

fn command_arguments(image_path: &Path, language: &str) -> Vec<OsString> {
    vec![
        image_path.as_os_str().to_owned(),
        OsString::from("stdout"),
        OsString::from("--psm"),
        OsString::from("6"),
        OsString::from("-l"),
        OsString::from(language),
    ]
}

fn temporary_image_path() -> io::Result<PathBuf> {
    let directory = std::env::temp_dir().join("flash-shot-ocr");
    fs::create_dir_all(&directory)?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    Ok(directory.join(format!("ocr-{}-{timestamp}.png", std::process::id())))
}

struct TemporaryImage {
    path: PathBuf,
}

impl TemporaryImage {
    fn create(path: PathBuf) -> io::Result<Self> {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryImage {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        OCR_COMMAND_TIMEOUT, TemporaryImage, command_arguments, listed_languages,
        temporary_image_path, wait_for_child,
    };
    use std::{
        ffi::OsString,
        path::Path,
        process::{Command, Stdio},
        time::Duration,
    };

    #[test]
    fn ocr_commands_have_a_bounded_timeout() {
        assert_eq!(OCR_COMMAND_TIMEOUT, Duration::from_secs(20));
    }

    #[test]
    fn completed_ocr_processes_are_reaped_before_the_deadline() {
        let mut command = if cfg!(windows) {
            let mut command = Command::new("cmd");
            command.args(["/C", "exit 0"]);
            command
        } else {
            let mut command = Command::new("sh");
            command.args(["-c", "exit 0"]);
            command
        };
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        wait_for_child(&mut child, Duration::from_secs(2)).unwrap();
        assert!(child.wait_with_output().unwrap().status.success());
    }

    #[test]
    fn stuck_ocr_processes_are_terminated_at_the_deadline() {
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

        let error = wait_for_child(&mut child, Duration::from_millis(100)).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(child.try_wait().unwrap().is_some());
    }

    #[test]
    fn tesseract_uses_stdout_without_a_persistent_output_file() {
        let arguments = command_arguments(Path::new("selection.png"), "eng+chi_sim");

        assert_eq!(
            arguments,
            [
                OsString::from("selection.png"),
                OsString::from("stdout"),
                OsString::from("--psm"),
                OsString::from("6"),
                OsString::from("-l"),
                OsString::from("eng+chi_sim"),
            ]
        );
    }

    #[test]
    fn temporary_ocr_image_is_removed_when_the_task_finishes() {
        let path = temporary_image_path().unwrap();
        let image = TemporaryImage::create(path.clone()).unwrap();

        assert!(path.is_file());
        drop(image);
        assert!(!path.exists());
    }

    #[test]
    fn language_listing_skips_the_tesseract_header() {
        assert_eq!(
            listed_languages(
                b"List of available languages in C:\\tessdata/ (3):\nchi_sim\neng\nosd\n"
            ),
            ["chi_sim", "eng", "osd"]
        );
    }
}
