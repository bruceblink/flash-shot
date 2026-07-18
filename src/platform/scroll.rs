//! Explicit user-triggered wheel input for assisted scroll capture.

use std::io;

use crate::domain::geometry::PhysicalPoint;

pub const DEFAULT_SCROLL_NOTCHES: i32 = -3;

/// Moves the cursor to `target` and injects a bounded number of vertical wheel notches.
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
    use windows_sys::Win32::UI::{
        Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_WHEEL, MOUSEINPUT, SendInput,
        },
        WindowsAndMessaging::SetCursorPos,
    };

    const WHEEL_DELTA: i32 = 120;

    pub fn scroll_notches_at(target: PhysicalPoint, notches: i32) -> io::Result<()> {
        // SAFETY: the coordinates are physical virtual-desktop pixels accepted by SetCursorPos.
        if unsafe { SetCursorPos(target.x, target.y) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let mouse_data = notches
            .checked_mul(WHEEL_DELTA)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "scroll amount overflow"))?
            as u32;
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
        if unsafe { SendInput(1, &input, size_of::<INPUT>() as i32) } != 1 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
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
