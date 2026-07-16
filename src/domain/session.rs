//! Explicit capture-session lifecycle and transition validation.

use std::{error::Error, fmt};

use super::geometry::PhysicalRect;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureSessionState {
    Idle,
    Capturing,
    Selecting,
    Exporting,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureSession {
    state: CaptureSessionState,
    selection: Option<PhysicalRect>,
    failure: Option<String>,
}

impl Default for CaptureSession {
    fn default() -> Self {
        Self {
            state: CaptureSessionState::Idle,
            selection: None,
            failure: None,
        }
    }
}

impl CaptureSession {
    pub const fn state(&self) -> CaptureSessionState {
        self.state
    }

    pub const fn selection(&self) -> Option<PhysicalRect> {
        self.selection
    }

    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    pub fn begin(&mut self) -> Result<(), TransitionError> {
        self.require(CaptureSessionState::Idle, "begin")?;
        self.state = CaptureSessionState::Capturing;
        Ok(())
    }

    pub fn frames_ready(&mut self) -> Result<(), TransitionError> {
        self.require(CaptureSessionState::Capturing, "frames_ready")?;
        self.state = CaptureSessionState::Selecting;
        Ok(())
    }

    pub fn select(&mut self, selection: PhysicalRect) -> Result<(), TransitionError> {
        self.require(CaptureSessionState::Selecting, "select")?;
        if selection.width() == 0 || selection.height() == 0 {
            return Err(TransitionError::InvalidSelection);
        }
        self.selection = Some(selection);
        Ok(())
    }

    pub fn start_export(&mut self) -> Result<PhysicalRect, TransitionError> {
        self.require(CaptureSessionState::Selecting, "start_export")?;
        let selection = self.selection.ok_or(TransitionError::MissingSelection)?;
        self.state = CaptureSessionState::Exporting;
        Ok(selection)
    }

    pub fn export_completed(&mut self) -> Result<(), TransitionError> {
        self.require(CaptureSessionState::Exporting, "export_completed")?;
        self.state = CaptureSessionState::Completed;
        Ok(())
    }

    pub fn cancel(&mut self) -> Result<(), TransitionError> {
        if matches!(
            self.state,
            CaptureSessionState::Capturing | CaptureSessionState::Selecting
        ) {
            self.state = CaptureSessionState::Cancelled;
            self.selection = None;
            Ok(())
        } else {
            Err(TransitionError::InvalidTransition {
                state: self.state,
                operation: "cancel",
            })
        }
    }

    pub fn fail(&mut self, message: impl Into<String>) -> Result<(), TransitionError> {
        if matches!(
            self.state,
            CaptureSessionState::Capturing
                | CaptureSessionState::Selecting
                | CaptureSessionState::Exporting
        ) {
            self.state = CaptureSessionState::Failed;
            self.failure = Some(message.into());
            Ok(())
        } else {
            Err(TransitionError::InvalidTransition {
                state: self.state,
                operation: "fail",
            })
        }
    }

    pub fn reset(&mut self) -> Result<(), TransitionError> {
        if matches!(
            self.state,
            CaptureSessionState::Completed
                | CaptureSessionState::Cancelled
                | CaptureSessionState::Failed
        ) {
            *self = Self::default();
            Ok(())
        } else {
            Err(TransitionError::InvalidTransition {
                state: self.state,
                operation: "reset",
            })
        }
    }

    fn require(
        &self,
        expected: CaptureSessionState,
        operation: &'static str,
    ) -> Result<(), TransitionError> {
        if self.state == expected {
            Ok(())
        } else {
            Err(TransitionError::InvalidTransition {
                state: self.state,
                operation,
            })
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransitionError {
    InvalidTransition {
        state: CaptureSessionState,
        operation: &'static str,
    },
    InvalidSelection,
    MissingSelection,
}

impl fmt::Display for TransitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTransition { state, operation } => {
                write!(formatter, "cannot {operation} while session is {state:?}")
            }
            Self::InvalidSelection => formatter.write_str("selection must have non-zero size"),
            Self::MissingSelection => formatter.write_str("a selection is required before export"),
        }
    }
}

impl Error for TransitionError {}

#[cfg(test)]
mod tests {
    use super::{CaptureSession, CaptureSessionState, TransitionError};
    use crate::domain::geometry::PhysicalRect;

    fn selection() -> PhysicalRect {
        PhysicalRect {
            left: -500,
            top: 20,
            right: 300,
            bottom: 620,
        }
    }

    #[test]
    fn successful_capture_has_deterministic_lifecycle() {
        let mut session = CaptureSession::default();

        session.begin().unwrap();
        session.frames_ready().unwrap();
        session.select(selection()).unwrap();
        assert_eq!(session.start_export().unwrap(), selection());
        session.export_completed().unwrap();
        session.reset().unwrap();

        assert_eq!(session.state(), CaptureSessionState::Idle);
        assert_eq!(session.selection(), None);
    }

    #[test]
    fn repeated_begin_is_rejected_without_mutating_state() {
        let mut session = CaptureSession::default();
        session.begin().unwrap();

        assert!(matches!(
            session.begin(),
            Err(TransitionError::InvalidTransition {
                state: CaptureSessionState::Capturing,
                operation: "begin"
            })
        ));
        assert_eq!(session.state(), CaptureSessionState::Capturing);
    }

    #[test]
    fn cancellation_discards_selection_and_can_reset() {
        let mut session = CaptureSession::default();
        session.begin().unwrap();
        session.frames_ready().unwrap();
        session.select(selection()).unwrap();

        session.cancel().unwrap();
        assert_eq!(session.state(), CaptureSessionState::Cancelled);
        assert_eq!(session.selection(), None);
        session.reset().unwrap();
        assert_eq!(session.state(), CaptureSessionState::Idle);
    }

    #[test]
    fn failure_is_observable_and_reset_clears_it() {
        let mut session = CaptureSession::default();
        session.begin().unwrap();

        session.fail("access denied").unwrap();
        assert_eq!(session.state(), CaptureSessionState::Failed);
        assert_eq!(session.failure(), Some("access denied"));
        session.reset().unwrap();
        assert_eq!(session.failure(), None);
    }
}
