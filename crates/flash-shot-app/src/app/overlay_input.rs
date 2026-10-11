//! Selection input buffering and toolbar gesture state for the capture overlay.

use super::*;

/// Captures the newest pointer sample that arrived before one rendered frame.
///
/// A new sample replaces the previous one while a callback is already scheduled. The generation
/// makes an old callback harmless after a new gesture or mouse-up discards its pending sample.
/// `push` returns a generation exactly once per pending frame, which is the testable scheduling
/// contract used by the real GPUI `on_mouse_move` listener below.
#[derive(Debug)]
pub(super) struct FrameInputBatch<T> {
    pub(super) latest: Option<T>,
    pub(super) generation: u64,
    pub(super) scheduled: bool,
}

impl<T> Default for FrameInputBatch<T> {
    fn default() -> Self {
        Self {
            latest: None,
            generation: 0,
            scheduled: false,
        }
    }
}

impl<T> FrameInputBatch<T> {
    /// Stores the latest input and returns a generation only when this frame needs a new callback.
    pub(super) fn push(&mut self, input: T) -> Option<u64> {
        self.latest = Some(input);
        if self.scheduled {
            None
        } else {
            self.scheduled = true;
            Some(self.generation)
        }
    }

    /// Takes the latest sample only when the callback still belongs to the active gesture.
    pub(super) fn take(&mut self, generation: u64) -> Option<T> {
        if self.scheduled && self.generation == generation {
            self.scheduled = false;
            self.latest.take()
        } else {
            None
        }
    }

    /// Cancels an outstanding frame callback before a new gesture or final mouse position wins.
    pub(super) fn invalidate(&mut self) {
        self.latest = None;
        self.scheduled = false;
        self.generation = self.generation.wrapping_add(1);
    }
}

/// Contains the pointer state that must stay internally consistent when one frame applies it.
#[derive(Clone, Copy, Debug)]
pub(super) struct PendingSelectionUpdate {
    pub(super) operation_generation: u64,
    pub(super) hover_point: Option<PhysicalPoint>,
    pub(super) dragging_point: Option<PhysicalPoint>,
    pub(super) preserve_aspect_ratio: bool,
    pub(super) resize_from_center: bool,
}

/// Retains the pointer and toolbar geometry captured at the start of a manual toolbar move.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ToolbarDragGesture {
    pub(super) pointer_start: ViewPoint,
    pub(super) toolbar_start: ActionToolbarLayout,
    pub(super) selection: PhysicalRect,
}

/// Stores a manually chosen toolbar origin until the selection changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ManualToolbarPosition {
    pub(super) selection: PhysicalRect,
    pub(super) left: f32,
    pub(super) top: f32,
}
