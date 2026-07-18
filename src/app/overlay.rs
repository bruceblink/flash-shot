//! Per-display borderless capture overlays backed by the shared capture session.

use std::sync::Arc;

use gpui::{
    Bounds, Context, Entity, FocusHandle, Focusable, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ObjectFit, Pixels, Render, RenderImage, Subscription, Window,
    canvas, div, fill, img, point, prelude::*, px, rgba, size,
};

use super::FlashShotApp;
use crate::{
    domain::{
        annotation::{Annotation, AnnotationKind, AnnotationTool},
        geometry::{PhysicalPoint, PhysicalRect},
        selection::{PreviewTransform, ViewPoint, ViewRect},
    },
    platform::cursor,
    platform::display::DisplayInfo,
    theme::ThemeColors,
};

// Keep the action row above a scaled Windows taskbar when the borderless overlay
// extends over the full display rather than the working area.
const OVERLAY_BOTTOM_SAFE_INSET: f32 = 96.0;

pub(super) struct CaptureOverlay {
    app: Entity<FlashShotApp>,
    display: DisplayInfo,
    preview: Arc<RenderImage>,
    focus_handle: FocusHandle,
    _app_observation: Subscription,
}

impl CaptureOverlay {
    pub(super) fn new(
        app: Entity<FlashShotApp>,
        display: DisplayInfo,
        preview: Arc<RenderImage>,
        cx: &mut Context<Self>,
    ) -> Self {
        let observation = cx.observe(&app, |_, _, cx| cx.notify());
        Self {
            app,
            display,
            preview,
            focus_handle: cx.focus_handle(),
            _app_observation: observation,
        }
    }

    fn transform(&self, viewport: Bounds<Pixels>) -> Option<PreviewTransform> {
        PreviewTransform::contain(self.display.physical_bounds, view_rect(viewport))
    }

    fn begin_selection(
        &mut self,
        event: &MouseDownEvent,
        viewport: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(point) = self
            .transform(viewport)
            .and_then(|transform| transform.view_to_physical(view_point(event.position)))
        else {
            return;
        };
        let resize_handle = self
            .app
            .read(cx)
            .selection_drag
            .selection()
            .and_then(|selection| {
                self.transform(viewport)?.resize_handle_at(
                    selection,
                    view_point(event.position),
                    10.0,
                )
            });
        let app = self.app.clone();
        cx.defer(move |cx| {
            app.update(cx, |app, cx| {
                app.begin_overlay_selection(point, resize_handle);
                cx.notify();
            })
        });
    }

    fn update_selection(
        &mut self,
        event: &MouseMoveEvent,
        viewport: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(transform) = self.transform(viewport) else {
            return;
        };
        let point = transform.view_to_pixel(view_point(event.position));
        let app = self.app.clone();
        let dragging_point = event
            .dragging()
            .then(cursor::position)
            .transpose()
            .ok()
            .flatten();
        cx.defer(move |cx| {
            app.update(cx, |app, cx| {
                app.update_overlay_hover(point, cx);
                if let Some(point) = dragging_point {
                    app.update_overlay_selection(point, cx);
                }
            })
        });
    }

    fn finish_selection(
        &mut self,
        event: &MouseUpEvent,
        viewport: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let point = cursor::position().ok().or_else(|| {
            self.transform(viewport).and_then(|transform| {
                transform.view_to_physical(clamp_to_view(transform, event.position))
            })
        });
        let Some(point) = point else { return };
        let app = self.app.clone();
        cx.defer(move |cx| app.update(cx, |app, cx| app.finish_overlay_selection(point, cx)));
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let app = self.app.clone();
        let event = event.clone();
        cx.defer(move |cx| app.update(cx, |app, cx| app.handle_key_down(&event, cx)));
    }
}

impl Focusable for CaptureOverlay {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for CaptureOverlay {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = ThemeColors::default();
        let display_bounds = self.display.physical_bounds;
        let app = self.app.read(cx);
        let selection = app.selection_drag.selection();
        let inspection_target = app.inspection_target;
        let annotations = app
            .annotation_document
            .as_ref()
            .map(|document| document.annotations().to_vec())
            .unwrap_or_default();
        let annotation_preview = app
            .annotation_document
            .as_ref()
            .and_then(|document| app.annotation_editor.preview(document.canvas_bounds()));
        let selected_tool = app.annotation_tool;
        let status = app.status.clone();
        let viewport = local_viewport(window);
        let transform = self.transform(viewport);
        let selected_on_display =
            selection.and_then(|selection| intersect(selection, display_bounds));
        let target_on_display = selection
            .is_none()
            .then(|| inspection_target.and_then(|target| intersect(target.bounds, display_bounds)))
            .flatten();
        let can_export = selection.is_some();

        div()
            .size_full()
            .relative()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .bg(colors.background)
            .child(
                img(self.preview.clone())
                    .size_full()
                    .object_fit(ObjectFit::Fill),
            )
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
                            this.begin_selection(event, local_viewport(window), cx);
                        }),
                    )
                    .on_mouse_move(cx.listener(move |this, event, window, cx| {
                        this.update_selection(event, local_viewport(window), cx)
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, event, window, cx| {
                            this.finish_selection(event, local_viewport(window), cx)
                        }),
                    )
                    .child(
                        canvas(
                            move |bounds, _, _| {
                                (bounds, transform, selected_on_display, target_on_display)
                            },
                            move |bounds, (_, transform, selection, target), window, _| {
                                paint_selection_mask(
                                    window, bounds, transform, selection, target, colors,
                                )
                            },
                        )
                        .size_full(),
                    ),
            )
            .child(
                canvas(
                    move |bounds, _, _| (bounds, transform, annotations, annotation_preview),
                    move |bounds, (_, transform, annotations, preview), window, _| {
                        paint_annotations(
                            window,
                            bounds,
                            transform,
                            &annotations,
                            preview.as_ref(),
                            colors,
                        )
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
                    .absolute()
                    .left(px(18.0))
                    .top(px(18.0))
                    .flex()
                    .gap_2()
                    .child(
                        div()
                            .id("overlay-tool-selection")
                            .px_3()
                            .py_2()
                            .bg(if selected_tool.is_some() {
                                colors.panel
                            } else {
                                colors.accent
                            })
                            .text_color(if selected_tool.is_some() {
                                colors.text
                            } else {
                                colors.background
                            })
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| {
                                let app = this.app.clone();
                                cx.defer(move |cx| {
                                    app.update(cx, |app, cx| app.select_selection_tool(cx));
                                });
                            }))
                            .child("Select"),
                    )
                    .child(
                        div()
                            .id("overlay-tool-rectangle")
                            .px_3()
                            .py_2()
                            .bg(if selected_tool == Some(AnnotationTool::Rectangle) {
                                colors.accent
                            } else {
                                colors.panel
                            })
                            .text_color(if selected_tool == Some(AnnotationTool::Rectangle) {
                                colors.background
                            } else {
                                colors.text
                            })
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| {
                                let app = this.app.clone();
                                cx.defer(move |cx| {
                                    app.update(cx, |app, cx| app.select_rectangle_tool(cx));
                                });
                            }))
                            .child("Rectangle"),
                    )
                    .child(
                        div()
                            .id("overlay-tool-ellipse")
                            .px_3()
                            .py_2()
                            .bg(if selected_tool == Some(AnnotationTool::Ellipse) {
                                colors.accent
                            } else {
                                colors.panel
                            })
                            .text_color(if selected_tool == Some(AnnotationTool::Ellipse) {
                                colors.background
                            } else {
                                colors.text
                            })
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| {
                                let app = this.app.clone();
                                cx.defer(move |cx| {
                                    app.update(cx, |app, cx| app.select_ellipse_tool(cx));
                                });
                            }))
                            .child("Ellipse"),
                    )
                    .child(
                        div()
                            .id("overlay-tool-line")
                            .px_3()
                            .py_2()
                            .bg(if selected_tool == Some(AnnotationTool::Line) {
                                colors.accent
                            } else {
                                colors.panel
                            })
                            .text_color(if selected_tool == Some(AnnotationTool::Line) {
                                colors.background
                            } else {
                                colors.text
                            })
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| {
                                let app = this.app.clone();
                                cx.defer(move |cx| {
                                    app.update(cx, |app, cx| app.select_line_tool(cx));
                                });
                            }))
                            .child("Line"),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .left(px(18.0))
                    .bottom(px(OVERLAY_BOTTOM_SAFE_INSET))
                    .px_3()
                    .py_2()
                    .bg(rgba(0x111827D9))
                    .text_color(colors.text)
                    .text_sm()
                    .child(status),
            )
            .child(
                div()
                    .absolute()
                    .right(px(18.0))
                    .bottom(px(OVERLAY_BOTTOM_SAFE_INSET))
                    .flex()
                    .gap_2()
                    .when(can_export, |actions| {
                        actions
                            .child(
                                div()
                                    .id("overlay-copy")
                                    .px_3()
                                    .py_2()
                                    .bg(colors.accent)
                                    .text_color(colors.background)
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let app = this.app.clone();
                                        cx.defer(move |cx| {
                                            app.update(cx, |app, cx| app.copy_selection(cx))
                                        });
                                    }))
                                    .child("Copy"),
                            )
                            .child(
                                div()
                                    .id("overlay-save")
                                    .px_3()
                                    .py_2()
                                    .bg(rgba(0x111827E6))
                                    .text_color(colors.text)
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let app = this.app.clone();
                                        cx.defer(move |cx| {
                                            app.update(cx, |app, cx| app.save_selection(cx))
                                        });
                                    }))
                                    .child("Save"),
                            )
                            .child(
                                div()
                                    .id("overlay-quick-save")
                                    .px_3()
                                    .py_2()
                                    .bg(rgba(0x111827E6))
                                    .text_color(colors.text)
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let app = this.app.clone();
                                        cx.defer(move |cx| {
                                            app.update(cx, |app, cx| app.quick_save_selection(cx))
                                        });
                                    }))
                                    .child("Quick save"),
                            )
                    })
                    .child(
                        div()
                            .id("overlay-cancel")
                            .px_3()
                            .py_2()
                            .bg(rgba(0x111827E6))
                            .text_color(colors.text)
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| {
                                let app = this.app.clone();
                                cx.defer(move |cx| app.update(cx, |app, cx| app.reset(cx)));
                            }))
                            .child("Cancel"),
                    ),
            )
    }
}

fn paint_selection_mask(
    window: &mut Window,
    viewport: Bounds<Pixels>,
    transform: Option<PreviewTransform>,
    selection: Option<PhysicalRect>,
    target: Option<PhysicalRect>,
    colors: ThemeColors,
) {
    let Some(transform) = transform else {
        window.paint_quad(fill(viewport, rgba(0x00000066)));
        return;
    };
    let Some(selection) = selection else {
        window.paint_quad(fill(viewport, rgba(0x00000066)));
        if let Some(target) = target {
            paint_outline(window, transform, target, colors.accent);
        }
        return;
    };
    let start = transform.physical_to_view(PhysicalPoint {
        x: selection.left,
        y: selection.top,
    });
    let end = transform.physical_to_view(PhysicalPoint {
        x: selection.right,
        y: selection.bottom,
    });
    let selection = Bounds::new(
        point(px(start.x), px(start.y)),
        size(px(end.x - start.x), px(end.y - start.y)),
    );
    let shade = rgba(0x00000066);
    window.paint_quad(fill(
        Bounds::new(
            viewport.origin,
            size(viewport.size.width, selection.origin.y - viewport.origin.y),
        ),
        shade,
    ));
    window.paint_quad(fill(
        Bounds::new(
            point(viewport.origin.x, selection.bottom()),
            size(viewport.size.width, viewport.bottom() - selection.bottom()),
        ),
        shade,
    ));
    window.paint_quad(fill(
        Bounds::new(
            point(viewport.origin.x, selection.origin.y),
            size(
                selection.origin.x - viewport.origin.x,
                selection.size.height,
            ),
        ),
        shade,
    ));
    window.paint_quad(fill(
        Bounds::new(
            point(selection.right(), selection.origin.y),
            size(viewport.right() - selection.right(), selection.size.height),
        ),
        shade,
    ));
    window.paint_quad(gpui::outline(
        selection,
        colors.accent,
        gpui::BorderStyle::Solid,
    ));
}

fn paint_annotations(
    window: &mut Window,
    _viewport: Bounds<Pixels>,
    transform: Option<PreviewTransform>,
    annotations: &[Annotation],
    preview: Option<&Annotation>,
    colors: ThemeColors,
) {
    let Some(transform) = transform else {
        return;
    };
    for annotation in annotations.iter().chain(preview) {
        match annotation.kind {
            AnnotationKind::Rectangle { bounds } => {
                paint_outline(window, transform, bounds, colors.accent)
            }
            AnnotationKind::Ellipse { bounds } => {
                paint_ellipse_outline(window, transform, bounds, colors.accent)
            }
            AnnotationKind::Line { start, end } => {
                paint_line(window, transform, start, end, colors.accent)
            }
            _ => {}
        }
    }
}

#[cfg(test)]
fn outline_shape_bounds(annotation: &Annotation) -> Option<PhysicalRect> {
    match annotation.kind {
        AnnotationKind::Rectangle { bounds } | AnnotationKind::Ellipse { bounds } => Some(bounds),
        _ => None,
    }
}

fn paint_outline(
    window: &mut Window,
    transform: PreviewTransform,
    rect: PhysicalRect,
    color: gpui::Hsla,
) {
    let start = transform.physical_to_view(PhysicalPoint {
        x: rect.left,
        y: rect.top,
    });
    let end = transform.physical_to_view(PhysicalPoint {
        x: rect.right,
        y: rect.bottom,
    });
    window.paint_quad(gpui::outline(
        Bounds::new(
            point(px(start.x), px(start.y)),
            size(px(end.x - start.x), px(end.y - start.y)),
        ),
        color,
        gpui::BorderStyle::Solid,
    ));
}

fn paint_line(
    window: &mut Window,
    transform: PreviewTransform,
    start: PhysicalPoint,
    end: PhysicalPoint,
    color: gpui::Hsla,
) {
    let start = transform.physical_to_view(start);
    let end = transform.physical_to_view(end);
    let mut path = gpui::PathBuilder::stroke(px(1.0));
    path.move_to(point(px(start.x), px(start.y)));
    path.line_to(point(px(end.x), px(end.y)));
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

fn paint_ellipse_outline(
    window: &mut Window,
    transform: PreviewTransform,
    rect: PhysicalRect,
    color: gpui::Hsla,
) {
    let start = transform.physical_to_view(PhysicalPoint {
        x: rect.left,
        y: rect.top,
    });
    let end = transform.physical_to_view(PhysicalPoint {
        x: rect.right,
        y: rect.bottom,
    });
    let center_x = (start.x + end.x) / 2.0;
    let center_y = (start.y + end.y) / 2.0;
    let radius_x = (end.x - start.x).abs() / 2.0;
    let radius_y = (end.y - start.y).abs() / 2.0;
    if radius_x == 0.0 || radius_y == 0.0 {
        return;
    }
    let mut path = gpui::PathBuilder::stroke(px(1.0));
    const SEGMENTS: u32 = 32;
    for index in 0..=SEGMENTS {
        let angle = std::f32::consts::TAU * index as f32 / SEGMENTS as f32;
        let point = point(
            px(center_x + radius_x * angle.cos()),
            px(center_y + radius_y * angle.sin()),
        );
        if index == 0 {
            path.move_to(point);
        } else {
            path.line_to(point);
        }
    }
    path.close();
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

fn view_rect(bounds: Bounds<Pixels>) -> ViewRect {
    ViewRect {
        left: f32::from(bounds.origin.x),
        top: f32::from(bounds.origin.y),
        width: f32::from(bounds.size.width),
        height: f32::from(bounds.size.height),
    }
}

fn local_viewport(window: &Window) -> Bounds<Pixels> {
    Bounds::new(point(px(0.0), px(0.0)), window.bounds().size)
}

fn view_point(position: gpui::Point<Pixels>) -> ViewPoint {
    ViewPoint {
        x: f32::from(position.x),
        y: f32::from(position.y),
    }
}

fn clamp_to_view(transform: PreviewTransform, position: gpui::Point<Pixels>) -> ViewPoint {
    let fitted = transform.fitted_view();
    ViewPoint {
        x: f32::from(position.x).clamp(fitted.left, fitted.right()),
        y: f32::from(position.y).clamp(fitted.top, fitted.bottom()),
    }
}

fn intersect(left: PhysicalRect, right: PhysicalRect) -> Option<PhysicalRect> {
    let result = PhysicalRect {
        left: left.left.max(right.left),
        top: left.top.max(right.top),
        right: left.right.min(right.right),
        bottom: left.bottom.min(right.bottom),
    };
    (result.width() > 0 && result.height() > 0).then_some(result)
}

#[cfg(test)]
mod tests {
    use super::{intersect, outline_shape_bounds};
    use crate::domain::{
        annotation::{Annotation, AnnotationId, AnnotationKind, AnnotationStyle},
        geometry::{PhysicalPoint, PhysicalRect},
    };

    #[test]
    fn clips_shared_selection_to_each_display() {
        let selection = PhysicalRect {
            left: -200,
            top: 100,
            right: 300,
            bottom: 500,
        };
        let display = PhysicalRect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };

        assert_eq!(
            intersect(selection, display),
            Some(PhysicalRect {
                left: 0,
                top: 100,
                right: 300,
                bottom: 500,
            })
        );
    }

    #[test]
    fn shape_bounds_helper_selects_rectangle_and_ellipse_but_not_line_geometry() {
        let rectangle = Annotation {
            id: AnnotationId::new(1),
            kind: AnnotationKind::Rectangle {
                bounds: PhysicalRect {
                    left: 10,
                    top: 20,
                    right: 30,
                    bottom: 40,
                },
            },
            style: AnnotationStyle::default(),
        };
        let ellipse = Annotation {
            id: AnnotationId::new(2),
            kind: AnnotationKind::Ellipse {
                bounds: PhysicalRect {
                    left: 20,
                    top: 30,
                    right: 40,
                    bottom: 50,
                },
            },
            style: AnnotationStyle::default(),
        };
        let line = Annotation {
            id: AnnotationId::new(3),
            kind: AnnotationKind::Line {
                start: PhysicalPoint { x: 10, y: 20 },
                end: PhysicalPoint { x: 30, y: 40 },
            },
            style: AnnotationStyle::default(),
        };

        assert_eq!(
            outline_shape_bounds(&rectangle),
            Some(PhysicalRect {
                left: 10,
                top: 20,
                right: 30,
                bottom: 40,
            })
        );
        assert_eq!(
            outline_shape_bounds(&ellipse),
            Some(PhysicalRect {
                left: 20,
                top: 30,
                right: 40,
                bottom: 50,
            })
        );
        assert_eq!(outline_shape_bounds(&line), None);
    }
}
