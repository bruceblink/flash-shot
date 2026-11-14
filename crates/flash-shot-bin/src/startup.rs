//! Loads preferences before granting startup access to managed screenshot history.

use std::{
    io,
    path::{Path, PathBuf},
};

use flash_shot::{history::ScreenshotHistory, i18n::Locale, settings::UserSettings};

pub(super) struct StartupProfile {
    pub history: ScreenshotHistory,
    pub settings: UserSettings,
    pub settings_path: PathBuf,
}

#[derive(Debug)]
pub(super) enum StartupError {
    Settings(io::Error),
    History(io::Error),
}

/// An unreadable existing configuration must not authorize retention or later preference writes.
/// The directory provider stays lazy so even creating the managed root requires trusted settings.
pub(super) fn initialize_profile(
    config_directory: &Path,
    managed_directory: impl FnOnce() -> io::Result<PathBuf>,
    emergency_directory: PathBuf,
) -> Result<StartupProfile, StartupError> {
    let (mut settings, settings_path) =
        UserSettings::load(config_directory).map_err(StartupError::Settings)?;
    let (history, preferred_rejected) = super::open_history_with_fallback(
        settings.quick_save_directory.clone(),
        managed_directory(),
        emergency_directory,
        usize::from(settings.history_limit),
    )
    .map_err(StartupError::History)?;
    if preferred_rejected {
        settings.quick_save_directory = None;
        if let Err(error) = settings.save(&settings_path) {
            log::warn!(
                target: "flash_shot::history",
                "history_fallback_preference_clear_failed error={error}"
            );
        }
    }
    Ok(StartupProfile {
        history,
        settings,
        settings_path,
    })
}

fn settings_recovery_message(locale: Locale, path: &Path, kind: io::ErrorKind) -> String {
    let reason = match (locale, kind) {
        (Locale::English, io::ErrorKind::Unsupported) => {
            "The settings file requires a newer version of Flash Shot."
        }
        (Locale::English, io::ErrorKind::PermissionDenied) => {
            "Flash Shot cannot read the settings file because access was denied."
        }
        (Locale::English, _) => "Flash Shot could not read or decode the settings file.",
        (Locale::SimplifiedChinese, io::ErrorKind::Unsupported) => {
            "设置文件需要更新版本的 Flash Shot。"
        }
        (Locale::SimplifiedChinese, io::ErrorKind::PermissionDenied) => {
            "Flash Shot 没有读取设置文件的权限。"
        }
        (Locale::SimplifiedChinese, _) => "Flash Shot 无法读取或解析设置文件。",
    };
    match locale {
        Locale::English => format!(
            "{reason}\n\n{}\n\nSettings and screenshot history have not been changed.\nUse a compatible app version, restore a valid settings file from backup, or restore read access.\n\nChoose Retry after fixing the file. Cancel exits without opening history.",
            path.display()
        ),
        Locale::SimplifiedChinese => format!(
            "{reason}\n\n{}\n\n原设置和截图历史均未改动。\n请使用兼容的应用版本、从备份恢复有效设置，或恢复读取权限。\n\n修复文件后选择“重试”。选择“取消”将退出，不打开历史目录。",
            path.display()
        ),
    }
}

/// The startup dialog has no save/reset action: only a successful reload permits initialization.
#[cfg(windows)]
pub(super) fn retry_settings_load(path: &Path, kind: io::ErrorKind) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        IDRETRY, MB_ICONERROR, MB_RETRYCANCEL, MessageBoxW,
    };

    let message: Vec<u16> = settings_recovery_message(Locale::system_default(), path, kind)
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let title: Vec<u16> = "Flash Shot".encode_utf16().chain(Some(0)).collect();
    // SAFETY: both strings are NUL terminated and remain live for this synchronous modal call.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_ICONERROR | MB_RETRYCANCEL,
        ) == IDRETRY
    }
}

#[cfg(not(windows))]
pub(super) fn retry_settings_load(path: &Path, kind: io::ErrorKind) -> bool {
    eprintln!(
        "{}",
        settings_recovery_message(Locale::system_default(), path, kind)
    );
    false
}

#[cfg(test)]
mod tests {
    use super::{StartupError, initialize_profile, settings_recovery_message};
    use flash_shot::{history::ScreenshotHistory, i18n::Locale};
    use std::{fs, io, path::PathBuf};

    fn test_directory(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "flash-shot-startup-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    #[test]
    fn rejected_settings_never_open_or_prune_existing_history() {
        for count in [100, 300] {
            for (case, bytes) in [
                (
                    "future",
                    br#"{"version":99,"history_limit":300}"#.as_slice(),
                ),
                ("corrupt", b"{invalid settings".as_slice()),
            ] {
                let root = test_directory(&format!("{case}-{count}"));
                let config = root.join("config");
                let managed = root.join("history");
                fs::create_dir_all(&config).unwrap();
                let path = config.join("settings.json");
                fs::write(&path, bytes).unwrap();
                let mut history = ScreenshotHistory::open_with_limit(&managed, count).unwrap();
                for number in 0..count {
                    let image = managed.join(format!("{number}.png"));
                    fs::write(&image, number.to_le_bytes()).unwrap();
                    history.record(image).unwrap();
                }
                drop(history);
                let index = fs::read(managed.join("history.json")).unwrap();
                let result = initialize_profile(
                    &config,
                    || panic!("rejected settings must not request the managed history root"),
                    root.join("emergency"),
                );
                assert!(matches!(result, Err(StartupError::Settings(_))));
                assert_eq!(fs::read(&path).unwrap(), bytes);
                assert_eq!(fs::read(managed.join("history.json")).unwrap(), index);
                for number in 0..count {
                    assert_eq!(
                        fs::read(managed.join(format!("{number}.png"))).unwrap(),
                        number.to_le_bytes()
                    );
                }
                assert!(!root.join("emergency").exists());
                assert!(!config.join("settings.json.tmp").exists());
                fs::remove_dir_all(root).unwrap();
            }
        }
    }

    #[test]
    fn unreadable_settings_stop_startup_and_can_be_retried_after_repair() {
        let root = test_directory("unreadable");
        let config = root.join("config");
        let path = config.join("settings.json");
        fs::create_dir_all(&path).unwrap();
        assert!(matches!(
            initialize_profile(
                &config,
                || panic!("unreadable settings must not open history"),
                root.join("emergency"),
            ),
            Err(StartupError::Settings(_))
        ));
        fs::remove_dir(&path).unwrap();
        fs::write(&path, br#"{"version":1,"history_limit":100}"#).unwrap();
        let profile =
            initialize_profile(&config, || Ok(root.join("history")), root.join("emergency"))
                .unwrap();
        assert_eq!(profile.history.limit(), 100);
        assert_eq!(profile.settings.history_limit, 100);
        assert_eq!(
            fs::read(&path).unwrap(),
            br#"{"version":1,"history_limit":100}"#
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn exclusively_open_settings_are_not_replaced_by_defaults() {
        use std::os::windows::fs::OpenOptionsExt;

        let root = test_directory("sharing-conflict");
        let config = root.join("config");
        fs::create_dir_all(&config).unwrap();
        let path = config.join("settings.json");
        let bytes = br#"{"version":1,"history_limit":300}"#;
        fs::write(&path, bytes).unwrap();
        let locked = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        assert!(matches!(
            initialize_profile(
                &config,
                || panic!("a sharing conflict must not open history"),
                root.join("emergency"),
            ),
            Err(StartupError::Settings(_))
        ));
        drop(locked);
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let profile =
            initialize_profile(&config, || Ok(root.join("history")), root.join("emergency"))
                .unwrap();
        assert_eq!(profile.history.limit(), 300);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_settings_still_allow_first_run_defaults() {
        let root = test_directory("first-run");
        let profile = initialize_profile(
            &root.join("config"),
            || Ok(root.join("history")),
            root.join("emergency"),
        )
        .unwrap();
        assert_eq!(profile.settings.history_limit, 30);
        assert_eq!(profile.history.limit(), 30);
        assert!(!profile.settings_path.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recovery_instructions_are_localized_without_raw_setting_values() {
        let path = std::path::Path::new("config/settings.json");
        let english = settings_recovery_message(Locale::English, path, io::ErrorKind::Unsupported);
        let chinese =
            settings_recovery_message(Locale::SimplifiedChinese, path, io::ErrorKind::Other);
        assert!(english.contains("newer version"));
        assert!(english.contains("Retry"));
        assert!(english.contains("Cancel"));
        assert!(english.contains("have not been changed"));
        assert!(chinese.contains("无法读取或解析"));
        assert!(chinese.contains("重试"));
        assert!(chinese.contains("不打开历史目录"));
        assert!(english.contains(&path.display().to_string()));
        assert!(chinese.contains(&path.display().to_string()));
    }
}
