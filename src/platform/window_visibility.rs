//! Native visibility control for the GPUI-owned main window.

use std::io;

const PIN_WINDOW_MIN_WIDTH: i32 = 180;
const PIN_WINDOW_MIN_HEIGHT: i32 = 140;
const PIN_WINDOW_MAX_WIDTH: i32 = 3_840;
const PIN_WINDOW_MAX_HEIGHT: i32 = 2_160;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeWindowCenter {
    x_twice: i64,
    y_twice: i64,
}

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

/// Shows a previously hidden window without changing the user's active window.
pub fn show(handle: isize) -> io::Result<()> {
    platform::show(handle)
}

pub fn make_topmost(handle: isize) -> io::Result<()> {
    platform::make_topmost(handle)
}

/// Computes the bounded logical content size that GPUI should apply for one Pin zoom step.
pub fn scaled_pin_size(width: f32, height: f32, scale: f32) -> (f32, f32) {
    (
        scaled_extent(
            width.round() as i32,
            scale,
            PIN_WINDOW_MIN_WIDTH,
            PIN_WINDOW_MAX_WIDTH,
        ) as f32,
        scaled_extent(
            height.round() as i32,
            scale,
            PIN_WINDOW_MIN_HEIGHT,
            PIN_WINDOW_MAX_HEIGHT,
        ) as f32,
    )
}

/// Snapshots the native outer-frame center before GPUI queues a content resize.
pub fn snapshot_window_center(handle: isize) -> io::Result<NativeWindowCenter> {
    platform::snapshot_window_center(handle)
}

/// Restores the saved center after GPUI has synchronized its renderer and native size.
pub fn recenter_window(handle: isize, center: NativeWindowCenter) -> io::Result<()> {
    platform::recenter_window(handle, center)
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
    use super::{NativeWindowCenter, centered_outer_origin};
    use std::{ffi::c_void, io};
    use windows_sys::Win32::{
        Foundation::RECT,
        UI::WindowsAndMessaging::{
            GWL_EXSTYLE, GetWindowLongPtrW, GetWindowRect, HWND_TOPMOST, IsWindow, LWA_ALPHA,
            SW_HIDE, SW_RESTORE, SW_SHOWNOACTIVATE, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOMOVE,
            SWP_NOSIZE, SWP_NOZORDER, SetForegroundWindow, SetLayeredWindowAttributes,
            SetWindowLongPtrW, SetWindowPos, ShowWindow, WS_EX_LAYERED, WS_EX_TRANSPARENT,
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

    pub fn show(handle: isize) -> io::Result<()> {
        let window = window(handle)?;
        // SAFETY: window is a live HWND. This restores visibility without requesting foreground.
        unsafe { ShowWindow(window, SW_SHOWNOACTIVATE) };
        Ok(())
    }

    pub fn make_topmost(handle: isize) -> io::Result<()> {
        let window = window(handle)?;
        // SAFETY: window is live and the call only changes its z-order. Dispatch it
        // asynchronously to avoid re-entering a GPUI render callback on Windows.
        if unsafe {
            SetWindowPos(
                window,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn snapshot_window_center(handle: isize) -> io::Result<NativeWindowCenter> {
        let window = window(handle)?;
        let mut rect = RECT::default();
        // SAFETY: window is live and rect is valid writable storage for its outer bounds.
        if unsafe { GetWindowRect(window, &mut rect) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(NativeWindowCenter {
            x_twice: i64::from(rect.left) + i64::from(rect.right),
            y_twice: i64::from(rect.top) + i64::from(rect.bottom),
        })
    }

    pub fn recenter_window(handle: isize, center: NativeWindowCenter) -> io::Result<()> {
        let window = window(handle)?;
        let mut rect = RECT::default();
        // SAFETY: window is live and rect is valid writable storage for its resized outer bounds.
        if unsafe { GetWindowRect(window, &mut rect) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let width = rect.right.saturating_sub(rect.left);
        let height = rect.bottom.saturating_sub(rect.top);
        let (left, top) = centered_outer_origin(center, width, height).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "resized Pin bounds are invalid")
        })?;
        // SAFETY: this task runs after GPUI's queued resize and only restores position.
        if unsafe {
            SetWindowPos(
                window,
                std::ptr::null_mut(),
                left,
                top,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
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
    use super::NativeWindowCenter;
    use std::io;

    pub fn hide(_handle: isize) -> io::Result<()> {
        Ok(())
    }

    pub fn restore(_handle: isize) -> io::Result<()> {
        Ok(())
    }

    pub fn show(_handle: isize) -> io::Result<()> {
        Ok(())
    }

    pub fn make_topmost(_handle: isize) -> io::Result<()> {
        Ok(())
    }

    pub fn snapshot_window_center(_handle: isize) -> io::Result<NativeWindowCenter> {
        Ok(NativeWindowCenter {
            x_twice: 0,
            y_twice: 0,
        })
    }

    pub fn recenter_window(_handle: isize, _center: NativeWindowCenter) -> io::Result<()> {
        Ok(())
    }

    pub fn set_opacity(_handle: isize, _opacity: u8) -> io::Result<()> {
        Ok(())
    }

    pub fn set_mouse_through(_handle: isize, _enabled: bool) -> io::Result<()> {
        Ok(())
    }
}

/// Converts a doubled center and resized outer extent into an origin without losing negative
/// half-pixel coordinates to truncation toward zero.
fn centered_outer_origin(
    center: NativeWindowCenter,
    width: i32,
    height: i32,
) -> Option<(i32, i32)> {
    if width <= 0 || height <= 0 {
        return None;
    }
    let left = (center.x_twice - i64::from(width)).div_euclid(2);
    let top = (center.y_twice - i64::from(height)).div_euclid(2);
    Some((i32::try_from(left).ok()?, i32::try_from(top).ok()?))
}

#[cfg(test)]
mod tests {
    use super::{
        NativeWindowCenter, PIN_WINDOW_MAX_WIDTH, PIN_WINDOW_MIN_WIDTH, centered_outer_origin,
        scaled_extent,
    };

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

    #[test]
    fn pin_recenter_handles_odd_extents_and_negative_display_coordinates() {
        let center = NativeWindowCenter {
            x_twice: -21,
            y_twice: 19,
        };

        assert_eq!(centered_outer_origin(center, 5, 4), Some((-13, 7)));
        assert_eq!(2 * -13 + 5, -21);
        assert_eq!(2 * 7 + 4, 18);
        assert_eq!(centered_outer_origin(center, 0, 4), None);
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
