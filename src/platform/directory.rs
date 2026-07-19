//! Opens user-visible directories without invoking a command shell.

use std::{io, path::Path};

/// Opens an existing directory in the system file manager.
pub fn open(path: &Path) -> io::Result<()> {
    platform::open(path)
}

#[cfg(windows)]
mod platform {
    use std::{io, path::Path};
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};

    pub fn open(path: &Path) -> io::Result<()> {
        if !path.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "directory does not exist",
            ));
        }
        let path = wide_path(path)?;
        // SAFETY: all pointers refer to NUL-terminated UTF-16 strings that outlive this call.
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                std::ptr::null(),
                path.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        } as isize;
        if result <= 32 {
            return Err(io::Error::from_raw_os_error(result as i32));
        }
        Ok(())
    }

    fn wide_path(path: &Path) -> io::Result<Vec<u16>> {
        let path = path.as_os_str().to_string_lossy();
        if path.encode_utf16().any(|unit| unit == 0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "directory path contains a NUL character",
            ));
        }
        Ok(path.encode_utf16().chain(Some(0)).collect())
    }

    #[cfg(test)]
    mod tests {
        use super::wide_path;
        use std::path::Path;

        #[test]
        fn directory_paths_are_encoded_as_nul_terminated_utf16() {
            let encoded = wide_path(Path::new(r"C:\Users\Example\Flash Shot")).unwrap();

            assert_eq!(encoded.last(), Some(&0));
            assert_eq!(
                String::from_utf16(&encoded[..encoded.len() - 1]).unwrap(),
                r"C:\Users\Example\Flash Shot"
            );
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use std::{io, path::Path};

    pub fn open(_path: &Path) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "opening directories is currently Windows-only",
        ))
    }
}
