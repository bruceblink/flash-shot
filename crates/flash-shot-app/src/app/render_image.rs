//! GPUI upload images built directly from captured BGRA pixels.

use std::{io, sync::Arc};

use gpui::RenderImage;
use image::{Frame, RgbaImage};

use crate::platform::capture::CaptureFrame;

pub(super) const HISTORY_THUMBNAIL_WIDTH: u32 = 160;
pub(super) const HISTORY_THUMBNAIL_HEIGHT: u32 = 100;

pub(super) struct CaptureRenderImage {
    pub(super) image: Arc<RenderImage>,
    pub(super) upload_bytes: usize,
}

pub(super) fn render_image_from_capture(frame: &CaptureFrame) -> io::Result<CaptureRenderImage> {
    frame.validate()?;
    let row_bytes = frame.width as usize * 4;
    let upload_bytes = row_bytes
        .checked_mul(frame.height as usize)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "render image size overflow"))?;
    let mut pixels = Vec::with_capacity(upload_bytes);
    for row in frame.pixels.chunks_exact(frame.stride) {
        pixels.extend_from_slice(&row[..row_bytes]);
    }
    let pixels = RgbaImage::from_raw(frame.width, frame.height, pixels)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid render image pixels"))?;

    // GPUI stores decoded image buffers as BGRA, matching the capture backend.
    Ok(CaptureRenderImage {
        image: Arc::new(RenderImage::new(vec![Frame::new(pixels)])),
        upload_bytes,
    })
}

/// Downscales a capture before it reaches the UI thread, bounding preview memory and upload work.
pub(super) fn history_thumbnail_frame(frame: &CaptureFrame) -> io::Result<CaptureFrame> {
    frame.validate()?;
    let (width, height) = thumbnail_dimensions(frame.width, frame.height);
    let stride = width as usize * 4;
    let length = stride
        .checked_mul(height as usize)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "thumbnail size overflow"))?;
    let mut pixels = vec![0; length];
    for target_y in 0..height {
        let source_y = (target_y as u64 * frame.height as u64 / height as u64) as usize;
        for target_x in 0..width {
            let source_x = (target_x as u64 * frame.width as u64 / width as u64) as usize;
            let source = source_y * frame.stride + source_x * 4;
            let target = target_y as usize * stride + target_x as usize * 4;
            pixels[target..target + 4].copy_from_slice(&frame.pixels[source..source + 4]);
        }
    }
    Ok(CaptureFrame {
        bounds: crate::domain::geometry::PhysicalRect {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        },
        width,
        height,
        stride,
        format: frame.format,
        pixels: pixels.into(),
        capture_duration: frame.capture_duration,
        cpu_copy_count: frame.cpu_copy_count.saturating_add(1),
    })
}

/// Calculates a bounded preview size while preserving the captured image ratio.
fn thumbnail_dimensions(width: u32, height: u32) -> (u32, u32) {
    if u64::from(width) * u64::from(HISTORY_THUMBNAIL_HEIGHT)
        > u64::from(height) * u64::from(HISTORY_THUMBNAIL_WIDTH)
    {
        (
            HISTORY_THUMBNAIL_WIDTH,
            (u64::from(height) * u64::from(HISTORY_THUMBNAIL_WIDTH) / u64::from(width)).max(1)
                as u32,
        )
    } else {
        (
            (u64::from(width) * u64::from(HISTORY_THUMBNAIL_HEIGHT) / u64::from(height)).max(1)
                as u32,
            HISTORY_THUMBNAIL_HEIGHT,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{history_thumbnail_frame, render_image_from_capture};
    use crate::{
        domain::geometry::PhysicalRect,
        platform::capture::{CaptureFrame, PixelFormat},
    };
    use std::{sync::Arc, time::Duration};

    #[test]
    fn render_image_keeps_bgra_bytes_without_png_round_trip() {
        let frame = CaptureFrame {
            bounds: PhysicalRect {
                left: 0,
                top: 0,
                right: 2,
                bottom: 1,
            },
            width: 2,
            height: 1,
            stride: 8,
            format: PixelFormat::Bgra8,
            pixels: Arc::from([1, 2, 3, 255, 4, 5, 6, 255]),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 1,
        };

        let rendered = render_image_from_capture(&frame).unwrap();

        assert_eq!(rendered.upload_bytes, 8);
        assert_eq!(
            rendered.image.as_bytes(0),
            Some(&[1, 2, 3, 255, 4, 5, 6, 255][..])
        );
    }

    #[test]
    fn render_image_drops_stride_padding_before_upload() {
        let frame = CaptureFrame {
            bounds: PhysicalRect {
                left: 0,
                top: 0,
                right: 1,
                bottom: 2,
            },
            width: 1,
            height: 2,
            stride: 8,
            format: PixelFormat::Bgra8,
            pixels: Arc::from([1, 2, 3, 255, 99, 99, 99, 99, 4, 5, 6, 255, 88, 88, 88, 88]),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 1,
        };

        let rendered = render_image_from_capture(&frame).unwrap();

        assert_eq!(rendered.upload_bytes, 8);
        assert_eq!(
            rendered.image.as_bytes(0),
            Some(&[1, 2, 3, 255, 4, 5, 6, 255][..])
        );
    }

    #[test]
    fn history_thumbnail_is_bounded_and_preserves_source_pixels() {
        let frame = CaptureFrame {
            bounds: PhysicalRect {
                left: 0,
                top: 0,
                right: 4,
                bottom: 2,
            },
            width: 4,
            height: 2,
            stride: 16,
            format: PixelFormat::Bgra8,
            pixels: Arc::from([
                1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255, 13, 14, 15, 255, 16, 17,
                18, 255, 19, 20, 21, 255, 22, 23, 24, 255,
            ]),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 1,
        };

        let thumbnail_frame = history_thumbnail_frame(&frame).unwrap();
        let thumbnail = render_image_from_capture(&thumbnail_frame).unwrap();

        assert_eq!(thumbnail.upload_bytes, 160 * 80 * 4);
        assert_eq!(thumbnail.image.as_bytes(0).unwrap()[..4], [1, 2, 3, 255]);
    }

    #[test]
    fn history_thumbnail_frame_keeps_ui_uploads_bounded() {
        let frame = CaptureFrame {
            bounds: PhysicalRect {
                left: 0,
                top: 0,
                right: 1_600,
                bottom: 100,
            },
            width: 1_600,
            height: 100,
            stride: 6_400,
            format: PixelFormat::Bgra8,
            pixels: vec![0; 640_000].into(),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 1,
        };

        let thumbnail = history_thumbnail_frame(&frame).unwrap();

        assert_eq!((thumbnail.width, thumbnail.height), (160, 10));
        assert_eq!(thumbnail.pixels.len(), 6_400);
    }
}
