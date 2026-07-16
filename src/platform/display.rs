//! Physical display enumeration used by the capture pipeline.

use std::io;

use crate::domain::geometry::PhysicalRect;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisplayRotation {
    Landscape,
    Portrait,
    LandscapeFlipped,
    PortraitFlipped,
    Unknown,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DisplayInfo {
    pub id: String,
    pub physical_bounds: PhysicalRect,
    pub work_area: PhysicalRect,
    pub dpi_x: u32,
    pub dpi_y: u32,
    pub scale_factor: f32,
    pub rotation: DisplayRotation,
    pub bits_per_pixel: u32,
    pub primary: bool,
}

pub trait DisplayProvider {
    fn displays(&self) -> io::Result<Vec<DisplayInfo>>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemDisplayProvider;

impl DisplayProvider for SystemDisplayProvider {
    fn displays(&self) -> io::Result<Vec<DisplayInfo>> {
        platform::enumerate_displays()
    }
}

#[cfg(windows)]
mod platform {
    use super::{DisplayInfo, DisplayRotation};
    use crate::domain::geometry::PhysicalRect;
    use std::{io, mem::size_of, ptr};
    use windows_sys::Win32::{
        Foundation::{LPARAM, RECT},
        Graphics::Gdi::{
            DEVMODEW, DMDO_90, DMDO_180, DMDO_270, DMDO_DEFAULT, ENUM_CURRENT_SETTINGS,
            EnumDisplayMonitors, EnumDisplaySettingsW, GetMonitorInfoW, HDC, HMONITOR,
            MONITORINFOEXW,
        },
        UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
    };

    pub fn enumerate_displays() -> io::Result<Vec<DisplayInfo>> {
        let mut displays = Vec::new();
        // SAFETY: the callback writes only through the valid vector pointer for this call.
        let succeeded = unsafe {
            EnumDisplayMonitors(
                ptr::null_mut(),
                ptr::null(),
                Some(collect_monitor),
                (&mut displays as *mut Vec<DisplayInfo>) as LPARAM,
            )
        };
        if succeeded == 0 {
            return Err(io::Error::last_os_error());
        }
        if displays.is_empty() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no displays found"));
        }
        Ok(displays)
    }

    unsafe extern "system" fn collect_monitor(
        monitor: HMONITOR,
        _device_context: HDC,
        _bounds: *mut RECT,
        data: LPARAM,
    ) -> i32 {
        // SAFETY: EnumDisplayMonitors passes back the pointer supplied by enumerate_displays.
        let displays = unsafe { &mut *(data as *mut Vec<DisplayInfo>) };
        match display_info(monitor) {
            Ok(display) => {
                displays.push(display);
                1
            }
            Err(error) => {
                log::warn!(target: "flash_shot::display", "display_enumeration_failed error={error}");
                1
            }
        }
    }

    fn display_info(monitor: HMONITOR) -> io::Result<DisplayInfo> {
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        // SAFETY: info has the required size and remains valid for the call.
        if unsafe { GetMonitorInfoW(monitor, &mut info.monitorInfo) } == 0 {
            return Err(io::Error::last_os_error());
        }

        let id = wide_string(&info.szDevice);
        let (dpi_x, dpi_y) = monitor_dpi(monitor);
        let mut mode = DEVMODEW {
            dmSize: size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        // SAFETY: the device name is NUL terminated and mode declares its structure size.
        let has_mode = unsafe {
            EnumDisplaySettingsW(info.szDevice.as_ptr(), ENUM_CURRENT_SETTINGS, &mut mode)
        } != 0;

        Ok(DisplayInfo {
            id,
            physical_bounds: rect(info.monitorInfo.rcMonitor),
            work_area: rect(info.monitorInfo.rcWork),
            dpi_x,
            dpi_y,
            scale_factor: dpi_x as f32 / 96.0,
            rotation: if has_mode {
                // SAFETY: EnumDisplaySettingsW populated the display variant of this union.
                rotation(unsafe { mode.Anonymous1.Anonymous2.dmDisplayOrientation })
            } else {
                DisplayRotation::Unknown
            },
            bits_per_pixel: if has_mode { mode.dmBitsPerPel } else { 0 },
            primary: info.monitorInfo.dwFlags & 1 != 0,
        })
    }

    fn monitor_dpi(monitor: HMONITOR) -> (u32, u32) {
        let (mut x, mut y) = (96, 96);
        // SAFETY: x and y are valid out parameters. Failure keeps the 96-DPI fallback.
        if unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y) } < 0 {
            (96, 96)
        } else {
            (x, y)
        }
    }

    const fn rect(value: RECT) -> PhysicalRect {
        PhysicalRect {
            left: value.left,
            top: value.top,
            right: value.right,
            bottom: value.bottom,
        }
    }

    fn wide_string(value: &[u16]) -> String {
        let length = value
            .iter()
            .position(|character| *character == 0)
            .unwrap_or(value.len());
        String::from_utf16_lossy(&value[..length])
    }

    const fn rotation(value: u32) -> DisplayRotation {
        match value {
            DMDO_DEFAULT => DisplayRotation::Landscape,
            DMDO_90 => DisplayRotation::Portrait,
            DMDO_180 => DisplayRotation::LandscapeFlipped,
            DMDO_270 => DisplayRotation::PortraitFlipped,
            _ => DisplayRotation::Unknown,
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::DisplayInfo;
    use std::io;

    pub fn enumerate_displays() -> io::Result<Vec<DisplayInfo>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "display enumeration is currently Windows-only",
        ))
    }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::{DisplayProvider, SystemDisplayProvider};

    #[cfg(windows)]
    #[test]
    fn system_has_at_least_one_valid_physical_display() {
        let displays = SystemDisplayProvider.displays().unwrap();

        assert!(!displays.is_empty());
        assert!(displays.iter().all(|display| {
            !display.id.is_empty()
                && display.physical_bounds.width() > 0
                && display.physical_bounds.height() > 0
                && display.dpi_x > 0
                && display.dpi_y > 0
        }));
        assert!(displays.iter().any(|display| display.primary));
    }
}
