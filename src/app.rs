//! GPUI application shell.

use gpui::{Context, Window, div, prelude::*, rems};

use crate::domain::ProductRoadmap;
use crate::theme::ThemeColors;

pub struct FlashShotApp {
    roadmap: ProductRoadmap,
    colors: ThemeColors,
}

impl FlashShotApp {
    pub fn new() -> Self {
        Self {
            roadmap: ProductRoadmap::current(),
            colors: ThemeColors::default(),
        }
    }
}

impl Render for FlashShotApp {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors;
        let phases =
            self.roadmap
                .phases()
                .iter()
                .enumerate()
                .map(move |(index, phase)| {
                    let status = if index == 0 { "IN PROGRESS" } else { "PLANNED" };
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .p_4()
                        .border_1()
                        .border_color(colors.border)
                        .rounded_md()
                        .bg(colors.panel)
                        .child(
                            div()
                                .flex()
                                .justify_between()
                                .items_center()
                                .child(div().text_lg().text_color(colors.text).child(phase.name))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(if index == 0 {
                                            colors.success
                                        } else {
                                            colors.muted
                                        })
                                        .child(status),
                                ),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(colors.muted)
                                .child(phase.outcome),
                        )
                        .children(phase.capabilities.iter().map(|capability| {
                            div()
                                .flex()
                                .gap_2()
                                .text_sm()
                                .child(div().text_color(colors.accent).child("+"))
                                .child(div().text_color(colors.text).child(format!(
                                    "{}: {}",
                                    capability.name, capability.description
                                )))
                        }))
                });

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.background)
            .text_color(colors.text)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .id("roadmap-scroll")
                    .overflow_y_scroll()
                    .p_8()
                    .flex()
                    .flex_col()
                    .gap_6()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(div().text_size(rems(2.0)).child("Flash Shot"))
                            .child(div().text_color(colors.muted).child(
                                "Native screenshot workflows, built for speed and reliability.",
                            )),
                    )
                    .child(div().flex().flex_col().gap_3().children(phases))
                    .child(
                        div()
                            .mt_auto()
                            .text_sm()
                            .text_color(colors.muted)
                            .child("Milestone 0 / GPUI application shell"),
                    ),
            )
    }
}
