//! Versioned, renderer-independent annotation documents and reversible commands.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::geometry::{PhysicalPoint, PhysicalRect};
use super::selection::ResizeHandle;

pub const ANNOTATION_DOCUMENT_VERSION: u32 = 1;
const MIN_FREEHAND_SAMPLE_DISTANCE: u32 = 2;
pub const SEQUENCE_MARKER_RADIUS: i32 = 14;
pub const TEXT_ANNOTATION_HEIGHT: i32 = 28;
pub const TEXT_ANNOTATION_ADVANCE: i32 = 16;
pub const WATERMARK_CONTENT: &str = "Flash Shot";

pub const DEFAULT_TEXT_FONT_SIZE: u32 = 24;

fn default_watermark_content() -> String {
    WATERMARK_CONTENT.to_owned()
}

const fn default_text_font_size() -> u32 {
    DEFAULT_TEXT_FONT_SIZE
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct AnnotationId(u64);

impl AnnotationId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AnnotationStyle {
    pub stroke_rgba: u32,
    pub fill_rgba: Option<u32>,
    pub stroke_width: u32,
    #[serde(default = "default_text_font_size")]
    pub text_font_size: u32,
}

impl Default for AnnotationStyle {
    fn default() -> Self {
        Self {
            stroke_rgba: 0xFF3B30FF,
            fill_rgba: None,
            stroke_width: 4,
            text_font_size: DEFAULT_TEXT_FONT_SIZE,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum AnnotationKind {
    Watermark {
        origin: PhysicalPoint,
        #[serde(default = "default_watermark_content")]
        content: String,
    },
    Text {
        origin: PhysicalPoint,
        content: String,
    },
    Number {
        center: PhysicalPoint,
        value: u32,
    },
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Annotation {
    pub id: AnnotationId,
    pub kind: AnnotationKind,
    pub style: AnnotationStyle,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
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

    /// Serializes the renderer-independent document with its explicit schema
    /// version so it can be safely persisted independently of the PNG pixels.
    pub fn to_json(&self) -> Result<String, AnnotationError> {
        serde_json::to_string(self)
            .map_err(|error| AnnotationError::DocumentFormat(error.to_string()))
    }

    /// Restores a document only when it matches the supported schema and all
    /// coordinates remain inside the declared physical-pixel canvas.
    pub fn from_json(json: &str) -> Result<Self, AnnotationError> {
        let document: Self = serde_json::from_str(json)
            .map_err(|error| AnnotationError::DocumentFormat(error.to_string()))?;
        if document.version != ANNOTATION_DOCUMENT_VERSION {
            return Err(AnnotationError::UnsupportedVersion(document.version));
        }
        let mut validated = Self::new(document.canvas_bounds)?;
        for annotation in document.annotations {
            if !annotation_is_within_canvas(&annotation, validated.canvas_bounds) {
                return Err(AnnotationError::AnnotationOutsideCanvas(annotation.id));
            }
            validated.insert(annotation)?;
        }
        Ok(validated)
    }

    /// Re-expresses every annotation in another canvas with identical pixel
    /// dimensions. This preserves geometry while making virtual-desktop
    /// captures portable alongside a PNG whose origin is `(0, 0)`.
    pub fn rebased_to(&self, canvas_bounds: PhysicalRect) -> Result<Self, AnnotationError> {
        if self.canvas_bounds.width() != canvas_bounds.width()
            || self.canvas_bounds.height() != canvas_bounds.height()
        {
            return Err(AnnotationError::IncompatibleCanvasBounds);
        }
        let delta_x = canvas_bounds.left.saturating_sub(self.canvas_bounds.left);
        let delta_y = canvas_bounds.top.saturating_sub(self.canvas_bounds.top);
        Ok(Self {
            version: self.version,
            canvas_bounds,
            annotations: self
                .annotations
                .iter()
                .map(|annotation| annotation.translated(delta_x, delta_y))
                .collect(),
        })
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
    pub const fn supports_fill(&self) -> bool {
        matches!(
            self.kind,
            AnnotationKind::Rectangle { .. } | AnnotationKind::Ellipse { .. }
        )
    }

    pub const fn supports_clockwise_rotation(&self) -> bool {
        !matches!(
            self.kind,
            AnnotationKind::Watermark { .. }
                | AnnotationKind::Text { .. }
                | AnnotationKind::Number { .. }
        )
    }

    pub const fn text_font_size(&self) -> u32 {
        if self.style.text_font_size == 0 {
            1
        } else {
            self.style.text_font_size
        }
    }

    /// Tests a physical image coordinate against this annotation's visible geometry.
    pub fn hit_test(&self, point: PhysicalPoint, tolerance: u32) -> bool {
        let threshold = self
            .style
            .stroke_width
            .saturating_add(tolerance.saturating_mul(2));
        match self.kind {
            AnnotationKind::Watermark {
                origin,
                ref content,
            } => text_bounds(origin, content, self.text_font_size()).contains(point),
            AnnotationKind::Text {
                origin,
                ref content,
                ..
            } => text_bounds(origin, content, self.text_font_size()).contains(point),
            AnnotationKind::Number { center, .. } => {
                let radius = SEQUENCE_MARKER_RADIUS;
                PhysicalRect {
                    left: center.x.saturating_sub(radius),
                    top: center.y.saturating_sub(radius),
                    right: center.x.saturating_add(radius),
                    bottom: center.y.saturating_add(radius),
                }
                .contains(point)
            }
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
            AnnotationKind::Watermark {
                origin,
                ref content,
            } => text_bounds(origin, content, self.text_font_size()),
            AnnotationKind::Text {
                origin,
                ref content,
                ..
            } => text_bounds(origin, content, self.text_font_size()),
            AnnotationKind::Number { center, .. } => marker_bounds(center),
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
                AnnotationKind::Watermark {
                    origin,
                    ref content,
                } => AnnotationKind::Watermark {
                    origin: translate(origin),
                    content: content.clone(),
                },
                AnnotationKind::Text {
                    origin,
                    ref content,
                } => AnnotationKind::Text {
                    origin: translate(origin),
                    content: content.clone(),
                },
                AnnotationKind::Number { center, value } => AnnotationKind::Number {
                    center: translate(center),
                    value,
                },
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

    /// Translates an annotation without allowing any part of its bounds to
    /// leave the canvas. The original size is retained at each edge.
    pub fn translated_within(
        &self,
        canvas_bounds: PhysicalRect,
        delta_x: i32,
        delta_y: i32,
    ) -> Self {
        let bounds = self.bounds();
        let delta_x = delta_x.clamp(
            canvas_bounds.left.saturating_sub(bounds.left),
            canvas_bounds.right.saturating_sub(bounds.right),
        );
        let delta_y = delta_y.clamp(
            canvas_bounds.top.saturating_sub(bounds.top),
            canvas_bounds.bottom.saturating_sub(bounds.bottom),
        );
        self.translated(delta_x, delta_y)
    }

    /// Makes a distinct annotation offset inside the image canvas so a copied
    /// annotation remains immediately visible and selectable.
    pub fn duplicated(&self, id: AnnotationId, canvas_bounds: PhysicalRect, offset: i32) -> Self {
        let mut duplicate = self.translated_within(canvas_bounds, offset, offset);
        duplicate.id = id;
        duplicate
    }

    /// Rotates drawable geometry a quarter turn clockwise around its bounds
    /// center, then translates it back inside the canvas when necessary.
    /// Text is deliberately excluded until the renderers support rotated glyphs.
    pub fn rotated_clockwise_within(&self, canvas_bounds: PhysicalRect) -> Option<Self> {
        if !self.supports_clockwise_rotation() {
            return None;
        }
        let source = self.bounds();
        let rotate = |point| rotate_clockwise(point, source);
        let rotate_rect = |bounds: PhysicalRect| {
            bounds_for_points(&[
                rotate(PhysicalPoint {
                    x: bounds.left,
                    y: bounds.top,
                }),
                rotate(PhysicalPoint {
                    x: bounds.right,
                    y: bounds.top,
                }),
                rotate(PhysicalPoint {
                    x: bounds.left,
                    y: bounds.bottom,
                }),
                rotate(PhysicalPoint {
                    x: bounds.right,
                    y: bounds.bottom,
                }),
            ])
        };
        let kind = match self.kind {
            AnnotationKind::Watermark { .. }
            | AnnotationKind::Text { .. }
            | AnnotationKind::Number { .. } => unreachable!(),
            AnnotationKind::Blur { bounds } => AnnotationKind::Blur {
                bounds: rotate_rect(bounds),
            },
            AnnotationKind::Mosaic { bounds } => AnnotationKind::Mosaic {
                bounds: rotate_rect(bounds),
            },
            AnnotationKind::Highlight { bounds } => AnnotationKind::Highlight {
                bounds: rotate_rect(bounds),
            },
            AnnotationKind::Rectangle { bounds } => AnnotationKind::Rectangle {
                bounds: rotate_rect(bounds),
            },
            AnnotationKind::Ellipse { bounds } => AnnotationKind::Ellipse {
                bounds: rotate_rect(bounds),
            },
            AnnotationKind::Line { start, end } => AnnotationKind::Line {
                start: rotate(start),
                end: rotate(end),
            },
            AnnotationKind::Arrow { start, end } => AnnotationKind::Arrow {
                start: rotate(start),
                end: rotate(end),
            },
            AnnotationKind::Freehand { ref points } => AnnotationKind::Freehand {
                points: points.iter().copied().map(rotate).collect(),
            },
        };
        Some(
            Self {
                id: self.id,
                kind,
                style: self.style,
            }
            .translated_within(canvas_bounds, 0, 0),
        )
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
                AnnotationKind::Watermark { ref content, .. } => AnnotationKind::Watermark {
                    origin: PhysicalPoint {
                        x: bounds.left,
                        y: bounds.top,
                    },
                    content: content.clone(),
                },
                AnnotationKind::Text { ref content, .. } => AnnotationKind::Text {
                    origin: PhysicalPoint {
                        x: bounds.left,
                        y: bounds.top,
                    },
                    content: content.clone(),
                },
                AnnotationKind::Number { value, .. } => AnnotationKind::Number {
                    center: PhysicalPoint {
                        x: (bounds.left + bounds.right) / 2,
                        y: (bounds.top + bounds.bottom) / 2,
                    },
                    value,
                },
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
    Watermark,
    Text,
    Number,
    Blur,
    Mosaic,
    Highlight,
    Rectangle,
    Ellipse,
    Line,
    Arrow,
    Freehand,
}

impl AnnotationTool {
    pub const fn supports_fill(self) -> bool {
        matches!(self, Self::Rectangle | Self::Ellipse)
    }
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
    sequence_number: Option<u32>,
    text: Option<String>,
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
            sequence_number: None,
            text: None,
        }
    }

    pub const fn start(&self) -> PhysicalPoint {
        self.start
    }

    pub const fn current(&self) -> PhysicalPoint {
        self.current
    }

    pub fn update(&mut self, point: PhysicalPoint) {
        if self.tool == AnnotationTool::Freehand
            && let Some(last_sample) = self.points.last().copied()
            && squared_distance(last_sample, point)
                >= u64::from(MIN_FREEHAND_SAMPLE_DISTANCE).pow(2)
        {
            self.points.push(point);
        }
        self.current = point;
    }

    fn with_sequence_number(mut self, value: u32) -> Self {
        self.sequence_number = Some(value);
        self
    }

    fn with_text(mut self, text: String) -> Self {
        self.text = Some(text);
        self
    }

    pub fn preview(&self) -> Option<Annotation> {
        let has_visible_geometry = match self.tool {
            AnnotationTool::Watermark => self.text.as_ref().is_some_and(|text| !text.is_empty()),
            AnnotationTool::Text => self.text.as_ref().is_some_and(|text| !text.is_empty()),
            AnnotationTool::Number => true,
            AnnotationTool::Freehand => self.points.len() >= 2,
            _ => self.start != self.current,
        };
        has_visible_geometry.then(|| Annotation {
            id: self.id,
            kind: match self.tool {
                AnnotationTool::Watermark => AnnotationKind::Watermark {
                    origin: self.start,
                    content: self.text.clone().unwrap_or_default(),
                },
                AnnotationTool::Text => AnnotationKind::Text {
                    origin: self.start,
                    content: self.text.clone().unwrap_or_default(),
                },
                AnnotationTool::Number => AnnotationKind::Number {
                    center: self.start,
                    value: self.sequence_number.unwrap_or(1),
                },
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

    pub fn begin_number(
        &mut self,
        document: &AnnotationDocument,
        id: AnnotationId,
        style: AnnotationStyle,
        center: PhysicalPoint,
        value: u32,
    ) -> Result<(), AnnotationError> {
        if self.has_active_gesture() {
            return Err(AnnotationError::DraftInProgress);
        }
        if document.annotation(id).is_some() {
            return Err(AnnotationError::DuplicateId(id));
        }
        self.draft = Some(
            AnnotationDraft::begin(
                id,
                AnnotationTool::Number,
                style,
                clamp_to_canvas(document.canvas_bounds(), center),
            )
            .with_sequence_number(value),
        );
        Ok(())
    }

    pub fn begin_text(
        &mut self,
        document: &AnnotationDocument,
        id: AnnotationId,
        style: AnnotationStyle,
        origin: PhysicalPoint,
        text: String,
    ) -> Result<(), AnnotationError> {
        self.begin_text_for_tool(document, id, AnnotationTool::Text, style, origin, text)
    }

    pub fn begin_watermark(
        &mut self,
        document: &AnnotationDocument,
        id: AnnotationId,
        style: AnnotationStyle,
        origin: PhysicalPoint,
        text: String,
    ) -> Result<(), AnnotationError> {
        self.begin_text_for_tool(document, id, AnnotationTool::Watermark, style, origin, text)
    }

    fn begin_text_for_tool(
        &mut self,
        document: &AnnotationDocument,
        id: AnnotationId,
        tool: AnnotationTool,
        style: AnnotationStyle,
        origin: PhysicalPoint,
        text: String,
    ) -> Result<(), AnnotationError> {
        if self.has_active_gesture() {
            return Err(AnnotationError::DraftInProgress);
        }
        if document.annotation(id).is_some() {
            return Err(AnnotationError::DuplicateId(id));
        }
        self.draft = Some(
            AnnotationDraft::begin(
                id,
                tool,
                style,
                clamp_to_canvas(document.canvas_bounds(), origin),
            )
            .with_text(text),
        );
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

fn rotate_clockwise(point: PhysicalPoint, bounds: PhysicalRect) -> PhysicalPoint {
    let center_x2 = i64::from(bounds.left) + i64::from(bounds.right);
    let center_y2 = i64::from(bounds.top) + i64::from(bounds.bottom);
    let x2 = i64::from(point.x) * 2;
    let y2 = i64::from(point.y) * 2;
    PhysicalPoint {
        x: round_divide_by_two(center_x2 + y2 - center_y2),
        y: round_divide_by_two(center_y2 - x2 + center_x2),
    }
}

fn round_divide_by_two(value: i64) -> i32 {
    let rounded = if value >= 0 { value + 1 } else { value - 1 };
    (rounded / 2).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
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

fn annotation_is_within_canvas(annotation: &Annotation, canvas: PhysicalRect) -> bool {
    let contains = |point: PhysicalPoint| {
        point.x >= canvas.left
            && point.x <= canvas.right
            && point.y >= canvas.top
            && point.y <= canvas.bottom
    };
    match annotation.kind {
        AnnotationKind::Watermark { origin, .. } => contains(origin),
        AnnotationKind::Text { origin, .. } => contains(origin),
        AnnotationKind::Number { center, .. } => contains(center),
        AnnotationKind::Blur { bounds }
        | AnnotationKind::Mosaic { bounds }
        | AnnotationKind::Highlight { bounds }
        | AnnotationKind::Rectangle { bounds }
        | AnnotationKind::Ellipse { bounds } => {
            bounds.left >= canvas.left
                && bounds.top >= canvas.top
                && bounds.right <= canvas.right
                && bounds.bottom <= canvas.bottom
                && bounds.width() > 0
                && bounds.height() > 0
        }
        AnnotationKind::Line { start, end } | AnnotationKind::Arrow { start, end } => {
            contains(start) && contains(end)
        }
        AnnotationKind::Freehand { ref points } => {
            points.len() >= 2 && points.iter().copied().all(contains)
        }
    }
}

fn marker_bounds(center: PhysicalPoint) -> PhysicalRect {
    PhysicalRect {
        left: center.x.saturating_sub(SEQUENCE_MARKER_RADIUS),
        top: center.y.saturating_sub(SEQUENCE_MARKER_RADIUS),
        right: center.x.saturating_add(SEQUENCE_MARKER_RADIUS),
        bottom: center.y.saturating_add(SEQUENCE_MARKER_RADIUS),
    }
}

fn text_bounds(origin: PhysicalPoint, content: &str, font_size: u32) -> PhysicalRect {
    let glyphs = content.chars().count().max(1) as i32;
    let advance = i32::try_from(font_size.saturating_mul(2).div_ceil(3)).unwrap_or(i32::MAX);
    let height = i32::try_from(font_size.saturating_add(4)).unwrap_or(i32::MAX);
    PhysicalRect {
        left: origin.x,
        top: origin.y,
        right: origin.x.saturating_add(glyphs.saturating_mul(advance)),
        bottom: origin.y.saturating_add(height),
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
    IncompatibleCanvasBounds,
    UnsupportedVersion(u32),
    AnnotationOutsideCanvas(AnnotationId),
    DocumentFormat(String),
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
            Self::IncompatibleCanvasBounds => {
                formatter.write_str("annotation document canvas dimensions do not match")
            }
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "annotation document version {version} is unsupported"
                )
            }
            Self::AnnotationOutsideCanvas(id) => {
                write!(
                    formatter,
                    "annotation {} is outside the document canvas",
                    id.value()
                )
            }
            Self::DocumentFormat(error) => {
                write!(formatter, "invalid annotation document: {error}")
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
        AnnotationTool, CommandHistory, DEFAULT_TEXT_FONT_SIZE, WATERMARK_CONTENT,
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
    fn text_draft_commits_unicode_content_and_is_reversible() {
        let mut document = AnnotationDocument::new(canvas()).unwrap();
        let mut history = CommandHistory::default();
        let mut editor = AnnotationEditor::default();
        let origin = PhysicalPoint { x: -120, y: 240 };

        editor
            .begin_text(
                &document,
                AnnotationId::new(80),
                AnnotationStyle::default(),
                origin,
                "Hello, 中文".to_owned(),
            )
            .unwrap();
        assert_eq!(
            editor.preview(document.canvas_bounds()).unwrap().kind,
            AnnotationKind::Text {
                origin,
                content: "Hello, 中文".to_owned(),
            }
        );
        assert!(editor.commit(&mut document, &mut history).unwrap());
        let text = document.annotation(AnnotationId::new(80)).unwrap().clone();
        assert!(text.hit_test(origin, 0));
        assert!(text.hit_test(PhysicalPoint { x: 7, y: 250 }, 0));
        assert!(!text.hit_test(PhysicalPoint { x: 500, y: 250 }, 0));

        assert!(history.undo(&mut document).unwrap());
        assert!(document.annotations().is_empty());
        assert!(history.redo(&mut document).unwrap());
        assert_eq!(document.annotation(AnnotationId::new(80)), Some(&text));
    }

    #[test]
    fn watermark_commits_from_a_single_click_and_remains_reversible() {
        let mut document = AnnotationDocument::new(canvas()).unwrap();
        let mut history = CommandHistory::default();
        let mut editor = AnnotationEditor::default();
        let origin = PhysicalPoint { x: -120, y: 240 };

        editor
            .begin_watermark(
                &document,
                AnnotationId::new(81),
                AnnotationStyle::default(),
                origin,
                "Internal only".to_owned(),
            )
            .unwrap();
        assert_eq!(
            editor.preview(document.canvas_bounds()).unwrap().kind,
            AnnotationKind::Watermark {
                origin,
                content: "Internal only".to_owned(),
            }
        );
        assert!(editor.commit(&mut document, &mut history).unwrap());
        let watermark = document.annotation(AnnotationId::new(81)).unwrap().clone();
        assert!(watermark.hit_test(origin, 0));
        assert!(history.undo(&mut document).unwrap());
        assert!(document.annotations().is_empty());
        assert!(history.redo(&mut document).unwrap());
        assert_eq!(document.annotation(AnnotationId::new(81)), Some(&watermark));
    }

    #[test]
    fn legacy_watermarks_default_to_the_original_content_when_deserialized() {
        let document = AnnotationDocument::from_json(
            r#"{
                "version": 1,
                "canvas_bounds": {"left": 0, "top": 0, "right": 100, "bottom": 100},
                "annotations": [{
                    "id": 81,
                    "kind": {"Watermark": {"origin": {"x": 4, "y": 8}}},
                    "style": {"stroke_rgba": 4282071295, "fill_rgba": null, "stroke_width": 4}
                }]
            }"#,
        )
        .unwrap();

        assert_eq!(
            document.annotation(AnnotationId::new(81)).unwrap().kind,
            AnnotationKind::Watermark {
                origin: PhysicalPoint { x: 4, y: 8 },
                content: WATERMARK_CONTENT.to_owned(),
            }
        );
    }

    #[test]
    fn number_marker_commits_from_a_single_click_and_is_reversible() {
        let mut document = AnnotationDocument::new(canvas()).unwrap();
        let mut history = CommandHistory::default();
        let mut editor = AnnotationEditor::default();
        let center = PhysicalPoint { x: -120, y: 240 };

        editor
            .begin_number(
                &document,
                AnnotationId::new(90),
                AnnotationStyle::default(),
                center,
                12,
            )
            .unwrap();
        assert_eq!(
            editor.preview(document.canvas_bounds()).unwrap().kind,
            AnnotationKind::Number { center, value: 12 }
        );

        assert!(editor.commit(&mut document, &mut history).unwrap());
        let marker = document.annotation(AnnotationId::new(90)).unwrap().clone();
        assert_eq!(marker.kind, AnnotationKind::Number { center, value: 12 });
        assert!(marker.hit_test(center, 0));
        assert!(!marker.hit_test(PhysicalPoint { x: -200, y: 240 }, 0));

        assert!(history.undo(&mut document).unwrap());
        assert!(document.annotations().is_empty());
        assert!(history.redo(&mut document).unwrap());
        assert_eq!(document.annotation(AnnotationId::new(90)), Some(&marker));
    }

    #[test]
    fn number_marker_moves_and_resizes_by_its_fixed_bounds() {
        let marker = Annotation {
            id: AnnotationId::new(91),
            kind: AnnotationKind::Number {
                center: PhysicalPoint { x: 100, y: 100 },
                value: 7,
            },
            style: AnnotationStyle::default(),
        };

        assert_eq!(
            marker.translated(10, -20).kind,
            AnnotationKind::Number {
                center: PhysicalPoint { x: 110, y: 80 },
                value: 7,
            }
        );
        assert_eq!(
            marker
                .resized(PhysicalRect {
                    left: 200,
                    top: 300,
                    right: 260,
                    bottom: 360,
                })
                .kind,
            AnnotationKind::Number {
                center: PhysicalPoint { x: 230, y: 330 },
                value: 7,
            }
        );
    }

    #[test]
    fn text_bounds_follow_the_explicit_font_size_not_the_stroke_width() {
        let text = Annotation {
            id: AnnotationId::new(92),
            kind: AnnotationKind::Text {
                origin: PhysicalPoint { x: 10, y: 20 },
                content: "Hi".to_owned(),
            },
            style: AnnotationStyle {
                stroke_width: 10,
                text_font_size: 32,
                ..AnnotationStyle::default()
            },
        };

        assert_eq!(text.text_font_size(), 32);
        assert_eq!(
            text.bounds(),
            PhysicalRect {
                left: 10,
                top: 20,
                right: 54,
                bottom: 56,
            }
        );
    }

    #[test]
    fn legacy_annotation_styles_default_to_the_original_text_size() {
        let document = AnnotationDocument::from_json(
            r#"{
                "version": 1,
                "canvas_bounds": {"left": 0, "top": 0, "right": 100, "bottom": 100},
                "annotations": [{
                    "id": 83,
                    "kind": {"Text": {"origin": {"x": 4, "y": 8}, "content": "Note"}},
                    "style": {"stroke_rgba": 4282071295, "fill_rgba": null, "stroke_width": 10}
                }]
            }"#,
        )
        .unwrap();

        assert_eq!(
            document
                .annotation(AnnotationId::new(83))
                .unwrap()
                .text_font_size(),
            DEFAULT_TEXT_FONT_SIZE
        );
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
    fn document_json_round_trips_version_canvas_and_annotations() {
        let mut document = AnnotationDocument::new(canvas()).unwrap();
        let annotation = rectangle(
            42,
            PhysicalRect {
                left: 20,
                top: 30,
                right: 120,
                bottom: 130,
            },
        );
        let mut history = CommandHistory::default();
        history
            .apply(&mut document, AnnotationCommand::Insert(annotation))
            .unwrap();

        let json = document.to_json().unwrap();
        assert!(json.contains("\"version\":1"));
        assert_eq!(AnnotationDocument::from_json(&json).unwrap(), document);
    }

    #[test]
    fn document_json_rejects_unknown_versions_and_outside_geometry() {
        let unknown_version = r#"{
            "version": 2,
            "canvas_bounds": {"left": 0, "top": 0, "right": 100, "bottom": 100},
            "annotations": []
        }"#;
        assert_eq!(
            AnnotationDocument::from_json(unknown_version),
            Err(AnnotationError::UnsupportedVersion(2))
        );

        let outside_line = r#"{
            "version": 1,
            "canvas_bounds": {"left": 0, "top": 0, "right": 100, "bottom": 100},
            "annotations": [{
                "id": 42,
                "kind": {"Line": {"start": {"x": 10, "y": 10}, "end": {"x": 120, "y": 10}}},
                "style": {"stroke_rgba": 4282071295, "fill_rgba": null, "stroke_width": 4}
            }]
        }"#;
        assert_eq!(
            AnnotationDocument::from_json(outside_line),
            Err(AnnotationError::AnnotationOutsideCanvas(AnnotationId::new(
                42
            )))
        );
    }

    #[test]
    fn document_rebase_preserves_geometry_relative_to_the_canvas() {
        let source_canvas = PhysicalRect {
            left: -1920,
            top: 100,
            right: -1820,
            bottom: 200,
        };
        let target_canvas = PhysicalRect {
            left: 0,
            top: 0,
            right: 100,
            bottom: 100,
        };
        let mut document = AnnotationDocument::new(source_canvas).unwrap();
        let mut history = CommandHistory::default();
        history
            .apply(
                &mut document,
                AnnotationCommand::Insert(rectangle(
                    42,
                    PhysicalRect {
                        left: -1900,
                        top: 120,
                        right: -1880,
                        bottom: 140,
                    },
                )),
            )
            .unwrap();

        let rebased = document.rebased_to(target_canvas).unwrap();
        assert_eq!(rebased.canvas_bounds(), target_canvas);
        assert_eq!(
            rebased.annotation(AnnotationId::new(42)).unwrap().bounds(),
            PhysicalRect {
                left: 20,
                top: 20,
                right: 40,
                bottom: 40,
            }
        );
        assert_eq!(
            document.rebased_to(PhysicalRect {
                left: 0,
                top: 0,
                right: 101,
                bottom: 100,
            }),
            Err(AnnotationError::IncompatibleCanvasBounds)
        );
    }

    #[test]
    fn duplicate_offsets_a_copy_and_preserves_the_original() {
        let original = rectangle(
            42,
            PhysicalRect {
                left: 10,
                top: 20,
                right: 120,
                bottom: 130,
            },
        );
        let canvas = PhysicalRect {
            left: 0,
            top: 0,
            right: 640,
            bottom: 480,
        };

        let duplicate = original.duplicated(AnnotationId::new(43), canvas, 12);

        assert_eq!(duplicate.id, AnnotationId::new(43));
        assert_eq!(original.id, AnnotationId::new(42));
        assert_eq!(
            duplicate.bounds(),
            PhysicalRect {
                left: 22,
                top: 32,
                right: 132,
                bottom: 142,
            }
        );
    }

    #[test]
    fn duplicate_clamps_the_offset_at_the_canvas_edge() {
        let annotation = Annotation {
            id: AnnotationId::new(42),
            kind: AnnotationKind::Rectangle {
                bounds: PhysicalRect {
                    left: 500,
                    top: 300,
                    right: 640,
                    bottom: 480,
                },
            },
            style: AnnotationStyle::default(),
        };
        let canvas = PhysicalRect {
            left: 0,
            top: 0,
            right: 640,
            bottom: 480,
        };

        let duplicate = annotation.duplicated(AnnotationId::new(43), canvas, 12);

        assert_eq!(duplicate.bounds(), annotation.bounds());
    }

    #[test]
    fn clockwise_rotation_preserves_line_length_and_is_reversible() {
        let canvas = PhysicalRect {
            left: 0,
            top: 0,
            right: 640,
            bottom: 480,
        };
        let original = Annotation {
            id: AnnotationId::new(42),
            kind: AnnotationKind::Line {
                start: PhysicalPoint { x: 100, y: 120 },
                end: PhysicalPoint { x: 200, y: 120 },
            },
            style: AnnotationStyle::default(),
        };

        let rotated = original.rotated_clockwise_within(canvas).unwrap();
        assert_eq!(
            rotated.kind,
            AnnotationKind::Line {
                start: PhysicalPoint { x: 150, y: 170 },
                end: PhysicalPoint { x: 150, y: 70 },
            }
        );

        let mut document = AnnotationDocument::new(canvas).unwrap();
        let mut history = CommandHistory::default();
        history
            .apply(&mut document, AnnotationCommand::Insert(original.clone()))
            .unwrap();
        history
            .apply(&mut document, AnnotationCommand::Replace(rotated.clone()))
            .unwrap();
        assert!(history.undo(&mut document).unwrap());
        assert_eq!(document.annotation(original.id), Some(&original));
        assert!(history.redo(&mut document).unwrap());
        assert_eq!(document.annotation(original.id), Some(&rotated));
    }

    #[test]
    fn clockwise_rotation_swaps_rect_dimensions_and_rejects_text() {
        let canvas = PhysicalRect {
            left: 0,
            top: 0,
            right: 640,
            bottom: 480,
        };
        let rectangle = rectangle(
            42,
            PhysicalRect {
                left: 100,
                top: 100,
                right: 160,
                bottom: 200,
            },
        );
        let rotated = rectangle.rotated_clockwise_within(canvas).unwrap();
        assert_eq!(rotated.bounds().width(), 100);
        assert_eq!(rotated.bounds().height(), 60);

        let text = Annotation {
            id: AnnotationId::new(43),
            kind: AnnotationKind::Text {
                origin: PhysicalPoint { x: 20, y: 20 },
                content: "Hello".to_owned(),
            },
            style: AnnotationStyle::default(),
        };
        assert_eq!(text.rotated_clockwise_within(canvas), None);
    }

    #[test]
    fn fill_capability_is_limited_to_closed_shape_tools() {
        assert!(AnnotationTool::Rectangle.supports_fill());
        assert!(AnnotationTool::Ellipse.supports_fill());
        assert!(!AnnotationTool::Arrow.supports_fill());

        let rectangle = rectangle(
            42,
            PhysicalRect {
                left: 0,
                top: 0,
                right: 10,
                bottom: 10,
            },
        );
        assert!(rectangle.supports_fill());
        let line = Annotation {
            id: AnnotationId::new(43),
            kind: AnnotationKind::Line {
                start: PhysicalPoint { x: 0, y: 0 },
                end: PhysicalPoint { x: 10, y: 10 },
            },
            style: AnnotationStyle::default(),
        };
        assert!(!line.supports_fill());
    }

    #[test]
    fn keyboard_translation_preserves_size_and_clamps_to_canvas() {
        let annotation = rectangle(
            42,
            PhysicalRect {
                left: 500,
                top: 300,
                right: 640,
                bottom: 480,
            },
        );
        let canvas = PhysicalRect {
            left: 0,
            top: 0,
            right: 640,
            bottom: 480,
        };

        let moved = annotation.translated_within(canvas, -10, -20);
        let clamped = annotation.translated_within(canvas, 10, 20);

        assert_eq!(
            moved.bounds(),
            PhysicalRect {
                left: 490,
                top: 280,
                right: 630,
                bottom: 460,
            }
        );
        assert_eq!(clamped.bounds(), annotation.bounds());
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
                    text_font_size: 24,
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
