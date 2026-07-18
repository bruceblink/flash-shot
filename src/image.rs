//! Pixel-correct frame cropping and PNG output independent from the UI viewport.

use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::Path,
    sync::Arc,
};

use crate::{
    domain::{
        annotation::{Annotation, AnnotationDocument, AnnotationKind},
        geometry::{PhysicalPoint, PhysicalRect},
    },
    platform::capture::{CaptureFrame, PixelFormat},
};

impl CaptureFrame {
    /// Composites renderer-independent annotations at original physical-pixel
    /// coordinates, producing a new immutable frame suitable for export.
    pub fn composite_annotations(&self, document: &AnnotationDocument) -> io::Result<Self> {
        self.validate()?;
        if self.format != PixelFormat::Bgra8 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "unsupported pixel format",
            ));
        }
        let mut pixels = self.pixels.to_vec();
        for annotation in document.annotations() {
            draw_annotation(&mut pixels, self, annotation);
        }
        Ok(Self {
            bounds: self.bounds,
            width: self.width,
            height: self.height,
            stride: self.stride,
            format: self.format,
            pixels: Arc::from(pixels),
            capture_duration: self.capture_duration,
            cpu_copy_count: self.cpu_copy_count.saturating_add(1),
        })
    }
}

fn draw_annotation(pixels: &mut [u8], frame: &CaptureFrame, annotation: &Annotation) {
    let color = rgba_bytes(annotation.style.stroke_rgba);
    let fill = annotation.style.fill_rgba.map(rgba_bytes);
    let radius = annotation.style.stroke_width.max(1).div_ceil(2) as i32;
    match annotation.kind {
        AnnotationKind::Rectangle { bounds } => {
            if let Some(fill) = fill {
                fill_rect(pixels, frame, bounds, fill);
            }
            draw_rect(pixels, frame, bounds, color, radius);
        }
        AnnotationKind::Ellipse { bounds } => {
            draw_ellipse(pixels, frame, bounds, color, fill, radius);
        }
        AnnotationKind::Line { start, end } => draw_line(pixels, frame, start, end, color, radius),
        AnnotationKind::Arrow { start, end } => {
            draw_line(pixels, frame, start, end, color, radius);
            draw_arrow_head(pixels, frame, start, end, color, radius);
        }
        AnnotationKind::Freehand { ref points } => {
            for segment in points.windows(2) {
                draw_line(pixels, frame, segment[0], segment[1], color, radius);
            }
        }
    }
}

fn rgba_bytes(rgba: u32) -> [u8; 4] {
    rgba.to_be_bytes()
}

fn draw_rect(
    pixels: &mut [u8],
    frame: &CaptureFrame,
    bounds: PhysicalRect,
    color: [u8; 4],
    radius: i32,
) {
    draw_line(
        pixels,
        frame,
        PhysicalPoint {
            x: bounds.left,
            y: bounds.top,
        },
        PhysicalPoint {
            x: bounds.right,
            y: bounds.top,
        },
        color,
        radius,
    );
    draw_line(
        pixels,
        frame,
        PhysicalPoint {
            x: bounds.right,
            y: bounds.top,
        },
        PhysicalPoint {
            x: bounds.right,
            y: bounds.bottom,
        },
        color,
        radius,
    );
    draw_line(
        pixels,
        frame,
        PhysicalPoint {
            x: bounds.right,
            y: bounds.bottom,
        },
        PhysicalPoint {
            x: bounds.left,
            y: bounds.bottom,
        },
        color,
        radius,
    );
    draw_line(
        pixels,
        frame,
        PhysicalPoint {
            x: bounds.left,
            y: bounds.bottom,
        },
        PhysicalPoint {
            x: bounds.left,
            y: bounds.top,
        },
        color,
        radius,
    );
}

fn fill_rect(pixels: &mut [u8], frame: &CaptureFrame, bounds: PhysicalRect, color: [u8; 4]) {
    for y in bounds.top..bounds.bottom {
        for x in bounds.left..bounds.right {
            blend_pixel(pixels, frame, PhysicalPoint { x, y }, color);
        }
    }
}

fn draw_ellipse(
    pixels: &mut [u8],
    frame: &CaptureFrame,
    bounds: PhysicalRect,
    color: [u8; 4],
    fill: Option<[u8; 4]>,
    radius: i32,
) {
    let width = bounds.width() as f64;
    let height = bounds.height() as f64;
    if width == 0.0 || height == 0.0 {
        return;
    }
    let center_x = (f64::from(bounds.left) + f64::from(bounds.right)) / 2.0;
    let center_y = (f64::from(bounds.top) + f64::from(bounds.bottom)) / 2.0;
    let outer_x = width / 2.0 + f64::from(radius);
    let outer_y = height / 2.0 + f64::from(radius);
    let inner_x = (width / 2.0 - f64::from(radius)).max(0.0);
    let inner_y = (height / 2.0 - f64::from(radius)).max(0.0);
    for y in bounds.top.saturating_sub(radius)..=bounds.bottom.saturating_add(radius) {
        for x in bounds.left.saturating_sub(radius)..=bounds.right.saturating_add(radius) {
            let dx = f64::from(x) - center_x;
            let dy = f64::from(y) - center_y;
            let in_outer = (dx / outer_x).powi(2) + (dy / outer_y).powi(2) <= 1.0;
            if !in_outer {
                continue;
            }
            let in_inner = inner_x > 0.0
                && inner_y > 0.0
                && (dx / inner_x).powi(2) + (dy / inner_y).powi(2) < 1.0;
            if in_inner {
                if let Some(fill) = fill {
                    blend_pixel(pixels, frame, PhysicalPoint { x, y }, fill);
                }
            } else {
                blend_pixel(pixels, frame, PhysicalPoint { x, y }, color);
            }
        }
    }
}

fn draw_line(
    pixels: &mut [u8],
    frame: &CaptureFrame,
    start: PhysicalPoint,
    end: PhysicalPoint,
    color: [u8; 4],
    radius: i32,
) {
    let dx = end.x.saturating_sub(start.x).unsigned_abs();
    let dy = end.y.saturating_sub(start.y).unsigned_abs();
    let steps = dx.max(dy).max(1);
    for step in 0..=steps {
        let t = f64::from(step) / f64::from(steps);
        let x = (f64::from(start.x) + f64::from(end.x - start.x) * t).round() as i32;
        let y = (f64::from(start.y) + f64::from(end.y - start.y) * t).round() as i32;
        draw_disc(pixels, frame, PhysicalPoint { x, y }, radius, color);
    }
}

fn draw_arrow_head(
    pixels: &mut [u8],
    frame: &CaptureFrame,
    start: PhysicalPoint,
    end: PhysicalPoint,
    color: [u8; 4],
    radius: i32,
) {
    let dx = f64::from(end.x) - f64::from(start.x);
    let dy = f64::from(end.y) - f64::from(start.y);
    let length = dx.hypot(dy);
    if length == 0.0 {
        return;
    }
    let size = f64::from(radius.max(3) * 4);
    let unit_x = dx / length;
    let unit_y = dy / length;
    for angle in [0.55_f64, -0.55_f64] {
        let cosine = angle.cos();
        let sine = angle.sin();
        let backward_x = -unit_x * cosine - unit_y * sine;
        let backward_y = -unit_x * sine + unit_y * cosine;
        let point = PhysicalPoint {
            x: (f64::from(end.x) + backward_x * size).round() as i32,
            y: (f64::from(end.y) + backward_y * size).round() as i32,
        };
        draw_line(pixels, frame, end, point, color, radius);
    }
}

fn draw_disc(
    pixels: &mut [u8],
    frame: &CaptureFrame,
    center: PhysicalPoint,
    radius: i32,
    color: [u8; 4],
) {
    for y in center.y.saturating_sub(radius)..=center.y.saturating_add(radius) {
        for x in center.x.saturating_sub(radius)..=center.x.saturating_add(radius) {
            let dx = i64::from(x) - i64::from(center.x);
            let dy = i64::from(y) - i64::from(center.y);
            if dx * dx + dy * dy <= i64::from(radius) * i64::from(radius) {
                blend_pixel(pixels, frame, PhysicalPoint { x, y }, color);
            }
        }
    }
}

fn blend_pixel(pixels: &mut [u8], frame: &CaptureFrame, point: PhysicalPoint, color: [u8; 4]) {
    let Some(local) = frame.bounds.translate_to_local(point) else {
        return;
    };
    let offset = local.y as usize * frame.stride + local.x as usize * 4;
    let alpha = u16::from(color[3]);
    let inverse = 255 - alpha;
    pixels[offset] =
        ((u16::from(color[2]) * alpha + u16::from(pixels[offset]) * inverse) / 255) as u8;
    pixels[offset + 1] =
        ((u16::from(color[1]) * alpha + u16::from(pixels[offset + 1]) * inverse) / 255) as u8;
    pixels[offset + 2] =
        ((u16::from(color[0]) * alpha + u16::from(pixels[offset + 2]) * inverse) / 255) as u8;
    pixels[offset + 3] = (alpha + u16::from(pixels[offset + 3]) * inverse / 255) as u8;
}

impl CaptureFrame {
    pub fn crop(&self, selection: PhysicalRect) -> io::Result<Self> {
        self.validate()?;
        let left = selection.left.max(self.bounds.left);
        let top = selection.top.max(self.bounds.top);
        let right = selection.right.min(self.bounds.right);
        let bottom = selection.bottom.min(self.bounds.bottom);
        if left >= right || top >= bottom {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "selection does not intersect the capture frame",
            ));
        }

        let bounds = PhysicalRect {
            left,
            top,
            right,
            bottom,
        };
        let width = bounds.width();
        let height = bounds.height();
        let stride = width as usize * 4;
        let mut pixels = vec![0_u8; stride * height as usize];
        let source_x = (left - self.bounds.left) as usize * 4;
        let source_y = (top - self.bounds.top) as usize;

        for row in 0..height as usize {
            let source_start = (source_y + row) * self.stride + source_x;
            let source_end = source_start + stride;
            let target_start = row * stride;
            pixels[target_start..target_start + stride]
                .copy_from_slice(&self.pixels[source_start..source_end]);
        }

        Ok(Self {
            bounds,
            width,
            height,
            stride,
            format: self.format,
            pixels: Arc::from(pixels),
            capture_duration: self.capture_duration,
            cpu_copy_count: self.cpu_copy_count + 1,
        })
    }

    pub fn encode_png(&self) -> io::Result<Vec<u8>> {
        self.validate()?;
        if self.format != PixelFormat::Bgra8 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "unsupported pixel format",
            ));
        }

        let mut rgba = Vec::with_capacity(self.width as usize * self.height as usize * 4);
        for row in self.pixels.chunks_exact(self.stride) {
            for pixel in row[..self.width as usize * 4].chunks_exact(4) {
                rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
            }
        }

        let mut encoded = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut encoded, self.width, self.height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().map_err(png_error)?;
            writer.write_image_data(&rgba).map_err(png_error)?;
        }
        Ok(encoded)
    }

    pub fn save_png(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref();
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty());
        if let Some(parent) = parent {
            fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension("png.tmp");
        let encoded = self.encode_png()?;
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(&encoded)?;
        file.sync_all()?;
        drop(file);
        replace_file(&temporary, path)
    }
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // SAFETY: both paths are valid NUL-terminated UTF-16 buffers for the duration of the call.
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

fn png_error(error: png::EncodingError) -> io::Error {
    io::Error::other(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::annotation::{
        Annotation, AnnotationCommand, AnnotationDocument, AnnotationId, AnnotationKind,
        AnnotationStyle, CommandHistory,
    };
    use std::{io::Cursor, time::Duration};

    fn test_frame() -> CaptureFrame {
        CaptureFrame {
            bounds: PhysicalRect {
                left: -2,
                top: 10,
                right: 1,
                bottom: 12,
            },
            width: 3,
            height: 2,
            stride: 12,
            format: PixelFormat::Bgra8,
            pixels: Arc::from([
                1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255, 13, 14, 15, 255, 16, 17,
                18, 255,
            ]),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 1,
        }
    }

    #[test]
    fn crop_uses_virtual_desktop_physical_coordinates() {
        let cropped = test_frame()
            .crop(PhysicalRect {
                left: -1,
                top: 10,
                right: 1,
                bottom: 12,
            })
            .unwrap();

        assert_eq!(cropped.width, 2);
        assert_eq!(cropped.height, 2);
        assert_eq!(
            cropped.pixels.as_ref(),
            &[4, 5, 6, 255, 7, 8, 9, 255, 13, 14, 15, 255, 16, 17, 18, 255]
        );
        assert_eq!(cropped.cpu_copy_count, 2);
    }

    #[test]
    fn png_converts_bgra_to_pixel_correct_rgba() {
        let frame = test_frame()
            .crop(PhysicalRect {
                left: -2,
                top: 10,
                right: -1,
                bottom: 11,
            })
            .unwrap();
        let encoded = frame.encode_png().unwrap();
        let decoder = png::Decoder::new(Cursor::new(encoded));
        let mut reader = decoder.read_info().unwrap();
        let mut output = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut output).unwrap();

        assert_eq!(info.width, 1);
        assert_eq!(info.height, 1);
        assert_eq!(&output[..info.buffer_size()], &[3, 2, 1, 255]);
    }

    #[test]
    fn save_png_replaces_destination_atomically() {
        let directory = std::env::temp_dir().join(format!(
            "flash-shot-image-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let path = directory.join("capture.png");
        fs::create_dir_all(&directory).unwrap();
        fs::write(&path, b"old capture").unwrap();

        test_frame().save_png(&path).unwrap();

        assert!(fs::metadata(&path).unwrap().len() > 0);
        assert_ne!(fs::read(&path).unwrap(), b"old capture");
        assert!(!path.with_extension("png.tmp").exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn composite_uses_physical_coordinates_and_preserves_source_frame() {
        let frame = test_frame();
        let mut document = AnnotationDocument::new(frame.bounds).unwrap();
        let mut history = CommandHistory::default();
        history
            .apply(
                &mut document,
                AnnotationCommand::Insert(Annotation {
                    id: AnnotationId::new(1),
                    kind: AnnotationKind::Line {
                        start: PhysicalPoint { x: -2, y: 10 },
                        end: PhysicalPoint { x: 0, y: 10 },
                    },
                    style: AnnotationStyle {
                        stroke_rgba: 0xFF0000FF,
                        stroke_width: 1,
                        fill_rgba: None,
                    },
                }),
            )
            .unwrap();

        let composited = frame.composite_annotations(&document).unwrap();

        assert_eq!(
            frame.pixel_at(PhysicalPoint { x: -2, y: 10 }).unwrap().red,
            3
        );
        assert_eq!(
            composited
                .pixel_at(PhysicalPoint { x: -2, y: 10 })
                .unwrap()
                .red,
            255
        );
        assert_eq!(composited.cpu_copy_count, frame.cpu_copy_count + 1);
    }

    #[test]
    fn composite_blends_fill_then_draws_outline_and_clips_to_frame() {
        let frame = CaptureFrame {
            bounds: PhysicalRect {
                left: 0,
                top: 0,
                right: 8,
                bottom: 8,
            },
            width: 8,
            height: 8,
            stride: 32,
            format: PixelFormat::Bgra8,
            pixels: Arc::from([10, 10, 10, 255].repeat(64)),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 1,
        };
        let mut document = AnnotationDocument::new(frame.bounds).unwrap();
        let mut history = CommandHistory::default();
        history
            .apply(
                &mut document,
                AnnotationCommand::Insert(Annotation {
                    id: AnnotationId::new(2),
                    kind: AnnotationKind::Rectangle {
                        bounds: PhysicalRect {
                            left: -2,
                            top: -2,
                            right: 7,
                            bottom: 7,
                        },
                    },
                    style: AnnotationStyle {
                        stroke_rgba: 0x0000FFFF,
                        fill_rgba: Some(0x00FF0080),
                        stroke_width: 1,
                    },
                }),
            )
            .unwrap();

        let composited = frame.composite_annotations(&document).unwrap();

        let interior = composited.pixel_at(PhysicalPoint { x: 4, y: 4 }).unwrap();
        assert_eq!(interior.green, 132);
        assert_eq!(interior.alpha, 255);
        let edge = composited.pixel_at(PhysicalPoint { x: 6, y: 4 }).unwrap();
        assert_eq!(edge.blue, 255);
    }

    #[test]
    fn composite_renders_ellipse_arrow_and_freehand_without_viewport_scaling() {
        let frame = CaptureFrame {
            bounds: PhysicalRect {
                left: 0,
                top: 0,
                right: 32,
                bottom: 32,
            },
            width: 32,
            height: 32,
            stride: 128,
            format: PixelFormat::Bgra8,
            pixels: Arc::from(vec![0; 32 * 32 * 4]),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 1,
        };
        let mut document = AnnotationDocument::new(frame.bounds).unwrap();
        let mut history = CommandHistory::default();
        for (id, kind) in [
            (
                3,
                AnnotationKind::Ellipse {
                    bounds: PhysicalRect {
                        left: 2,
                        top: 2,
                        right: 12,
                        bottom: 12,
                    },
                },
            ),
            (
                4,
                AnnotationKind::Arrow {
                    start: PhysicalPoint { x: 15, y: 4 },
                    end: PhysicalPoint { x: 27, y: 12 },
                },
            ),
            (
                5,
                AnnotationKind::Freehand {
                    points: vec![
                        PhysicalPoint { x: 4, y: 20 },
                        PhysicalPoint { x: 12, y: 24 },
                        PhysicalPoint { x: 20, y: 20 },
                    ],
                },
            ),
        ] {
            history
                .apply(
                    &mut document,
                    AnnotationCommand::Insert(Annotation {
                        id: AnnotationId::new(id),
                        kind,
                        style: AnnotationStyle::default(),
                    }),
                )
                .unwrap();
        }

        let composited = frame.composite_annotations(&document).unwrap();

        assert_ne!(
            composited
                .pixel_at(PhysicalPoint { x: 7, y: 2 })
                .unwrap()
                .alpha,
            0
        );
        assert_ne!(
            composited
                .pixel_at(PhysicalPoint { x: 27, y: 12 })
                .unwrap()
                .alpha,
            0
        );
        assert_ne!(
            composited
                .pixel_at(PhysicalPoint { x: 12, y: 24 })
                .unwrap()
                .alpha,
            0
        );
    }
}
