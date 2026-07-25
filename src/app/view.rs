//! The small, on-demand settings window for the background capture service.

use gpui::{ObjectFit, Window, div, img, prelude::*, px, rgba};

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
        let recent_history: Vec<_> = self.history.entries().iter().take(5).cloned().collect();
        let history_entries: Vec<_> = recent_history
            .into_iter()
            .map(|entry| {
                let thumbnail = self.history_thumbnail(&entry.path, cx);
                (entry, thumbnail)
            })
            .collect();
        let app = cx.entity();

        div()
            .size_full()
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .bg(colors.background)
            .text_color(colors.text)
            .child(settings_header(
                colors,
                is_idle,
                self.delayed_capture_remaining_seconds,
                cx,
            ))
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
                            .child(settings_page_intro(self.settings_section, colors))
                            .when(
                                self.settings_section == SettingsSection::Capture,
                                |content| {
                                    content.child(capture_settings(
                                        self,
                                        colors,
                                        is_idle,
                                        app.clone(),
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
    is_idle: bool,
    delayed_capture_remaining_seconds: Option<u8>,
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
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .id("settings-capture")
                        .px_3()
                        .py_1()
                        .bg(if is_idle { colors.accent } else { colors.panel })
                        .text_sm()
                        .text_color(if is_idle {
                            colors.background
                        } else {
                            colors.muted
                        })
                        .when(is_idle, |button| {
                            button
                                .cursor_pointer()
                                .hover(|style| style.bg(rgba(0x81D4FAFF)))
                                .on_click(cx.listener(|this, _, _, cx| this.start_capture(cx)))
                        })
                        .child(capture_command_label(delayed_capture_remaining_seconds)),
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
                ),
        )
}

/// Labels the header command so a queued delayed capture can be cancelled in place.
fn capture_command_label(delayed_capture_remaining_seconds: Option<u8>) -> &'static str {
    if delayed_capture_remaining_seconds.is_some() {
        "Cancel delay"
    } else {
        "Capture"
    }
}

fn capture_settings(
    app_state: &FlashShotApp,
    colors: crate::theme::ThemeColors,
    is_idle: bool,
    app: gpui::Entity<FlashShotApp>,
) -> gpui::Div {
    settings_section("Capture behavior", colors)
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
        .child(
            settings_row("Global shortcut", colors).child(settings_toggle(
                "settings-shortcut-enabled",
                app_state.capture_shortcut_enabled,
                colors,
                is_idle,
                {
                    let app = app.clone();
                    move |_, _, cx| app.update(cx, |this, cx| this.toggle_capture_shortcut(cx))
                },
            )),
        )
        .child(
            settings_row("Include cursor", colors).child(settings_toggle(
                "settings-cursor",
                app_state.include_cursor,
                colors,
                is_idle,
                {
                    let app = app.clone();
                    move |_, _, cx| app.update(cx, |this, cx| this.toggle_capture_cursor(cx))
                },
            )),
        )
        .child(settings_row("Shortcut", colors).child(
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
            settings_row("Capture delay", colors).child(div().flex().gap_1().children(
                [0, 3, 5, 10].map(|delay_seconds| {
                    let app = app.clone();
                    settings_delay_button(
                        format!("settings-delay-{delay_seconds}"),
                        delay_seconds,
                        app_state.capture_delay_seconds == delay_seconds,
                        colors,
                        is_idle,
                        move |_, _, cx| {
                            app.update(cx, |this, cx| this.set_capture_delay(delay_seconds, cx))
                        },
                    )
                }),
            )),
        )
        .child(
            settings_row("Color copy format", colors).child(settings_button(
                "settings-color-format",
                app_state.color_format_label(),
                colors,
                is_idle,
                move |_, _, cx| app.update(cx, |this, cx| this.cycle_color_format(cx)),
            )),
        )
}

fn file_settings(
    app_state: &FlashShotApp,
    colors: crate::theme::ThemeColors,
    is_idle: bool,
    app: gpui::Entity<FlashShotApp>,
) -> gpui::Div {
    settings_section("Open and history", colors).child(
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
    settings_section("Recording", colors)
        .child(settings_row("Display", colors).child(settings_button(
            "settings-recording-display",
            display,
            colors,
            !recording_active && !recording_starting,
            {
                let app = app.clone();
                move |_, _, cx| app.update(cx, |this, cx| this.cycle_recording_display(cx))
            },
        )))
        .child(settings_row("Audio", colors).child(settings_button(
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
    settings_section("System", colors)
        .child(settings_row("Appearance", colors).child(settings_button(
            "settings-theme-mode",
            app_state.settings.theme_mode.label(),
            colors,
            true,
            {
                let app = app.clone();
                move |_, _, cx| app.update(cx, |this, cx| this.toggle_theme_mode(cx))
            },
        )))
        .child(
            settings_row("Start with Windows", colors).child(settings_toggle(
                "settings-auto-start",
                app_state.auto_start_enabled,
                colors,
                true,
                {
                    let app = app.clone();
                    move |_, _, cx| app.update(cx, |this, cx| this.toggle_auto_start(cx))
                },
            )),
        )
        .child(settings_row("Updates", colors).child(settings_button(
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
    entries: Vec<(
        crate::history::HistoryEntry,
        Option<std::sync::Arc<gpui::RenderImage>>,
    )>,
    colors: crate::theme::ThemeColors,
    is_idle: bool,
    app: gpui::Entity<FlashShotApp>,
) -> gpui::Div {
    let now_ms = current_timestamp_ms();
    settings_section("Recent captures", colors)
        .children(entries.into_iter().map(|(entry, thumbnail)| {
            let label = history_entry_label(&entry, now_ms);
            history_row(&label, thumbnail, colors)
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
                    format!("settings-copy-history-{}", entry.created_at_ms),
                    "Copy",
                    colors,
                    is_idle,
                    {
                        let app = app.clone();
                        let path = entry.path.clone();
                        move |_, _, cx| {
                            app.update(cx, |this, cx| this.copy_history_image(path.clone(), cx))
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

/// Renders a fixed preview well so history metadata and actions stay aligned while it loads.
fn history_row(
    label: &str,
    thumbnail: Option<std::sync::Arc<gpui::RenderImage>>,
    colors: crate::theme::ThemeColors,
) -> gpui::Div {
    settings_row(label, colors).child(
        div()
            .w(px(72.0))
            .h(px(46.0))
            .flex_none()
            .overflow_hidden()
            .border_1()
            .border_color(colors.border)
            .bg(colors.panel)
            .when_some(thumbnail, |preview, thumbnail| {
                preview.child(img(thumbnail).size_full().object_fit(ObjectFit::Cover))
            }),
    )
}

fn current_timestamp_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

/// Adds a concise age to a history item so users can scan recent captures quickly.
fn history_entry_label(entry: &crate::history::HistoryEntry, now_ms: u128) -> String {
    let name = entry
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Capture");
    format!(
        "{name} - {}",
        relative_timestamp_label(entry.created_at_ms, now_ms)
    )
}

fn relative_timestamp_label(created_at_ms: u128, now_ms: u128) -> String {
    let elapsed_seconds = now_ms.saturating_sub(created_at_ms) / 1_000;
    match elapsed_seconds {
        0..=59 => "Just now".to_owned(),
        60..=3_599 => format!("{}m ago", elapsed_seconds / 60),
        3_600..=86_399 => format!("{}h ago", elapsed_seconds / 3_600),
        _ => format!("{}d ago", elapsed_seconds / 86_400),
    }
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
        .border_l_2()
        .border_color(if active {
            colors.accent
        } else {
            colors.background
        })
        .text_sm()
        .cursor_pointer()
        .bg(if active {
            colors.panel
        } else {
            colors.background
        })
        .text_color(if active { colors.text } else { colors.muted })
        .hover(|style| style.bg(colors.panel).text_color(colors.text))
        .on_click(move |_, _, cx| {
            app.update(cx, |this, cx| this.select_settings_section(section, cx))
        })
        .child(label)
}

/// Gives every settings section a stable title and a short task-oriented summary.
fn settings_page_intro(section: SettingsSection, colors: crate::theme::ThemeColors) -> gpui::Div {
    let (title, summary) = match section {
        SettingsSection::Capture => ("Capture", "Shortcut, cursor, and export behavior"),
        SettingsSection::Files => ("Files", "Open images and manage saved captures"),
        SettingsSection::Recording => ("Recording", "Choose a source and control recording"),
        SettingsSection::System => ("System", "Startup and update preferences"),
    };
    div()
        .pb_3()
        .border_b_1()
        .border_color(colors.border)
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_lg().child(title))
        .child(div().text_sm().text_color(colors.muted).child(summary))
}

fn settings_section(label: &str, colors: crate::theme::ThemeColors) -> gpui::Div {
    div()
        .pb_5()
        .border_b_1()
        .border_color(colors.border)
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .text_sm()
                .text_color(colors.text)
                .child(label.to_owned()),
        )
}

fn settings_row(label: &str, colors: crate::theme::ThemeColors) -> gpui::Div {
    div().flex().items_center().justify_between().gap_3().child(
        div()
            .text_sm()
            .text_color(colors.muted)
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
        .bg(colors.background)
        .text_sm()
        .text_color(if enabled { colors.text } else { colors.muted })
        .when(enabled, |button| {
            button
                .cursor_pointer()
                .hover(|style| style.bg(colors.panel))
                .on_click(on_click)
        })
        .child(label.to_owned())
}

fn settings_toggle(
    id: impl Into<gpui::ElementId>,
    enabled_value: bool,
    colors: crate::theme::ThemeColors,
    enabled: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    // A fixed-size track keeps binary preferences easy to scan without letting
    // the label change shift adjacent settings rows.
    div()
        .id(id)
        .w(px(36.0))
        .h(px(20.0))
        .p(px(2.0))
        .flex()
        .items_center()
        .when(enabled_value, |toggle| toggle.justify_end())
        .rounded_full()
        .bg(if enabled_value && enabled {
            colors.accent
        } else {
            colors.panel
        })
        .border_1()
        .border_color(colors.border)
        .when(enabled, |toggle| {
            toggle
                .cursor_pointer()
                .hover(|style| style.bg(colors.accent))
                .on_click(on_click)
        })
        .child(div().size(px(14.0)).rounded_full().bg(colors.text))
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

fn settings_delay_button(
    id: impl Into<gpui::ElementId>,
    delay_seconds: u8,
    selected: bool,
    colors: crate::theme::ThemeColors,
    enabled: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let label = if delay_seconds == 0 {
        "Off".to_owned()
    } else {
        format!("{delay_seconds}s")
    };
    settings_button(id, &label, colors, enabled, on_click)
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

#[cfg(test)]
mod tests {
    use super::{
        capture_command_label, history_entry_label, relative_timestamp_label, settings_page_intro,
    };
    use crate::app::SettingsSection;
    use crate::history::HistoryEntry;
    use std::path::PathBuf;

    #[test]
    fn capture_header_turns_into_a_delay_cancellation_command() {
        assert_eq!(capture_command_label(None), "Capture");
        assert_eq!(capture_command_label(Some(3)), "Cancel delay");
    }

    #[test]
    fn every_settings_section_has_a_renderable_page_intro() {
        let colors = crate::theme::ThemeColors::default();
        for section in [
            SettingsSection::Capture,
            SettingsSection::Files,
            SettingsSection::Recording,
            SettingsSection::System,
        ] {
            let _ = settings_page_intro(section, colors);
        }
    }

    #[test]
    fn history_labels_include_a_file_name_and_human_readable_age() {
        let entry = HistoryEntry {
            path: PathBuf::from("F:/captures/example.png"),
            created_at_ms: 1_000_000,
        };

        assert_eq!(
            history_entry_label(&entry, 1_000_000),
            "example.png - Just now"
        );
        assert_eq!(relative_timestamp_label(1_000_000, 1_065_000), "1m ago");
        assert_eq!(relative_timestamp_label(1_000_000, 4_600_000), "1h ago");
        assert_eq!(relative_timestamp_label(1_000_000, 173_800_000), "2d ago");
    }
}
