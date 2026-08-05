//! The small, on-demand settings window for the background capture service.

use gpui::{
    CursorStyle, ElementInputHandler, FocusHandle, KeyDownEvent, ObjectFit, Window, canvas, div,
    img, prelude::*, px, rgba,
};

use super::{FlashShotApp, HistoryFilter, SettingsSection};
use crate::{domain::session::CaptureSessionState, platform::shortcut::CaptureShortcut};

const HISTORY_PREVIEW_LIMIT: usize = 5;

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
        let history_total = self.history.entries().len();
        let history_query = self.history_search_query().trim().to_lowercase();
        let filtered_history_total = self
            .history
            .entries()
            .iter()
            .filter(|entry| history_entry_matches(entry, self.history_filter, &history_query))
            .count();
        let history_entries: Vec<_> = visible_history_entries(
            self.history.entries(),
            self.history_expanded,
            self.history_filter,
            &history_query,
        )
        .into_iter()
        .map(|entry| {
            let thumbnail = self.history_thumbnail(&entry.path, cx);
            let deleting = self.history_deletions_in_flight.contains(&entry.path);
            (entry, thumbnail, deleting)
        })
        .collect();
        let app = cx.entity();

        div()
            .size_full()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                this.handle_history_search_key(&event.keystroke, cx);
            }))
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
                    .min_h(px(0.0))
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
                            .min_h(px(0.0))
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
                            .when(self.settings_section == SettingsSection::Files, |content| {
                                content.child(history_settings(
                                    HistoryViewState {
                                        entries: history_entries,
                                        total_entries: history_total,
                                        filtered_entries: filtered_history_total,
                                        expanded: self.history_expanded,
                                        filter: self.history_filter,
                                        clear_confirmation: self.history_clear_confirmation,
                                        clear_in_flight: self.history_clear_in_flight,
                                        retention_in_flight: self
                                            .history_retention_target
                                            .is_some(),
                                        deletion_in_flight: !self
                                            .history_deletions_in_flight
                                            .is_empty(),
                                        search_query: self.history_search_query().to_owned(),
                                        search_active: self.history_search_is_active(),
                                        search_focus: self.focus_handle.clone(),
                                    },
                                    colors,
                                    is_idle,
                                    app.clone(),
                                ))
                            }),
                    ),
            )
            .child(
                div()
                    .h(px(42.0))
                    .flex_none()
                    .px_5()
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_t_1()
                    .border_color(colors.border)
                    .text_sm()
                    .text_color(colors.muted)
                    .child(
                        div()
                            .size(px(7.0))
                            .rounded_full()
                            .bg(status_indicator_color(&self.status, is_idle, colors)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_ellipsis()
                            .child(self.status.clone()),
                    ),
            )
    }
}

/// Chooses a semantic status color so failures cannot look like a healthy idle state.
fn status_indicator_color(
    status: &str,
    is_idle: bool,
    colors: crate::theme::ThemeColors,
) -> gpui::Hsla {
    let normalized = status.to_ascii_lowercase();
    let failure = [
        "could not",
        "failed",
        "unavailable",
        "error",
        "invalid",
        "cannot",
        "not found",
        "needs attention",
    ]
    .iter()
    .any(|marker| normalized.contains(marker));
    if failure {
        return colors.danger;
    }
    let busy = [
        "checking",
        "recognizing",
        "translating",
        "capturing",
        "saving",
        "opening",
        "starting",
        "preparing",
        "updating",
    ]
    .iter()
    .any(|marker| normalized.contains(marker));
    if !is_idle || busy {
        colors.accent
    } else {
        colors.success
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
        .flex_none()
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
                .child(
                    div()
                        .text_sm()
                        .text_color(colors.muted)
                        .child("Capture center"),
                ),
        )
        .child(
            div().flex().items_center().gap_2().child(
                div()
                    .id("settings-capture")
                    .h(px(32.0))
                    .px_3()
                    .flex()
                    .items_center()
                    .rounded_sm()
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

/// Makes the configured capture shortcut and its current system registration readable together.
fn capture_shortcut_summary(shortcut: &str, registered: bool) -> String {
    if registered {
        format!("Registered: {shortcut}")
    } else {
        format!("Disabled: {shortcut}")
    }
}

fn capture_settings(
    app_state: &FlashShotApp,
    colors: crate::theme::ThemeColors,
    is_idle: bool,
    app: gpui::Entity<FlashShotApp>,
) -> gpui::Div {
    let quick_actions = settings_section("Screenshot", colors).child(
        div()
            .flex()
            .flex_wrap()
            .gap_2()
            .child(quick_action_button(
                "settings-capture-region",
                "Region capture",
                colors,
                is_idle,
                true,
                {
                    let app = app.clone();
                    move |_, _, cx| app.update(cx, |this, cx| this.start_capture(cx))
                },
            ))
            .child(quick_action_button(
                "settings-capture-full-screen",
                "Full screen",
                colors,
                is_idle,
                false,
                {
                    let app = app.clone();
                    move |_, _, cx| app.update(cx, |this, cx| this.start_full_screen_capture(cx))
                },
            ))
            .child(quick_action_button(
                "settings-capture-focused-window",
                "Focused window",
                colors,
                is_idle,
                false,
                {
                    let app = app.clone();
                    move |_, _, cx| app.update(cx, |this, cx| this.start_focused_window_capture(cx))
                },
            ))
            .child(quick_action_button(
                "settings-copy-full-screen",
                "Copy full screen",
                colors,
                is_idle,
                false,
                {
                    let app = app.clone();
                    move |_, _, cx| app.update(cx, |this, cx| this.copy_full_screen(cx))
                },
            ))
            .child(quick_action_button(
                "settings-save-full-screen",
                "Save full screen",
                colors,
                is_idle,
                false,
                {
                    let app = app.clone();
                    move |_, _, cx| app.update(cx, |this, cx| this.quick_save_full_screen(cx))
                },
            ))
            .child(quick_action_button(
                "settings-pin-full-screen",
                "Pin full screen",
                colors,
                is_idle,
                false,
                {
                    let app = app.clone();
                    move |_, _, cx| app.update(cx, |this, cx| this.pin_full_screen(cx))
                },
            ))
            .child(quick_action_button(
                "settings-pin-clipboard",
                "Pin clipboard",
                colors,
                is_idle,
                false,
                {
                    let app = app.clone();
                    move |_, _, cx| app.update(cx, |this, cx| this.pin_clipboard_image(cx))
                },
            ))
            .child(quick_action_button(
                "settings-restore-pin-input",
                "Restore pin input",
                colors,
                is_idle,
                false,
                {
                    let app = app.clone();
                    move |_, _, cx| app.update(cx, |this, cx| this.restore_pinned_window_input(cx))
                },
            )),
    );

    let preferences = settings_section("Capture preferences", colors)
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
                        .child(capture_shortcut_summary(
                            &app_state.capture_shortcut,
                            app_state.capture_shortcut_enabled,
                        )),
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
                    settings_segment_button(
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
            settings_row("Full screen key", colors).child(settings_button(
                "settings-full-screen-shortcut",
                super::workflow::shortcut_option_label(
                    app_state.settings.full_screen_shortcut.as_deref(),
                ),
                colors,
                is_idle,
                {
                    let app = app.clone();
                    move |_, _, cx| app.update(cx, |this, cx| this.cycle_full_screen_shortcut(cx))
                },
            )),
        )
        .child(
            settings_row("Focused window key", colors).child(settings_button(
                "settings-focused-window-shortcut",
                super::workflow::shortcut_option_label(
                    app_state.settings.focused_window_shortcut.as_deref(),
                ),
                colors,
                is_idle,
                {
                    let app = app.clone();
                    move |_, _, cx| {
                        app.update(cx, |this, cx| this.cycle_focused_window_shortcut(cx))
                    }
                },
            )),
        )
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
                {
                    let app = app.clone();
                    move |_, _, cx| app.update(cx, |this, cx| this.cycle_color_format(cx))
                },
            )),
        )
        .child(
            settings_row("Local OCR", colors).child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_end()
                    .gap_2()
                    .child(settings_button(
                        "settings-ocr-language",
                        super::workflow::ocr_language_label(
                            app_state.settings.ocr_language.as_deref(),
                        ),
                        colors,
                        true,
                        {
                            let app = app.clone();
                            move |_, _, cx| app.update(cx, |this, cx| this.cycle_ocr_language(cx))
                        },
                    ))
                    .child(settings_button(
                        "settings-check-ocr-support",
                        "Check support",
                        colors,
                        true,
                        {
                            let app = app.clone();
                            move |_, _, cx| app.update(cx, |this, cx| this.check_ocr_support(cx))
                        },
                    )),
            ),
        )
        .child(settings_row("Translation", colors).child(settings_button(
            "settings-check-translation-support",
            "Check configuration",
            colors,
            true,
            move |_, _, cx| app.update(cx, |this, cx| this.check_translation_support(cx)),
        )));

    div()
        .flex()
        .flex_col()
        .gap_5()
        .child(quick_actions)
        .child(preferences)
}

fn file_settings(
    app_state: &FlashShotApp,
    colors: crate::theme::ThemeColors,
    is_idle: bool,
    app: gpui::Entity<FlashShotApp>,
) -> gpui::Div {
    settings_section("Quick save", colors)
        .child(
            div()
                .text_sm()
                .text_color(colors.muted)
                .child(app_state.history.root().display().to_string()),
        )
        .child(settings_row("Save folder", colors).child(settings_button(
            "settings-quick-save-folder",
            "Choose folder",
            colors,
            is_idle,
            {
                let app = app.clone();
                move |_, _, cx| app.update(cx, |this, cx| this.choose_quick_save_directory(cx))
            },
        )))
        .child(settings_row("File name", colors).child(settings_button(
            "settings-quick-save-prefix",
            &format!("{}-timestamp", app_state.settings.quick_save_prefix),
            colors,
            is_idle,
            {
                let app = app.clone();
                move |_, _, cx| app.update(cx, |this, cx| this.cycle_quick_save_prefix(cx))
            },
        )))
        .child(
            settings_section("Open and history", colors).child(
                div()
                    .flex()
                    .flex_wrap()
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
                            move |_, _, cx| {
                                app.update(cx, |this, cx| this.open_editable_project(cx))
                            }
                        },
                    ))
                    .child(settings_button(
                        "settings-open-screenshot-folder",
                        "Open folder",
                        colors,
                        is_idle,
                        {
                            let app = app.clone();
                            move |_, _, cx| {
                                app.update(cx, |this, cx| this.open_history_directory(cx))
                            }
                        },
                    ))
                    .child(settings_button(
                        "settings-export-format",
                        &format!("Save as {}", app_state.export_format_label()),
                        colors,
                        is_idle,
                        {
                            let app = app.clone();
                            move |_, _, cx| app.update(cx, |this, cx| this.cycle_export_format(cx))
                        },
                    ))
                    .child(settings_button(
                        "settings-history-retention",
                        &app_state.history_retention_target.map_or_else(
                            || format!("Keep {}", app_state.settings.history_limit),
                            |limit| format!("Updating to {limit}..."),
                        ),
                        colors,
                        is_idle
                            && !app_state.history_clear_in_flight
                            && !app_state.history_clear_confirmation
                            && app_state.history_deletions_in_flight.is_empty()
                            && app_state.history_retention_target.is_none(),
                        move |_, _, cx| app.update(cx, |this, cx| this.cycle_history_limit(cx)),
                    )),
            ),
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
                    "settings-check-recording-support",
                    "Check support",
                    colors,
                    !recording_active && !recording_starting,
                    {
                        let app = app.clone();
                        move |_, _, cx| app.update(cx, |this, cx| this.check_recording_support(cx))
                    },
                ))
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

struct HistoryViewState {
    entries: Vec<(
        crate::history::HistoryEntry,
        Option<std::sync::Arc<gpui::RenderImage>>,
        bool,
    )>,
    total_entries: usize,
    filtered_entries: usize,
    expanded: bool,
    filter: HistoryFilter,
    clear_confirmation: bool,
    clear_in_flight: bool,
    retention_in_flight: bool,
    deletion_in_flight: bool,
    search_query: String,
    search_active: bool,
    search_focus: FocusHandle,
}

fn history_settings(
    state: HistoryViewState,
    colors: crate::theme::ThemeColors,
    is_idle: bool,
    app: gpui::Entity<FlashShotApp>,
) -> gpui::Div {
    let HistoryViewState {
        entries,
        total_entries,
        filtered_entries,
        expanded,
        filter,
        clear_confirmation,
        clear_in_flight,
        retention_in_flight,
        deletion_in_flight,
        search_query,
        search_active,
        search_focus,
    } = state;
    let now_ms = current_timestamp_ms();
    let is_empty = entries.is_empty();
    settings_section("Recent captures", colors)
        .child(history_search_box(
            &search_query,
            search_active,
            search_focus,
            colors,
            app.clone(),
        ))
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap_1()
                .children(HistoryFilter::ALL.map(|candidate| {
                    let selected = candidate == filter;
                    let filter_app = app.clone();
                    settings_segment_button(
                        format!("settings-history-filter-{}", candidate.label()),
                        candidate.label(),
                        selected,
                        colors,
                        move |_, _, cx| {
                            filter_app
                                .update(cx, |this, cx| this.select_history_filter(candidate, cx))
                        },
                    )
                })),
        )
        .when(filtered_entries > HISTORY_PREVIEW_LIMIT, |section| {
            let remaining = filtered_entries.saturating_sub(HISTORY_PREVIEW_LIMIT);
            let toggle_app = app.clone();
            let toggle_label = if expanded {
                "Show recent".to_owned()
            } else {
                format!("Show {remaining} more")
            };
            section.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(colors.muted)
                            .child(history_visibility_label(filtered_entries, expanded)),
                    )
                    .child(settings_button(
                        "settings-toggle-history-list",
                        &toggle_label,
                        colors,
                        true,
                        move |_, _, cx| {
                            toggle_app.update(cx, |this, cx| this.toggle_history_expanded(cx))
                        },
                    )),
            )
        })
        .when(is_empty, |section| {
            section.child(
                div()
                    .text_sm()
                    .text_color(colors.muted)
                    .child(empty_history_message(total_entries, filter, &search_query)),
            )
        })
        .children(entries.into_iter().map(|(entry, thumbnail, deleting)| {
            let label = history_entry_label(&entry, now_ms);
            history_row(&label, thumbnail, colors).child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(settings_button(
                        format!("settings-open-history-{}", entry.created_at_ms),
                        "Open",
                        colors,
                        is_idle && !deleting,
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
                        is_idle && !deleting,
                        {
                            let app = app.clone();
                            let path = entry.path.clone();
                            move |_, _, cx| {
                                app.update(cx, |this, cx| this.copy_history_image(path.clone(), cx))
                            }
                        },
                    ))
                    .child(settings_button(
                        format!("settings-pin-history-{}", entry.created_at_ms),
                        "Pin",
                        colors,
                        is_idle && !deleting,
                        {
                            let app = app.clone();
                            let path = entry.path.clone();
                            move |_, _, cx| {
                                app.update(cx, |this, cx| this.pin_history_image(path.clone(), cx))
                            }
                        },
                    ))
                    .child(settings_danger_button(
                        format!("settings-remove-history-{}", entry.created_at_ms),
                        if deleting { "Removing..." } else { "Remove" },
                        colors,
                        is_idle
                            && !deleting
                            && !clear_confirmation
                            && !clear_in_flight
                            && !retention_in_flight,
                        {
                            let app = app.clone();
                            let path = entry.path.clone();
                            move |_, _, cx| {
                                app.update(cx, |this, cx| {
                                    this.remove_history_image(path.clone(), cx)
                                })
                            }
                        },
                    )),
            )
        }))
        .when(!clear_confirmation, |section| {
            let clear_app = app.clone();
            section.child(settings_danger_button(
                "settings-clear-history",
                if clear_in_flight {
                    "Clearing..."
                } else {
                    "Clear history"
                },
                colors,
                is_idle
                    && total_entries > 0
                    && !clear_in_flight
                    && !retention_in_flight
                    && !deletion_in_flight,
                move |_, _, cx| clear_app.update(cx, |this, cx| this.request_history_clear(cx)),
            ))
        })
        .when(clear_confirmation, |section| {
            let confirm_app = app.clone();
            section.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(colors.muted)
                            .child(history_clear_confirmation_label(total_entries)),
                    )
                    .child(settings_danger_button(
                        "settings-confirm-clear-history",
                        "Delete captures",
                        colors,
                        is_idle && !retention_in_flight,
                        move |_, _, cx| confirm_app.update(cx, |this, cx| this.clear_history(cx)),
                    ))
                    .child(settings_button(
                        "settings-cancel-clear-history",
                        "Cancel",
                        colors,
                        true,
                        move |_, _, cx| app.update(cx, |this, cx| this.cancel_history_clear(cx)),
                    )),
            )
        })
}

fn history_search_box(
    query: &str,
    active: bool,
    focus_handle: FocusHandle,
    colors: crate::theme::ThemeColors,
    app: gpui::Entity<FlashShotApp>,
) -> gpui::Stateful<gpui::Div> {
    let input_app = app.clone();
    let input_focus = focus_handle.clone();
    let activate_app = app.clone();
    let activate_focus = focus_handle.clone();
    div()
        .id("settings-history-search")
        .h(px(36.0))
        .w_full()
        .relative()
        .px_3()
        .flex()
        .items_center()
        .gap_2()
        .rounded_sm()
        .border_1()
        .border_color(if active { colors.accent } else { colors.border })
        .bg(colors.panel)
        .text_sm()
        .cursor(CursorStyle::IBeam)
        .on_click(move |_, window, cx| {
            activate_app.update(cx, |this, cx| this.activate_history_search(cx));
            activate_focus.focus(window, cx);
        })
        .child(
            canvas(
                move |bounds, _, _| (bounds, active),
                move |bounds, (_, active), window, cx| {
                    if active {
                        window.handle_input(
                            &input_focus,
                            ElementInputHandler::new(bounds, input_app.clone()),
                            cx,
                        );
                    }
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0(),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .text_ellipsis_start()
                .text_color(if query.is_empty() {
                    colors.muted
                } else {
                    colors.text
                })
                .child(if query.is_empty() {
                    "Search captures".to_owned()
                } else {
                    query.to_owned()
                }),
        )
        .when(!query.is_empty(), |search| {
            search.child(
                div()
                    .id("settings-clear-history-search")
                    .w(px(24.0))
                    .h(px(24.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_sm()
                    .text_color(colors.muted)
                    .cursor_pointer()
                    .hover(|style| style.bg(colors.background).text_color(colors.text))
                    .on_click(move |_, _, cx| {
                        app.update(cx, |this, cx| this.clear_history_search(cx))
                    })
                    .child("X"),
            )
        })
}

/// Makes the destructive confirmation name its exact scope instead of relying on a generic warning.
fn history_clear_confirmation_label(total_entries: usize) -> String {
    format!("Delete all {total_entries} saved capture(s)?")
}

/// Separates preview metadata from its commands so narrow settings windows can wrap actions safely.
fn history_row(
    label: &str,
    thumbnail: Option<std::sync::Arc<gpui::RenderImage>>,
    colors: crate::theme::ThemeColors,
) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .pb_3()
        .border_b_1()
        .border_color(colors.border)
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
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
                .child(
                    div()
                        .flex_1()
                        .text_sm()
                        .text_color(colors.muted)
                        .child(label.to_owned()),
                ),
        )
}

fn current_timestamp_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

/// Explains where the first managed screenshot will appear before history has any entries.
fn empty_history_message(total_entries: usize, filter: HistoryFilter, query: &str) -> String {
    if total_entries == 0 {
        "Saved screenshots will appear here.".to_owned()
    } else if !query.trim().is_empty() {
        format!("No captures match \"{}\".", query.trim())
    } else {
        format!("No {} captures yet.", filter.label().to_lowercase())
    }
}

/// Returns the lightweight default history preview or the explicit full list.
/// Keeping the slice selection separate prevents unopened settings windows from decoding every thumbnail.
fn visible_history_entries(
    entries: &std::collections::VecDeque<crate::history::HistoryEntry>,
    expanded: bool,
    filter: HistoryFilter,
    query: &str,
) -> Vec<crate::history::HistoryEntry> {
    let limit = if expanded {
        usize::MAX
    } else {
        HISTORY_PREVIEW_LIMIT
    };
    entries
        .iter()
        .filter(|entry| history_entry_matches(entry, filter, query))
        .take(limit)
        .cloned()
        .collect()
}

fn history_entry_matches(
    entry: &crate::history::HistoryEntry,
    filter: HistoryFilter,
    query: &str,
) -> bool {
    if !filter.matches(entry.source) {
        return false;
    }
    let query = query.trim();
    if query.is_empty() {
        return true;
    }
    entry
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_lowercase().contains(query))
        || entry.source.label().to_lowercase().contains(query)
}

/// Describes whether the history settings page is showing its bounded preview or every retained item.
fn history_visibility_label(total_entries: usize, expanded: bool) -> String {
    if expanded {
        format!("Showing all {total_entries} captures")
    } else {
        format!("Showing {HISTORY_PREVIEW_LIMIT} of {total_entries} captures")
    }
}

/// Adds a concise age to a history item so users can scan recent captures quickly.
fn history_entry_label(entry: &crate::history::HistoryEntry, now_ms: u128) -> String {
    let name = entry
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Capture");
    format!(
        "{name} - {} - {}",
        entry.source.label(),
        relative_timestamp_label(entry.created_at_ms, now_ms),
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
                "Actions",
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

/// Gives every section a stable task-oriented title.
fn settings_page_intro(section: SettingsSection, colors: crate::theme::ThemeColors) -> gpui::Div {
    let title = match section {
        SettingsSection::Capture => "Quick actions",
        SettingsSection::Files => "Files",
        SettingsSection::Recording => "Recording",
        SettingsSection::System => "System",
    };
    div()
        .pb_3()
        .border_b_1()
        .border_color(colors.border)
        .flex()
        .flex_col()
        .child(div().text_lg().child(title))
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

/// Keeps a preference label and its control readable when the settings window narrows.
fn settings_row(label: &str, colors: crate::theme::ThemeColors) -> gpui::Div {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
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
        .h(px(32.0))
        .px_3()
        .flex()
        .items_center()
        .justify_center()
        .rounded_sm()
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

fn settings_danger_button(
    id: impl Into<gpui::ElementId>,
    label: &str,
    colors: crate::theme::ThemeColors,
    enabled: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    settings_button(id, label, colors, enabled, on_click)
        .border_color(if enabled {
            colors.danger
        } else {
            colors.border
        })
        .text_color(if enabled { colors.danger } else { colors.muted })
}

fn quick_action_button(
    id: impl Into<gpui::ElementId>,
    label: &str,
    colors: crate::theme::ThemeColors,
    enabled: bool,
    primary: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h(px(42.0))
        .when(primary, |button| button.w_full())
        .when(!primary, |button| button.flex_1().min_w(px(140.0)))
        .px_3()
        .flex()
        .items_center()
        .justify_center()
        .rounded_sm()
        .border_1()
        .border_color(if primary {
            colors.accent
        } else {
            colors.border
        })
        .bg(if primary && enabled {
            colors.accent
        } else {
            colors.panel
        })
        .text_sm()
        .text_color(if primary && enabled {
            colors.background
        } else if enabled {
            colors.text
        } else {
            colors.muted
        })
        .when(enabled, |button| {
            button
                .cursor_pointer()
                .hover(move |style| {
                    style.bg(if primary {
                        gpui::Hsla::from(rgba(0x81D4FAFF))
                    } else {
                        colors.background
                    })
                })
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

fn settings_segment_button(
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
        capture_command_label, capture_shortcut_summary, history_clear_confirmation_label,
        history_entry_label, history_entry_matches, history_visibility_label,
        relative_timestamp_label, settings_page_intro, status_indicator_color,
        visible_history_entries,
    };
    use crate::app::{HistoryFilter, SettingsSection};
    use crate::history::{HistoryEntry, HistorySource};
    use crate::theme::ThemeColors;
    use std::collections::VecDeque;
    use std::path::PathBuf;

    #[test]
    fn capture_header_turns_into_a_delay_cancellation_command() {
        assert_eq!(capture_command_label(None), "Capture");
        assert_eq!(capture_command_label(Some(3)), "Cancel delay");
    }

    #[test]
    fn shortcut_summary_distinguishes_registered_and_disabled_keys() {
        assert_eq!(
            capture_shortcut_summary("Ctrl+Alt+S", true),
            "Registered: Ctrl+Alt+S"
        );
        assert_eq!(
            capture_shortcut_summary("Ctrl+Alt+S", false),
            "Disabled: Ctrl+Alt+S"
        );
    }

    #[test]
    fn history_clear_confirmation_names_the_exact_number_of_captures() {
        assert_eq!(
            history_clear_confirmation_label(12),
            "Delete all 12 saved capture(s)?"
        );
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
    fn empty_history_section_explains_when_saved_captures_appear() {
        assert_eq!(
            super::empty_history_message(0, HistoryFilter::All, ""),
            "Saved screenshots will appear here."
        );
        assert_eq!(
            super::empty_history_message(3, HistoryFilter::Pinned, ""),
            "No pinned captures yet."
        );
        assert_eq!(
            super::empty_history_message(3, HistoryFilter::All, "invoice"),
            "No captures match \"invoice\"."
        );
    }

    #[test]
    fn settings_rows_render_for_compact_and_wide_labels() {
        let colors = crate::theme::ThemeColors::default();
        let _ = super::settings_row("Audio", colors);
        let _ = super::settings_row("Start with Windows", colors);
    }

    #[test]
    fn status_indicator_colors_follow_operation_outcomes() {
        let colors = ThemeColors::default();
        assert_eq!(
            status_indicator_color("Saved screenshot", true, colors),
            colors.success
        );
        assert_eq!(
            status_indicator_color("Capturing virtual desktop...", true, colors),
            colors.accent
        );
        assert_eq!(
            status_indicator_color("Could not save screenshot", true, colors),
            colors.danger
        );
    }

    #[test]
    fn history_labels_include_a_file_name_and_human_readable_age() {
        let entry = HistoryEntry {
            path: PathBuf::from("F:/captures/example.png"),
            created_at_ms: 1_000_000,
            source: HistorySource::Selection,
        };

        assert_eq!(
            history_entry_label(&entry, 1_000_000),
            "example.png - Selection - Just now"
        );
        assert_eq!(relative_timestamp_label(1_000_000, 1_065_000), "1m ago");
        assert_eq!(relative_timestamp_label(1_000_000, 4_600_000), "1h ago");
        assert_eq!(relative_timestamp_label(1_000_000, 173_800_000), "2d ago");
    }

    #[test]
    fn history_preview_expands_only_after_an_explicit_request() {
        let entries = (0..7)
            .map(|created_at_ms| HistoryEntry {
                path: PathBuf::from(format!("F:/captures/{created_at_ms}.png")),
                created_at_ms,
                source: HistorySource::Unknown,
            })
            .collect::<VecDeque<_>>();

        assert_eq!(
            visible_history_entries(&entries, false, HistoryFilter::All, "").len(),
            5
        );
        assert_eq!(
            visible_history_entries(&entries, true, HistoryFilter::All, "").len(),
            7
        );
        assert_eq!(
            history_visibility_label(7, false),
            "Showing 5 of 7 captures"
        );
        assert_eq!(history_visibility_label(7, true), "Showing all 7 captures");
    }

    #[test]
    fn history_source_filter_applies_before_the_preview_limit() {
        let entries = (0..12)
            .map(|created_at_ms| HistoryEntry {
                path: PathBuf::from(format!("F:/captures/{created_at_ms}.png")),
                created_at_ms,
                source: if created_at_ms % 2 == 0 {
                    HistorySource::Pinned
                } else {
                    HistorySource::Selection
                },
            })
            .collect::<VecDeque<_>>();

        let preview = visible_history_entries(&entries, false, HistoryFilter::Pinned, "");
        assert_eq!(preview.len(), 5);
        assert!(
            preview
                .iter()
                .all(|entry| entry.source == HistorySource::Pinned)
        );
        assert_eq!(
            visible_history_entries(&entries, true, HistoryFilter::Pinned, "").len(),
            6
        );
    }

    #[test]
    fn history_search_matches_file_names_and_source_labels() {
        let entry = HistoryEntry {
            path: PathBuf::from("F:/captures/Quarterly-Report.png"),
            created_at_ms: 1,
            source: HistorySource::Pinned,
        };

        assert!(history_entry_matches(
            &entry,
            HistoryFilter::All,
            "quarterly"
        ));
        assert!(history_entry_matches(&entry, HistoryFilter::All, "pinned"));
        assert!(!history_entry_matches(
            &entry,
            HistoryFilter::Selection,
            "quarterly"
        ));
        assert!(!history_entry_matches(
            &entry,
            HistoryFilter::All,
            "invoice"
        ));
    }
}
