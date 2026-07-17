//! GPUI capture workspace and screenshot workflow orchestration.

use std::{cell::Cell, rc::Rc, sync::Arc, time::Instant};

use gpui::{
    AsyncApp, BorderStyle, Bounds, Context, FocusHandle, Focusable, Image, ImageFormat,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, Pixels,
    Render, WeakEntity, Window, canvas, div, fill, img, outline, point, prelude::*, px, size,
};

use crate::{
    domain::{
        geometry::PhysicalPoint,
        selection::{PreviewTransform, SelectionDrag, ViewPoint, ViewRect},
        session::{CaptureSession, CaptureSessionState},
    },
    performance::PerformanceRecorder,
    platform::{
        capture::{CaptureBackend, CaptureFrame, SystemCaptureBackend},
        clipboard::{ClipboardService, SystemClipboard},
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
        self.hover_pixel = None;
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
        self.hover_pixel = None;
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

    fn nudge_selection(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if event.keystroke.modifiers.control
            || event.keystroke.modifiers.alt
            || event.keystroke.modifiers.platform
            || event.keystroke.modifiers.function
        {
            return;
        }
        let step = if event.keystroke.modifiers.shift {
            10
        } else {
            1
        };
        let (delta_x, delta_y) = match event.keystroke.key.as_str() {
            "left" => (-step, 0),
            "right" => (step, 0),
            "up" => (0, -step),
            "down" => (0, step),
            _ => return,
        };
        let Some(frame) = self.frame.as_ref() else {
            return;
        };
        let Some(selection) = self.selection_drag.nudge(frame.bounds, delta_x, delta_y) else {
            return;
        };
        if self.session.select(selection).is_ok() {
            self.status = format!(
                "Selection: {} x {} physical pixels",
                selection.width(),
                selection.height()
            );
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn update_selection(
        &mut self,
        event: &MouseMoveEvent,
        viewport: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.update_hover_pixel(event, viewport, cx);
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

    fn update_hover_pixel(
        &mut self,
        event: &MouseMoveEvent,
        viewport: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let hover_pixel = self
            .preview_transform(viewport)
            .and_then(|transform| transform.view_to_pixel(view_point(event.position)));
        if self.hover_pixel == hover_pixel {
            return;
        }
        self.hover_pixel = hover_pixel;
        if let Some((point, color)) = hover_pixel.and_then(|point| {
            self.frame
                .as_ref()?
                .pixel_at(point)
                .map(|color| (point, color))
        }) {
            self.status = if let Some(selection) = self.selection_drag.selection() {
                format!(
                    "{} x {} px | ({}, {}) {}",
                    selection.width(),
                    selection.height(),
                    point.x,
                    point.y,
                    color.hex_rgb()
                )
            } else {
                format!("({}, {}) {}", point.x, point.y, color.hex_rgb())
            };
        } else if let Some(selection) = self.selection_drag.selection() {
            self.status = format!(
                "{} x {} physical pixels",
                selection.width(),
                selection.height()
            );
        } else if let Some(frame) = self.frame.as_ref() {
            self.status = format!("{} x {} physical pixels", frame.width, frame.height);
        }
        cx.notify();
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

    fn copy_selection(&mut self, cx: &mut Context<Self>) {
        let selection = match self.session.start_export() {
            Ok(selection) => selection,
            Err(error) => {
                self.status = error.to_string();
                cx.notify();
                return;
            }
        };
        let Some(frame) = self.frame.clone() else {
            let message = "capture frame is unavailable".to_owned();
            let _ = self.session.fail(message.clone());
            self.status = message;
            cx.notify();
            return;
        };

        self.status = "Copying selection...".to_owned();
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move { copy_frame_selection(&frame, selection, &SystemClipboard) })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| this.finish_copy(result, cx));
                }
            }
        })
        .detach();
    }

    fn finish_copy(&mut self, result: std::io::Result<()>, cx: &mut Context<Self>) {
        match result {
            Ok(()) => {
                if let Err(error) = self.session.export_completed() {
                    self.status = error.to_string();
                } else {
                    self.status = "Selection copied to clipboard".to_owned();
                }
            }
            Err(error) => {
                let message = format!("Copy failed: {error}");
                let _ = self.session.fail(message.clone());
                self.status = message;
            }
        }
        cx.notify();
    }

    fn preview_transform(&self, viewport: Bounds<Pixels>) -> Option<PreviewTransform> {
        let frame = self.frame.as_ref()?;
        PreviewTransform::contain(frame.bounds, view_rect(viewport))
    }
}

impl Focusable for FlashShotApp {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
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

fn copy_frame_selection(
    frame: &CaptureFrame,
    selection: crate::domain::geometry::PhysicalRect,
    clipboard: &impl ClipboardService,
) -> std::io::Result<()> {
    clipboard.copy_image(&frame.crop(selection)?)
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

fn paint_magnifier(
    window: &mut Window,
    viewport: Bounds<Pixels>,
    transform: PreviewTransform,
    frame: &CaptureFrame,
    hover_pixel: PhysicalPoint,
    colors: ThemeColors,
) {
    const GRID_RADIUS: i32 = 4;
    const CELL_SIZE: f32 = 10.0;
    const GRID_SIZE: f32 = CELL_SIZE * (GRID_RADIUS as f32 * 2.0 + 1.0);
    const PADDING: f32 = 3.0;
    const OFFSET: f32 = 18.0;

    let cursor = transform.physical_to_view(hover_pixel);
    let viewport = view_rect(viewport);
    let panel_size = GRID_SIZE + PADDING * 2.0;
    let left = if cursor.x + OFFSET + panel_size <= viewport.right() {
        cursor.x + OFFSET
    } else {
        cursor.x - OFFSET - panel_size
    }
    .clamp(
        viewport.left,
        (viewport.right() - panel_size).max(viewport.left),
    );
    let top = if cursor.y + OFFSET + panel_size <= viewport.bottom() {
        cursor.y + OFFSET
    } else {
        cursor.y - OFFSET - panel_size
    }
    .clamp(
        viewport.top,
        (viewport.bottom() - panel_size).max(viewport.top),
    );

    let panel_bounds = Bounds::new(
        point(px(left), px(top)),
        size(px(panel_size), px(panel_size)),
    );
    window.paint_quad(fill(panel_bounds, colors.panel));
    window.paint_quad(outline(panel_bounds, colors.border, BorderStyle::Solid));

    for row in -GRID_RADIUS..=GRID_RADIUS {
        for column in -GRID_RADIUS..=GRID_RADIUS {
            let sample = PhysicalPoint {
                x: (hover_pixel.x + column).clamp(frame.bounds.left, frame.bounds.right - 1),
                y: (hover_pixel.y + row).clamp(frame.bounds.top, frame.bounds.bottom - 1),
            };
            let Some(color) = frame.pixel_at(sample) else {
                continue;
            };
            let cell_left = left + PADDING + (column + GRID_RADIUS) as f32 * CELL_SIZE;
            let cell_top = top + PADDING + (row + GRID_RADIUS) as f32 * CELL_SIZE;
            let cell_bounds = Bounds::new(
                point(px(cell_left), px(cell_top)),
                size(px(CELL_SIZE), px(CELL_SIZE)),
            );
            window.paint_quad(fill(cell_bounds, gpui::rgba(color.rgba_u32())));
            if row == 0 && column == 0 {
                window.paint_quad(outline(cell_bounds, colors.accent, BorderStyle::Solid));
            }
        }
    }
}

impl Render for FlashShotApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors;
        let is_idle = self.session.state() == CaptureSessionState::Idle;
        let is_busy = self.session.state() == CaptureSessionState::Capturing;
        let is_exporting = self.session.state() == CaptureSessionState::Exporting;
        let preview = self.preview.clone();
        let frame_bounds = self.frame.as_ref().map(|frame| frame.bounds);
        let frame = self.frame.clone();
        let selection = self.selection_drag.selection();
        let can_copy =
            selection.is_some() && self.session.state() == CaptureSessionState::Selecting;
        let hover_pixel = self.hover_pixel;
        let viewport_bounds = Rc::new(Cell::new(Bounds::default()));

        div()
            .size_full()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event, _, cx| this.nudge_selection(event, cx)))
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
                                        cx.listener(move |this, event, window, cx| {
                                            this.focus_handle.focus(window, cx);
                                            this.begin_selection(event, mouse_bounds.get());
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
                                                (
                                                    frame.clone(),
                                                    frame_bounds,
                                                    selection,
                                                    hover_pixel,
                                                )
                                            },
                                            move |bounds,
                                                  (frame, frame_bounds, selection, hover_pixel),
                                                  window,
                                                  _| {
                                                let Some(frame_bounds) = frame_bounds else {
                                                    return;
                                                };
                                                let Some(transform) = PreviewTransform::contain(
                                                    frame_bounds,
                                                    view_rect(bounds),
                                                ) else {
                                                    return;
                                                };
                                                if let Some(selection) = selection {
                                                    let start =
                                                        transform.physical_to_view(PhysicalPoint {
                                                            x: selection.left,
                                                            y: selection.top,
                                                        });
                                                    let end =
                                                        transform.physical_to_view(PhysicalPoint {
                                                            x: selection.right,
                                                            y: selection.bottom,
                                                        });
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
                                                }

                                                if let Some((frame, hover_pixel)) =
                                                    frame.zip(hover_pixel)
                                                {
                                                    paint_magnifier(
                                                        window,
                                                        bounds,
                                                        transform,
                                                        &frame,
                                                        hover_pixel,
                                                        colors,
                                                    );
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
                                .flex()
                                .items_center()
                                .gap_3()
                                .when(can_copy, |actions| {
                                    actions.child(
                                        div()
                                            .id("copy-selection")
                                            .px_3()
                                            .py_1()
                                            .rounded_md()
                                            .cursor_pointer()
                                            .text_sm()
                                            .bg(colors.accent)
                                            .text_color(colors.background)
                                            .on_click(
                                                cx.listener(|this, _, _, cx| {
                                                    this.copy_selection(cx)
                                                }),
                                            )
                                            .child("Copy"),
                                    )
                                })
                                .when(!is_exporting, |actions| {
                                    actions.child(
                                        div()
                                            .id("cancel-capture")
                                            .px_3()
                                            .py_1()
                                            .cursor_pointer()
                                            .text_sm()
                                            .text_color(colors.accent)
                                            .on_click(cx.listener(|this, _, _, cx| this.reset(cx)))
                                            .child(
                                                if self.session.state()
                                                    == CaptureSessionState::Completed
                                                {
                                                    "Done"
                                                } else {
                                                    "Cancel"
                                                },
                                            ),
                                    )
                                }),
                        )
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::copy_frame_selection;
    use crate::{
        domain::geometry::PhysicalRect,
        platform::{
            capture::{CaptureFrame, PixelFormat},
            clipboard::ClipboardService,
        },
    };
    use std::{cell::RefCell, io, sync::Arc, time::Duration};

    #[derive(Default)]
    struct RecordingClipboard {
        copied: RefCell<Option<CaptureFrame>>,
    }

    impl ClipboardService for RecordingClipboard {
        fn copy_image(&self, frame: &CaptureFrame) -> io::Result<()> {
            self.copied.replace(Some(frame.clone()));
            Ok(())
        }
    }

    #[test]
    fn copy_uses_the_pixel_correct_selected_region() {
        let frame = CaptureFrame {
            bounds: PhysicalRect {
                left: -2,
                top: 10,
                right: 1,
                bottom: 12,
            },
            width: 3,
            height: 2,
            stride: 12,
            format: PixelFormat::Bgra8,
            pixels: Arc::from([
                1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255, 13, 14, 15, 255, 16, 17,
                18, 255,
            ]),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 1,
        };
        let clipboard = RecordingClipboard::default();

        copy_frame_selection(
            &frame,
            PhysicalRect {
                left: -1,
                top: 10,
                right: 1,
                bottom: 12,
            },
            &clipboard,
        )
        .unwrap();

        let copied = clipboard.copied.borrow();
        let copied = copied.as_ref().unwrap();
        assert_eq!((copied.width, copied.height), (2, 2));
        assert_eq!(
            copied.pixels.as_ref(),
            &[4, 5, 6, 255, 7, 8, 9, 255, 13, 14, 15, 255, 16, 17, 18, 255]
        );
    }
}
