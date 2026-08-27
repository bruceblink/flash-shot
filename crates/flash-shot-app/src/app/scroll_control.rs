//! Small movable controller used while a user captures scrolling content.

use gpui::{
    Context, Entity, FocusHandle, Focusable, FontWeight, KeyDownEvent, Keystroke, Render,
    Subscription, Window, div, prelude::*, px,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::FlashShotApp;
use crate::i18n::{Locale, UiText};

const MANUAL_SCROLL_BUTTON_HEIGHT: f32 = 32.0;
const MANUAL_SCROLL_AUTO_BUTTON_MIN_WIDTH: f32 = 176.0;
const MANUAL_SCROLL_CAPTURE_BUTTON_MIN_WIDTH: f32 = 122.0;
const MANUAL_SCROLL_FINISH_BUTTON_MIN_WIDTH: f32 = 84.0;
const MANUAL_SCROLL_CANCEL_BUTTON_MIN_WIDTH: f32 = 80.0;
const MANUAL_SCROLL_STATUS_MAX_WIDTH: f32 = 200.0;

pub(super) struct ManualScrollControl {
    app: Entity<FlashShotApp>,
    focus_handle: FocusHandle,
    _app_observation: Subscription,
    topmost_requested: bool,
}

impl ManualScrollControl {
    pub(super) fn new(app: Entity<FlashShotApp>, cx: &mut Context<Self>) -> Self {
        let observation = cx.observe(&app, |_, _, cx| cx.notify());
        Self {
            app,
            focus_handle: cx.focus_handle(),
            _app_observation: observation,
            topmost_requested: false,
        }
    }
}

impl Focusable for ManualScrollControl {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ManualScrollControl {
    /// Dispatches focused-controller shortcuts without affecting global application shortcuts.
    fn handle_key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let Some(shortcut) = manual_scroll_shortcut(&event.keystroke) else {
            return;
        };
        let app = self.app.clone();
        cx.defer(move |cx| {
            app.update(cx, |app, cx| match shortcut {
                ManualScrollShortcut::Cancel => app.cancel_manual_scroll(cx),
                ManualScrollShortcut::Capture => app.capture_manual_scroll_frame(cx),
                ManualScrollShortcut::AutoCapture => app.auto_capture_manual_scroll_frame(cx),
                ManualScrollShortcut::Finish => app.finish_manual_scroll(cx),
            });
        });
    }
}

#[derive(Clone, Copy)]
enum ManualScrollButtonTone {
    Neutral,
    Primary,
    Success,
    Destructive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ManualScrollShortcut {
    Cancel,
    Capture,
    AutoCapture,
    Finish,
}

impl ManualScrollShortcut {
    /// Returns the stable keyboard order used by the visible scrolling commands.
    const fn tab_index(self) -> isize {
        match self {
            Self::AutoCapture => 0,
            Self::Capture => 1,
            Self::Finish => 2,
            Self::Cancel => 3,
        }
    }

    /// Gives longer commands more room while keeping all four actions on one 520 px row.
    const fn min_width(self) -> f32 {
        match self {
            Self::AutoCapture => MANUAL_SCROLL_AUTO_BUTTON_MIN_WIDTH,
            Self::Capture => MANUAL_SCROLL_CAPTURE_BUTTON_MIN_WIDTH,
            Self::Finish => MANUAL_SCROLL_FINISH_BUTTON_MIN_WIDTH,
            Self::Cancel => MANUAL_SCROLL_CANCEL_BUTTON_MIN_WIDTH,
        }
    }
}

/// Builds one scroll command with a stable size and an explicit enabled/disabled state.
fn manual_scroll_button(
    id: &'static str,
    action: ManualScrollShortcut,
    label: impl Into<String>,
    colors: crate::theme::ThemeColors,
    enabled: bool,
    tone: ManualScrollButtonTone,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let (active_background, active_text) = match tone {
        ManualScrollButtonTone::Neutral => (colors.panel, colors.text),
        ManualScrollButtonTone::Primary => (colors.accent, colors.background),
        ManualScrollButtonTone::Success => (colors.success, colors.background),
        ManualScrollButtonTone::Destructive => (colors.panel, colors.danger),
    };
    div()
        .id(id)
        .h(px(MANUAL_SCROLL_BUTTON_HEIGHT))
        .flex_1()
        .min_w(px(action.min_width()))
        .px_2()
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .border_1()
        .border_color(if enabled {
            match tone {
                ManualScrollButtonTone::Neutral => colors.border,
                ManualScrollButtonTone::Primary => colors.accent,
                ManualScrollButtonTone::Success => colors.success,
                ManualScrollButtonTone::Destructive => colors.danger,
            }
        } else {
            colors.border
        })
        .bg(if enabled {
            active_background
        } else {
            colors.panel
        })
        .text_color(if enabled { active_text } else { colors.muted })
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .when(enabled, |button| {
            button
                .tab_index(action.tab_index())
                .focus_visible(|style| style.border_color(colors.accent))
                .cursor_pointer()
                .hover(|style| style.bg(colors.background).text_color(colors.text))
                .on_click(on_click)
        })
        .child(
            div()
                .w_full()
                .min_w(px(0.0))
                .truncate()
                .text_center()
                .child(label.into()),
        )
}

impl Render for ManualScrollControl {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.topmost_requested
            && let Ok(handle) = window.window_handle()
            && let RawWindowHandle::Win32(handle) = handle.as_raw()
        {
            self.topmost_requested = true;
            let hwnd = handle.hwnd.get();
            // Keep scrolling controls above the target application so each capture action stays
            // available while the user moves through the page. The deferred call avoids re-entering
            // GPUI's native window dispatch from inside the render callback.
            cx.defer(move |_| {
                if let Err(error) = crate::platform::window_visibility::make_topmost(hwnd) {
                    log::warn!(target: "flash_shot::scroll", "scroll_control_topmost_failed error={error}");
                }
            });
        }
        let app = self.app.read(cx);
        let colors = app.colors;
        let locale = app.settings.locale;
        let status = app.status.clone();
        let frame_count = app.manual_scroll.frame_count();
        let capture_in_flight = app.manual_scroll_capture_in_flight;
        let auto_capture_pending = app.manual_scroll_auto_capture_generation.is_some();
        let controls_busy = capture_in_flight || auto_capture_pending;
        let retry_available = app.manual_scroll.failure().is_some();
        let can_finish = app.manual_scroll.can_finish();
        let frame_count_label = manual_scroll_frame_count_label(locale, frame_count, can_finish);

        div()
            .size_full()
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .bg(colors.background)
            .border_1()
            .border_color(colors.border)
            .rounded_lg()
            .track_focus(&self.focus_handle)
            .tab_group()
            .tab_stop(false)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                this.handle_key_down(event, cx);
            }))
            .child(
                div()
                    .w_full()
                    .min_w(px(0.0))
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(colors.text)
                            .truncate()
                            .child(locale.text(UiText::ScrollingScreenshot)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .max_w(px(MANUAL_SCROLL_STATUS_MAX_WIDTH))
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .bg(colors.panel)
                            .text_xs()
                            .text_color(colors.muted)
                            .truncate()
                            .child(frame_count_label),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .min_w(px(0.0))
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(manual_scroll_button(
                        "scroll-auto-capture-next",
                        ManualScrollShortcut::AutoCapture,
                        auto_scroll_capture_label(locale, auto_capture_pending),
                        colors,
                        !controls_busy,
                        ManualScrollButtonTone::Primary,
                        cx.listener(|this, _, _, cx| {
                            let app = this.app.clone();
                            cx.defer(move |cx| {
                                app.update(cx, |app, cx| app.auto_capture_manual_scroll_frame(cx))
                            });
                        }),
                    ))
                    .child(manual_scroll_button(
                        "scroll-capture-next",
                        ManualScrollShortcut::Capture,
                        manual_scroll_capture_label(locale, capture_in_flight, retry_available),
                        colors,
                        !controls_busy,
                        ManualScrollButtonTone::Neutral,
                        cx.listener(|this, _, _, cx| {
                            let app = this.app.clone();
                            cx.defer(move |cx| {
                                app.update(cx, |app, cx| app.capture_manual_scroll_frame(cx))
                            });
                        }),
                    ))
                    .child(manual_scroll_button(
                        "scroll-finish",
                        ManualScrollShortcut::Finish,
                        manual_scroll_finish_label(locale, can_finish),
                        colors,
                        !controls_busy && can_finish,
                        if can_finish {
                            ManualScrollButtonTone::Success
                        } else {
                            ManualScrollButtonTone::Neutral
                        },
                        cx.listener(|this, _, _, cx| {
                            let app = this.app.clone();
                            cx.defer(move |cx| {
                                app.update(cx, |app, cx| app.finish_manual_scroll(cx))
                            });
                        }),
                    ))
                    .child(manual_scroll_button(
                        "scroll-cancel",
                        ManualScrollShortcut::Cancel,
                        locale.text(UiText::OverlayCancel),
                        colors,
                        true,
                        ManualScrollButtonTone::Destructive,
                        cx.listener(|this, _, _, cx| {
                            let app = this.app.clone();
                            cx.defer(move |cx| {
                                app.update(cx, |app, cx| app.cancel_manual_scroll(cx))
                            });
                        }),
                    )),
            )
            .child(
                div()
                    .w_full()
                    .min_w(px(0.0))
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .bg(colors.panel)
                    .text_xs()
                    .text_color(colors.muted)
                    .truncate()
                    .child(status),
            )
    }
}

/// Maps focused-controller keys to one scrolling action while leaving modified system keys alone.
fn manual_scroll_shortcut(keystroke: &Keystroke) -> Option<ManualScrollShortcut> {
    let plain = !keystroke.modifiers.modified();
    let without_system_modifiers = !keystroke.modifiers.secondary()
        && !keystroke.modifiers.platform
        && !keystroke.modifiers.alt
        && !keystroke.modifiers.function;
    match keystroke.key.as_str() {
        "escape" if plain => Some(ManualScrollShortcut::Cancel),
        "enter" if plain => Some(ManualScrollShortcut::Finish),
        "space" if without_system_modifiers && !keystroke.modifiers.shift => {
            Some(ManualScrollShortcut::Capture)
        }
        "space" if without_system_modifiers && keystroke.modifiers.shift => {
            Some(ManualScrollShortcut::AutoCapture)
        }
        _ => None,
    }
}

/// Keeps the primary action explicit while one scroll frame is being captured.
fn manual_scroll_capture_label(
    locale: Locale,
    capture_in_flight: bool,
    retry_available: bool,
) -> &'static str {
    if capture_in_flight {
        locale.text(UiText::ScrollingCaptureInProgress)
    } else if retry_available {
        locale.text(UiText::ScrollingRetryView)
    } else {
        locale.text(UiText::ScrollingCaptureView)
    }
}

/// Explains that automatic capture is waiting for the target application to repaint.
fn auto_scroll_capture_label(locale: Locale, auto_capture_pending: bool) -> &'static str {
    if auto_capture_pending {
        locale.text(UiText::ScrollingScrolling)
    } else {
        locale.text(UiText::ScrollingScrollDownCapture)
    }
}

/// Names the next required action until a second viewport makes stitching possible.
fn manual_scroll_finish_label(locale: Locale, can_finish: bool) -> &'static str {
    if can_finish {
        locale.text(UiText::ScrollingFinish)
    } else {
        locale.text(UiText::ScrollingNotReady)
    }
}

/// Summarizes the session stage next to the frame count so the next action is obvious.
fn manual_scroll_frame_count_label(locale: Locale, frame_count: usize, can_finish: bool) -> String {
    let count = match frame_count {
        0 => locale.text(UiText::ScrollingNoFrames).to_owned(),
        1 => locale.text(UiText::ScrollingOneFrame).to_owned(),
        count => locale.format_template(
            UiText::ScrollingManyFrames,
            &[("count", &count.to_string())],
        ),
    };
    if can_finish {
        locale.format_template(UiText::ScrollingReadyToFinish, &[("count", &count)])
    } else {
        locale.format_template(UiText::ScrollingCaptureAnother, &[("count", &count)])
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MANUAL_SCROLL_AUTO_BUTTON_MIN_WIDTH, MANUAL_SCROLL_CANCEL_BUTTON_MIN_WIDTH,
        MANUAL_SCROLL_CAPTURE_BUTTON_MIN_WIDTH, MANUAL_SCROLL_FINISH_BUTTON_MIN_WIDTH,
        ManualScrollShortcut, auto_scroll_capture_label, manual_scroll_capture_label,
        manual_scroll_finish_label, manual_scroll_frame_count_label, manual_scroll_shortcut,
    };
    use crate::i18n::Locale;
    use gpui::Keystroke;

    #[test]
    fn capture_action_describes_its_busy_state() {
        assert_eq!(
            manual_scroll_capture_label(Locale::English, false, false),
            "Capture view"
        );
        assert_eq!(
            manual_scroll_capture_label(Locale::English, false, true),
            "Retry view"
        );
        assert_eq!(
            manual_scroll_capture_label(Locale::English, true, true),
            "Capturing..."
        );
        assert_eq!(
            manual_scroll_capture_label(Locale::SimplifiedChinese, false, true),
            "重试视口"
        );
    }

    #[test]
    fn finish_action_requires_an_overlapping_viewport() {
        assert_eq!(
            manual_scroll_finish_label(Locale::English, false),
            "Not ready"
        );
        assert_eq!(manual_scroll_finish_label(Locale::English, true), "Finish");
        assert_eq!(
            manual_scroll_finish_label(Locale::SimplifiedChinese, true),
            "完成"
        );
    }

    #[test]
    fn frame_count_badge_explains_when_the_session_can_finish() {
        assert_eq!(
            manual_scroll_frame_count_label(Locale::English, 1, false),
            "1 frame - capture another"
        );
        assert_eq!(
            manual_scroll_frame_count_label(Locale::English, 2, true),
            "2 frames - ready to finish"
        );
        assert_eq!(
            manual_scroll_frame_count_label(Locale::English, 0, false),
            "No frames - capture another"
        );
        assert_eq!(
            manual_scroll_frame_count_label(Locale::SimplifiedChinese, 2, true),
            "2 个视口，可以完成"
        );
    }

    #[test]
    fn automatic_capture_action_reports_its_settle_delay() {
        assert_eq!(
            auto_scroll_capture_label(Locale::English, false),
            "Scroll down + capture"
        );
        assert_eq!(
            auto_scroll_capture_label(Locale::English, true),
            "Scrolling..."
        );
        assert_eq!(
            auto_scroll_capture_label(Locale::SimplifiedChinese, false),
            "向下滚动并捕获"
        );
    }

    #[test]
    fn visible_actions_keep_primary_width_and_keyboard_order() {
        assert_eq!(ManualScrollShortcut::AutoCapture.tab_index(), 0);
        assert_eq!(ManualScrollShortcut::Capture.tab_index(), 1);
        assert_eq!(ManualScrollShortcut::Finish.tab_index(), 2);
        assert_eq!(ManualScrollShortcut::Cancel.tab_index(), 3);
        assert_eq!(
            ManualScrollShortcut::AutoCapture.min_width(),
            MANUAL_SCROLL_AUTO_BUTTON_MIN_WIDTH
        );
        assert_eq!(
            ManualScrollShortcut::Capture.min_width(),
            MANUAL_SCROLL_CAPTURE_BUTTON_MIN_WIDTH
        );
        assert_eq!(
            ManualScrollShortcut::Finish.min_width(),
            MANUAL_SCROLL_FINISH_BUTTON_MIN_WIDTH
        );
        assert_eq!(
            ManualScrollShortcut::Cancel.min_width(),
            MANUAL_SCROLL_CANCEL_BUTTON_MIN_WIDTH
        );
        let minimum_buttons = [
            ManualScrollShortcut::AutoCapture,
            ManualScrollShortcut::Capture,
            ManualScrollShortcut::Finish,
            ManualScrollShortcut::Cancel,
        ]
        .into_iter()
        .map(ManualScrollShortcut::min_width)
        .sum::<f32>();
        // A 520 px control has 472 px after root padding and the three 8 px gaps.
        assert!(minimum_buttons <= 472.0);
    }

    #[test]
    fn focused_keys_keep_modified_shortcuts_isolated() {
        assert_eq!(
            manual_scroll_shortcut(&Keystroke::parse("escape").unwrap()),
            Some(ManualScrollShortcut::Cancel)
        );
        assert_eq!(
            manual_scroll_shortcut(&Keystroke::parse("space").unwrap()),
            Some(ManualScrollShortcut::Capture)
        );
        assert_eq!(
            manual_scroll_shortcut(&Keystroke::parse("shift-space").unwrap()),
            Some(ManualScrollShortcut::AutoCapture)
        );
        assert_eq!(
            manual_scroll_shortcut(&Keystroke::parse("enter").unwrap()),
            Some(ManualScrollShortcut::Finish)
        );
        for key in [
            "shift-escape",
            "ctrl-escape",
            "alt-escape",
            "cmd-escape",
            "fn-escape",
            "ctrl-space",
            "alt-space",
            "cmd-space",
            "shift-enter",
        ] {
            assert_eq!(
                manual_scroll_shortcut(&Keystroke::parse(key).unwrap()),
                None,
                "{key}"
            );
        }
    }
}
