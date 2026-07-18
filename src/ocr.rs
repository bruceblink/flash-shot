//! Lazily invoked local OCR through a separately installed Tesseract executable.

use std::{
    ffi::OsString,
    fs::{self, OpenOptions},
    io,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::platform::capture::CaptureFrame;

const TESSERACT_PATH_ENV: &str = "FLASH_SHOT_TESSERACT";
const TESSERACT_LANGUAGE_ENV: &str = "FLASH_SHOT_OCR_LANGUAGE";

/// Runs the local OCR executable only when the user explicitly requests text recognition.
pub fn recognize(frame: &CaptureFrame) -> io::Result<String> {
    let image_path = temporary_image_path()?;
    let temporary = TemporaryImage::create(image_path)?;
    frame.save_png(temporary.path())?;

    let output = Command::new(executable_path())
        .args(command_arguments(temporary.path(), &language()))
        .output()?;
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

fn executable_path() -> OsString {
    std::env::var_os(TESSERACT_PATH_ENV).unwrap_or_else(|| OsString::from("tesseract"))
}

fn language() -> String {
    std::env::var(TESSERACT_LANGUAGE_ENV).unwrap_or_else(|_| "eng".to_owned())
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
    use super::{TemporaryImage, command_arguments, temporary_image_path};
    use std::{ffi::OsString, path::Path};

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
}
