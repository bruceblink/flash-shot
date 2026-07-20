//! The small, on-demand settings window for the background capture service.

use gpui::{Window, div, prelude::*, px};

use super::{FlashShotApp, SettingsSection};
use crate::{domain::session::CaptureSessionState, platform::shortcut::CaptureShortcut};

impl gpui::Render for FlashShotApp {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let colors = self.colors;
        let is_idle = self.session.state() == CaptureSessionState::Idle;
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
            .child(settings_header(colors, cx))
            .child(
                div()
                    .id("settings-workspace")
                    .flex_1()
                    .flex()
                    .child(settings_navigation(
                        self.settings_section,
                        colors,
                        app.clone(),
                    ))
                    .child(
                        div()
                            .id("settings-content")
                            .flex_1()
                            .overflow_y_scroll()
                            .p_5()
                            .flex()
                            .flex_col()
                            .gap_5()
                            .when(
                                self.settings_section == SettingsSection::Capture,
                                |content| {
                                    content.child(capture_settings(
                                        self,
                                        colors,
                                        is_idle,
                                        app.clone(),
                                        cx,
                                    ))
                                },
                            )
                            .when(self.settings_section == SettingsSection::Files, |content| {
                                content.child(file_settings(self, colors, is_idle, app.clone()))
                            })
                            .when(
                                self.settings_section == SettingsSection::Recording,
                                |content| {
                                    content.child(recording_settings(
                                        colors,
                                        recording_active,
                                        recording_starting,
                                        recording_paused,
                                        &recording_display,
                                        &recording_audio,
                                        app.clone(),
                                    ))
                                },
                            )
                            .when(
                                self.settings_section == SettingsSection::System,
                                |content| content.child(system_settings(self, colors, app.clone())),
                            )
                            .when(
                                self.settings_section == SettingsSection::Files
                                    && !history_entries.is_empty(),
                                |content| {
                                    content.child(history_settings(
                                        history_entries,
                                        colors,
                                        is_idle,
                                        app.clone(),
                                    ))
                                },
                            ),
                    ),
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

fn settings_header(
    colors: crate::theme::ThemeColors,
    cx: &mut gpui::Context<FlashShotApp>,
) -> gpui::Div {
    div()
        .h(px(56.0))
        .px_5()
        .flex()
        .items_center()
        .justify_between()
        .border_b_1()
        .border_color(colors.border)
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_lg().child("Flash Shot"))
                .child(div().text_sm().text_color(colors.muted).child("Settings")),
        )
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
        )
}

fn capture_settings(
    app_state: &FlashShotApp,
    colors: crate::theme::ThemeColors,
    is_idle: bool,
    app: gpui::Entity<FlashShotApp>,
    cx: &mut gpui::Context<FlashShotApp>,
) -> gpui::Div {
    settings_section("Capture behavior")
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
                        .child(app_state.capture_shortcut.clone()),
                ),
        )
        .child(settings_row("Global shortcut").child(settings_toggle(
            "settings-shortcut-enabled",
            app_state.capture_shortcut_enabled,
            colors,
            is_idle,
            {
                let app = app.clone();
                move |_, _, cx| app.update(cx, |this, cx| this.toggle_capture_shortcut(cx))
            },
        )))
        .child(settings_row("Include cursor").child(settings_toggle(
            "settings-cursor",
            app_state.include_cursor,
            colors,
            is_idle,
            {
                let app = app.clone();
                move |_, _, cx| app.update(cx, |this, cx| this.toggle_capture_cursor(cx))
            },
        )))
        .child(settings_row("Shortcut").child(
            div().flex().flex_wrap().justify_end().gap_2().children(
                CaptureShortcut::PRESETS.into_iter().map(|preset| {
                    let app = app.clone();
                    settings_shortcut_button(
                        format!("settings-shortcut-{preset}"),
                        preset,
                        app_state.capture_shortcut == preset,
                        colors,
                        move |_, _, cx| {
                            app.update(cx, |this, cx| this.select_capture_shortcut(preset, cx))
                        },
                    )
                }),
            ),
        ))
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
                    .when(is_idle, |button| {
                        button.on_click(cx.listener(|this, _, _, cx| this.cycle_capture_delay(cx)))
                    })
                    .child(if app_state.capture_delay_seconds == 0 {
                        "Off".to_owned()
                    } else {
                        format!("{} seconds", app_state.capture_delay_seconds)
                    }),
            ),
        )
}

fn file_settings(
    app_state: &FlashShotApp,
    colors: crate::theme::ThemeColors,
    is_idle: bool,
    app: gpui::Entity<FlashShotApp>,
) -> gpui::Div {
    settings_section("Open and history").child(
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
                    move |_, _, cx| app.update(cx, |this, cx| this.open_image(cx))
                },
            ))
            .child(settings_button(
                "settings-open-project",
                "Open Project",
                colors,
                is_idle,
                {
                    let app = app.clone();
                    move |_, _, cx| app.update(cx, |this, cx| this.open_editable_project(cx))
                },
            ))
            .child(settings_button(
                "settings-history-retention",
                &format!("Keep {}", app_state.settings.history_limit),
                colors,
                is_idle,
                move |_, _, cx| app.update(cx, |this, cx| this.cycle_history_limit(cx)),
            )),
    )
}

fn recording_settings(
    colors: crate::theme::ThemeColors,
    recording_active: bool,
    recording_starting: bool,
    recording_paused: bool,
    display: &str,
    audio: &str,
    app: gpui::Entity<FlashShotApp>,
) -> gpui::Div {
    settings_section("Recording")
        .child(settings_row("Display").child(settings_button(
            "settings-recording-display",
            display,
            colors,
            !recording_active && !recording_starting,
            {
                let app = app.clone();
                move |_, _, cx| app.update(cx, |this, cx| this.cycle_recording_display(cx))
            },
        )))
        .child(settings_row("Audio").child(settings_button(
            "settings-recording-audio",
            audio,
            colors,
            !recording_active && !recording_starting,
            {
                let app = app.clone();
                move |_, _, cx| app.update(cx, |this, cx| this.cycle_recording_audio(cx))
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
                        move |_, _, cx| app.update(cx, |this, cx| this.toggle_display_recording(cx))
                    },
                ))
                .when(recording_active && !recording_starting, |row| {
                    row.child(settings_button(
                        "settings-pause-recording",
                        if recording_paused { "Resume" } else { "Pause" },
                        colors,
                        true,
                        move |_, _, cx| app.update(cx, |this, cx| this.toggle_recording_pause(cx)),
                    ))
                }),
        )
}

fn system_settings(
    app_state: &FlashShotApp,
    colors: crate::theme::ThemeColors,
    app: gpui::Entity<FlashShotApp>,
) -> gpui::Div {
    settings_section("System")
        .child(settings_row("Start with Windows").child(settings_toggle(
            "settings-auto-start",
            app_state.auto_start_enabled,
            colors,
            true,
            {
                let app = app.clone();
                move |_, _, cx| app.update(cx, |this, cx| this.toggle_auto_start(cx))
            },
        )))
        .child(settings_row("Updates").child(settings_button(
            "settings-check-updates",
            if app_state.update_check_in_flight {
                "Checking..."
            } else {
                "Check now"
            },
            colors,
            !app_state.update_check_in_flight,
            move |_, _, cx| app.update(cx, |this, cx| this.check_for_updates(cx)),
        )))
}

fn history_settings(
    entries: Vec<crate::history::HistoryEntry>,
    colors: crate::theme::ThemeColors,
    is_idle: bool,
    app: gpui::Entity<FlashShotApp>,
) -> gpui::Div {
    settings_section("Recent captures")
        .children(entries.into_iter().map(|entry| {
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
                        app.update(cx, |this, cx| this.open_history_image(path.clone(), cx))
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
                        app.update(cx, |this, cx| this.remove_history_image(path.clone(), cx))
                    }
                },
            ))
        }))
        .child(settings_button(
            "settings-clear-history",
            "Clear history",
            colors,
            is_idle,
            move |_, _, cx| app.update(cx, |this, cx| this.clear_history(cx)),
        ))
}

fn settings_navigation(
    selected: SettingsSection,
    colors: crate::theme::ThemeColors,
    app: gpui::Entity<FlashShotApp>,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id("settings-navigation")
        .w(px(132.0))
        .p_3()
        .border_r_1()
        .border_color(colors.border)
        .flex()
        .flex_col()
        .gap_1()
        .children([
            settings_navigation_item(
                "settings-nav-capture",
                "Capture",
                SettingsSection::Capture,
                selected,
                colors,
                app.clone(),
            ),
            settings_navigation_item(
                "settings-nav-files",
                "Files",
                SettingsSection::Files,
                selected,
                colors,
                app.clone(),
            ),
            settings_navigation_item(
                "settings-nav-recording",
                "Recording",
                SettingsSection::Recording,
                selected,
                colors,
                app.clone(),
            ),
            settings_navigation_item(
                "settings-nav-system",
                "System",
                SettingsSection::System,
                selected,
                colors,
                app,
            ),
        ])
}

fn settings_navigation_item(
    id: &'static str,
    label: &'static str,
    section: SettingsSection,
    selected: SettingsSection,
    colors: crate::theme::ThemeColors,
    app: gpui::Entity<FlashShotApp>,
) -> gpui::Stateful<gpui::Div> {
    let active = selected == section;
    div()
        .id(id)
        .w_full()
        .px_3()
        .py_2()
        .text_sm()
        .cursor_pointer()
        .bg(if active {
            colors.panel
        } else {
            colors.background
        })
        .text_color(if active { colors.text } else { colors.muted })
        .on_click(move |_, _, cx| {
            app.update(cx, |this, cx| this.select_settings_section(section, cx))
        })
        .child(label)
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

fn settings_shortcut_button(
    id: impl Into<gpui::ElementId>,
    label: &str,
    selected: bool,
    colors: crate::theme::ThemeColors,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    settings_button(id, label, colors, true, on_click)
        .border_color(if selected {
            colors.accent
        } else {
            colors.border
        })
        .bg(if selected {
            colors.accent
        } else {
            colors.panel
        })
        .text_color(if selected {
            colors.background
        } else {
            colors.text
        })
}
