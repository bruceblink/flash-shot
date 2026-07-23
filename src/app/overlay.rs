//! Per-display borderless capture overlays backed by the shared capture session.

use std::sync::Arc;

use gpui::{
    Bounds, Context, ElementInputHandler, Entity, FocusHandle, Focusable, Hsla, KeyDownEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, Pixels, Render,
    RenderImage, Subscription, TextAlign, TextRun, Window, canvas, div, fill, img, point,
    prelude::*, px, rgba, size,
};

use super::FlashShotApp;
use crate::{
    domain::{
        annotation::{
            Annotation, AnnotationId, AnnotationKind, AnnotationTool, SEQUENCE_MARKER_RADIUS,
        },
        geometry::{PhysicalPoint, PhysicalRect},
        selection::{PreviewTransform, SelectionDrag, ViewPoint, ViewRect},
    },
    platform::cursor,
    platform::display::DisplayInfo,
    theme::ThemeColors,
};

const OVERLAY_EDGE_INSET: f32 = 18.0;
// Keep fallback controls above a scaled Windows taskbar when the borderless
// overlay extends over the full display rather than the working area.
const OVERLAY_BOTTOM_SAFE_INSET: f32 = 96.0;
const OVERLAY_ACTION_BAR_WIDTH: f32 = 620.0;
const OVERLAY_ACTION_BAR_GAP: f32 = 12.0;
const OVERLAY_ACTION_ITEM_GAP: f32 = 4.0;
const OVERLAY_ACTION_ITEM_HEIGHT: f32 = 34.0;
const OVERLAY_ACTION_BAR_PADDING: f32 = 4.0;
const OVERLAY_DIMENSION_LABEL_WIDTH: f32 = 112.0;
const OVERLAY_DIMENSION_LABEL_HEIGHT: f32 = 26.0;
const OVERLAY_DIMENSION_LABEL_GAP: f32 = 8.0;
// Keep the terminal screenshot actions on one compact row while optional
// actions expand only after the user asks for them.
const OVERLAY_PRIMARY_ACTION_WIDTHS: [f32; 6] = [76.0, 48.0, 52.0, 52.0, 56.0, 62.0];
const OVERLAY_MORE_ACTION_WIDTHS: [f32; 11] = [
    150.0, 125.0, 150.0, 100.0, 65.0, 55.0, 90.0, 95.0, 115.0, 80.0, 60.0,
];
const OVERLAY_RECOGNITION_ACTION_WIDTHS: [f32; 2] = [60.0, 90.0];
const ANNOTATION_COLORS: [u32; 5] = [0xFF3B30FF, 0xFFCC00FF, 0x34C759FF, 0x007AFFFF, 0xAF52DEFF];
const ANNOTATION_WIDTHS: [u32; 4] = [1, 3, 6, 10];
const ANNOTATION_FONT_SIZES: [u32; 4] = [16, 24, 32, 48];
const ANNOTATION_OPACITIES: [u8; 4] = [255, 192, 128, 64];
const MAGNIFIER_RADIUS: i32 = 4;
const MAGNIFIER_CELL_SIZE: f32 = 12.0;
const MAGNIFIER_GAP: f32 = 18.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SelectionCursor {
    Crosshair,
    Move,
    ResizeNwse,
    ResizeNesw,
}

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
        let annotation_resize_handle = self.app.read(cx).selected_annotation.and_then(|id| {
            let annotation = self
                .app
                .read(cx)
                .annotation_document
                .as_ref()?
                .annotation(id)?;
            self.transform(viewport)?.resize_handle_at(
                annotation.bounds(),
                view_point(event.position),
                10.0,
            )
        });
        let app = self.app.clone();
        cx.defer(move |cx| {
            app.update(cx, |app, cx| {
                app.begin_overlay_selection(point, resize_handle, annotation_resize_handle);
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
        let preserve_aspect_ratio = event.modifiers.shift;
        let resize_from_center = event.modifiers.alt;
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
                    app.update_overlay_selection(
                        point,
                        preserve_aspect_ratio,
                        resize_from_center,
                        cx,
                    );
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
        let preserve_aspect_ratio = event.modifiers.shift;
        let resize_from_center = event.modifiers.alt;
        cx.defer(move |cx| {
            app.update(cx, |app, cx| {
                app.finish_overlay_selection(point, preserve_aspect_ratio, resize_from_center, cx)
            })
        });
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let app = self.app.clone();
        let event = event.clone();
        cx.defer(move |cx| {
            app.update(cx, |app, cx| {
                if !app.handle_text_edit_key(&event.keystroke, cx) {
                    app.handle_key_down(&event, cx);
                }
            })
        });
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
        // The session owns the committed selection. Keep rendering it after the
        // drag has ended, even if a late pointer event clears transient UI state.
        let selection = visible_selection(app.selection_drag, app.session.selection());
        let inspection_target = app.inspection_target;
        let annotations = app
            .annotation_document
            .as_ref()
            .map(|document| document.annotations().to_vec())
            .unwrap_or_default();
        let layer_annotations = annotations.clone();
        let annotation_preview = app
            .annotation_document
            .as_ref()
            .and_then(|document| app.annotation_editor.preview(document.canvas_bounds()));
        let text_edit = app.text_edit().cloned();
        let text_edit_annotation = app.text_edit_annotation();
        let selected_annotation = app.selected_annotation;
        let can_delete = selected_annotation.is_some();
        let selected_tool = app.annotation_tool;
        let can_edit_text = selected_annotation
            .and_then(|id| app.annotation_document.as_ref()?.annotation(id))
            .is_some_and(|annotation| {
                matches!(
                    annotation.kind,
                    AnnotationKind::Text { .. } | AnnotationKind::Watermark { .. }
                )
            });
        let can_adjust_font_size = selected_annotation
            .and_then(|id| app.annotation_document.as_ref()?.annotation(id))
            .is_some_and(is_text_annotation)
            || matches!(
                selected_tool,
                Some(AnnotationTool::Text | AnnotationTool::Watermark)
            );
        let can_rotate = selected_annotation
            .and_then(|id| app.annotation_document.as_ref()?.annotation(id))
            .is_some_and(Annotation::supports_clockwise_rotation);
        let can_fill = selected_annotation
            .and_then(|id| app.annotation_document.as_ref()?.annotation(id))
            .is_some_and(Annotation::supports_fill)
            || selected_tool.is_some_and(AnnotationTool::supports_fill);
        let selected_number = selected_annotation.and_then(|id| {
            app.annotation_document
                .as_ref()?
                .annotation(id)
                .and_then(|annotation| match annotation.kind {
                    AnnotationKind::Number { value, .. } => Some(value),
                    _ => None,
                })
        });
        let annotation_color = app.annotation_style.stroke_rgba;
        let annotation_width = app.annotation_style.stroke_width;
        let annotation_font_size = app.annotation_style.text_font_size;
        let annotation_opacity = (app.annotation_style.stroke_rgba & 0xFF) as u8;
        let fill_enabled = app.annotation_style.fill_rgba.is_some();
        let can_undo = app.annotation_history.undo_len() > 0;
        let can_redo = app.annotation_history.redo_len() > 0;
        let status = app.status.clone();
        let show_more_actions = app.overlay_more_actions;
        let show_annotation_controls = app.overlay_annotation_controls;
        let recognition_result = app.recognition_result.clone();
        let hover_pixel = app.hover_pixel;
        let frame = app.frame.clone();
        let viewport = local_viewport(window);
        let transform = self.transform(viewport);
        let selected_on_display =
            selection.and_then(|selection| intersect(selection, display_bounds));
        let action_layout = action_toolbar_layout(
            selected_on_display,
            transform,
            viewport,
            show_more_actions,
            recognition_result.is_some(),
        );
        let dimension_layout = selection_dimension_label_layout(
            selected_on_display,
            transform,
            viewport,
            action_layout,
        );
        let target_on_display = selection
            .is_none()
            .then(|| inspection_target.and_then(|target| intersect(target.bounds, display_bounds)))
            .flatten();
        let can_export = selection.is_some();
        let selection_cursor = selection_cursor(
            selection,
            transform,
            view_point(window.mouse_position()),
            selected_tool.is_some(),
            app.selection_drag.is_moving(),
        );
        if text_edit.is_some() {
            window.handle_input(
                &self.focus_handle,
                ElementInputHandler::new(viewport, self.app.clone()),
                cx,
            );
        }

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
                    .when(selection_cursor == SelectionCursor::Move, |overlay| {
                        overlay.cursor_move()
                    })
                    .when(selection_cursor == SelectionCursor::ResizeNwse, |overlay| {
                        overlay.cursor_nwse_resize()
                    })
                    .when(selection_cursor == SelectionCursor::ResizeNesw, |overlay| {
                        overlay.cursor_nesw_resize()
                    })
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
                    move |bounds, _, _| {
                        (
                            bounds,
                            transform,
                            annotations,
                            annotation_preview,
                            selected_annotation,
                            text_edit_annotation,
                            text_edit,
                        )
                    },
                    move |_bounds,
                          (
                        _,
                        transform,
                        annotations,
                        preview,
                        selected_annotation,
                        text_edit_annotation,
                        text_edit,
                    ),
                          window,
                          cx| {
                        paint_annotations(
                            window,
                            transform,
                            &annotations,
                            preview.as_ref(),
                            AnnotationPaintState {
                                selected: selected_annotation,
                                hidden: text_edit_annotation,
                            },
                            colors,
                            cx,
                        );
                        if let Some(edit) = text_edit {
                            paint_text_annotation(
                                window,
                                transform.unwrap_or_else(|| unreachable!()),
                                edit.origin,
                                &edit.content,
                                0xFFFFFFFF,
                                annotation_font_size,
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
                canvas(
                    move |bounds, _, _| (bounds, transform, hover_pixel, frame),
                    move |viewport, (_, transform, hover_pixel, frame), window, _| {
                        paint_magnifier(window, viewport, transform, hover_pixel, frame.as_ref())
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .bottom_0(),
            )
            .when_some(dimension_layout, |overlay, layout| {
                let selection = selected_on_display.expect("dimension layout requires selection");
                overlay.child(
                    div()
                        .id("overlay-selection-dimensions")
                        .absolute()
                        .left(px(layout.left))
                        .top(px(layout.top))
                        .w(px(OVERLAY_DIMENSION_LABEL_WIDTH))
                        .h(px(OVERLAY_DIMENSION_LABEL_HEIGHT))
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(rgba(0x111827E6))
                        .border_1()
                        .border_color(colors.accent)
                        .text_color(colors.text)
                        .text_sm()
                        .child(format!("{} x {} px", selection.width(), selection.height())),
                )
            })
            .when(!layer_annotations.is_empty(), |overlay| {
                overlay.child(
                    div()
                        .absolute()
                        .right(px(18.0))
                        .top(px(18.0))
                        .w(px(180.0))
                        .p_2()
                        .bg(rgba(0x111827E6))
                        .border_1()
                        .border_color(colors.border)
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().text_sm().text_color(colors.muted).child("Layers"))
                        .children(layer_annotations.iter().rev().enumerate().map(
                            |(reverse_index, annotation)| {
                                let id = annotation.id;
                                let position = layer_annotations.len() - reverse_index;
                                let is_selected = selected_annotation == Some(id);
                                div()
                                    .id(format!("overlay-layer-{}", id.value()))
                                    .px_2()
                                    .py_1()
                                    .bg(if is_selected {
                                        colors.accent
                                    } else {
                                        colors.panel
                                    })
                                    .text_color(if is_selected {
                                        colors.background
                                    } else {
                                        colors.text
                                    })
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let app = this.app.clone();
                                        cx.defer(move |cx| {
                                            app.update(cx, |app, cx| {
                                                app.select_annotation_layer(id, cx);
                                            });
                                        });
                                    }))
                                    .child(format!(
                                        "{position}. {}",
                                        annotation_layer_label(&annotation.kind)
                                    ))
                            },
                        )),
                )
            })
            .when(show_annotation_controls, |overlay| {
                overlay.child(
                    div()
                        .absolute()
                        .left(px(OVERLAY_EDGE_INSET))
                        .top(px(18.0))
                        .flex()
                        .gap_2()
                        .when(can_undo, |tools| {
                            tools.child(
                                div()
                                    .id("overlay-undo")
                                    .px_3()
                                    .py_2()
                                    .bg(colors.panel)
                                    .text_color(colors.text)
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let app = this.app.clone();
                                        cx.defer(move |cx| {
                                            app.update(cx, |app, cx| app.undo_annotation(cx));
                                        });
                                    }))
                                    .child("Undo"),
                            )
                        })
                        .when(can_redo, |tools| {
                            tools.child(
                                div()
                                    .id("overlay-redo")
                                    .px_3()
                                    .py_2()
                                    .bg(colors.panel)
                                    .text_color(colors.text)
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let app = this.app.clone();
                                        cx.defer(move |cx| {
                                            app.update(cx, |app, cx| app.redo_annotation(cx));
                                        });
                                    }))
                                    .child("Redo"),
                            )
                        })
                        .when(can_delete, |tools| {
                            tools.child(
                                div()
                                    .id("overlay-delete")
                                    .px_3()
                                    .py_2()
                                    .bg(rgba(0xB91C1CFF))
                                    .text_color(colors.text)
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let app = this.app.clone();
                                        cx.defer(move |cx| {
                                            app.update(cx, |app, cx| {
                                                app.delete_selected_annotation(cx);
                                            });
                                        });
                                    }))
                                    .child("Delete"),
                            )
                        })
                        .when(can_edit_text, |tools| {
                            tools.child(
                                div()
                                    .id("overlay-edit-text")
                                    .px_3()
                                    .py_2()
                                    .bg(colors.panel)
                                    .text_color(colors.text)
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let app = this.app.clone();
                                        cx.defer(move |cx| {
                                            app.update(cx, |app, cx| {
                                                app.edit_selected_text_annotation(cx);
                                            });
                                        });
                                    }))
                                    .child("Edit text"),
                            )
                        })
                        .when_some(selected_number, |tools, value| {
                            tools
                                .child(
                                    div()
                                        .id("overlay-number-decrement")
                                        .px_3()
                                        .py_2()
                                        .bg(colors.panel)
                                        .text_color(colors.text)
                                        .cursor_pointer()
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            let app = this.app.clone();
                                            cx.defer(move |cx| {
                                                app.update(cx, |app, cx| {
                                                    app.adjust_selected_number(-1, cx);
                                                });
                                            });
                                        }))
                                        .child("-"),
                                )
                                .child(
                                    div()
                                        .px_2()
                                        .py_2()
                                        .text_color(colors.text)
                                        .child(value.to_string()),
                                )
                                .child(
                                    div()
                                        .id("overlay-number-increment")
                                        .px_3()
                                        .py_2()
                                        .bg(colors.panel)
                                        .text_color(colors.text)
                                        .cursor_pointer()
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            let app = this.app.clone();
                                            cx.defer(move |cx| {
                                                app.update(cx, |app, cx| {
                                                    app.adjust_selected_number(1, cx);
                                                });
                                            });
                                        }))
                                        .child("+"),
                                )
                        })
                        .when(can_delete, |tools| {
                            tools.child(
                                div()
                                    .id("overlay-duplicate")
                                    .px_3()
                                    .py_2()
                                    .bg(colors.panel)
                                    .text_color(colors.text)
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let app = this.app.clone();
                                        cx.defer(move |cx| {
                                            app.update(cx, |app, cx| {
                                                app.duplicate_selected_annotation(cx);
                                            });
                                        });
                                    }))
                                    .child("Duplicate"),
                            )
                        })
                        .when(can_rotate, |tools| {
                            tools.child(
                                div()
                                    .id("overlay-rotate-clockwise")
                                    .px_3()
                                    .py_2()
                                    .bg(colors.panel)
                                    .text_color(colors.text)
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let app = this.app.clone();
                                        cx.defer(move |cx| {
                                            app.update(cx, |app, cx| {
                                                app.rotate_selected_annotation_clockwise(cx);
                                            });
                                        });
                                    }))
                                    .child("Rotate 90"),
                            )
                        })
                        .when(can_delete, |tools| {
                            tools
                                .child(
                                    div()
                                        .id("overlay-bring-forward")
                                        .px_3()
                                        .py_2()
                                        .bg(colors.panel)
                                        .text_color(colors.text)
                                        .cursor_pointer()
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            let app = this.app.clone();
                                            cx.defer(move |cx| {
                                                app.update(cx, |app, cx| {
                                                    app.bring_selected_annotation_forward(cx);
                                                });
                                            });
                                        }))
                                        .child("Forward"),
                                )
                                .child(
                                    div()
                                        .id("overlay-send-backward")
                                        .px_3()
                                        .py_2()
                                        .bg(colors.panel)
                                        .text_color(colors.text)
                                        .cursor_pointer()
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            let app = this.app.clone();
                                            cx.defer(move |cx| {
                                                app.update(cx, |app, cx| {
                                                    app.send_selected_annotation_backward(cx);
                                                });
                                            });
                                        }))
                                        .child("Backward"),
                                )
                                .child(
                                    div()
                                        .id("overlay-bring-to-front")
                                        .px_3()
                                        .py_2()
                                        .bg(colors.panel)
                                        .text_color(colors.text)
                                        .cursor_pointer()
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            let app = this.app.clone();
                                            cx.defer(move |cx| {
                                                app.update(cx, |app, cx| {
                                                    app.bring_selected_annotation_to_front(cx);
                                                });
                                            });
                                        }))
                                        .child("Front"),
                                )
                                .child(
                                    div()
                                        .id("overlay-send-to-back")
                                        .px_3()
                                        .py_2()
                                        .bg(colors.panel)
                                        .text_color(colors.text)
                                        .cursor_pointer()
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            let app = this.app.clone();
                                            cx.defer(move |cx| {
                                                app.update(cx, |app, cx| {
                                                    app.send_selected_annotation_to_back(cx);
                                                });
                                            });
                                        }))
                                        .child("Back"),
                                )
                        })
                        .child(
                            div()
                                .id("overlay-tool-watermark")
                                .px_3()
                                .py_2()
                                .bg(if selected_tool == Some(AnnotationTool::Watermark) {
                                    colors.accent
                                } else {
                                    colors.panel
                                })
                                .text_color(if selected_tool == Some(AnnotationTool::Watermark) {
                                    colors.background
                                } else {
                                    colors.text
                                })
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    let app = this.app.clone();
                                    cx.defer(move |cx| {
                                        app.update(cx, |app, cx| app.select_watermark_tool(cx));
                                    });
                                }))
                                .child("Watermark"),
                        )
                        .child(
                            div()
                                .id("overlay-tool-text")
                                .px_3()
                                .py_2()
                                .bg(if selected_tool == Some(AnnotationTool::Text) {
                                    colors.accent
                                } else {
                                    colors.panel
                                })
                                .text_color(if selected_tool == Some(AnnotationTool::Text) {
                                    colors.background
                                } else {
                                    colors.text
                                })
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    let app = this.app.clone();
                                    cx.defer(move |cx| {
                                        app.update(cx, |app, cx| app.select_text_tool(cx))
                                    });
                                }))
                                .child("Text"),
                        )
                        .child(
                            div()
                                .id("overlay-tool-number")
                                .px_3()
                                .py_2()
                                .bg(if selected_tool == Some(AnnotationTool::Number) {
                                    colors.accent
                                } else {
                                    colors.panel
                                })
                                .text_color(if selected_tool == Some(AnnotationTool::Number) {
                                    colors.background
                                } else {
                                    colors.text
                                })
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    let app = this.app.clone();
                                    cx.defer(move |cx| {
                                        app.update(cx, |app, cx| app.select_number_tool(cx))
                                    });
                                }))
                                .child("Number"),
                        )
                        .child(
                            div()
                                .id("overlay-tool-blur")
                                .px_3()
                                .py_2()
                                .bg(if selected_tool == Some(AnnotationTool::Blur) {
                                    colors.accent
                                } else {
                                    colors.panel
                                })
                                .text_color(if selected_tool == Some(AnnotationTool::Blur) {
                                    colors.background
                                } else {
                                    colors.text
                                })
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    let app = this.app.clone();
                                    cx.defer(move |cx| {
                                        app.update(cx, |app, cx| app.select_blur_tool(cx));
                                    });
                                }))
                                .child("Blur"),
                        )
                        .child(
                            div()
                                .id("overlay-tool-mosaic")
                                .px_3()
                                .py_2()
                                .bg(if selected_tool == Some(AnnotationTool::Mosaic) {
                                    colors.accent
                                } else {
                                    colors.panel
                                })
                                .text_color(if selected_tool == Some(AnnotationTool::Mosaic) {
                                    colors.background
                                } else {
                                    colors.text
                                })
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    let app = this.app.clone();
                                    cx.defer(move |cx| {
                                        app.update(cx, |app, cx| app.select_mosaic_tool(cx));
                                    });
                                }))
                                .child("Mosaic"),
                        )
                        .child(
                            div()
                                .id("overlay-tool-highlight")
                                .px_3()
                                .py_2()
                                .bg(if selected_tool == Some(AnnotationTool::Highlight) {
                                    colors.accent
                                } else {
                                    colors.panel
                                })
                                .text_color(if selected_tool == Some(AnnotationTool::Highlight) {
                                    colors.background
                                } else {
                                    colors.text
                                })
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    let app = this.app.clone();
                                    cx.defer(move |cx| {
                                        app.update(cx, |app, cx| app.select_highlight_tool(cx));
                                    });
                                }))
                                .child("Highlight"),
                        )
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
                        )
                        .child(
                            div()
                                .id("overlay-tool-arrow")
                                .px_3()
                                .py_2()
                                .bg(if selected_tool == Some(AnnotationTool::Arrow) {
                                    colors.accent
                                } else {
                                    colors.panel
                                })
                                .text_color(if selected_tool == Some(AnnotationTool::Arrow) {
                                    colors.background
                                } else {
                                    colors.text
                                })
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    let app = this.app.clone();
                                    cx.defer(move |cx| {
                                        app.update(cx, |app, cx| app.select_arrow_tool(cx));
                                    });
                                }))
                                .child("Arrow"),
                        )
                        .child(
                            div()
                                .id("overlay-tool-freehand")
                                .px_3()
                                .py_2()
                                .bg(if selected_tool == Some(AnnotationTool::Freehand) {
                                    colors.accent
                                } else {
                                    colors.panel
                                })
                                .text_color(if selected_tool == Some(AnnotationTool::Freehand) {
                                    colors.background
                                } else {
                                    colors.text
                                })
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    let app = this.app.clone();
                                    cx.defer(move |cx| {
                                        app.update(cx, |app, cx| app.select_freehand_tool(cx));
                                    });
                                }))
                                .child("Freehand"),
                        ),
                )
            })
            .when(show_annotation_controls, |overlay| {
                overlay.child(
                    div()
                        .absolute()
                        .left(px(18.0))
                        .top(px(58.0))
                        .flex()
                        .gap_2()
                        .children(ANNOTATION_COLORS.into_iter().map(|color| {
                            div()
                                .id(format!("overlay-color-{color:08x}"))
                                .w(px(22.0))
                                .h(px(22.0))
                                .bg(rgba(color))
                                .border_2()
                                .border_color(if color == annotation_color {
                                    colors.text
                                } else {
                                    colors.panel
                                })
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let app = this.app.clone();
                                    cx.defer(move |cx| {
                                        app.update(cx, |app, cx| {
                                            app.select_annotation_color(color, cx)
                                        });
                                    });
                                }))
                        })),
                )
            })
            .when(
                show_annotation_controls && can_adjust_font_size,
                |overlay| {
                    overlay.child(
                        div()
                            .absolute()
                            .left(px(18.0))
                            .top(px(178.0))
                            .flex()
                            .gap_2()
                            .children(ANNOTATION_FONT_SIZES.into_iter().map(|font_size| {
                                div()
                                    .id(format!("overlay-font-size-{font_size}"))
                                    .w(px(30.0))
                                    .h(px(22.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .bg(colors.panel)
                                    .border_2()
                                    .border_color(if font_size == annotation_font_size {
                                        colors.text
                                    } else {
                                        colors.border
                                    })
                                    .text_color(colors.text)
                                    .text_xs()
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let app = this.app.clone();
                                        cx.defer(move |cx| {
                                            app.update(cx, |app, cx| {
                                                app.select_annotation_font_size(font_size, cx)
                                            });
                                        });
                                    }))
                                    .child(font_size.to_string())
                            })),
                    )
                },
            )
            .when(show_annotation_controls && can_fill, |overlay| {
                overlay.child(
                    div()
                        .absolute()
                        .left(px(18.0))
                        .top(px(118.0))
                        .px_3()
                        .py_2()
                        .id("overlay-fill")
                        .bg(if fill_enabled {
                            colors.accent
                        } else {
                            colors.panel
                        })
                        .text_color(if fill_enabled {
                            colors.background
                        } else {
                            colors.text
                        })
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| {
                            let app = this.app.clone();
                            cx.defer(move |cx| {
                                app.update(cx, |app, cx| app.toggle_annotation_fill(cx));
                            });
                        }))
                        .child("Fill"),
                )
            })
            .when(show_annotation_controls, |overlay| {
                overlay.child(
                    div()
                        .absolute()
                        .left(px(18.0))
                        .top(px(88.0))
                        .flex()
                        .gap_2()
                        .children(ANNOTATION_WIDTHS.into_iter().map(|width| {
                            div()
                                .id(format!("overlay-width-{width}"))
                                .w(px(22.0))
                                .h(px(22.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(colors.panel)
                                .border_2()
                                .border_color(if width == annotation_width {
                                    colors.text
                                } else {
                                    colors.border
                                })
                                .text_color(colors.text)
                                .text_xs()
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let app = this.app.clone();
                                    cx.defer(move |cx| {
                                        app.update(cx, |app, cx| {
                                            app.select_annotation_width(width, cx)
                                        });
                                    });
                                }))
                                .child(width.to_string())
                        })),
                )
            })
            .when(show_annotation_controls, |overlay| {
                overlay.child(
                    div()
                        .absolute()
                        .left(px(18.0))
                        .top(px(148.0))
                        .flex()
                        .gap_2()
                        .children(ANNOTATION_OPACITIES.into_iter().map(|opacity| {
                            div()
                                .id(format!("overlay-opacity-{opacity}"))
                                .w(px(28.0))
                                .h(px(22.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(rgba((annotation_color & 0xFFFFFF00) | u32::from(opacity)))
                                .border_2()
                                .border_color(if opacity == annotation_opacity {
                                    colors.text
                                } else {
                                    colors.border
                                })
                                .text_color(colors.text)
                                .text_xs()
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let app = this.app.clone();
                                    cx.defer(move |cx| {
                                        app.update(cx, |app, cx| {
                                            app.select_annotation_opacity(opacity, cx)
                                        });
                                    });
                                }))
                                .child((u16::from(opacity) * 100 / 255).to_string())
                        })),
                )
            })
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
                    .when_some(action_layout, |actions, layout| {
                        actions
                            .left(px(layout.left))
                            .top(px(layout.top))
                            .w(px(layout.width))
                    })
                    .when(action_layout.is_none(), |actions| {
                        actions
                            .right(px(OVERLAY_EDGE_INSET))
                            .bottom(px(OVERLAY_BOTTOM_SAFE_INSET))
                    })
                    .flex()
                    .flex_wrap()
                    .justify_end()
                    .gap_1()
                    .p_1()
                    .rounded_md()
                    .border_1()
                    .border_color(rgba(0xFFFFFF24))
                    .bg(rgba(0x15171BF5))
                    .shadow_lg()
                    .when(can_export, |actions| {
                        actions
                            .child(
                                div()
                                    .id("overlay-annotation-controls")
                                    .w(px(76.0))
                                    .h(px(OVERLAY_ACTION_ITEM_HEIGHT))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_sm()
                                    .bg(if show_annotation_controls {
                                        colors.accent
                                    } else {
                                        colors.panel
                                    })
                                    .text_color(if show_annotation_controls {
                                        colors.background
                                    } else {
                                        colors.text
                                    })
                                    .text_sm()
                                    .cursor_pointer()
                                    .hover(|style| style.bg(rgba(0x3A4049FF)))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let app = this.app.clone();
                                        cx.defer(move |cx| {
                                            app.update(cx, |app, cx| {
                                                app.toggle_overlay_annotation_controls(cx)
                                            })
                                        });
                                    }))
                                    .child("Annotate"),
                            )
                            .child(
                                div()
                                    .id("overlay-pin")
                                    .w(px(48.0))
                                    .h(px(OVERLAY_ACTION_ITEM_HEIGHT))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_sm()
                                    .bg(colors.panel)
                                    .text_color(colors.text)
                                    .text_sm()
                                    .cursor_pointer()
                                    .hover(|style| style.bg(rgba(0x3A4049FF)))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let app = this.app.clone();
                                        cx.defer(move |cx| {
                                            app.update(cx, |app, cx| app.pin_selection(cx))
                                        });
                                    }))
                                    .child("Pin"),
                            )
                            .child(
                                div()
                                    .id("overlay-copy")
                                    .w(px(52.0))
                                    .h(px(OVERLAY_ACTION_ITEM_HEIGHT))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_sm()
                                    .bg(colors.accent)
                                    .text_color(colors.background)
                                    .text_sm()
                                    .cursor_pointer()
                                    .hover(|style| style.bg(rgba(0x81D4FAFF)))
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
                                    .w(px(52.0))
                                    .h(px(OVERLAY_ACTION_ITEM_HEIGHT))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_sm()
                                    .bg(colors.panel)
                                    .text_color(colors.text)
                                    .text_sm()
                                    .cursor_pointer()
                                    .hover(|style| style.bg(rgba(0x3A4049FF)))
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
                                    .id("overlay-more-actions")
                                    .w(px(56.0))
                                    .h(px(OVERLAY_ACTION_ITEM_HEIGHT))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_sm()
                                    .bg(if show_more_actions {
                                        Hsla::from(rgba(0x3A4049FF))
                                    } else {
                                        colors.panel
                                    })
                                    .text_color(colors.text)
                                    .text_sm()
                                    .cursor_pointer()
                                    .hover(|style| style.bg(rgba(0x3A4049FF)))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let app = this.app.clone();
                                        cx.defer(move |cx| {
                                            app.update(cx, |app, cx| {
                                                app.toggle_overlay_more_actions(cx)
                                            })
                                        });
                                    }))
                                    .child(if show_more_actions { "Less" } else { "More" }),
                            )
                            .child(
                                div()
                                    .id("overlay-cancel")
                                    .w(px(62.0))
                                    .h(px(OVERLAY_ACTION_ITEM_HEIGHT))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_sm()
                                    .bg(rgba(0x2A2023FF))
                                    .text_color(rgba(0xFFB4ABFF))
                                    .text_sm()
                                    .cursor_pointer()
                                    .hover(|style| style.bg(rgba(0x493035FF)))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let app = this.app.clone();
                                        cx.defer(move |cx| app.update(cx, |app, cx| app.reset(cx)));
                                    }))
                                    .child("Cancel"),
                            )
                            .when(show_more_actions, |actions| {
                                actions
                                    .child(
                                        div()
                                            .id("overlay-save-annotations")
                                            .px_3()
                                            .py_2()
                                            .bg(rgba(0x111827E6))
                                            .text_color(colors.text)
                                            .cursor_pointer()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.save_annotation_document(cx)
                                                    })
                                                });
                                            }))
                                            .child("Save annotations"),
                                    )
                                    .child(
                                        div()
                                            .id("overlay-save-editable-project")
                                            .px_3()
                                            .py_2()
                                            .bg(rgba(0x111827E6))
                                            .text_color(colors.text)
                                            .cursor_pointer()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.save_editable_project(cx)
                                                    })
                                                });
                                            }))
                                            .child("Save editable"),
                                    )
                                    .child(
                                        div()
                                            .id("overlay-open-annotations")
                                            .px_3()
                                            .py_2()
                                            .bg(rgba(0x111827E6))
                                            .text_color(colors.text)
                                            .cursor_pointer()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.open_annotation_document(cx)
                                                    })
                                                });
                                            }))
                                            .child("Open annotations"),
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
                                                    app.update(cx, |app, cx| {
                                                        app.quick_save_selection(cx)
                                                    })
                                                });
                                            }))
                                            .child("Quick save"),
                                    )
                                    .child(
                                        div()
                                            .id("overlay-manual-scroll")
                                            .px_3()
                                            .py_2()
                                            .bg(rgba(0x111827E6))
                                            .text_color(colors.text)
                                            .cursor_pointer()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.start_manual_scroll(cx)
                                                    })
                                                });
                                            }))
                                            .child("Scroll"),
                                    )
                                    .child(
                                        div()
                                            .id("overlay-qr")
                                            .px_3()
                                            .py_2()
                                            .bg(rgba(0x111827E6))
                                            .text_color(colors.text)
                                            .cursor_pointer()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.recognize_qr_selection(cx)
                                                    })
                                                });
                                            }))
                                            .child("QR"),
                                    )
                                    .child(
                                        div()
                                            .id("overlay-ocr")
                                            .px_3()
                                            .py_2()
                                            .bg(rgba(0x111827E6))
                                            .text_color(colors.text)
                                            .cursor_pointer()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.recognize_text_selection(cx)
                                                    })
                                                });
                                            }))
                                            .child("OCR"),
                                    )
                                    .child(
                                        div()
                                            .id("overlay-copy-color")
                                            .px_3()
                                            .py_2()
                                            .bg(rgba(0x111827E6))
                                            .text_color(colors.text)
                                            .cursor_pointer()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.copy_hover_color(cx)
                                                    })
                                                });
                                            }))
                                            .child("Copy color"),
                                    )
                                    .child(
                                        div()
                                            .id("overlay-translate")
                                            .px_3()
                                            .py_2()
                                            .bg(rgba(0x111827E6))
                                            .text_color(colors.text)
                                            .cursor_pointer()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.translate_selection(cx)
                                                    })
                                                });
                                            }))
                                            .child("Translate"),
                                    )
                                    .child(
                                        div()
                                            .id("overlay-record-area")
                                            .px_3()
                                            .py_2()
                                            .bg(rgba(0x111827E6))
                                            .text_color(colors.text)
                                            .cursor_pointer()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.start_region_recording(cx)
                                                    })
                                                });
                                            }))
                                            .child("Record area"),
                                    )
                                    .child(
                                        div()
                                            .id("overlay-record-window")
                                            .px_3()
                                            .py_2()
                                            .bg(rgba(0x111827E6))
                                            .text_color(colors.text)
                                            .cursor_pointer()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.start_selected_window_recording(cx)
                                                    })
                                                });
                                            }))
                                            .child("Record window"),
                                    )
                                    .when_some(recognition_result, |actions, _result| {
                                        actions
                                            .child(
                                                div()
                                                    .id("overlay-copy-recognition")
                                                    .px_3()
                                                    .py_2()
                                                    .bg(rgba(0x111827E6))
                                                    .text_color(colors.text)
                                                    .cursor_pointer()
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        let app = this.app.clone();
                                                        cx.defer(move |cx| {
                                                            app.update(cx, |app, cx| {
                                                                app.copy_recognition_result(cx)
                                                            })
                                                        });
                                                    }))
                                                    .child("Copy text"),
                                            )
                                            .child(
                                                div()
                                                    .id("overlay-clear-recognition")
                                                    .px_3()
                                                    .py_2()
                                                    .bg(rgba(0x111827E6))
                                                    .text_color(colors.text)
                                                    .cursor_pointer()
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        let app = this.app.clone();
                                                        cx.defer(move |cx| {
                                                            app.update(cx, |app, cx| {
                                                                app.clear_recognition_result(cx)
                                                            })
                                                        });
                                                    }))
                                                    .child("Clear result"),
                                            )
                                    })
                            })
                    })
                    .when(!can_export, |actions| {
                        actions.child(
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
                        )
                    }),
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
            paint_outline(window, transform, target, colors.accent, 1);
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
    let selection_bounds = Bounds::new(
        point(px(start.x), px(start.y)),
        size(px(end.x - start.x), px(end.y - start.y)),
    );
    let shade = rgba(0x00000066);
    window.paint_quad(fill(
        Bounds::new(
            viewport.origin,
            size(
                viewport.size.width,
                selection_bounds.origin.y - viewport.origin.y,
            ),
        ),
        shade,
    ));
    window.paint_quad(fill(
        Bounds::new(
            point(viewport.origin.x, selection_bounds.bottom()),
            size(
                viewport.size.width,
                viewport.bottom() - selection_bounds.bottom(),
            ),
        ),
        shade,
    ));
    window.paint_quad(fill(
        Bounds::new(
            point(viewport.origin.x, selection_bounds.origin.y),
            size(
                selection_bounds.origin.x - viewport.origin.x,
                selection_bounds.size.height,
            ),
        ),
        shade,
    ));
    window.paint_quad(fill(
        Bounds::new(
            point(selection_bounds.right(), selection_bounds.origin.y),
            size(
                viewport.right() - selection_bounds.right(),
                selection_bounds.size.height,
            ),
        ),
        shade,
    ));
    window.paint_quad(gpui::outline(
        selection_bounds,
        colors.accent,
        gpui::BorderStyle::Solid,
    ));
    // These are the same corners used by PreviewTransform::resize_handle_at,
    // so the visible affordance matches the physical-pixel hit targets.
    paint_resize_handles(window, transform, selection, colors.accent);
}

fn paint_magnifier(
    window: &mut Window,
    viewport: Bounds<Pixels>,
    transform: Option<PreviewTransform>,
    hover_pixel: Option<PhysicalPoint>,
    frame: Option<&crate::platform::capture::CaptureFrame>,
) {
    let (Some(transform), Some(center), Some(frame)) = (transform, hover_pixel, frame) else {
        return;
    };
    if !frame.bounds.contains(center) {
        return;
    }

    let view_center = transform.physical_to_view(center);
    let grid_cells = (MAGNIFIER_RADIUS * 2 + 1) as f32;
    let grid_size = grid_cells * MAGNIFIER_CELL_SIZE;
    let origin = magnifier_origin(view_center, viewport, grid_size);
    let panel = Bounds::new(
        point(px(origin.x - 4.0), px(origin.y - 4.0)),
        size(px(grid_size + 8.0), px(grid_size + 8.0)),
    );
    window.paint_quad(fill(panel, rgba(0x111827F2)));
    window.paint_quad(gpui::outline(
        panel,
        rgba(0xF4F6F8FF),
        gpui::BorderStyle::Solid,
    ));

    for row in -MAGNIFIER_RADIUS..=MAGNIFIER_RADIUS {
        for column in -MAGNIFIER_RADIUS..=MAGNIFIER_RADIUS {
            let sample_point = PhysicalPoint {
                x: center
                    .x
                    .saturating_add(column)
                    .clamp(frame.bounds.left, frame.bounds.right.saturating_sub(1)),
                y: center
                    .y
                    .saturating_add(row)
                    .clamp(frame.bounds.top, frame.bounds.bottom.saturating_sub(1)),
            };
            let Some(color) = frame.pixel_at(sample_point) else {
                continue;
            };
            let cell = Bounds::new(
                point(
                    px(origin.x + (column + MAGNIFIER_RADIUS) as f32 * MAGNIFIER_CELL_SIZE),
                    px(origin.y + (row + MAGNIFIER_RADIUS) as f32 * MAGNIFIER_CELL_SIZE),
                ),
                size(px(MAGNIFIER_CELL_SIZE), px(MAGNIFIER_CELL_SIZE)),
            );
            window.paint_quad(fill(cell, rgba(color.rgba_u32())));
            window.paint_quad(gpui::outline(
                cell,
                if row == 0 && column == 0 {
                    rgba(0xFFFFFFFF)
                } else {
                    rgba(0x00000055)
                },
                gpui::BorderStyle::Solid,
            ));
        }
    }
}

fn magnifier_origin(view_center: ViewPoint, viewport: Bounds<Pixels>, grid_size: f32) -> ViewPoint {
    let min_x = f32::from(viewport.origin.x) + 4.0;
    let min_y = f32::from(viewport.origin.y) + 4.0;
    let max_x = (f32::from(viewport.right()) - grid_size - 4.0).max(min_x);
    let max_y = (f32::from(viewport.bottom()) - grid_size - 4.0).max(min_y);
    ViewPoint {
        x: (view_center.x + MAGNIFIER_GAP).clamp(min_x, max_x),
        y: (view_center.y + MAGNIFIER_GAP).clamp(min_y, max_y),
    }
}

#[derive(Clone, Copy)]
struct AnnotationPaintState {
    selected: Option<AnnotationId>,
    hidden: Option<AnnotationId>,
}

fn paint_annotations(
    window: &mut Window,
    transform: Option<PreviewTransform>,
    annotations: &[Annotation],
    preview: Option<&Annotation>,
    state: AnnotationPaintState,
    colors: ThemeColors,
    cx: &mut gpui::App,
) {
    let Some(transform) = transform else {
        return;
    };
    for annotation in annotations
        .iter()
        .filter(|annotation| {
            Some(annotation.id) != preview.map(|preview| preview.id)
                && Some(annotation.id) != state.hidden
        })
        .chain(preview)
    {
        let color = rgba(annotation.style.stroke_rgba).into();
        match annotation.kind {
            AnnotationKind::Watermark {
                origin,
                ref content,
            } => paint_text_annotation(
                window,
                transform,
                origin,
                content,
                annotation.style.stroke_rgba,
                annotation.text_font_size(),
                cx,
            ),
            AnnotationKind::Text {
                origin,
                ref content,
            } => paint_text_annotation(
                window,
                transform,
                origin,
                content,
                annotation.style.stroke_rgba,
                annotation.text_font_size(),
                cx,
            ),
            AnnotationKind::Number { center, value } => paint_number_marker(
                window,
                transform,
                center,
                value,
                annotation.style.stroke_rgba,
            ),
            AnnotationKind::Blur { bounds } => {
                paint_rect_fill(window, transform, bounds, rgba(0xCBD5E188));
                paint_outline(window, transform, bounds, colors.muted, 1);
            }
            AnnotationKind::Mosaic { bounds } => {
                paint_rect_fill(window, transform, bounds, rgba(0x11182799));
                paint_mosaic_grid(window, transform, bounds, colors.muted);
            }
            AnnotationKind::Highlight { bounds } => {
                paint_rect_fill(
                    window,
                    transform,
                    bounds,
                    rgba(annotation.style.stroke_rgba),
                );
            }
            AnnotationKind::Rectangle { bounds } => {
                if let Some(fill_color) = annotation.style.fill_rgba {
                    paint_rect_fill(window, transform, bounds, rgba(fill_color));
                }
                paint_outline(
                    window,
                    transform,
                    bounds,
                    color,
                    annotation.style.stroke_width,
                )
            }
            AnnotationKind::Ellipse { bounds } => paint_ellipse_outline(
                window,
                transform,
                bounds,
                color,
                annotation.style.stroke_width,
                annotation.style.fill_rgba.map(rgba),
            ),
            AnnotationKind::Line { start, end } => paint_line(
                window,
                transform,
                start,
                end,
                color,
                annotation.style.stroke_width,
            ),
            AnnotationKind::Arrow { start, end } => paint_arrow(
                window,
                transform,
                start,
                end,
                color,
                annotation.style.stroke_width,
            ),
            AnnotationKind::Freehand { ref points } => paint_freehand(
                window,
                transform,
                points,
                color,
                annotation.style.stroke_width,
            ),
        }
        if Some(annotation.id) == state.selected {
            paint_outline(window, transform, annotation.bounds(), colors.success, 1);
            paint_resize_handles(window, transform, annotation.bounds(), colors.success);
        }
    }
}

fn annotation_layer_label(kind: &AnnotationKind) -> &'static str {
    match kind {
        AnnotationKind::Watermark { .. } => "Watermark",
        AnnotationKind::Text { .. } => "Text",
        AnnotationKind::Number { .. } => "Number",
        AnnotationKind::Blur { .. } => "Blur",
        AnnotationKind::Mosaic { .. } => "Mosaic",
        AnnotationKind::Highlight { .. } => "Highlight",
        AnnotationKind::Rectangle { .. } => "Rectangle",
        AnnotationKind::Ellipse { .. } => "Ellipse",
        AnnotationKind::Line { .. } => "Line",
        AnnotationKind::Arrow { .. } => "Arrow",
        AnnotationKind::Freehand { .. } => "Freehand",
    }
}

fn is_text_annotation(annotation: &Annotation) -> bool {
    matches!(
        annotation.kind,
        AnnotationKind::Text { .. } | AnnotationKind::Watermark { .. }
    )
}

fn paint_text_annotation(
    window: &mut Window,
    transform: PreviewTransform,
    origin: PhysicalPoint,
    content: &str,
    color: u32,
    font_size: u32,
    cx: &mut gpui::App,
) {
    let view_origin = transform.physical_to_view(origin);
    let glyph_width = (transform
        .physical_to_view(PhysicalPoint {
            x: origin.x.saturating_add(
                i32::try_from(font_size.saturating_mul(2).div_ceil(3)).unwrap_or(i32::MAX),
            ),
            y: origin.y,
        })
        .x
        - view_origin.x)
        .abs()
        .max(8.0);
    if content.is_empty() {
        return;
    }
    let style = window.text_style();
    let run = TextRun {
        len: content.len(),
        font: style.font(),
        color: rgba(color).into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window
        .text_system()
        .shape_line(content.into(), px(glyph_width), &[run], None);
    let _ = line.paint(
        point(px(view_origin.x), px(view_origin.y)),
        px(glyph_width * (font_size as f32 / 24.0).max(0.5)),
        TextAlign::Left,
        None,
        window,
        cx,
    );
}

fn paint_number_marker(
    window: &mut Window,
    transform: PreviewTransform,
    center: PhysicalPoint,
    value: u32,
    color: u32,
) {
    let view_center = transform.physical_to_view(center);
    let radius = (transform
        .physical_to_view(PhysicalPoint {
            x: center.x.saturating_add(SEQUENCE_MARKER_RADIUS),
            y: center.y,
        })
        .x
        - view_center.x)
        .abs();
    if radius <= 0.0 {
        return;
    }
    let mut path = gpui::PathBuilder::fill();
    const SEGMENTS: u32 = 32;
    for index in 0..=SEGMENTS {
        let angle = std::f32::consts::TAU * index as f32 / SEGMENTS as f32;
        let point = point(
            px(view_center.x + radius * angle.cos()),
            px(view_center.y + radius * angle.sin()),
        );
        if index == 0 {
            path.move_to(point);
        } else {
            path.line_to(point);
        }
    }
    path.close();
    if let Ok(path) = path.build() {
        window.paint_path(path, rgba(color));
    }
    paint_number_label(
        window,
        view_center,
        value,
        (radius / SEQUENCE_MARKER_RADIUS as f32).max(0.5),
    );
}

fn paint_number_label(window: &mut Window, center: ViewPoint, value: u32, scale: f32) {
    const DIGITS: [[u8; 5]; 10] = [
        [0b111, 0b101, 0b101, 0b101, 0b111],
        [0b010, 0b110, 0b010, 0b010, 0b111],
        [0b111, 0b001, 0b111, 0b100, 0b111],
        [0b111, 0b001, 0b111, 0b001, 0b111],
        [0b101, 0b101, 0b111, 0b001, 0b001],
        [0b111, 0b100, 0b111, 0b001, 0b111],
        [0b111, 0b100, 0b111, 0b101, 0b111],
        [0b111, 0b001, 0b010, 0b010, 0b010],
        [0b111, 0b101, 0b111, 0b101, 0b111],
        [0b111, 0b101, 0b111, 0b001, 0b111],
    ];
    let digits = value.to_string();
    let width = digits.len() as f32 * 4.0 - 1.0;
    let left = center.x - width * scale / 2.0;
    let top = center.y - 2.5 * scale;
    for (digit_index, digit) in digits.bytes().enumerate() {
        let Some(rows) = digit
            .checked_sub(b'0')
            .and_then(|index| DIGITS.get(index as usize))
        else {
            continue;
        };
        for (row, bits) in rows.iter().enumerate() {
            for column in 0..3 {
                if bits & (1 << (2 - column)) != 0 {
                    window.paint_quad(fill(
                        Bounds::new(
                            point(
                                px(left + (digit_index as f32 * 4.0 + column as f32) * scale),
                                px(top + row as f32 * scale),
                            ),
                            size(px(scale.max(1.0)), px(scale.max(1.0))),
                        ),
                        rgba(0xFFFFFFFF),
                    ));
                }
            }
        }
    }
}

fn paint_mosaic_grid(
    window: &mut Window,
    transform: PreviewTransform,
    bounds: PhysicalRect,
    color: gpui::Hsla,
) {
    const BLOCK_SIZE: i32 = 10;
    for x in (bounds.left..=bounds.right).step_by(BLOCK_SIZE as usize) {
        paint_line(
            window,
            transform,
            PhysicalPoint { x, y: bounds.top },
            PhysicalPoint {
                x,
                y: bounds.bottom,
            },
            color,
            1,
        );
    }
    for y in (bounds.top..=bounds.bottom).step_by(BLOCK_SIZE as usize) {
        paint_line(
            window,
            transform,
            PhysicalPoint { x: bounds.left, y },
            PhysicalPoint { x: bounds.right, y },
            color,
            1,
        );
    }
}

fn paint_rect_fill(
    window: &mut Window,
    transform: PreviewTransform,
    bounds: PhysicalRect,
    color: gpui::Rgba,
) {
    let start = transform.physical_to_view(PhysicalPoint {
        x: bounds.left,
        y: bounds.top,
    });
    let end = transform.physical_to_view(PhysicalPoint {
        x: bounds.right,
        y: bounds.bottom,
    });
    window.paint_quad(fill(
        Bounds::new(
            point(px(start.x), px(start.y)),
            size(px(end.x - start.x), px(end.y - start.y)),
        ),
        color,
    ));
}

fn paint_resize_handles(
    window: &mut Window,
    transform: PreviewTransform,
    bounds: PhysicalRect,
    color: gpui::Hsla,
) {
    const HANDLE_SIZE: f32 = 8.0;
    for physical_point in resize_handle_points(bounds) {
        let view_point = transform.physical_to_view(physical_point);
        window.paint_quad(fill(
            Bounds::new(
                point(
                    px(view_point.x - HANDLE_SIZE / 2.0),
                    px(view_point.y - HANDLE_SIZE / 2.0),
                ),
                size(px(HANDLE_SIZE), px(HANDLE_SIZE)),
            ),
            color,
        ));
    }
}

fn resize_handle_points(bounds: PhysicalRect) -> [PhysicalPoint; 4] {
    [
        PhysicalPoint {
            x: bounds.left,
            y: bounds.top,
        },
        PhysicalPoint {
            x: bounds.right,
            y: bounds.top,
        },
        PhysicalPoint {
            x: bounds.left,
            y: bounds.bottom,
        },
        PhysicalPoint {
            x: bounds.right,
            y: bounds.bottom,
        },
    ]
}

#[cfg(test)]
fn outline_shape_bounds(annotation: &Annotation) -> Option<PhysicalRect> {
    match annotation.kind {
        AnnotationKind::Blur { bounds }
        | AnnotationKind::Mosaic { bounds }
        | AnnotationKind::Highlight { bounds }
        | AnnotationKind::Rectangle { bounds }
        | AnnotationKind::Ellipse { bounds } => Some(bounds),
        _ => None,
    }
}

fn paint_outline(
    window: &mut Window,
    transform: PreviewTransform,
    rect: PhysicalRect,
    color: gpui::Hsla,
    stroke_width: u32,
) {
    let start = transform.physical_to_view(PhysicalPoint {
        x: rect.left,
        y: rect.top,
    });
    let end = transform.physical_to_view(PhysicalPoint {
        x: rect.right,
        y: rect.bottom,
    });
    let mut path = gpui::PathBuilder::stroke(px(stroke_width.max(1) as f32));
    path.move_to(point(px(start.x), px(start.y)));
    path.line_to(point(px(end.x), px(start.y)));
    path.line_to(point(px(end.x), px(end.y)));
    path.line_to(point(px(start.x), px(end.y)));
    path.close();
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

fn paint_line(
    window: &mut Window,
    transform: PreviewTransform,
    start: PhysicalPoint,
    end: PhysicalPoint,
    color: gpui::Hsla,
    stroke_width: u32,
) {
    let start = transform.physical_to_view(start);
    let end = transform.physical_to_view(end);
    let mut path = gpui::PathBuilder::stroke(px(stroke_width.max(1) as f32));
    path.move_to(point(px(start.x), px(start.y)));
    path.line_to(point(px(end.x), px(end.y)));
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

fn paint_arrow(
    window: &mut Window,
    transform: PreviewTransform,
    start: PhysicalPoint,
    end: PhysicalPoint,
    color: gpui::Hsla,
    stroke_width: u32,
) {
    paint_line(window, transform, start, end, color, stroke_width);
    let arrow_head_size = stroke_width.div_ceil(2).max(3) as f32 * 4.0;
    let (left, right) = arrow_head_points(start, end, arrow_head_size, 0.55);
    for point in [left, right].into_iter().flatten() {
        paint_line(window, transform, end, point, color, stroke_width);
    }
}

fn paint_freehand(
    window: &mut Window,
    transform: PreviewTransform,
    points: &[PhysicalPoint],
    color: gpui::Hsla,
    stroke_width: u32,
) {
    for segment in points.windows(2) {
        paint_line(
            window,
            transform,
            segment[0],
            segment[1],
            color,
            stroke_width,
        );
    }
}

fn arrow_head_points(
    start: PhysicalPoint,
    end: PhysicalPoint,
    size: f32,
    angle: f32,
) -> (Option<PhysicalPoint>, Option<PhysicalPoint>) {
    let dx = (end.x - start.x) as f32;
    let dy = (end.y - start.y) as f32;
    let length = dx.hypot(dy);
    if length == 0.0 {
        return (None, None);
    }
    let unit_x = dx / length;
    let unit_y = dy / length;
    let point_for = |angle: f32| PhysicalPoint {
        x: (end.x as f32 + (-unit_x * angle.cos() - unit_y * angle.sin()) * size).round() as i32,
        y: (end.y as f32 + (-unit_x * angle.sin() + unit_y * angle.cos()) * size).round() as i32,
    };
    (Some(point_for(angle)), Some(point_for(-angle)))
}

fn paint_ellipse_outline(
    window: &mut Window,
    transform: PreviewTransform,
    rect: PhysicalRect,
    color: gpui::Hsla,
    stroke_width: u32,
    fill_color: Option<gpui::Rgba>,
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
    if let Some(fill_color) = fill_color {
        let mut path = gpui::PathBuilder::fill();
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
            window.paint_path(path, fill_color);
        }
    }
    let mut path = gpui::PathBuilder::stroke(px(stroke_width.max(1) as f32));
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

fn visible_selection(
    drag: SelectionDrag,
    committed_selection: Option<PhysicalRect>,
) -> Option<PhysicalRect> {
    // The local drag retains the editing bounds after mouse-up. Prefer it so
    // switching to an annotation tool cannot make the selection frame vanish.
    drag.selection()
        .filter(|selection| selection.width() > 0 && selection.height() > 0)
        .or(committed_selection)
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ActionToolbarLayout {
    left: f32,
    top: f32,
    width: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SelectionDimensionLayout {
    left: f32,
    top: f32,
}

/// Positions the pixel-size readout near the selection without covering export controls.
fn selection_dimension_label_layout(
    selection: Option<PhysicalRect>,
    transform: Option<PreviewTransform>,
    viewport: Bounds<Pixels>,
    action_toolbar: Option<ActionToolbarLayout>,
) -> Option<SelectionDimensionLayout> {
    let selection = selection?;
    let transform = transform?;
    let viewport = view_rect(viewport);
    let top_left = transform.physical_to_view(PhysicalPoint {
        x: selection.left,
        y: selection.top,
    });
    let bottom_right = transform.physical_to_view(PhysicalPoint {
        x: selection.right,
        y: selection.bottom,
    });
    let left_min = viewport.left + OVERLAY_EDGE_INSET;
    let left_max =
        (viewport.right() - OVERLAY_EDGE_INSET - OVERLAY_DIMENSION_LABEL_WIDTH).max(left_min);
    let left = top_left.x.clamp(left_min, left_max);
    let above = top_left.y - OVERLAY_DIMENSION_LABEL_HEIGHT - OVERLAY_DIMENSION_LABEL_GAP;
    let below = bottom_right.y + OVERLAY_DIMENSION_LABEL_GAP;
    let can_place_above = above >= viewport.top + OVERLAY_EDGE_INSET;
    let overlaps_toolbar_below = action_toolbar.is_some_and(|toolbar| {
        below < toolbar.top + OVERLAY_ACTION_ITEM_HEIGHT
            && below + OVERLAY_DIMENSION_LABEL_HEIGHT > toolbar.top
    });
    let top = if can_place_above || overlaps_toolbar_below {
        above.max(viewport.top + OVERLAY_EDGE_INSET)
    } else {
        below.min(viewport.bottom() - OVERLAY_BOTTOM_SAFE_INSET - OVERLAY_DIMENSION_LABEL_HEIGHT)
    };
    Some(SelectionDimensionLayout { left, top })
}

fn action_toolbar_layout(
    selection: Option<PhysicalRect>,
    transform: Option<PreviewTransform>,
    viewport: Bounds<Pixels>,
    show_more_actions: bool,
    has_recognition_result: bool,
) -> Option<ActionToolbarLayout> {
    let selection = selection?;
    let transform = transform?;
    let viewport = view_rect(viewport);
    let width = (viewport.width - OVERLAY_EDGE_INSET * 2.0).clamp(1.0, OVERLAY_ACTION_BAR_WIDTH);
    let height = action_toolbar_height(width, show_more_actions, has_recognition_result);
    let selection_top = transform
        .physical_to_view(PhysicalPoint {
            x: selection.left,
            y: selection.top,
        })
        .y;
    let selection_bottom = transform
        .physical_to_view(PhysicalPoint {
            x: selection.right,
            y: selection.bottom,
        })
        .y;
    let selection_right = transform
        .physical_to_view(PhysicalPoint {
            x: selection.right,
            y: selection.bottom,
        })
        .x;
    let left_min = viewport.left + OVERLAY_EDGE_INSET;
    let left_limit = (viewport.right() - OVERLAY_EDGE_INSET - width).max(left_min);
    let left = (selection_right - width).clamp(left_min, left_limit);
    let lowest_top = (viewport.bottom() - OVERLAY_BOTTOM_SAFE_INSET - height)
        .max(viewport.top + OVERLAY_EDGE_INSET);
    let below = selection_bottom + OVERLAY_ACTION_BAR_GAP;
    let above = selection_top - height - OVERLAY_ACTION_BAR_GAP;
    let top = if below <= lowest_top {
        below
    } else {
        above.max(viewport.top + OVERLAY_EDGE_INSET).min(lowest_top)
    };
    Some(ActionToolbarLayout { left, top, width })
}

fn action_toolbar_height(width: f32, show_more_actions: bool, has_recognition_result: bool) -> f32 {
    let mut rows = 1_u32;
    let mut row_width = 0.0;
    let content_width = (width - OVERLAY_ACTION_BAR_PADDING * 2.0).max(1.0);
    let more_widths = show_more_actions
        .then_some(OVERLAY_MORE_ACTION_WIDTHS)
        .into_iter()
        .flatten();
    let recognition_widths = (show_more_actions && has_recognition_result)
        .then_some(OVERLAY_RECOGNITION_ACTION_WIDTHS)
        .into_iter()
        .flatten();
    for item_width in OVERLAY_PRIMARY_ACTION_WIDTHS
        .into_iter()
        .chain(more_widths)
        .chain(recognition_widths)
    {
        let next_width = if row_width == 0.0 {
            item_width
        } else {
            row_width + OVERLAY_ACTION_ITEM_GAP + item_width
        };
        if row_width > 0.0 && next_width > content_width {
            rows = rows.saturating_add(1);
            row_width = item_width;
        } else {
            row_width = next_width;
        }
    }
    rows as f32 * OVERLAY_ACTION_ITEM_HEIGHT
        + rows.saturating_sub(1) as f32 * OVERLAY_ACTION_ITEM_GAP
        + OVERLAY_ACTION_BAR_PADDING * 2.0
}

/// Chooses pointer feedback without letting selection movement override an active drawing tool.
fn selection_cursor(
    selection: Option<PhysicalRect>,
    transform: Option<PreviewTransform>,
    pointer: ViewPoint,
    annotation_tool_active: bool,
    moving: bool,
) -> SelectionCursor {
    if annotation_tool_active {
        return SelectionCursor::Crosshair;
    }
    if moving {
        return SelectionCursor::Move;
    }
    let Some((selection, transform)) = selection.zip(transform) else {
        return SelectionCursor::Crosshair;
    };
    if let Some(handle) = transform.resize_handle_at(selection, pointer, 10.0) {
        return match handle {
            crate::domain::selection::ResizeHandle::TopLeft
            | crate::domain::selection::ResizeHandle::BottomRight => SelectionCursor::ResizeNwse,
            crate::domain::selection::ResizeHandle::TopRight
            | crate::domain::selection::ResizeHandle::BottomLeft => SelectionCursor::ResizeNesw,
        };
    }
    if transform
        .view_to_physical(pointer)
        .is_some_and(|point| selection.contains(point))
    {
        SelectionCursor::Move
    } else {
        SelectionCursor::Crosshair
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ActionToolbarLayout, MAGNIFIER_CELL_SIZE, MAGNIFIER_RADIUS, SelectionCursor,
        SelectionDimensionLayout, action_toolbar_height, action_toolbar_layout,
        annotation_layer_label, arrow_head_points, intersect, is_text_annotation, magnifier_origin,
        outline_shape_bounds, resize_handle_points, selection_cursor,
        selection_dimension_label_layout, visible_selection,
    };
    use crate::domain::{
        annotation::{Annotation, AnnotationId, AnnotationKind, AnnotationStyle},
        geometry::{PhysicalPoint, PhysicalRect},
        selection::{PreviewTransform, SelectionDrag, ViewPoint},
    };
    use gpui::{Bounds, point, px, size};

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
    fn magnifier_stays_inside_the_overlay_at_viewport_edges() {
        let viewport = Bounds::new(point(px(0.0), px(0.0)), size(px(300.0), px(200.0)));
        let grid_size = (MAGNIFIER_RADIUS * 2 + 1) as f32 * MAGNIFIER_CELL_SIZE;

        assert_eq!(
            magnifier_origin(ViewPoint { x: 295.0, y: 195.0 }, viewport, grid_size),
            ViewPoint { x: 188.0, y: 88.0 }
        );
        assert_eq!(
            magnifier_origin(ViewPoint { x: 0.0, y: 0.0 }, viewport, grid_size),
            ViewPoint { x: 18.0, y: 18.0 }
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

    #[test]
    fn annotation_layer_labels_cover_every_drawable_kind() {
        assert_eq!(
            annotation_layer_label(&AnnotationKind::Text {
                origin: PhysicalPoint { x: 0, y: 0 },
                content: "Note".to_owned(),
            }),
            "Text"
        );
        assert_eq!(
            annotation_layer_label(&AnnotationKind::Freehand {
                points: vec![PhysicalPoint { x: 0, y: 0 }, PhysicalPoint { x: 1, y: 1 }],
            }),
            "Freehand"
        );
    }

    #[test]
    fn text_annotation_helper_excludes_non_text_annotations() {
        let text = Annotation {
            id: AnnotationId::new(1),
            kind: AnnotationKind::Text {
                origin: PhysicalPoint { x: 0, y: 0 },
                content: "Note".to_owned(),
            },
            style: AnnotationStyle::default(),
        };
        let line = Annotation {
            id: AnnotationId::new(2),
            kind: AnnotationKind::Line {
                start: PhysicalPoint { x: 0, y: 0 },
                end: PhysicalPoint { x: 1, y: 1 },
            },
            style: AnnotationStyle::default(),
        };

        assert!(is_text_annotation(&text));
        assert!(!is_text_annotation(&line));
    }

    #[test]
    fn arrow_head_uses_two_symmetric_wings_and_skips_zero_length_arrows() {
        let start = PhysicalPoint { x: 10, y: 20 };
        let end = PhysicalPoint { x: 30, y: 20 };
        let (left, right) = arrow_head_points(start, end, 12.0, 0.55);

        assert_eq!(left, Some(PhysicalPoint { x: 20, y: 14 }));
        assert_eq!(right, Some(PhysicalPoint { x: 20, y: 26 }));
        assert_eq!(arrow_head_points(end, end, 12.0, 0.55), (None, None));
    }

    #[test]
    fn retained_selection_stays_visible_after_the_drag_finishes() {
        let committed = PhysicalRect {
            left: 10,
            top: 20,
            right: 110,
            bottom: 120,
        };
        let mut drag = SelectionDrag::default();
        drag.select(committed);

        assert!(!drag.is_dragging());
        assert_eq!(visible_selection(drag, Some(committed)), Some(committed));
    }

    #[test]
    fn retained_selection_stays_visible_while_an_annotation_is_active() {
        let committed = PhysicalRect {
            left: 10,
            top: 20,
            right: 110,
            bottom: 120,
        };
        let retained = PhysicalRect {
            left: 200,
            top: 300,
            right: 400,
            bottom: 500,
        };
        let mut drag = SelectionDrag::default();
        drag.select(retained);

        assert!(!drag.is_dragging());
        assert_eq!(visible_selection(drag, Some(committed)), Some(retained));
    }

    #[test]
    fn active_drag_overrides_the_previous_committed_selection() {
        let committed = PhysicalRect {
            left: 10,
            top: 20,
            right: 110,
            bottom: 120,
        };
        let current = PhysicalRect {
            left: 200,
            top: 300,
            right: 400,
            bottom: 500,
        };
        let mut drag = SelectionDrag::default();
        drag.begin(PhysicalPoint {
            x: current.left,
            y: current.top,
        });
        drag.update(PhysicalPoint {
            x: current.right,
            y: current.bottom,
        });

        assert!(drag.is_dragging());
        assert_eq!(visible_selection(drag, Some(committed)), Some(current));
    }

    #[test]
    fn committed_selection_uses_move_cursor_inside_and_resize_cursor_at_corners() {
        let image = PhysicalRect {
            left: 0,
            top: 0,
            right: 1000,
            bottom: 500,
        };
        let selection = PhysicalRect {
            left: 200,
            top: 100,
            right: 800,
            bottom: 400,
        };
        let transform = PreviewTransform::contain(
            image,
            crate::domain::selection::ViewRect {
                left: 0.0,
                top: 0.0,
                width: 1000.0,
                height: 500.0,
            },
        )
        .unwrap();

        assert_eq!(
            selection_cursor(
                Some(selection),
                Some(transform),
                ViewPoint { x: 500.0, y: 250.0 },
                false,
                false,
            ),
            SelectionCursor::Move
        );
        assert_eq!(
            selection_cursor(
                Some(selection),
                Some(transform),
                ViewPoint { x: 200.0, y: 100.0 },
                false,
                false,
            ),
            SelectionCursor::ResizeNwse
        );
        assert_eq!(
            selection_cursor(
                Some(selection),
                Some(transform),
                ViewPoint { x: 100.0, y: 50.0 },
                false,
                false,
            ),
            SelectionCursor::Crosshair
        );
        assert_eq!(
            selection_cursor(
                Some(selection),
                Some(transform),
                ViewPoint { x: 500.0, y: 250.0 },
                true,
                false,
            ),
            SelectionCursor::Crosshair
        );
    }

    #[test]
    fn zero_sized_drag_keeps_the_committed_selection_visible() {
        let committed = PhysicalRect {
            left: 10,
            top: 20,
            right: 110,
            bottom: 120,
        };
        let mut drag = SelectionDrag::default();
        drag.begin(PhysicalPoint { x: 300, y: 400 });

        assert!(drag.is_dragging());
        assert_eq!(visible_selection(drag, Some(committed)), Some(committed));
    }

    #[test]
    fn compact_action_toolbar_stays_near_the_selection_and_above_the_taskbar_safe_area() {
        let viewport = Bounds::new(point(px(0.0), px(0.0)), size(px(1280.0), px(720.0)));
        let transform = PreviewTransform::contain(
            PhysicalRect {
                left: 0,
                top: 0,
                right: 1280,
                bottom: 720,
            },
            super::view_rect(viewport),
        );
        let selection = PhysicalRect {
            left: 900,
            top: 580,
            right: 1200,
            bottom: 700,
        };

        assert_eq!(
            action_toolbar_layout(Some(selection), transform, viewport, false, false),
            Some(ActionToolbarLayout {
                left: 580.0,
                top: 526.0,
                width: 620.0,
            })
        );
    }

    #[test]
    fn selection_dimensions_stay_visible_and_avoid_a_toolbar_below_the_selection() {
        let viewport = Bounds::new(point(px(0.0), px(0.0)), size(px(1280.0), px(720.0)));
        let transform = PreviewTransform::contain(
            PhysicalRect {
                left: 0,
                top: 0,
                right: 1280,
                bottom: 720,
            },
            super::view_rect(viewport),
        );
        let selection = PhysicalRect {
            left: 100,
            top: 300,
            right: 600,
            bottom: 500,
        };
        let toolbar = ActionToolbarLayout {
            left: 18.0,
            top: 508.0,
            width: 620.0,
        };

        assert_eq!(
            selection_dimension_label_layout(Some(selection), transform, viewport, Some(toolbar)),
            Some(SelectionDimensionLayout {
                left: 100.0,
                top: 266.0,
            })
        );
    }

    #[test]
    fn expanded_action_toolbar_narrows_and_accounts_for_extra_rows_on_small_overlays() {
        let viewport = Bounds::new(point(px(0.0), px(0.0)), size(px(360.0), px(720.0)));
        let transform = PreviewTransform::contain(
            PhysicalRect {
                left: 0,
                top: 0,
                right: 360,
                bottom: 720,
            },
            super::view_rect(viewport),
        );
        let selection = PhysicalRect {
            left: 20,
            top: 400,
            right: 340,
            bottom: 600,
        };

        assert_eq!(action_toolbar_height(324.0, false, false), 80.0);
        assert_eq!(action_toolbar_height(324.0, true, false), 232.0);
        assert_eq!(action_toolbar_height(324.0, true, true), 232.0);
        let layout =
            action_toolbar_layout(Some(selection), transform, viewport, true, false).unwrap();
        assert_eq!(layout.left, 18.0);
        assert!((layout.top - 156.0).abs() < 0.01);
        assert_eq!(layout.width, 324.0);
    }

    #[test]
    fn selection_resize_handles_cover_all_four_corners() {
        let selection = PhysicalRect {
            left: -400,
            top: 50,
            right: 800,
            bottom: 600,
        };

        assert_eq!(
            resize_handle_points(selection),
            [
                PhysicalPoint { x: -400, y: 50 },
                PhysicalPoint { x: 800, y: 50 },
                PhysicalPoint { x: -400, y: 600 },
                PhysicalPoint { x: 800, y: 600 },
            ]
        );
    }
}
