//! GPUI capture workspace and screenshot workflow orchestration.

use std::{cell::Cell, rc::Rc, sync::Arc, time::Instant};

use gpui::{
    AsyncApp, BorderStyle, Bounds, Context, Image, ImageFormat, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ObjectFit, Pixels, Render, WeakEntity, Window, canvas, div, fill,
    img, outline, point, prelude::*, px, size,
};

use crate::{
    domain::{
        selection::{PreviewTransform, SelectionDrag, ViewPoint, ViewRect},
        session::{CaptureSession, CaptureSessionState},
    },
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
    selection_drag: SelectionDrag,
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
        self.selection_drag.clear();
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
        self.selection_drag.clear();
        self.status = "Ready - Ctrl+Shift+Print Screen".to_owned();
        cx.notify();
    }

    fn begin_selection(&mut self, event: &MouseDownEvent, viewport: Bounds<Pixels>) {
        let Some(transform) = self.preview_transform(viewport) else {
            return;
        };
        let view_point = view_point(event.position);
        if let Some((selection, handle)) = self.selection_drag.selection().and_then(|selection| {
            transform
                .resize_handle_at(selection, view_point, 10.0)
                .map(|handle| (selection, handle))
        }) {
            self.selection_drag.begin_resize(selection, handle);
        } else if let Some(point) = transform.view_to_physical(view_point) {
            self.selection_drag.begin(point);
        }
    }

    fn update_selection(
        &mut self,
        event: &MouseMoveEvent,
        viewport: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if !event.dragging() {
            return;
        }
        let Some(transform) = self.preview_transform(viewport) else {
            return;
        };
        let fitted = transform.fitted_view();
        let clamped = ViewPoint {
            x: f32::from(event.position.x).clamp(fitted.left, fitted.right()),
            y: f32::from(event.position.y).clamp(fitted.top, fitted.bottom()),
        };
        if let Some(point) = transform.view_to_physical(clamped) {
            self.selection_drag.update(point);
            if let Some(selection) = self.selection_drag.selection() {
                self.status = format!(
                    "Selection: {} x {} physical pixels",
                    selection.width(),
                    selection.height()
                );
            }
            cx.notify();
        }
    }

    fn finish_selection(
        &mut self,
        event: &MouseUpEvent,
        viewport: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(transform) = self.preview_transform(viewport) else {
            return;
        };
        let fitted = transform.fitted_view();
        let clamped = ViewPoint {
            x: f32::from(event.position.x).clamp(fitted.left, fitted.right()),
            y: f32::from(event.position.y).clamp(fitted.top, fitted.bottom()),
        };
        if let Some(point) = transform.view_to_physical(clamped) {
            self.selection_drag.update(point);
        }
        if let Some(selection) = self.selection_drag.selection()
            && selection.width() > 0
            && selection.height() > 0
        {
            let _ = self.session.select(selection);
            self.status = format!(
                "Selection: {} x {} physical pixels",
                selection.width(),
                selection.height()
            );
        }
        cx.notify();
    }

    fn preview_transform(&self, viewport: Bounds<Pixels>) -> Option<PreviewTransform> {
        let frame = self.frame.as_ref()?;
        PreviewTransform::contain(frame.bounds, view_rect(viewport))
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

fn view_rect(bounds: Bounds<Pixels>) -> ViewRect {
    ViewRect {
        left: f32::from(bounds.origin.x),
        top: f32::from(bounds.origin.y),
        width: f32::from(bounds.size.width),
        height: f32::from(bounds.size.height),
    }
}

fn view_point(position: gpui::Point<Pixels>) -> ViewPoint {
    ViewPoint {
        x: f32::from(position.x),
        y: f32::from(position.y),
    }
}

impl Render for FlashShotApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors;
        let is_idle = self.session.state() == CaptureSessionState::Idle;
        let is_busy = self.session.state() == CaptureSessionState::Capturing;
        let preview = self.preview.clone();
        let frame_bounds = self.frame.as_ref().map(|frame| frame.bounds);
        let selection = self.selection_drag.selection();
        let viewport_bounds = Rc::new(Cell::new(Bounds::default()));

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
                        let measured_bounds = viewport_bounds.clone();
                        let mouse_bounds = viewport_bounds.clone();
                        let move_bounds = viewport_bounds.clone();
                        let up_bounds = viewport_bounds.clone();
                        div()
                            .size_full()
                            .relative()
                            .border_1()
                            .border_color(colors.border)
                            .bg(colors.panel)
                            .child(img(preview).size_full().object_fit(ObjectFit::Contain))
                            .child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .left_0()
                                    .right_0()
                                    .bottom_0()
                                    .cursor_crosshair()
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |this, event, _, _| {
                                            this.begin_selection(event, mouse_bounds.get())
                                        }),
                                    )
                                    .on_mouse_move(cx.listener(move |this, event, _, cx| {
                                        this.update_selection(event, move_bounds.get(), cx)
                                    }))
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(move |this, event, _, cx| {
                                            this.finish_selection(event, up_bounds.get(), cx)
                                        }),
                                    )
                                    .child(
                                        canvas(
                                            move |bounds, _, _| {
                                                measured_bounds.set(bounds);
                                                (frame_bounds, selection)
                                            },
                                            move |bounds, (frame_bounds, selection), window, _| {
                                                let Some((frame_bounds, selection)) =
                                                    frame_bounds.zip(selection)
                                                else {
                                                    return;
                                                };
                                                let Some(transform) = PreviewTransform::contain(
                                                    frame_bounds,
                                                    view_rect(bounds),
                                                ) else {
                                                    return;
                                                };
                                                let start = transform.physical_to_view(
                                                    crate::domain::geometry::PhysicalPoint {
                                                        x: selection.left,
                                                        y: selection.top,
                                                    },
                                                );
                                                let end = transform.physical_to_view(
                                                    crate::domain::geometry::PhysicalPoint {
                                                        x: selection.right,
                                                        y: selection.bottom,
                                                    },
                                                );
                                                window.paint_quad(outline(
                                                    Bounds::new(
                                                        point(px(start.x), px(start.y)),
                                                        size(
                                                            px(end.x - start.x),
                                                            px(end.y - start.y),
                                                        ),
                                                    ),
                                                    colors.accent,
                                                    BorderStyle::Solid,
                                                ));
                                                for handle in [
                                                    start,
                                                    ViewPoint {
                                                        x: end.x,
                                                        y: start.y,
                                                    },
                                                    ViewPoint {
                                                        x: start.x,
                                                        y: end.y,
                                                    },
                                                    end,
                                                ] {
                                                    window.paint_quad(fill(
                                                        Bounds::new(
                                                            point(
                                                                px(handle.x - 4.0),
                                                                px(handle.y - 4.0),
                                                            ),
                                                            size(px(8.0), px(8.0)),
                                                        ),
                                                        colors.accent,
                                                    ));
                                                }
                                            },
                                        )
                                        .size_full(),
                                    ),
                            )
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
