//! Flash Shot desktop entry point.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    let diagnostics = flash_shot::diagnostics::init().unwrap_or_else(|error| {
        eprintln!("failed to initialize diagnostics: {error}");
        std::process::exit(1);
    });
    log::info!(target: "flash_shot::lifecycle", "application_start");
    flash_shot::run();
    log::info!(target: "flash_shot::lifecycle", "application_exit");
    drop(diagnostics);
}
