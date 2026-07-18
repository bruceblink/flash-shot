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
}

impl fmt::Display for AnnotationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCanvasBounds => formatter.write_str("annotation canvas must be non-empty"),
            Self::DuplicateId(id) => {
                write!(formatter, "annotation id {} already exists", id.value())
            }
            Self::MissingId(id) => write!(formatter, "annotation id {} does not exist", id.value()),
        }
    }
}

impl std::error::Error for AnnotationError {}

#[cfg(test)]
mod tests {
    use super::{
        ANNOTATION_DOCUMENT_VERSION, Annotation, AnnotationCommand, AnnotationDocument,
        AnnotationError, AnnotationId, AnnotationKind, AnnotationStyle, CommandHistory,
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
}
