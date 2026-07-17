//! GPUI rendering for the capture workspace.

use std::{cell::Cell, rc::Rc};

use gpui::{
    BorderStyle, Bounds, MouseButton, ObjectFit, Pixels, Render, Window, canvas, div, fill, img,
    outline, point, prelude::*, px, size,
};

use super::{FlashShotApp, workflow::view_rect};
use crate::{
    domain::{
        geometry::PhysicalPoint,
        selection::{PreviewTransform, ViewPoint},
        session::CaptureSessionState,
    },
    platform::capture::CaptureFrame,
    theme::ThemeColors,
};

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
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let colors = self.colors;
        let is_idle = self.session.state() == CaptureSessionState::Idle;
        let is_busy = self.session.state() == CaptureSessionState::Capturing;
        let is_exporting = self.session.state() == CaptureSessionState::Exporting;
        let preview = self.preview.clone();
        let frame_bounds = self.frame.as_ref().map(|frame| frame.bounds);
        let frame = self.frame.clone();
        let selection = self.selection_drag.selection();
        let inspection_target = self.inspection_target;
        let can_export =
            selection.is_some() && self.session.state() == CaptureSessionState::Selecting;
        let hover_pixel = self.hover_pixel;
        let viewport_bounds = Rc::new(Cell::new(Bounds::default()));

        div()
            .size_full()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event, _, cx| this.handle_key_down(event, cx)))
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
                                                    inspection_target,
                                                    hover_pixel,
                                                )
                                            },
                                            move |bounds,
                                                  (
                                                frame,
                                                frame_bounds,
                                                selection,
                                                inspection_target,
                                                hover_pixel,
                                            ),
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
                                                if selection.is_none()
                                                    && let Some(target) = inspection_target
                                                {
                                                    let start =
                                                        transform.physical_to_view(PhysicalPoint {
                                                            x: target.bounds.left,
                                                            y: target.bounds.top,
                                                        });
                                                    let end =
                                                        transform.physical_to_view(PhysicalPoint {
                                                            x: target.bounds.right,
                                                            y: target.bounds.bottom,
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
                                                }
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
                                .when(can_export, |actions| {
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
                                .when(can_export, |actions| {
                                    actions.child(
                                        div()
                                            .id("save-selection")
                                            .px_3()
                                            .py_1()
                                            .rounded_md()
                                            .cursor_pointer()
                                            .text_sm()
                                            .border_1()
                                            .border_color(colors.accent)
                                            .text_color(colors.accent)
                                            .on_click(
                                                cx.listener(|this, _, _, cx| {
                                                    this.save_selection(cx)
                                                }),
                                            )
                                            .child("Save"),
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
