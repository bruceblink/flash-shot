//! Versioned, renderer-independent annotation documents and reversible commands.

use std::fmt;

use super::geometry::{PhysicalPoint, PhysicalRect};
use super::selection::ResizeHandle;

pub const ANNOTATION_DOCUMENT_VERSION: u32 = 1;
const MIN_FREEHAND_SAMPLE_DISTANCE: u32 = 2;

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
    Blur {
        bounds: PhysicalRect,
    },
    Mosaic {
        bounds: PhysicalRect,
    },
    Highlight {
        bounds: PhysicalRect,
    },
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
    Freehand {
        points: Vec<PhysicalPoint>,
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

    fn reorder(&mut self, id: AnnotationId, index: usize) -> Result<usize, AnnotationError> {
        let current_index = self
            .annotations
            .iter()
            .position(|annotation| annotation.id == id)
            .ok_or(AnnotationError::MissingId(id))?;
        let annotation = self.annotations.remove(current_index);
        self.annotations
            .insert(index.min(self.annotations.len()), annotation);
        Ok(current_index)
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
            AnnotationKind::Blur { bounds } => bounds.contains(point),
            AnnotationKind::Mosaic { bounds } => bounds.contains(point),
            AnnotationKind::Highlight { bounds } => bounds.contains(point),
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
            AnnotationKind::Freehand { ref points } => points.windows(2).any(|segment| {
                segment_distance_squared(point, segment[0], segment[1])
                    <= u64::from(threshold).pow(2)
            }),
        }
    }

    pub fn bounds(&self) -> PhysicalRect {
        match self.kind {
            AnnotationKind::Blur { bounds }
            | AnnotationKind::Mosaic { bounds }
            | AnnotationKind::Highlight { bounds }
            | AnnotationKind::Rectangle { bounds }
            | AnnotationKind::Ellipse { bounds } => bounds,
            AnnotationKind::Line { start, end } | AnnotationKind::Arrow { start, end } => {
                PhysicalRect::new(start, end)
            }
            AnnotationKind::Freehand { ref points } => bounds_for_points(points),
        }
    }

    fn translated(&self, delta_x: i32, delta_y: i32) -> Self {
        let translate = |point: PhysicalPoint| PhysicalPoint {
            x: point.x.saturating_add(delta_x),
            y: point.y.saturating_add(delta_y),
        };
        Self {
            id: self.id,
            kind: match self.kind {
                AnnotationKind::Blur { bounds } => AnnotationKind::Blur {
                    bounds: translate_rect(bounds, delta_x, delta_y),
                },
                AnnotationKind::Mosaic { bounds } => AnnotationKind::Mosaic {
                    bounds: translate_rect(bounds, delta_x, delta_y),
                },
                AnnotationKind::Highlight { bounds } => AnnotationKind::Highlight {
                    bounds: translate_rect(bounds, delta_x, delta_y),
                },
                AnnotationKind::Rectangle { bounds } => AnnotationKind::Rectangle {
                    bounds: translate_rect(bounds, delta_x, delta_y),
                },
                AnnotationKind::Ellipse { bounds } => AnnotationKind::Ellipse {
                    bounds: translate_rect(bounds, delta_x, delta_y),
                },
                AnnotationKind::Line { start, end } => AnnotationKind::Line {
                    start: translate(start),
                    end: translate(end),
                },
                AnnotationKind::Arrow { start, end } => AnnotationKind::Arrow {
                    start: translate(start),
                    end: translate(end),
                },
                AnnotationKind::Freehand { ref points } => AnnotationKind::Freehand {
                    points: points.iter().copied().map(translate).collect(),
                },
            },
            style: self.style,
        }
    }

    fn resized(&self, bounds: PhysicalRect) -> Self {
        let source = self.bounds();
        let scale_point = |point: PhysicalPoint| PhysicalPoint {
            x: scale_coordinate(
                point.x,
                source.left,
                source.right,
                bounds.left,
                bounds.right,
            ),
            y: scale_coordinate(
                point.y,
                source.top,
                source.bottom,
                bounds.top,
                bounds.bottom,
            ),
        };
        Self {
            id: self.id,
            kind: match self.kind {
                AnnotationKind::Blur { .. } => AnnotationKind::Blur { bounds },
                AnnotationKind::Mosaic { .. } => AnnotationKind::Mosaic { bounds },
                AnnotationKind::Highlight { .. } => AnnotationKind::Highlight { bounds },
                AnnotationKind::Rectangle { .. } => AnnotationKind::Rectangle { bounds },
                AnnotationKind::Ellipse { .. } => AnnotationKind::Ellipse { bounds },
                AnnotationKind::Line { start, end } => AnnotationKind::Line {
                    start: scale_point(start),
                    end: scale_point(end),
                },
                AnnotationKind::Arrow { start, end } => AnnotationKind::Arrow {
                    start: scale_point(start),
                    end: scale_point(end),
                },
                AnnotationKind::Freehand { ref points } => AnnotationKind::Freehand {
                    points: points.iter().copied().map(scale_point).collect(),
                },
            },
            style: self.style,
        }
    }
}

/// The drawable tools whose pointer gestures create a single annotation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnnotationTool {
    Blur,
    Mosaic,
    Highlight,
    Rectangle,
    Ellipse,
    Line,
    Arrow,
    Freehand,
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
    points: Vec<PhysicalPoint>,
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
            points: vec![start],
        }
    }

    pub const fn start(&self) -> PhysicalPoint {
        self.start
    }

    pub const fn current(&self) -> PhysicalPoint {
        self.current
    }

    pub fn update(&mut self, point: PhysicalPoint) {
        if self.tool == AnnotationTool::Freehand {
            if let Some(last_sample) = self.points.last().copied() {
                if squared_distance(last_sample, point)
                    >= u64::from(MIN_FREEHAND_SAMPLE_DISTANCE).pow(2)
                {
                    self.points.push(point);
                }
            }
        }
        self.current = point;
    }

    pub fn preview(&self) -> Option<Annotation> {
        let has_visible_geometry = match self.tool {
            AnnotationTool::Freehand => self.points.len() >= 2,
            _ => self.start != self.current,
        };
        has_visible_geometry.then(|| Annotation {
            id: self.id,
            kind: match self.tool {
                AnnotationTool::Blur => AnnotationKind::Blur {
                    bounds: PhysicalRect::new(self.start, self.current),
                },
                AnnotationTool::Mosaic => AnnotationKind::Mosaic {
                    bounds: PhysicalRect::new(self.start, self.current),
                },
                AnnotationTool::Highlight => AnnotationKind::Highlight {
                    bounds: PhysicalRect::new(self.start, self.current),
                },
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
                AnnotationTool::Freehand => AnnotationKind::Freehand {
                    points: self.points.clone(),
                },
            },
            style: self.style,
        })
    }
}

/// In-progress translation of an existing annotation, kept outside the
/// document until the pointer gesture is committed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnnotationMove {
    original: Annotation,
    anchor: PhysicalPoint,
    current: PhysicalPoint,
}

impl AnnotationMove {
    fn begin(annotation: Annotation, anchor: PhysicalPoint) -> Self {
        Self {
            original: annotation,
            anchor,
            current: anchor,
        }
    }

    pub fn preview(&self, canvas_bounds: PhysicalRect) -> Annotation {
        let bounds = self.original.bounds();
        let requested_x = self.current.x.saturating_sub(self.anchor.x);
        let requested_y = self.current.y.saturating_sub(self.anchor.y);
        let delta_x = requested_x.clamp(
            canvas_bounds.left.saturating_sub(bounds.left),
            canvas_bounds.right.saturating_sub(bounds.right),
        );
        let delta_y = requested_y.clamp(
            canvas_bounds.top.saturating_sub(bounds.top),
            canvas_bounds.bottom.saturating_sub(bounds.bottom),
        );
        self.original.translated(delta_x, delta_y)
    }
}

/// In-progress resize of an existing annotation from one of its bounding-box
/// corners. The opposite corner is fixed for the duration of the gesture.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnnotationResize {
    original: Annotation,
    anchor: PhysicalPoint,
    current: PhysicalPoint,
}

impl AnnotationResize {
    fn begin(annotation: Annotation, handle: ResizeHandle) -> Self {
        let bounds = annotation.bounds();
        let (anchor, current) = match handle {
            ResizeHandle::TopLeft => (
                PhysicalPoint {
                    x: bounds.right,
                    y: bounds.bottom,
                },
                PhysicalPoint {
                    x: bounds.left,
                    y: bounds.top,
                },
            ),
            ResizeHandle::TopRight => (
                PhysicalPoint {
                    x: bounds.left,
                    y: bounds.bottom,
                },
                PhysicalPoint {
                    x: bounds.right,
                    y: bounds.top,
                },
            ),
            ResizeHandle::BottomLeft => (
                PhysicalPoint {
                    x: bounds.right,
                    y: bounds.top,
                },
                PhysicalPoint {
                    x: bounds.left,
                    y: bounds.bottom,
                },
            ),
            ResizeHandle::BottomRight => (
                PhysicalPoint {
                    x: bounds.left,
                    y: bounds.top,
                },
                PhysicalPoint {
                    x: bounds.right,
                    y: bounds.bottom,
                },
            ),
        };
        Self {
            original: annotation,
            anchor,
            current,
        }
    }

    pub fn preview(&self, canvas_bounds: PhysicalRect) -> Annotation {
        let point = clamp_to_canvas(canvas_bounds, self.current);
        self.original.resized(PhysicalRect::new(self.anchor, point))
    }
}

/// Domain controller for creating annotations through pointer gestures.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AnnotationEditor {
    draft: Option<AnnotationDraft>,
    moving: Option<AnnotationMove>,
    resizing: Option<AnnotationResize>,
}

impl AnnotationEditor {
    pub fn draft(&self) -> Option<&AnnotationDraft> {
        self.draft.as_ref()
    }

    pub fn moving(&self) -> Option<&AnnotationMove> {
        self.moving.as_ref()
    }

    pub fn resizing(&self) -> Option<&AnnotationResize> {
        self.resizing.as_ref()
    }

    pub fn preview(&self, canvas_bounds: PhysicalRect) -> Option<Annotation> {
        self.draft
            .as_ref()
            .and_then(AnnotationDraft::preview)
            .or_else(|| {
                self.moving
                    .as_ref()
                    .map(|moving| moving.preview(canvas_bounds))
            })
            .or_else(|| {
                self.resizing
                    .as_ref()
                    .map(|resizing| resizing.preview(canvas_bounds))
            })
    }

    pub fn begin(
        &mut self,
        document: &AnnotationDocument,
        id: AnnotationId,
        tool: AnnotationTool,
        style: AnnotationStyle,
        start: PhysicalPoint,
    ) -> Result<(), AnnotationError> {
        if self.has_active_gesture() {
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

    pub fn begin_move(
        &mut self,
        document: &AnnotationDocument,
        id: AnnotationId,
        anchor: PhysicalPoint,
    ) -> Result<(), AnnotationError> {
        if self.has_active_gesture() {
            return Err(AnnotationError::DraftInProgress);
        }
        let annotation = document
            .annotation(id)
            .cloned()
            .ok_or(AnnotationError::MissingId(id))?;
        self.moving = Some(AnnotationMove::begin(
            annotation,
            clamp_to_canvas(document.canvas_bounds(), anchor),
        ));
        Ok(())
    }

    pub fn begin_resize(
        &mut self,
        document: &AnnotationDocument,
        id: AnnotationId,
        handle: ResizeHandle,
    ) -> Result<(), AnnotationError> {
        if self.has_active_gesture() {
            return Err(AnnotationError::DraftInProgress);
        }
        let annotation = document
            .annotation(id)
            .cloned()
            .ok_or(AnnotationError::MissingId(id))?;
        self.resizing = Some(AnnotationResize::begin(annotation, handle));
        Ok(())
    }

    pub fn update(&mut self, document: &AnnotationDocument, point: PhysicalPoint) -> bool {
        let point = clamp_to_canvas(document.canvas_bounds(), point);
        if let Some(draft) = &mut self.draft {
            draft.update(point);
            return true;
        }
        if let Some(moving) = &mut self.moving {
            moving.current = point;
            return true;
        }
        if let Some(resizing) = &mut self.resizing {
            resizing.current = point;
            return true;
        }
        false
    }

    pub fn cancel(&mut self) -> bool {
        self.draft.take().is_some()
            || self.moving.take().is_some()
            || self.resizing.take().is_some()
    }

    pub fn commit(
        &mut self,
        document: &mut AnnotationDocument,
        history: &mut CommandHistory,
    ) -> Result<bool, AnnotationError> {
        if let Some(draft) = self.draft.take() {
            let Some(annotation) = draft.preview() else {
                return Ok(false);
            };
            history.apply(document, AnnotationCommand::Insert(annotation))?;
            return Ok(true);
        }
        if let Some(moving) = self.moving.take() {
            let preview = moving.preview(document.canvas_bounds());
            if preview == moving.original {
                return Ok(false);
            }
            history.apply(document, AnnotationCommand::Replace(preview))?;
            return Ok(true);
        }
        let Some(resizing) = self.resizing.take() else {
            return Ok(false);
        };
        let preview = resizing.preview(document.canvas_bounds());
        if preview == resizing.original {
            return Ok(false);
        }
        history.apply(document, AnnotationCommand::Replace(preview))?;
        Ok(true)
    }

    fn has_active_gesture(&self) -> bool {
        self.draft.is_some() || self.moving.is_some() || self.resizing.is_some()
    }
}

fn scale_coordinate(
    value: i32,
    source_start: i32,
    source_end: i32,
    target_start: i32,
    target_end: i32,
) -> i32 {
    let source_span = i64::from(source_end) - i64::from(source_start);
    if source_span == 0 {
        return target_start;
    }
    let target_span = i64::from(target_end) - i64::from(target_start);
    let offset = i64::from(value) - i64::from(source_start);
    let scaled = i64::from(target_start) + offset * target_span / source_span;
    scaled.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

fn translate_rect(bounds: PhysicalRect, delta_x: i32, delta_y: i32) -> PhysicalRect {
    PhysicalRect {
        left: bounds.left.saturating_add(delta_x),
        top: bounds.top.saturating_add(delta_y),
        right: bounds.right.saturating_add(delta_x),
        bottom: bounds.bottom.saturating_add(delta_y),
    }
}

fn clamp_to_canvas(bounds: PhysicalRect, point: PhysicalPoint) -> PhysicalPoint {
    PhysicalPoint {
        x: point.x.clamp(bounds.left, bounds.right),
        y: point.y.clamp(bounds.top, bounds.bottom),
    }
}

fn bounds_for_points(points: &[PhysicalPoint]) -> PhysicalRect {
    let Some(&first) = points.first() else {
        return PhysicalRect::default();
    };
    let (left, top, right, bottom) = points.iter().skip(1).fold(
        (first.x, first.y, first.x, first.y),
        |(left, top, right, bottom), point| {
            (
                left.min(point.x),
                top.min(point.y),
                right.max(point.x),
                bottom.max(point.y),
            )
        },
    );
    PhysicalRect {
        left,
        top,
        right,
        bottom,
    }
}

fn squared_distance(first: PhysicalPoint, second: PhysicalPoint) -> u64 {
    let dx = i64::from(first.x) - i64::from(second.x);
    let dy = i64::from(first.y) - i64::from(second.y);
    (dx * dx + dy * dy) as u64
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
    Reorder { id: AnnotationId, index: usize },
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
            Self::Reorder { id, index } => Ok(Self::Reorder {
                id,
                index: document.reorder(id, index)?,
            }),
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
    use crate::domain::selection::ResizeHandle;

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
    fn reordering_annotations_is_undoable_and_redoable() {
        let mut document = AnnotationDocument::new(canvas()).unwrap();
        let mut history = CommandHistory::default();
        let first = rectangle(
            1,
            PhysicalRect {
                left: -800,
                top: 100,
                right: -600,
                bottom: 300,
            },
        );
        let second = rectangle(
            2,
            PhysicalRect {
                left: -500,
                top: 100,
                right: -300,
                bottom: 300,
            },
        );
        let third = rectangle(
            3,
            PhysicalRect {
                left: -200,
                top: 100,
                right: 0,
                bottom: 300,
            },
        );

        for annotation in [first.clone(), second.clone(), third.clone()] {
            history
                .apply(&mut document, AnnotationCommand::Insert(annotation))
                .unwrap();
        }
        history
            .apply(
                &mut document,
                AnnotationCommand::Reorder {
                    id: first.id,
                    index: 2,
                },
            )
            .unwrap();
        assert_eq!(
            document.annotations(),
            &[second.clone(), third.clone(), first.clone()]
        );

        assert!(history.undo(&mut document).unwrap());
        assert_eq!(
            document.annotations(),
            &[first.clone(), second.clone(), third.clone()]
        );

        assert!(history.redo(&mut document).unwrap());
        assert_eq!(document.annotations(), &[second, third, first]);
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
    fn highlight_gesture_uses_a_filled_rect_with_direct_interior_hit_testing() {
        let mut document = AnnotationDocument::new(canvas()).unwrap();
        let mut history = CommandHistory::default();
        let mut editor = AnnotationEditor::default();

        editor
            .begin(
                &document,
                AnnotationId::new(90),
                AnnotationTool::Highlight,
                AnnotationStyle {
                    stroke_rgba: 0xFFCC0066,
                    fill_rgba: None,
                    stroke_width: 1,
                },
                PhysicalPoint { x: -100, y: 100 },
            )
            .unwrap();
        editor.update(&document, PhysicalPoint { x: 100, y: 200 });

        assert_eq!(
            editor.draft().unwrap().preview().unwrap().kind,
            AnnotationKind::Highlight {
                bounds: PhysicalRect {
                    left: -100,
                    top: 100,
                    right: 100,
                    bottom: 200,
                },
            }
        );
        assert!(editor.commit(&mut document, &mut history).unwrap());
        let highlight = &document.annotations()[0];
        assert!(highlight.hit_test(PhysicalPoint { x: 0, y: 150 }, 0));
        assert!(!highlight.hit_test(PhysicalPoint { x: 100, y: 150 }, 0));
    }

    #[test]
    fn mosaic_gesture_uses_a_resizable_rect_with_interior_hit_testing() {
        let mut document = AnnotationDocument::new(canvas()).unwrap();
        let mut history = CommandHistory::default();
        let mut editor = AnnotationEditor::default();

        editor
            .begin(
                &document,
                AnnotationId::new(91),
                AnnotationTool::Mosaic,
                AnnotationStyle::default(),
                PhysicalPoint { x: -100, y: 100 },
            )
            .unwrap();
        editor.update(&document, PhysicalPoint { x: 100, y: 200 });

        assert_eq!(
            editor.draft().unwrap().preview().unwrap().kind,
            AnnotationKind::Mosaic {
                bounds: PhysicalRect {
                    left: -100,
                    top: 100,
                    right: 100,
                    bottom: 200,
                },
            }
        );
        assert!(editor.commit(&mut document, &mut history).unwrap());
        let mosaic = &document.annotations()[0];
        assert!(mosaic.hit_test(PhysicalPoint { x: 0, y: 150 }, 0));
        assert!(!mosaic.hit_test(PhysicalPoint { x: 100, y: 150 }, 0));
    }

    #[test]
    fn blur_gesture_uses_a_resizable_rect_with_interior_hit_testing() {
        let mut document = AnnotationDocument::new(canvas()).unwrap();
        let mut history = CommandHistory::default();
        let mut editor = AnnotationEditor::default();

        editor
            .begin(
                &document,
                AnnotationId::new(92),
                AnnotationTool::Blur,
                AnnotationStyle::default(),
                PhysicalPoint { x: -100, y: 100 },
            )
            .unwrap();
        editor.update(&document, PhysicalPoint { x: 100, y: 200 });

        assert_eq!(
            editor.draft().unwrap().preview().unwrap().kind,
            AnnotationKind::Blur {
                bounds: PhysicalRect {
                    left: -100,
                    top: 100,
                    right: 100,
                    bottom: 200,
                },
            }
        );
        assert!(editor.commit(&mut document, &mut history).unwrap());
        let blur = &document.annotations()[0];
        assert!(blur.hit_test(PhysicalPoint { x: 0, y: 150 }, 0));
        assert!(!blur.hit_test(PhysicalPoint { x: 100, y: 150 }, 0));
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

    #[test]
    fn moving_existing_annotation_stays_temporary_until_commit_and_is_undoable() {
        let mut document = AnnotationDocument::new(canvas()).unwrap();
        let mut history = CommandHistory::default();
        let original = rectangle(
            201,
            PhysicalRect {
                left: -500,
                top: 100,
                right: -200,
                bottom: 300,
            },
        );
        history
            .apply(&mut document, AnnotationCommand::Insert(original.clone()))
            .unwrap();
        let mut editor = AnnotationEditor::default();

        editor
            .begin_move(&document, original.id, PhysicalPoint { x: -400, y: 200 })
            .unwrap();
        assert!(editor.update(&document, PhysicalPoint { x: -100, y: 500 }));
        assert_eq!(document.annotation(original.id), Some(&original));
        assert_eq!(history.undo_len(), 1);
        assert_eq!(
            editor.preview(document.canvas_bounds()).unwrap().kind,
            AnnotationKind::Rectangle {
                bounds: PhysicalRect {
                    left: -200,
                    top: 400,
                    right: 100,
                    bottom: 600,
                },
            }
        );

        assert!(editor.commit(&mut document, &mut history).unwrap());
        let moved = document.annotation(original.id).unwrap().clone();
        assert_ne!(moved, original);
        assert_eq!(history.undo_len(), 2);
        assert!(history.undo(&mut document).unwrap());
        assert_eq!(document.annotation(original.id), Some(&original));
    }

    #[test]
    fn moving_clamps_to_canvas_and_noop_or_cancel_does_not_create_history() {
        let mut document = AnnotationDocument::new(canvas()).unwrap();
        let mut history = CommandHistory::default();
        let original = Annotation {
            id: AnnotationId::new(202),
            kind: AnnotationKind::Arrow {
                start: PhysicalPoint { x: 1800, y: 900 },
                end: PhysicalPoint { x: 1900, y: 1000 },
            },
            style: AnnotationStyle::default(),
        };
        history
            .apply(&mut document, AnnotationCommand::Insert(original.clone()))
            .unwrap();
        let mut editor = AnnotationEditor::default();

        editor
            .begin_move(&document, original.id, PhysicalPoint { x: 1850, y: 950 })
            .unwrap();
        editor.update(&document, PhysicalPoint { x: 9000, y: 9000 });
        assert_eq!(
            editor.preview(document.canvas_bounds()).unwrap().kind,
            AnnotationKind::Arrow {
                start: PhysicalPoint { x: 1820, y: 980 },
                end: PhysicalPoint { x: 1920, y: 1080 },
            }
        );
        assert!(editor.cancel());
        assert_eq!(document.annotation(original.id), Some(&original));
        assert_eq!(history.undo_len(), 1);

        editor
            .begin_move(&document, original.id, PhysicalPoint { x: 1850, y: 950 })
            .unwrap();
        assert!(!editor.commit(&mut document, &mut history).unwrap());
        assert_eq!(history.undo_len(), 1);
        assert_eq!(
            editor.begin_move(
                &document,
                AnnotationId::new(999),
                PhysicalPoint { x: 0, y: 0 }
            ),
            Err(AnnotationError::MissingId(AnnotationId::new(999)))
        );
    }

    #[test]
    fn resizing_shape_previews_with_opposite_corner_fixed_then_commits_once() {
        let mut document = AnnotationDocument::new(canvas()).unwrap();
        let mut history = CommandHistory::default();
        let original = rectangle(
            301,
            PhysicalRect {
                left: -500,
                top: 100,
                right: -100,
                bottom: 500,
            },
        );
        history
            .apply(&mut document, AnnotationCommand::Insert(original.clone()))
            .unwrap();
        let mut editor = AnnotationEditor::default();

        editor
            .begin_resize(&document, original.id, ResizeHandle::TopLeft)
            .unwrap();
        editor.update(&document, PhysicalPoint { x: -700, y: -100 });
        assert_eq!(document.annotation(original.id), Some(&original));
        assert_eq!(history.undo_len(), 1);
        assert_eq!(
            editor.preview(document.canvas_bounds()).unwrap().kind,
            AnnotationKind::Rectangle {
                bounds: PhysicalRect {
                    left: -700,
                    top: 0,
                    right: -100,
                    bottom: 500,
                },
            }
        );

        assert!(editor.commit(&mut document, &mut history).unwrap());
        assert_eq!(history.undo_len(), 2);
        assert!(history.undo(&mut document).unwrap());
        assert_eq!(document.annotation(original.id), Some(&original));
    }

    #[test]
    fn resizing_line_scales_endpoints_and_clamps_to_canvas() {
        let mut document = AnnotationDocument::new(canvas()).unwrap();
        let mut history = CommandHistory::default();
        let original = Annotation {
            id: AnnotationId::new(302),
            kind: AnnotationKind::Line {
                start: PhysicalPoint { x: 100, y: 100 },
                end: PhysicalPoint { x: 500, y: 300 },
            },
            style: AnnotationStyle::default(),
        };
        history
            .apply(&mut document, AnnotationCommand::Insert(original.clone()))
            .unwrap();
        let mut editor = AnnotationEditor::default();

        editor
            .begin_resize(&document, original.id, ResizeHandle::BottomRight)
            .unwrap();
        editor.update(&document, PhysicalPoint { x: 9000, y: 9000 });
        assert_eq!(
            editor.preview(document.canvas_bounds()).unwrap().kind,
            AnnotationKind::Line {
                start: PhysicalPoint { x: 100, y: 100 },
                end: PhysicalPoint { x: 1920, y: 1080 },
            }
        );
        assert!(editor.commit(&mut document, &mut history).unwrap());
        assert_eq!(history.undo_len(), 2);
    }

    #[test]
    fn resize_cancel_noop_and_competing_gesture_leave_history_unchanged() {
        let mut document = AnnotationDocument::new(canvas()).unwrap();
        let mut history = CommandHistory::default();
        let original = rectangle(
            303,
            PhysicalRect {
                left: 100,
                top: 100,
                right: 300,
                bottom: 300,
            },
        );
        history
            .apply(&mut document, AnnotationCommand::Insert(original.clone()))
            .unwrap();
        let mut editor = AnnotationEditor::default();

        editor
            .begin_resize(&document, original.id, ResizeHandle::BottomRight)
            .unwrap();
        assert_eq!(
            editor.begin_move(&document, original.id, PhysicalPoint { x: 200, y: 200 }),
            Err(AnnotationError::DraftInProgress)
        );
        assert!(!editor.commit(&mut document, &mut history).unwrap());
        assert_eq!(history.undo_len(), 1);

        editor
            .begin_resize(&document, original.id, ResizeHandle::BottomRight)
            .unwrap();
        editor.update(&document, PhysicalPoint { x: 600, y: 600 });
        assert!(editor.cancel());
        assert_eq!(document.annotation(original.id), Some(&original));
        assert_eq!(history.undo_len(), 1);
    }

    #[test]
    fn freehand_draft_debounces_nearby_samples_and_commits_one_path() {
        let mut document = AnnotationDocument::new(canvas()).unwrap();
        let mut history = CommandHistory::default();
        let mut editor = AnnotationEditor::default();

        editor
            .begin(
                &document,
                AnnotationId::new(401),
                AnnotationTool::Freehand,
                AnnotationStyle::default(),
                PhysicalPoint { x: -100, y: 100 },
            )
            .unwrap();
        editor.update(&document, PhysicalPoint { x: -99, y: 100 });
        editor.update(&document, PhysicalPoint { x: -98, y: 100 });
        editor.update(&document, PhysicalPoint { x: -90, y: 105 });
        editor.update(&document, PhysicalPoint { x: -80, y: 110 });
        assert!(document.annotations().is_empty());
        assert_eq!(history.undo_len(), 0);
        assert_eq!(
            editor.preview(document.canvas_bounds()).unwrap().kind,
            AnnotationKind::Freehand {
                points: vec![
                    PhysicalPoint { x: -100, y: 100 },
                    PhysicalPoint { x: -98, y: 100 },
                    PhysicalPoint { x: -90, y: 105 },
                    PhysicalPoint { x: -80, y: 110 },
                ],
            }
        );

        assert!(editor.commit(&mut document, &mut history).unwrap());
        assert_eq!(document.annotations().len(), 1);
        assert_eq!(history.undo_len(), 1);
    }

    #[test]
    fn freehand_paths_hit_test_and_transform_like_other_annotations() {
        let mut document = AnnotationDocument::new(canvas()).unwrap();
        let mut history = CommandHistory::default();
        let original = Annotation {
            id: AnnotationId::new(402),
            kind: AnnotationKind::Freehand {
                points: vec![
                    PhysicalPoint { x: -500, y: 100 },
                    PhysicalPoint { x: -300, y: 200 },
                    PhysicalPoint { x: -100, y: 100 },
                ],
            },
            style: AnnotationStyle::default(),
        };
        assert!(original.hit_test(PhysicalPoint { x: -300, y: 201 }, 0));
        assert!(!original.hit_test(PhysicalPoint { x: -300, y: 260 }, 0));
        history
            .apply(&mut document, AnnotationCommand::Insert(original.clone()))
            .unwrap();
        let mut editor = AnnotationEditor::default();

        editor
            .begin_move(&document, original.id, PhysicalPoint { x: -300, y: 150 })
            .unwrap();
        editor.update(&document, PhysicalPoint { x: -200, y: 250 });
        assert_eq!(
            editor.preview(document.canvas_bounds()).unwrap().kind,
            AnnotationKind::Freehand {
                points: vec![
                    PhysicalPoint { x: -400, y: 200 },
                    PhysicalPoint { x: -200, y: 300 },
                    PhysicalPoint { x: 0, y: 200 },
                ],
            }
        );
        assert!(editor.commit(&mut document, &mut history).unwrap());

        editor
            .begin_resize(&document, original.id, ResizeHandle::BottomRight)
            .unwrap();
        editor.update(&document, PhysicalPoint { x: 400, y: 600 });
        assert_eq!(
            editor.preview(document.canvas_bounds()).unwrap().kind,
            AnnotationKind::Freehand {
                points: vec![
                    PhysicalPoint { x: -400, y: 200 },
                    PhysicalPoint { x: 0, y: 600 },
                    PhysicalPoint { x: 400, y: 200 },
                ],
            }
        );
    }

    #[test]
    fn freehand_needs_two_distinct_samples_before_entering_history() {
        let mut document = AnnotationDocument::new(canvas()).unwrap();
        let mut history = CommandHistory::default();
        let mut editor = AnnotationEditor::default();

        editor
            .begin(
                &document,
                AnnotationId::new(403),
                AnnotationTool::Freehand,
                AnnotationStyle::default(),
                PhysicalPoint { x: 0, y: 0 },
            )
            .unwrap();
        editor.update(&document, PhysicalPoint { x: 1, y: 0 });
        assert!(!editor.commit(&mut document, &mut history).unwrap());
        assert!(document.annotations().is_empty());
        assert_eq!(history.undo_len(), 0);
    }
}
