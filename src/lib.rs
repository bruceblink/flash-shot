//! Flash Shot application library.

pub mod app;
pub mod diagnostics;
pub mod domain;
pub mod performance;
pub mod single_instance;
pub mod theme;

use app::FlashShotApp;
use gpui::*;
use performance::PerformanceRecorder;
use std::time::Instant;

actions!(flash_shot, [Quit]);

fn build_menus() -> Vec<Menu> {
    vec![Menu {
        name: "Flash Shot".into(),
        items: vec![MenuItem::action("Quit Flash Shot", Quit)],
        disabled: false,
    }]
}

/// Starts the native GPUI application.
pub fn run(
    started_at: Instant,
    performance: PerformanceRecorder,
) -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let _guard = runtime.enter();

    gpui_platform::application().run(move |cx| {
        cx.set_menus(build_menus());
        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("ctrl-q", Quit, None),
            KeyBinding::new("alt-f4", Quit, None),
        ]);

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(920.), px(760.)), cx)),
            window_min_size: Some(size(px(680.), px(560.))),
            titlebar: Some(TitlebarOptions {
                title: Some("Flash Shot".into()),
                ..Default::default()
            }),
            ..Default::default()
        };

        if let Err(error) = cx.open_window(options, move |window, cx| {
            let performance = performance.clone();
            window.on_next_frame(move |_, _| {
                performance.record_duration("startup_to_first_frame", started_at.elapsed());
            });
            cx.new(|_| FlashShotApp::new())
        }) {
            log::error!(target: "flash_shot::lifecycle", "main_window_open_failed error={error}");
            cx.quit();
            return;
        }
        cx.activate(true);
    });
    Ok(())
}
