//! The small, on-demand settings window for the background capture service.

use gpui::{
    CursorStyle, ElementInputHandler, FocusHandle, FontWeight, KeyDownEvent, ObjectFit, Window,
    canvas, div, img, prelude::*, px,
};

use super::{
    FlashShotApp, HistoryClearScope, HistoryFilter, SettingsSection, history_entry_matches,
};
use crate::{domain::session::CaptureSessionState, platform::shortcut::CaptureShortcut};

const HISTORY_PREVIEW_LIMIT: usize = 5;
const SETTINGS_CONTENT_MAX_WIDTH: f32 = 960.0;
const COMPACT_SETTINGS_NAVIGATION_BREAKPOINT: f32 = 640.0;

type HistoryEntryView = (
    crate::history::HistoryEntry,
    Option<std::sync::Arc<gpui::RenderImage>>,
    bool,
    bool,
    bool,
);

#[derive(Clone, Copy)]
struct RecordingViewState {
    active: bool,
    starting: bool,
    stopping: bool,
    display_discovery_in_flight: bool,
    audio_discovery_in_flight: bool,
    support_check_in_flight: bool,
    paused: bool,
    progress: crate::recording::RecordingProgress,
}

impl gpui::Render for FlashShotApp {
    /// Renders the tray service's settings workspace with a readable content column.
    /// Keeping the column bounded prevents wide windows from separating a preference label from its control.
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let colors = self.colors;
        let compact_navigation =
            uses_compact_settings_navigation(f32::from(window.bounds().size.width));
        let is_idle = self.session.state() == CaptureSessionState::Idle;
        let recording_state = RecordingViewState {
            active: self.recording_control.is_some() || self.recording_acceptance_active,
            starting: self.recording_start_in_flight,
            stopping: self.recording_stopping,
            display_discovery_in_flight: self.recording_display_discovery_in_flight,
            audio_discovery_in_flight: self.recording_audio_discovery_in_flight,
            support_check_in_flight: self.recording_support_check_in_flight,
            paused: self.recording_paused,
            progress: self.recording_progress,
        };
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
        let history_selected_count = self
            .history
            .entries()
            .iter()
            .filter(|entry| self.history_selected_paths.contains(&entry.path))
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
            let selected = self.history_selected_paths.contains(&entry.path);
            let focused = self.history_keyboard_focus.as_ref() == Some(&entry.path);
            (entry, thumbnail, deleting, selected, focused)
        })
        .collect();
        let app = cx.entity();

        div()
            .size_full()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                this.handle_history_key(&event.keystroke, cx);
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
                    .when(compact_navigation, |workspace| workspace.flex_col())
                    .child(settings_navigation(
                        self.settings_section,
                        colors,
                        app.clone(),
                        compact_navigation,
                        &self.settings_navigation_focus,
                    ))
                    .child(
                        div()
                            .id("settings-content")
                            .flex_1()
                            .min_w(px(0.0))
                            .min_h(px(0.0))
                            .overflow_y_scroll()
                            .p_5()
                            .child(
                                div()
                                    .id("settings-content-column")
                                    .w_full()
                                    .min_w(px(0.0))
                                    .max_w(px(SETTINGS_CONTENT_MAX_WIDTH))
                                    .mx_auto()
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
                                    .when(
                                        self.settings_section == SettingsSection::Files,
                                        |content| {
                                            content.child(file_settings(
                                                self,
                                                colors,
                                                is_idle,
                                                app.clone(),
                                            ))
                                        },
                                    )
                                    .when(
                                        self.settings_section == SettingsSection::Recording,
                                        |content| {
                                            content.child(recording_settings(
                                                colors,
                                                recording_state,
                                                &recording_display,
                                                &recording_audio,
                                                app.clone(),
                                            ))
                                        },
                                    )
                                    .when(
                                        self.settings_section == SettingsSection::System,
                                        |content| {
                                            content.child(system_settings(
                                                self,
                                                colors,
                                                app.clone(),
                                            ))
                                        },
                                    )
                                    .when(
                                        self.settings_section == SettingsSection::Files,
                                        |content| {
                                            content.child(history_settings(
                                                HistoryViewState {
                                                    entries: history_entries,
                                                    total_entries: history_total,
                                                    filtered_entries: filtered_history_total,
                                                    expanded: self.history_expanded,
                                                    filter: self.history_filter,
                                                    clear_confirmation: self
                                                        .history_clear_confirmation,
                                                    clear_scope: self.history_clear_scope,
                                                    clear_count: self.history_clear_count,
                                                    clear_in_flight: self.history_clear_in_flight,
                                                    retention_in_flight: self
                                                        .history_retention_target
                                                        .is_some(),
                                                    deletion_in_flight: !self
                                                        .history_deletions_in_flight
                                                        .is_empty(),
                                                    search_query: self
                                                        .history_search_query()
                                                        .to_owned(),
                                                    search_active: self.history_search_is_active(),
                                                    search_focus: self.focus_handle.clone(),
                                                    selected_entries: history_selected_count,
                                                },
                                                colors,
                                                is_idle,
                                                app.clone(),
                                            ))
                                        },
                                    ),
                            ),
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

/// Switches the translation action to cancellation while its independent request is in flight.
fn translation_service_test_label(in_flight: bool) -> &'static str {
    if in_flight {
        "Cancel test"
    } else {
        "Test service"
    }
}

/// Keeps the OCR support action readable while the local capability probe is running.
fn ocr_support_check_label(in_flight: bool) -> &'static str {
    if in_flight {
        "Checking..."
    } else {
        "Check support"
    }
}

/// Switches the update action to cancellation while a manifest request is outstanding.
fn update_check_label(in_flight: bool) -> &'static str {
    if in_flight {
        "Cancel check"
    } else {
        "Check now"
    }
}

fn settings_header(
    colors: crate::theme::ThemeColors,
    is_idle: bool,
    delayed_capture_remaining_seconds: Option<u8>,
    cx: &mut gpui::Context<FlashShotApp>,
) -> gpui::Div {
    div()
        .h(px(72.0))
        .flex_none()
        .px_6()
        .flex()
        .items_center()
        .justify_between()
        .border_b_1()
        .border_color(colors.border)
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .size(px(32.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_md()
                        .bg(colors.accent)
                        .text_color(colors.background)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("F"),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_lg()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Flash Shot"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(colors.muted)
                                .child("Capture workspace"),
                        ),
                ),
        )
        .child(
            div().flex().items_center().gap_2().child(
                div()
                    .id("settings-capture")
                    .h(px(36.0))
                    .px_4()
                    .flex()
                    .items_center()
                    .rounded_md()
                    .border_1()
                    .border_color(if is_idle {
                        colors.accent
                    } else {
                        colors.border
                    })
                    .bg(if is_idle { colors.accent } else { colors.panel })
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(if is_idle {
                        colors.background
                    } else {
                        colors.muted
                    })
                    .when(is_idle, |button| {
                        button
                            .focusable()
                            .focus_visible(|style| style.border_color(colors.text))
                            .cursor_pointer()
                            .hover(|style| style.bg(colors.panel).text_color(colors.text))
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
            .w_full()
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
                        !app_state.ocr_support_check_in_flight,
                        {
                            let app = app.clone();
                            move |_, _, cx| app.update(cx, |this, cx| this.cycle_ocr_language(cx))
                        },
                    ))
                    .child(settings_button(
                        "settings-check-ocr-support",
                        ocr_support_check_label(app_state.ocr_support_check_in_flight),
                        colors,
                        !app_state.ocr_support_check_in_flight,
                        {
                            let app = app.clone();
                            move |_, _, cx| app.update(cx, |this, cx| this.check_ocr_support(cx))
                        },
                    )),
            ),
        )
        .child(settings_row("Translation", colors).child(settings_button(
            "settings-test-translation-service",
            translation_service_test_label(app_state.translation_service_test_in_flight),
            colors,
            true,
            move |_, _, cx| {
                app.update(cx, |this, cx| {
                    if this.translation_service_test_in_flight {
                        this.cancel_translation_service_test(cx);
                    } else {
                        this.test_translation_service(cx);
                    }
                })
            },
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
                .w_full()
                .min_w(px(0.0))
                .text_ellipsis_start()
                .text_sm()
                .text_color(colors.muted)
                .child(settings_path_label(app_state.history.root())),
        )
        .child(settings_row("Folder access", colors).child(settings_button(
            "settings-check-quick-save-folder",
            if app_state.quick_save_directory_check_in_flight {
                "Checking..."
            } else {
                "Check folder"
            },
            colors,
            is_idle && !app_state.quick_save_directory_check_in_flight,
            {
                let app = app.clone();
                move |_, _, cx| app.update(cx, |this, cx| this.check_quick_save_directory(cx))
            },
        )))
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
                // Own the available width so narrow windows wrap actions instead of clipping them.
                div()
                    .w_full()
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
                        &history_retention_label(
                            app_state.settings.history_limit,
                            app_state.history_retention_target,
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

/// Formats a local folder without exposing Windows' internal extended-path prefix in the UI.
fn settings_path_label(path: &std::path::Path) -> String {
    let label = path.to_string_lossy();
    label
        .strip_prefix(r"\\?\")
        .unwrap_or(label.as_ref())
        .to_owned()
}

/// Makes the retention action explicit so a user knows the number refers to saved captures.
fn history_retention_label(current_limit: u16, target: Option<u16>) -> String {
    target.map_or_else(
        || format!("Keep {current_limit} captures"),
        |limit| format!("Updating to {limit} captures..."),
    )
}

/// Renders recording choices and commands, wrapping the command row before a narrow window clips it.
fn recording_settings(
    colors: crate::theme::ThemeColors,
    state: RecordingViewState,
    display: &str,
    audio: &str,
    app: gpui::Entity<FlashShotApp>,
) -> gpui::Div {
    let source_discovery_busy = recording_source_discovery_busy(state);
    let lifecycle_busy = state.active || state.starting || state.stopping;
    let support_check_available = !lifecycle_busy && !source_discovery_busy;
    let settings_idle = support_check_available && !state.support_check_in_flight;
    let recording_toggle_enabled = !state.starting
        && !state.stopping
        && !source_discovery_busy
        && !state.support_check_in_flight;
    settings_section("Recording", colors)
        .child(settings_row("Display", colors).child(settings_button(
            "settings-recording-display",
            if state.display_discovery_in_flight {
                "Discovering..."
            } else {
                display
            },
            colors,
            settings_idle,
            {
                let app = app.clone();
                move |_, _, cx| app.update(cx, |this, cx| this.cycle_recording_display(cx))
            },
        )))
        .child(settings_row("Audio", colors).child(settings_button(
            "settings-recording-audio",
            if state.audio_discovery_in_flight {
                "Discovering..."
            } else {
                audio
            },
            colors,
            settings_idle,
            {
                let app = app.clone();
                move |_, _, cx| app.update(cx, |this, cx| this.cycle_recording_audio(cx))
            },
        )))
        .child(
            div()
                .w_full()
                .flex()
                .flex_wrap()
                .gap_2()
                .child(settings_button(
                    "settings-check-recording-support",
                    recording_support_check_label(state.support_check_in_flight),
                    colors,
                    support_check_available,
                    {
                        let app = app.clone();
                        move |_, _, cx| {
                            app.update(cx, |this, cx| {
                                if this.recording_support_check_in_flight {
                                    this.cancel_recording_support_check(cx);
                                } else {
                                    this.check_recording_support(cx);
                                }
                            })
                        }
                    },
                ))
                .child(settings_button(
                    "settings-record-display",
                    recording_toggle_label(state),
                    colors,
                    recording_toggle_enabled,
                    {
                        let app = app.clone();
                        move |_, _, cx| app.update(cx, |this, cx| this.toggle_display_recording(cx))
                    },
                ))
                .when(state.active && !state.starting && !state.stopping, |row| {
                    row.child(settings_button(
                        "settings-pause-recording",
                        if state.paused { "Resume" } else { "Pause" },
                        colors,
                        true,
                        move |_, _, cx| app.update(cx, |this, cx| this.toggle_recording_pause(cx)),
                    ))
                }),
        )
        .when(recording_status_visible(state), |section| {
            section.child(
                settings_row("Status", colors).child(
                    div()
                        .flex_1()
                        .min_w(px(160.0))
                        .text_sm()
                        .text_color(colors.text)
                        .child(recording_progress_label(
                            state.active,
                            state.starting,
                            state.stopping,
                            state.paused,
                            state.progress,
                        )),
                ),
            )
        })
}

/// Keeps the recording status row visible for every non-idle lifecycle phase, including stop.
fn recording_status_visible(state: RecordingViewState) -> bool {
    state.starting || state.active || state.stopping
}

/// Reports whether display or audio discovery is still changing the next recording input.
fn recording_source_discovery_busy(state: RecordingViewState) -> bool {
    state.display_discovery_in_flight || state.audio_discovery_in_flight
}

/// Switches FFmpeg support checking to an explicit cancellation action while probing.
fn recording_support_check_label(in_flight: bool) -> &'static str {
    if in_flight {
        "Cancel check"
    } else {
        "Check support"
    }
}

/// Gives the record command a truthful label while source discovery temporarily owns the action.
fn recording_toggle_label(state: RecordingViewState) -> &'static str {
    if state.starting {
        "Preparing..."
    } else if state.stopping {
        "Stopping..."
    } else if recording_source_discovery_busy(state) {
        "Discovering..."
    } else if state.active {
        "Stop recording"
    } else {
        "Record display"
    }
}

/// Summarizes recording lifecycle and FFmpeg progress in the settings page while a capture runs.
fn recording_progress_label(
    recording_active: bool,
    recording_starting: bool,
    recording_stopping: bool,
    recording_paused: bool,
    progress: crate::recording::RecordingProgress,
) -> String {
    if recording_starting {
        return "Preparing recording...".to_owned();
    }
    if recording_stopping {
        return "Stopping recording...".to_owned();
    }
    if !recording_active {
        return "Recording is idle".to_owned();
    }
    let seconds = progress.output_time_us.unwrap_or_default() / 1_000_000;
    let frames = progress.frame.unwrap_or_default();
    let state = if recording_paused {
        "Paused"
    } else {
        "Recording"
    };
    format!("{state} - {seconds}s, {frames} frames")
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
            update_check_label(app_state.update_check_in_flight),
            colors,
            true,
            move |_, _, cx| {
                app.update(cx, |this, cx| {
                    if this.update_check_in_flight {
                        this.cancel_update_check(cx);
                    } else {
                        this.check_for_updates(cx);
                    }
                })
            },
        )))
}

struct HistoryViewState {
    entries: Vec<HistoryEntryView>,
    total_entries: usize,
    filtered_entries: usize,
    expanded: bool,
    filter: HistoryFilter,
    clear_confirmation: bool,
    clear_scope: HistoryClearScope,
    clear_count: usize,
    clear_in_flight: bool,
    retention_in_flight: bool,
    deletion_in_flight: bool,
    search_query: String,
    search_active: bool,
    search_focus: FocusHandle,
    selected_entries: usize,
}

/// Renders search, bulk actions, and previews for recent captures.
/// Summary text and its action stay adjacent so their relationship remains clear on wide windows.
/// A pending deletion confirmation stays beside the batch actions instead of below a long list.
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
        clear_scope,
        clear_count,
        clear_in_flight,
        retention_in_flight,
        deletion_in_flight,
        search_query,
        search_active,
        search_focus,
        selected_entries,
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
                .w_full()
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
        .child(
            div()
                .text_xs()
                .text_color(colors.muted)
                .child(history_result_summary(
                    total_entries,
                    filtered_entries,
                    filter,
                    &search_query,
                )),
        )
        .when(filtered_entries > 0 || selected_entries > 0, |section| {
            let actions_enabled = is_idle
                && !clear_in_flight
                && !clear_confirmation
                && !retention_in_flight
                && !deletion_in_flight;
            let select_app = app.clone();
            let clear_selection_app = app.clone();
            let delete_selected_app = app.clone();
            section.child(
                div()
                    .w_full()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .min_w(px(100.0))
                            .text_xs()
                            .text_color(colors.muted)
                            .child(format!("{selected_entries} selected")),
                    )
                    .when(filtered_entries > 0, |bar| {
                        bar.child(settings_button(
                            "settings-select-filtered-history",
                            "Select all filtered",
                            colors,
                            actions_enabled,
                            move |_, _, cx| {
                                select_app.update(cx, |this, cx| this.select_filtered_history(cx))
                            },
                        ))
                    })
                    .when(selected_entries > 0, |bar| {
                        bar.child(settings_button(
                            "settings-clear-history-selection",
                            "Clear selection",
                            colors,
                            actions_enabled,
                            move |_, _, cx| {
                                clear_selection_app
                                    .update(cx, |this, cx| this.clear_history_selection(cx))
                            },
                        ))
                        .child(settings_danger_button(
                            "settings-delete-selected-history",
                            "Delete selected",
                            colors,
                            actions_enabled,
                            move |_, _, cx| {
                                delete_selected_app
                                    .update(cx, |this, cx| this.request_selected_history_clear(cx))
                            },
                        ))
                    }),
            )
        })
        .when(clear_confirmation, |section| {
            let confirm_app = app.clone();
            let cancel_app = app.clone();
            section.child(
                div()
                    .w_full()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(colors.muted)
                            .child(history_clear_confirmation_label(clear_count, clear_scope)),
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
                        move |_, _, cx| {
                            cancel_app.update(cx, |this, cx| this.cancel_history_clear(cx))
                        },
                    )),
            )
        })
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
                    .w_full()
                    .flex()
                    .flex_wrap()
                    .items_center()
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
        .when(
            filtered_entries > 0
                && (filter != HistoryFilter::All || !search_query.trim().is_empty()),
            |section| {
                let filtered_app = app.clone();
                let filtered_label = format!("Delete {filtered_entries} filtered");
                section.child(settings_danger_button(
                    "settings-clear-filtered-history",
                    &filtered_label,
                    colors,
                    is_idle
                        && !clear_in_flight
                        && !clear_confirmation
                        && !retention_in_flight
                        && !deletion_in_flight,
                    move |_, _, cx| {
                        filtered_app.update(cx, |this, cx| this.request_filtered_history_clear(cx))
                    },
                ))
            },
        )
        .when(is_empty, |section| {
            section.child(empty_history_state(
                empty_history_message(total_entries, filter, &search_query),
                colors,
            ))
        })
        .children(
            entries
                .into_iter()
                .map(|(entry, thumbnail, deleting, selected, focused)| {
                    let label = history_entry_label(&entry, now_ms);
                    let selection_enabled = is_idle
                        && !deleting
                        && !clear_confirmation
                        && !clear_in_flight
                        && !retention_in_flight
                        && !deletion_in_flight;
                    history_row(&label, thumbnail, selected, focused, colors).child(
                        div()
                            .w_full()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .child(history_selection_button(
                                format!("settings-select-history-{}", entry.created_at_ms),
                                if selected { "Selected" } else { "Select" },
                                selected,
                                colors,
                                selection_enabled,
                                {
                                    let app = app.clone();
                                    let path = entry.path.clone();
                                    move |_, _, cx| {
                                        app.update(cx, |this, cx| {
                                            this.toggle_history_selection(path.clone(), cx)
                                        })
                                    }
                                },
                            ))
                            .child(settings_button(
                                format!("settings-open-history-{}", entry.created_at_ms),
                                "Open",
                                colors,
                                is_idle && !deleting,
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
                                format!("settings-copy-history-{}", entry.created_at_ms),
                                "Copy",
                                colors,
                                is_idle && !deleting,
                                {
                                    let app = app.clone();
                                    let path = entry.path.clone();
                                    move |_, _, cx| {
                                        app.update(cx, |this, cx| {
                                            this.copy_history_image(path.clone(), cx)
                                        })
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
                                        app.update(cx, |this, cx| {
                                            this.pin_history_image(path.clone(), cx)
                                        })
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
                }),
        )
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
        .rounded_md()
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
                    .rounded_md()
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.panel)
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
fn history_clear_confirmation_label(count: usize, scope: HistoryClearScope) -> String {
    match scope {
        HistoryClearScope::All => format!("Delete all {count} saved capture(s)?"),
        HistoryClearScope::Filtered => format!("Delete {count} filtered saved capture(s)?"),
        HistoryClearScope::Selected => format!("Delete {count} selected saved capture(s)?"),
    }
}

/// Separates preview metadata from its commands so narrow settings windows wrap actions safely.
/// The metadata column may shrink, but it uses an ellipsis instead of clipping a filename mid-word.
fn history_row(
    label: &str,
    thumbnail: Option<std::sync::Arc<gpui::RenderImage>>,
    selected: bool,
    focused: bool,
    colors: crate::theme::ThemeColors,
) -> gpui::Div {
    div()
        .p_3()
        .flex()
        .flex_col()
        .gap_2()
        .rounded_md()
        .border_1()
        .border_color(if focused {
            colors.text
        } else if selected {
            colors.accent
        } else {
            colors.border
        })
        .bg(colors.panel)
        .hover(|style| style.border_color(colors.accent))
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
                        .rounded_sm()
                        .bg(colors.panel)
                        .when_some(thumbnail, |preview, thumbnail| {
                            preview.child(img(thumbnail).size_full().object_fit(ObjectFit::Contain))
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_ellipsis()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(colors.text)
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

/// Gives an empty or filtered history result a deliberate visual state instead of a loose label.
fn empty_history_state(message: String, colors: crate::theme::ThemeColors) -> gpui::Div {
    div()
        .p_4()
        .flex()
        .items_center()
        .rounded_md()
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel)
        .text_sm()
        .text_color(colors.muted)
        .child(message)
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

/// Describes whether the history settings page is showing its bounded preview or every retained item.
fn history_visibility_label(total_entries: usize, expanded: bool) -> String {
    if expanded {
        format!("Showing all {total_entries} captures")
    } else {
        format!("Showing {HISTORY_PREVIEW_LIMIT} of {total_entries} captures")
    }
}

/// Keeps search and source-filter feedback visible even when the preview list is short or empty.
fn history_result_summary(
    total_entries: usize,
    filtered_entries: usize,
    filter: HistoryFilter,
    query: &str,
) -> String {
    let query = query.trim();
    if !query.is_empty() {
        return format!("{} match(es) for \"{query}\"", filtered_entries);
    }
    if filter == HistoryFilter::All {
        format!("{total_entries} capture(s)")
    } else {
        format!(
            "{} {} capture(s)",
            filtered_entries,
            filter.label().to_ascii_lowercase()
        )
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

/// Switches the section picker above the content when a side rail would starve it of width.
fn uses_compact_settings_navigation(window_width: f32) -> bool {
    window_width < COMPACT_SETTINGS_NAVIGATION_BREAKPOINT
}

/// Accepts the two conventional activation keys for a focused settings destination.
///
/// Navigation items remain pointer-friendly, but keyboard users should be able to activate the
/// same destination without depending on a mouse click or a global shortcut.
fn settings_navigation_activation(keystroke: &gpui::Keystroke) -> bool {
    !keystroke.modifiers.modified() && matches!(keystroke.key.as_str(), "enter" | "space")
}

/// Maps layout-appropriate arrow keys to one step through the settings workspaces.
///
/// The compact row reads horizontally while the wide rail reads vertically, so the direction
/// follows the visible arrangement instead of forcing users to remember one global key pair.
fn settings_navigation_direction(keystroke: &gpui::Keystroke, compact: bool) -> Option<i8> {
    if keystroke.modifiers.modified() {
        return None;
    }
    match (compact, keystroke.key.as_str()) {
        (true, "left") | (false, "up") => Some(-1),
        (true, "right") | (false, "down") => Some(1),
        _ => None,
    }
}

/// Selects the next settings workspace and wraps at either end of the visible navigation list.
fn adjacent_settings_section(section: SettingsSection, direction: i8) -> SettingsSection {
    const SECTIONS: [SettingsSection; 4] = settings_sections();
    let index = settings_section_index(section);
    let next = (index as isize + isize::from(direction)).rem_euclid(SECTIONS.len() as isize);
    SECTIONS[next as usize]
}

/// Keeps the settings order in one place for traversal and focus-handle lookup.
const fn settings_sections() -> [SettingsSection; 4] {
    [
        SettingsSection::Capture,
        SettingsSection::Files,
        SettingsSection::Recording,
        SettingsSection::System,
    ]
}

fn settings_section_index(section: SettingsSection) -> usize {
    settings_sections()
        .iter()
        .position(|candidate| *candidate == section)
        .expect("every settings section must be listed in navigation")
}

/// Renders a vertical navigation rail on roomy windows and a compact section row on narrow ones.
fn settings_navigation(
    selected: SettingsSection,
    colors: crate::theme::ThemeColors,
    app: gpui::Entity<FlashShotApp>,
    compact: bool,
    focus_handles: &[FocusHandle; 4],
) -> gpui::Stateful<gpui::Div> {
    div()
        .id("settings-navigation")
        .bg(colors.panel)
        .when(compact, |navigation| {
            navigation
                .w_full()
                .flex_none()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(colors.border)
                .flex()
                .items_center()
                .gap_2()
        })
        .when(!compact, |navigation| {
            navigation
                .w(px(164.0))
                .p_4()
                .border_r_1()
                .border_color(colors.border)
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .px_2()
                        .pb_2()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(colors.muted)
                        .child("WORKFLOW"),
                )
        })
        .children(settings_navigation_items().into_iter().map(|item| {
            settings_navigation_item(
                item,
                selected,
                colors,
                app.clone(),
                compact,
                (*focus_handles).clone(),
            )
        }))
}

#[derive(Clone, Copy)]
struct SettingsNavigationItem {
    id: &'static str,
    label: &'static str,
    description: &'static str,
    section: SettingsSection,
}

/// Keeps the navigation vocabulary task-oriented so the compact and wide layouts tell the same story.
fn settings_navigation_items() -> [SettingsNavigationItem; 4] {
    [
        SettingsNavigationItem {
            id: "settings-nav-capture",
            label: "Capture",
            description: "Screenshot, annotate, export",
            section: SettingsSection::Capture,
        },
        SettingsNavigationItem {
            id: "settings-nav-files",
            label: "Library",
            description: "Saved images and history",
            section: SettingsSection::Files,
        },
        SettingsNavigationItem {
            id: "settings-nav-recording",
            label: "Record",
            description: "Screen and audio",
            section: SettingsSection::Recording,
        },
        SettingsNavigationItem {
            id: "settings-nav-system",
            label: "App",
            description: "Theme, startup, updates",
            section: SettingsSection::System,
        },
    ]
}

/// Keeps each section reachable at the minimum window size without taking a second line for detail text.
fn settings_navigation_item(
    item: SettingsNavigationItem,
    selected: SettingsSection,
    colors: crate::theme::ThemeColors,
    app: gpui::Entity<FlashShotApp>,
    compact: bool,
    focus_handles: [FocusHandle; 4],
) -> gpui::Stateful<gpui::Div> {
    let active = selected == item.section;
    let keyboard_app = app.clone();
    let item_focus = focus_handles[settings_section_index(item.section)].clone();
    div()
        .id(item.id)
        .flex()
        .when(compact, |item| {
            item.flex_1()
                .min_w(px(0.0))
                .h(px(36.0))
                .px_2()
                .items_center()
                .justify_center()
        })
        .when(!compact, |item| {
            item.w_full()
                .min_h(px(52.0))
                .px_3()
                .py_2()
                .flex_col()
                .justify_center()
                .gap_1()
        })
        .rounded_md()
        .border_1()
        .border_color(if active { colors.accent } else { colors.panel })
        .text_sm()
        .cursor_pointer()
        .bg(if active { colors.accent } else { colors.panel })
        .text_color(if active {
            colors.background
        } else {
            colors.text
        })
        .focusable()
        .track_focus(&item_focus)
        .focus_visible(|style| style.border_color(colors.accent))
        .hover(move |style| {
            style
                .bg(if active {
                    colors.accent
                } else {
                    colors.background
                })
                .border_color(if active { colors.accent } else { colors.border })
                .text_color(if active {
                    colors.background
                } else {
                    colors.text
                })
        })
        .on_key_down(move |event, window, cx| {
            if settings_navigation_activation(&event.keystroke) {
                keyboard_app.update(cx, |this, cx| {
                    this.select_settings_section(item.section, cx)
                });
            } else if let Some(direction) = settings_navigation_direction(&event.keystroke, compact)
            {
                keyboard_app.update(cx, |this, cx| {
                    let section = adjacent_settings_section(this.settings_section, direction);
                    this.select_settings_section(section, cx);
                    let target = focus_handles[settings_section_index(section)].clone();
                    target.focus(window, cx);
                });
            }
        })
        .on_click(move |_, _, cx| {
            app.update(cx, |this, cx| {
                this.select_settings_section(item.section, cx)
            })
        })
        .child(
            div()
                .min_w(px(0.0))
                .when(compact, |label| label.text_ellipsis())
                .font_weight(FontWeight::SEMIBOLD)
                .child(item.label),
        )
        .when(!compact, |element| {
            element.child(
                div()
                    .text_xs()
                    .text_color(if active {
                        colors.background.opacity(0.78)
                    } else {
                        colors.muted
                    })
                    .child(item.description),
            )
        })
}

/// Gives every section a stable task-oriented title.
fn settings_page_intro(section: SettingsSection, colors: crate::theme::ThemeColors) -> gpui::Div {
    let (title, description) = settings_page_copy(section);
    div()
        .pb_4()
        .border_b_1()
        .border_color(colors.border)
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_lg()
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(div().text_sm().text_color(colors.muted).child(description))
}

/// Returns the short heading and purpose statement shown before each settings task group.
fn settings_page_copy(section: SettingsSection) -> (&'static str, &'static str) {
    match section {
        SettingsSection::Capture => (
            "Capture",
            "Start a screenshot or adjust capture preferences.",
        ),
        SettingsSection::Files => (
            "Library",
            "Find saved captures, change output, and manage history.",
        ),
        SettingsSection::Recording => (
            "Record",
            "Choose a display, audio source, and recording controls.",
        ),
        SettingsSection::System => ("App", "Set appearance, startup, and update preferences."),
    }
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
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors.muted)
                .child(label.to_owned()),
        )
}

/// Keeps labels in a stable column so controls stay nearby on wide windows and wrap below on narrow ones.
fn settings_row(label: &str, colors: crate::theme::ThemeColors) -> gpui::Div {
    div()
        .w_full()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_3()
        .min_h(px(40.0))
        .py_1()
        .child(
            div()
                .flex_1()
                .min_w(px(160.0))
                .max_w(px(220.0))
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
        .h(px(34.0))
        .px_3()
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel)
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(if enabled { colors.text } else { colors.muted })
        .when(enabled, |button| {
            button
                .focusable()
                .focus_visible(|style| style.border_color(colors.accent))
                .cursor_pointer()
                .hover(|style| style.bg(colors.background))
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
        .h(px(44.0))
        .when(primary, |button| button.w_full())
        .when(!primary, |button| button.flex_1().min_w(px(140.0)))
        .px_3()
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
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
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(if primary && enabled {
            colors.background
        } else if enabled {
            colors.text
        } else {
            colors.muted
        })
        .when(enabled, |button| {
            button
                .focusable()
                .focus_visible(|style| style.border_color(colors.accent))
                .cursor_pointer()
                .hover(move |style| {
                    style
                        .bg(if primary {
                            colors.panel
                        } else {
                            colors.background
                        })
                        .text_color(colors.text)
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
                .focusable()
                .focus_visible(|style| style.border_color(colors.accent))
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

/// Renders a history row's binary selection state while preserving disabled-operation feedback.
fn history_selection_button(
    id: impl Into<gpui::ElementId>,
    label: &str,
    selected: bool,
    colors: crate::theme::ThemeColors,
    enabled: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h(px(34.0))
        .px_3()
        .flex()
        .items_center()
        .justify_center()
        .border_color(if selected && enabled {
            colors.accent
        } else {
            colors.border
        })
        .border_1()
        .rounded_md()
        .bg(if selected && enabled {
            colors.accent
        } else {
            colors.panel
        })
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(if selected && enabled {
            colors.background
        } else if enabled {
            colors.text
        } else {
            colors.muted
        })
        .when(enabled, |button| {
            button
                .focusable()
                .focus_visible(|style| style.border_color(colors.accent))
                .cursor_pointer()
                .hover(|style| {
                    style
                        .bg(if selected {
                            colors.accent
                        } else {
                            colors.background
                        })
                        .text_color(if selected {
                            colors.background
                        } else {
                            colors.text
                        })
                })
                .on_click(on_click)
        })
        .child(label.to_owned())
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
        RecordingViewState, adjacent_settings_section, capture_command_label,
        capture_shortcut_summary, history_clear_confirmation_label, history_entry_label,
        history_entry_matches, history_result_summary, history_retention_label,
        history_visibility_label, ocr_support_check_label, recording_progress_label,
        recording_source_discovery_busy, recording_status_visible, recording_support_check_label,
        recording_toggle_label, relative_timestamp_label, settings_navigation_activation,
        settings_navigation_direction, settings_navigation_items, settings_page_copy,
        settings_page_intro, settings_path_label, status_indicator_color,
        translation_service_test_label, update_check_label, uses_compact_settings_navigation,
        visible_history_entries,
    };
    use crate::app::{HistoryClearScope, HistoryFilter, SettingsSection};
    use crate::history::{HistoryEntry, HistorySource};
    use crate::recording::RecordingProgress;
    use crate::theme::ThemeColors;
    use std::collections::VecDeque;
    use std::path::{Path, PathBuf};

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
            history_clear_confirmation_label(12, HistoryClearScope::All),
            "Delete all 12 saved capture(s)?"
        );
        assert_eq!(
            history_clear_confirmation_label(3, HistoryClearScope::Filtered),
            "Delete 3 filtered saved capture(s)?"
        );
        assert_eq!(
            history_clear_confirmation_label(2, HistoryClearScope::Selected),
            "Delete 2 selected saved capture(s)?"
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
    fn settings_navigation_uses_task_oriented_labels() {
        let items = settings_navigation_items();
        assert_eq!(
            items.map(|item| item.label),
            ["Capture", "Library", "Record", "App"]
        );
        assert_eq!(
            items.map(|item| item.description),
            [
                "Screenshot, annotate, export",
                "Saved images and history",
                "Screen and audio",
                "Theme, startup, updates",
            ]
        );
    }

    #[test]
    fn settings_navigation_accepts_plain_enter_and_space_only() {
        for key in ["enter", "space"] {
            assert!(settings_navigation_activation(
                &gpui::Keystroke::parse(key).unwrap()
            ));
        }
        for key in ["shift-enter", "ctrl-enter", "alt-space", "cmd-space"] {
            assert!(!settings_navigation_activation(
                &gpui::Keystroke::parse(key).unwrap()
            ));
        }
    }

    #[test]
    fn settings_navigation_uses_the_visible_layout_direction() {
        assert_eq!(
            settings_navigation_direction(&gpui::Keystroke::parse("left").unwrap(), true),
            Some(-1)
        );
        assert_eq!(
            settings_navigation_direction(&gpui::Keystroke::parse("down").unwrap(), false),
            Some(1)
        );
        assert_eq!(
            settings_navigation_direction(&gpui::Keystroke::parse("right").unwrap(), false),
            None
        );
        assert_eq!(
            settings_navigation_direction(&gpui::Keystroke::parse("shift-left").unwrap(), true),
            None
        );
    }

    #[test]
    fn settings_navigation_wraps_when_traversing_past_the_first_or_last_section() {
        assert_eq!(
            adjacent_settings_section(SettingsSection::Capture, -1),
            SettingsSection::System
        );
        assert_eq!(
            adjacent_settings_section(SettingsSection::System, 1),
            SettingsSection::Capture
        );
        assert_eq!(
            adjacent_settings_section(SettingsSection::Files, 1),
            SettingsSection::Recording
        );
    }

    #[test]
    fn settings_page_copy_matches_navigation_purposes() {
        assert_eq!(
            settings_page_copy(SettingsSection::Capture),
            (
                "Capture",
                "Start a screenshot or adjust capture preferences."
            )
        );
        assert_eq!(
            settings_page_copy(SettingsSection::Files),
            (
                "Library",
                "Find saved captures, change output, and manage history."
            )
        );
        assert_eq!(
            settings_page_copy(SettingsSection::Recording),
            (
                "Record",
                "Choose a display, audio source, and recording controls."
            )
        );
        assert_eq!(
            settings_page_copy(SettingsSection::System),
            ("App", "Set appearance, startup, and update preferences.")
        );
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
    fn recording_progress_label_explains_start_pause_and_live_progress() {
        assert_eq!(
            recording_progress_label(false, true, false, false, RecordingProgress::default()),
            "Preparing recording..."
        );
        assert_eq!(
            recording_progress_label(
                true,
                false,
                false,
                false,
                RecordingProgress {
                    output_time_us: Some(2_000_000),
                    frame: Some(48),
                    finished: false,
                }
            ),
            "Recording - 2s, 48 frames"
        );
        assert_eq!(
            recording_progress_label(
                true,
                false,
                false,
                true,
                RecordingProgress {
                    output_time_us: Some(2_000_000),
                    frame: Some(48),
                    finished: false,
                }
            ),
            "Paused - 2s, 48 frames"
        );
        assert_eq!(
            recording_progress_label(false, false, true, false, RecordingProgress::default()),
            "Stopping recording..."
        );
    }

    #[test]
    fn recording_status_remains_visible_while_stopping() {
        assert!(recording_status_visible(RecordingViewState {
            active: false,
            starting: false,
            stopping: true,
            display_discovery_in_flight: false,
            audio_discovery_in_flight: false,
            support_check_in_flight: false,
            paused: false,
            progress: RecordingProgress::default(),
        }));
        assert!(!recording_status_visible(RecordingViewState {
            active: false,
            starting: false,
            stopping: false,
            display_discovery_in_flight: false,
            audio_discovery_in_flight: false,
            support_check_in_flight: false,
            paused: false,
            progress: RecordingProgress::default(),
        }));
    }

    #[test]
    fn recording_controls_explain_discovery_busy_state() {
        let state = RecordingViewState {
            active: false,
            starting: false,
            stopping: false,
            display_discovery_in_flight: true,
            audio_discovery_in_flight: false,
            support_check_in_flight: false,
            paused: false,
            progress: RecordingProgress::default(),
        };
        assert!(recording_source_discovery_busy(state));
        assert_eq!(recording_toggle_label(state), "Discovering...");
    }

    #[test]
    fn recording_support_check_label_explains_the_cancel_action() {
        assert_eq!(recording_support_check_label(false), "Check support");
        assert_eq!(recording_support_check_label(true), "Cancel check");
    }

    #[test]
    fn settings_navigation_compacts_before_the_content_column_becomes_too_narrow() {
        assert!(uses_compact_settings_navigation(639.0));
        assert!(!uses_compact_settings_navigation(640.0));
    }

    #[test]
    fn file_settings_labels_explain_retention_and_hide_internal_path_prefixes() {
        assert_eq!(history_retention_label(30, None), "Keep 30 captures");
        assert_eq!(
            history_retention_label(30, Some(100)),
            "Updating to 100 captures..."
        );
        assert_eq!(
            settings_path_label(Path::new(r"\\?\C:\captures")),
            r"C:\captures"
        );
        assert_eq!(settings_path_label(Path::new("F:/captures")), "F:/captures");
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
    fn translation_test_label_explains_its_independent_busy_state() {
        assert_eq!(translation_service_test_label(false), "Test service");
        assert_eq!(translation_service_test_label(true), "Cancel test");
    }

    #[test]
    fn ocr_support_check_label_explains_its_busy_state() {
        assert_eq!(ocr_support_check_label(false), "Check support");
        assert_eq!(ocr_support_check_label(true), "Checking...");
    }

    #[test]
    fn update_check_label_explains_the_cancel_action() {
        assert_eq!(update_check_label(false), "Check now");
        assert_eq!(update_check_label(true), "Cancel check");
    }

    #[test]
    fn history_result_summary_explains_filters_and_queries() {
        assert_eq!(
            history_result_summary(12, 12, HistoryFilter::All, ""),
            "12 capture(s)"
        );
        assert_eq!(
            history_result_summary(12, 3, HistoryFilter::Pinned, ""),
            "3 pinned capture(s)"
        );
        assert_eq!(
            history_result_summary(12, 2, HistoryFilter::All, "invoice"),
            "2 match(es) for \"invoice\""
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
        assert!(history_entry_matches(
            &entry,
            HistoryFilter::All,
            "QUARTERLY"
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
