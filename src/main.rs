//! Flash Shot desktop entry point.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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
    let history = flash_shot::history::managed_history_directory()
        .and_then(flash_shot::history::ScreenshotHistory::open)
        .unwrap_or_else(|error| {
            log::error!(target: "flash_shot::history", "history_init_failed error={error}");
            std::process::exit(1);
        });
    let (settings, settings_path) = flash_shot::settings::UserSettings::load(
        &diagnostics.paths.config_dir,
    )
    .unwrap_or_else(|error| {
        log::warn!(target: "flash_shot::settings", "settings_load_failed error={error}");
        (
            flash_shot::settings::UserSettings::default(),
            diagnostics.paths.config_dir.join("settings.json"),
        )
    });
    log::info!(target: "flash_shot::lifecycle", "application_start");
    if let Err(error) = flash_shot::run(started_at, performance, history, settings, settings_path) {
        log::error!(target: "flash_shot::lifecycle", "application_run_failed error={error}");
        std::process::exit(1);
    }
    log::info!(target: "flash_shot::lifecycle", "application_exit");
    drop(diagnostics);
}
