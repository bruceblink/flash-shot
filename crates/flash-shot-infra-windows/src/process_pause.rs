//! Windows process pause/resume implemented through owned thread handles.

use std::io;

#[cfg(windows)]
mod platform {
    use super::io;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First,
                Thread32Next,
            },
            Threading::{OpenThread, ResumeThread, SuspendThread, THREAD_SUSPEND_RESUME},
        },
    };

    pub fn set_paused(process_id: u32, paused: bool) -> io::Result<()> {
        // SAFETY: the snapshot is owned locally and closed before return.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };
        // SAFETY: entry is correctly initialized and valid for the duration of enumeration.
        let mut found = unsafe { Thread32First(snapshot, &mut entry) } != 0;
        while found {
            if entry.th32OwnerProcessID == process_id {
                // SAFETY: the resulting thread handle, if any, is closed below.
                let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                if !thread.is_null() {
                    // SAFETY: this is an owned thread handle with suspend/resume access.
                    let result = unsafe {
                        if paused {
                            SuspendThread(thread)
                        } else {
                            ResumeThread(thread)
                        }
                    };
                    // SAFETY: OpenThread returned this handle and it has not been closed.
                    unsafe { CloseHandle(thread) };
                    if result == u32::MAX {
                        // SAFETY: snapshot is still valid and owned here.
                        unsafe { CloseHandle(snapshot) };
                        return Err(io::Error::last_os_error());
                    }
                }
            }
            entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
            // SAFETY: entry remains valid and snapshot stays open during enumeration.
            found = unsafe { Thread32Next(snapshot, &mut entry) } != 0;
        }
        // SAFETY: the snapshot was successfully created and is closed exactly once here.
        unsafe { CloseHandle(snapshot) };
        Ok(())
    }
}

#[cfg(not(windows))]
mod platform {
    use super::io;

    pub fn set_paused(_process_id: u32, _paused: bool) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "process pause is currently Windows-only",
        ))
    }
}

pub use platform::set_paused;
