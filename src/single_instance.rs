//! Process-level single-instance ownership.

use std::io;

#[cfg(windows)]
mod platform {
    use super::io;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE},
        System::Threading::CreateMutexW,
    };

    const MUTEX_NAME: &str = "Local\\BruceBlink.FlashShot.SingleInstance";

    pub struct SingleInstance {
        handle: HANDLE,
    }

    impl SingleInstance {
        pub fn acquire() -> io::Result<Option<Self>> {
            let name: Vec<u16> = MUTEX_NAME.encode_utf16().chain(Some(0)).collect();
            // SAFETY: the name is NUL terminated and the returned handle is owned here.
            let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }

            // SAFETY: GetLastError is read immediately after CreateMutexW.
            if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                // SAFETY: CreateMutexW returned this valid handle to us.
                unsafe { CloseHandle(handle) };
                return Ok(None);
            }
            Ok(Some(Self { handle }))
        }
    }

    impl Drop for SingleInstance {
        fn drop(&mut self) {
            // SAFETY: this handle is owned by the guard and closed exactly once.
            unsafe { CloseHandle(self.handle) };
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::io;

    pub struct SingleInstance;

    impl SingleInstance {
        pub fn acquire() -> io::Result<Option<Self>> {
            Ok(Some(Self))
        }
    }
}

pub use platform::SingleInstance;
