//! Versioned local preferences for the background capture service.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

const SETTINGS_FILE: &str = "settings.json";
const SETTINGS_VERSION: u8 = 1;

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct UserSettings {
    version: u8,
    pub capture_shortcut: Option<String>,
    pub include_cursor: bool,
    pub capture_delay_seconds: u8,
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            capture_shortcut: None,
            include_cursor: false,
            capture_delay_seconds: 0,
        }
    }
}

impl UserSettings {
    pub fn load(config_dir: impl AsRef<Path>) -> io::Result<(Self, PathBuf)> {
        let path = config_dir.as_ref().join(SETTINGS_FILE);
        match fs::read(&path) {
            Ok(bytes) => {
                let mut settings =
                    serde_json::from_slice::<Self>(&bytes).map_err(io::Error::other)?;
                if settings.version > SETTINGS_VERSION {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "settings were created by a newer version of Flash Shot",
                    ));
                }
                settings.version = SETTINGS_VERSION;
                settings.capture_delay_seconds =
                    Self::normalize_capture_delay(settings.capture_delay_seconds);
                Ok((settings, path))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok((Self::default(), path)),
            Err(error) => Err(error),
        }
    }

    pub fn save(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref();
        let parent = path.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "settings path has no parent directory",
            )
        })?;
        fs::create_dir_all(parent)?;
        let temporary = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        fs::write(&temporary, bytes)?;
        fs::rename(temporary, path)
    }

    pub const fn normalize_capture_delay(seconds: u8) -> u8 {
        match seconds {
            0 | 3 | 5 | 10 => seconds,
            _ => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::UserSettings;

    fn directory(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "flash-shot-settings-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    #[test]
    fn missing_settings_use_the_current_version_defaults() {
        let directory = directory("missing");
        let (settings, _) = UserSettings::load(&directory).unwrap();

        assert_eq!(settings.capture_shortcut, None);
        assert!(!settings.include_cursor);
        assert_eq!(settings.capture_delay_seconds, 0);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn settings_round_trip_through_an_atomic_file() {
        let directory = directory("round-trip");
        let (mut settings, path) = UserSettings::load(&directory).unwrap();
        settings.capture_shortcut = Some("Ctrl+Alt+S".to_owned());
        settings.include_cursor = true;
        settings.capture_delay_seconds = 5;
        settings.save(&path).unwrap();

        let (reopened, _) = UserSettings::load(&directory).unwrap();
        assert_eq!(reopened.capture_shortcut.as_deref(), Some("Ctrl+Alt+S"));
        assert!(reopened.include_cursor);
        assert_eq!(reopened.capture_delay_seconds, 5);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn newer_settings_are_not_silently_overwritten() {
        let directory = directory("newer");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("settings.json"), r#"{"version":99}"#).unwrap();

        assert_eq!(
            UserSettings::load(&directory).unwrap_err().kind(),
            std::io::ErrorKind::Unsupported
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn invalid_saved_capture_delay_falls_back_to_off() {
        let directory = directory("invalid-delay");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("settings.json"),
            r#"{"version":1,"capture_delay_seconds":7}"#,
        )
        .unwrap();

        let (settings, _) = UserSettings::load(&directory).unwrap();

        assert_eq!(settings.capture_delay_seconds, 0);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
