//! Small movable controller used while a user manually scrolls the target content.

use gpui::{
    Context, Entity, FocusHandle, Focusable, Render, Subscription, Window, div, prelude::*,
};

use super::FlashShotApp;
use crate::theme::ThemeColors;

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
        let colors = ThemeColors::default();
        let app = self.app.read(cx);
        let status = app.status.clone();
        let frame_count = app.manual_scroll.frame_count();

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
                            .text_color(colors.text)
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| {
                                let app = this.app.clone();
                                cx.defer(move |cx| {
                                    app.update(cx, |app, cx| app.assist_manual_scroll(cx))
                                });
                            }))
                            .child("Scroll down"),
                    )
                    .child(
                        div()
                            .id("scroll-capture-next")
                            .px_3()
                            .py_1()
                            .bg(colors.accent)
                            .text_color(colors.background)
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| {
                                let app = this.app.clone();
                                cx.defer(move |cx| {
                                    app.update(cx, |app, cx| app.capture_manual_scroll_frame(cx))
                                });
                            }))
                            .child("Capture next"),
                    )
                    .child(
                        div()
                            .id("scroll-finish")
                            .px_3()
                            .py_1()
                            .bg(colors.panel)
                            .text_color(colors.text)
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| {
                                let app = this.app.clone();
                                cx.defer(move |cx| {
                                    app.update(cx, |app, cx| app.finish_manual_scroll(cx))
                                });
                            }))
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
