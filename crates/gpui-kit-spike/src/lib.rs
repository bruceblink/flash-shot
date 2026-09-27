use gpui_kit::component::{
    IconName, Root,
    button::{Button, ButtonVariants},
    popover::Popover,
};
use gpui_kit::{
    AppContext, Context, InteractiveElement, IntoElement, ParentElement, Render, Styled, Window,
    WindowOptions, div, px, size,
};

/// Holds only the probe state needed to verify a component callback.
///
/// The spike is deliberately isolated from Flash Shot's production state machine: a successful
/// click changes this counter, while all screenshot and settings behavior remains untouched.
pub struct ProbeView {
    pub clicks: usize,
}

impl Render for ProbeView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let click_count = self.clicks;
        div()
            .id("gpui-kit-probe")
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .child(
                Button::new("primary-action")
                    .primary()
                    .label(format!("GPUI Kit action ({click_count})"))
                    .accessibility_label("Run GPUI Kit compatibility action")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.clicks += 1;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("icon-action")
                    .ghost()
                    .icon(IconName::Check)
                    .accessibility_label("Confirm GPUI Kit icon action")
                    .tooltip("Icon action"),
            )
            .child(
                Popover::new("popover")
                    .trigger(Button::new("popover-trigger").label("Open popover"))
                    .content(|_, _, _| popover_content()),
            )
    }
}

#[cfg(not(test))]
fn popover_content() -> impl IntoElement {
    div()
        .id("popover-content")
        .w(px(180.))
        .h(px(56.))
        .p_3()
        .child("GPUI Kit popover")
}

#[cfg(test)]
fn popover_content() -> impl IntoElement {
    use gpui_kit::TestSupportExt;

    div()
        .id("popover-content")
        .test_support()
        .w(px(180.))
        .h(px(56.))
        .p_3()
        .child("GPUI Kit popover")
}

/// Opens the smallest real GPUI Kit window used by the compatibility acceptance run.
///
/// The callback, icon, tooltip, focusable button and popover are all rendered by gpui-kit.
/// Closing this process is the rollback boundary: the production application never imports this
/// probe's state or window.
pub fn open_probe_window(cx: &mut gpui_kit::App) {
    cx.open_window(
        WindowOptions {
            window_bounds: Some(gpui_kit::WindowBounds::centered(
                size(px(420.), px(420.)),
                cx,
            )),
            ..Default::default()
        },
        |window, cx| {
            let view = cx.new(|_| ProbeView { clicks: 0 });
            cx.new(|cx| Root::new(view, window, cx))
        },
    )
    .expect("failed to open gpui-kit compatibility window");
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;
    use gpui_kit::test::TestWindowExt;

    #[gpui_kit::test]
    fn components_render_and_dispatch_events(cx: &mut TestAppContext) {
        cx.update(gpui_kit::component::init);
        let handle = cx.add_window(|_, _| ProbeView { clicks: 0 });
        cx.update_window(handle.into(), |_, window, cx| {
            window.draw(cx).clear(cx);
            assert_eq!(
                window.find("primary-action").label(),
                Some("Run GPUI Kit compatibility action")
            );
            assert_eq!(
                window.find("icon-action").label(),
                Some("Confirm GPUI Kit icon action")
            );
            assert!(window.try_find("popover-content").is_none());
            window.click("primary-action", cx);
            window.click("popover-trigger", cx);
            window.render_frame(cx);
            let content = window.find("popover-content");
            assert!(content.visible());
        })
        .unwrap();
        handle
            .update(cx, |view, _, _| assert_eq!(view.clicks, 1))
            .unwrap();
    }
}
