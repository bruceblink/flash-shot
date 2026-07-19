//! The small, on-demand settings window for the background capture service.

use gpui::{Window, div, prelude::*, px};

use super::FlashShotApp;
use crate::domain::session::CaptureSessionState;

impl gpui::Render for FlashShotApp {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let colors = self.colors;
        let is_idle = self.session.state() == CaptureSessionState::Idle;
        let delayed_capture = self.delayed_capture_generation.is_some();
        let recording_active = self.recording_control.is_some();
        let recording_starting = self.recording_start_in_flight;
        let recording_paused = self.recording_paused;
        let recording_audio =
            super::workflow::recording_audio_selection_label(&self.recording_audio);
        let recording_display =
            super::workflow::recording_display_selection_label(&self.recording_display);
        let history_entries: Vec<_> = self.history.entries().iter().take(5).cloned().collect();
        let app = cx.entity();

        div()
            .size_full()
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .bg(colors.background)
            .text_color(colors.text)
            .child(
                div()
                    .h(px(56.0))
                    .px_5()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(colors.border)
                    .child(div().text_lg().child("Flash Shot"))
                    .child(
                        div()
                            .id("settings-hide")
                            .px_3()
                            .py_1()
                            .text_sm()
                            .text_color(colors.muted)
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, _| this.hide_settings_window()))
                            .child("Close"),
                    ),
            )
            .child(
                div()
                    .id("settings-content")
                    .flex_1()
                    .overflow_y_scroll()
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_5()
                    .child(
                        settings_section("Capture")
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap_3()
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .gap_1()
                                            .child(div().text_sm().child("Global shortcut"))
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(colors.muted)
                                                    .child(self.capture_shortcut.clone()),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .id("settings-capture")
                                            .px_4()
                                            .py_2()
                                            .bg(colors.accent)
                                            .text_color(colors.background)
                                            .cursor_pointer()
                                            .when(is_idle || delayed_capture, |button| {
                                                button.on_click(cx.listener(|this, _, _, cx| {
                                                    if this.delayed_capture_generation.is_some() {
                                                        this.cancel_delayed_capture(cx);
                                                    } else {
                                                        this.start_capture(cx);
                                                    }
                                                }))
                                            })
                                            .child(if delayed_capture {
                                                "Cancel delay"
                                            } else {
                                                "Capture"
                                            }),
                                    ),
                            )
                            .child(settings_row("Include cursor").child(settings_toggle(
                                "settings-cursor",
                                self.include_cursor,
                                colors,
                                is_idle && !delayed_capture,
                                {
                                    let app = app.clone();
                                    move |_, _, cx| {
                                        app.update(cx, |this, cx| this.toggle_capture_cursor(cx))
                                    }
                                },
                            )))
                            .child(
                                settings_row("Capture delay").child(
                                    div()
                                        .id("settings-delay")
                                        .px_3()
                                        .py_1()
                                        .border_1()
                                        .border_color(colors.border)
                                        .text_color(colors.muted)
                                        .cursor_pointer()
                                        .when(is_idle && !delayed_capture, |button| {
                                            button.on_click(cx.listener(|this, _, _, cx| {
                                                this.cycle_capture_delay(cx)
                                            }))
                                        })
                                        .child(if self.capture_delay_seconds == 0 {
                                            "Off".to_owned()
                                        } else {
                                            format!("{} seconds", self.capture_delay_seconds)
                                        }),
                                ),
                            ),
                    )
                    .child(
                        settings_section("Files").child(
                            div()
                                .flex()
                                .gap_2()
                                .child(settings_button(
                                    "settings-open-image",
                                    "Open PNG",
                                    colors,
                                    is_idle,
                                    {
                                        let app = app.clone();
                                        move |_, _, cx| {
                                            app.update(cx, |this, cx| this.open_image(cx))
                                        }
                                    },
                                ))
                                .child(settings_button(
                                    "settings-open-project",
                                    "Open Project",
                                    colors,
                                    is_idle,
                                    {
                                        let app = app.clone();
                                        move |_, _, cx| {
                                            app.update(cx, |this, cx| {
                                                this.open_editable_project(cx)
                                            })
                                        }
                                    },
                                )),
                        ),
                    )
                    .child(
                        settings_section("Recording")
                            .child(settings_row("Display").child(settings_button(
                                "settings-recording-display",
                                &recording_display,
                                colors,
                                !recording_active && !recording_starting,
                                {
                                    let app = app.clone();
                                    move |_, _, cx| {
                                        app.update(cx, |this, cx| this.cycle_recording_display(cx))
                                    }
                                },
                            )))
                            .child(settings_row("Audio").child(settings_button(
                                "settings-recording-audio",
                                &recording_audio,
                                colors,
                                !recording_active && !recording_starting,
                                {
                                    let app = app.clone();
                                    move |_, _, cx| {
                                        app.update(cx, |this, cx| this.cycle_recording_audio(cx))
                                    }
                                },
                            )))
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(settings_button(
                                        "settings-record-display",
                                        if recording_starting {
                                            "Preparing..."
                                        } else if recording_active {
                                            "Stop recording"
                                        } else {
                                            "Record display"
                                        },
                                        colors,
                                        !recording_starting,
                                        {
                                            let app = app.clone();
                                            move |_, _, cx| {
                                                app.update(cx, |this, cx| {
                                                    this.toggle_display_recording(cx)
                                                })
                                            }
                                        },
                                    ))
                                    .when(recording_active && !recording_starting, |row| {
                                        row.child(settings_button(
                                            "settings-pause-recording",
                                            if recording_paused { "Resume" } else { "Pause" },
                                            colors,
                                            true,
                                            {
                                                let app = app.clone();
                                                move |_, _, cx| {
                                                    app.update(cx, |this, cx| {
                                                        this.toggle_recording_pause(cx)
                                                    })
                                                }
                                            },
                                        ))
                                    }),
                            ),
                    )
                    .child(
                        settings_section("System")
                            .child(settings_row("Start with Windows").child(settings_toggle(
                                "settings-auto-start",
                                self.auto_start_enabled,
                                colors,
                                true,
                                {
                                    let app = app.clone();
                                    move |_, _, cx| {
                                        app.update(cx, |this, cx| this.toggle_auto_start(cx))
                                    }
                                },
                            )))
                            .child(settings_row("Updates").child(settings_button(
                                "settings-check-updates",
                                if self.update_check_in_flight {
                                    "Checking..."
                                } else {
                                    "Check now"
                                },
                                colors,
                                !self.update_check_in_flight,
                                {
                                    let app = app.clone();
                                    move |_, _, cx| {
                                        app.update(cx, |this, cx| this.check_for_updates(cx))
                                    }
                                },
                            ))),
                    )
                    .when(!history_entries.is_empty(), |content| {
                        content.child(
                            settings_section("Recent captures")
                                .children(history_entries.into_iter().map(|entry| {
                                    settings_row(
                                        entry
                                            .path
                                            .file_name()
                                            .and_then(|name| name.to_str())
                                            .unwrap_or("Capture"),
                                    )
                                    .child(settings_button(
                                        format!("settings-open-history-{}", entry.created_at_ms),
                                        "Open",
                                        colors,
                                        is_idle,
                                        {
                                            let app = app.clone();
                                            let path = entry.path.clone();
                                            move |_, _, cx| {
                                                app.update(cx, |this, cx| {
                                                    this.open_history_image(path.clone(), cx)
                                                })
                                            }
                                        },
                                    ))
                                    .child(settings_button(
                                        format!("settings-remove-history-{}", entry.created_at_ms),
                                        "Remove",
                                        colors,
                                        is_idle,
                                        {
                                            let app = app.clone();
                                            let path = entry.path.clone();
                                            move |_, _, cx| {
                                                app.update(cx, |this, cx| {
                                                    this.remove_history_image(path.clone(), cx)
                                                })
                                            }
                                        },
                                    ))
                                }))
                                .child(settings_button(
                                    "settings-clear-history",
                                    "Clear history",
                                    colors,
                                    is_idle,
                                    {
                                        let app = app.clone();
                                        move |_, _, cx| {
                                            app.update(cx, |this, cx| this.clear_history(cx))
                                        }
                                    },
                                )),
                        )
                    }),
            )
            .child(
                div()
                    .h(px(42.0))
                    .px_5()
                    .flex()
                    .items_center()
                    .border_t_1()
                    .border_color(colors.border)
                    .text_sm()
                    .text_color(colors.muted)
                    .child(self.status.clone()),
            )
    }
}

fn settings_section(label: &str) -> gpui::Div {
    div()
        .p_4()
        .border_1()
        .border_color(crate::theme::ThemeColors::default().border)
        .flex()
        .flex_col()
        .gap_3()
        .child(div().text_sm().child(label.to_owned()))
}

fn settings_row(label: &str) -> gpui::Div {
    div().flex().items_center().justify_between().gap_3().child(
        div()
            .text_sm()
            .text_color(crate::theme::ThemeColors::default().muted)
            .child(label.to_owned()),
    )
}

fn settings_button(
    id: impl Into<gpui::ElementId>,
    label: &str,
    colors: crate::theme::ThemeColors,
    enabled: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .px_3()
        .py_1()
        .border_1()
        .border_color(colors.border)
        .text_sm()
        .text_color(if enabled { colors.text } else { colors.muted })
        .when(enabled, |button| button.cursor_pointer().on_click(on_click))
        .child(label.to_owned())
}

fn settings_toggle(
    id: impl Into<gpui::ElementId>,
    enabled_value: bool,
    colors: crate::theme::ThemeColors,
    enabled: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    settings_button(
        id,
        if enabled_value { "On" } else { "Off" },
        colors,
        enabled,
        on_click,
    )
    .bg(if enabled_value {
        colors.accent
    } else {
        colors.panel
    })
    .text_color(if enabled_value {
        colors.background
    } else {
        colors.text
    })
}
