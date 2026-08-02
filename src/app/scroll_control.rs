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
        let auto_capture_pending = app.manual_scroll_auto_capture_generation.is_some();
        let controls_busy = capture_in_flight || auto_capture_pending;
        let retry_available = app.manual_scroll.failure().is_some();
        let can_finish = app.manual_scroll.can_finish();

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
                            .text_color(if controls_busy {
                                colors.muted
                            } else {
                                colors.text
                            })
                            .when(!controls_busy, |button| {
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
                            .id("scroll-auto-capture-next")
                            .px_3()
                            .py_1()
                            .bg(colors.panel)
                            .text_color(if controls_busy {
                                colors.muted
                            } else {
                                colors.text
                            })
                            .when(!controls_busy, |button| {
                                button
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let app = this.app.clone();
                                        cx.defer(move |cx| {
                                            app.update(cx, |app, cx| {
                                                app.auto_capture_manual_scroll_frame(cx)
                                            })
                                        });
                                    }))
                            })
                            .child(auto_scroll_capture_label(auto_capture_pending)),
                    )
                    .child(
                        div()
                            .id("scroll-capture-next")
                            .px_3()
                            .py_1()
                            .bg(colors.accent)
                            .text_color(if controls_busy {
                                colors.muted
                            } else {
                                colors.background
                            })
                            .when(!controls_busy, |button| {
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
                            .child(manual_scroll_capture_label(
                                capture_in_flight,
                                retry_available,
                            )),
                    )
                    .child(
                        div()
                            .id("scroll-finish")
                            .px_3()
                            .py_1()
                            .bg(colors.panel)
                            .text_color(if controls_busy || !can_finish {
                                colors.muted
                            } else {
                                colors.text
                            })
                            .when(!controls_busy && can_finish, |button| {
                                button
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let app = this.app.clone();
                                        cx.defer(move |cx| {
                                            app.update(cx, |app, cx| app.finish_manual_scroll(cx))
                                        });
                                    }))
                            })
                            .child(manual_scroll_finish_label(can_finish)),
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
fn manual_scroll_capture_label(capture_in_flight: bool, retry_available: bool) -> &'static str {
    if capture_in_flight {
        "Capturing..."
    } else if retry_available {
        "Retry frame"
    } else {
        "Capture next"
    }
}

/// Explains that automatic capture is waiting for the target application to repaint.
fn auto_scroll_capture_label(auto_capture_pending: bool) -> &'static str {
    if auto_capture_pending {
        "Waiting..."
    } else {
        "Scroll + capture"
    }
}

/// Names the next required action until a second viewport makes stitching possible.
fn manual_scroll_finish_label(can_finish: bool) -> &'static str {
    if can_finish {
        "Finish"
    } else {
        "Capture another"
    }
}

#[cfg(test)]
mod tests {
    use super::{
        auto_scroll_capture_label, manual_scroll_capture_label, manual_scroll_finish_label,
    };

    #[test]
    fn capture_action_describes_its_busy_state() {
        assert_eq!(manual_scroll_capture_label(false, false), "Capture next");
        assert_eq!(manual_scroll_capture_label(false, true), "Retry frame");
        assert_eq!(manual_scroll_capture_label(true, true), "Capturing...");
    }

    #[test]
    fn finish_action_requires_an_overlapping_viewport() {
        assert_eq!(manual_scroll_finish_label(false), "Capture another");
        assert_eq!(manual_scroll_finish_label(true), "Finish");
    }

    #[test]
    fn automatic_capture_action_reports_its_settle_delay() {
        assert_eq!(auto_scroll_capture_label(false), "Scroll + capture");
        assert_eq!(auto_scroll_capture_label(true), "Waiting...");
    }
}
