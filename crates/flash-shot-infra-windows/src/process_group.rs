//! Owns a recording process tree for deterministic cleanup on Windows.

use std::{io, process::Child};

#[cfg(windows)]
mod platform {
    use super::{Child, io};
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject,
        },
    };

    pub struct ProcessGroup {
        handle: HANDLE,
    }

    impl ProcessGroup {
        pub fn create() -> io::Result<Self> {
            // SAFETY: no name is supplied and the returned handle is owned by this value.
            let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            // SAFETY: limits has the exact type and remains valid for the duration of the call.
            let configured = unsafe {
                SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as *const _,
                    std::mem::size_of_val(&limits) as u32,
                )
            };
            if configured == 0 {
                // SAFETY: this handle was returned by CreateJobObjectW and is not yet closed.
                unsafe { CloseHandle(handle) };
                return Err(io::Error::last_os_error());
            }
            Ok(Self { handle })
        }

        pub fn assign(&self, child: &Child) -> io::Result<()> {
            // SAFETY: the job handle and child process handle are valid for this call.
            if unsafe { AssignProcessToJobObject(self.handle, child.as_raw_handle()) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }

        pub fn terminate(&self) -> io::Result<()> {
            // SAFETY: this valid job handle is owned by this value.
            if unsafe { TerminateJobObject(self.handle, 1) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
    }

    impl Drop for ProcessGroup {
        fn drop(&mut self) {
            // SAFETY: close is called once for this owned Job Object handle. The configured limit
            // tears down any remaining child processes in the group.
            unsafe { CloseHandle(self.handle) };
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::{Child, io};

    pub struct ProcessGroup;

    impl ProcessGroup {
        pub fn create() -> io::Result<Self> {
            Ok(Self)
        }

        pub fn assign(&self, _child: &Child) -> io::Result<()> {
            Ok(())
        }

        pub fn terminate(&self) -> io::Result<()> {
            Ok(())
        }
    }
}

pub use platform::ProcessGroup;
