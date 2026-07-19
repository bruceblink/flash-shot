//! Process-wide screenshot hotkey registration and lifecycle.

use async_channel::Receiver;
use std::{fmt, io};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureShortcut {
    control: bool,
    alt: bool,
    shift: bool,
    key: ShortcutKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShortcutKey {
    PrintScreen,
    Letter(char),
    Function(u8),
}

impl Default for CaptureShortcut {
    fn default() -> Self {
        Self {
            control: true,
            alt: false,
            shift: true,
            key: ShortcutKey::PrintScreen,
        }
    }
}

impl CaptureShortcut {
    pub const PRESETS: [&'static str; 4] = [
        "Ctrl+Shift+Print Screen",
        "Ctrl+Alt+S",
        "Ctrl+Shift+S",
        "Shift+F12",
    ];

    pub fn from_environment() -> Result<Self, ShortcutParseError> {
        match std::env::var("FLASH_SHOT_CAPTURE_HOTKEY") {
            Ok(value) if !value.trim().is_empty() => value.parse(),
            Ok(_) | Err(std::env::VarError::NotPresent) => Ok(Self::default()),
            Err(std::env::VarError::NotUnicode(_)) => Err(ShortcutParseError),
        }
    }

    pub fn parse_preset(value: &str) -> Result<Self, ShortcutParseError> {
        if !Self::PRESETS.contains(&value) {
            return Err(ShortcutParseError);
        }
        value.parse()
    }
}

impl fmt::Display for CaptureShortcut {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if self.control {
            parts.push("Ctrl".to_owned());
        }
        if self.alt {
            parts.push("Alt".to_owned());
        }
        if self.shift {
            parts.push("Shift".to_owned());
        }
        parts.push(match self.key {
            ShortcutKey::PrintScreen => "Print Screen".to_owned(),
            ShortcutKey::Letter(letter) => letter.to_string(),
            ShortcutKey::Function(number) => format!("F{number}"),
        });
        formatter.write_str(&parts.join("+"))
    }
}

impl std::str::FromStr for CaptureShortcut {
    type Err = ShortcutParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut control = false;
        let mut alt = false;
        let mut shift = false;
        let mut key = None;
        for part in value
            .split('+')
            .map(str::trim)
            .filter(|part| !part.is_empty())
        {
            let normalized = part.to_ascii_lowercase().replace([' ', '-'], "");
            match normalized.as_str() {
                "ctrl" | "control" if !control => control = true,
                "alt" if !alt => alt = true,
                "shift" if !shift => shift = true,
                "printscreen" | "prtsc" if key.is_none() => key = Some(ShortcutKey::PrintScreen),
                _ if key.is_none() && normalized.len() == 1 => {
                    let letter = normalized.chars().next().unwrap();
                    if letter.is_ascii_alphabetic() {
                        key = Some(ShortcutKey::Letter(letter.to_ascii_uppercase()));
                    } else {
                        return Err(ShortcutParseError);
                    }
                }
                _ if key.is_none() => {
                    let number = normalized
                        .strip_prefix('f')
                        .and_then(|number| number.parse::<u8>().ok())
                        .filter(|number| (1..=24).contains(number));
                    key = Some(
                        number
                            .map(ShortcutKey::Function)
                            .ok_or(ShortcutParseError)?,
                    );
                }
                _ => return Err(ShortcutParseError),
            }
        }
        if !(control || alt || shift) || key.is_none() {
            return Err(ShortcutParseError);
        }
        Ok(Self {
            control,
            alt,
            shift,
            key: key.unwrap(),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShortcutParseError;

impl fmt::Display for ShortcutParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("expected Ctrl/Alt/Shift plus A-Z, F1-F24, or PrintScreen")
    }
}

impl std::error::Error for ShortcutParseError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShortcutEvent {
    CaptureRequested,
}

pub struct GlobalShortcutService {
    listener: platform::ShortcutListener,
}

impl GlobalShortcutService {
    pub fn register_capture(
        shortcut: CaptureShortcut,
    ) -> io::Result<(Self, Receiver<ShortcutEvent>)> {
        let (listener, events) = platform::ShortcutListener::register(shortcut)?;
        Ok((Self { listener }, events))
    }

    pub fn is_active(&self) -> bool {
        self.listener.is_active()
    }
}

#[cfg(windows)]
mod platform {
    use super::{CaptureShortcut, ShortcutEvent, ShortcutKey};
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
        pub fn register(shortcut: CaptureShortcut) -> io::Result<(Self, Receiver<ShortcutEvent>)> {
            Self::register_key(modifiers(shortcut), virtual_key(shortcut))
        }

        pub(super) fn register_key(
            modifiers: u32,
            virtual_key: u32,
        ) -> io::Result<(Self, Receiver<ShortcutEvent>)> {
            let (event_tx, event_rx) = async_channel::bounded(1);
            let (ready_tx, ready_rx) = mpsc::sync_channel(1);
            let thread = thread::Builder::new()
                .name("flash-shot-hotkey".to_owned())
                .spawn(move || message_loop(event_tx, ready_tx, modifiers, virtual_key))?;

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

    fn modifiers(shortcut: CaptureShortcut) -> u32 {
        let mut modifiers = MOD_NOREPEAT;
        if shortcut.control {
            modifiers |= MOD_CONTROL;
        }
        if shortcut.alt {
            modifiers |= windows_sys::Win32::UI::Input::KeyboardAndMouse::MOD_ALT;
        }
        if shortcut.shift {
            modifiers |= MOD_SHIFT;
        }
        modifiers
    }

    fn virtual_key(shortcut: CaptureShortcut) -> u32 {
        match shortcut.key {
            ShortcutKey::PrintScreen => VK_SNAPSHOT as u32,
            ShortcutKey::Letter(letter) => letter as u32,
            ShortcutKey::Function(number) => 0x70 + u32::from(number - 1),
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
        modifiers: u32,
        virtual_key: u32,
    ) {
        // SAFETY: called from the listener thread itself.
        let thread_id = unsafe { GetCurrentThreadId() };
        // SAFETY: a null HWND registers a thread-level hotkey.
        if unsafe { RegisterHotKey(ptr::null_mut(), HOTKEY_ID, modifiers, virtual_key) } == 0 {
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
    use super::CaptureShortcut;
    #[cfg(windows)]
    use super::platform::ShortcutListener;
    #[cfg(windows)]
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{MOD_NOREPEAT, VK_F24};

    #[cfg(windows)]
    #[test]
    fn capture_hotkey_registers_and_stops_cleanly() {
        let (listener, _events) =
            ShortcutListener::register_key(MOD_NOREPEAT, VK_F24 as u32).unwrap();
        assert!(listener.is_active());
        drop(listener);
    }

    #[test]
    fn parses_supported_shortcut_forms_and_normalizes_the_display() {
        assert_eq!(
            "ctrl + alt + s"
                .parse::<CaptureShortcut>()
                .unwrap()
                .to_string(),
            "Ctrl+Alt+S"
        );
        assert_eq!(
            "Shift+F12".parse::<CaptureShortcut>().unwrap().to_string(),
            "Shift+F12"
        );
        assert_eq!(
            "Ctrl+PrtSc".parse::<CaptureShortcut>().unwrap().to_string(),
            "Ctrl+Print Screen"
        );
    }

    #[test]
    fn preset_shortcuts_are_safe_and_displayable() {
        for preset in CaptureShortcut::PRESETS {
            let shortcut = CaptureShortcut::parse_preset(preset).unwrap();
            assert!(!shortcut.to_string().is_empty());
        }
        assert!(CaptureShortcut::parse_preset("Ctrl+Alt+F12").is_err());
    }

    #[test]
    fn rejects_unsafe_or_ambiguous_shortcut_forms() {
        for shortcut in [
            "S",
            "Ctrl",
            "Ctrl+Shift",
            "Ctrl+1",
            "Ctrl+F25",
            "Ctrl+Ctrl+S",
        ] {
            assert!(shortcut.parse::<CaptureShortcut>().is_err(), "{shortcut}");
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::{CaptureShortcut, ShortcutEvent};
    use async_channel::Receiver;
    use std::io;

    pub struct ShortcutListener;

    impl ShortcutListener {
        pub fn register(_shortcut: CaptureShortcut) -> io::Result<(Self, Receiver<ShortcutEvent>)> {
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
