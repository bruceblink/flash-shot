//! GPUI capture workspace and screenshot workflow orchestration.

use std::{sync::Arc, time::Instant};

use gpui::{
    AsyncApp, Context, Image, ImageFormat, ObjectFit, Render, WeakEntity, Window, div, img,
    prelude::*, px,
};

use crate::{
    domain::session::{CaptureSession, CaptureSessionState},
    performance::PerformanceRecorder,
    platform::{
        capture::{CaptureBackend, CaptureFrame, SystemCaptureBackend},
        display::{DisplayProvider, SystemDisplayProvider},
        shortcut::{GlobalShortcutService, ShortcutEvent},
    },
    theme::ThemeColors,
};

pub struct FlashShotApp {
    colors: ThemeColors,
    session: CaptureSession,
    frame: Option<CaptureFrame>,
    preview: Option<Arc<Image>>,
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

    fn start_capture(&mut self, cx: &mut Context<Self>) {
        if self.session.state() != CaptureSessionState::Idle {
            return;
        }
        if let Err(error) = self.session.begin() {
            self.status = error.to_string();
            cx.notify();
            return;
        }
        self.frame = None;
        self.preview = None;
        self.status = "Capturing primary display...".to_owned();
        cx.notify();

        let started_at = Instant::now();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move { capture_primary_display() })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.finish_capture(result, started_at, cx)
                    });
                }
            }
        })
        .detach();
    }

    fn finish_capture(
        &mut self,
        result: std::io::Result<(CaptureFrame, Vec<u8>)>,
        started_at: Instant,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok((frame, png)) => {
                if let Err(error) = self.session.frames_ready() {
                    self.status = error.to_string();
                    cx.notify();
                    return;
                }
                self.performance
                    .record_duration("shortcut_to_frame_ready", started_at.elapsed());
                self.status = format!(
                    "{} x {} physical pixels - {:.1} ms - {} CPU copy",
                    frame.width,
                    frame.height,
                    frame.capture_duration.as_secs_f64() * 1_000.0,
                    frame.cpu_copy_count
                );
                self.preview = Some(Arc::new(Image::from_bytes(ImageFormat::Png, png)));
                self.frame = Some(frame);
            }
            Err(error) => {
                let message = format!("Capture failed: {error}");
                let _ = self.session.fail(message.clone());
                self.status = message;
                log::warn!(target: "flash_shot::capture", "capture_failed error={error}");
            }
        }
        cx.notify();
    }

    fn reset(&mut self, cx: &mut Context<Self>) {
        if matches!(
            self.session.state(),
            CaptureSessionState::Selecting
                | CaptureSessionState::Completed
                | CaptureSessionState::Cancelled
                | CaptureSessionState::Failed
        ) {
            if self.session.state() == CaptureSessionState::Selecting {
                let _ = self.session.cancel();
            }
            let _ = self.session.reset();
        }
        self.frame = None;
        self.preview = None;
        self.status = "Ready - Ctrl+Shift+Print Screen".to_owned();
        cx.notify();
    }
}

fn capture_primary_display() -> std::io::Result<(CaptureFrame, Vec<u8>)> {
    let display = SystemDisplayProvider
        .displays()?
        .into_iter()
        .find(|display| display.primary)
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "primary display missing")
        })?;
    let frame = SystemCaptureBackend.capture(display.physical_bounds)?;
    let png = frame.encode_png()?;
    Ok((frame, png))
}

impl Render for FlashShotApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors;
        let is_idle = self.session.state() == CaptureSessionState::Idle;
        let is_busy = self.session.state() == CaptureSessionState::Capturing;
        let preview = self.preview.clone();

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.background)
            .text_color(colors.text)
            .child(
                div()
                    .h(px(58.0))
                    .px_5()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(colors.border)
                    .child(div().text_lg().child("Flash Shot"))
                    .child(
                        div()
                            .id("capture-action")
                            .px_4()
                            .py_2()
                            .rounded_md()
                            .bg(if is_idle {
                                colors.accent
                            } else {
                                colors.border
                            })
                            .text_color(colors.background)
                            .when(is_idle, |button| {
                                button
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| this.start_capture(cx)))
                            })
                            .child(if is_busy { "Capturing..." } else { "Capture" }),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .p_5()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(if let Some(preview) = preview {
                        div()
                            .size_full()
                            .border_1()
                            .border_color(colors.border)
                            .bg(colors.panel)
                            .child(img(preview).size_full().object_fit(ObjectFit::Contain))
                            .into_any_element()
                    } else {
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap_3()
                            .text_color(colors.muted)
                            .child(div().text_lg().child("No capture selected"))
                            .child(div().text_sm().child("Ctrl+Shift+Print Screen"))
                            .into_any_element()
                    }),
            )
            .child(
                div()
                    .h(px(48.0))
                    .px_5()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_t_1()
                    .border_color(colors.border)
                    .child(
                        div()
                            .text_sm()
                            .text_color(colors.muted)
                            .child(self.status.clone()),
                    )
                    .when(!is_idle, |bar| {
                        bar.child(
                            div()
                                .id("cancel-capture")
                                .px_3()
                                .py_1()
                                .cursor_pointer()
                                .text_sm()
                                .text_color(colors.accent)
                                .on_click(cx.listener(|this, _, _, cx| this.reset(cx)))
                                .child("Cancel"),
                        )
                    }),
            )
    }
}
