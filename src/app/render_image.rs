//! GPUI upload images built directly from captured BGRA pixels.

use std::{io, sync::Arc};

use gpui::RenderImage;
use image::{Frame, RgbaImage};

use crate::platform::capture::CaptureFrame;

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

#[cfg(test)]
mod tests {
    use super::render_image_from_capture;
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
}
