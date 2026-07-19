//! GPUI capture workspace state and module boundaries.

mod overlay;
mod pinned;
mod render_image;
mod scroll_control;
mod view;
mod workflow;

use std::{ops::Range, path::PathBuf, sync::Arc};

use gpui::{
    AsyncApp, Context, EntityInputHandler, FocusHandle, Focusable, RenderImage, Subscription,
    UTF16Selection, WeakEntity, Window, WindowHandle,
};

use crate::{
    domain::{
        annotation::{
            AnnotationDocument, AnnotationEditor, AnnotationId, AnnotationStyle, AnnotationTool,
            CommandHistory,
        },
        geometry::PhysicalPoint,
        selection::SelectionDrag,
        session::CaptureSession,
    },
    history::ScreenshotHistory,
    performance::PerformanceRecorder,
    platform::{
        autostart::{AutoStartService, AutoStartState, SystemAutoStart},
        capture::CaptureFrame,
        shortcut::{CaptureShortcut, GlobalShortcutService, ShortcutEvent},
        tray::{TrayEvent, TrayNotification, TrayService},
        window_inspector::InspectionTarget,
    },
    settings::UserSettings,
    theme::ThemeColors,
};

pub struct FlashShotApp {
    colors: ThemeColors,
    session: CaptureSession,
    frame: Option<CaptureFrame>,
    annotation_document: Option<AnnotationDocument>,
    annotation_history: CommandHistory,
    annotation_editor: AnnotationEditor,
    annotation_tool: Option<AnnotationTool>,
    annotation_style: AnnotationStyle,
    selected_annotation: Option<AnnotationId>,
    next_annotation_id: u64,
    next_sequence_number: u32,
    text_edit: Option<TextEdit>,
    text_edit_annotation: Option<AnnotationId>,
    preview: Option<Arc<RenderImage>>,
    selection_drag: SelectionDrag,
    hover_pixel: Option<PhysicalPoint>,
    inspection_target: Option<InspectionTarget>,
    pending_click_target: Option<InspectionTarget>,
    inspection_request: Option<PhysicalPoint>,
    inspection_in_flight: bool,
    manual_scroll: crate::scroll::ManualScrollCapture,
    manual_scroll_selection: Option<crate::domain::geometry::PhysicalRect>,
    manual_scroll_capture_in_flight: bool,
    recording_control: Option<crate::recording::RecordingControl>,
    recording_progress: crate::recording::RecordingProgress,
    recording_start_in_flight: bool,
    recording_paused: bool,
    recording_audio: RecordingAudioSelection,
    recording_audio_discovery_in_flight: bool,
    recording_display: RecordingDisplaySelection,
    recording_display_discovery_in_flight: bool,
    update_check_in_flight: bool,
    auto_start_enabled: bool,
    capture_delay_seconds: u8,
    delayed_capture_generation: Option<u64>,
    delayed_capture_remaining_seconds: Option<u8>,
    include_cursor: bool,
    recognition_result: Option<RecognitionResult>,
    overlay_more_actions: bool,
    overlay_annotation_controls: bool,
    operation_generation: u64,
    overlay_windows: Vec<WindowHandle<overlay::CaptureOverlay>>,
    scroll_window: Option<WindowHandle<scroll_control::ManualScrollControl>>,
    settings_window_handle: Option<isize>,
    focus_handle: FocusHandle,
    capture_shortcut: String,
    settings_section: SettingsSection,
    settings: UserSettings,
    settings_path: PathBuf,
    status: String,
    performance: PerformanceRecorder,
    history: ScreenshotHistory,
    _shutdown: Subscription,
    _shortcut: Option<GlobalShortcutService>,
    _tray: Option<TrayService>,
}

/// The settings window is intentionally segmented so the capture service has
/// no always-visible command surface and each configuration task stays small.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum SettingsSection {
    #[default]
    Capture,
    Files,
    Recording,
    System,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TextEdit {
    pub(super) origin: PhysicalPoint,
    pub(super) content: String,
    pub(super) selected_range: Range<usize>,
    pub(super) marked_range: Option<Range<usize>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RecognitionResult {
    pub(super) title: String,
    pub(super) text: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) enum RecordingAudioSelection {
    #[default]
    Automatic,
    Disabled,
    Source(crate::recording::AudioSource),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) enum RecordingDisplaySelection {
    #[default]
    Primary,
    Display {
        id: String,
        label: String,
    },
}

impl TextEdit {
    pub(super) fn new(origin: PhysicalPoint) -> Self {
        Self::with_content(origin, String::new(), false)
    }

    pub(super) fn with_content(origin: PhysicalPoint, content: String, select_all: bool) -> Self {
        let selected_range = if select_all { 0..content.len() } else { 0..0 };
        Self {
            origin,
            content,
            selected_range,
            marked_range: None,
        }
    }
}

impl FlashShotApp {
    pub(crate) fn set_settings_window_handle(&mut self, handle: isize) {
        self.settings_window_handle = Some(handle);
    }

    pub(super) fn notify_user(&self, title: &str, body: &str) {
        let Some(tray) = self._tray.as_ref() else {
            return;
        };
        if let Err(error) = tray.notify(TrayNotification::new(title, body)) {
            log::warn!(target: "flash_shot::tray", "user_notification_failed error={error}");
        }
    }

    pub fn new(
        performance: PerformanceRecorder,
        history: ScreenshotHistory,
        settings: UserSettings,
        settings_path: PathBuf,
        cx: &mut Context<Self>,
    ) -> Self {
        let shutdown = cx.on_app_quit(|this, cx| {
            this.shutdown(cx);
            async {}
        });
        let capture_shortcut = match settings
            .capture_shortcut
            .as_deref()
            .map(str::parse)
            .transpose()
        {
            Ok(Some(shortcut)) => shortcut,
            Ok(None) => match CaptureShortcut::from_environment() {
                Ok(shortcut) => shortcut,
                Err(error) => {
                    log::warn!(target: "flash_shot::shortcut", "capture_hotkey_config_invalid error={error}");
                    CaptureShortcut::default()
                }
            },
            Err(error) => {
                log::warn!(target: "flash_shot::shortcut", "saved_capture_hotkey_invalid error={error}");
                CaptureShortcut::default()
            }
        };
        let capture_shortcut_label = capture_shortcut.to_string();
        let shortcut = match GlobalShortcutService::register_capture(capture_shortcut) {
            Ok((service, events)) => {
                Self::listen_for_shortcut(events, cx);
                Some(service)
            }
            Err(error) => {
                log::warn!(target: "flash_shot::shortcut", "capture_hotkey_unavailable error={error}");
                None
            }
        };
        let status = if shortcut.is_some() {
            format!("Ready - {capture_shortcut_label}")
        } else {
            "Ready - global shortcut unavailable".to_owned()
        };
        let tray = match TrayService::start() {
            Ok((service, events)) => {
                Self::listen_for_tray(events, cx);
                Some(service)
            }
            Err(error) => {
                log::warn!(target: "flash_shot::tray", "tray_unavailable error={error}");
                None
            }
        };
        let auto_start_enabled = match std::env::current_exe()
            .ok()
            .map(|executable| SystemAutoStart.state(&executable))
        {
            Some(Ok(AutoStartState::Enabled)) => true,
            Some(Ok(AutoStartState::Disabled | AutoStartState::ManagedByAnotherExecutable)) => {
                false
            }
            Some(Err(error)) => {
                log::warn!(target: "flash_shot::autostart", "auto_start_state_read_failed error={error}");
                false
            }
            None => false,
        };

        Self {
            colors: ThemeColors::default(),
            session: CaptureSession::default(),
            frame: None,
            annotation_document: None,
            annotation_history: CommandHistory::default(),
            annotation_editor: AnnotationEditor::default(),
            annotation_tool: None,
            annotation_style: AnnotationStyle::default(),
            selected_annotation: None,
            next_annotation_id: 1,
            next_sequence_number: 1,
            text_edit: None,
            text_edit_annotation: None,
            preview: None,
            selection_drag: SelectionDrag::default(),
            hover_pixel: None,
            inspection_target: None,
            pending_click_target: None,
            inspection_request: None,
            inspection_in_flight: false,
            manual_scroll: crate::scroll::ManualScrollCapture::default(),
            manual_scroll_selection: None,
            manual_scroll_capture_in_flight: false,
            recording_control: None,
            recording_progress: Default::default(),
            recording_start_in_flight: false,
            recording_paused: false,
            recording_audio: RecordingAudioSelection::Automatic,
            recording_audio_discovery_in_flight: false,
            recording_display: RecordingDisplaySelection::Primary,
            recording_display_discovery_in_flight: false,
            update_check_in_flight: false,
            auto_start_enabled,
            capture_delay_seconds: settings.capture_delay_seconds,
            delayed_capture_generation: None,
            delayed_capture_remaining_seconds: None,
            include_cursor: settings.include_cursor,
            recognition_result: None,
            overlay_more_actions: false,
            overlay_annotation_controls: false,
            operation_generation: 0,
            overlay_windows: Vec::new(),
            scroll_window: None,
            settings_window_handle: None,
            focus_handle: cx.focus_handle(),
            capture_shortcut: capture_shortcut_label,
            settings_section: SettingsSection::default(),
            settings,
            settings_path,
            status,
            performance,
            history,
            _shutdown: shutdown,
            _shortcut: shortcut,
            _tray: tray,
        }
    }

    fn listen_for_shortcut(events: async_channel::Receiver<ShortcutEvent>, cx: &mut Context<Self>) {
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                while let Ok(ShortcutEvent::CaptureRequested) = events.recv().await {
                    if let Some(this) = this.upgrade() {
                        this.update(&mut cx, |this, cx| this.start_capture(cx));
                    } else {
                        break;
                    }
                }
            }
        })
        .detach();
    }

    fn listen_for_tray(events: async_channel::Receiver<TrayEvent>, cx: &mut Context<Self>) {
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                while let Ok(event) = events.recv().await {
                    let Some(this) = this.upgrade() else {
                        break;
                    };
                    match event {
                        TrayEvent::CaptureRequested => {
                            this.update(&mut cx, |this, cx| this.start_capture(cx));
                        }
                        TrayEvent::FullScreenCaptureRequested => {
                            this.update(&mut cx, |this, cx| this.start_full_screen_capture(cx));
                        }
                        TrayEvent::DelayedCaptureRequested => {
                            this.update(&mut cx, |this, cx| this.start_delayed_capture(3, cx));
                        }
                        TrayEvent::OpenImageRequested => {
                            this.update(&mut cx, |this, cx| this.open_image(cx));
                        }
                        TrayEvent::HistoryRequested => {
                            this.update(&mut cx, |this, cx| this.show_history_window(cx));
                        }
                        TrayEvent::SettingsRequested => {
                            this.update(&mut cx, |this, cx| this.show_settings_window(cx));
                        }
                        TrayEvent::QuitRequested => {
                            cx.update(|cx| cx.quit());
                            break;
                        }
                    }
                }
            }
        })
        .detach();
    }
}

impl EntityInputHandler for FlashShotApp {
    fn text_for_range(
        &mut self,
        range_utf16: std::ops::Range<usize>,
        actual_range: &mut Option<std::ops::Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let edit = self.text_edit.as_ref()?;
        let range = utf16_range_to_byte_range(&edit.content, &range_utf16);
        *actual_range = Some(byte_range_to_utf16_range(&edit.content, &range));
        Some(edit.content[range].to_owned())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let edit = self.text_edit.as_ref()?;
        Some(UTF16Selection {
            range: byte_range_to_utf16_range(&edit.content, &edit.selected_range),
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<std::ops::Range<usize>> {
        let edit = self.text_edit.as_ref()?;
        edit.marked_range
            .as_ref()
            .map(|range| byte_range_to_utf16_range(&edit.content, range))
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.unmark_text_edit(cx);
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<std::ops::Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace_text_edit(range_utf16, text, None, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<std::ops::Range<usize>>,
        text: &str,
        selected_range_utf16: Option<std::ops::Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace_text_edit(range_utf16, text, selected_range_utf16, cx);
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: std::ops::Range<usize>,
        bounds: gpui::Bounds<gpui::Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<gpui::Bounds<gpui::Pixels>> {
        self.text_edit.as_ref().map(|_| bounds)
    }

    fn character_index_for_point(
        &mut self,
        _point: gpui::Point<gpui::Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        self.text_edit
            .as_ref()
            .map(|edit| edit.content.chars().map(char::len_utf16).sum::<usize>())
    }

    fn accepts_text_input(&self, _window: &mut Window, _cx: &mut Context<Self>) -> bool {
        self.text_edit.is_some()
    }
}

fn byte_range_to_utf16_range(text: &str, range: &std::ops::Range<usize>) -> std::ops::Range<usize> {
    let utf16_offset = |offset| text[..offset].chars().map(char::len_utf16).sum();
    utf16_offset(range.start)..utf16_offset(range.end)
}

fn utf16_range_to_byte_range(text: &str, range: &std::ops::Range<usize>) -> std::ops::Range<usize> {
    let byte_offset = |target| {
        let mut units = 0;
        let mut bytes = 0;
        for character in text.chars() {
            if units >= target {
                break;
            }
            units += character.len_utf16();
            bytes += character.len_utf8();
        }
        bytes
    };
    byte_offset(range.start)..byte_offset(range.end)
}

impl Focusable for FlashShotApp {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::{byte_range_to_utf16_range, utf16_range_to_byte_range};

    #[test]
    fn utf16_ranges_round_trip_for_mixed_language_and_surrogate_pair_text() {
        let text = "Hello, 中文 👋";
        let chinese_start = text.find('中').unwrap();
        let emoji_start = text.find('👋').unwrap();
        let range = chinese_start..emoji_start;

        let utf16 = byte_range_to_utf16_range(text, &range);
        assert_eq!(&text[utf16_range_to_byte_range(text, &utf16)], "中文 ");

        let emoji_utf16 = byte_range_to_utf16_range(text, &(emoji_start..text.len()));
        assert_eq!(emoji_utf16.end - emoji_utf16.start, 2);
        assert_eq!(
            utf16_range_to_byte_range(text, &emoji_utf16),
            emoji_start..text.len()
        );
    }
}
