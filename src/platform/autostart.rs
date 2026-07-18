//! Per-user Windows sign-in launch configuration.

use std::{io, path::Path};

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const VALUE_NAME: &str = "Flash Shot";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AutoStartState {
    Enabled,
    Disabled,
    ManagedByAnotherExecutable,
}

pub trait AutoStartService {
    fn state(&self, executable: &Path) -> io::Result<AutoStartState>;
    fn set_enabled(&self, executable: &Path, enabled: bool) -> io::Result<AutoStartState>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemAutoStart;

impl AutoStartService for SystemAutoStart {
    fn state(&self, executable: &Path) -> io::Result<AutoStartState> {
        platform::state(executable)
    }

    fn set_enabled(&self, executable: &Path, enabled: bool) -> io::Result<AutoStartState> {
        platform::set_enabled(executable, enabled)
    }
}

pub fn command_for(executable: &Path) -> io::Result<String> {
    if executable.as_os_str().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "auto-start executable path cannot be empty",
        ));
    }
    Ok(format!("\"{}\"", executable.display()))
}

fn is_our_command(value: &str, executable: &Path) -> io::Result<bool> {
    Ok(value.eq_ignore_ascii_case(&command_for(executable)?))
}

fn state_for_value(value: Option<&str>, executable: &Path) -> io::Result<AutoStartState> {
    match value {
        Some(value) if is_our_command(value, executable)? => Ok(AutoStartState::Enabled),
        Some(_) => Ok(AutoStartState::ManagedByAnotherExecutable),
        None => Ok(AutoStartState::Disabled),
    }
}

#[cfg(windows)]
mod platform {
    use super::{
        AutoStartState, RUN_KEY, VALUE_NAME, command_for, is_our_command, state_for_value,
    };
    use std::{io, path::Path};
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};

    pub fn state(executable: &Path) -> io::Result<AutoStartState> {
        let run = RegKey::predef(HKEY_CURRENT_USER).open_subkey(RUN_KEY);
        let Ok(run) = run else {
            return Ok(AutoStartState::Disabled);
        };
        match run.get_value::<String, _>(VALUE_NAME) {
            Ok(value) => state_for_value(Some(&value), executable),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                state_for_value(None, executable)
            }
            Err(error) => Err(error),
        }
    }

    pub fn set_enabled(executable: &Path, enabled: bool) -> io::Result<AutoStartState> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if enabled {
            if state(executable)? == AutoStartState::ManagedByAnotherExecutable {
                return Ok(AutoStartState::ManagedByAnotherExecutable);
            }
            let (run, _) = hkcu.create_subkey(RUN_KEY)?;
            run.set_value(VALUE_NAME, &command_for(executable)?)?;
            return Ok(AutoStartState::Enabled);
        }

        let run = match hkcu
            .open_subkey_with_flags(RUN_KEY, winreg::enums::KEY_WRITE | winreg::enums::KEY_READ)
        {
            Ok(run) => run,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(AutoStartState::Disabled);
            }
            Err(error) => return Err(error),
        };
        match run.get_value::<String, _>(VALUE_NAME) {
            Ok(value) if is_our_command(&value, executable)? => run.delete_value(VALUE_NAME)?,
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        Ok(AutoStartState::Disabled)
    }
}

#[cfg(not(windows))]
mod platform {
    use super::AutoStartState;
    use std::{io, path::Path};

    pub fn state(_executable: &Path) -> io::Result<AutoStartState> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "auto-start is currently Windows-only",
        ))
    }

    pub fn set_enabled(_executable: &Path, _enabled: bool) -> io::Result<AutoStartState> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "auto-start is currently Windows-only",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{AutoStartState, command_for, is_our_command, state_for_value};
    use std::path::Path;

    #[test]
    fn command_quotes_the_full_executable_path() {
        let executable = Path::new(r"C:\Program Files\Flash Shot\flash-shot.exe");
        assert_eq!(
            command_for(executable).unwrap(),
            r#""C:\Program Files\Flash Shot\flash-shot.exe""#
        );
        assert!(
            is_our_command(
                r#""c:\program files\flash shot\flash-shot.exe""#,
                executable
            )
            .unwrap()
        );
    }

    #[test]
    fn state_distinguishes_another_program_from_disabled() {
        assert_ne!(
            AutoStartState::ManagedByAnotherExecutable,
            AutoStartState::Disabled
        );
    }

    #[test]
    fn another_installation_is_never_treated_as_ours() {
        let executable = Path::new(r"C:\Program Files\Flash Shot\flash-shot.exe");
        assert_eq!(
            state_for_value(Some(r#""D:\Portable\flash-shot.exe""#), executable).unwrap(),
            AutoStartState::ManagedByAnotherExecutable
        );
    }
}
