//! Versioned, renderer-independent annotation documents and reversible commands.

use std::fmt;

use super::geometry::{PhysicalPoint, PhysicalRect};

pub const ANNOTATION_DOCUMENT_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AnnotationId(u64);

impl AnnotationId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AnnotationStyle {
    pub stroke_rgba: u32,
    pub fill_rgba: Option<u32>,
    pub stroke_width: u32,
}

impl Default for AnnotationStyle {
    fn default() -> Self {
        Self {
            stroke_rgba: 0xFF3B30FF,
            fill_rgba: None,
            stroke_width: 4,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AnnotationKind {
    Rectangle {
        bounds: PhysicalRect,
    },
    Ellipse {
        bounds: PhysicalRect,
    },
    Line {
        start: PhysicalPoint,
        end: PhysicalPoint,
    },
    Arrow {
        start: PhysicalPoint,
        end: PhysicalPoint,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Annotation {
    pub id: AnnotationId,
    pub kind: AnnotationKind,
    pub style: AnnotationStyle,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnnotationDocument {
    version: u32,
    canvas_bounds: PhysicalRect,
    annotations: Vec<Annotation>,
}

impl AnnotationDocument {
    pub fn new(canvas_bounds: PhysicalRect) -> Result<Self, AnnotationError> {
        if canvas_bounds.width() == 0 || canvas_bounds.height() == 0 {
            return Err(AnnotationError::InvalidCanvasBounds);
        }
        Ok(Self {
            version: ANNOTATION_DOCUMENT_VERSION,
            canvas_bounds,
            annotations: Vec::new(),
        })
    }

    pub const fn version(&self) -> u32 {
        self.version
    }

    pub const fn canvas_bounds(&self) -> PhysicalRect {
        self.canvas_bounds
    }

    pub fn annotations(&self) -> &[Annotation] {
        &self.annotations
    }

    pub fn annotation(&self, id: AnnotationId) -> Option<&Annotation> {
        self.annotations
            .iter()
            .find(|annotation| annotation.id == id)
    }

    /// Returns the uppermost annotation whose visible pixels include `point`.
    ///
    /// Later annotations are painted above earlier ones, so hit testing walks
    /// the document in reverse paint order.
    pub fn annotation_at(&self, point: PhysicalPoint, tolerance: u32) -> Option<&Annotation> {
        self.annotations
            .iter()
            .rev()
            .find(|annotation| annotation.hit_test(point, tolerance))
    }

    fn insert(&mut self, annotation: Annotation) -> Result<(), AnnotationError> {
        if self.annotation(annotation.id).is_some() {
            return Err(AnnotationError::DuplicateId(annotation.id));
        }
        self.annotations.push(annotation);
        Ok(())
    }

    fn remove(&mut self, id: AnnotationId) -> Result<Annotation, AnnotationError> {
        let index = self
            .annotations
            .iter()
            .position(|annotation| annotation.id == id)
            .ok_or(AnnotationError::MissingId(id))?;
        Ok(self.annotations.remove(index))
    }

    fn replace(&mut self, annotation: Annotation) -> Result<Annotation, AnnotationError> {
        let existing = self
            .annotations
            .iter_mut()
            .find(|existing| existing.id == annotation.id)
            .ok_or(AnnotationError::MissingId(annotation.id))?;
        Ok(std::mem::replace(existing, annotation))
    }
}

impl Annotation {
    /// Tests a physical image coordinate against this annotation's visible geometry.
    pub fn hit_test(&self, point: PhysicalPoint, tolerance: u32) -> bool {
        let threshold = self
            .style
            .stroke_width
            .saturating_add(tolerance.saturating_mul(2));
        match self.kind {
            AnnotationKind::Rectangle { bounds } => {
                if bounds.width() == 0 || bounds.height() == 0 {
                    return false;
                }
                if self.style.fill_rgba.is_some() && bounds.contains(point) {
                    return true;
                }
                rect_edge_distance(point, bounds) <= threshold
            }
            AnnotationKind::Ellipse { bounds } => {
                ellipse_hit_test(point, bounds, self.style.fill_rgba.is_some(), threshold)
            }
            AnnotationKind::Line { start, end } | AnnotationKind::Arrow { start, end } => {
                segment_distance_squared(point, start, end) <= u64::from(threshold).pow(2)
            }
        }
    }
}

/// The drawable tools whose pointer gestures create a single annotation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnnotationTool {
    Rectangle,
    Ellipse,
    Line,
    Arrow,
}

/// In-progress pointer gesture. A draft is intentionally absent from the
/// document until it is committed, keeping pointer movement out of undo history.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnnotationDraft {
    id: AnnotationId,
    tool: AnnotationTool,
    style: AnnotationStyle,
    start: PhysicalPoint,
    current: PhysicalPoint,
}

impl AnnotationDraft {
    pub fn begin(
        id: AnnotationId,
        tool: AnnotationTool,
        style: AnnotationStyle,
        start: PhysicalPoint,
    ) -> Self {
        Self {
            id,
            tool,
            style,
            start,
            current: start,
        }
    }

    pub const fn start(&self) -> PhysicalPoint {
        self.start
    }

    pub const fn current(&self) -> PhysicalPoint {
        self.current
    }

    pub fn update(&mut self, point: PhysicalPoint) {
        self.current = point;
    }

    pub fn preview(&self) -> Option<Annotation> {
        (self.start != self.current).then(|| Annotation {
            id: self.id,
            kind: match self.tool {
                AnnotationTool::Rectangle => AnnotationKind::Rectangle {
                    bounds: PhysicalRect::new(self.start, self.current),
                },
                AnnotationTool::Ellipse => AnnotationKind::Ellipse {
                    bounds: PhysicalRect::new(self.start, self.current),
                },
                AnnotationTool::Line => AnnotationKind::Line {
                    start: self.start,
                    end: self.current,
                },
                AnnotationTool::Arrow => AnnotationKind::Arrow {
                    start: self.start,
                    end: self.current,
                },
            },
            style: self.style,
        })
    }
}

/// Domain controller for creating annotations through pointer gestures.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AnnotationEditor {
    draft: Option<AnnotationDraft>,
}

impl AnnotationEditor {
    pub fn draft(&self) -> Option<&AnnotationDraft> {
        self.draft.as_ref()
    }

    pub fn begin(
        &mut self,
        document: &AnnotationDocument,
        id: AnnotationId,
        tool: AnnotationTool,
        style: AnnotationStyle,
        start: PhysicalPoint,
    ) -> Result<(), AnnotationError> {
        if self.draft.is_some() {
            return Err(AnnotationError::DraftInProgress);
        }
        if document.annotation(id).is_some() {
            return Err(AnnotationError::DuplicateId(id));
        }
        self.draft = Some(AnnotationDraft::begin(
            id,
            tool,
            style,
            clamp_to_canvas(document.canvas_bounds(), start),
        ));
        Ok(())
    }

    pub fn update(&mut self, document: &AnnotationDocument, point: PhysicalPoint) -> bool {
        let Some(draft) = &mut self.draft else {
            return false;
        };
        draft.update(clamp_to_canvas(document.canvas_bounds(), point));
        true
    }

    pub fn cancel(&mut self) -> bool {
        self.draft.take().is_some()
    }

    pub fn commit(
        &mut self,
        document: &mut AnnotationDocument,
        history: &mut CommandHistory,
    ) -> Result<bool, AnnotationError> {
        let Some(draft) = self.draft.take() else {
            return Ok(false);
        };
        let Some(annotation) = draft.preview() else {
            return Ok(false);
        };
        history.apply(document, AnnotationCommand::Insert(annotation))?;
        Ok(true)
    }
}

fn clamp_to_canvas(bounds: PhysicalRect, point: PhysicalPoint) -> PhysicalPoint {
    PhysicalPoint {
        x: point.x.clamp(bounds.left, bounds.right),
        y: point.y.clamp(bounds.top, bounds.bottom),
    }
}

fn rect_edge_distance(point: PhysicalPoint, bounds: PhysicalRect) -> u32 {
    let horizontal = if point.x < bounds.left {
        bounds.left.saturating_sub(point.x)
    } else if point.x > bounds.right {
        point.x.saturating_sub(bounds.right)
    } else {
        0
    };
    let vertical = if point.y < bounds.top {
        bounds.top.saturating_sub(point.y)
    } else if point.y > bounds.bottom {
        point.y.saturating_sub(bounds.bottom)
    } else {
        0
    };
    if horizontal > 0 || vertical > 0 {
        horizontal.max(vertical) as u32
    } else {
        let left = point.x.saturating_sub(bounds.left);
        let right = bounds.right.saturating_sub(point.x);
        let top = point.y.saturating_sub(bounds.top);
        let bottom = bounds.bottom.saturating_sub(point.y);
        left.min(right).min(top).min(bottom) as u32
    }
}

fn ellipse_hit_test(
    point: PhysicalPoint,
    bounds: PhysicalRect,
    filled: bool,
    threshold: u32,
) -> bool {
    if bounds.width() == 0 || bounds.height() == 0 {
        return false;
    }
    let outer = expand_rect(bounds, threshold as i32);
    if !inside_ellipse(point, outer) {
        return false;
    }
    if filled && inside_ellipse(point, bounds) {
        return true;
    }
    let inner = inset_rect(bounds, threshold as i32);
    inner.width() == 0 || inner.height() == 0 || !inside_ellipse(point, inner)
}

fn expand_rect(bounds: PhysicalRect, amount: i32) -> PhysicalRect {
    PhysicalRect {
        left: bounds.left.saturating_sub(amount),
        top: bounds.top.saturating_sub(amount),
        right: bounds.right.saturating_add(amount),
        bottom: bounds.bottom.saturating_add(amount),
    }
}

fn inset_rect(bounds: PhysicalRect, amount: i32) -> PhysicalRect {
    PhysicalRect {
        left: bounds.left.saturating_add(amount),
        top: bounds.top.saturating_add(amount),
        right: bounds.right.saturating_sub(amount),
        bottom: bounds.bottom.saturating_sub(amount),
    }
}

fn inside_ellipse(point: PhysicalPoint, bounds: PhysicalRect) -> bool {
    let width = f64::from(bounds.width());
    let height = f64::from(bounds.height());
    if width == 0.0 || height == 0.0 {
        return false;
    }
    let center_x = (f64::from(bounds.left) + f64::from(bounds.right)) / 2.0;
    let center_y = (f64::from(bounds.top) + f64::from(bounds.bottom)) / 2.0;
    let normalized_x = (f64::from(point.x) - center_x) / (width / 2.0);
    let normalized_y = (f64::from(point.y) - center_y) / (height / 2.0);
    normalized_x.mul_add(normalized_x, normalized_y * normalized_y) <= 1.0
}

fn segment_distance_squared(point: PhysicalPoint, start: PhysicalPoint, end: PhysicalPoint) -> u64 {
    let dx = i64::from(end.x) - i64::from(start.x);
    let dy = i64::from(end.y) - i64::from(start.y);
    let length_squared = dx * dx + dy * dy;
    if length_squared == 0 {
        let point_dx = i64::from(point.x) - i64::from(start.x);
        let point_dy = i64::from(point.y) - i64::from(start.y);
        return (point_dx * point_dx + point_dy * point_dy) as u64;
    }
    let offset_x = i64::from(point.x) - i64::from(start.x);
    let offset_y = i64::from(point.y) - i64::from(start.y);
    let projection = (offset_x * dx + offset_y * dy) as f64 / length_squared as f64;
    let clamped = projection.clamp(0.0, 1.0);
    let nearest_x = f64::from(start.x) + dx as f64 * clamped;
    let nearest_y = f64::from(start.y) + dy as f64 * clamped;
    let nearest_dx = f64::from(point.x) - nearest_x;
    let nearest_dy = f64::from(point.y) - nearest_y;
    (nearest_dx.mul_add(nearest_dx, nearest_dy * nearest_dy)).round() as u64
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AnnotationCommand {
    Insert(Annotation),
    Delete(AnnotationId),
    Replace(Annotation),
}

impl AnnotationCommand {
    fn apply(self, document: &mut AnnotationDocument) -> Result<Self, AnnotationError> {
        match self {
            Self::Insert(annotation) => {
                let id = annotation.id;
                document.insert(annotation)?;
                Ok(Self::Delete(id))
            }
            Self::Delete(id) => Ok(Self::Insert(document.remove(id)?)),
            Self::Replace(annotation) => Ok(Self::Replace(document.replace(annotation)?)),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CommandHistory {
    undo: Vec<AnnotationCommand>,
    redo: Vec<AnnotationCommand>,
}

impl CommandHistory {
    pub fn apply(
        &mut self,
        document: &mut AnnotationDocument,
        command: AnnotationCommand,
    ) -> Result<(), AnnotationError> {
        let inverse = command.apply(document)?;
        self.undo.push(inverse);
        self.redo.clear();
        Ok(())
    }

    pub fn undo(&mut self, document: &mut AnnotationDocument) -> Result<bool, AnnotationError> {
        let Some(command) = self.undo.pop() else {
            return Ok(false);
        };
        let inverse = command.apply(document)?;
        self.redo.push(inverse);
        Ok(true)
    }

    pub fn redo(&mut self, document: &mut AnnotationDocument) -> Result<bool, AnnotationError> {
        let Some(command) = self.redo.pop() else {
            return Ok(false);
        };
        let inverse = command.apply(document)?;
        self.undo.push(inverse);
        Ok(true)
    }

    pub const fn undo_len(&self) -> usize {
        self.undo.len()
    }

    pub const fn redo_len(&self) -> usize {
        self.redo.len()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AnnotationError {
    InvalidCanvasBounds,
    DuplicateId(AnnotationId),
    MissingId(AnnotationId),
    DraftInProgress,
}

impl fmt::Display for AnnotationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCanvasBounds => formatter.write_str("annotation canvas must be non-empty"),
            Self::DuplicateId(id) => {
                write!(formatter, "annotation id {} already exists", id.value())
            }
            Self::MissingId(id) => write!(formatter, "annotation id {} does not exist", id.value()),
            Self::DraftInProgress => {
                formatter.write_str("an annotation gesture is already in progress")
            }
        }
    }
}

impl std::error::Error for AnnotationError {}

#[cfg(test)]
mod tests {
    use super::{
        ANNOTATION_DOCUMENT_VERSION, Annotation, AnnotationCommand, AnnotationDocument,
        AnnotationEditor, AnnotationError, AnnotationId, AnnotationKind, AnnotationStyle,
        AnnotationTool, CommandHistory,
    };
    use crate::domain::geometry::{PhysicalPoint, PhysicalRect};

    fn canvas() -> PhysicalRect {
        PhysicalRect {
            left: -1920,
            top: 0,
            right: 1920,
            bottom: 1080,
        }
    }

    fn rectangle(id: u64, bounds: PhysicalRect) -> Annotation {
        Annotation {
            id: AnnotationId::new(id),
            kind: AnnotationKind::Rectangle { bounds },
            style: AnnotationStyle::default(),
        }
    }

    #[test]
    fn document_starts_versioned_with_logical_canvas_coordinates() {
        let document = AnnotationDocument::new(canvas()).unwrap();

        assert_eq!(document.version(), ANNOTATION_DOCUMENT_VERSION);
        assert_eq!(document.canvas_bounds(), canvas());
        assert!(document.annotations().is_empty());
    }

    #[test]
    fn commands_are_reversible_and_redo_is_invalidated_by_new_work() {
        let mut document = AnnotationDocument::new(canvas()).unwrap();
        let mut history = CommandHistory::default();
        let first = rectangle(
            11,
            PhysicalRect {
                left: -100,
                top: 100,
                right: 200,
                bottom: 400,
            },
        );
        let replacement = Annotation {
            id: first.id,
            kind: AnnotationKind::Arrow {
                start: PhysicalPoint { x: -50, y: 150 },
                end: PhysicalPoint { x: 160, y: 330 },
            },
            style: AnnotationStyle {
                stroke_width: 8,
                ..AnnotationStyle::default()
            },
        };

        history
            .apply(&mut document, AnnotationCommand::Insert(first.clone()))
            .unwrap();
        history
            .apply(
                &mut document,
                AnnotationCommand::Replace(replacement.clone()),
            )
            .unwrap();
        assert_eq!(document.annotation(first.id), Some(&replacement));
        assert_eq!(history.undo_len(), 2);

        assert!(history.undo(&mut document).unwrap());
        assert_eq!(document.annotation(first.id), Some(&first));
        assert_eq!(history.redo_len(), 1);

        history
            .apply(
                &mut document,
                AnnotationCommand::Insert(rectangle(
                    12,
                    PhysicalRect {
                        left: 400,
                        top: 200,
                        right: 600,
                        bottom: 500,
                    },
                )),
            )
            .unwrap();
        assert_eq!(history.redo_len(), 0);
        assert!(!history.redo(&mut document).unwrap());
    }

    #[test]
    fn stable_ids_cannot_be_duplicated_or_deleted_twice() {
        let mut document = AnnotationDocument::new(canvas()).unwrap();
        let annotation = rectangle(
            42,
            PhysicalRect {
                left: 0,
                top: 0,
                right: 10,
                bottom: 10,
            },
        );

        let mut history = CommandHistory::default();
        history
            .apply(&mut document, AnnotationCommand::Insert(annotation.clone()))
            .unwrap();
        assert_eq!(
            history.apply(&mut document, AnnotationCommand::Insert(annotation)),
            Err(AnnotationError::DuplicateId(AnnotationId::new(42)))
        );
        assert_eq!(
            history.apply(
                &mut document,
                AnnotationCommand::Delete(AnnotationId::new(99))
            ),
            Err(AnnotationError::MissingId(AnnotationId::new(99)))
        );
    }

    #[test]
    fn empty_annotation_canvas_is_rejected() {
        assert_eq!(
            AnnotationDocument::new(PhysicalRect {
                left: 10,
                top: 10,
                right: 10,
                bottom: 20,
            }),
            Err(AnnotationError::InvalidCanvasBounds)
        );
    }

    #[test]
    fn hit_testing_prefers_the_topmost_visible_annotation() {
        let mut document = AnnotationDocument::new(canvas()).unwrap();
        let mut history = CommandHistory::default();
        let bottom = rectangle(
            1,
            PhysicalRect {
                left: 100,
                top: 100,
                right: 500,
                bottom: 500,
            },
        );
        let top = Annotation {
            id: AnnotationId::new(2),
            kind: AnnotationKind::Line {
                start: PhysicalPoint { x: 100, y: 300 },
                end: PhysicalPoint { x: 500, y: 300 },
            },
            style: AnnotationStyle {
                stroke_width: 6,
                ..AnnotationStyle::default()
            },
        };

        history
            .apply(&mut document, AnnotationCommand::Insert(bottom.clone()))
            .unwrap();
        history
            .apply(&mut document, AnnotationCommand::Insert(top.clone()))
            .unwrap();

        assert_eq!(
            document.annotation_at(PhysicalPoint { x: 300, y: 303 }, 2),
            Some(&top)
        );
        assert_eq!(
            document.annotation_at(PhysicalPoint { x: 100, y: 250 }, 2),
            Some(&bottom)
        );
        assert_eq!(
            document.annotation_at(PhysicalPoint { x: 300, y: 250 }, 2),
            None
        );
    }

    #[test]
    fn outline_shapes_ignore_their_empty_interior_but_fills_select_it() {
        let outline = Annotation {
            id: AnnotationId::new(1),
            kind: AnnotationKind::Rectangle {
                bounds: PhysicalRect {
                    left: 100,
                    top: 100,
                    right: 300,
                    bottom: 300,
                },
            },
            style: AnnotationStyle {
                stroke_width: 4,
                ..AnnotationStyle::default()
            },
        };
        let filled = Annotation {
            id: AnnotationId::new(2),
            kind: AnnotationKind::Ellipse {
                bounds: PhysicalRect {
                    left: 100,
                    top: 100,
                    right: 300,
                    bottom: 300,
                },
            },
            style: AnnotationStyle {
                fill_rgba: Some(0xFFFFFFFF),
                ..AnnotationStyle::default()
            },
        };

        assert!(outline.hit_test(PhysicalPoint { x: 102, y: 200 }, 0));
        assert!(!outline.hit_test(PhysicalPoint { x: 200, y: 200 }, 0));
        assert!(filled.hit_test(PhysicalPoint { x: 200, y: 200 }, 0));
        assert!(!filled.hit_test(PhysicalPoint { x: 100, y: 100 }, 0));
    }

    #[test]
    fn line_hit_testing_handles_diagonals_endpoints_and_degenerate_segments() {
        let line = Annotation {
            id: AnnotationId::new(3),
            kind: AnnotationKind::Line {
                start: PhysicalPoint { x: -100, y: -100 },
                end: PhysicalPoint { x: 100, y: 100 },
            },
            style: AnnotationStyle {
                stroke_width: 3,
                ..AnnotationStyle::default()
            },
        };
        let dot = Annotation {
            id: AnnotationId::new(4),
            kind: AnnotationKind::Arrow {
                start: PhysicalPoint { x: 20, y: 20 },
                end: PhysicalPoint { x: 20, y: 20 },
            },
            style: AnnotationStyle::default(),
        };

        assert!(line.hit_test(PhysicalPoint { x: 2, y: 0 }, 0));
        assert!(line.hit_test(PhysicalPoint { x: 102, y: 100 }, 0));
        assert!(!line.hit_test(PhysicalPoint { x: 10, y: 20 }, 0));
        assert!(dot.hit_test(PhysicalPoint { x: 23, y: 20 }, 0));
        assert!(!dot.hit_test(PhysicalPoint { x: 30, y: 20 }, 0));
    }

    #[test]
    fn pointer_gesture_keeps_draft_out_of_document_until_commit() {
        let mut document = AnnotationDocument::new(canvas()).unwrap();
        let mut history = CommandHistory::default();
        let mut editor = AnnotationEditor::default();

        editor
            .begin(
                &document,
                AnnotationId::new(100),
                AnnotationTool::Rectangle,
                AnnotationStyle::default(),
                PhysicalPoint { x: -100, y: 100 },
            )
            .unwrap();
        assert!(editor.update(&document, PhysicalPoint { x: 200, y: 400 }));
        assert!(document.annotations().is_empty());
        assert_eq!(history.undo_len(), 0);
        assert_eq!(
            editor.draft().unwrap().preview().unwrap().kind,
            AnnotationKind::Rectangle {
                bounds: PhysicalRect {
                    left: -100,
                    top: 100,
                    right: 200,
                    bottom: 400,
                },
            }
        );

        assert!(editor.commit(&mut document, &mut history).unwrap());
        assert!(editor.draft().is_none());
        assert_eq!(document.annotations().len(), 1);
        assert_eq!(history.undo_len(), 1);
        assert!(history.undo(&mut document).unwrap());
        assert!(document.annotations().is_empty());
    }

    #[test]
    fn cancelled_or_zero_sized_gestures_do_not_create_history_entries() {
        let mut document = AnnotationDocument::new(canvas()).unwrap();
        let mut history = CommandHistory::default();
        let mut editor = AnnotationEditor::default();

        editor
            .begin(
                &document,
                AnnotationId::new(101),
                AnnotationTool::Ellipse,
                AnnotationStyle::default(),
                PhysicalPoint { x: 10, y: 10 },
            )
            .unwrap();
        assert!(editor.cancel());
        assert!(!editor.commit(&mut document, &mut history).unwrap());

        editor
            .begin(
                &document,
                AnnotationId::new(102),
                AnnotationTool::Line,
                AnnotationStyle::default(),
                PhysicalPoint { x: 20, y: 20 },
            )
            .unwrap();
        assert!(!editor.commit(&mut document, &mut history).unwrap());
        assert!(document.annotations().is_empty());
        assert_eq!(history.undo_len(), 0);
    }

    #[test]
    fn editor_clamps_gestures_to_canvas_and_rejects_concurrent_or_duplicate_ids() {
        let mut document = AnnotationDocument::new(canvas()).unwrap();
        let mut history = CommandHistory::default();
        let mut editor = AnnotationEditor::default();
        let id = AnnotationId::new(103);

        editor
            .begin(
                &document,
                id,
                AnnotationTool::Arrow,
                AnnotationStyle::default(),
                PhysicalPoint { x: -9000, y: -9000 },
            )
            .unwrap();
        assert_eq!(
            editor.begin(
                &document,
                AnnotationId::new(104),
                AnnotationTool::Arrow,
                AnnotationStyle::default(),
                PhysicalPoint { x: 0, y: 0 },
            ),
            Err(AnnotationError::DraftInProgress)
        );
        editor.update(&document, PhysicalPoint { x: 9000, y: 9000 });
        assert!(editor.commit(&mut document, &mut history).unwrap());
        assert_eq!(
            document.annotation(id).unwrap().kind,
            AnnotationKind::Arrow {
                start: PhysicalPoint { x: -1920, y: 0 },
                end: PhysicalPoint { x: 1920, y: 1080 },
            }
        );
        assert_eq!(
            editor.begin(
                &document,
                id,
                AnnotationTool::Arrow,
                AnnotationStyle::default(),
                PhysicalPoint { x: 0, y: 0 },
            ),
            Err(AnnotationError::DuplicateId(id))
        );
    }
}
