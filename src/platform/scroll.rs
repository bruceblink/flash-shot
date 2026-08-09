//! Explicit user-triggered wheel input for assisted scroll capture.

use std::io;

use crate::domain::geometry::PhysicalPoint;

pub const DEFAULT_SCROLL_NOTCHES: i32 = -3;

/// Temporarily moves the cursor to `target`, injects bounded vertical wheel input, and restores
/// the user's original cursor position before returning.
///
/// This is intentionally invoked only by an explicit control in the manual scroll workflow.
pub fn scroll_notches_at(target: PhysicalPoint, notches: i32) -> io::Result<()> {
    if notches == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "scroll notch count must not be zero",
        ));
    }
    platform::scroll_notches_at(target, notches)
}

#[cfg(windows)]
mod platform {
    use super::PhysicalPoint;
    use std::{io, mem::size_of};
    use windows_sys::Win32::{
        Foundation::POINT,
        UI::{
            Input::KeyboardAndMouse::{
                INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_WHEEL, MOUSEINPUT, SendInput,
            },
            WindowsAndMessaging::{GetCursorPos, SetCursorPos},
        },
    };

    const WHEEL_DELTA: i32 = 120;

    pub fn scroll_notches_at(target: PhysicalPoint, notches: i32) -> io::Result<()> {
        let mouse_data = notches
            .checked_mul(WHEEL_DELTA)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "scroll amount overflow"))?
            as u32;
        let mut original = POINT { x: 0, y: 0 };
        // SAFETY: `original` is a valid out parameter for the synchronous Windows API call.
        if unsafe { GetCursorPos(&mut original) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the coordinates are physical virtual-desktop pixels accepted by SetCursorPos.
        if unsafe { SetCursorPos(target.x, target.y) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    mouseData: mouse_data,
                    dwFlags: MOUSEEVENTF_WHEEL,
                    ..Default::default()
                },
            },
        };
        // SAFETY: input is initialized as a MOUSEINPUT and remains valid for this synchronous call.
        let input_result = if unsafe { SendInput(1, &input, size_of::<INPUT>() as i32) } == 1 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        };
        // Restore the pointer even when wheel injection fails, so this helper never leaves a
        // hidden cursor side effect behind the scrolling workflow.
        // SAFETY: the saved coordinates came from GetCursorPos and remain valid here.
        let restore_result = if unsafe { SetCursorPos(original.x, original.y) } == 1 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        };
        match (input_result, restore_result) {
            (Err(error), _) => Err(error),
            (Ok(()), Ok(())) => Ok(()),
            (Ok(()), Err(error)) => Err(io::Error::other(format!(
                "scroll input succeeded but cursor restore failed: {error}"
            ))),
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::PhysicalPoint;
    use std::io;

    pub fn scroll_notches_at(_target: PhysicalPoint, _notches: i32) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "assisted scrolling is currently Windows-only",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{PhysicalPoint, scroll_notches_at};

    #[test]
    fn zero_notches_are_rejected_without_injecting_input() {
        let error = scroll_notches_at(PhysicalPoint { x: 0, y: 0 }, 0).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }
}
