//! Pixel-correct frame cropping and PNG output independent from the UI viewport.

use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::Path,
    sync::Arc,
};

use crate::{
    domain::geometry::PhysicalRect,
    platform::capture::{CaptureFrame, PixelFormat},
};

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
}
