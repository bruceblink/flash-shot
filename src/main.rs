//! Flash Shot desktop entry point.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{io, path::PathBuf};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
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
    let (mut settings, settings_path) = flash_shot::settings::UserSettings::load(
        &diagnostics.paths.config_dir,
    )
    .unwrap_or_else(|error| {
        log::warn!(target: "flash_shot::settings", "settings_load_failed error={error}");
        (
            flash_shot::settings::UserSettings::default(),
            diagnostics.paths.config_dir.join("settings.json"),
        )
    });
    let (history, preferred_history_rejected) = open_history_with_fallback(
        settings.quick_save_directory.clone(),
        flash_shot::history::managed_history_directory(),
        diagnostics.paths.data_dir.join("history"),
        usize::from(settings.history_limit),
    )
    .unwrap_or_else(|error| {
        log::error!(target: "flash_shot::history", "history_init_failed error={error}");
        std::process::exit(1);
    });
    if preferred_history_rejected {
        settings.quick_save_directory = None;
        if let Err(error) = settings.save(&settings_path) {
            log::warn!(
                target: "flash_shot::history",
                "history_fallback_preference_clear_failed error={error}"
            );
        }
    }
    log::info!(target: "flash_shot::lifecycle", "application_start");
    if let Err(error) = flash_shot::run(started_at, performance, history, settings, settings_path) {
        log::error!(target: "flash_shot::lifecycle", "application_run_failed error={error}");
        std::process::exit(1);
    }
    log::info!(target: "flash_shot::lifecycle", "application_exit");
    drop(diagnostics);
}

/// Opens the preferred, managed, or emergency history root in that order.
///
/// The returned flag tells startup whether a configured preferred root was rejected, so the
/// caller can clear stale settings while keeping the application usable.
fn open_history_with_fallback(
    preferred_directory: Option<PathBuf>,
    fallback_directory: io::Result<PathBuf>,
    emergency_directory: PathBuf,
    limit: usize,
) -> io::Result<(flash_shot::history::ScreenshotHistory, bool)> {
    let preferred_configured = preferred_directory.is_some();
    if let Some(preferred_directory) = preferred_directory {
        match flash_shot::history::ScreenshotHistory::open_with_limit(preferred_directory, limit) {
            Ok(history) => return Ok((history, false)),
            Err(error) => {
                log::warn!(
                    target: "flash_shot::history",
                    "preferred_history_init_failed error={error}"
                );
            }
        }
    }

    if let Ok(fallback_directory) = fallback_directory {
        match flash_shot::history::ScreenshotHistory::open_with_limit(fallback_directory, limit) {
            Ok(history) => return Ok((history, preferred_configured)),
            Err(error) => log::warn!(
                target: "flash_shot::history",
                "managed_history_fallback_failed error={error}"
            ),
        }
    }

    flash_shot::history::ScreenshotHistory::open_with_limit(emergency_directory, limit)
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
            Ok(fallback.clone()),
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
            Ok(fallback.clone()),
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
            Err(std::io::Error::other("managed history unavailable")),
            emergency.clone(),
            30,
        )
        .unwrap();

        assert_eq!(history.0.root(), emergency.canonicalize().unwrap());
        assert!(!history.1);
        std::fs::remove_dir_all(root).unwrap();
    }
}
