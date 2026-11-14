//! Flash Shot desktop entry point.

#![cfg_attr(
    all(not(debug_assertions), not(feature = "dev-tools")),
    windows_subsystem = "windows"
)]

use std::{io, path::PathBuf};

mod startup;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    #[cfg(feature = "dev-tools")]
    match flash_shot::dev_tools::run_from_environment() {
        Ok(true) => return,
        Ok(false) => {}
        Err(error) => {
            eprintln!("Flash Shot development tool dispatch failed: {error}");
            std::process::exit(64);
        }
    }

    let started_at = std::time::Instant::now();
    let _single_instance = match flash_shot::single_instance::SingleInstance::acquire() {
        Ok(Some(instance)) => instance,
        Ok(None) => return,
        Err(error) => {
            eprintln!("failed to enforce single instance: {error}");
            std::process::exit(1);
        }
    };
    let diagnostics = flash_shot::diagnostics::init().unwrap_or_else(|error| {
        eprintln!("failed to initialize diagnostics: {error}");
        std::process::exit(1);
    });
    let performance = flash_shot::performance::PerformanceRecorder::new(
        diagnostics.paths.data_dir.join("metrics"),
    )
    .unwrap_or_else(|error| {
        log::error!(target: "flash_shot::performance", "performance_recorder_init_failed error={error}");
        std::process::exit(1);
    });
    let profile = loop {
        match startup::initialize_profile(
            &diagnostics.paths.config_dir,
            flash_shot::history::managed_history_directory,
            diagnostics.paths.data_dir.join("history"),
        ) {
            Ok(profile) => break profile,
            Err(startup::StartupError::Settings(error)) => {
                log::warn!(
                    target: "flash_shot::settings",
                    "settings_load_rejected kind={:?} os_error={:?}; history initialization skipped",
                    error.kind(),
                    error.raw_os_error()
                );
                if !startup::retry_settings_load(
                    &diagnostics.paths.config_dir.join("settings.json"),
                    error.kind(),
                ) {
                    log::info!(target: "flash_shot::settings", "settings_startup_cancelled");
                    return;
                }
                log::info!(target: "flash_shot::settings", "settings_reload_requested");
            }
            Err(startup::StartupError::History(error)) => {
                log::error!(
                    target: "flash_shot::history",
                    "history_init_failed stage={:?} failed_deletions={} root={} error={}",
                    error.stage,
                    error.failed_deletions,
                    error.root.display(),
                    error.source,
                );
                if !startup::retry_history_load(&error) {
                    log::info!(target: "flash_shot::history", "history_startup_cancelled");
                    return;
                }
                log::info!(target: "flash_shot::history", "history_reload_requested");
            }
        }
    };
    log::info!(target: "flash_shot::lifecycle", "application_start");
    if let Err(error) = flash_shot::run(
        started_at,
        performance,
        profile.history,
        profile.settings,
        profile.settings_path,
    ) {
        log::error!(target: "flash_shot::lifecycle", "application_run_failed error={error}");
        std::process::exit(1);
    }
    log::info!(target: "flash_shot::lifecycle", "application_exit");
    drop(diagnostics);
}

/// Opens the preferred, managed, or emergency history root in that order.
///
/// Only a rejected root permits fallback. Once a root is established, index and retention
/// failures must stay attached to that history so startup can offer a meaningful retry.
fn open_history_with_fallback(
    preferred_directory: Option<PathBuf>,
    fallback_directory: impl FnOnce() -> io::Result<PathBuf>,
    emergency_directory: PathBuf,
    limit: usize,
) -> Result<(flash_shot::history::ScreenshotHistory, bool), flash_shot::history::HistoryOpenError> {
    use flash_shot::history::{HistoryOpenStage, ScreenshotHistory};

    let preferred_configured = preferred_directory.is_some();
    if let Some(preferred_directory) = preferred_directory {
        match ScreenshotHistory::open_for_startup(preferred_directory, limit) {
            Ok(history) => return Ok((history, false)),
            Err(error) if error.stage == HistoryOpenStage::Root => {
                log::warn!(
                    target: "flash_shot::history",
                    "preferred_history_init_failed error={error}"
                );
            }
            Err(error) => return Err(error),
        }
    }

    if let Ok(fallback_directory) = fallback_directory() {
        match ScreenshotHistory::open_for_startup(fallback_directory, limit) {
            Ok(history) => return Ok((history, preferred_configured)),
            Err(error) if error.stage == HistoryOpenStage::Root => log::warn!(
                target: "flash_shot::history",
                "managed_history_fallback_failed error={error}"
            ),
            Err(error) => return Err(error),
        }
    }

    ScreenshotHistory::open_for_startup(emergency_directory, limit)
        .map(|history| (history, preferred_configured))
}

#[cfg(test)]
mod tests {
    use super::open_history_with_fallback;
    use std::path::PathBuf;

    #[test]
    fn stale_preferred_history_falls_back_to_the_managed_directory() {
        let root = std::env::temp_dir().join(format!(
            "flash-shot-history-fallback-{}",
            std::process::id()
        ));
        let fallback = root.join("fallback");
        let invalid_preferred = PathBuf::from("preferred\0history");

        let history = open_history_with_fallback(
            Some(invalid_preferred),
            || Ok(fallback.clone()),
            root.join("emergency"),
            30,
        )
        .unwrap();

        assert_eq!(history.0.root(), fallback.canonicalize().unwrap());
        assert!(history.1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn healthy_preferred_history_is_used_without_touching_the_fallback() {
        let root = std::env::temp_dir().join(format!(
            "flash-shot-history-preferred-{}",
            std::process::id()
        ));
        let preferred = root.join("preferred");
        let fallback = root.join("fallback");

        let history = open_history_with_fallback(
            Some(preferred.clone()),
            || Ok(fallback.clone()),
            root.join("emergency"),
            30,
        )
        .unwrap();

        assert_eq!(history.0.root(), preferred.canonicalize().unwrap());
        assert!(!history.1);
        assert!(!fallback.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unavailable_managed_history_uses_the_emergency_application_directory() {
        let root = std::env::temp_dir().join(format!(
            "flash-shot-history-emergency-{}",
            std::process::id()
        ));
        let emergency = root.join("emergency");

        let history = open_history_with_fallback(
            None,
            || Err(std::io::Error::other("managed history unavailable")),
            emergency.clone(),
            30,
        )
        .unwrap();

        assert_eq!(history.0.root(), emergency.canonicalize().unwrap());
        assert!(!history.1);
        std::fs::remove_dir_all(root).unwrap();
    }
}
