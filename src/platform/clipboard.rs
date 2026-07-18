//! Native image clipboard output with PNG and DIB compatibility formats.

use std::io;

use crate::platform::capture::CaptureFrame;

pub trait ClipboardService {
    fn copy_image(&self, frame: &CaptureFrame) -> io::Result<()>;
    fn copy_text(&self, text: &str) -> io::Result<()>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClipboard;

impl ClipboardService for SystemClipboard {
    fn copy_image(&self, frame: &CaptureFrame) -> io::Result<()> {
        platform::copy_image(frame)
    }

    fn copy_text(&self, text: &str) -> io::Result<()> {
        platform::copy_text(text)
    }
}

fn encode_dib(frame: &CaptureFrame) -> io::Result<Vec<u8>> {
    frame.validate()?;
    let header_size = 40_usize;
    let pixel_size = frame.width as usize * frame.height as usize * 4;
    let mut dib = vec![0_u8; header_size + pixel_size];
    write_u32(&mut dib, 0, header_size as u32);
    write_i32(&mut dib, 4, frame.width as i32);
    write_i32(&mut dib, 8, frame.height as i32);
    write_u16(&mut dib, 12, 1);
    write_u16(&mut dib, 14, 32);
    write_u32(&mut dib, 20, pixel_size as u32);

    let target_stride = frame.width as usize * 4;
    for target_row in 0..frame.height as usize {
        let source_row = frame.height as usize - target_row - 1;
        let source_start = source_row * frame.stride;
        let target_start = header_size + target_row * target_stride;
        dib[target_start..target_start + target_stride]
            .copy_from_slice(&frame.pixels[source_start..source_start + target_stride]);
    }
    Ok(dib)
}

fn write_u16(target: &mut [u8], offset: usize, value: u16) {
    target[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn write_u32(target: &mut [u8], offset: usize, value: u32) {
    target[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_i32(target: &mut [u8], offset: usize, value: i32) {
    target[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

#[cfg(windows)]
mod platform {
    use super::{CaptureFrame, encode_dib};
    use std::{io, ptr, thread, time::Duration};
    use windows_sys::Win32::{
        Foundation::{GlobalFree, HANDLE, HGLOBAL},
        System::{
            DataExchange::{
                CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW,
                SetClipboardData,
            },
            Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
            Ole::{CF_DIB, CF_UNICODETEXT},
        },
    };

    const OPEN_ATTEMPTS: usize = 8;

    pub fn copy_image(frame: &CaptureFrame) -> io::Result<()> {
        let png = frame.encode_png()?;
        let dib = encode_dib(frame)?;
        let clipboard = ClipboardGuard::open()?;
        // SAFETY: clipboard is open on this thread.
        if unsafe { EmptyClipboard() } == 0 {
            return Err(io::Error::last_os_error());
        }

        let png_name: Vec<u16> = "PNG".encode_utf16().chain(Some(0)).collect();
        // SAFETY: the format name is NUL terminated.
        let png_format = unsafe { RegisterClipboardFormatW(png_name.as_ptr()) };
        if png_format == 0 {
            return Err(io::Error::last_os_error());
        }

        set_data(png_format, &png)?;
        set_data(CF_DIB as u32, &dib)?;
        drop(clipboard);
        Ok(())
    }

    pub fn copy_text(text: &str) -> io::Result<()> {
        let _clipboard = ClipboardGuard::open()?;
        // SAFETY: clipboard is open on this thread.
        if unsafe { EmptyClipboard() } == 0 {
            return Err(io::Error::last_os_error());
        }
        set_data(CF_UNICODETEXT as u32, &utf16_bytes(text))
    }

    pub(super) fn utf16_bytes(text: &str) -> Vec<u8> {
        text.encode_utf16()
            .chain(Some(0))
            .flat_map(u16::to_le_bytes)
            .collect()
    }

    fn set_data(format: u32, bytes: &[u8]) -> io::Result<()> {
        let memory = GlobalMemory::copy_from(bytes)?;
        // SAFETY: clipboard is open, memory is movable and ownership transfers on success.
        if unsafe { SetClipboardData(format, memory.handle as HANDLE) }.is_null() {
            return Err(io::Error::last_os_error());
        }
        memory.transfer();
        Ok(())
    }

    struct ClipboardGuard;

    impl ClipboardGuard {
        fn open() -> io::Result<Self> {
            for attempt in 0..OPEN_ATTEMPTS {
                // SAFETY: a null owner is valid for a short synchronous clipboard operation.
                if unsafe { OpenClipboard(ptr::null_mut()) } != 0 {
                    return Ok(Self);
                }
                if attempt + 1 < OPEN_ATTEMPTS {
                    thread::sleep(Duration::from_millis(5));
                }
            }
            Err(io::Error::last_os_error())
        }
    }

    impl Drop for ClipboardGuard {
        fn drop(&mut self) {
            // SAFETY: this guard represents a successful OpenClipboard call.
            unsafe { CloseClipboard() };
        }
    }

    struct GlobalMemory {
        handle: HGLOBAL,
        transferred: bool,
    }

    impl GlobalMemory {
        fn copy_from(bytes: &[u8]) -> io::Result<Self> {
            // SAFETY: allocation size is derived from the source slice.
            let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) };
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: handle is a valid movable allocation.
            let destination = unsafe { GlobalLock(handle) };
            if destination.is_null() {
                // SAFETY: ownership has not transferred.
                unsafe { GlobalFree(handle) };
                return Err(io::Error::last_os_error());
            }
            // SAFETY: destination has at least bytes.len() bytes and does not overlap source.
            unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), destination.cast(), bytes.len()) };
            // SAFETY: balances the successful GlobalLock.
            unsafe { GlobalUnlock(handle) };
            Ok(Self {
                handle,
                transferred: false,
            })
        }

        fn transfer(mut self) {
            self.transferred = true;
        }
    }

    impl Drop for GlobalMemory {
        fn drop(&mut self) {
            if !self.transferred {
                // SAFETY: failed clipboard transfers leave ownership with this wrapper.
                unsafe { GlobalFree(self.handle) };
            }
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::CaptureFrame;
    use std::io;

    pub fn copy_image(_frame: &CaptureFrame) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "image clipboard is currently Windows-only",
        ))
    }

    pub fn copy_text(_text: &str) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "text clipboard is currently Windows-only",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::encode_dib;
    #[cfg(windows)]
    use super::platform::utf16_bytes;
    use crate::{
        domain::geometry::PhysicalRect,
        platform::capture::{CaptureFrame, PixelFormat},
    };
    use std::{sync::Arc, time::Duration};

    #[test]
    fn dib_is_bottom_up_and_preserves_bgra_pixels() {
        let frame = CaptureFrame {
            bounds: PhysicalRect {
                left: 0,
                top: 0,
                right: 1,
                bottom: 2,
            },
            width: 1,
            height: 2,
            stride: 4,
            format: PixelFormat::Bgra8,
            pixels: Arc::from([1, 2, 3, 255, 4, 5, 6, 255]),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 1,
        };

        let dib = encode_dib(&frame).unwrap();

        assert_eq!(&dib[0..4], &40_u32.to_le_bytes());
        assert_eq!(&dib[4..8], &1_i32.to_le_bytes());
        assert_eq!(&dib[8..12], &2_i32.to_le_bytes());
        assert_eq!(&dib[40..44], &[4, 5, 6, 255]);
        assert_eq!(&dib[44..48], &[1, 2, 3, 255]);
    }

    #[cfg(windows)]
    #[test]
    fn text_clipboard_encoding_is_nul_terminated_utf16() {
        assert_eq!(
            utf16_bytes("Hi 世界"),
            vec![72, 0, 105, 0, 32, 0, 22, 78, 76, 117, 0, 0]
        );
    }
}
