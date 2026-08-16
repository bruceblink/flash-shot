//! Immutable BGRA capture frames shared by capture, export, and UI workflows.

use std::{io, sync::Arc, time::Duration};

use flash_shot_domain::domain::geometry::{PhysicalPoint, PhysicalRect};

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
    /// Checks frame geometry before callers use stride-based row indexing or export pixels.
    pub fn validate(&self) -> io::Result<()> {
        let row_bytes = usize::try_from(self.width)
            .ok()
            .and_then(|width| width.checked_mul(4))
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "frame row overflow"))?;
        let required = self
            .stride
            .checked_mul(self.height as usize)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "frame size overflow"))?;
        if self.width == 0 || self.height == 0 || self.stride < row_bytes {
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

    /// Samples one physical desktop coordinate and converts stored BGRA bytes into a color.
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

#[cfg(test)]
mod tests {
    use super::{CaptureFrame, PixelColor, PixelFormat};
    use flash_shot_domain::domain::geometry::{PhysicalPoint, PhysicalRect};
    use std::{sync::Arc, time::Duration};

    fn frame() -> CaptureFrame {
        CaptureFrame {
            bounds: PhysicalRect {
                left: 10,
                top: 20,
                right: 12,
                bottom: 21,
            },
            width: 2,
            height: 1,
            stride: 8,
            format: PixelFormat::Bgra8,
            pixels: Arc::from([3, 2, 1, 255, 6, 5, 4, 128]),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 1,
        }
    }

    #[test]
    fn pixel_lookup_uses_physical_frame_coordinates_and_bgra_order() {
        assert_eq!(
            frame().pixel_at(PhysicalPoint { x: 11, y: 20 }),
            Some(PixelColor {
                red: 4,
                green: 5,
                blue: 6,
                alpha: 128,
            })
        );
        assert_eq!(frame().pixel_at(PhysicalPoint { x: 12, y: 20 }), None);
    }

    #[test]
    fn validation_rejects_a_stride_shorter_than_one_row() {
        let mut invalid = frame();
        invalid.stride = 4;
        assert!(invalid.validate().is_err());
    }
}
