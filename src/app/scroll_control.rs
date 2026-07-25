//! Small movable controller used while a user manually scrolls the target content.

use gpui::{
    Context, Entity, FocusHandle, Focusable, Render, Subscription, Window, div, prelude::*,
};

use super::FlashShotApp;

pub(super) struct ManualScrollControl {
    app: Entity<FlashShotApp>,
    focus_handle: FocusHandle,
    _app_observation: Subscription,
}

impl ManualScrollControl {
    pub(super) fn new(app: Entity<FlashShotApp>, cx: &mut Context<Self>) -> Self {
        let observation = cx.observe(&app, |_, _, cx| cx.notify());
        Self {
            app,
            focus_handle: cx.focus_handle(),
            _app_observation: observation,
        }
    }
}

impl Focusable for ManualScrollControl {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ManualScrollControl {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app = self.app.read(cx);
        let colors = app.colors;
        let status = app.status.clone();
        let frame_count = app.manual_scroll.frame_count();
        let capture_in_flight = app.manual_scroll_capture_in_flight;

        div()
            .size_full()
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .bg(colors.background)
            .border_1()
            .border_color(colors.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .text_color(colors.text)
                            .child("Manual scroll"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(colors.muted)
                            .child(format!("{frame_count} frames")),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        div()
                            .id("scroll-assist-down")
                            .px_3()
                            .py_1()
                            .bg(colors.panel)
                            .text_color(if capture_in_flight {
                                colors.muted
                            } else {
                                colors.text
                            })
                            .when(!capture_in_flight, |button| {
                                button
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let app = this.app.clone();
                                        cx.defer(move |cx| {
                                            app.update(cx, |app, cx| app.assist_manual_scroll(cx))
                                        });
                                    }))
                            })
                            .child("Scroll down"),
                    )
                    .child(
                        div()
                            .id("scroll-capture-next")
                            .px_3()
                            .py_1()
                            .bg(colors.accent)
                            .text_color(if capture_in_flight {
                                colors.muted
                            } else {
                                colors.background
                            })
                            .when(!capture_in_flight, |button| {
                                button
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let app = this.app.clone();
                                        cx.defer(move |cx| {
                                            app.update(cx, |app, cx| {
                                                app.capture_manual_scroll_frame(cx)
                                            })
                                        });
                                    }))
                            })
                            .child(manual_scroll_capture_label(capture_in_flight)),
                    )
                    .child(
                        div()
                            .id("scroll-finish")
                            .px_3()
                            .py_1()
                            .bg(colors.panel)
                            .text_color(if capture_in_flight {
                                colors.muted
                            } else {
                                colors.text
                            })
                            .when(!capture_in_flight, |button| {
                                button
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let app = this.app.clone();
                                        cx.defer(move |cx| {
                                            app.update(cx, |app, cx| app.finish_manual_scroll(cx))
                                        });
                                    }))
                            })
                            .child("Finish"),
                    )
                    .child(
                        div()
                            .id("scroll-cancel")
                            .px_3()
                            .py_1()
                            .text_color(colors.muted)
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| {
                                let app = this.app.clone();
                                cx.defer(move |cx| {
                                    app.update(cx, |app, cx| app.cancel_manual_scroll(cx))
                                });
                            }))
                            .child("Cancel"),
                    ),
            )
            .child(div().text_xs().text_color(colors.muted).child(status))
    }
}

/// Keeps the primary action explicit while one scroll frame is being captured.
fn manual_scroll_capture_label(capture_in_flight: bool) -> &'static str {
    if capture_in_flight {
        "Capturing..."
    } else {
        "Capture next"
    }
}

#[cfg(test)]
mod tests {
    use super::manual_scroll_capture_label;

    #[test]
    fn capture_action_describes_its_busy_state() {
        assert_eq!(manual_scroll_capture_label(false), "Capture next");
        assert_eq!(manual_scroll_capture_label(true), "Capturing...");
    }
}
