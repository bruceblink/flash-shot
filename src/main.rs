//! Flash Shot desktop entry point.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    let started_at = std::time::Instant::now();
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
    log::info!(target: "flash_shot::lifecycle", "application_start");
    flash_shot::run(started_at, performance);
    log::info!(target: "flash_shot::lifecycle", "application_exit");
    drop(diagnostics);
}
