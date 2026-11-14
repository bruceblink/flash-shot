//! Loads preferences before granting startup access to managed screenshot history.

use std::{
    io,
    path::{Path, PathBuf},
};

use flash_shot::{
    history::{HistoryOpenError, HistoryOpenStage, ScreenshotHistory},
    i18n::Locale,
    settings::UserSettings,
};

pub(super) struct StartupProfile {
    pub history: ScreenshotHistory,
    pub settings: UserSettings,
    pub settings_path: PathBuf,
}

#[derive(Debug)]
pub(super) enum StartupError {
    Settings(io::Error),
    History(HistoryOpenError),
}

fn history_recovery_message(locale: Locale, error: &HistoryOpenError) -> String {
    let reason = match (locale, error.stage) {
        (Locale::English, HistoryOpenStage::Root) => {
            "Flash Shot could not open a screenshot history directory."
        }
        (Locale::English, HistoryOpenStage::IndexRead) => {
            "Flash Shot could not read or decode the screenshot history index. No screenshots were deleted."
        }
        (Locale::English, HistoryOpenStage::IndexWrite) => {
            "Flash Shot could not save the normalized history index. The previous index and all screenshots are unchanged."
        }
        (Locale::English, HistoryOpenStage::Retention) => {
            "Flash Shot could not delete some excess screenshots. Failed items remain in the index for retry; other old items may already have been deleted by the retention policy."
        }
        (Locale::English, HistoryOpenStage::RetentionIndexWrite) => {
            "Flash Shot could not save the retention result. Some excess screenshots may already have been deleted. The saved index still lists all items; retry removes missing entries and resumes cleanup."
        }
        (Locale::SimplifiedChinese, HistoryOpenStage::Root) => "Flash Shot 无法打开截图历史目录。",
        (Locale::SimplifiedChinese, HistoryOpenStage::IndexRead) => {
            "Flash Shot 无法读取或解析截图历史索引。未删除任何截图。"
        }
        (Locale::SimplifiedChinese, HistoryOpenStage::IndexWrite) => {
            "Flash Shot 无法保存规范化后的历史索引。原索引和所有截图均未改动。"
        }
        (Locale::SimplifiedChinese, HistoryOpenStage::Retention) => {
            "Flash Shot 无法删除部分超出保留数量的截图。删除失败项仍在索引中，可重试；其他旧截图可能已按保留策略删除。"
        }
        (Locale::SimplifiedChinese, HistoryOpenStage::RetentionIndexWrite) => {
            "Flash Shot 无法保存历史清理结果。部分超出保留数量的截图可能已删除。已保存的索引仍登记全部条目；重试会移除已不存在的条目，并继续清理。"
        }
    };
    match locale {
        Locale::English => format!(
            "{reason}\n\n{}\n\nFailed deletions: {}\nRestore directory read/write access, release files in use, or remove a conflicting history.json.tmp directory. For an invalid index, restore a valid history.json from backup.\n\nChoose Retry after fixing the cause. Cancel exits. Flash Shot will not switch to a different history directory for an index or cleanup error.",
            error.root.display(),
            error.failed_deletions,
        ),
        Locale::SimplifiedChinese => format!(
            "{reason}\n\n{}\n\n删除失败数量：{}\n请恢复目录读写权限、关闭占用文件的程序，或移走冲突的 history.json.tmp 目录。索引内容无效时，请从备份恢复有效的 history.json。\n\n修复后选择“重试”。选择“取消”将退出。索引或清理失败时，Flash Shot 不会切换到其他历史目录。",
            error.root.display(),
            error.failed_deletions,
        ),
    }
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
        managed_directory,
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
pub(super) fn retry_settings_load(path: &Path, kind: io::ErrorKind) -> bool {
    retry_load_dialog(settings_recovery_message(
        Locale::system_default(),
        path,
        kind,
    ))
}

pub(super) fn retry_history_load(error: &HistoryOpenError) -> bool {
    retry_load_dialog(history_recovery_message(Locale::system_default(), error))
}

#[cfg(windows)]
fn retry_load_dialog(message: String) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        IDRETRY, MB_ICONERROR, MB_RETRYCANCEL, MessageBoxW,
    };

    let message: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
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
fn retry_load_dialog(message: String) -> bool {
    eprintln!("{message}");
    false
}

#[cfg(test)]
mod tests {
    use super::{
        StartupError, history_recovery_message, initialize_profile, settings_recovery_message,
    };
    use flash_shot::{
        history::{HistoryOpenError, HistoryOpenStage, ScreenshotHistory},
        i18n::Locale,
    };
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
    fn preferred_index_errors_stop_startup_without_fallback_or_preference_changes() {
        for (case, expected_stage) in [
            ("invalid-json", HistoryOpenStage::IndexRead),
            ("unreadable-index", HistoryOpenStage::IndexRead),
            ("temporary-conflict", HistoryOpenStage::IndexWrite),
        ] {
            let root = test_directory(case);
            let config = root.join("config");
            let preferred = root.join("preferred");
            fs::create_dir_all(&config).unwrap();
            let mut history = ScreenshotHistory::open_with_limit(&preferred, 30).unwrap();
            let image = preferred.join("capture.png");
            fs::write(&image, b"capture").unwrap();
            history.record(image.clone()).unwrap();
            let index_path = preferred.join("history.json");
            match case {
                "invalid-json" => fs::write(&index_path, b"{invalid index").unwrap(),
                "unreadable-index" => {
                    fs::remove_file(&index_path).unwrap();
                    fs::create_dir(&index_path).unwrap();
                }
                _ => fs::create_dir(preferred.join("history.json.tmp")).unwrap(),
            }
            let index_bytes = fs::read(&index_path).ok();
            let path = config.join("settings.json");
            let mut settings = flash_shot::settings::UserSettings::default();
            settings.quick_save_directory = Some(preferred.clone());
            settings.save(&path).unwrap();
            let settings_bytes = fs::read(&path).unwrap();

            let error = match initialize_profile(
                &config,
                || panic!("an index failure must not request another history root"),
                root.join("emergency"),
            ) {
                Err(StartupError::History(error)) => error,
                _ => panic!("expected a history startup error"),
            };

            assert_eq!(error.stage, expected_stage);
            assert_eq!(error.root, preferred.canonicalize().unwrap());
            assert_eq!(fs::read(&path).unwrap(), settings_bytes);
            assert_eq!(fs::read(&image).unwrap(), b"capture");
            assert_eq!(fs::read(&index_path).ok(), index_bytes);
            assert!(!root.join("emergency").exists());
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn managed_index_errors_do_not_create_emergency_history() {
        let root = test_directory("managed-index-failure");
        let managed = root.join("managed");
        fs::create_dir_all(&managed).unwrap();
        fs::write(managed.join("history.json"), b"invalid index").unwrap();

        let result = initialize_profile(
            &root.join("config"),
            || Ok(managed.clone()),
            root.join("emergency"),
        );

        assert!(matches!(
            result,
            Err(StartupError::History(HistoryOpenError {
                stage: HistoryOpenStage::IndexRead,
                ..
            }))
        ));
        assert!(!root.join("emergency").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_index_commit_can_be_retried_with_the_same_settings_and_history() {
        let root = test_directory("index-commit-retry");
        let config = root.join("config");
        let preferred = root.join("preferred");
        fs::create_dir_all(&config).unwrap();
        let mut settings = flash_shot::settings::UserSettings::default();
        settings.history_limit = 10;
        settings.quick_save_directory = Some(preferred.clone());
        let settings_path = config.join("settings.json");
        settings.save(&settings_path).unwrap();
        let original_settings = fs::read(&settings_path).unwrap();
        let mut history = ScreenshotHistory::open_with_limit(&preferred, 30).unwrap();
        for number in 0..11 {
            let image = preferred.join(format!("{number}.png"));
            fs::write(&image, b"capture").unwrap();
            history.record(image).unwrap();
        }
        let original_index = fs::read(preferred.join("history.json")).unwrap();
        fs::create_dir(preferred.join("history.json.tmp")).unwrap();

        let result = initialize_profile(
            &config,
            || panic!("a failed index commit must not request another history root"),
            root.join("emergency"),
        );

        assert!(matches!(
            result,
            Err(StartupError::History(HistoryOpenError {
                stage: HistoryOpenStage::IndexWrite,
                ..
            }))
        ));
        assert_eq!(fs::read(&settings_path).unwrap(), original_settings);
        assert_eq!(
            fs::read(preferred.join("history.json")).unwrap(),
            original_index
        );
        for number in 0..11 {
            assert_eq!(
                fs::read(preferred.join(format!("{number}.png"))).unwrap(),
                b"capture"
            );
        }
        fs::remove_dir(preferred.join("history.json.tmp")).unwrap();
        let profile = initialize_profile(
            &config,
            || panic!("a repaired preferred root must not request a fallback"),
            root.join("emergency"),
        )
        .unwrap();
        assert_eq!(profile.history.entries().len(), 10);
        assert_eq!(fs::read(&settings_path).unwrap(), original_settings);
        assert!(!preferred.join("0.png").exists());
        assert!(!root.join("emergency").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn failed_retention_keeps_the_preferred_directory_and_can_be_retried() {
        use std::os::windows::fs::OpenOptionsExt;

        let root = test_directory("retention-retry");
        let config = root.join("config");
        let preferred = root.join("preferred");
        fs::create_dir_all(&config).unwrap();
        let mut settings = flash_shot::settings::UserSettings::default();
        settings.history_limit = 10;
        settings.quick_save_directory = Some(preferred.clone());
        let settings_path = config.join("settings.json");
        settings.save(&settings_path).unwrap();
        let settings_bytes = fs::read(&settings_path).unwrap();
        let mut history = ScreenshotHistory::open_with_limit(&preferred, 30).unwrap();
        for number in 0..11 {
            let image = preferred.join(format!("{number}.png"));
            fs::write(&image, b"capture").unwrap();
            history.record(image).unwrap();
        }
        let locked = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(preferred.join("0.png"))
            .unwrap();

        let result = initialize_profile(
            &config,
            || panic!("retention failure must not request another history root"),
            root.join("emergency"),
        );

        assert!(matches!(
            result,
            Err(StartupError::History(HistoryOpenError {
                stage: HistoryOpenStage::Retention,
                failed_deletions: 1,
                ..
            }))
        ));
        assert_eq!(fs::read(&settings_path).unwrap(), settings_bytes);
        assert!(preferred.join("0.png").exists());
        assert!(!root.join("emergency").exists());
        drop(locked);
        let profile = initialize_profile(
            &config,
            || panic!("a repaired preferred root must not request a fallback"),
            root.join("emergency"),
        )
        .unwrap();
        assert_eq!(profile.history.entries().len(), 10);
        assert_eq!(
            profile.settings.quick_save_directory,
            Some(preferred.clone())
        );
        assert!(!preferred.join("0.png").exists());
        assert_eq!(fs::read(&settings_path).unwrap(), settings_bytes);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn history_recovery_instructions_distinguish_untouched_and_partial_cleanup() {
        for stage in [
            HistoryOpenStage::Root,
            HistoryOpenStage::IndexRead,
            HistoryOpenStage::IndexWrite,
            HistoryOpenStage::Retention,
            HistoryOpenStage::RetentionIndexWrite,
        ] {
            let error = HistoryOpenError {
                root: PathBuf::from("private/history"),
                stage,
                source: io::Error::other("raw diagnostic is not UI text"),
                failed_deletions: usize::from(stage == HistoryOpenStage::Retention),
            };
            let english = history_recovery_message(Locale::English, &error);
            let chinese = history_recovery_message(Locale::SimplifiedChinese, &error);
            assert!(english.contains("Retry"));
            assert!(english.contains("Cancel"));
            assert!(chinese.contains("重试"));
            assert!(chinese.contains("取消"));
            assert!(english.contains(&error.root.display().to_string()));
            assert!(chinese.contains(&error.root.display().to_string()));
            assert!(!english.contains("raw diagnostic"));
            assert!(!chinese.contains("raw diagnostic"));
            if stage == HistoryOpenStage::IndexWrite {
                assert!(english.contains("unchanged"));
                assert!(chinese.contains("均未改动"));
            }
            if matches!(
                stage,
                HistoryOpenStage::Retention | HistoryOpenStage::RetentionIndexWrite
            ) {
                assert!(english.contains("may already have been deleted"));
                assert!(chinese.contains("可能已"));
                assert!(!english.contains("unchanged"));
                assert!(!chinese.contains("均未改动"));
            }
        }
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
