//! Immutable capture frames and the platform capture boundary.

use std::{io, sync::Arc, time::Duration};

use crate::domain::geometry::{PhysicalPoint, PhysicalRect};
use crate::platform::display::{DisplayProvider, SystemDisplayProvider, virtual_desktop_bounds};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PixelFormat {
    Bgra8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PixelColor {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
}

impl PixelColor {
    pub fn hex_rgb(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.red, self.green, self.blue)
    }

    pub const fn rgba_u32(self) -> u32 {
        u32::from_be_bytes([self.red, self.green, self.blue, self.alpha])
    }
}

#[derive(Clone, Debug)]
pub struct CaptureFrame {
    pub bounds: PhysicalRect,
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub format: PixelFormat,
    pub pixels: Arc<[u8]>,
    pub capture_duration: Duration,
    pub cpu_copy_count: u32,
}

impl CaptureFrame {
    pub fn validate(&self) -> io::Result<()> {
        let required = self
            .stride
            .checked_mul(self.height as usize)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "frame size overflow"))?;
        if self.width == 0 || self.height == 0 || self.stride < self.width as usize * 4 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid frame dimensions",
            ));
        }
        if self.pixels.len() != required {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "pixel buffer length does not match frame geometry",
            ));
        }
        Ok(())
    }

    pub fn pixel_at(&self, point: PhysicalPoint) -> Option<PixelColor> {
        if self.format != PixelFormat::Bgra8 {
            return None;
        }
        let local = self.bounds.translate_to_local(point)?;
        let offset = (local.y as usize)
            .checked_mul(self.stride)?
            .checked_add(local.x as usize * 4)?;
        let pixel = self.pixels.get(offset..offset + 4)?;
        Some(PixelColor {
            red: pixel[2],
            green: pixel[1],
            blue: pixel[0],
            alpha: pixel[3],
        })
    }
}

pub trait CaptureBackend {
    fn capture(&self, bounds: PhysicalRect) -> io::Result<CaptureFrame>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemCaptureBackend;

impl CaptureBackend for SystemCaptureBackend {
    fn capture(&self, bounds: PhysicalRect) -> io::Result<CaptureFrame> {
        platform::capture(bounds)
    }
}

#[derive(Clone, Debug)]
pub struct VirtualDesktopCapture {
    pub frame: CaptureFrame,
    pub display_count: usize,
}

pub fn capture_virtual_desktop() -> io::Result<VirtualDesktopCapture> {
    let displays = SystemDisplayProvider.displays()?;
    let bounds = virtual_desktop_bounds(&displays)?;
    let frame = SystemCaptureBackend.capture(bounds)?;
    Ok(VirtualDesktopCapture {
        frame,
        display_count: displays.len(),
    })
}

#[cfg(windows)]
mod platform {
    use super::{CaptureFrame, PixelFormat};
    use crate::domain::geometry::PhysicalRect;
    use std::{io, mem::size_of, ptr, sync::Arc, time::Instant};
    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap,
        CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, HBITMAP, HDC,
        HGDIOBJ, ReleaseDC, SRCCOPY, SelectObject,
    };

    pub fn capture(bounds: PhysicalRect) -> io::Result<CaptureFrame> {
        let width = bounds.width();
        let height = bounds.height();
        if width == 0 || height == 0 || width > i32::MAX as u32 || height > i32::MAX as u32 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "capture bounds must have supported non-zero dimensions",
            ));
        }

        let started_at = Instant::now();
        // SAFETY: a null HWND requests the virtual desktop DC and is released by ScreenDc.
        let screen_dc = ScreenDc::acquire()?;
        let memory_dc = CompatibleDc::create(screen_dc.0)?;
        let bitmap = Bitmap::create(screen_dc.0, width as i32, height as i32)?;
        let selection = BitmapSelection::select(memory_dc.0, bitmap.0)?;

        // SAFETY: both DCs and the selected bitmap are valid for the requested dimensions.
        if unsafe {
            BitBlt(
                memory_dc.0,
                0,
                0,
                width as i32,
                height as i32,
                screen_dc.0,
                bounds.left,
                bounds.top,
                SRCCOPY | CAPTUREBLT,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }

        let stride = width as usize * 4;
        let length = stride
            .checked_mul(height as usize)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "capture size overflow"))?;
        let mut pixels = vec![0_u8; length];
        let mut bitmap_info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                biSizeImage: length as u32,
                ..Default::default()
            },
            ..Default::default()
        };
        drop(selection);

        // SAFETY: the bitmap is no longer selected, and the output buffer matches the DIB.
        let rows = unsafe {
            GetDIBits(
                memory_dc.0,
                bitmap.0,
                0,
                height,
                pixels.as_mut_ptr().cast(),
                &mut bitmap_info,
                DIB_RGB_COLORS,
            )
        };
        if rows != height as i32 {
            return Err(io::Error::last_os_error());
        }

        let frame = CaptureFrame {
            bounds,
            width,
            height,
            stride,
            format: PixelFormat::Bgra8,
            pixels: Arc::from(pixels),
            capture_duration: started_at.elapsed(),
            cpu_copy_count: 1,
        };
        frame.validate()?;
        Ok(frame)
    }

    struct ScreenDc(HDC);

    impl ScreenDc {
        fn acquire() -> io::Result<Self> {
            // SAFETY: a null HWND requests the desktop DC.
            let dc = unsafe { GetDC(ptr::null_mut()) };
            if dc.is_null() {
                Err(io::Error::last_os_error())
            } else {
                Ok(Self(dc))
            }
        }
    }

    impl Drop for ScreenDc {
        fn drop(&mut self) {
            // SAFETY: this DC was obtained by GetDC with a null HWND.
            unsafe { ReleaseDC(ptr::null_mut(), self.0) };
        }
    }

    struct CompatibleDc(HDC);

    impl CompatibleDc {
        fn create(source: HDC) -> io::Result<Self> {
            // SAFETY: source is a valid screen DC.
            let dc = unsafe { CreateCompatibleDC(source) };
            if dc.is_null() {
                Err(io::Error::last_os_error())
            } else {
                Ok(Self(dc))
            }
        }
    }

    impl Drop for CompatibleDc {
        fn drop(&mut self) {
            // SAFETY: this memory DC is owned by the wrapper.
            unsafe { DeleteDC(self.0) };
        }
    }

    struct Bitmap(HBITMAP);

    impl Bitmap {
        fn create(source: HDC, width: i32, height: i32) -> io::Result<Self> {
            // SAFETY: source is valid and dimensions were validated.
            let bitmap = unsafe { CreateCompatibleBitmap(source, width, height) };
            if bitmap.is_null() {
                Err(io::Error::last_os_error())
            } else {
                Ok(Self(bitmap))
            }
        }
    }

    impl Drop for Bitmap {
        fn drop(&mut self) {
            // SAFETY: selection guard is dropped before this owned bitmap.
            unsafe { DeleteObject(self.0 as HGDIOBJ) };
        }
    }

    struct BitmapSelection {
        dc: HDC,
        previous: HGDIOBJ,
    }

    impl BitmapSelection {
        fn select(dc: HDC, bitmap: HBITMAP) -> io::Result<Self> {
            // SAFETY: both handles are valid and compatible.
            let previous = unsafe { SelectObject(dc, bitmap as HGDIOBJ) };
            if previous.is_null() {
                Err(io::Error::last_os_error())
            } else {
                Ok(Self { dc, previous })
            }
        }
    }

    impl Drop for BitmapSelection {
        fn drop(&mut self) {
            // SAFETY: restores the object returned by SelectObject into the same DC.
            unsafe { SelectObject(self.dc, self.previous) };
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::CaptureFrame;
    use crate::domain::geometry::PhysicalRect;
    use std::io;

    pub fn capture(_bounds: PhysicalRect) -> io::Result<CaptureFrame> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "screen capture is currently Windows-only",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{CaptureBackend, CaptureFrame, PixelColor, PixelFormat, SystemCaptureBackend};
    use crate::domain::geometry::{PhysicalPoint, PhysicalRect};
    #[cfg(windows)]
    use crate::platform::display::{
        DisplayProvider, SystemDisplayProvider, virtual_desktop_bounds,
    };
    use std::{sync::Arc, time::Duration};

    #[cfg(windows)]
    #[test]
    fn captures_an_immutable_virtual_desktop_frame() {
        let displays = SystemDisplayProvider.displays().unwrap();
        let bounds = virtual_desktop_bounds(&displays).unwrap();

        let frame = SystemCaptureBackend.capture(bounds).unwrap();

        assert_eq!(frame.bounds, bounds);
        assert_eq!(frame.width, bounds.width());
        assert_eq!(frame.height, bounds.height());
        assert_eq!(frame.pixels.len(), frame.stride * frame.height as usize);
        assert_eq!(frame.cpu_copy_count, 1);
        assert!(frame.pixels.iter().any(|value| *value != 0));
    }

    #[test]
    fn samples_bgra_pixels_by_virtual_desktop_coordinate() {
        let frame = CaptureFrame {
            bounds: PhysicalRect {
                left: -2,
                top: 10,
                right: 0,
                bottom: 11,
            },
            width: 2,
            height: 1,
            stride: 8,
            format: PixelFormat::Bgra8,
            pixels: Arc::from([0x33, 0x22, 0x11, 0xFF, 0xCC, 0xBB, 0xAA, 0x80]),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 1,
        };

        assert_eq!(
            frame.pixel_at(PhysicalPoint { x: -1, y: 10 }),
            Some(PixelColor {
                red: 0xAA,
                green: 0xBB,
                blue: 0xCC,
                alpha: 0x80,
            })
        );
        assert_eq!(frame.pixel_at(PhysicalPoint { x: 0, y: 10 }), None);
    }

    #[test]
    fn formats_sampled_colors_for_display_and_gpui() {
        let color = PixelColor {
            red: 0x12,
            green: 0xAB,
            blue: 0x05,
            alpha: 0xFF,
        };

        assert_eq!(color.hex_rgb(), "#12AB05");
        assert_eq!(color.rgba_u32(), 0x12AB05FF);
    }
}
