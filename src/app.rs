//! GPUI capture workspace state and module boundaries.

mod view;
mod workflow;

use std::sync::Arc;

use gpui::{AsyncApp, Context, FocusHandle, Focusable, Image, WeakEntity};

use crate::{
    domain::{geometry::PhysicalPoint, selection::SelectionDrag, session::CaptureSession},
    performance::PerformanceRecorder,
    platform::{
        capture::CaptureFrame,
        shortcut::{GlobalShortcutService, ShortcutEvent},
    },
    theme::ThemeColors,
};

pub struct FlashShotApp {
    colors: ThemeColors,
    session: CaptureSession,
    frame: Option<CaptureFrame>,
    preview: Option<Arc<Image>>,
    selection_drag: SelectionDrag,
    hover_pixel: Option<PhysicalPoint>,
    focus_handle: FocusHandle,
    status: String,
    performance: PerformanceRecorder,
    _shortcut: Option<GlobalShortcutService>,
}

impl FlashShotApp {
    pub fn new(performance: PerformanceRecorder, cx: &mut Context<Self>) -> Self {
        let shortcut = match GlobalShortcutService::register_capture() {
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
            "Ready - Ctrl+Shift+Print Screen".to_owned()
        } else {
            "Ready - global shortcut unavailable".to_owned()
        };

        Self {
            colors: ThemeColors::default(),
            session: CaptureSession::default(),
            frame: None,
            preview: None,
            selection_drag: SelectionDrag::default(),
            hover_pixel: None,
            focus_handle: cx.focus_handle(),
            status,
            performance,
            _shortcut: shortcut,
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
}

impl Focusable for FlashShotApp {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
