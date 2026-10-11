//! Focus ownership and cursor feedback for the capture overlay.

use super::*;

impl Focusable for CaptureOverlay {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// Chooses pointer feedback without letting selection movement override an active drawing tool.
pub(super) fn selection_cursor(
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
