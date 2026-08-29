//! The small, on-demand settings window for the background capture service.

use std::collections::HashSet;

use gpui::{
    CursorStyle, ElementInputHandler, FocusHandle, FontWeight, KeyDownEvent, ObjectFit, Window,
    canvas, div, img, prelude::*, px,
};

use super::{
    FlashShotApp, HistoryClearScope, HistoryFilter, SettingsSection, history_entry_matches,
};
use crate::{
    domain::session::CaptureSessionState,
    i18n::{Locale, UiText},
    platform::shortcut::CaptureShortcut,
    theme::ThemeMetrics,
};

const HISTORY_PREVIEW_LIMIT: usize = 5;
const SETTINGS_CONTENT_MAX_WIDTH: f32 = 960.0;
const COMPACT_SETTINGS_NAVIGATION_BREAKPOINT: f32 = 640.0;

type HistoryEntryView = (
    crate::history::HistoryEntry,
    Option<std::sync::Arc<gpui::RenderImage>>,
    Option<&'static str>,
    bool,
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

#[derive(Clone, Copy)]
struct RecordingDirectoryViewState<'a> {
    path: &'a str,
    custom: bool,
    check_in_flight: bool,
}

impl gpui::Render for FlashShotApp {
    /// Renders the tray service's settings workspace with a readable content column.
    /// Keeping the column bounded prevents wide windows from separating a preference label from its control.
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let colors = self.colors;
        let metrics = ThemeMetrics::default();
        let locale = self.settings.locale;
        let compact_navigation =
            uses_compact_settings_navigation(f32::from(window.bounds().size.width));
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
        let is_idle = settings_actions_available(
            self.session.state() == CaptureSessionState::Idle,
            recording_state,
        );
        let recording_audio =
            super::workflow::recording_audio_selection_label(locale, &self.recording_audio);
        let recording_display =
            super::workflow::recording_display_selection_label(locale, &self.recording_display);
        let recording_directory = super::workflow::recording_directory_for_display(
            self.settings.recording_directory.as_deref(),
        )
        .map(|path| settings_path_label(&path))
        .unwrap_or_else(|| locale.text(UiText::RecordingFolderUnavailable).to_owned());
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
        let visible_entries = visible_history_entries(
            self.history.entries(),
            self.history_expanded,
            self.history_filter,
            &history_query,
        );
        let visible_thumbnail_paths = visible_entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<HashSet<_>>();
        // Do not keep decoding records that became hidden after a filter, search, or collapse.
        super::workflow::retain_history_thumbnail_pending(
            &mut self.history_thumbnail_pending,
            &visible_thumbnail_paths,
        );
        let history_entries: Vec<_> = visible_entries
            .into_iter()
            .map(|entry| {
                let thumbnail = self.history_thumbnail(&entry.path, cx);
                let thumbnail_failed = self.history_thumbnail_failed.contains(&entry.path);
                let thumbnail_status = history_thumbnail_status(
                    locale,
                    thumbnail.is_some(),
                    thumbnail_failed,
                    self.history_thumbnail_loading.contains(&entry.path),
                );
                let deleting = self.history_deletions_in_flight.contains(&entry.path);
                let selected = self.history_selected_paths.contains(&entry.path);
                let focused = self.history_keyboard_focus.as_ref() == Some(&entry.path);
                (
                    entry,
                    thumbnail,
                    thumbnail_status,
                    thumbnail_failed,
                    deleting,
                    selected,
                    focused,
                )
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
            .bg(colors.canvas)
            .text_color(colors.text)
            .child(settings_header(
                colors,
                is_idle,
                self.delayed_capture_remaining_seconds,
                locale,
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
                        locale,
                    ))
                    .child(
                        div()
                            .id("settings-content")
                            .flex_1()
                            .min_w(px(0.0))
                            .min_h(px(0.0))
                            .overflow_y_scroll()
                            .p(px(metrics.space_4))
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
                                    .child(settings_page_intro(
                                        self.settings_section,
                                        colors,
                                        locale,
                                    ))
                                    .when(
                                        self.settings_section == SettingsSection::Capture,
                                        |content| {
                                            content.child(capture_settings(
                                                self,
                                                colors,
                                                is_idle,
                                                app.clone(),
                                                locale,
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
                                                locale,
                                            ))
                                        },
                                    )
                                    .when(
                                        self.settings_section == SettingsSection::Recording,
                                        |content| {
                                            content.child(recording_settings(
                                                locale,
                                                colors,
                                                recording_state,
                                                &recording_display,
                                                &recording_audio,
                                                RecordingDirectoryViewState {
                                                    path: &recording_directory,
                                                    custom: self
                                                        .settings
                                                        .recording_directory
                                                        .is_some(),
                                                    check_in_flight: self
                                                        .recording_directory_check_in_flight,
                                                },
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
                                                    reader_in_flight: self.history_reader.is_some(),
                                                    file_read_in_flight: self
                                                        .history_file_read_in_flight(),
                                                    mutation_pending: self
                                                        .history_mutation_pending(),
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
                                                locale,
                                            ))
                                        },
                                    ),
                            ),
                    ),
            )
            .child(
                div()
                    .h(px(metrics.status_height))
                    .flex_none()
                    .px_5()
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_t_1()
                    .border_color(colors.border)
                    .bg(colors.surface)
                    .text_sm()
                    .text_color(colors.text_muted)
                    .child(
                        div()
                            .w(px(3.0))
                            .h(px(20.0))
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

/// Chooses a semantic status color so failures and cancellations cannot look like success.
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
        "无法",
        "失败",
        "不可用",
        "错误",
        "无效",
        "找不到",
        "需要检查",
    ]
    .iter()
    .any(|marker| normalized.contains(marker));
    if failure {
        return colors.danger;
    }
    let cancelled = ["cancelled", "canceled", "已取消", "取消"]
        .iter()
        .any(|marker| normalized.contains(marker));
    if cancelled {
        return colors.muted;
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
        "正在",
        "等待",
        "读取",
        "拼接",
        "滚动",
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
fn translation_service_test_label(locale: Locale, in_flight: bool) -> &'static str {
    if in_flight {
        locale.text(UiText::TranslationServiceCancelTest)
    } else {
        locale.text(UiText::TranslationServiceTest)
    }
}

/// Keeps the OCR support action readable while the local capability probe is running.
fn ocr_support_check_label(locale: Locale, in_flight: bool) -> &'static str {
    if in_flight {
        locale.text(UiText::OcrSupportCheckInProgress)
    } else {
        locale.text(UiText::OcrSupportCheck)
    }
}

/// Switches the update action to cancellation while a manifest request is outstanding.
fn update_check_label_for_locale(locale: Locale, in_flight: bool) -> &'static str {
    if in_flight {
        locale.text(UiText::CancelCheck)
    } else {
        locale.text(UiText::CheckNow)
    }
}

fn settings_header(
    colors: crate::theme::ThemeColors,
    is_idle: bool,
    delayed_capture_remaining_seconds: Option<u8>,
    locale: Locale,
    cx: &mut gpui::Context<FlashShotApp>,
) -> gpui::Div {
    let metrics = ThemeMetrics::default();
    div()
        .h(px(metrics.header_height))
        .flex_none()
        .px_6()
        .flex()
        .items_center()
        .justify_between()
        .border_t_1()
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.surface)
        .shadow_sm()
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .size(px(36.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_sm()
                        .border_1()
                        .border_color(colors.accent)
                        .bg(colors.accent)
                        .text_color(colors.canvas)
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
                                .text_color(colors.text)
                                .child(locale.text(UiText::AppName)),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(colors.text_muted)
                                .child(locale.text(UiText::WorkspaceSubtitle)),
                        ),
                ),
        )
        .child(
            div().flex().items_center().gap_2().child(
                div()
                    .id("settings-capture")
                    .h(px(metrics.control_height))
                    .px_4()
                    .flex()
                    .items_center()
                    .rounded_sm()
                    .border_1()
                    .border_color(if is_idle {
                        colors.accent
                    } else {
                        colors.border
                    })
                    .bg(if is_idle {
                        colors.accent
                    } else {
                        colors.surface_elevated
                    })
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(if is_idle {
                        colors.canvas
                    } else {
                        colors.text_disabled
                    })
                    .when(is_idle, |button| {
                        button
                            .focusable()
                            .focus_visible(|style| style.border_color(colors.focus))
                            .cursor_pointer()
                            .hover(|style| {
                                style
                                    .bg(colors.accent_hover)
                                    .border_color(colors.accent_hover)
                                    .text_color(colors.canvas)
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.start_capture(cx)))
                    })
                    .child(capture_command_label(
                        locale,
                        delayed_capture_remaining_seconds,
                    )),
            ),
        )
}

/// Labels the header command so a queued delayed capture can be cancelled in place.
fn capture_command_label(
    locale: Locale,
    delayed_capture_remaining_seconds: Option<u8>,
) -> &'static str {
    if delayed_capture_remaining_seconds.is_some() {
        locale.text(UiText::CancelDelay)
    } else {
        locale.text(UiText::Capture)
    }
}

/// Keeps settings actions visually disabled while capture or recording owns shared resources.
fn settings_actions_available(session_idle: bool, recording_state: RecordingViewState) -> bool {
    session_idle
        && !recording_state.active
        && !recording_state.starting
        && !recording_state.stopping
}

/// Makes the configured capture shortcut and its current system registration readable together.
fn capture_shortcut_summary(locale: Locale, shortcut: &str, registered: bool) -> String {
    if registered {
        locale.format_template(UiText::RegisteredShortcut, &[("shortcut", shortcut)])
    } else {
        locale.format_template(UiText::DisabledShortcut, &[("shortcut", shortcut)])
    }
}

fn capture_settings(
    app_state: &FlashShotApp,
    colors: crate::theme::ThemeColors,
    is_idle: bool,
    app: gpui::Entity<FlashShotApp>,
    locale: Locale,
) -> gpui::Div {
    // History readers and managed saves own files that capture actions may otherwise replace.
    // Clipboard encoding owns only a frozen image snapshot, so it must not block a new capture.
    let capture_actions_enabled = is_idle
        && app_state.history_reader.is_none()
        && app_state.history_write_generation.is_none()
        && !app_state.history_root_change_in_flight;
    let quick_actions = settings_section(locale.text(UiText::Screenshot), colors).child(
        div()
            .w_full()
            .flex()
            .flex_wrap()
            .gap_2()
            .child(quick_action_button(
                "settings-capture-region",
                locale.text(UiText::RegionCapture),
                colors,
                capture_actions_enabled,
                true,
                {
                    let app = app.clone();
                    move |_, _, cx| app.update(cx, |this, cx| this.start_capture(cx))
                },
            ))
            .child(quick_action_button(
                "settings-capture-full-screen",
                locale.text(UiText::FullScreen),
                colors,
                capture_actions_enabled,
                false,
                {
                    let app = app.clone();
                    move |_, _, cx| app.update(cx, |this, cx| this.start_full_screen_capture(cx))
                },
            ))
            .child(quick_action_button(
                "settings-capture-focused-window",
                locale.text(UiText::FocusedWindow),
                colors,
                capture_actions_enabled,
                false,
                {
                    let app = app.clone();
                    move |_, _, cx| app.update(cx, |this, cx| this.start_focused_window_capture(cx))
                },
            )),
    );

    let recovery_app = app.clone();
    let preferences = settings_section(locale.text(UiText::CapturePreferences), colors)
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_sm().child(locale.text(UiText::GlobalShortcut)))
                .child(
                    div()
                        .text_sm()
                        .text_color(colors.muted)
                        .child(capture_shortcut_summary(
                            locale,
                            &app_state.capture_shortcut,
                            app_state.capture_shortcut_enabled,
                        )),
                ),
        )
        .child(
            settings_row(locale.text(UiText::GlobalShortcut), colors).child(settings_toggle(
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
            settings_row(locale.text(UiText::IncludeCursor), colors).child(settings_toggle(
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
        .child(settings_row(locale.text(UiText::Shortcut), colors).child(
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
            settings_row(locale.text(UiText::FullScreenKey), colors).child(settings_button(
                "settings-full-screen-shortcut",
                super::workflow::shortcut_option_label(
                    locale,
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
            settings_row(locale.text(UiText::FocusedWindowKey), colors).child(settings_button(
                "settings-focused-window-shortcut",
                super::workflow::shortcut_option_label(
                    locale,
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
            settings_row(locale.text(UiText::CaptureDelay), colors).child(
                div()
                    .flex()
                    .gap_1()
                    .children([0, 3, 5, 10].map(|delay_seconds| {
                        let app = app.clone();
                        settings_delay_button(
                            format!("settings-delay-{delay_seconds}"),
                            delay_seconds,
                            app_state.capture_delay_seconds == delay_seconds,
                            colors,
                            is_idle,
                            locale,
                            move |_, _, cx| {
                                app.update(cx, |this, cx| this.set_capture_delay(delay_seconds, cx))
                            },
                        )
                    })),
            ),
        )
        .child(
            settings_row(locale.text(UiText::ColorCopyFormat), colors).child(settings_button(
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
            settings_row(
                app_state.settings.locale.text(UiText::SettingsLocalOcr),
                colors,
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_end()
                    .gap_2()
                    .child(settings_button(
                        "settings-ocr-language",
                        super::workflow::ocr_language_label(
                            app_state.settings.locale,
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
                        ocr_support_check_label(
                            app_state.settings.locale,
                            app_state.ocr_support_check_in_flight,
                        ),
                        colors,
                        !app_state.ocr_support_check_in_flight,
                        {
                            let app = app.clone();
                            move |_, _, cx| app.update(cx, |this, cx| this.check_ocr_support(cx))
                        },
                    )),
            ),
        )
        .child(
            settings_row(
                app_state.settings.locale.text(UiText::SettingsTranslation),
                colors,
            )
            .child(settings_button(
                "settings-test-translation-service",
                translation_service_test_label(
                    app_state.settings.locale,
                    app_state.translation_service_test_in_flight,
                ),
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
            )),
        );

    let pin_recovery =
        settings_section(locale.text(UiText::PinRecovery), colors).child(settings_button(
            "settings-restore-pin-input",
            locale.text(UiText::RestorePinInput),
            colors,
            is_idle,
            move |_, _, cx| {
                recovery_app.update(cx, |this, cx| this.restore_pinned_window_input(cx))
            },
        ));

    div()
        .flex()
        .flex_col()
        .gap_5()
        .child(quick_actions)
        .child(preferences)
        .child(pin_recovery)
}

fn file_settings(
    app_state: &FlashShotApp,
    colors: crate::theme::ThemeColors,
    is_idle: bool,
    app: gpui::Entity<FlashShotApp>,
    locale: Locale,
) -> gpui::Div {
    settings_section(locale.text(UiText::LibraryQuickSave), colors)
        .child(
            div()
                .w_full()
                .min_w(px(0.0))
                .text_ellipsis_start()
                .text_sm()
                .text_color(colors.muted)
                .child(settings_path_label(app_state.history.root())),
        )
        .child(
            settings_row(locale.text(UiText::LibraryFolderAccess), colors).child(settings_button(
                "settings-check-quick-save-folder",
                if app_state.quick_save_directory_check_in_flight {
                    locale.text(UiText::LibraryChecking)
                } else {
                    locale.text(UiText::LibraryCheckFolder)
                },
                colors,
                is_idle
                    && !app_state.quick_save_directory_check_in_flight
                    && !app_state.history_root_change_in_flight
                    && app_state.history_write_generation.is_none(),
                {
                    let app = app.clone();
                    move |_, _, cx| app.update(cx, |this, cx| this.check_quick_save_directory(cx))
                },
            )),
        )
        .child(
            settings_row(locale.text(UiText::LibrarySaveFolder), colors).child(settings_button(
                "settings-quick-save-folder",
                locale.text(UiText::LibraryChooseFolder),
                colors,
                is_idle
                    && !app_state.history_root_change_in_flight
                    && app_state.history_write_generation.is_none()
                    && !app_state.history_file_read_in_flight()
                    && !app_state.history_mutation_pending(),
                {
                    let app = app.clone();
                    move |_, _, cx| app.update(cx, |this, cx| this.choose_quick_save_directory(cx))
                },
            )),
        )
        .child(
            settings_row(locale.text(UiText::LibraryFileName), colors).child(settings_button(
                "settings-quick-save-prefix",
                &format!("{}+time+UUIDv7", app_state.settings.quick_save_prefix),
                colors,
                is_idle,
                {
                    let app = app.clone();
                    move |_, _, cx| app.update(cx, |this, cx| this.cycle_quick_save_prefix(cx))
                },
            )),
        )
        .child(
            settings_section(locale.text(UiText::LibraryOpenAndHistory), colors).child(
                // Own the available width so narrow windows wrap actions instead of clipping them.
                div()
                    .w_full()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(settings_button(
                        "settings-open-image",
                        locale.text(UiText::LibraryOpenPng),
                        colors,
                        is_idle,
                        {
                            let app = app.clone();
                            move |_, _, cx| app.update(cx, |this, cx| this.open_image(cx))
                        },
                    ))
                    .child(settings_button(
                        "settings-open-project",
                        locale.text(UiText::LibraryOpenProject),
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
                        locale.text(UiText::LibraryOpenFolder),
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
                        &locale.format_template(
                            UiText::LibrarySaveAs,
                            &[("format", app_state.export_format_label())],
                        ),
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
                            locale,
                            app_state.settings.history_limit,
                            app_state.history_retention_target,
                        ),
                        colors,
                        is_idle
                            && app_state.history_mutation_can_start()
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
fn history_retention_label(locale: Locale, current_limit: u16, target: Option<u16>) -> String {
    let count = current_limit.to_string();
    target.map_or_else(
        || locale.format_template(UiText::LibraryKeepCaptures, &[("count", &count)]),
        |limit| {
            let count = limit.to_string();
            locale.format_template(UiText::LibraryUpdatingCaptures, &[("count", &count)])
        },
    )
}

/// Renders recording choices and commands, wrapping the command row before a narrow window clips it.
fn recording_settings(
    locale: Locale,
    colors: crate::theme::ThemeColors,
    state: RecordingViewState,
    display: &str,
    audio: &str,
    directory: RecordingDirectoryViewState<'_>,
    app: gpui::Entity<FlashShotApp>,
) -> gpui::Div {
    let metrics = ThemeMetrics::default();
    let directory_check_in_flight = directory.check_in_flight;
    let source_discovery_busy = recording_source_discovery_busy(state);
    let lifecycle_busy = state.active || state.starting || state.stopping;
    let support_check_available =
        !lifecycle_busy && !source_discovery_busy && !directory_check_in_flight;
    let settings_idle = support_check_available && !state.support_check_in_flight;
    let recording_toggle_enabled = recording_toggle_enabled(state, directory_check_in_flight);
    settings_section(locale.text(UiText::Record), colors)
        .child(
            settings_row(locale.text(UiText::RecordingSettingsDisplay), colors).child(
                settings_button(
                    "settings-recording-display",
                    if state.display_discovery_in_flight {
                        locale.text(UiText::RecordingDiscoveringAction)
                    } else {
                        display
                    },
                    colors,
                    settings_idle,
                    {
                        let app = app.clone();
                        move |_, _, cx| app.update(cx, |this, cx| this.cycle_recording_display(cx))
                    },
                ),
            ),
        )
        .child(
            settings_row(locale.text(UiText::RecordingSettingsAudio), colors).child(
                settings_button(
                    "settings-recording-audio",
                    if state.audio_discovery_in_flight {
                        locale.text(UiText::RecordingDiscoveringAction)
                    } else {
                        audio
                    },
                    colors,
                    settings_idle,
                    {
                        let app = app.clone();
                        move |_, _, cx| app.update(cx, |this, cx| this.cycle_recording_audio(cx))
                    },
                ),
            ),
        )
        .child(
            settings_row(locale.text(UiText::RecordingSettingsVideoFolder), colors).child(
                div()
                    .flex_1()
                    .min_w(px(metrics.settings_control_column_min_width))
                    .flex()
                    .flex_col()
                    .gap(px(metrics.space_2))
                    .child(
                        div()
                            .w_full()
                            .min_w(px(0.0))
                            .text_ellipsis_start()
                            .text_sm()
                            .text_color(colors.text)
                            .child(directory.path.to_owned()),
                    )
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .flex_wrap()
                            .gap(px(metrics.space_2))
                            .child(settings_button(
                                "settings-recording-folder-choose",
                                locale.text(UiText::RecordingChooseFolderAction),
                                colors,
                                settings_idle,
                                {
                                    let app = app.clone();
                                    move |_, _, cx| {
                                        app.update(cx, |this, cx| {
                                            this.choose_recording_directory(cx)
                                        })
                                    }
                                },
                            ))
                            .child(settings_button(
                                "settings-recording-folder-check",
                                if directory_check_in_flight {
                                    locale.text(UiText::RecordingCheckingFolderAction)
                                } else {
                                    locale.text(UiText::RecordingCheckFolderAction)
                                },
                                colors,
                                settings_idle,
                                {
                                    let app = app.clone();
                                    move |_, _, cx| {
                                        app.update(cx, |this, cx| {
                                            this.check_recording_directory(cx)
                                        })
                                    }
                                },
                            ))
                            .child(settings_button(
                                "settings-recording-folder-open",
                                locale.text(UiText::RecordingOpenFolderAction),
                                colors,
                                settings_idle,
                                {
                                    let app = app.clone();
                                    move |_, _, cx| {
                                        app.update(cx, |this, cx| this.open_recording_directory(cx))
                                    }
                                },
                            ))
                            .child(settings_button(
                                "settings-recording-folder-default",
                                locale.text(UiText::RecordingUseDefaultFolderAction),
                                colors,
                                settings_idle && directory.custom,
                                {
                                    let app = app.clone();
                                    move |_, _, cx| {
                                        app.update(cx, |this, cx| {
                                            this.use_default_recording_directory(cx)
                                        })
                                    }
                                },
                            )),
                    ),
            ),
        )
        .child(
            div()
                .w_full()
                .flex()
                .flex_wrap()
                .gap(px(metrics.space_2))
                .child(settings_button(
                    "settings-check-recording-support",
                    recording_support_check_label(locale, state.support_check_in_flight),
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
                    recording_toggle_label(locale, state, directory_check_in_flight),
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
                        if state.paused {
                            locale.text(UiText::RecordingResumeAction)
                        } else {
                            locale.text(UiText::RecordingPauseAction)
                        },
                        colors,
                        true,
                        move |_, _, cx| app.update(cx, |this, cx| this.toggle_recording_pause(cx)),
                    ))
                }),
        )
        .when(recording_status_visible(state), |section| {
            section.child(
                settings_row(locale.text(UiText::RecordingStatusLabel), colors).child(
                    div()
                        .flex_1()
                        .min_w(px(metrics.settings_control_column_min_width))
                        .text_sm()
                        .text_color(colors.text)
                        .child(recording_progress_label(
                            locale,
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

/// Keeps the record command available as an explicit cancellation while FFmpeg is starting.
///
/// Source discovery, support checks, and graceful shutdown still own their actions, but a pending
/// startup can be invalidated safely before a late FFmpeg process reaches the application.
fn recording_toggle_enabled(state: RecordingViewState, directory_check_in_flight: bool) -> bool {
    !state.stopping
        && !recording_source_discovery_busy(state)
        && !state.support_check_in_flight
        && !directory_check_in_flight
}

/// Switches FFmpeg support checking to an explicit cancellation action while probing.
fn recording_support_check_label(locale: Locale, in_flight: bool) -> &'static str {
    if in_flight {
        locale.text(UiText::RecordingCancelCheckAction)
    } else {
        locale.text(UiText::RecordingCheckSupportAction)
    }
}

/// Gives the record command a truthful label while source discovery temporarily owns the action.
fn recording_toggle_label(
    locale: Locale,
    state: RecordingViewState,
    directory_check_in_flight: bool,
) -> &'static str {
    if state.starting {
        locale.text(UiText::RecordingCancelStartAction)
    } else if state.stopping {
        locale.text(UiText::RecordingStoppingAction)
    } else if recording_source_discovery_busy(state) {
        locale.text(UiText::RecordingDiscoveringAction)
    } else if directory_check_in_flight {
        locale.text(UiText::RecordingCheckingFolderAction)
    } else if state.active {
        locale.text(UiText::RecordingStopAction)
    } else {
        locale.text(UiText::RecordingRecordDisplayAction)
    }
}

/// Summarizes recording lifecycle and FFmpeg progress in the settings page while a capture runs.
fn recording_progress_label(
    locale: Locale,
    recording_active: bool,
    recording_starting: bool,
    recording_stopping: bool,
    recording_paused: bool,
    progress: crate::recording::RecordingProgress,
) -> String {
    if recording_starting {
        return locale.text(UiText::RecordingProgressPreparing).to_owned();
    }
    if recording_stopping {
        return locale.text(UiText::RecordingProgressStopping).to_owned();
    }
    if !recording_active {
        return locale.text(UiText::RecordingProgressIdle).to_owned();
    }
    let seconds = progress.output_time_us.unwrap_or_default() / 1_000_000;
    let frames = progress.frame.unwrap_or_default();
    let state = if recording_paused {
        locale.text(UiText::RecordingStatePaused)
    } else {
        locale.text(UiText::RecordingStateActive)
    };
    locale.format_template(
        UiText::RecordingProgressSummary,
        &[
            ("state", state),
            ("seconds", &seconds.to_string()),
            ("frames", &frames.to_string()),
        ],
    )
}

fn system_settings(
    app_state: &FlashShotApp,
    colors: crate::theme::ThemeColors,
    app: gpui::Entity<FlashShotApp>,
) -> gpui::Div {
    let locale = app_state.settings.locale;
    let theme_label = match app_state.settings.theme_mode {
        crate::theme::ThemeMode::Dark => locale.text(UiText::Dark),
        crate::theme::ThemeMode::Light => locale.text(UiText::Light),
    };
    settings_section(locale.text(UiText::App), colors)
        .child(
            settings_row(locale.text(UiText::Appearance), colors).child(settings_button(
                "settings-theme-mode",
                theme_label,
                colors,
                true,
                {
                    let app = app.clone();
                    move |_, _, cx| app.update(cx, |this, cx| this.toggle_theme_mode(cx))
                },
            )),
        )
        .child(
            settings_row(locale.text(UiText::Language), colors).child(settings_button(
                "settings-locale",
                locale.label(),
                colors,
                true,
                {
                    let app = app.clone();
                    move |_, _, cx| app.update(cx, |this, cx| this.cycle_locale(cx))
                },
            )),
        )
        .child(
            settings_row(locale.text(UiText::StartWithWindows), colors).child(settings_toggle(
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
        .child(
            settings_row(locale.text(UiText::Updates), colors).child(settings_button(
                "settings-check-updates",
                update_check_label_for_locale(locale, app_state.update_check_in_flight),
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
            )),
        )
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
    reader_in_flight: bool,
    file_read_in_flight: bool,
    mutation_pending: bool,
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
    locale: Locale,
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
        reader_in_flight,
        file_read_in_flight,
        mutation_pending,
        retention_in_flight,
        deletion_in_flight,
        search_query,
        search_active,
        search_focus,
        selected_entries,
    } = state;
    let metrics = ThemeMetrics::default();
    let now_ms = current_timestamp_ms();
    let is_empty = entries.is_empty();
    settings_section(locale.text(UiText::LibraryRecentCaptures), colors)
        .child(history_search_box(
            &search_query,
            search_active,
            search_focus,
            colors,
            app.clone(),
            locale,
        ))
        .child(
            div()
                .w_full()
                .flex()
                .flex_wrap()
                .gap(px(metrics.space_1))
                .children(HistoryFilter::ALL.map(|candidate| {
                    let selected = candidate == filter;
                    let filter_app = app.clone();
                    settings_segment_button(
                        format!("settings-history-filter-{}", candidate.label()),
                        history_filter_label(locale, candidate),
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
                    locale,
                    total_entries,
                    filtered_entries,
                    filter,
                    &search_query,
                )),
        )
        .when(filtered_entries > 0 || selected_entries > 0, |section| {
            let selection_actions_enabled = is_idle
                && !clear_in_flight
                && !clear_confirmation
                && !retention_in_flight
                && !deletion_in_flight
                && !mutation_pending;
            let destructive_actions_enabled = selection_actions_enabled && !file_read_in_flight;
            let select_app = app.clone();
            let clear_selection_app = app.clone();
            let delete_selected_app = app.clone();
            section.child(
                div()
                    .w_full()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(metrics.library_action_gap))
                    .child(
                        div()
                            .min_w(px(metrics.library_selection_min_width))
                            .text_xs()
                            .text_color(colors.muted)
                            .child(history_selected_count_label(locale, selected_entries)),
                    )
                    .when(filtered_entries > 0, |bar| {
                        bar.child(settings_button(
                            "settings-select-filtered-history",
                            locale.text(UiText::LibrarySelectAllFiltered),
                            colors,
                            selection_actions_enabled,
                            move |_, _, cx| {
                                select_app.update(cx, |this, cx| this.select_filtered_history(cx))
                            },
                        ))
                    })
                    .when(selected_entries > 0, |bar| {
                        bar.child(settings_button(
                            "settings-clear-history-selection",
                            locale.text(UiText::LibraryClearSelection),
                            colors,
                            selection_actions_enabled,
                            move |_, _, cx| {
                                clear_selection_app
                                    .update(cx, |this, cx| this.clear_history_selection(cx))
                            },
                        ))
                        .child(settings_danger_button(
                            "settings-delete-selected-history",
                            locale.text(UiText::LibraryDeleteSelected),
                            colors,
                            destructive_actions_enabled,
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
                    .gap(px(metrics.library_action_gap))
                    .child(div().text_sm().text_color(colors.muted).child(
                        history_clear_confirmation_label(locale, clear_count, clear_scope),
                    ))
                    .child(settings_danger_button(
                        "settings-confirm-clear-history",
                        locale.text(UiText::LibraryDeleteCaptures),
                        colors,
                        is_idle
                            && !file_read_in_flight
                            && !clear_in_flight
                            && !retention_in_flight
                            && !deletion_in_flight,
                        move |_, _, cx| confirm_app.update(cx, |this, cx| this.clear_history(cx)),
                    ))
                    .child(settings_button(
                        "settings-cancel-clear-history",
                        locale.text(UiText::OverlayCancel),
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
                locale.text(UiText::LibraryShowRecent).to_owned()
            } else {
                let count = remaining.to_string();
                locale.format_template(UiText::LibraryShowMore, &[("count", &count)])
            };
            section.child(
                div()
                    .w_full()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(metrics.library_action_gap))
                    .child(
                        div()
                            .text_sm()
                            .text_color(colors.muted)
                            .child(history_visibility_label(locale, filtered_entries, expanded)),
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
                let count = filtered_entries.to_string();
                let filtered_label =
                    locale.format_template(UiText::LibraryDeleteFiltered, &[("count", &count)]);
                section.child(settings_danger_button(
                    "settings-clear-filtered-history",
                    &filtered_label,
                    colors,
                    is_idle
                        && !file_read_in_flight
                        && !clear_in_flight
                        && !clear_confirmation
                        && !retention_in_flight
                        && !deletion_in_flight
                        && !mutation_pending,
                    move |_, _, cx| {
                        filtered_app.update(cx, |this, cx| this.request_filtered_history_clear(cx))
                    },
                ))
            },
        )
        .when(is_empty, |section| {
            section.child(empty_history_state(
                empty_history_message(locale, total_entries, filter, &search_query),
                colors,
            ))
        })
        .children(entries.into_iter().map(
            |(
                entry,
                thumbnail,
                thumbnail_status,
                thumbnail_failed,
                deleting,
                selected,
                focused,
            )| {
                let label = history_entry_label(locale, &entry, now_ms);
                let selection_enabled = is_idle
                    && !deleting
                    && !clear_confirmation
                    && !clear_in_flight
                    && !retention_in_flight
                    && !deletion_in_flight
                    && !mutation_pending;
                let reader_enabled = is_idle
                    && !reader_in_flight
                    && !deleting
                    && !clear_confirmation
                    && !clear_in_flight
                    && !retention_in_flight
                    && !deletion_in_flight
                    && !mutation_pending;
                history_row(
                    &label,
                    thumbnail,
                    thumbnail_status,
                    selected,
                    focused,
                    colors,
                    metrics,
                )
                .child(
                    div()
                        .w_full()
                        .flex()
                        .flex_wrap()
                        .gap(px(metrics.library_action_gap))
                        .child(history_selection_button(
                            format!("settings-select-history-{}", entry.created_at_ms),
                            if selected {
                                locale.text(UiText::OverlaySelected)
                            } else {
                                locale.text(UiText::OverlaySelect)
                            },
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
                            locale.text(UiText::LibraryOpen),
                            colors,
                            reader_enabled,
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
                            if reader_in_flight {
                                locale.text(UiText::LibraryWorking)
                            } else {
                                locale.text(UiText::OverlayCopy)
                            },
                            colors,
                            reader_enabled,
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
                            locale.text(UiText::OverlayPin),
                            colors,
                            reader_enabled,
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
                        .when(thumbnail_failed, |actions| {
                            let retry_app = app.clone();
                            let retry_path = entry.path.clone();
                            actions.child(settings_button(
                                format!("settings-retry-history-preview-{}", entry.created_at_ms),
                                locale.text(UiText::LibraryRetryPreview),
                                colors,
                                reader_enabled,
                                move |_, _, cx| {
                                    retry_app.update(cx, |this, cx| {
                                        this.retry_history_thumbnail(retry_path.clone(), cx)
                                    })
                                },
                            ))
                        })
                        .child(settings_danger_button(
                            format!("settings-remove-history-{}", entry.created_at_ms),
                            if deleting {
                                locale.text(UiText::LibraryRemoving)
                            } else {
                                locale.text(UiText::LibraryRemove)
                            },
                            colors,
                            is_idle
                                && !file_read_in_flight
                                && !deleting
                                && !clear_confirmation
                                && !clear_in_flight
                                && !retention_in_flight
                                && !mutation_pending,
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
            },
        ))
        .when(!clear_confirmation, |section| {
            let clear_app = app.clone();
            section.child(settings_danger_button(
                "settings-clear-history",
                if clear_in_flight {
                    locale.text(UiText::LibraryClearing)
                } else {
                    locale.text(UiText::LibraryClearHistory)
                },
                colors,
                is_idle
                    && !file_read_in_flight
                    && total_entries > 0
                    && !clear_in_flight
                    && !retention_in_flight
                    && !deletion_in_flight
                    && !mutation_pending,
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
    locale: Locale,
) -> gpui::Stateful<gpui::Div> {
    let metrics = ThemeMetrics::default();
    let input_app = app.clone();
    let input_focus = focus_handle.clone();
    let activate_app = app.clone();
    let activate_focus = focus_handle.clone();
    div()
        .id("settings-history-search")
        .h(px(metrics.control_height))
        .w_full()
        .relative()
        .px(px(metrics.space_3))
        .flex()
        .items_center()
        .gap(px(metrics.space_2))
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
                    locale.text(UiText::LibrarySearchCaptures).to_owned()
                } else {
                    query.to_owned()
                }),
        )
        .when(!query.is_empty(), |search| {
            search.child(
                div()
                    .id("settings-clear-history-search")
                    .w(px(metrics.search_clear_size))
                    .h(px(metrics.search_clear_size))
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

/// Maps internal history filters to the active language without changing their stable IDs.
fn history_filter_label(locale: Locale, filter: HistoryFilter) -> &'static str {
    match filter {
        HistoryFilter::All => locale.text(UiText::LibraryFilterAll),
        HistoryFilter::Selection => locale.text(UiText::LibraryFilterSelections),
        HistoryFilter::Scrolling => locale.text(UiText::LibraryFilterScrolling),
        HistoryFilter::FullScreen => locale.text(UiText::LibraryFilterFullScreen),
        HistoryFilter::Pinned => locale.text(UiText::LibraryFilterPinned),
    }
}

/// Keeps English filter names grammatical in sentences while Chinese keeps its display wording.
fn history_filter_summary_label(locale: Locale, filter: HistoryFilter) -> String {
    let label = history_filter_label(locale, filter);
    match locale {
        Locale::English => label.to_ascii_lowercase(),
        Locale::SimplifiedChinese => label.to_owned(),
    }
}

/// Makes the destructive confirmation name its exact scope instead of relying on a generic warning.
fn history_clear_confirmation_label(
    locale: Locale,
    count: usize,
    scope: HistoryClearScope,
) -> String {
    let count = count.to_string();
    match scope {
        HistoryClearScope::All => {
            locale.format_template(UiText::LibraryClearAllConfirmation, &[("count", &count)])
        }
        HistoryClearScope::Filtered => locale.format_template(
            UiText::LibraryClearFilteredConfirmation,
            &[("count", &count)],
        ),
        HistoryClearScope::Selected => locale.format_template(
            UiText::LibraryClearSelectedConfirmation,
            &[("count", &count)],
        ),
    }
}

fn history_selected_count_label(locale: Locale, count: usize) -> String {
    let count = count.to_string();
    locale.format_template(UiText::LibrarySelectedCount, &[("count", &count)])
}

/// Chooses a localized placeholder for a history preview without hiding a successfully decoded image.
fn history_thumbnail_status(
    locale: Locale,
    has_thumbnail: bool,
    failed: bool,
    loading: bool,
) -> Option<&'static str> {
    if has_thumbnail {
        None
    } else if failed {
        Some(locale.text(UiText::LibraryPreviewUnavailable))
    } else if loading {
        Some(locale.text(UiText::LibraryPreviewLoading))
    } else {
        None
    }
}

/// Separates preview metadata from its commands so narrow settings windows wrap actions safely.
/// The metadata column may shrink, but it uses an ellipsis instead of clipping a filename mid-word.
fn history_row(
    label: &str,
    thumbnail: Option<std::sync::Arc<gpui::RenderImage>>,
    thumbnail_status: Option<&str>,
    selected: bool,
    focused: bool,
    colors: crate::theme::ThemeColors,
    metrics: ThemeMetrics,
) -> gpui::Div {
    div()
        .p(px(metrics.library_row_padding))
        .flex()
        .flex_col()
        .gap(px(metrics.library_row_gap))
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
                .gap(px(metrics.library_row_gap))
                .child(
                    div()
                        .w(px(metrics.library_thumbnail_width))
                        .h(px(metrics.library_thumbnail_height))
                        .flex_none()
                        .overflow_hidden()
                        .border_1()
                        .border_color(colors.border)
                        .rounded_sm()
                        .bg(colors.panel)
                        .when_some(thumbnail, |preview, thumbnail| {
                            preview.child(img(thumbnail).size_full().object_fit(ObjectFit::Contain))
                        })
                        .when_some(thumbnail_status, |preview, status| {
                            preview
                                .flex()
                                .items_center()
                                .justify_center()
                                .p_1()
                                .text_xs()
                                .text_color(colors.muted)
                                .child(status.to_owned())
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
fn empty_history_message(
    locale: Locale,
    total_entries: usize,
    filter: HistoryFilter,
    query: &str,
) -> String {
    if total_entries == 0 {
        locale.text(UiText::LibraryEmpty).to_owned()
    } else if !query.trim().is_empty() {
        locale.format_template(UiText::LibraryNoMatches, &[("query", query.trim())])
    } else {
        locale.format_template(
            UiText::LibraryNoFiltered,
            &[("filter", &history_filter_summary_label(locale, filter))],
        )
    }
}

/// Gives an empty or filtered history result a deliberate visual state instead of a loose label.
fn empty_history_state(message: String, colors: crate::theme::ThemeColors) -> gpui::Div {
    let metrics = ThemeMetrics::default();
    div()
        .p(px(metrics.space_4))
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
fn history_visibility_label(locale: Locale, total_entries: usize, expanded: bool) -> String {
    let count = total_entries.to_string();
    if expanded {
        locale.format_template(UiText::LibraryShowingAll, &[("count", &count)])
    } else {
        let shown = HISTORY_PREVIEW_LIMIT.to_string();
        locale.format_template(
            UiText::LibraryShowingPreview,
            &[("shown", &shown), ("count", &count)],
        )
    }
}

/// Keeps search and source-filter feedback visible even when the preview list is short or empty.
fn history_result_summary(
    locale: Locale,
    total_entries: usize,
    filtered_entries: usize,
    filter: HistoryFilter,
    query: &str,
) -> String {
    let query = query.trim();
    if !query.is_empty() {
        let count = filtered_entries.to_string();
        return locale.format_template(
            UiText::LibraryMatches,
            &[("count", &count), ("query", query)],
        );
    }
    if filter == HistoryFilter::All {
        let count = total_entries.to_string();
        locale.format_template(UiText::LibraryCaptureCount, &[("count", &count)])
    } else {
        let count = filtered_entries.to_string();
        locale.format_template(
            UiText::LibraryFilteredCaptureCount,
            &[
                ("count", &count),
                ("filter", &history_filter_summary_label(locale, filter)),
            ],
        )
    }
}

/// Adds a concise age to a history item so users can scan recent captures quickly.
fn history_entry_label(
    locale: Locale,
    entry: &crate::history::HistoryEntry,
    now_ms: u128,
) -> String {
    let name = entry
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(locale.text(UiText::LibraryEntryFallback));
    format!(
        "{name} - {} - {}",
        history_source_label(locale, entry.source),
        relative_timestamp_label(locale, entry.created_at_ms, now_ms),
    )
}

fn history_source_label(locale: Locale, source: crate::history::HistorySource) -> &'static str {
    match source {
        crate::history::HistorySource::Unknown => locale.text(UiText::LibrarySourceSavedCapture),
        crate::history::HistorySource::Selection => locale.text(UiText::LibrarySourceSelection),
        crate::history::HistorySource::Scrolling => locale.text(UiText::LibrarySourceScrolling),
        crate::history::HistorySource::FullScreen => locale.text(UiText::LibrarySourceFullScreen),
        crate::history::HistorySource::Pinned => locale.text(UiText::LibrarySourcePinned),
    }
}

fn relative_timestamp_label(locale: Locale, created_at_ms: u128, now_ms: u128) -> String {
    let elapsed_seconds = now_ms.saturating_sub(created_at_ms) / 1_000;
    match elapsed_seconds {
        0..=59 => locale.text(UiText::LibraryJustNow).to_owned(),
        60..=3_599 => {
            let count = (elapsed_seconds / 60).to_string();
            locale.format_template(UiText::LibraryMinutesAgo, &[("count", &count)])
        }
        3_600..=86_399 => {
            let count = (elapsed_seconds / 3_600).to_string();
            locale.format_template(UiText::LibraryHoursAgo, &[("count", &count)])
        }
        _ => {
            let count = (elapsed_seconds / 86_400).to_string();
            locale.format_template(UiText::LibraryDaysAgo, &[("count", &count)])
        }
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
    locale: Locale,
) -> gpui::Stateful<gpui::Div> {
    let metrics = ThemeMetrics::default();
    div()
        .id("settings-navigation")
        .bg(colors.surface)
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
                .w(px(metrics.navigation_width))
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
                        .text_color(colors.text_muted)
                        .child(locale.text(UiText::Workflow)),
                )
        })
        .children(
            settings_navigation_items_for_locale(locale)
                .into_iter()
                .map(|item| {
                    settings_navigation_item(
                        item,
                        selected,
                        colors,
                        app.clone(),
                        compact,
                        (*focus_handles).clone(),
                    )
                }),
        )
}

#[derive(Clone, Copy)]
struct SettingsNavigationItem {
    id: &'static str,
    label: &'static str,
    description: &'static str,
    section: SettingsSection,
}

/// Keeps the navigation vocabulary task-oriented so the compact and wide layouts tell the same story.
fn settings_navigation_items_for_locale(locale: Locale) -> [SettingsNavigationItem; 4] {
    [
        SettingsNavigationItem {
            id: "settings-nav-capture",
            label: locale.text(UiText::Capture),
            description: locale.text(UiText::CaptureDescription),
            section: SettingsSection::Capture,
        },
        SettingsNavigationItem {
            id: "settings-nav-files",
            label: locale.text(UiText::Library),
            description: locale.text(UiText::LibraryDescription),
            section: SettingsSection::Files,
        },
        SettingsNavigationItem {
            id: "settings-nav-recording",
            label: locale.text(UiText::Record),
            description: locale.text(UiText::RecordDescription),
            section: SettingsSection::Recording,
        },
        SettingsNavigationItem {
            id: "settings-nav-system",
            label: locale.text(UiText::App),
            description: locale.text(UiText::AppDescription),
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
    let metrics = ThemeMetrics::default();
    let active = selected == item.section;
    let keyboard_app = app.clone();
    let item_focus = focus_handles[settings_section_index(item.section)].clone();
    div()
        .id(item.id)
        .flex()
        .when(compact, |item| {
            item.flex_1()
                .min_w(px(0.0))
                .h(px(metrics.control_height))
                .px_2()
                .items_center()
                .justify_center()
        })
        .when(!compact, |item| {
            item.w_full()
                .min_h(px(metrics.row_min_height + metrics.space_3))
                .px_3()
                .py_2()
                .flex_col()
                .justify_center()
                .gap_1()
        })
        .rounded_sm()
        .border_1()
        .border_color(if active {
            colors.accent
        } else {
            colors.surface
        })
        .text_sm()
        .cursor_pointer()
        .bg(if active {
            colors.surface_elevated
        } else {
            colors.surface
        })
        .text_color(if active { colors.accent } else { colors.text })
        .focusable()
        .track_focus(&item_focus)
        .focus_visible(|style| style.border_color(colors.focus))
        .hover(move |style| {
            style
                .bg(colors.surface_hover)
                .border_color(if active { colors.accent } else { colors.border })
                .text_color(if active { colors.accent } else { colors.text })
        })
        .active(move |style| {
            style
                .bg(colors.accent_pressed)
                .border_color(colors.accent_pressed)
                .text_color(colors.canvas)
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
        .child(div().w(px(3.0)).h(px(22.0)).rounded_full().bg(if active {
            colors.accent
        } else {
            colors.surface
        }))
        .child(
            div()
                .min_w(px(0.0))
                .when(compact, |label| label.text_ellipsis().flex_1())
                .font_weight(FontWeight::SEMIBOLD)
                .child(item.label),
        )
        .when(!compact, |element| {
            element.child(
                div()
                    .text_xs()
                    .text_color(colors.text_muted)
                    .child(item.description),
            )
        })
}

/// Gives every section a stable task-oriented title.
fn settings_page_intro(
    section: SettingsSection,
    colors: crate::theme::ThemeColors,
    locale: Locale,
) -> gpui::Div {
    let (title, description) = settings_page_copy_for_locale(section, locale);
    div()
        .pb_4()
        .border_b_1()
        .border_color(colors.border)
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .w(px(3.0))
                        .h(px(20.0))
                        .rounded_full()
                        .bg(colors.accent),
                )
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(title),
                ),
        )
        .child(
            div()
                .text_sm()
                .text_color(colors.text_muted)
                .child(description),
        )
}

/// Returns the short heading and purpose statement shown before each settings task group.
fn settings_page_copy_for_locale(
    section: SettingsSection,
    locale: Locale,
) -> (&'static str, &'static str) {
    match section {
        SettingsSection::Capture => (
            locale.text(UiText::Capture),
            locale.text(UiText::CapturePageDescription),
        ),
        SettingsSection::Files => (
            locale.text(UiText::Library),
            locale.text(UiText::LibraryPageDescription),
        ),
        SettingsSection::Recording => (
            locale.text(UiText::Record),
            locale.text(UiText::RecordPageDescription),
        ),
        SettingsSection::System => (
            locale.text(UiText::App),
            locale.text(UiText::AppPageDescription),
        ),
    }
}

fn settings_section(label: &str, colors: crate::theme::ThemeColors) -> gpui::Div {
    let metrics = ThemeMetrics::default();
    div()
        .pb_5()
        .border_b_1()
        .border_color(colors.border)
        .flex()
        .flex_col()
        .gap(px(metrics.settings_section_gap))
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
    let metrics = ThemeMetrics::default();
    div()
        .w_full()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(metrics.settings_row_gap))
        .min_h(px(metrics.row_min_height))
        .py_1()
        .child(
            div()
                .flex_1()
                .min_w(px(metrics.settings_label_min_width))
                .max_w(px(metrics.settings_label_max_width))
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
    let metrics = ThemeMetrics::default();
    div()
        .id(id)
        .h(px(metrics.control_height))
        .px_3()
        .flex()
        .items_center()
        .justify_center()
        .rounded_sm()
        .border_1()
        .border_color(colors.border)
        .bg(colors.surface_elevated)
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(if enabled {
            colors.text
        } else {
            colors.text_disabled
        })
        .when(enabled, |button| {
            button
                .focusable()
                .focus_visible(|style| style.border_color(colors.focus))
                .cursor_pointer()
                .hover(|style| style.bg(colors.surface_hover).border_color(colors.accent))
                .active(|style| {
                    style
                        .bg(colors.accent_pressed)
                        .border_color(colors.accent_pressed)
                        .text_color(colors.canvas)
                })
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
        .text_color(if enabled {
            colors.danger
        } else {
            colors.text_disabled
        })
        .when(enabled, |button| {
            button.active(|style| {
                style
                    .bg(colors.danger)
                    .border_color(colors.danger)
                    .text_color(colors.canvas)
            })
        })
}

fn quick_action_button(
    id: impl Into<gpui::ElementId>,
    label: &str,
    colors: crate::theme::ThemeColors,
    enabled: bool,
    primary: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let metrics = ThemeMetrics::default();
    div()
        .id(id)
        .h(px(metrics.toolbar_height))
        .when(primary, |button| button.w_full())
        .when(!primary, |button| button.flex_1().min_w(px(140.0)))
        .px_3()
        .flex()
        .items_center()
        .justify_center()
        .rounded_sm()
        .border_1()
        .border_color(if primary && enabled {
            colors.accent
        } else {
            colors.border
        })
        .bg(if primary && enabled {
            colors.accent
        } else {
            colors.surface_elevated
        })
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(if primary && enabled {
            colors.background
        } else if enabled {
            colors.text
        } else {
            colors.text_disabled
        })
        .when(enabled, |button| {
            button
                .focusable()
                .focus_visible(|style| style.border_color(colors.focus))
                .cursor_pointer()
                .hover(move |style| {
                    style
                        .bg(if primary {
                            colors.accent_hover
                        } else {
                            colors.surface_hover
                        })
                        .text_color(if primary { colors.canvas } else { colors.text })
                })
                .active(move |style| {
                    style
                        .bg(if primary {
                            colors.accent_pressed
                        } else {
                            colors.surface_hover
                        })
                        .text_color(if primary { colors.canvas } else { colors.text })
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
    let metrics = ThemeMetrics::default();
    // A fixed-size track keeps binary preferences easy to scan without letting
    // the label change shift adjacent settings rows.
    div()
        .id(id)
        .w(px(metrics.toggle_width))
        .h(px(metrics.toggle_height))
        .p(px(metrics.space_1 / 2.0))
        .flex()
        .items_center()
        .when(enabled_value, |toggle| toggle.justify_end())
        .rounded_full()
        .bg(if enabled_value && enabled {
            colors.accent
        } else {
            colors.surface_elevated
        })
        .border_1()
        .border_color(colors.border)
        .when(enabled, |toggle| {
            toggle
                .focusable()
                .focus_visible(|style| style.border_color(colors.focus))
                .cursor_pointer()
                .hover(|style| style.bg(colors.surface_hover).border_color(colors.accent))
                .active(|style| style.bg(colors.accent_pressed))
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
            colors.surface_elevated
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
    locale: Locale,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let label = if delay_seconds == 0 {
        locale.text(UiText::CaptureDelayOff).to_owned()
    } else {
        let seconds = delay_seconds.to_string();
        locale.format_template(UiText::CaptureDelaySeconds, &[("seconds", &seconds)])
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
        history_thumbnail_status, history_visibility_label, ocr_support_check_label,
        recording_progress_label, recording_source_discovery_busy, recording_status_visible,
        recording_support_check_label, recording_toggle_enabled, recording_toggle_label,
        relative_timestamp_label, settings_actions_available, settings_navigation_activation,
        settings_navigation_direction, settings_navigation_items_for_locale,
        settings_page_copy_for_locale, settings_page_intro, settings_path_label,
        status_indicator_color, translation_service_test_label, update_check_label_for_locale,
        uses_compact_settings_navigation, visible_history_entries,
    };
    use crate::app::{HistoryClearScope, HistoryFilter, SettingsSection};
    use crate::history::{HistoryEntry, HistorySource};
    use crate::i18n::Locale;
    use crate::recording::RecordingProgress;
    use crate::theme::ThemeColors;
    use std::collections::VecDeque;
    use std::path::{Path, PathBuf};

    #[test]
    fn capture_header_turns_into_a_delay_cancellation_command() {
        assert_eq!(capture_command_label(Locale::English, None), "Capture");
        assert_eq!(
            capture_command_label(Locale::English, Some(3)),
            "Cancel delay"
        );
    }

    #[test]
    fn history_thumbnail_status_prefers_success_then_failure_then_loading() {
        assert_eq!(
            history_thumbnail_status(Locale::English, true, true, true),
            None
        );
        assert_eq!(
            history_thumbnail_status(Locale::English, false, true, true),
            Some("Preview unavailable")
        );
        assert_eq!(
            history_thumbnail_status(Locale::SimplifiedChinese, false, false, true),
            Some("正在加载预览...")
        );
        assert_eq!(
            history_thumbnail_status(Locale::SimplifiedChinese, false, false, false),
            None
        );
    }

    #[test]
    fn shortcut_summary_distinguishes_registered_and_disabled_keys() {
        assert_eq!(
            capture_shortcut_summary(Locale::English, "Ctrl+Alt+S", true),
            "Registered: Ctrl+Alt+S"
        );
        assert_eq!(
            capture_shortcut_summary(Locale::English, "Ctrl+Alt+S", false),
            "Disabled: Ctrl+Alt+S"
        );
        assert_eq!(
            capture_shortcut_summary(Locale::SimplifiedChinese, "Ctrl+Alt+S", true),
            "已注册：Ctrl+Alt+S"
        );
        assert_eq!(
            capture_shortcut_summary(Locale::SimplifiedChinese, "Ctrl+Alt+S", false),
            "已禁用：Ctrl+Alt+S"
        );
    }

    #[test]
    fn history_clear_confirmation_names_the_exact_number_of_captures() {
        assert_eq!(
            history_clear_confirmation_label(Locale::English, 12, HistoryClearScope::All),
            "Delete all 12 saved captures?"
        );
        assert_eq!(
            history_clear_confirmation_label(Locale::English, 3, HistoryClearScope::Filtered),
            "Delete 3 filtered saved captures?"
        );
        assert_eq!(
            history_clear_confirmation_label(Locale::English, 2, HistoryClearScope::Selected),
            "Delete 2 selected saved captures?"
        );
        assert_eq!(
            history_clear_confirmation_label(
                Locale::SimplifiedChinese,
                2,
                HistoryClearScope::Selected
            ),
            "删除 2 张已选择的已保存截图？"
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
            let _ = settings_page_intro(section, colors, Locale::English);
        }
    }

    #[test]
    fn settings_navigation_uses_task_oriented_labels() {
        let items = settings_navigation_items_for_locale(Locale::English);
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
                "Theme, language, startup, updates",
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
            settings_page_copy_for_locale(SettingsSection::Capture, Locale::English),
            (
                "Capture",
                "Start a screenshot or adjust capture preferences."
            )
        );
        assert_eq!(
            settings_page_copy_for_locale(SettingsSection::Files, Locale::English),
            (
                "Library",
                "Find saved captures, change output, and manage history."
            )
        );
        assert_eq!(
            settings_page_copy_for_locale(SettingsSection::Recording, Locale::English),
            (
                "Record",
                "Choose capture sources, an output folder, and recording controls."
            )
        );
        assert_eq!(
            settings_page_copy_for_locale(SettingsSection::System, Locale::English),
            (
                "App",
                "Set appearance, language, startup, and update preferences."
            )
        );
    }

    #[test]
    fn settings_actions_wait_for_recording_lifecycle_to_settle() {
        let idle_recording = RecordingViewState {
            active: false,
            starting: false,
            stopping: false,
            display_discovery_in_flight: false,
            audio_discovery_in_flight: false,
            support_check_in_flight: false,
            paused: false,
            progress: RecordingProgress::default(),
        };
        assert!(settings_actions_available(true, idle_recording));
        assert!(!settings_actions_available(false, idle_recording));

        assert!(!settings_actions_available(
            true,
            RecordingViewState {
                active: true,
                ..idle_recording
            }
        ));
        assert!(!settings_actions_available(
            true,
            RecordingViewState {
                starting: true,
                ..idle_recording
            }
        ));
        assert!(!settings_actions_available(
            true,
            RecordingViewState {
                stopping: true,
                ..idle_recording
            }
        ));
    }

    #[test]
    fn empty_history_section_explains_when_saved_captures_appear() {
        assert_eq!(
            super::empty_history_message(Locale::English, 0, HistoryFilter::All, ""),
            "Saved screenshots will appear here."
        );
        assert_eq!(
            super::empty_history_message(Locale::English, 3, HistoryFilter::Pinned, ""),
            "No pinned captures yet."
        );
        assert_eq!(
            super::empty_history_message(Locale::English, 3, HistoryFilter::All, "invoice"),
            "No captures match \"invoice\"."
        );
        assert_eq!(
            super::empty_history_message(
                Locale::SimplifiedChinese,
                3,
                HistoryFilter::All,
                "invoice"
            ),
            "没有与“invoice”匹配的截图。"
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
            recording_progress_label(
                Locale::English,
                false,
                true,
                false,
                false,
                RecordingProgress::default(),
            ),
            "Preparing recording..."
        );
        assert_eq!(
            recording_progress_label(
                Locale::English,
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
                Locale::English,
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
            recording_progress_label(
                Locale::English,
                false,
                false,
                true,
                false,
                RecordingProgress::default(),
            ),
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
        assert_eq!(
            recording_toggle_label(Locale::English, state, false),
            "Discovering..."
        );
    }

    #[test]
    fn recording_controls_keep_a_startup_cancellation_action_available() {
        let idle = RecordingViewState {
            active: false,
            starting: false,
            stopping: false,
            display_discovery_in_flight: false,
            audio_discovery_in_flight: false,
            support_check_in_flight: false,
            paused: false,
            progress: RecordingProgress::default(),
        };
        let starting = RecordingViewState {
            starting: true,
            ..idle
        };

        assert_eq!(
            recording_toggle_label(Locale::English, starting, false),
            "Cancel start"
        );
        assert!(recording_toggle_enabled(starting, false));
        assert!(!recording_toggle_enabled(
            RecordingViewState {
                stopping: true,
                ..idle
            },
            false,
        ));
        assert_eq!(
            recording_toggle_label(Locale::English, idle, true),
            "Checking folder..."
        );
        assert!(!recording_toggle_enabled(idle, true));
    }

    #[test]
    fn recording_support_check_label_explains_the_cancel_action() {
        assert_eq!(
            recording_support_check_label(Locale::English, false),
            "Check support"
        );
        assert_eq!(
            recording_support_check_label(Locale::English, true),
            "Cancel check"
        );
    }

    #[test]
    fn settings_navigation_compacts_before_the_content_column_becomes_too_narrow() {
        assert!(uses_compact_settings_navigation(639.0));
        assert!(!uses_compact_settings_navigation(640.0));
    }

    #[test]
    fn file_settings_labels_explain_retention_and_hide_internal_path_prefixes() {
        assert_eq!(
            history_retention_label(Locale::English, 30, None),
            "Keep 30 captures"
        );
        assert_eq!(
            history_retention_label(Locale::English, 30, Some(100)),
            "Updating to 100 captures..."
        );
        assert_eq!(
            history_retention_label(Locale::SimplifiedChinese, 30, Some(100)),
            "正在更新为保留 100 张截图..."
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
        assert_eq!(
            status_indicator_color("Screen recording startup cancelled", true, colors),
            colors.muted
        );
    }

    #[test]
    fn translation_test_label_explains_its_independent_busy_state() {
        assert_eq!(
            translation_service_test_label(Locale::English, false),
            "Test service"
        );
        assert_eq!(
            translation_service_test_label(Locale::English, true),
            "Cancel test"
        );
        assert_eq!(
            translation_service_test_label(Locale::SimplifiedChinese, false),
            "测试服务"
        );
        assert_eq!(
            translation_service_test_label(Locale::SimplifiedChinese, true),
            "取消测试"
        );
    }

    #[test]
    fn ocr_support_check_label_explains_its_busy_state() {
        assert_eq!(
            ocr_support_check_label(Locale::English, false),
            "Check support"
        );
        assert_eq!(
            ocr_support_check_label(Locale::English, true),
            "Checking local OCR support..."
        );
        assert_eq!(
            ocr_support_check_label(Locale::SimplifiedChinese, false),
            "检查支持"
        );
        assert_eq!(
            ocr_support_check_label(Locale::SimplifiedChinese, true),
            "正在检查本地 OCR 支持..."
        );
    }

    #[test]
    fn update_check_label_explains_the_cancel_action() {
        assert_eq!(
            update_check_label_for_locale(Locale::English, false),
            "Check now"
        );
        assert_eq!(
            update_check_label_for_locale(Locale::English, true),
            "Cancel check"
        );
    }

    #[test]
    fn history_result_summary_explains_filters_and_queries() {
        assert_eq!(
            history_result_summary(Locale::English, 12, 12, HistoryFilter::All, ""),
            "12 capture(s)"
        );
        assert_eq!(
            history_result_summary(Locale::English, 12, 3, HistoryFilter::Pinned, ""),
            "3 pinned capture(s)"
        );
        assert_eq!(
            history_result_summary(Locale::English, 12, 2, HistoryFilter::All, "invoice"),
            "2 match(es) for \"invoice\""
        );
        assert_eq!(
            history_result_summary(Locale::SimplifiedChinese, 12, 3, HistoryFilter::Pinned, ""),
            "贴图 3 张"
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
            history_entry_label(Locale::English, &entry, 1_000_000),
            "example.png - Selection - Just now"
        );
        assert_eq!(
            history_entry_label(Locale::SimplifiedChinese, &entry, 1_000_000),
            "example.png - 选区截图 - 刚刚"
        );
        assert_eq!(
            relative_timestamp_label(Locale::English, 1_000_000, 1_065_000),
            "1m ago"
        );
        assert_eq!(
            relative_timestamp_label(Locale::English, 1_000_000, 4_600_000),
            "1h ago"
        );
        assert_eq!(
            relative_timestamp_label(Locale::SimplifiedChinese, 1_000_000, 173_800_000),
            "2 天前"
        );
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
            history_visibility_label(Locale::English, 7, false),
            "Showing 5 of 7 captures"
        );
        assert_eq!(
            history_visibility_label(Locale::English, 7, true),
            "Showing all 7 captures"
        );
        assert_eq!(
            history_visibility_label(Locale::SimplifiedChinese, 7, false),
            "正在显示 7 张截图中的 5 张"
        );
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
