//! Immutable capture frames and the platform capture boundary.

use std::{io, sync::Arc, time::Duration};

use crate::domain::geometry::{PhysicalPoint, PhysicalRect};
use crate::platform::display::{
    DisplayInfo, DisplayProvider, SystemDisplayProvider, virtual_desktop_bounds,
};

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

#[derive(Clone, Debug)]
pub struct DisplayCapture {
    pub display: DisplayInfo,
    pub frame: CaptureFrame,
}

pub fn capture_displays() -> io::Result<Vec<DisplayCapture>> {
    capture_displays_with(&SystemDisplayProvider, &SystemCaptureBackend)
}

pub fn compose_virtual_desktop(captures: &[DisplayCapture]) -> io::Result<CaptureFrame> {
    let displays: Vec<_> = captures
        .iter()
        .map(|capture| capture.display.clone())
        .collect();
    let bounds = virtual_desktop_bounds(&displays)?;
    let width = bounds.width();
    let height = bounds.height();
    let stride = width as usize * 4;
    let length = stride
        .checked_mul(height as usize)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "frame size overflow"))?;
    let mut pixels = vec![0_u8; length];
    let mut capture_duration = Duration::ZERO;
    let mut cpu_copy_count = 0_u32;

    for capture in captures {
        capture.frame.validate()?;
        if capture.frame.bounds != capture.display.physical_bounds {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "captured frame bounds do not match the display",
            ));
        }
        let destination_x = capture.frame.bounds.left.saturating_sub(bounds.left) as usize * 4;
        let destination_y = capture.frame.bounds.top.saturating_sub(bounds.top) as usize;
        let row_bytes = capture.frame.width as usize * 4;
        for row in 0..capture.frame.height as usize {
            let source_start = row * capture.frame.stride;
            let destination_start = (destination_y + row) * stride + destination_x;
            pixels[destination_start..destination_start + row_bytes]
                .copy_from_slice(&capture.frame.pixels[source_start..source_start + row_bytes]);
        }
        capture_duration = capture_duration.max(capture.frame.capture_duration);
        cpu_copy_count = cpu_copy_count.saturating_add(capture.frame.cpu_copy_count);
    }

    let frame = CaptureFrame {
        bounds,
        width,
        height,
        stride,
        format: PixelFormat::Bgra8,
        pixels: Arc::from(pixels),
        capture_duration,
        cpu_copy_count: cpu_copy_count.saturating_add(1),
    };
    frame.validate()?;
    Ok(frame)
}

fn capture_displays_with(
    display_provider: &impl DisplayProvider,
    capture_backend: &impl CaptureBackend,
) -> io::Result<Vec<DisplayCapture>> {
    let mut displays = display_provider.displays()?;
    if displays.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "display capture requires at least one display",
        ));
    }
    displays.sort_by(|left, right| {
        left.physical_bounds
            .top
            .cmp(&right.physical_bounds.top)
            .then_with(|| left.physical_bounds.left.cmp(&right.physical_bounds.left))
            .then_with(|| left.id.cmp(&right.id))
    });

    displays
        .into_iter()
        .map(|display| {
            let frame = capture_backend.capture(display.physical_bounds)?;
            frame.validate()?;
            if frame.bounds != display.physical_bounds {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "captured frame bounds do not match the display",
                ));
            }
            Ok(DisplayCapture { display, frame })
        })
        .collect()
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
    use super::{
        CaptureBackend, CaptureFrame, PixelColor, PixelFormat, SystemCaptureBackend,
        capture_displays_with, compose_virtual_desktop,
    };
    use crate::domain::geometry::{PhysicalPoint, PhysicalRect};
    use crate::platform::display::{DisplayInfo, DisplayProvider, DisplayRotation};
    #[cfg(windows)]
    use crate::platform::display::{SystemDisplayProvider, virtual_desktop_bounds};
    use std::{io, sync::Arc, time::Duration};

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

    #[cfg(windows)]
    #[test]
    fn captures_one_immutable_frame_for_each_display() {
        let expected = SystemDisplayProvider.displays().unwrap();

        let captures = super::capture_displays().unwrap();

        assert_eq!(captures.len(), expected.len());
        assert!(captures.iter().all(|capture| {
            capture.frame.bounds == capture.display.physical_bounds
                && capture.frame.width == capture.display.physical_bounds.width()
                && capture.frame.height == capture.display.physical_bounds.height()
                && capture.frame.pixels.len()
                    == capture.frame.stride * capture.frame.height as usize
                && capture.frame.cpu_copy_count == 1
        }));
        assert!(captures.windows(2).all(|captures| {
            let left = &captures[0].display;
            let right = &captures[1].display;
            (
                left.physical_bounds.top,
                left.physical_bounds.left,
                &left.id,
            ) <= (
                right.physical_bounds.top,
                right.physical_bounds.left,
                &right.id,
            )
        }));
    }

    #[test]
    fn display_capture_order_is_stable_and_preserves_frame_coordinates() {
        let provider = StubDisplayProvider {
            displays: vec![
                display(
                    "right",
                    PhysicalRect {
                        left: 1920,
                        top: 0,
                        right: 3840,
                        bottom: 1080,
                    },
                ),
                display(
                    "left",
                    PhysicalRect {
                        left: -1280,
                        top: 200,
                        right: 0,
                        bottom: 1224,
                    },
                ),
                display(
                    "primary",
                    PhysicalRect {
                        left: 0,
                        top: 0,
                        right: 1920,
                        bottom: 1080,
                    },
                ),
            ],
        };

        let captures = capture_displays_with(&provider, &SolidCaptureBackend).unwrap();

        assert_eq!(
            captures
                .iter()
                .map(|capture| capture.display.id.as_str())
                .collect::<Vec<_>>(),
            ["primary", "right", "left"]
        );
        assert!(captures.iter().all(|capture| {
            capture.frame.bounds == capture.display.physical_bounds
                && capture.frame.pixel_at(PhysicalPoint {
                    x: capture.display.physical_bounds.left,
                    y: capture.display.physical_bounds.top,
                }) == Some(PixelColor {
                    red: 3,
                    green: 2,
                    blue: 1,
                    alpha: 255,
                })
        }));
    }

    #[test]
    fn display_capture_rejects_empty_and_mismatched_results() {
        let empty = StubDisplayProvider { displays: vec![] };
        assert_eq!(
            capture_displays_with(&empty, &SolidCaptureBackend)
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );

        let provider = StubDisplayProvider {
            displays: vec![display(
                "primary",
                PhysicalRect {
                    left: 0,
                    top: 0,
                    right: 1920,
                    bottom: 1080,
                },
            )],
        };
        assert_eq!(
            capture_displays_with(&provider, &MismatchedCaptureBackend)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn composes_staggered_display_frames_in_virtual_desktop_coordinates() {
        let top = PhysicalRect {
            left: 0,
            top: -1,
            right: 2,
            bottom: 0,
        };
        let bottom = PhysicalRect {
            left: -1,
            top: 0,
            right: 1,
            bottom: 1,
        };
        let captures = [
            super::DisplayCapture {
                display: display("top", top),
                frame: solid_frame(top, [1, 2, 3, 255]),
            },
            super::DisplayCapture {
                display: display("bottom", bottom),
                frame: solid_frame(bottom, [4, 5, 6, 255]),
            },
        ];

        let frame = compose_virtual_desktop(&captures).unwrap();

        assert_eq!(
            frame.bounds,
            PhysicalRect {
                left: -1,
                top: -1,
                right: 2,
                bottom: 1,
            }
        );
        assert_eq!(frame.width, 3);
        assert_eq!(frame.height, 2);
        assert_eq!(frame.cpu_copy_count, 3);
        assert_eq!(
            frame.pixel_at(PhysicalPoint { x: 0, y: -1 }),
            Some(PixelColor {
                red: 3,
                green: 2,
                blue: 1,
                alpha: 255,
            })
        );
        assert_eq!(
            frame.pixel_at(PhysicalPoint { x: -1, y: 0 }),
            Some(PixelColor {
                red: 6,
                green: 5,
                blue: 4,
                alpha: 255,
            })
        );
        assert_eq!(
            frame.pixel_at(PhysicalPoint { x: -1, y: -1 }),
            Some(PixelColor {
                red: 0,
                green: 0,
                blue: 0,
                alpha: 0,
            })
        );
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

    fn display(id: &str, physical_bounds: PhysicalRect) -> DisplayInfo {
        DisplayInfo {
            id: id.to_owned(),
            platform_id: 0,
            physical_bounds,
            work_area: physical_bounds,
            dpi_x: 96,
            dpi_y: 96,
            scale_factor: 1.0,
            rotation: DisplayRotation::Landscape,
            bits_per_pixel: 32,
            primary: id == "primary",
        }
    }

    struct StubDisplayProvider {
        displays: Vec<DisplayInfo>,
    }

    impl DisplayProvider for StubDisplayProvider {
        fn displays(&self) -> io::Result<Vec<DisplayInfo>> {
            Ok(self.displays.clone())
        }
    }

    struct SolidCaptureBackend;

    impl CaptureBackend for SolidCaptureBackend {
        fn capture(&self, bounds: PhysicalRect) -> io::Result<CaptureFrame> {
            Ok(solid_frame(bounds, [1, 2, 3, 255]))
        }
    }

    fn solid_frame(bounds: PhysicalRect, color: [u8; 4]) -> CaptureFrame {
        let stride = bounds.width() as usize * 4;
        let mut pixels = vec![0; stride * bounds.height() as usize];
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.copy_from_slice(&color);
        }
        CaptureFrame {
            bounds,
            width: bounds.width(),
            height: bounds.height(),
            stride,
            format: PixelFormat::Bgra8,
            pixels: Arc::from(pixels),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 1,
        }
    }

    struct MismatchedCaptureBackend;

    impl CaptureBackend for MismatchedCaptureBackend {
        fn capture(&self, _bounds: PhysicalRect) -> io::Result<CaptureFrame> {
            SolidCaptureBackend.capture(PhysicalRect {
                left: 0,
                top: 0,
                right: 1,
                bottom: 1,
            })
        }
    }
}
