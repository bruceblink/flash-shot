//! Private, renderer-independent capture preparation shared by app workflows.

use std::io;

use crate::{
    domain::{annotation::AnnotationDocument, geometry::PhysicalRect},
    platform::capture::CaptureFrame,
};

/// Composites the editable document and crops one physical selection for a workflow worker.
///
/// The returned frame owns its pixels and can cross the GPUI background-task boundary. Keeping
/// this operation here prevents Copy, Save, Pin, OCR, and translation from maintaining separate
/// ordering for annotation compositing and physical-pixel cropping.
pub(super) fn prepare_selection_frame(
    frame: &CaptureFrame,
    document: &AnnotationDocument,
    selection: PhysicalRect,
) -> io::Result<CaptureFrame> {
    frame.composite_annotations(document)?.crop(selection)
}

/// Prepares a selection while preserving the last reversible cancellation boundary.
///
/// Pixel compositing and cropping are synchronous. The callback is checked before preparation
/// and again before the caller can hand the owned frame to a native sink; it cannot interrupt
/// work that is already inside the image operation.
pub(super) fn prepare_selection_frame_cancellable(
    frame: &CaptureFrame,
    document: &AnnotationDocument,
    selection: PhysicalRect,
    is_cancelled: impl Fn() -> bool,
) -> io::Result<Option<CaptureFrame>> {
    if is_cancelled() {
        return Ok(None);
    }
    let prepared = prepare_selection_frame(frame, document, selection)?;
    if is_cancelled() {
        return Ok(None);
    }
    Ok(Some(prepared))
}
