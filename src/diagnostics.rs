//! Local application directories and privacy-safe structured diagnostics.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    panic,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use directories::ProjectDirs;
use log::{LevelFilter, Log, Metadata, Record};

const LOG_FILE_NAME: &str = "flash-shot.jsonl";
type PanicHook = Box<dyn Fn(&panic::PanicHookInfo<'_>) + Sync + Send + 'static>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub log_dir: PathBuf,
}

impl AppPaths {
    pub fn discover() -> io::Result<Self> {
        let project = ProjectDirs::from("com", "BruceBlink", "Flash Shot").ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "application directories unavailable",
            )
        })?;

        let paths = Self {
            config_dir: project.config_dir().to_path_buf(),
            data_dir: project.data_dir().to_path_buf(),
            cache_dir: project.cache_dir().to_path_buf(),
            log_dir: project.data_dir().join("logs"),
        };
        paths.create()?;
        Ok(paths)
    }

    fn create(&self) -> io::Result<()> {
        for path in [
            &self.config_dir,
            &self.data_dir,
            &self.cache_dir,
            &self.log_dir,
        ] {
            fs::create_dir_all(path)?;
        }
        Ok(())
    }
}

struct JsonLogger {
    file: Mutex<File>,
    level: LevelFilter,
}

impl Log for JsonLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let entry = serde_json::json!({
            "timestamp_ms": unix_timestamp_ms(),
            "level": record.level().as_str(),
            "target": record.target(),
            "event": record.args().to_string(),
        });
        if let Ok(mut file) = self.file.lock() {
            let _ = writeln!(file, "{entry}");
            let _ = file.flush();
        }
    }

    fn flush(&self) {
        if let Ok(mut file) = self.file.lock() {
            let _ = file.flush();
        }
    }
}

pub struct DiagnosticsGuard {
    pub paths: AppPaths,
    previous_panic_hook: Option<PanicHook>,
}

impl Drop for DiagnosticsGuard {
    fn drop(&mut self) {
        log::logger().flush();
        if let Some(hook) = self.previous_panic_hook.take() {
            panic::set_hook(hook);
        }
    }
}

pub fn init() -> Result<DiagnosticsGuard, Box<dyn std::error::Error>> {
    let paths = AppPaths::discover()?;
    let file = open_log_file(&paths.log_dir)?;
    let level = configured_level();
    log::set_boxed_logger(Box::new(JsonLogger {
        file: Mutex::new(file),
        level,
    }))?;
    log::set_max_level(level);

    let previous_panic_hook = panic::take_hook();
    panic::set_hook(Box::new(|info| {
        let location = info
            .location()
            .map(|location| format!("{}:{}", location.file(), location.line()))
            .unwrap_or_else(|| "unknown".to_owned());
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
            .unwrap_or("non-string panic payload");
        log::error!(target: "flash_shot::panic", "panic location={location} message={message}");
    }));

    Ok(DiagnosticsGuard {
        paths,
        previous_panic_hook: Some(previous_panic_hook),
    })
}

fn open_log_file(log_dir: &Path) -> io::Result<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join(LOG_FILE_NAME))
}

fn configured_level() -> LevelFilter {
    std::env::var("FLASH_SHOT_LOG")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(LevelFilter::Info)
}

fn unix_timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::{JsonLogger, LOG_FILE_NAME, Log, Metadata, Record};
    use log::{Level, LevelFilter};
    use std::{fs::OpenOptions, sync::Mutex};

    #[test]
    fn logger_writes_valid_json_without_recording_arguments_as_fields() {
        let directory =
            std::env::temp_dir().join(format!("flash-shot-diagnostics-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join(LOG_FILE_NAME);
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)
            .unwrap();
        let logger = JsonLogger {
            file: Mutex::new(file),
            level: LevelFilter::Info,
        };
        let args = format_args!("capture_failed");
        let record = Record::builder()
            .args(args)
            .level(Level::Warn)
            .target("flash_shot::capture")
            .build();

        logger.log(&record);
        logger.flush();

        let line = std::fs::read_to_string(&path).unwrap();
        let value: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(value["event"], "capture_failed");
        assert_eq!(value["target"], "flash_shot::capture");
        assert!(value.get("timestamp_ms").is_some());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn logger_respects_level_filter() {
        let metadata = Metadata::builder()
            .level(Level::Debug)
            .target("flash_shot")
            .build();
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(std::env::temp_dir().join("flash-shot-level-test.jsonl"))
            .unwrap();
        let logger = JsonLogger {
            file: Mutex::new(file),
            level: LevelFilter::Info,
        };

        assert!(!logger.enabled(&metadata));
    }
}
