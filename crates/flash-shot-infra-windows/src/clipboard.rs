//! Native image clipboard output with PNG and DIB compatibility formats.

use std::io;

use flash_shot_domain::domain::geometry::PhysicalRect;

use flash_shot_image::{CaptureFrame, PixelFormat};

/// Coordinates a cancellable image copy at the point where it can change the clipboard.
///
/// Image encoding and acquiring the clipboard stay reversible. The caller must claim the commit
/// immediately before `EmptyClipboard`, because Windows cannot restore the previous contents once
/// that call succeeds.
pub trait ClipboardCommitGate: Send + Sync {
    fn is_cancelled(&self) -> bool;
    fn begin_clipboard_commit(&self) -> bool;
    fn finish_clipboard_commit(&self);
}

pub trait ClipboardService {
    fn copy_image(&self, frame: &CaptureFrame) -> io::Result<()>;

    /// Copies pixels only after the caller claims the non-reversible clipboard commit.
    ///
    /// Implementations that cannot move the gate deeper use this final check directly before
    /// `copy_image`. The Windows implementation overrides it to claim immediately before
    /// `EmptyClipboard`.
    fn copy_image_cancellable(
        &self,
        frame: &CaptureFrame,
        gate: &dyn ClipboardCommitGate,
    ) -> io::Result<bool> {
        if gate.is_cancelled() || !gate.begin_clipboard_commit() {
            return Ok(false);
        }
        let result = self.copy_image(frame);
        gate.finish_clipboard_commit();
        result.map(|()| true)
    }

    fn copy_text(&self, text: &str) -> io::Result<()>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClipboard;

impl ClipboardService for SystemClipboard {
    fn copy_image(&self, frame: &CaptureFrame) -> io::Result<()> {
        platform::copy_image(frame)
    }

    fn copy_image_cancellable(
        &self,
        frame: &CaptureFrame,
        gate: &dyn ClipboardCommitGate,
    ) -> io::Result<bool> {
        platform::copy_image_cancellable(frame, gate)
    }

    fn copy_text(&self, text: &str) -> io::Result<()> {
        platform::copy_text(text)
    }
}

impl SystemClipboard {
    /// Reads the current image clipboard payload into the capture frame used by editing and pinning.
    pub fn read_image(&self) -> io::Result<CaptureFrame> {
        platform::read_image()
    }
}

/// Converts decoded RGBA clipboard pixels into the app's immutable BGRA frame representation.
fn frame_from_rgba(width: usize, height: usize, rgba: &[u8]) -> io::Result<CaptureFrame> {
    let width_u32 = u32::try_from(width)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "clipboard image is too wide"))?;
    let height_u32 = u32::try_from(height)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "clipboard image is too tall"))?;
    let right = i32::try_from(width)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "clipboard image is too wide"))?;
    let bottom = i32::try_from(height)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "clipboard image is too tall"))?;
    let stride = width.checked_mul(4).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "clipboard image size overflow")
    })?;
    let expected = stride.checked_mul(height).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "clipboard image size overflow")
    })?;
    if width == 0 || height == 0 || rgba.len() != expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "clipboard image pixels do not match its dimensions",
        ));
    }

    let mut bgra = Vec::with_capacity(expected);
    for pixel in rgba.chunks_exact(4) {
        bgra.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
    }
    let frame = CaptureFrame {
        bounds: PhysicalRect {
            left: 0,
            top: 0,
            right,
            bottom,
        },
        width: width_u32,
        height: height_u32,
        stride,
        format: PixelFormat::Bgra8,
        pixels: bgra.into(),
        capture_duration: std::time::Duration::ZERO,
        cpu_copy_count: 1,
    };
    frame.validate()?;
    Ok(frame)
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
    use super::{CaptureFrame, ClipboardCommitGate, encode_dib, frame_from_rgba};
    use std::{
        io,
        marker::PhantomData,
        ptr, thread,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{
        Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND},
        System::{
            DataExchange::{
                CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW,
                SetClipboardData,
            },
            Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
            Ole::{CF_DIB, CF_UNICODETEXT},
        },
        UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, HWND_MESSAGE},
    };

    const READ_ATTEMPTS: usize = 8;
    const OPEN_RETRY_TIMEOUT: Duration = Duration::from_millis(200);
    const OPEN_RETRY_DELAY: Duration = Duration::from_millis(5);

    pub fn read_image() -> io::Result<CaptureFrame> {
        let mut last_error = None;
        for attempt in 0..READ_ATTEMPTS {
            match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_image()) {
                Ok(image) => {
                    return frame_from_rgba(image.width, image.height, image.bytes.as_ref());
                }
                Err(error) => last_error = Some(error),
            }
            if attempt + 1 < READ_ATTEMPTS {
                thread::sleep(Duration::from_millis(5));
            }
        }
        Err(clipboard_error(
            last_error.expect("clipboard read attempted"),
        ))
    }

    fn clipboard_error(error: arboard::Error) -> io::Error {
        io::Error::other(format!("could not read clipboard image: {error}"))
    }

    pub fn copy_image(frame: &CaptureFrame) -> io::Result<()> {
        let png = frame.encode_png()?;
        let dib = encode_dib(frame)?;
        let clipboard = ClipboardGuard::open()?;
        write_image_to_open_clipboard(&png, &dib)?;
        drop(clipboard);
        Ok(())
    }

    /// Keeps cancellation available while encoding and waiting for clipboard ownership.
    ///
    /// The gate is deliberately claimed only after `OpenClipboard` succeeds and immediately
    /// before `EmptyClipboard`, which is the first irreversible mutation of clipboard state.
    pub fn copy_image_cancellable(
        frame: &CaptureFrame,
        gate: &dyn ClipboardCommitGate,
    ) -> io::Result<bool> {
        if gate.is_cancelled() {
            return Ok(false);
        }
        let png = frame.encode_png()?;
        if gate.is_cancelled() {
            return Ok(false);
        }
        let dib = encode_dib(frame)?;
        if gate.is_cancelled() {
            return Ok(false);
        }
        let Some(clipboard) = ClipboardGuard::open_cancellable(gate)? else {
            return Ok(false);
        };
        if !gate.begin_clipboard_commit() {
            return Ok(false);
        }
        let result = write_image_to_open_clipboard(&png, &dib);
        gate.finish_clipboard_commit();
        drop(clipboard);
        result.map(|()| true)
    }

    /// Publishes pre-encoded data while the caller owns the native clipboard.
    fn write_image_to_open_clipboard(png: &[u8], dib: &[u8]) -> io::Result<()> {
        // SAFETY: clipboard is open on this thread.
        if unsafe { EmptyClipboard() } == 0 {
            return Err(io::Error::other(format!(
                "EmptyClipboard(image) failed: {}",
                io::Error::last_os_error()
            )));
        }

        let png_name: Vec<u16> = "PNG".encode_utf16().chain(Some(0)).collect();
        // SAFETY: the format name is NUL terminated.
        let png_format = unsafe { RegisterClipboardFormatW(png_name.as_ptr()) };
        if png_format == 0 {
            return Err(io::Error::last_os_error());
        }

        set_data(png_format, png, "PNG")?;
        set_data(CF_DIB as u32, dib, "CF_DIB")?;
        Ok(())
    }

    pub fn copy_text(text: &str) -> io::Result<()> {
        let _clipboard = ClipboardGuard::open()?;
        // SAFETY: clipboard is open on this thread.
        if unsafe { EmptyClipboard() } == 0 {
            return Err(io::Error::other(format!(
                "EmptyClipboard(text) failed: {}",
                io::Error::last_os_error()
            )));
        }
        set_data(CF_UNICODETEXT as u32, &utf16_bytes(text), "CF_UNICODETEXT")
    }

    pub(super) fn utf16_bytes(text: &str) -> Vec<u8> {
        text.encode_utf16()
            .chain(Some(0))
            .flat_map(u16::to_le_bytes)
            .collect()
    }

    fn set_data(format: u32, bytes: &[u8], label: &str) -> io::Result<()> {
        let memory = GlobalMemory::copy_from(bytes)?;
        // SAFETY: clipboard is open, memory is movable and ownership transfers on success.
        if unsafe { SetClipboardData(format, memory.handle as HANDLE) }.is_null() {
            return Err(io::Error::other(format!(
                "SetClipboardData({label}) failed: {}",
                io::Error::last_os_error()
            )));
        }
        memory.transfer();
        Ok(())
    }

    /// Keeps a non-null, thread-owned HWND alive for the complete clipboard transaction.
    ///
    /// Windows can lose the open-clipboard association between multiple `SetClipboardData` calls
    /// when `OpenClipboard` receives a null owner. A message-only `STATIC` window gives
    /// `EmptyClipboard` a stable owner without adding a visible window or a custom message loop.
    struct ClipboardOwnerWindow {
        handle: HWND,
        _not_send: PhantomData<*const ()>,
    }

    impl ClipboardOwnerWindow {
        fn new() -> io::Result<Self> {
            let class_name = "STATIC".encode_utf16().chain(Some(0)).collect::<Vec<_>>();
            // SAFETY: the predefined class name is NUL terminated, every optional pointer is
            // null, and HWND_MESSAGE creates a non-visible window owned by this thread.
            let handle = unsafe {
                CreateWindowExW(
                    0,
                    class_name.as_ptr(),
                    ptr::null(),
                    0,
                    0,
                    0,
                    0,
                    0,
                    HWND_MESSAGE,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null(),
                )
            };
            if handle.is_null() {
                Err(io::Error::last_os_error())
            } else {
                Ok(Self {
                    handle,
                    _not_send: PhantomData,
                })
            }
        }
    }

    impl Drop for ClipboardOwnerWindow {
        fn drop(&mut self) {
            // SAFETY: this HWND was created by `new` on the same thread and remains owned here.
            unsafe { DestroyWindow(self.handle) };
        }
    }

    struct ClipboardGuard {
        _owner: ClipboardOwnerWindow,
    }

    impl ClipboardGuard {
        fn open() -> io::Result<Self> {
            retry_clipboard_operation(
                OPEN_RETRY_TIMEOUT,
                OPEN_RETRY_DELAY,
                || false,
                open_clipboard_once,
            )?
            .ok_or_else(|| io::Error::other("clipboard open was unexpectedly cancelled"))
        }

        /// Retries for ownership without holding the cancellation gate during a reversible wait.
        fn open_cancellable(gate: &dyn ClipboardCommitGate) -> io::Result<Option<Self>> {
            retry_clipboard_operation(
                OPEN_RETRY_TIMEOUT,
                OPEN_RETRY_DELAY,
                || gate.is_cancelled(),
                open_clipboard_once,
            )
        }
    }

    /// Opens the process-global clipboard once so the retry policy remains independently testable.
    fn open_clipboard_once() -> io::Result<ClipboardGuard> {
        let owner = ClipboardOwnerWindow::new()?;
        // SAFETY: the message-only owner is valid and remains alive inside the returned guard.
        if unsafe { OpenClipboard(owner.handle) } != 0 {
            Ok(ClipboardGuard { _owner: owner })
        } else {
            Err(io::Error::last_os_error())
        }
    }

    /// Retries a reversible clipboard operation until it succeeds, is cancelled, or exhausts its
    /// fixed contention budget. Cancellation is checked before every attempt so waiting never
    /// claims the caller's irreversible clipboard commit gate.
    fn retry_clipboard_operation<T>(
        timeout: Duration,
        retry_delay: Duration,
        mut cancelled: impl FnMut() -> bool,
        mut operation: impl FnMut() -> io::Result<T>,
    ) -> io::Result<Option<T>> {
        let deadline = Instant::now() + timeout;
        loop {
            if cancelled() {
                return Ok(None);
            }
            match operation() {
                Ok(value) => return Ok(Some(value)),
                Err(error) if Instant::now() >= deadline => return Err(error),
                Err(error) => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    thread::sleep(retry_delay.min(remaining));
                    if retry_delay >= remaining {
                        return Err(error);
                    }
                }
            }
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

    #[cfg(test)]
    mod tests {
        use super::{ClipboardOwnerWindow, retry_clipboard_operation};
        use std::{cell::Cell, io, time::Duration};
        use windows_sys::Win32::UI::WindowsAndMessaging::{IsWindow, IsWindowVisible};

        #[test]
        fn clipboard_owner_is_a_hidden_thread_window_with_bounded_lifetime() {
            let owner = ClipboardOwnerWindow::new().unwrap();
            let handle = owner.handle;
            // SAFETY: the handle belongs to the owner that is alive in this scope.
            assert_ne!(unsafe { IsWindow(handle) }, 0);
            // SAFETY: HWND_MESSAGE windows are never part of the visible desktop hierarchy.
            assert_eq!(unsafe { IsWindowVisible(handle) }, 0);

            drop(owner);
            // SAFETY: querying a stale HWND is supported and must report that it is gone.
            assert_eq!(unsafe { IsWindow(handle) }, 0);
        }

        #[test]
        fn clipboard_retry_recovers_from_transient_contention() {
            let attempts = Cell::new(0);
            let result = retry_clipboard_operation(
                Duration::from_secs(1),
                Duration::ZERO,
                || false,
                || {
                    let attempt = attempts.get() + 1;
                    attempts.set(attempt);
                    if attempt < 3 {
                        Err(io::Error::new(io::ErrorKind::WouldBlock, "clipboard busy"))
                    } else {
                        Ok(42)
                    }
                },
            )
            .unwrap();

            assert_eq!(result, Some(42));
            assert_eq!(attempts.get(), 3);
        }

        #[test]
        fn clipboard_retry_checks_cancellation_before_another_attempt() {
            let attempts = Cell::new(0);
            let result = retry_clipboard_operation(
                Duration::from_secs(1),
                Duration::ZERO,
                || attempts.get() == 1,
                || -> io::Result<()> {
                    attempts.set(attempts.get() + 1);
                    Err(io::Error::new(io::ErrorKind::WouldBlock, "clipboard busy"))
                },
            )
            .unwrap();

            assert_eq!(result, None);
            assert_eq!(attempts.get(), 1);
        }

        #[test]
        fn clipboard_retry_attempts_once_when_the_budget_is_zero() {
            let attempts = Cell::new(0);
            let error = retry_clipboard_operation(
                Duration::ZERO,
                Duration::from_millis(5),
                || false,
                || -> io::Result<()> {
                    attempts.set(attempts.get() + 1);
                    Err(io::Error::new(io::ErrorKind::WouldBlock, "clipboard busy"))
                },
            )
            .unwrap_err();

            assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
            assert_eq!(attempts.get(), 1);
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::{CaptureFrame, ClipboardCommitGate};
    use std::io;

    pub fn copy_image(_frame: &CaptureFrame) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "image clipboard is currently Windows-only",
        ))
    }

    pub fn copy_image_cancellable(
        frame: &CaptureFrame,
        gate: &dyn ClipboardCommitGate,
    ) -> io::Result<bool> {
        if gate.is_cancelled() || !gate.begin_clipboard_commit() {
            return Ok(false);
        }
        let result = copy_image(frame);
        gate.finish_clipboard_commit();
        result.map(|()| true)
    }

    pub fn read_image() -> io::Result<CaptureFrame> {
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
    #[cfg(windows)]
    use super::platform::utf16_bytes;
    use super::{encode_dib, frame_from_rgba};
    use flash_shot_domain::domain::geometry::PhysicalRect;
    use flash_shot_image::{CaptureFrame, PixelFormat};
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

    #[test]
    fn clipboard_rgba_pixels_convert_to_a_valid_bgra_frame() {
        let frame = frame_from_rgba(2, 1, &[3, 2, 1, 255, 6, 5, 4, 128]).unwrap();

        assert_eq!((frame.width, frame.height, frame.stride), (2, 1, 8));
        assert_eq!(frame.bounds.right, 2);
        assert_eq!(frame.bounds.bottom, 1);
        assert_eq!(frame.pixels.as_ref(), &[1, 2, 3, 255, 4, 5, 6, 128]);
    }

    #[test]
    fn clipboard_image_conversion_rejects_truncated_pixels() {
        let error = frame_from_rgba(2, 1, &[0; 4]).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
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
