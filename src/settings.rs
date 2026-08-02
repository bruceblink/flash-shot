//! Versioned local preferences for the background capture service.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use crate::theme::ThemeMode;

const SETTINGS_FILE: &str = "settings.json";
const SETTINGS_VERSION: u8 = 1;
pub const DEFAULT_HISTORY_LIMIT: u16 = 30;
pub const DEFAULT_COLOR_FORMAT: u8 = 0;
pub const DEFAULT_EXPORT_FORMAT: u8 = 0;
pub const OCR_LANGUAGE_OPTIONS: [Option<&str>; 4] =
    [None, Some("eng"), Some("chi_sim"), Some("eng+chi_sim")];

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct UserSettings {
    version: u8,
    pub capture_shortcut: Option<String>,
    pub capture_shortcut_enabled: bool,
    pub include_cursor: bool,
    pub capture_delay_seconds: u8,
    pub history_limit: u16,
    pub color_format: u8,
    pub export_format: u8,
    /// A standard Tesseract language selected in Settings; `None` preserves the environment fallback.
    pub ocr_language: Option<String>,
    pub theme_mode: ThemeMode,
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            capture_shortcut: None,
            capture_shortcut_enabled: true,
            include_cursor: false,
            capture_delay_seconds: 0,
            history_limit: DEFAULT_HISTORY_LIMIT,
            color_format: DEFAULT_COLOR_FORMAT,
            export_format: DEFAULT_EXPORT_FORMAT,
            ocr_language: None,
            theme_mode: ThemeMode::Dark,
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
                settings.history_limit = Self::normalize_history_limit(settings.history_limit);
                settings.color_format = Self::normalize_color_format(settings.color_format);
                settings.export_format = Self::normalize_export_format(settings.export_format);
                settings.ocr_language = Self::normalize_ocr_language(settings.ocr_language);
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

    pub const fn normalize_history_limit(limit: u16) -> u16 {
        match limit {
            10 | 30 | 100 | 300 => limit,
            _ => DEFAULT_HISTORY_LIMIT,
        }
    }

    pub const fn normalize_color_format(format: u8) -> u8 {
        match format {
            0..=2 => format,
            _ => DEFAULT_COLOR_FORMAT,
        }
    }

    pub const fn normalize_export_format(format: u8) -> u8 {
        match format {
            0..=2 => format,
            _ => DEFAULT_EXPORT_FORMAT,
        }
    }

    pub const fn next_export_format(current: u8) -> u8 {
        (Self::normalize_export_format(current) + 1) % 3
    }

    /// Keeps persisted OCR choices within the small set that the settings UI can restore safely.
    pub fn normalize_ocr_language(language: Option<String>) -> Option<String> {
        language.filter(|language| {
            OCR_LANGUAGE_OPTIONS
                .iter()
                .flatten()
                .any(|supported| language == supported)
        })
    }

    /// Advances through automatic, English, Chinese, and bilingual local OCR presets.
    pub fn next_ocr_language(current: Option<&str>) -> Option<String> {
        let position = OCR_LANGUAGE_OPTIONS
            .iter()
            .position(|candidate| *candidate == current)
            .unwrap_or(0);
        OCR_LANGUAGE_OPTIONS[(position + 1) % OCR_LANGUAGE_OPTIONS.len()].map(str::to_owned)
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_COLOR_FORMAT, DEFAULT_HISTORY_LIMIT, UserSettings};
    use crate::theme::ThemeMode;

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
        assert!(settings.capture_shortcut_enabled);
        assert!(!settings.include_cursor);
        assert_eq!(settings.capture_delay_seconds, 0);
        assert_eq!(settings.history_limit, DEFAULT_HISTORY_LIMIT);
        assert_eq!(settings.color_format, DEFAULT_COLOR_FORMAT);
        assert_eq!(settings.ocr_language, None);
        assert_eq!(settings.theme_mode, ThemeMode::Dark);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn settings_round_trip_through_an_atomic_file() {
        let directory = directory("round-trip");
        let (mut settings, path) = UserSettings::load(&directory).unwrap();
        settings.capture_shortcut = Some("Ctrl+Alt+S".to_owned());
        settings.capture_shortcut_enabled = false;
        settings.include_cursor = true;
        settings.capture_delay_seconds = 5;
        settings.history_limit = 100;
        settings.color_format = 2;
        settings.ocr_language = Some("eng+chi_sim".to_owned());
        settings.theme_mode = ThemeMode::Light;
        settings.save(&path).unwrap();

        let (reopened, _) = UserSettings::load(&directory).unwrap();
        assert_eq!(reopened.capture_shortcut.as_deref(), Some("Ctrl+Alt+S"));
        assert!(!reopened.capture_shortcut_enabled);
        assert!(reopened.include_cursor);
        assert_eq!(reopened.capture_delay_seconds, 5);
        assert_eq!(reopened.history_limit, 100);
        assert_eq!(reopened.color_format, 2);
        assert_eq!(reopened.ocr_language.as_deref(), Some("eng+chi_sim"));
        assert_eq!(reopened.theme_mode, ThemeMode::Light);
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

    #[test]
    fn legacy_settings_keep_the_global_shortcut_enabled() {
        let directory = directory("legacy-shortcut");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("settings.json"), r#"{"version":1}"#).unwrap();

        let (settings, _) = UserSettings::load(&directory).unwrap();

        assert!(settings.capture_shortcut_enabled);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn invalid_saved_history_limit_falls_back_to_the_default() {
        let directory = directory("invalid-history-limit");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("settings.json"),
            r#"{"version":1,"history_limit":42}"#,
        )
        .unwrap();

        let (settings, _) = UserSettings::load(&directory).unwrap();

        assert_eq!(settings.history_limit, DEFAULT_HISTORY_LIMIT);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn invalid_saved_color_format_falls_back_to_hex() {
        let directory = directory("invalid-color-format");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("settings.json"),
            r#"{"version":1,"color_format":9}"#,
        )
        .unwrap();

        let (settings, _) = UserSettings::load(&directory).unwrap();

        assert_eq!(settings.color_format, DEFAULT_COLOR_FORMAT);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn export_format_cycles_between_png_jpeg_and_webp() {
        assert_eq!(UserSettings::next_export_format(0), 1);
        assert_eq!(UserSettings::next_export_format(1), 2);
        assert_eq!(UserSettings::next_export_format(2), 0);
        assert_eq!(UserSettings::next_export_format(99), 1);
    }

    #[test]
    fn invalid_saved_ocr_language_returns_to_the_environment_fallback() {
        let directory = directory("invalid-ocr-language");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("settings.json"),
            r#"{"version":1,"ocr_language":"unsupported"}"#,
        )
        .unwrap();

        let (settings, _) = UserSettings::load(&directory).unwrap();

        assert_eq!(settings.ocr_language, None);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn ocr_language_cycle_keeps_the_automatic_environment_option() {
        assert_eq!(
            UserSettings::next_ocr_language(None).as_deref(),
            Some("eng")
        );
        assert_eq!(UserSettings::next_ocr_language(Some("eng+chi_sim")), None);
    }
}
