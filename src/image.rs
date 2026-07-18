//! Pixel-correct frame cropping and PNG output independent from the UI viewport.

use std::{
    fs::{self, OpenOptions},
    io::{self, BufReader, Write},
    path::Path,
    sync::Arc,
};

use font_kit::{
    canvas::{Canvas, Format, RasterizationOptions},
    family_name::FamilyName,
    hinting::HintingOptions,
    properties::Properties,
    source::SystemSource,
};
use pathfinder_geometry::transform2d::Transform2F;

use crate::{
    domain::{
        annotation::{Annotation, AnnotationDocument, AnnotationKind, SEQUENCE_MARKER_RADIUS},
        geometry::{PhysicalPoint, PhysicalRect},
    },
    platform::capture::{CaptureFrame, PixelFormat},
};

const MOSAIC_BLOCK_SIZE: u32 = 10;
const BLUR_RADIUS: i32 = 4;
const TEXT_FONT_SIZE: f32 = 24.0;

impl CaptureFrame {
    /// Decodes an external PNG into the BGRA frame format used by the editor.
    pub fn open_png(path: impl AsRef<Path>) -> io::Result<Self> {
        let file = fs::File::open(path)?;
        let mut decoder = png::Decoder::new(BufReader::new(file));
        decoder.set_transformations(
            png::Transformations::EXPAND
                | png::Transformations::STRIP_16
                | png::Transformations::ALPHA,
        );
        let mut reader = decoder.read_info().map_err(png_decode_error)?;
        let buffer_size = reader.output_buffer_size().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "PNG output buffer size overflow",
            )
        })?;
        let mut rgba = vec![0; buffer_size];
        let info = reader.next_frame(&mut rgba).map_err(png_decode_error)?;
        if !matches!(info.color_type, png::ColorType::Rgba)
            || info.bit_depth != png::BitDepth::Eight
        {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "PNG must decode to 8-bit RGBA pixels",
            ));
        }
        let width = info.width;
        let height = info.height;
        if width == 0 || height == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "PNG must not be empty",
            ));
        }
        let stride = width as usize * 4;
        let length = stride
            .checked_mul(height as usize)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "PNG dimensions overflow"))?;
        let mut bgra = Vec::with_capacity(length);
        for pixel in rgba[..info.buffer_size()].chunks_exact(4) {
            bgra.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
        if bgra.len() != length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "PNG pixel data is truncated",
            ));
        }
        let frame = Self {
            bounds: PhysicalRect {
                left: 0,
                top: 0,
                right: width as i32,
                bottom: height as i32,
            },
            width,
            height,
            stride,
            format: PixelFormat::Bgra8,
            pixels: Arc::from(bgra),
            capture_duration: std::time::Duration::ZERO,
            cpu_copy_count: 1,
        };
        frame.validate()?;
        Ok(frame)
    }

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

    /// Finds QR payloads in this frame without writing pixel data to disk.
    pub fn decode_qr_codes(&self) -> io::Result<Vec<String>> {
        self.validate()?;
        if self.format != PixelFormat::Bgra8 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "unsupported pixel format",
            ));
        }
        let width = usize::try_from(self.width)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "QR width overflow"))?;
        let height = usize::try_from(self.height)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "QR height overflow"))?;
        let mut image = rqrr::PreparedImage::prepare_from_greyscale(width, height, |x, y| {
            let pixel = &self.pixels[y * self.stride + x * 4..][..4];
            (u16::from(pixel[2]) * 77 + u16::from(pixel[1]) * 150 + u16::from(pixel[0]) * 29)
                .div_ceil(256) as u8
        });
        let mut codes = Vec::new();
        for grid in image.detect_grids() {
            if let Ok((_, content)) = grid.decode()
                && !codes.contains(&content)
            {
                codes.push(content);
            }
        }
        Ok(codes)
    }
}

fn draw_annotation(pixels: &mut [u8], frame: &CaptureFrame, annotation: &Annotation) {
    let color = rgba_bytes(annotation.style.stroke_rgba);
    let fill = annotation.style.fill_rgba.map(rgba_bytes);
    let radius = annotation.style.stroke_width.max(1).div_ceil(2) as i32;
    match annotation.kind {
        AnnotationKind::Watermark { origin } => draw_text_annotation(
            pixels,
            frame,
            origin,
            crate::domain::annotation::WATERMARK_CONTENT,
            color,
        ),
        AnnotationKind::Text {
            origin,
            ref content,
        } => draw_text_annotation(pixels, frame, origin, content, color),
        AnnotationKind::Number { center, value } => {
            draw_number_marker(pixels, frame, center, value, color)
        }
        AnnotationKind::Blur { bounds } => blur_rect(pixels, frame, bounds),
        AnnotationKind::Mosaic { bounds } => mosaic_rect(pixels, frame, bounds),
        AnnotationKind::Highlight { bounds } => fill_rect(pixels, frame, bounds, color),
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

fn draw_text_annotation(
    pixels: &mut [u8],
    frame: &CaptureFrame,
    origin: PhysicalPoint,
    content: &str,
    color: [u8; 4],
) {
    let Ok(handle) =
        SystemSource::new().select_best_match(&[FamilyName::SansSerif], &Properties::new())
    else {
        return;
    };
    let Ok(font) = handle.load() else {
        return;
    };
    let mut pen_x = origin.x as f32;
    for character in content.chars() {
        let Some(glyph) = font.glyph_for_char(character) else {
            pen_x += crate::domain::annotation::TEXT_ANNOTATION_ADVANCE as f32;
            continue;
        };
        let Ok(bounds) = font.raster_bounds(
            glyph,
            TEXT_FONT_SIZE,
            Transform2F::default(),
            HintingOptions::None,
            RasterizationOptions::GrayscaleAa,
        ) else {
            continue;
        };
        let mut canvas = Canvas::new(bounds.size(), Format::A8);
        if font
            .rasterize_glyph(
                &mut canvas,
                glyph,
                TEXT_FONT_SIZE,
                Transform2F::from_translation(-bounds.origin().to_f32()),
                HintingOptions::None,
                RasterizationOptions::GrayscaleAa,
            )
            .is_ok()
        {
            for y in 0..canvas.size.y() {
                for x in 0..canvas.size.x() {
                    let alpha = canvas.pixels[y as usize * canvas.stride + x as usize];
                    if alpha == 0 {
                        continue;
                    }
                    let mut glyph_color = color;
                    glyph_color[3] = (u16::from(color[3]) * u16::from(alpha) / 255) as u8;
                    blend_pixel(
                        pixels,
                        frame,
                        PhysicalPoint {
                            x: pen_x.round() as i32 + bounds.origin_x() + x,
                            y: origin.y + bounds.origin_y() + y,
                        },
                        glyph_color,
                    );
                }
            }
        }
        pen_x += font
            .advance(glyph)
            .map(|advance| advance.x() * TEXT_FONT_SIZE / font.metrics().units_per_em as f32)
            .unwrap_or(crate::domain::annotation::TEXT_ANNOTATION_ADVANCE as f32);
    }
}

fn draw_number_marker(
    pixels: &mut [u8],
    frame: &CaptureFrame,
    center: PhysicalPoint,
    value: u32,
    color: [u8; 4],
) {
    let radius = SEQUENCE_MARKER_RADIUS;
    draw_disc(pixels, frame, center, radius, color);
    let digits = value.to_string();
    let width = digits.len() as i32 * 4 - 1;
    let start_x = center.x - width / 2;
    for (index, digit) in digits.bytes().enumerate() {
        draw_bitmap_digit(
            pixels,
            frame,
            start_x + index as i32 * 4,
            center.y - 2,
            digit,
            [255, 255, 255, 255],
        );
    }
}

fn draw_bitmap_digit(
    pixels: &mut [u8],
    frame: &CaptureFrame,
    left: i32,
    top: i32,
    digit: u8,
    color: [u8; 4],
) {
    const DIGITS: [[u8; 5]; 10] = [
        [0b111, 0b101, 0b101, 0b101, 0b111],
        [0b010, 0b110, 0b010, 0b010, 0b111],
        [0b111, 0b001, 0b111, 0b100, 0b111],
        [0b111, 0b001, 0b111, 0b001, 0b111],
        [0b101, 0b101, 0b111, 0b001, 0b001],
        [0b111, 0b100, 0b111, 0b001, 0b111],
        [0b111, 0b100, 0b111, 0b101, 0b111],
        [0b111, 0b001, 0b010, 0b010, 0b010],
        [0b111, 0b101, 0b111, 0b101, 0b111],
        [0b111, 0b101, 0b111, 0b001, 0b111],
    ];
    let Some(rows) = digit
        .checked_sub(b'0')
        .and_then(|index| DIGITS.get(index as usize))
    else {
        return;
    };
    for (row, bits) in rows.iter().enumerate() {
        for column in 0..3 {
            if bits & (1 << (2 - column)) != 0 {
                blend_pixel(
                    pixels,
                    frame,
                    PhysicalPoint {
                        x: left + column,
                        y: top + row as i32,
                    },
                    color,
                );
            }
        }
    }
}

fn blur_rect(pixels: &mut [u8], frame: &CaptureFrame, bounds: PhysicalRect) {
    let left = bounds.left.max(frame.bounds.left);
    let top = bounds.top.max(frame.bounds.top);
    let right = bounds.right.min(frame.bounds.right);
    let bottom = bounds.bottom.min(frame.bounds.bottom);
    if left >= right || top >= bottom {
        return;
    }
    let source = pixels.to_vec();
    for y in top..bottom {
        for x in left..right {
            let sample_left = (x - BLUR_RADIUS).max(left);
            let sample_top = (y - BLUR_RADIUS).max(top);
            let sample_right = (x + BLUR_RADIUS + 1).min(right);
            let sample_bottom = (y + BLUR_RADIUS + 1).min(bottom);
            let mut totals = [0_u64; 4];
            let mut count = 0_u64;
            for sample_y in sample_top..sample_bottom {
                for sample_x in sample_left..sample_right {
                    let offset = frame_offset(frame, sample_x, sample_y);
                    for (channel, total) in totals.iter_mut().enumerate() {
                        *total += u64::from(source[offset + channel]);
                    }
                    count += 1;
                }
            }
            let offset = frame_offset(frame, x, y);
            let average = totals.map(|total| (total / count.max(1)) as u8);
            pixels[offset..offset + 4].copy_from_slice(&average);
        }
    }
}

fn mosaic_rect(pixels: &mut [u8], frame: &CaptureFrame, bounds: PhysicalRect) {
    let left = bounds.left.max(frame.bounds.left);
    let top = bounds.top.max(frame.bounds.top);
    let right = bounds.right.min(frame.bounds.right);
    let bottom = bounds.bottom.min(frame.bounds.bottom);
    if left >= right || top >= bottom {
        return;
    }
    let block_size = MOSAIC_BLOCK_SIZE as i32;
    for block_top in (top..bottom).step_by(MOSAIC_BLOCK_SIZE as usize) {
        for block_left in (left..right).step_by(MOSAIC_BLOCK_SIZE as usize) {
            let block_right = (block_left + block_size).min(right);
            let block_bottom = (block_top + block_size).min(bottom);
            let pixel_count =
                u32::try_from((block_right - block_left) * (block_bottom - block_top))
                    .unwrap_or(u32::MAX)
                    .max(1);
            let mut totals = [0_u64; 4];
            for y in block_top..block_bottom {
                for x in block_left..block_right {
                    let offset = frame_offset(frame, x, y);
                    for (channel, total) in totals.iter_mut().enumerate() {
                        *total += u64::from(pixels[offset + channel]);
                    }
                }
            }
            let average = totals.map(|total| (total / u64::from(pixel_count)) as u8);
            for y in block_top..block_bottom {
                for x in block_left..block_right {
                    let offset = frame_offset(frame, x, y);
                    pixels[offset..offset + 4].copy_from_slice(&average);
                }
            }
        }
    }
}

fn frame_offset(frame: &CaptureFrame, x: i32, y: i32) -> usize {
    (y - frame.bounds.top) as usize * frame.stride + (x - frame.bounds.left) as usize * 4
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
    let Some(offset) = pixel_offset(frame, point) else {
        return;
    };
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

fn pixel_offset(frame: &CaptureFrame, point: PhysicalPoint) -> Option<usize> {
    let local = frame.bounds.translate_to_local(point)?;
    Some(local.y as usize * frame.stride + local.x as usize * 4)
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
    fn composite_blends_highlight_as_a_filled_physical_pixel_rect() {
        let frame = CaptureFrame {
            bounds: PhysicalRect {
                left: 0,
                top: 0,
                right: 4,
                bottom: 4,
            },
            width: 4,
            height: 4,
            stride: 16,
            format: PixelFormat::Bgra8,
            pixels: Arc::from([20, 20, 20, 255].repeat(16)),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 1,
        };
        let mut document = AnnotationDocument::new(frame.bounds).unwrap();
        let mut history = CommandHistory::default();
        history
            .apply(
                &mut document,
                AnnotationCommand::Insert(Annotation {
                    id: AnnotationId::new(6),
                    kind: AnnotationKind::Highlight {
                        bounds: PhysicalRect {
                            left: 1,
                            top: 1,
                            right: 3,
                            bottom: 3,
                        },
                    },
                    style: AnnotationStyle {
                        stroke_rgba: 0xFFCC0080,
                        fill_rgba: None,
                        stroke_width: 1,
                    },
                }),
            )
            .unwrap();

        let composited = frame.composite_annotations(&document).unwrap();
        let highlighted = composited.pixel_at(PhysicalPoint { x: 1, y: 1 }).unwrap();
        assert_eq!(
            (highlighted.red, highlighted.green, highlighted.blue),
            (137, 112, 9)
        );
        assert_eq!(
            composited
                .pixel_at(PhysicalPoint { x: 3, y: 3 })
                .unwrap()
                .red,
            20
        );
    }

    #[test]
    fn composite_pixelates_mosaic_rects_at_original_physical_pixels() {
        let frame = CaptureFrame {
            bounds: PhysicalRect {
                left: 0,
                top: 0,
                right: 12,
                bottom: 2,
            },
            width: 12,
            height: 2,
            stride: 48,
            format: PixelFormat::Bgra8,
            pixels: Arc::from(
                (0_u8..24)
                    .flat_map(|value| {
                        [value, value.saturating_add(1), value.saturating_add(2), 255]
                    })
                    .collect::<Vec<_>>(),
            ),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 1,
        };
        let mut document = AnnotationDocument::new(frame.bounds).unwrap();
        let mut history = CommandHistory::default();
        history
            .apply(
                &mut document,
                AnnotationCommand::Insert(Annotation {
                    id: AnnotationId::new(7),
                    kind: AnnotationKind::Mosaic {
                        bounds: PhysicalRect {
                            left: 1,
                            top: 0,
                            right: 12,
                            bottom: 2,
                        },
                    },
                    style: AnnotationStyle::default(),
                }),
            )
            .unwrap();

        let composited = frame.composite_annotations(&document).unwrap();
        assert_eq!(
            composited.pixel_at(PhysicalPoint { x: 0, y: 0 }),
            frame.pixel_at(PhysicalPoint { x: 0, y: 0 })
        );
        assert_eq!(
            composited.pixel_at(PhysicalPoint { x: 1, y: 0 }),
            composited.pixel_at(PhysicalPoint { x: 10, y: 1 })
        );
        assert_ne!(
            composited.pixel_at(PhysicalPoint { x: 1, y: 0 }),
            composited.pixel_at(PhysicalPoint { x: 11, y: 0 })
        );
    }

    #[test]
    fn composite_blurs_from_a_stable_region_snapshot() {
        let frame = CaptureFrame {
            bounds: PhysicalRect {
                left: 0,
                top: 0,
                right: 10,
                bottom: 1,
            },
            width: 10,
            height: 1,
            stride: 40,
            format: PixelFormat::Bgra8,
            pixels: Arc::from(
                (0_u8..10)
                    .flat_map(|value| [0, 0, value.saturating_mul(20), 255])
                    .collect::<Vec<_>>(),
            ),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 1,
        };
        let mut document = AnnotationDocument::new(frame.bounds).unwrap();
        let mut history = CommandHistory::default();
        history
            .apply(
                &mut document,
                AnnotationCommand::Insert(Annotation {
                    id: AnnotationId::new(8),
                    kind: AnnotationKind::Blur {
                        bounds: frame.bounds,
                    },
                    style: AnnotationStyle::default(),
                }),
            )
            .unwrap();

        let composited = frame.composite_annotations(&document).unwrap();
        assert_eq!(
            composited
                .pixel_at(PhysicalPoint { x: 0, y: 0 })
                .unwrap()
                .red,
            40
        );
        assert_eq!(
            composited
                .pixel_at(PhysicalPoint { x: 5, y: 0 })
                .unwrap()
                .red,
            100
        );
        assert_eq!(
            composited
                .pixel_at(PhysicalPoint { x: 9, y: 0 })
                .unwrap()
                .red,
            140
        );
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

    #[test]
    fn composite_renders_sequence_markers_in_original_image_pixels() {
        let frame = CaptureFrame {
            bounds: PhysicalRect {
                left: 0,
                top: 0,
                right: 64,
                bottom: 64,
            },
            width: 64,
            height: 64,
            stride: 256,
            format: PixelFormat::Bgra8,
            pixels: Arc::from(vec![0; 64 * 64 * 4]),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 1,
        };
        let mut document = AnnotationDocument::new(frame.bounds).unwrap();
        let mut history = CommandHistory::default();
        history
            .apply(
                &mut document,
                AnnotationCommand::Insert(Annotation {
                    id: AnnotationId::new(9),
                    kind: AnnotationKind::Number {
                        center: PhysicalPoint { x: 32, y: 32 },
                        value: 12,
                    },
                    style: AnnotationStyle {
                        stroke_rgba: 0xFF0000FF,
                        fill_rgba: None,
                        stroke_width: 1,
                    },
                }),
            )
            .unwrap();

        let composited = frame.composite_annotations(&document).unwrap();
        let edge = composited.pixel_at(PhysicalPoint { x: 32, y: 18 }).unwrap();
        let digit = composited.pixel_at(PhysicalPoint { x: 30, y: 30 }).unwrap();
        assert_eq!(
            (edge.red, edge.green, edge.blue, edge.alpha),
            (255, 0, 0, 255)
        );
        assert_eq!(
            (digit.red, digit.green, digit.blue, digit.alpha),
            (255, 255, 255, 255)
        );
        assert_eq!(
            composited.pixel_at(PhysicalPoint { x: 32, y: 17 }),
            frame.pixel_at(PhysicalPoint { x: 32, y: 17 })
        );
    }

    #[test]
    fn composite_renders_text_with_system_font_at_original_pixels() {
        let frame = CaptureFrame {
            bounds: PhysicalRect {
                left: 0,
                top: 0,
                right: 96,
                bottom: 64,
            },
            width: 96,
            height: 64,
            stride: 384,
            format: PixelFormat::Bgra8,
            pixels: Arc::from(vec![0; 96 * 64 * 4]),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 1,
        };
        let mut document = AnnotationDocument::new(frame.bounds).unwrap();
        let mut history = CommandHistory::default();
        history
            .apply(
                &mut document,
                AnnotationCommand::Insert(Annotation {
                    id: AnnotationId::new(10),
                    kind: AnnotationKind::Text {
                        origin: PhysicalPoint { x: 8, y: 8 },
                        content: "Text".to_owned(),
                    },
                    style: AnnotationStyle {
                        stroke_rgba: 0x00FF00FF,
                        fill_rgba: None,
                        stroke_width: 1,
                    },
                }),
            )
            .unwrap();

        let composited = frame.composite_annotations(&document).unwrap();
        assert!(composited.pixels.chunks_exact(4).any(|pixel| pixel[1] > 0));
    }

    #[test]
    fn open_png_converts_rgba_into_editor_bgra_pixels() {
        let directory = std::env::temp_dir().join(format!(
            "flash-shot-open-png-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("image.png");
        let mut encoder = png::Encoder::new(fs::File::create(&path).unwrap(), 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[3, 2, 1, 255, 6, 5, 4, 128])
            .unwrap();

        let frame = CaptureFrame::open_png(&path).unwrap();
        assert_eq!((frame.width, frame.height, frame.stride), (2, 1, 8));
        assert_eq!(frame.pixels.as_ref(), &[1, 2, 3, 255, 4, 5, 6, 128]);
        assert_eq!(frame.bounds.right, 2);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn open_png_expands_rgb_pixels_with_an_opaque_alpha_channel() {
        let directory = std::env::temp_dir().join(format!(
            "flash-shot-open-rgb-png-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("image.png");
        let mut encoder = png::Encoder::new(fs::File::create(&path).unwrap(), 1, 1);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[3, 2, 1])
            .unwrap();

        let frame = CaptureFrame::open_png(&path).unwrap();

        assert_eq!(frame.pixels.as_ref(), &[1, 2, 3, 255]);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn qr_decoder_returns_no_payload_for_an_ordinary_frame() {
        let frame = CaptureFrame {
            bounds: PhysicalRect {
                left: 0,
                top: 0,
                right: 2,
                bottom: 2,
            },
            width: 2,
            height: 2,
            stride: 8,
            format: PixelFormat::Bgra8,
            pixels: Arc::from([255; 16]),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 1,
        };

        assert!(frame.decode_qr_codes().unwrap().is_empty());
    }

    #[test]
    fn qr_decoder_reads_a_generated_code_from_bgra_pixels() {
        const SCALE: usize = 8;
        const QUIET_ZONE: usize = 4;
        let content = "https://flash-shot.local/qr";
        let code = qrcode::QrCode::new(content.as_bytes()).unwrap();
        let code_width = code.width();
        let width = (code_width + QUIET_ZONE * 2) * SCALE;
        let colors = code.to_colors();
        let mut pixels = vec![255; width * width * 4];
        for y in 0..code_width {
            for x in 0..code_width {
                if colors[y * code_width + x] != qrcode::Color::Dark {
                    continue;
                }
                for pixel_y in 0..SCALE {
                    for pixel_x in 0..SCALE {
                        let x = (x + QUIET_ZONE) * SCALE + pixel_x;
                        let y = (y + QUIET_ZONE) * SCALE + pixel_y;
                        let offset = (y * width + x) * 4;
                        pixels[offset..offset + 3].fill(0);
                    }
                }
            }
        }
        let frame = CaptureFrame {
            bounds: PhysicalRect {
                left: 0,
                top: 0,
                right: width as i32,
                bottom: width as i32,
            },
            width: width as u32,
            height: width as u32,
            stride: width * 4,
            format: PixelFormat::Bgra8,
            pixels: Arc::from(pixels),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 1,
        };

        assert_eq!(frame.decode_qr_codes().unwrap(), vec![content]);
    }

    #[test]
    fn composite_renders_a_watermark_at_original_pixels() {
        let frame = CaptureFrame {
            bounds: PhysicalRect {
                left: 0,
                top: 0,
                right: 128,
                bottom: 64,
            },
            width: 128,
            height: 64,
            stride: 512,
            format: PixelFormat::Bgra8,
            pixels: Arc::from(vec![0; 128 * 64 * 4]),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 1,
        };
        let mut document = AnnotationDocument::new(frame.bounds).unwrap();
        let mut history = CommandHistory::default();
        history
            .apply(
                &mut document,
                AnnotationCommand::Insert(Annotation {
                    id: AnnotationId::new(11),
                    kind: AnnotationKind::Watermark {
                        origin: PhysicalPoint { x: 8, y: 8 },
                    },
                    style: AnnotationStyle {
                        stroke_rgba: 0xFF0000FF,
                        fill_rgba: None,
                        stroke_width: 1,
                    },
                }),
            )
            .unwrap();

        let composited = frame.composite_annotations(&document).unwrap();
        assert!(composited.pixels.chunks_exact(4).any(|pixel| pixel[2] > 0));
    }

    #[test]
    fn composite_matches_the_multi_tool_golden_pixel_fingerprint() {
        let frame = CaptureFrame {
            bounds: PhysicalRect {
                left: 0,
                top: 0,
                right: 48,
                bottom: 36,
            },
            width: 48,
            height: 36,
            stride: 192,
            format: PixelFormat::Bgra8,
            pixels: Arc::from(
                (0..36)
                    .flat_map(|y| {
                        (0..48).flat_map(move |x| {
                            [
                                (x * 3 + y) as u8,
                                (x + y * 5) as u8,
                                (x * 7 + y * 2) as u8,
                                255,
                            ]
                        })
                    })
                    .collect::<Vec<_>>(),
            ),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 1,
        };
        let mut document = AnnotationDocument::new(frame.bounds).unwrap();
        let mut history = CommandHistory::default();
        let annotations = [
            Annotation {
                id: AnnotationId::new(20),
                kind: AnnotationKind::Rectangle {
                    bounds: PhysicalRect {
                        left: 2,
                        top: 2,
                        right: 22,
                        bottom: 18,
                    },
                },
                style: AnnotationStyle {
                    stroke_rgba: 0xFF3B30CC,
                    fill_rgba: Some(0xFF3B3044),
                    stroke_width: 2,
                },
            },
            Annotation {
                id: AnnotationId::new(21),
                kind: AnnotationKind::Ellipse {
                    bounds: PhysicalRect {
                        left: 18,
                        top: 5,
                        right: 40,
                        bottom: 25,
                    },
                },
                style: AnnotationStyle {
                    stroke_rgba: 0x007AFFFF,
                    fill_rgba: Some(0x007AFF55),
                    stroke_width: 2,
                },
            },
            Annotation {
                id: AnnotationId::new(22),
                kind: AnnotationKind::Arrow {
                    start: PhysicalPoint { x: 3, y: 31 },
                    end: PhysicalPoint { x: 43, y: 27 },
                },
                style: AnnotationStyle {
                    stroke_rgba: 0x34C759FF,
                    fill_rgba: None,
                    stroke_width: 2,
                },
            },
            Annotation {
                id: AnnotationId::new(23),
                kind: AnnotationKind::Freehand {
                    points: vec![
                        PhysicalPoint { x: 4, y: 22 },
                        PhysicalPoint { x: 11, y: 27 },
                        PhysicalPoint { x: 17, y: 22 },
                    ],
                },
                style: AnnotationStyle {
                    stroke_rgba: 0xAF52DEFF,
                    fill_rgba: None,
                    stroke_width: 2,
                },
            },
            Annotation {
                id: AnnotationId::new(24),
                kind: AnnotationKind::Highlight {
                    bounds: PhysicalRect {
                        left: 26,
                        top: 2,
                        right: 46,
                        bottom: 8,
                    },
                },
                style: AnnotationStyle {
                    stroke_rgba: 0xFFCC0066,
                    fill_rgba: None,
                    stroke_width: 1,
                },
            },
            Annotation {
                id: AnnotationId::new(25),
                kind: AnnotationKind::Mosaic {
                    bounds: PhysicalRect {
                        left: 28,
                        top: 26,
                        right: 47,
                        bottom: 35,
                    },
                },
                style: AnnotationStyle::default(),
            },
            Annotation {
                id: AnnotationId::new(26),
                kind: AnnotationKind::Blur {
                    bounds: PhysicalRect {
                        left: 1,
                        top: 29,
                        right: 17,
                        bottom: 35,
                    },
                },
                style: AnnotationStyle::default(),
            },
        ];
        for annotation in annotations {
            history
                .apply(&mut document, AnnotationCommand::Insert(annotation))
                .unwrap();
        }

        let composited = frame.composite_annotations(&document).unwrap();
        assert_eq!(fnv1a64(&composited.pixels), 15_232_202_040_409_263_755);
    }

    fn fnv1a64(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
    }
}

fn png_decode_error(error: png::DecodingError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}
