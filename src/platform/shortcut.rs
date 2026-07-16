//! Process-wide screenshot hotkey registration and lifecycle.

use async_channel::Receiver;
use std::io;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShortcutEvent {
    CaptureRequested,
}

pub struct GlobalShortcutService {
    listener: platform::ShortcutListener,
}

impl GlobalShortcutService {
    pub fn register_capture() -> io::Result<(Self, Receiver<ShortcutEvent>)> {
        let (listener, events) = platform::ShortcutListener::register()?;
        Ok((Self { listener }, events))
    }

    pub fn is_active(&self) -> bool {
        self.listener.is_active()
    }
}

#[cfg(windows)]
mod platform {
    use super::ShortcutEvent;
    use async_channel::Receiver;
    use std::{
        io, ptr,
        sync::mpsc::{self, SyncSender},
        thread::{self, JoinHandle},
    };
    use windows_sys::Win32::{
        Foundation::{LPARAM, WPARAM},
        System::Threading::GetCurrentThreadId,
        UI::{
            Input::KeyboardAndMouse::{
                MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, RegisterHotKey, UnregisterHotKey, VK_SNAPSHOT,
            },
            WindowsAndMessaging::{GetMessageW, MSG, PostThreadMessageW, WM_HOTKEY, WM_QUIT},
        },
    };

    const HOTKEY_ID: i32 = 1;

    pub struct ShortcutListener {
        thread_id: u32,
        thread: Option<JoinHandle<()>>,
    }

    impl ShortcutListener {
        pub fn register() -> io::Result<(Self, Receiver<ShortcutEvent>)> {
            let (event_tx, event_rx) = async_channel::bounded(1);
            let (ready_tx, ready_rx) = mpsc::sync_channel(1);
            let thread = thread::Builder::new()
                .name("flash-shot-hotkey".to_owned())
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
                    Err(io::Error::other("hotkey listener stopped during startup"))
                }
            }
        }

        pub const fn is_active(&self) -> bool {
            self.thread.is_some()
        }
    }

    impl Drop for ShortcutListener {
        fn drop(&mut self) {
            if let Some(thread) = self.thread.take() {
                // SAFETY: thread_id identifies the listener thread and WM_QUIT ends its loop.
                unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0 as WPARAM, 0 as LPARAM) };
                if thread.join().is_err() {
                    log::warn!(target: "flash_shot::shortcut", "hotkey_thread_join_failed");
                }
            }
        }
    }

    fn message_loop(
        events: async_channel::Sender<ShortcutEvent>,
        ready: SyncSender<io::Result<u32>>,
    ) {
        // SAFETY: called from the listener thread itself.
        let thread_id = unsafe { GetCurrentThreadId() };
        // SAFETY: a null HWND registers a thread-level hotkey.
        if unsafe {
            RegisterHotKey(
                ptr::null_mut(),
                HOTKEY_ID,
                MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT,
                VK_SNAPSHOT as u32,
            )
        } == 0
        {
            let _ = ready.send(Err(io::Error::last_os_error()));
            return;
        }
        if ready.send(Ok(thread_id)).is_err() {
            // SAFETY: balances the successful registration on this thread.
            unsafe { UnregisterHotKey(ptr::null_mut(), HOTKEY_ID) };
            return;
        }

        let mut message = MSG::default();
        loop {
            // SAFETY: message is a valid output buffer and the filter is unrestricted.
            let result = unsafe { GetMessageW(&mut message, ptr::null_mut(), 0, 0) };
            if result <= 0 {
                break;
            }
            if message.message == WM_HOTKEY && message.wParam == HOTKEY_ID as usize {
                let _ = events.try_send(ShortcutEvent::CaptureRequested);
            }
        }
        // SAFETY: balances the successful registration on this thread.
        unsafe { UnregisterHotKey(ptr::null_mut(), HOTKEY_ID) };
    }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::GlobalShortcutService;

    #[cfg(windows)]
    #[test]
    fn capture_hotkey_registers_and_stops_cleanly() {
        let (service, _events) = GlobalShortcutService::register_capture().unwrap();
        assert!(service.is_active());
        drop(service);
    }
}

#[cfg(not(windows))]
mod platform {
    use super::ShortcutEvent;
    use async_channel::Receiver;
    use std::io;

    pub struct ShortcutListener;

    impl ShortcutListener {
        pub fn register() -> io::Result<(Self, Receiver<ShortcutEvent>)> {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "global shortcuts are currently Windows-only",
            ))
        }

        pub const fn is_active(&self) -> bool {
            false
        }
    }
}
