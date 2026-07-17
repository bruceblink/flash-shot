//! Global cursor position in virtual-desktop physical coordinates.

use std::io;

use crate::domain::geometry::PhysicalPoint;

pub fn position() -> io::Result<PhysicalPoint> {
    platform::position()
}

#[cfg(windows)]
mod platform {
    use super::PhysicalPoint;
    use std::io;
    use windows_sys::Win32::{Foundation::POINT, UI::WindowsAndMessaging::GetCursorPos};

    pub fn position() -> io::Result<PhysicalPoint> {
        let mut point = POINT::default();
        // SAFETY: point is a valid writable output parameter.
        if unsafe { GetCursorPos(&mut point) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(PhysicalPoint {
                x: point.x,
                y: point.y,
            })
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::PhysicalPoint;
    use std::io;

    pub fn position() -> io::Result<PhysicalPoint> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "global cursor position is currently Windows-only",
        ))
    }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn system_cursor_has_a_physical_position() {
        super::position().unwrap();
    }
}
