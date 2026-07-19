//! GPUI rendering for the capture workspace.

use std::{cell::Cell, rc::Rc};

use gpui::{
    BorderStyle, Bounds, MouseButton, ObjectFit, Pixels, Render, Window, canvas, div, fill, img,
    outline, point, prelude::*, px, size,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

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
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        if self.main_window_handle.is_none()
            && let Ok(handle) = window.window_handle()
            && let RawWindowHandle::Win32(handle) = handle.as_raw()
        {
            self.main_window_handle = Some(handle.hwnd.get());
        }
        let colors = self.colors;
        let is_idle = self.session.state() == CaptureSessionState::Idle;
        let is_busy = self.session.state() == CaptureSessionState::Capturing;
        let delayed_capture = self.delayed_capture_generation.is_some();
        let delayed_remaining = self.delayed_capture_remaining_seconds;
        let is_exporting = self.session.state() == CaptureSessionState::Exporting;
        let recording_active = self.recording_control.is_some();
        let recording_starting = self.recording_start_in_flight;
        let recording_paused = self.recording_paused;
        let recording_audio =
            super::workflow::recording_audio_selection_label(&self.recording_audio);
        let recording_audio_discovery = self.recording_audio_discovery_in_flight;
        let recording_display =
            super::workflow::recording_display_selection_label(&self.recording_display);
        let recording_display_discovery = self.recording_display_discovery_in_flight;
        let update_check_in_flight = self.update_check_in_flight;
        let auto_start_enabled = self.auto_start_enabled;
        let preview = self.preview.clone();
        let frame_bounds = self.frame.as_ref().map(|frame| frame.bounds);
        let frame = self.frame.clone();
        let selection = self.selection_drag.selection();
        let inspection_target = self.inspection_target;
        let can_export =
            selection.is_some() && self.session.state() == CaptureSessionState::Selecting;
        let hover_pixel = self.hover_pixel;
        let history_entries: Vec<_> = self.history.entries().iter().take(5).cloned().collect();
        let recognition_result = self.recognition_result.clone();
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
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .id("auto-start")
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(if auto_start_enabled {
                                        colors.accent
                                    } else {
                                        colors.border
                                    })
                                    .text_color(if auto_start_enabled {
                                        colors.accent
                                    } else {
                                        colors.muted
                                    })
                                    .cursor_pointer()
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.toggle_auto_start(cx)),
                                    )
                                    .child("Start with Windows"),
                            )
                            .child(
                                div()
                                    .id("check-for-updates")
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(colors.border)
                                    .text_color(colors.muted)
                                    .when(!update_check_in_flight, |button| {
                                        button.cursor_pointer().on_click(
                                            cx.listener(|this, _, _, cx| {
                                                this.check_for_updates(cx)
                                            }),
                                        )
                                    })
                                    .child(if update_check_in_flight {
                                        "Checking..."
                                    } else {
                                        "Check Updates"
                                    }),
                            )
                            .child(
                                div()
                                    .id("recording-display")
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(colors.border)
                                    .text_color(colors.muted)
                                    .when(
                                        !recording_active
                                            && !recording_starting
                                            && !recording_display_discovery,
                                        |button| {
                                            button.cursor_pointer().on_click(cx.listener(
                                                |this, _, _, cx| this.cycle_recording_display(cx),
                                            ))
                                        },
                                    )
                                    .child(if recording_display_discovery {
                                        "Display...".to_owned()
                                    } else {
                                        format!("Display: {recording_display}")
                                    }),
                            )
                            .child(
                                div()
                                    .id("recording-audio")
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(colors.border)
                                    .text_color(colors.muted)
                                    .when(
                                        !recording_active
                                            && !recording_starting
                                            && !recording_audio_discovery,
                                        |button| {
                                            button.cursor_pointer().on_click(cx.listener(
                                                |this, _, _, cx| this.cycle_recording_audio(cx),
                                            ))
                                        },
                                    )
                                    .child(if recording_audio_discovery {
                                        "Audio...".to_owned()
                                    } else {
                                        format!("Audio: {recording_audio}")
                                    }),
                            )
                            .child(
                                div()
                                    .id("pause-recording")
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(colors.border)
                                    .text_color(colors.accent)
                                    .when(recording_active && !recording_starting, |button| {
                                        button.cursor_pointer().on_click(cx.listener(
                                            |this, _, _, cx| this.toggle_recording_pause(cx),
                                        ))
                                    })
                                    .child(if recording_paused { "Resume" } else { "Pause" }),
                            )
                            .child(
                                div()
                                    .id("record-display")
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(if recording_active {
                                        colors.accent
                                    } else {
                                        colors.border
                                    })
                                    .text_color(colors.accent)
                                    .when(!recording_starting, |button| {
                                        button.cursor_pointer().on_click(cx.listener(
                                            |this, _, _, cx| this.toggle_display_recording(cx),
                                        ))
                                    })
                                    .child(if recording_starting {
                                        "Preparing..."
                                    } else if recording_active {
                                        "Stop Recording"
                                    } else {
                                        "Record Display"
                                    }),
                            )
                            .child(
                                div()
                                    .id("open-image-action")
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(colors.accent)
                                    .text_color(colors.accent)
                                    .when(is_idle, |button| {
                                        button.cursor_pointer().on_click(
                                            cx.listener(|this, _, _, cx| this.open_image(cx)),
                                        )
                                    })
                                    .child("Open PNG"),
                            )
                            .child(
                                div()
                                    .id("open-project-action")
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(colors.accent)
                                    .text_color(colors.accent)
                                    .when(is_idle, |button| {
                                        button.cursor_pointer().on_click(cx.listener(
                                            |this, _, _, cx| this.open_editable_project(cx),
                                        ))
                                    })
                                    .child("Open Project"),
                            )
                            .child(
                                div()
                                    .id("capture-cursor")
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(if self.include_cursor {
                                        colors.accent
                                    } else {
                                        colors.border
                                    })
                                    .text_color(if self.include_cursor {
                                        colors.accent
                                    } else {
                                        colors.muted
                                    })
                                    .when(is_idle && !delayed_capture, |button| {
                                        button.cursor_pointer().on_click(cx.listener(
                                            |this, _, _, cx| this.toggle_capture_cursor(cx),
                                        ))
                                    })
                                    .child("Cursor"),
                            )
                            .child(
                                div()
                                    .id("capture-delay")
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(colors.border)
                                    .text_color(colors.muted)
                                    .when(is_idle && !delayed_capture, |button| {
                                        button.cursor_pointer().on_click(cx.listener(
                                            |this, _, _, cx| this.cycle_capture_delay(cx),
                                        ))
                                    })
                                    .child(if self.capture_delay_seconds == 0 {
                                        "Delay".to_owned()
                                    } else {
                                        format!("{}s", self.capture_delay_seconds)
                                    }),
                            )
                            .child(
                                div()
                                    .id("capture-action")
                                    .px_4()
                                    .py_2()
                                    .rounded_md()
                                    .bg(if is_idle || delayed_capture {
                                        colors.accent
                                    } else {
                                        colors.border
                                    })
                                    .text_color(colors.background)
                                    .when(is_idle || delayed_capture, |button| {
                                        button.cursor_pointer().on_click(cx.listener(
                                            |this, _, _, cx| {
                                                if this.delayed_capture_generation.is_some() {
                                                    this.cancel_delayed_capture(cx);
                                                } else {
                                                    this.start_capture(cx);
                                                }
                                            },
                                        ))
                                    })
                                    .child(if let Some(remaining) = delayed_remaining {
                                        format!("Cancel ({remaining}s)")
                                    } else if is_busy {
                                        "Capturing...".to_owned()
                                    } else {
                                        "Capture".to_owned()
                                    }),
                            ),
                    ),
            )
            .when(!history_entries.is_empty(), |layout| {
                layout.child(
                    div()
                        .px_5()
                        .pb_3()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(colors.muted)
                                        .child("Recent captures"),
                                )
                                .child(
                                    div()
                                        .id("clear-history")
                                        .text_sm()
                                        .text_color(colors.accent)
                                        .cursor_pointer()
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.clear_history(cx)),
                                        )
                                        .child("Clear"),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .children(history_entries.into_iter().map(|entry| {
                                    div()
                                        .id(format!("history-{}", entry.created_at_ms))
                                        .px_2()
                                        .py_1()
                                        .border_1()
                                        .border_color(colors.border)
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            div()
                                                .id(format!("open-history-{}", entry.created_at_ms))
                                                .text_xs()
                                                .text_color(colors.muted)
                                                .when(is_idle, |item| {
                                                    let path = entry.path.clone();
                                                    item.cursor_pointer().on_click(cx.listener(
                                                        move |this, _, _, cx| {
                                                            this.open_history_image(
                                                                path.clone(),
                                                                cx,
                                                            )
                                                        },
                                                    ))
                                                })
                                                .child(
                                                    entry
                                                        .path
                                                        .file_name()
                                                        .and_then(|name| name.to_str())
                                                        .unwrap_or("Screenshot")
                                                        .to_owned(),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .id(format!(
                                                    "remove-history-{}",
                                                    entry.created_at_ms
                                                ))
                                                .text_xs()
                                                .text_color(colors.muted)
                                                .cursor_pointer()
                                                .on_click({
                                                    let path = entry.path.clone();
                                                    cx.listener(move |this, _, _, cx| {
                                                        this.remove_history_image(path.clone(), cx)
                                                    })
                                                })
                                                .child("Remove"),
                                        )
                                })),
                        ),
                )
            })
            .when_some(recognition_result, |layout, result| {
                layout.child(
                    div()
                        .mx_5()
                        .mb_3()
                        .p_3()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .border_1()
                        .border_color(colors.accent)
                        .bg(colors.panel)
                        .child(
                            div().flex().items_center().justify_between().children([
                                div().text_sm().text_color(colors.text).child(result.title),
                                div()
                                    .flex()
                                    .gap_3()
                                    .child(
                                        div()
                                            .id("copy-recognition-result")
                                            .text_sm()
                                            .text_color(colors.accent)
                                            .cursor_pointer()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.copy_recognition_result(cx)
                                            }))
                                            .child("Copy"),
                                    )
                                    .child(
                                        div()
                                            .id("clear-recognition-result")
                                            .text_sm()
                                            .text_color(colors.muted)
                                            .cursor_pointer()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.clear_recognition_result(cx)
                                            }))
                                            .child("Close"),
                                    ),
                            ]),
                        )
                        .child(div().text_sm().text_color(colors.muted).child(result.text)),
                )
            })
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
                            .child(div().text_sm().child(self.capture_shortcut.clone()))
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
                                            .id("record-window")
                                            .px_3()
                                            .py_1()
                                            .rounded_md()
                                            .cursor_pointer()
                                            .text_sm()
                                            .border_1()
                                            .border_color(colors.accent)
                                            .text_color(colors.accent)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.start_selected_window_recording(cx)
                                            }))
                                            .child("Record Window"),
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
                                .when(can_export, |actions| {
                                    actions.child(
                                        div()
                                            .id("pin-selection")
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
                                                    this.pin_selection(cx)
                                                }),
                                            )
                                            .child("Pin"),
                                    )
                                })
                                .when(can_export, |actions| {
                                    actions.child(
                                        div()
                                            .id("record-selection")
                                            .px_3()
                                            .py_1()
                                            .rounded_md()
                                            .cursor_pointer()
                                            .text_sm()
                                            .border_1()
                                            .border_color(colors.accent)
                                            .text_color(colors.accent)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.start_region_recording(cx)
                                            }))
                                            .child("Record Area"),
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
