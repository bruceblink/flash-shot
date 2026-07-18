//! GPUI capture workspace state and module boundaries.

mod overlay;
mod render_image;
mod view;
mod workflow;

use std::sync::Arc;

use gpui::{AsyncApp, Context, FocusHandle, Focusable, RenderImage, WeakEntity, WindowHandle};

use crate::{
    domain::{geometry::PhysicalPoint, selection::SelectionDrag, session::CaptureSession},
    performance::PerformanceRecorder,
    platform::{
        capture::CaptureFrame,
        shortcut::{GlobalShortcutService, ShortcutEvent},
        tray::{TrayEvent, TrayService},
        window_inspector::InspectionTarget,
    },
    theme::ThemeColors,
};

pub struct FlashShotApp {
    colors: ThemeColors,
    session: CaptureSession,
    frame: Option<CaptureFrame>,
    preview: Option<Arc<RenderImage>>,
    selection_drag: SelectionDrag,
    hover_pixel: Option<PhysicalPoint>,
    inspection_target: Option<InspectionTarget>,
    pending_click_target: Option<InspectionTarget>,
    inspection_request: Option<PhysicalPoint>,
    inspection_in_flight: bool,
    overlay_windows: Vec<WindowHandle<overlay::CaptureOverlay>>,
    main_window_handle: Option<isize>,
    focus_handle: FocusHandle,
    status: String,
    performance: PerformanceRecorder,
    _shortcut: Option<GlobalShortcutService>,
    _tray: Option<TrayService>,
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

        Self {
            colors: ThemeColors::default(),
            session: CaptureSession::default(),
            frame: None,
            preview: None,
            selection_drag: SelectionDrag::default(),
            hover_pixel: None,
            inspection_target: None,
            pending_click_target: None,
            inspection_request: None,
            inspection_in_flight: false,
            overlay_windows: Vec::new(),
            main_window_handle: None,
            focus_handle: cx.focus_handle(),
            status,
            performance,
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

impl Focusable for FlashShotApp {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
