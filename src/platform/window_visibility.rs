//! Native visibility control for the GPUI-owned main window.

use std::io;

const PIN_WINDOW_MIN_WIDTH: i32 = 180;
const PIN_WINDOW_MIN_HEIGHT: i32 = 140;
const PIN_WINDOW_MAX_WIDTH: i32 = 3_840;
const PIN_WINDOW_MAX_HEIGHT: i32 = 2_160;

/// Computes a bounded pixel dimension for a relative Pin-window zoom operation.
fn scaled_extent(extent: i32, scale: f32, minimum: i32, maximum: i32) -> i32 {
    let extent = extent.max(1);
    let scaled = if scale.is_finite() && scale > 0.0 {
        (extent as f32 * scale).round() as i32
    } else {
        extent
    };
    scaled.clamp(minimum, maximum)
}

pub fn hide(handle: isize) -> io::Result<()> {
    platform::hide(handle)
}

pub fn restore(handle: isize) -> io::Result<()> {
    platform::restore(handle)
}

pub fn make_topmost(handle: isize) -> io::Result<()> {
    platform::make_topmost(handle)
}

/// Resizes a native window around its center while preserving its current aspect ratio.
pub fn resize_centered(handle: isize, scale: f32) -> io::Result<()> {
    platform::resize_centered(handle, scale)
}

#[cfg(windows)]
mod platform {
    use super::{
        PIN_WINDOW_MAX_HEIGHT, PIN_WINDOW_MAX_WIDTH, PIN_WINDOW_MIN_HEIGHT, PIN_WINDOW_MIN_WIDTH,
        scaled_extent,
    };
    use std::{ffi::c_void, io};
    use windows_sys::Win32::{
        Foundation::RECT,
        UI::WindowsAndMessaging::{
            GetWindowRect, HWND_TOPMOST, IsWindow, SW_HIDE, SW_RESTORE, SWP_NOACTIVATE, SWP_NOMOVE,
            SWP_NOSIZE, SWP_NOZORDER, SetForegroundWindow, SetWindowPos, ShowWindow,
        },
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

    pub fn make_topmost(handle: isize) -> io::Result<()> {
        let window = window(handle)?;
        // SAFETY: window is live and the call only changes its z-order.
        if unsafe { SetWindowPos(window, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn resize_centered(handle: isize, scale: f32) -> io::Result<()> {
        let window = window(handle)?;
        let mut rect = RECT::default();
        // SAFETY: window is a live HWND borrowed from GPUI and rect is valid writable storage.
        if unsafe { GetWindowRect(window, &mut rect) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let width = rect.right.saturating_sub(rect.left).max(1);
        let height = rect.bottom.saturating_sub(rect.top).max(1);
        let target_width = scaled_extent(width, scale, PIN_WINDOW_MIN_WIDTH, PIN_WINDOW_MAX_WIDTH);
        let target_height =
            scaled_extent(height, scale, PIN_WINDOW_MIN_HEIGHT, PIN_WINDOW_MAX_HEIGHT);
        let left = rect.left + (width - target_width) / 2;
        let top = rect.top + (height - target_height) / 2;

        // Preserve z-order and focus: Pin zoom should feel like changing the image size,
        // not like opening or moving a different window.
        if unsafe {
            SetWindowPos(
                window,
                std::ptr::null_mut(),
                left,
                top,
                target_width,
                target_height,
                SWP_NOACTIVATE | SWP_NOZORDER,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
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

    pub fn make_topmost(_handle: isize) -> io::Result<()> {
        Ok(())
    }

    pub fn resize_centered(_handle: isize, _scale: f32) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{PIN_WINDOW_MAX_WIDTH, PIN_WINDOW_MIN_WIDTH, scaled_extent};

    #[test]
    fn pin_window_zoom_scales_and_clamps_the_window_extent() {
        assert_eq!(
            scaled_extent(400, 1.25, PIN_WINDOW_MIN_WIDTH, PIN_WINDOW_MAX_WIDTH),
            500
        );
        assert_eq!(
            scaled_extent(100, 0.8, PIN_WINDOW_MIN_WIDTH, PIN_WINDOW_MAX_WIDTH),
            180
        );
        assert_eq!(
            scaled_extent(3_500, 1.25, PIN_WINDOW_MIN_WIDTH, PIN_WINDOW_MAX_WIDTH),
            PIN_WINDOW_MAX_WIDTH
        );
    }

    #[test]
    fn invalid_pin_window_zoom_factor_keeps_the_current_extent() {
        assert_eq!(
            scaled_extent(400, 0.0, PIN_WINDOW_MIN_WIDTH, PIN_WINDOW_MAX_WIDTH),
            400
        );
        assert_eq!(
            scaled_extent(400, f32::NAN, PIN_WINDOW_MIN_WIDTH, PIN_WINDOW_MAX_WIDTH),
            400
        );
    }
}
