//! Painting and annotation presentation helpers for the capture overlay.

use super::*;

pub(super) fn paint_selection_mask(
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

pub(super) fn paint_magnifier(
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

pub(super) fn magnifier_origin(
    view_center: ViewPoint,
    viewport: Bounds<Pixels>,
    grid_size: f32,
) -> ViewPoint {
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
pub(super) struct AnnotationPaintState {
    pub(super) selected: Option<AnnotationId>,
    pub(super) hidden: Option<AnnotationId>,
}

pub(super) fn paint_annotations(
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
                paint_outline(window, transform, bounds, color, annotation.stroke_width())
            }
            AnnotationKind::Ellipse { bounds } => paint_ellipse_outline(
                window,
                transform,
                bounds,
                color,
                annotation.stroke_width(),
                annotation.style.fill_rgba.map(rgba),
            ),
            AnnotationKind::Line { start, end } => paint_line(
                window,
                transform,
                start,
                end,
                color,
                annotation.stroke_width(),
            ),
            AnnotationKind::Arrow { start, end } => paint_arrow(
                window,
                transform,
                start,
                end,
                color,
                annotation.stroke_width(),
            ),
            AnnotationKind::Freehand { ref points } => {
                paint_freehand(window, transform, points, color, annotation.stroke_width())
            }
        }
        if Some(annotation.id) == state.selected {
            paint_outline(window, transform, annotation.bounds(), colors.success, 1);
            paint_resize_handles(window, transform, annotation.bounds(), colors.success);
        }
    }
}

pub(super) fn annotation_layer_label(locale: Locale, kind: &AnnotationKind) -> &'static str {
    match kind {
        AnnotationKind::Watermark { .. } => locale.text(UiText::OverlayWatermark),
        AnnotationKind::Text { .. } => locale.text(UiText::OverlayText),
        AnnotationKind::Number { .. } => locale.text(UiText::OverlayNumber),
        AnnotationKind::Blur { .. } => locale.text(UiText::OverlayBlur),
        AnnotationKind::Mosaic { .. } => locale.text(UiText::OverlayMosaic),
        AnnotationKind::Highlight { .. } => locale.text(UiText::OverlayHighlight),
        AnnotationKind::Rectangle { .. } => locale.text(UiText::OverlayRectangle),
        AnnotationKind::Ellipse { .. } => locale.text(UiText::OverlayEllipse),
        AnnotationKind::Line { .. } => locale.text(UiText::OverlayLine),
        AnnotationKind::Arrow { .. } => locale.text(UiText::OverlayArrow),
        AnnotationKind::Freehand { .. } => locale.text(UiText::OverlayFreehand),
    }
}

/// Formats one visible layer row through the active catalog while preserving its stable number.
pub(super) fn annotation_layer_entry_label(
    locale: Locale,
    position: usize,
    kind: &AnnotationKind,
) -> String {
    let position = position.to_string();
    let kind = annotation_layer_label(locale, kind);
    locale.format_template(
        UiText::OverlayLayerLabel,
        &[("position", &position), ("kind", kind)],
    )
}

/// Formats the compact opacity value through the active catalog without changing its width.
pub(super) fn annotation_opacity_value_label(locale: Locale, opacity: u8) -> String {
    let percent = (u16::from(opacity) * 100 / 255).to_string();
    locale.format_template(UiText::AnnotationOpacityValue, &[("percent", &percent)])
}

/// Formats a compact line-width value through the active catalog.
pub(super) fn annotation_width_value_label(locale: Locale, width: u32) -> String {
    let width = width.to_string();
    locale.format_template(UiText::AnnotationWidthValue, &[("width", &width)])
}

/// Formats a compact text-size value through the active catalog.
pub(super) fn annotation_text_size_value_label(locale: Locale, size: u32) -> String {
    let size = size.to_string();
    locale.format_template(UiText::AnnotationTextSizeValue, &[("size", &size)])
}

/// Formats the compact number-marker value through the active catalog.
pub(super) fn annotation_number_value_label(locale: Locale, value: u32) -> String {
    let value = value.to_string();
    locale.format_template(UiText::AnnotationNumberValue, &[("value", &value)])
}

#[cfg(test)]
pub(super) fn is_text_annotation(annotation: &Annotation) -> bool {
    matches!(
        annotation.kind,
        AnnotationKind::Text { .. } | AnnotationKind::Watermark { .. }
    )
}

pub(super) fn paint_text_annotation(
    window: &mut Window,
    transform: PreviewTransform,
    origin: PhysicalPoint,
    content: &str,
    color: u32,
    font_size: u32,
    cx: &mut gpui::App,
) {
    let view_origin = transform.physical_to_view(origin);
    let physical_font_size = i32::try_from(font_size.clamp(1, 96)).unwrap_or(96);
    let view_font_size = (transform
        .physical_to_view(PhysicalPoint {
            x: origin.x,
            y: origin.y.saturating_add(physical_font_size),
        })
        .y
        - view_origin.y)
        .abs()
        .clamp(MIN_ANNOTATION_VIEW_FONT_SIZE, MAX_ANNOTATION_VIEW_FONT_SIZE);
    let content = normalized_text_annotation_content(content);
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
        .shape_line(content.into(), px(view_font_size), &[run], None);
    let _ = line.paint(
        point(px(view_origin.x), px(view_origin.y)),
        px(view_font_size * 1.25),
        TextAlign::Left,
        None,
        window,
        cx,
    );
}

pub(super) fn paint_number_marker(
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

pub(super) fn paint_number_label(window: &mut Window, center: ViewPoint, value: u32, scale: f32) {
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

pub(super) fn paint_mosaic_grid(
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

pub(super) fn paint_rect_fill(
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

pub(super) fn paint_resize_handles(
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

pub(super) fn resize_handle_points(bounds: PhysicalRect) -> [PhysicalPoint; 4] {
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
pub(super) fn outline_shape_bounds(annotation: &Annotation) -> Option<PhysicalRect> {
    match annotation.kind {
        AnnotationKind::Blur { bounds }
        | AnnotationKind::Mosaic { bounds }
        | AnnotationKind::Highlight { bounds }
        | AnnotationKind::Rectangle { bounds }
        | AnnotationKind::Ellipse { bounds } => Some(bounds),
        _ => None,
    }
}

pub(super) fn paint_outline(
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

pub(super) fn paint_line(
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

pub(super) fn paint_arrow(
    window: &mut Window,
    transform: PreviewTransform,
    start: PhysicalPoint,
    end: PhysicalPoint,
    color: gpui::Hsla,
    stroke_width: u32,
) {
    paint_line(window, transform, start, end, color, stroke_width);
    let arrow_head_size = f64::from(stroke_width.div_ceil(2).max(3)) * 4.0;
    let (left, right) = arrow_head_points(start, end, arrow_head_size);
    for point in [left, right].into_iter().flatten() {
        paint_line(window, transform, end, point, color, stroke_width);
    }
}

pub(super) fn paint_freehand(
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

pub(super) fn paint_ellipse_outline(
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
