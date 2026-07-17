//! Native visibility control for the GPUI-owned main window.

use std::io;

pub fn hide(handle: isize) -> io::Result<()> {
    platform::hide(handle)
}

pub fn restore(handle: isize) -> io::Result<()> {
    platform::restore(handle)
}

#[cfg(windows)]
mod platform {
    use std::{ffi::c_void, io};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        IsWindow, SW_HIDE, SW_RESTORE, SetForegroundWindow, ShowWindow,
    };

    fn window(handle: isize) -> io::Result<*mut c_void> {
        let window = handle as *mut c_void;
        // SAFETY: this only queries whether the borrowed native handle is still a window.
        if unsafe { IsWindow(window) } == 0 {
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                "native window is unavailable",
            ))
        } else {
            Ok(window)
        }
    }

    pub fn hide(handle: isize) -> io::Result<()> {
        let window = window(handle)?;
        // SAFETY: window is a live HWND borrowed from GPUI.
        unsafe { ShowWindow(window, SW_HIDE) };
        Ok(())
    }

    pub fn restore(handle: isize) -> io::Result<()> {
        let window = window(handle)?;
        // SAFETY: window is a live HWND borrowed from GPUI.
        unsafe {
            ShowWindow(window, SW_RESTORE);
            SetForegroundWindow(window);
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod platform {
    use std::io;

    pub fn hide(_handle: isize) -> io::Result<()> {
        Ok(())
    }

    pub fn restore(_handle: isize) -> io::Result<()> {
        Ok(())
    }
}
