use flash_shot_gpui_kit_spike::open_probe_window;

/// Starts the isolated gpui-kit compatibility window without touching the production app.
fn main() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        open_probe_window(cx);
    });
}
