//! One-page native printing for immutable screenshot frames.
//!
//! Windows owns printer selection and device configuration through its print dialog. This module
//! only supplies a bounded, opaque BGRA snapshot and submits that snapshot as a single GDI page.

use std::io;

use flash_shot_image::CaptureFrame;

const MAX_PRINT_PIXELS: u64 = 64_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrintOutcome {
    Submitted,
    Cancelled,
}

/// Opens the Windows printer chooser and submits a single-page image job.
///
/// The caller must run this blocking function away from its UI thread. A private STA thread owns
/// the modal common dialog and synchronous spooler calls, while this function joins it so all
/// native handles and the pixel snapshot have one bounded lifetime.
#[cfg(windows)]
pub fn print_image(owner_hwnd: isize, frame: CaptureFrame) -> io::Result<PrintOutcome> {
    frame.validate()?;
    if owner_hwnd == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "print owner window handle is null",
        ));
    }

    std::thread::Builder::new()
        .name("flash-shot-print".to_owned())
        .spawn(move || print_image_on_sta(owner_hwnd, frame))?
        .join()
        .map_err(|_| io::Error::other("Windows print worker panicked"))?
}

/// Reports printing as unavailable where no Windows print dialog backend exists.
#[cfg(not(windows))]
pub fn print_image(_owner_hwnd: isize, _frame: CaptureFrame) -> io::Result<PrintOutcome> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "native screenshot printing is available only on Windows",
    ))
}

#[cfg(windows)]
fn print_image_on_sta(owner_hwnd: isize, frame: CaptureFrame) -> io::Result<PrintOutcome> {
    use std::mem::size_of;

    use windows_sys::Win32::{
        Foundation::HWND,
        System::Ole::OleInitialize,
        UI::{Controls::Dialogs::*, WindowsAndMessaging::IsWindow},
    };

    let apartment_status = unsafe { OleInitialize(std::ptr::null_mut()) };
    if apartment_status < 0 {
        return Err(hresult_error(
            "initialize print dialog apartment",
            apartment_status,
        ));
    }
    let _apartment = OleApartment;

    let owner = owner_hwnd as HWND;
    if unsafe { IsWindow(owner) } == 0 {
        return Ok(PrintOutcome::Cancelled);
    }

    let mut dialog = PRINTDLGEXW {
        lStructSize: size_of::<PRINTDLGEXW>() as u32,
        hwndOwner: owner,
        Flags: PD_RETURNDC
            | PD_NOPAGENUMS
            | PD_NOSELECTION
            | PD_NOCURRENTPAGE
            | PD_USEDEVMODECOPIESANDCOLLATE,
        nMinPage: 1,
        nMaxPage: 1,
        nCopies: 1,
        ..Default::default()
    };
    // PrintDlgExW is a modal system chooser; every returned global handle and printer DC is
    // released by DialogResources, including when the user cancels or the API reports failure.
    let resources = DialogResources::new(&mut dialog);
    let status = unsafe { PrintDlgExW(&mut dialog) };
    if status < 0 {
        return Err(hresult_error("open Windows print dialog", status));
    }
    if dialog.dwResultAction == PD_RESULT_CANCEL || unsafe { IsWindow(owner) } == 0 {
        return Ok(PrintOutcome::Cancelled);
    }
    if dialog.dwResultAction != PD_RESULT_PRINT || dialog.hDC.is_null() {
        return Err(io::Error::other(
            "Windows print dialog returned no selected printer",
        ));
    }

    let _owner_input = match OwnerInputGuard::disable(owner) {
        Ok(guard) => guard,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(PrintOutcome::Cancelled);
        }
        Err(error) => return Err(error),
    };
    submit_one_page(dialog.hDC, &frame)?;
    drop(resources);
    Ok(PrintOutcome::Submitted)
}

#[cfg(windows)]
struct OwnerInputGuard {
    hwnd: windows_sys::Win32::Foundation::HWND,
    was_enabled: bool,
}

#[cfg(windows)]
impl OwnerInputGuard {
    fn disable(hwnd: windows_sys::Win32::Foundation::HWND) -> io::Result<Self> {
        use windows_sys::Win32::UI::{
            Input::KeyboardAndMouse::{EnableWindow, IsWindowEnabled},
            WindowsAndMessaging::IsWindow,
        };

        if unsafe { IsWindow(hwnd) } == 0 {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "print owner window closed before job submission",
            ));
        }
        let was_enabled = unsafe { IsWindowEnabled(hwnd) } != 0;
        if was_enabled {
            // Keep the owner disabled after PrintDlgExW returns while synchronous spooler calls
            // run, so the rest of the app cannot accept input during the submitted operation.
            unsafe { EnableWindow(hwnd, 0) };
        }
        Ok(Self { hwnd, was_enabled })
    }
}

#[cfg(windows)]
impl Drop for OwnerInputGuard {
    fn drop(&mut self) {
        use windows_sys::Win32::UI::{
            Input::KeyboardAndMouse::EnableWindow, WindowsAndMessaging::IsWindow,
        };

        if self.was_enabled && unsafe { IsWindow(self.hwnd) } != 0 {
            // SAFETY: the HWND was enabled when this guard was created and is still a live window.
            unsafe { EnableWindow(self.hwnd, 1) };
        }
    }
}

#[cfg(windows)]
struct OleApartment;

#[cfg(windows)]
impl Drop for OleApartment {
    fn drop(&mut self) {
        use windows_sys::Win32::System::Ole::OleUninitialize;

        unsafe { OleUninitialize() };
    }
}

#[cfg(windows)]
struct DialogResources {
    dialog: *mut windows_sys::Win32::UI::Controls::Dialogs::PRINTDLGEXW,
}

#[cfg(windows)]
impl DialogResources {
    fn new(dialog: &mut windows_sys::Win32::UI::Controls::Dialogs::PRINTDLGEXW) -> Self {
        Self { dialog }
    }
}

#[cfg(windows)]
impl Drop for DialogResources {
    fn drop(&mut self) {
        use windows_sys::Win32::{Foundation::GlobalFree, Graphics::Gdi::DeleteDC};

        // SAFETY: PrintDlgExW owns these allocations until it returns. This guard runs after the
        // call and releases only non-null output handles exactly once.
        unsafe {
            let dialog = &mut *self.dialog;
            if !dialog.hDC.is_null() {
                DeleteDC(dialog.hDC);
                dialog.hDC = std::ptr::null_mut();
            }
            if !dialog.hDevMode.is_null() {
                GlobalFree(dialog.hDevMode);
                dialog.hDevMode = std::ptr::null_mut();
            }
            if !dialog.hDevNames.is_null() {
                GlobalFree(dialog.hDevNames);
                dialog.hDevNames = std::ptr::null_mut();
            }
        }
    }
}

#[cfg(windows)]
fn submit_one_page(
    device_context: windows_sys::Win32::Graphics::Gdi::HDC,
    frame: &CaptureFrame,
) -> io::Result<()> {
    use std::mem::size_of;

    use windows_sys::Win32::{
        Graphics::Gdi::{
            DIB_RGB_COLORS, GetDeviceCaps, HORZRES, SRCCOPY, STRETCH_HALFTONE, SetStretchBltMode,
            StretchDIBits, VERTRES,
        },
        Storage::Xps::{DOCINFOW, EndDoc, EndPage, StartDocW, StartPage},
    };

    let printable_width = unsafe { GetDeviceCaps(device_context, HORZRES as i32) };
    let printable_height = unsafe { GetDeviceCaps(device_context, VERTRES as i32) };
    let (destination, bitmap) = prepare_bitmap(frame, printable_width, printable_height)?;
    let doc_name = windows_sys::core::w!("Flash Shot screenshot");
    let document = DOCINFOW {
        cbSize: size_of::<DOCINFOW>() as i32,
        lpszDocName: doc_name,
        ..Default::default()
    };
    if unsafe { StartDocW(device_context, &document) } <= 0 {
        return Err(last_error("start screenshot print job"));
    }

    let page_result = (|| {
        if unsafe { StartPage(device_context) } <= 0 {
            return Err(last_error("start screenshot print page"));
        }
        if unsafe { SetStretchBltMode(device_context, STRETCH_HALFTONE) } == 0 {
            return Err(last_error("configure screenshot print scaling"));
        }
        let copied_lines = unsafe {
            StretchDIBits(
                device_context,
                destination.left,
                destination.top,
                destination.width,
                destination.height,
                0,
                0,
                bitmap.info.bmiHeader.biWidth,
                bitmap.info.bmiHeader.biHeight.abs(),
                bitmap.pixels.as_ptr().cast(),
                &bitmap.info,
                DIB_RGB_COLORS,
                SRCCOPY,
            )
        };
        if copied_lines <= 0 {
            return Err(last_error("draw screenshot print page"));
        }
        if unsafe { EndPage(device_context) } <= 0 {
            return Err(last_error("finish screenshot print page"));
        }
        Ok(())
    })();
    if let Err(error) = page_result {
        unsafe { windows_sys::Win32::Storage::Xps::AbortDoc(device_context) };
        return Err(error);
    }
    if unsafe { EndDoc(device_context) } <= 0 {
        unsafe { windows_sys::Win32::Storage::Xps::AbortDoc(device_context) };
        return Err(last_error("submit screenshot print job"));
    }
    Ok(())
}

#[cfg(windows)]
struct PreparedBitmap {
    info: windows_sys::Win32::Graphics::Gdi::BITMAPINFO,
    pixels: Vec<u8>,
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PrintRect {
    left: i32,
    top: i32,
    width: i32,
    height: i32,
}

#[cfg(windows)]
fn prepare_bitmap(
    frame: &CaptureFrame,
    printable_width: i32,
    printable_height: i32,
) -> io::Result<(PrintRect, PreparedBitmap)> {
    use windows_sys::Win32::Graphics::Gdi::{BI_RGB, BITMAPINFO, BITMAPINFOHEADER, RGBQUAD};

    frame.validate()?;
    if frame.width == 0 || frame.height == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "cannot print an empty image",
        ));
    }
    if u64::from(frame.width) * u64::from(frame.height) > MAX_PRINT_PIXELS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "image exceeds the 64 megapixel print limit",
        ));
    }
    if printable_width <= 0 || printable_height <= 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "selected printer reports an empty printable area",
        ));
    }

    let width = i32::try_from(frame.width)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "print image width overflow"))?;
    let height = i32::try_from(frame.height)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "print image height overflow"))?;
    let row_bytes = usize::try_from(frame.width)
        .ok()
        .and_then(|width| width.checked_mul(4))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "print row size overflow"))?;
    let pixel_bytes = row_bytes
        .checked_mul(frame.height as usize)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "print image size overflow"))?;
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(pixel_bytes)
        .map_err(|error| io::Error::other(format!("allocate print snapshot: {error}")))?;
    pixels.resize(pixel_bytes, 255);
    for row in 0..frame.height as usize {
        let source = &frame.pixels[row * frame.stride..row * frame.stride + row_bytes];
        let destination = &mut pixels[row * row_bytes..(row + 1) * row_bytes];
        let (source_pixels, _) = source.as_chunks::<4>();
        let (destination_pixels, _) = destination.as_chunks_mut::<4>();
        for (source_pixel, output_pixel) in source_pixels.iter().zip(destination_pixels) {
            let alpha = u16::from(source_pixel[3]);
            let inverse_alpha = 255 - alpha;
            for channel in 0..3 {
                output_pixel[channel] =
                    ((u16::from(source_pixel[channel]) * alpha + 255 * inverse_alpha + 127) / 255)
                        as u8;
            }
        }
    }

    let fitted = fit_print_rect(width, height, printable_width, printable_height)?;
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            biSizeImage: u32::try_from(pixel_bytes).unwrap_or(u32::MAX),
            ..Default::default()
        },
        bmiColors: [RGBQUAD::default()],
    };
    Ok((fitted, PreparedBitmap { info, pixels }))
}

#[cfg(windows)]
fn fit_print_rect(
    image_width: i32,
    image_height: i32,
    printable_width: i32,
    printable_height: i32,
) -> io::Result<PrintRect> {
    if image_width <= 0 || image_height <= 0 || printable_width <= 0 || printable_height <= 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "print image and printable area must be non-empty",
        ));
    }
    let scale = (printable_width as f64 / image_width as f64)
        .min(printable_height as f64 / image_height as f64);
    let width = (image_width as f64 * scale)
        .round()
        .clamp(1.0, printable_width as f64) as i32;
    let height = (image_height as f64 * scale)
        .round()
        .clamp(1.0, printable_height as f64) as i32;
    Ok(PrintRect {
        left: (printable_width - width) / 2,
        top: (printable_height - height) / 2,
        width,
        height,
    })
}

#[cfg(windows)]
fn hresult_error(operation: &str, status: windows_sys::core::HRESULT) -> io::Error {
    io::Error::other(format!(
        "{operation} failed with HRESULT 0x{:08X}",
        status as u32
    ))
}

#[cfg(windows)]
fn last_error(operation: &str) -> io::Error {
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(0) {
        io::Error::other(operation.to_owned())
    } else {
        io::Error::new(error.kind(), format!("{operation}: {error}"))
    }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::{PrintRect, fit_print_rect, prepare_bitmap};
    #[cfg(windows)]
    use flash_shot_image::{CaptureFrame, PixelFormat};
    #[cfg(windows)]
    use std::sync::Arc;

    #[cfg(windows)]
    fn frame(width: u32, height: u32, stride: usize, pixels: Vec<u8>) -> CaptureFrame {
        CaptureFrame {
            bounds: flash_shot_domain::domain::geometry::PhysicalRect {
                left: 0,
                top: 0,
                right: width as i32,
                bottom: height as i32,
            },
            width,
            height,
            stride,
            format: PixelFormat::Bgra8,
            pixels: Arc::from(pixels),
            capture_duration: std::time::Duration::ZERO,
            cpu_copy_count: 0,
        }
    }

    #[cfg(windows)]
    #[test]
    fn print_rect_preserves_aspect_ratio_and_centers_in_printable_area() {
        let rect = fit_print_rect(1600, 900, 1000, 1000).unwrap();

        assert_eq!(
            rect,
            PrintRect {
                left: 0,
                top: 218,
                width: 1000,
                height: 563,
            }
        );
    }

    #[cfg(windows)]
    #[test]
    fn print_rect_rejects_empty_images_or_printer_areas() {
        assert!(fit_print_rect(0, 1, 1, 1).is_err());
        assert!(fit_print_rect(1, 1, 0, 1).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn print_bitmap_flattens_alpha_on_white_and_ignores_source_row_padding() {
        let source = frame(2, 1, 12, vec![0, 0, 255, 128, 20, 30, 40, 0, 9, 8, 7, 6]);
        let (_, bitmap) = prepare_bitmap(&source, 100, 100).unwrap();

        assert_eq!(
            &bitmap.pixels[..8],
            &[127, 127, 255, 255, 255, 255, 255, 255]
        );
        assert_eq!(bitmap.info.bmiHeader.biHeight, -1);
        assert_eq!(bitmap.info.bmiHeader.biSizeImage, 8);
    }
}
