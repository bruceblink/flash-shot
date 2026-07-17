//! Windows notification-area entry with capture and quit commands.

use async_channel::Receiver;
use std::io;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrayEvent {
    CaptureRequested,
    QuitRequested,
}

pub struct TrayService {
    listener: platform::TrayListener,
}

impl TrayService {
    pub fn start() -> io::Result<(Self, Receiver<TrayEvent>)> {
        let (listener, events) = platform::TrayListener::start()?;
        Ok((Self { listener }, events))
    }

    pub fn is_active(&self) -> bool {
        self.listener.is_active()
    }
}

#[cfg(windows)]
mod platform {
    use super::TrayEvent;
    use async_channel::Receiver;
    use std::{
        io,
        mem::size_of,
        ptr,
        sync::mpsc::{self, SyncSender},
        thread::{self, JoinHandle},
    };
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, POINT, WPARAM},
        System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
        UI::{
            Shell::{
                NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
                Shell_NotifyIconW,
            },
            WindowsAndMessaging::{
                AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
                DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW, HWND_MESSAGE,
                IDI_APPLICATION, LoadIconW, MF_STRING, MSG, PostThreadMessageW, RegisterClassW,
                SetForegroundWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu,
                TranslateMessage, WM_APP, WM_LBUTTONDBLCLK, WM_QUIT, WM_RBUTTONUP, WNDCLASSW,
            },
        },
    };

    const ICON_ID: u32 = 1;
    const TRAY_CALLBACK: u32 = WM_APP + 1;
    const MENU_CAPTURE: usize = 1;
    const MENU_QUIT: usize = 2;
    const WINDOW_CLASS: &str = "FlashShot.TrayWindow";

    pub struct TrayListener {
        thread_id: u32,
        thread: Option<JoinHandle<()>>,
    }

    impl TrayListener {
        pub fn start() -> io::Result<(Self, Receiver<TrayEvent>)> {
            let (event_tx, event_rx) = async_channel::bounded(4);
            let (ready_tx, ready_rx) = mpsc::sync_channel(1);
            let thread = thread::Builder::new()
                .name("flash-shot-tray".to_owned())
                .spawn(move || message_loop(event_tx, ready_tx))?;
            match ready_rx.recv() {
                Ok(Ok(thread_id)) => Ok((
                    Self {
                        thread_id,
                        thread: Some(thread),
                    },
                    event_rx,
                )),
                Ok(Err(error)) => {
                    let _ = thread.join();
                    Err(error)
                }
                Err(_) => {
                    let _ = thread.join();
                    Err(io::Error::other("tray listener stopped during startup"))
                }
            }
        }

        pub const fn is_active(&self) -> bool {
            self.thread.is_some()
        }
    }

    impl Drop for TrayListener {
        fn drop(&mut self) {
            if let Some(thread) = self.thread.take() {
                // SAFETY: thread_id identifies the listener thread and WM_QUIT ends its loop.
                unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0 as WPARAM, 0 as LPARAM) };
                if thread.join().is_err() {
                    log::warn!(target: "flash_shot::tray", "tray_thread_join_failed");
                }
            }
        }
    }

    fn message_loop(events: async_channel::Sender<TrayEvent>, ready: SyncSender<io::Result<u32>>) {
        let result = unsafe { create_tray() };
        let (window, icon) = match result {
            Ok(value) => value,
            Err(error) => {
                let _ = ready.send(Err(error));
                return;
            }
        };
        // SAFETY: called on the listener thread.
        let thread_id = unsafe { GetCurrentThreadId() };
        if ready.send(Ok(thread_id)).is_err() {
            unsafe { remove_tray(window, &icon) };
            return;
        }

        let mut message = MSG::default();
        loop {
            // SAFETY: message is a valid output buffer and the filter is unrestricted.
            let result = unsafe { GetMessageW(&mut message, ptr::null_mut(), 0, 0) };
            if result <= 0 {
                break;
            }
            if message.message == TRAY_CALLBACK {
                handle_tray_message(window, message.lParam as u32, &events);
            } else {
                unsafe {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        }
        unsafe { remove_tray(window, &icon) };
    }

    unsafe fn create_tray() -> io::Result<(HWND, NOTIFYICONDATAW)> {
        let class = wide(WINDOW_CLASS);
        let instance = unsafe { GetModuleHandleW(ptr::null()) };
        if instance.is_null() {
            return Err(io::Error::last_os_error());
        }
        let window_class = WNDCLASSW {
            lpfnWndProc: Some(DefWindowProcW),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..Default::default()
        };
        if unsafe { RegisterClassW(&window_class) } == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(1410) {
                return Err(error);
            }
        }
        let window = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                ptr::null_mut(),
                instance,
                ptr::null(),
            )
        };
        if window.is_null() {
            return Err(io::Error::last_os_error());
        }

        let app_icon = unsafe { LoadIconW(instance, ptr::without_provenance(1)) };
        let icon_handle = if app_icon.is_null() {
            unsafe { LoadIconW(ptr::null_mut(), IDI_APPLICATION) }
        } else {
            app_icon
        };
        let mut icon = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: window,
            uID: ICON_ID,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: TRAY_CALLBACK,
            hIcon: icon_handle,
            ..Default::default()
        };
        copy_wide(&mut icon.szTip, "Flash Shot");
        if unsafe { Shell_NotifyIconW(NIM_ADD, &icon) } == 0 {
            unsafe { DestroyWindow(window) };
            return Err(io::Error::last_os_error());
        }
        Ok((window, icon))
    }

    fn handle_tray_message(window: HWND, message: u32, events: &async_channel::Sender<TrayEvent>) {
        match message {
            WM_LBUTTONDBLCLK => {
                let _ = events.try_send(TrayEvent::CaptureRequested);
            }
            WM_RBUTTONUP => {
                if let Some(event) = show_menu(window) {
                    let _ = events.try_send(event);
                }
            }
            _ => {}
        }
    }

    fn show_menu(window: HWND) -> Option<TrayEvent> {
        // SAFETY: menu is owned here and destroyed before return.
        let menu = unsafe { CreatePopupMenu() };
        if menu.is_null() {
            return None;
        }
        let capture = wide("Capture");
        let quit = wide("Quit Flash Shot");
        unsafe {
            AppendMenuW(menu, MF_STRING, MENU_CAPTURE, capture.as_ptr());
            AppendMenuW(menu, MF_STRING, MENU_QUIT, quit.as_ptr());
            SetForegroundWindow(window);
        }
        let mut cursor = POINT::default();
        let command = if unsafe { GetCursorPos(&mut cursor) } != 0 {
            unsafe {
                TrackPopupMenu(
                    menu,
                    TPM_RETURNCMD | TPM_RIGHTBUTTON,
                    cursor.x,
                    cursor.y,
                    0,
                    window,
                    ptr::null(),
                )
            }
        } else {
            0
        };
        unsafe { DestroyMenu(menu) };
        match command as usize {
            MENU_CAPTURE => Some(TrayEvent::CaptureRequested),
            MENU_QUIT => Some(TrayEvent::QuitRequested),
            _ => None,
        }
    }

    unsafe fn remove_tray(window: HWND, icon: &NOTIFYICONDATAW) {
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, icon);
            DestroyWindow(window);
        }
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }

    fn copy_wide(target: &mut [u16], value: &str) {
        let encoded: Vec<u16> = value.encode_utf16().collect();
        let length = encoded.len().min(target.len().saturating_sub(1));
        target[..length].copy_from_slice(&encoded[..length]);
        target[length] = 0;
    }
}

#[cfg(not(windows))]
mod platform {
    use super::TrayEvent;
    use async_channel::Receiver;
    use std::io;

    pub struct TrayListener;

    impl TrayListener {
        pub fn start() -> io::Result<(Self, Receiver<TrayEvent>)> {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "tray integration is currently Windows-only",
            ))
        }

        pub const fn is_active(&self) -> bool {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::TrayService;

    #[cfg(windows)]
    #[test]
    fn tray_starts_and_stops_cleanly() {
        let (tray, _events) = TrayService::start().unwrap();
        assert!(tray.is_active());
        drop(tray);
    }
}
