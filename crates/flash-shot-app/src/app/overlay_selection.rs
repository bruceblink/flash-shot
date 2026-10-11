//! Selection coordinate and boundary helpers for the capture overlay.

use super::*;

pub(super) fn view_rect(bounds: Bounds<Pixels>) -> ViewRect {
    ViewRect {
        left: f32::from(bounds.origin.x),
        top: f32::from(bounds.origin.y),
        width: f32::from(bounds.size.width),
        height: f32::from(bounds.size.height),
    }
}

pub(super) fn local_viewport(window: &Window) -> Bounds<Pixels> {
    Bounds::new(point(px(0.0), px(0.0)), window.bounds().size)
}

pub(super) fn view_point(position: gpui::Point<Pixels>) -> ViewPoint {
    ViewPoint {
        x: f32::from(position.x),
        y: f32::from(position.y),
    }
}

pub(super) fn clamp_to_view(
    transform: PreviewTransform,
    position: gpui::Point<Pixels>,
) -> ViewPoint {
    let fitted = transform.fitted_view();
    ViewPoint {
        x: f32::from(position.x).clamp(fitted.left, fitted.right()),
        y: f32::from(position.y).clamp(fitted.top, fitted.bottom()),
    }
}

/// Uses global physical pixels for full-display overlays so off-screen Win32 client insets do not
/// shift selection edges; windowed image editors continue to use their local preview transform.
pub(super) fn selection_point_from_view_or_screen(
    transform: PreviewTransform,
    position: gpui::Point<Pixels>,
    allow_cross_display_drag: bool,
    screen_pointer: Option<PhysicalPoint>,
    selection_bounds: PhysicalRect,
) -> Option<PhysicalPoint> {
    let view = view_point(position);
    if allow_cross_display_drag {
        let display_bounds = transform.image_bounds();
        let local_point = transform
            .view_to_physical(clamp_to_view(transform, position))
            .map(|point| map_display_point_to_frame(point, display_bounds, selection_bounds));
        let screen_point = screen_pointer
            .map(|point| map_display_point_to_frame(point, display_bounds, selection_bounds));
        match (screen_pointer, screen_point, local_point) {
            (Some(raw_screen), Some(screen), Some(local)) => Some(PhysicalPoint {
                // Prefer the local GPUI edge sample when the native pointer is just inside a
                // borderless client inset; keep the global point for interior/cross-display drag.
                x: if raw_screen.x >= display_bounds.left
                    && raw_screen.x < display_bounds.right
                    && (local.x == selection_bounds.right
                        || raw_screen.x >= display_bounds.right.saturating_sub(8))
                {
                    local.x
                } else {
                    screen.x
                },
                y: if raw_screen.y >= display_bounds.top
                    && raw_screen.y < display_bounds.bottom
                    && (local.y == selection_bounds.bottom
                        || raw_screen.y >= display_bounds.bottom.saturating_sub(8))
                {
                    local.y
                } else {
                    screen.y
                },
            }),
            (_, Some(point), None) | (None, None, Some(point)) => Some(point),
            (None, Some(point), Some(_)) => Some(point),
            (Some(_), None, Some(point)) => Some(point),
            (Some(_), None, None) => None,
            (None, None, None) => None,
        }
        .map(|point| snap_selection_pointer_to_frame_edge(point, selection_bounds))
    } else if transform.fitted_view().contains(view) {
        transform.view_to_physical(view)
    } else {
        transform.view_to_physical(clamp_to_view(transform, position))
    }
}

/// Maps a display-space pointer into the captured frame when capture and display dimensions differ.
pub(super) fn map_display_point_to_frame(
    point: PhysicalPoint,
    display_bounds: PhysicalRect,
    frame_bounds: PhysicalRect,
) -> PhysicalPoint {
    let map_axis = |value: i32,
                    display_start: i32,
                    display_extent: u32,
                    frame_start: i32,
                    frame_extent: u32| {
        if value < display_start {
            return frame_start.saturating_add(value.saturating_sub(display_start));
        }
        let display_end = display_start.saturating_add(display_extent as i32);
        if value >= display_end {
            return frame_start
                .saturating_add(frame_extent as i32)
                .saturating_add(value.saturating_sub(display_end));
        }
        let offset = value
            .saturating_sub(display_start)
            .clamp(0, display_extent.saturating_sub(1) as i32);
        frame_start.saturating_add(
            (i64::from(offset) * i64::from(frame_extent) / i64::from(display_extent.max(1))) as i32,
        )
    };
    PhysicalPoint {
        x: map_axis(
            point.x,
            display_bounds.left,
            display_bounds.width(),
            frame_bounds.left,
            frame_bounds.width(),
        ),
        y: map_axis(
            point.y,
            display_bounds.top,
            display_bounds.height(),
            frame_bounds.top,
            frame_bounds.height(),
        ),
    }
}

/// Treats the final addressable pixel as the exclusive edge when a physical drag reaches a frame border.
pub(super) fn snap_selection_pointer_to_frame_edge(
    point: PhysicalPoint,
    bounds: PhysicalRect,
) -> PhysicalPoint {
    PhysicalPoint {
        x: if point.x == bounds.right.saturating_sub(1) {
            bounds.right
        } else {
            point.x
        },
        y: if point.y == bounds.bottom.saturating_sub(1) {
            bounds.bottom
        } else {
            point.y
        },
    }
}

pub(super) fn intersect(left: PhysicalRect, right: PhysicalRect) -> Option<PhysicalRect> {
    let result = PhysicalRect {
        left: left.left.max(right.left),
        top: left.top.max(right.top),
        right: left.right.min(right.right),
        bottom: left.bottom.min(right.bottom),
    };
    (result.width() > 0 && result.height() > 0).then_some(result)
}

pub(super) fn visible_selection(
    drag: SelectionDrag,
    committed_selection: Option<PhysicalRect>,
) -> Option<PhysicalRect> {
    // The local drag retains the editing bounds after mouse-up. Prefer it so
    // switching to an annotation tool cannot make the selection frame vanish.
    drag.selection()
        .filter(|selection| selection.width() > 0 && selection.height() > 0)
        .or(committed_selection)
}

/// Limits the fast completion gesture to the platform's second left-button click.
pub(super) fn capture_double_click(click_count: usize) -> bool {
    click_count == 2
}
