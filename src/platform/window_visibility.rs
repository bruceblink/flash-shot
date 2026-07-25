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

/// Applies an alpha value to a native window without changing its z-order or focus.
pub fn set_opacity(handle: isize, opacity: u8) -> io::Result<()> {
    platform::set_opacity(handle, opacity)
}

/// Lets a pinned reference image receive no pointer input while retaining keyboard focus.
pub fn set_mouse_through(handle: isize, enabled: bool) -> io::Result<()> {
    platform::set_mouse_through(handle, enabled)
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
            GWL_EXSTYLE, GetWindowLongPtrW, GetWindowRect, HWND_TOPMOST, IsWindow, LWA_ALPHA,
            SW_HIDE, SW_RESTORE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
            SetForegroundWindow, SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos,
            ShowWindow, WS_EX_LAYERED, WS_EX_TRANSPARENT,
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

    pub fn set_opacity(handle: isize, opacity: u8) -> io::Result<()> {
        let window = window(handle)?;
        // SAFETY: window is a live HWND. This only adds the layered style needed for alpha.
        let extended_style = unsafe { GetWindowLongPtrW(window, GWL_EXSTYLE) };
        // SAFETY: window is a live HWND and the replacement retains every existing style bit.
        unsafe { SetWindowLongPtrW(window, GWL_EXSTYLE, extended_style | WS_EX_LAYERED as isize) };
        // SAFETY: window is live and opacity is a valid BYTE alpha value.
        if unsafe { SetLayeredWindowAttributes(window, 0, opacity, LWA_ALPHA) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn set_mouse_through(handle: isize, enabled: bool) -> io::Result<()> {
        let window = window(handle)?;
        let extended_style = unsafe { GetWindowLongPtrW(window, GWL_EXSTYLE) };
        let updated_style = mouse_through_style(extended_style, enabled);
        // SAFETY: window is live and the replacement preserves every unrelated style bit.
        unsafe { SetWindowLongPtrW(window, GWL_EXSTYLE, updated_style) };
        Ok(())
    }

    /// Adds or removes only the Windows hit-testing flag used for mouse-through reference windows.
    pub(super) fn mouse_through_style(style: isize, enabled: bool) -> isize {
        if enabled {
            style | WS_EX_TRANSPARENT as isize
        } else {
            style & !(WS_EX_TRANSPARENT as isize)
        }
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

    pub fn set_opacity(_handle: isize, _opacity: u8) -> io::Result<()> {
        Ok(())
    }

    pub fn set_mouse_through(_handle: isize, _enabled: bool) -> io::Result<()> {
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

    #[cfg(windows)]
    #[test]
    fn mouse_through_style_preserves_other_extended_window_flags() {
        use super::platform::mouse_through_style;
        use windows_sys::Win32::UI::WindowsAndMessaging::WS_EX_TRANSPARENT;

        let other_style = 0x1000isize;
        let enabled = mouse_through_style(other_style, true);
        assert_eq!(enabled & other_style, other_style);
        assert_ne!(enabled & WS_EX_TRANSPARENT as isize, 0);
        assert_eq!(mouse_through_style(enabled, false), other_style);
    }
}
