//! Windows notification-area entry with capture, file, and settings commands.

use async_channel::Receiver;
use std::io;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrayEvent {
    CaptureRequested,
    FullScreenCaptureRequested,
    FullScreenCopyRequested,
    DelayedCaptureRequested(u8),
    ToggleDisplayRecordingRequested,
    ToggleRecordingPauseRequested,
    OpenHistoryDirectoryRequested,
    OpenImageRequested,
    HistoryRequested,
    SettingsRequested,
    QuitRequested,
}

/// The recording lifecycle that determines the tray menu's available command.
///
/// The UI updates this value as FFmpeg changes state; the tray thread reads it only when it opens
/// a menu, keeping the displayed action aligned with the currently safe operation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TrayRecordingState {
    #[default]
    Idle,
    Starting,
    Recording,
    Stopping,
    Paused,
    Pausing,
    Resuming,
}

impl TrayRecordingState {
    /// Converts the lifecycle state to the compact value shared with the Windows tray thread.
    const fn as_u8(self) -> u8 {
        match self {
            Self::Idle => 0,
            Self::Starting => 1,
            Self::Recording => 2,
            Self::Stopping => 3,
            Self::Paused => 4,
            Self::Pausing => 5,
            Self::Resuming => 6,
        }
    }

    /// Restores a shared tray-state value, treating unknown values as the safe idle state.
    const fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Starting,
            2 => Self::Recording,
            3 => Self::Stopping,
            4 => Self::Paused,
            5 => Self::Pausing,
            6 => Self::Resuming,
            _ => Self::Idle,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrayNotification {
    pub title: String,
    pub body: String,
}

impl TrayNotification {
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
        }
    }
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

    pub fn notify(&self, notification: TrayNotification) -> io::Result<()> {
        self.listener.notify(notification)
    }

    /// Updates the recording command shown the next time the user opens the tray menu.
    pub fn set_recording_state(&self, state: TrayRecordingState) {
        self.listener.set_recording_state(state);
    }
}

#[cfg(windows)]
mod platform {
    use super::{TrayEvent, TrayNotification, TrayRecordingState};
    use async_channel::Receiver;
    use std::{
        io,
        mem::size_of,
        ptr,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicU8, Ordering},
            mpsc::{self, SyncSender},
        },
        thread::{self, JoinHandle},
    };
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
        System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
        UI::{
            Shell::{
                NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE,
                NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW,
            },
            WindowsAndMessaging::{
                AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
                DestroyWindow, DispatchMessageW, GWLP_USERDATA, GetCursorPos, GetMessageW,
                GetWindowLongPtrW, IDI_APPLICATION, LoadIconW, MF_GRAYED, MF_SEPARATOR, MF_STRING,
                MSG, PostMessageW, PostThreadMessageW, RegisterClassW, SetForegroundWindow,
                SetWindowLongPtrW, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu,
                TranslateMessage, WM_APP, WM_CONTEXTMENU, WM_LBUTTONUP, WM_NULL, WM_QUIT,
                WM_RBUTTONUP, WNDCLASSW,
            },
        },
    };

    const ICON_ID: u32 = 1;
    const TRAY_CALLBACK: u32 = WM_APP + 1;
    const TRAY_COMMAND: u32 = WM_APP + 2;
    const MENU_CAPTURE: usize = 1;
    const MENU_FULL_SCREEN_CAPTURE: usize = 2;
    const MENU_FULL_SCREEN_COPY: usize = 3;
    const MENU_DELAYED_CAPTURE_3_SECONDS: usize = 4;
    const MENU_DELAYED_CAPTURE_5_SECONDS: usize = 5;
    const MENU_DELAYED_CAPTURE_10_SECONDS: usize = 6;
    const MENU_TOGGLE_DISPLAY_RECORDING: usize = 7;
    const MENU_TOGGLE_RECORDING_PAUSE: usize = 8;
    const MENU_OPEN_HISTORY_DIRECTORY: usize = 9;
    const MENU_OPEN_IMAGE: usize = 10;
    const MENU_HISTORY: usize = 11;
    const MENU_SETTINGS: usize = 12;
    const MENU_QUIT: usize = 13;
    const WINDOW_CLASS: &str = "FlashShot.TrayWindow";

    pub struct TrayListener {
        thread_id: u32,
        thread: Option<JoinHandle<()>>,
        commands: Arc<Mutex<Vec<TrayCommand>>>,
        recording_state: Arc<AtomicU8>,
        active: Arc<AtomicBool>,
    }

    enum TrayCommand {
        Notify(TrayNotification),
    }

    struct TrayWindowContext {
        events: async_channel::Sender<TrayEvent>,
        recording_state: Arc<AtomicU8>,
    }

    impl TrayListener {
        pub fn start() -> io::Result<(Self, Receiver<TrayEvent>)> {
            let (event_tx, event_rx) = async_channel::bounded(4);
            let (ready_tx, ready_rx) = mpsc::sync_channel(1);
            let commands = Arc::new(Mutex::new(Vec::new()));
            let thread_commands = commands.clone();
            let recording_state = Arc::new(AtomicU8::new(TrayRecordingState::Idle.as_u8()));
            let thread_recording_state = recording_state.clone();
            let active = Arc::new(AtomicBool::new(false));
            let thread_active = active.clone();
            let thread = thread::Builder::new()
                .name("flash-shot-tray".to_owned())
                .spawn(move || {
                    message_loop(
                        event_tx,
                        ready_tx,
                        thread_commands,
                        thread_recording_state,
                        thread_active,
                    )
                })?;
            match ready_rx.recv() {
                Ok(Ok(thread_id)) => Ok((
                    Self {
                        thread_id,
                        thread: Some(thread),
                        commands,
                        recording_state,
                        active,
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

        pub fn is_active(&self) -> bool {
            self.thread.is_some() && self.active.load(Ordering::Acquire)
        }

        pub fn notify(&self, notification: TrayNotification) -> io::Result<()> {
            if !self.is_active() {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "tray listener is not active",
                ));
            }
            let mut commands = self
                .commands
                .lock()
                .map_err(|_| io::Error::other("tray command queue poisoned"))?;
            commands.push(TrayCommand::Notify(notification));
            drop(commands);
            // SAFETY: thread_id belongs to the active listener and the command only reads its queue.
            if unsafe { PostThreadMessageW(self.thread_id, TRAY_COMMAND, 0, 0) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }

        /// Shares the application recording state with the tray thread without blocking UI work.
        pub fn set_recording_state(&self, state: TrayRecordingState) {
            self.recording_state.store(state.as_u8(), Ordering::Release);
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

    fn message_loop(
        events: async_channel::Sender<TrayEvent>,
        ready: SyncSender<io::Result<u32>>,
        commands: Arc<Mutex<Vec<TrayCommand>>>,
        recording_state: Arc<AtomicU8>,
        active: Arc<AtomicBool>,
    ) {
        let context = Box::new(TrayWindowContext {
            events,
            recording_state,
        });
        let result = unsafe { create_tray(&context) };
        let (window, mut icon) = match result {
            Ok(value) => value,
            Err(error) => {
                let _ = ready.send(Err(error));
                return;
            }
        };
        // SAFETY: called on the listener thread.
        let thread_id = unsafe { GetCurrentThreadId() };
        active.store(true, Ordering::Release);
        if ready.send(Ok(thread_id)).is_err() {
            active.store(false, Ordering::Release);
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
            if message.message == TRAY_COMMAND {
                process_commands(&commands, &mut icon);
            } else {
                unsafe {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        }
        active.store(false, Ordering::Release);
        unsafe { remove_tray(window, &icon) };
    }

    unsafe fn create_tray(context: &TrayWindowContext) -> io::Result<(HWND, NOTIFYICONDATAW)> {
        let class = wide(WINDOW_CLASS);
        let instance = unsafe { GetModuleHandleW(ptr::null()) };
        if instance.is_null() {
            return Err(io::Error::last_os_error());
        }
        let window_class = WNDCLASSW {
            lpfnWndProc: Some(tray_window_proc),
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
                // A message-only window cannot be made foreground. The tray popup must have a
                // regular hidden top-level owner for Windows to display it reliably.
                ptr::null_mut(),
                ptr::null_mut(),
                instance,
                ptr::null(),
            )
        };
        if window.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the boxed context lives for the lifetime of the listener thread and is
        // cleared only after the message window is destroyed.
        unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, context as *const _ as isize) };

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

    fn process_commands(commands: &Mutex<Vec<TrayCommand>>, icon: &mut NOTIFYICONDATAW) {
        let commands = match commands.lock() {
            Ok(mut commands) => std::mem::take(&mut *commands),
            Err(_) => {
                log::warn!(target: "flash_shot::tray", "tray_command_queue_poisoned");
                return;
            }
        };
        for command in commands {
            match command {
                TrayCommand::Notify(notification) => show_notification(icon, notification),
            }
        }
    }

    fn show_notification(icon: &mut NOTIFYICONDATAW, notification: TrayNotification) {
        icon.uFlags = NIF_INFO;
        icon.dwInfoFlags = NIIF_INFO;
        copy_wide(&mut icon.szInfoTitle, &notification.title);
        copy_wide(&mut icon.szInfo, &notification.body);
        // SAFETY: icon is registered by this listener thread and contains valid, NUL-terminated text.
        if unsafe { Shell_NotifyIconW(NIM_MODIFY, icon) } == 0 {
            log::warn!(target: "flash_shot::tray", "tray_notification_failed error={}", io::Error::last_os_error());
        }
        icon.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    }

    unsafe extern "system" fn tray_window_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message == TRAY_CALLBACK {
            // SAFETY: create_tray installs this pointer before registering the icon, and the
            // listener keeps the boxed context alive until after DestroyWindow returns.
            let context =
                unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *const TrayWindowContext;
            if let Some(context) = unsafe { context.as_ref() } {
                handle_tray_message(window, lparam as u32, context);
            }
            return 0;
        }
        // SAFETY: unhandled messages use the standard message-window behavior.
        unsafe { DefWindowProcW(window, message, wparam, lparam) }
    }

    /// Opens the native popup for a shell click and forwards its selected command to GPUI.
    fn handle_tray_message(window: HWND, message: u32, context: &TrayWindowContext) {
        if tray_menu_requested(message)
            && let Some(event) = show_menu(
                window,
                TrayRecordingState::from_u8(context.recording_state.load(Ordering::Acquire)),
            )
        {
            let _ = context.events.try_send(event);
        }
    }

    pub(super) fn tray_menu_requested(message: u32) -> bool {
        // Windows 11 can report a context-menu callback while earlier shell versions report
        // button-up events, so accept both forms.
        matches!(message, WM_LBUTTONUP | WM_RBUTTONUP | WM_CONTEXTMENU)
    }

    /// Builds a popup menu that labels and enables the recording command for its current state.
    fn show_menu(window: HWND, recording_state: TrayRecordingState) -> Option<TrayEvent> {
        // SAFETY: menu is owned here and destroyed before return.
        let menu = unsafe { CreatePopupMenu() };
        if menu.is_null() {
            return None;
        }
        let capture = wide("Capture");
        let full_screen_capture = wide("Capture full screen");
        let full_screen_copy = wide("Copy full screen to clipboard");
        let delayed_capture_3_seconds = wide("Capture in 3 seconds");
        let delayed_capture_5_seconds = wide("Capture in 5 seconds");
        let delayed_capture_10_seconds = wide("Capture in 10 seconds");
        let (recording_label, recording_enabled) = recording_menu_presentation(recording_state);
        let toggle_display_recording = wide(recording_label);
        let toggle_recording_pause = recording_pause_menu_presentation(recording_state).map(wide);
        let open_history_directory = wide("Open screenshot folder");
        let open_image = wide("Open image");
        let history = wide("Screenshot history");
        let settings = wide("Settings");
        let quit = wide("Quit Flash Shot");
        unsafe {
            AppendMenuW(menu, MF_STRING, MENU_CAPTURE, capture.as_ptr());
            AppendMenuW(
                menu,
                MF_STRING,
                MENU_FULL_SCREEN_CAPTURE,
                full_screen_capture.as_ptr(),
            );
            AppendMenuW(
                menu,
                MF_STRING,
                MENU_FULL_SCREEN_COPY,
                full_screen_copy.as_ptr(),
            );
            AppendMenuW(
                menu,
                MF_STRING,
                MENU_DELAYED_CAPTURE_3_SECONDS,
                delayed_capture_3_seconds.as_ptr(),
            );
            AppendMenuW(
                menu,
                MF_STRING,
                MENU_DELAYED_CAPTURE_5_SECONDS,
                delayed_capture_5_seconds.as_ptr(),
            );
            AppendMenuW(
                menu,
                MF_STRING,
                MENU_DELAYED_CAPTURE_10_SECONDS,
                delayed_capture_10_seconds.as_ptr(),
            );
            AppendMenuW(
                menu,
                if recording_enabled {
                    MF_STRING
                } else {
                    MF_STRING | MF_GRAYED
                },
                MENU_TOGGLE_DISPLAY_RECORDING,
                toggle_display_recording.as_ptr(),
            );
            if let Some(toggle_recording_pause) = toggle_recording_pause.as_ref() {
                AppendMenuW(
                    menu,
                    MF_STRING,
                    MENU_TOGGLE_RECORDING_PAUSE,
                    toggle_recording_pause.as_ptr(),
                );
            }
            AppendMenuW(
                menu,
                MF_STRING,
                MENU_OPEN_HISTORY_DIRECTORY,
                open_history_directory.as_ptr(),
            );
            AppendMenuW(menu, MF_STRING, MENU_OPEN_IMAGE, open_image.as_ptr());
            AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
            AppendMenuW(menu, MF_STRING, MENU_HISTORY, history.as_ptr());
            AppendMenuW(menu, MF_STRING, MENU_SETTINGS, settings.as_ptr());
            AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
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
        // Windows otherwise keeps the popup associated with this foreground window and can
        // suppress the next taskbar interaction after a dismissal.
        unsafe { PostMessageW(window, WM_NULL, 0, 0) };
        unsafe { DestroyMenu(menu) };
        tray_event_for_command(command as usize)
    }

    pub(super) fn tray_event_for_command(command: usize) -> Option<TrayEvent> {
        match command {
            MENU_CAPTURE => Some(TrayEvent::CaptureRequested),
            MENU_FULL_SCREEN_CAPTURE => Some(TrayEvent::FullScreenCaptureRequested),
            MENU_FULL_SCREEN_COPY => Some(TrayEvent::FullScreenCopyRequested),
            MENU_DELAYED_CAPTURE_3_SECONDS => Some(TrayEvent::DelayedCaptureRequested(3)),
            MENU_DELAYED_CAPTURE_5_SECONDS => Some(TrayEvent::DelayedCaptureRequested(5)),
            MENU_DELAYED_CAPTURE_10_SECONDS => Some(TrayEvent::DelayedCaptureRequested(10)),
            MENU_TOGGLE_DISPLAY_RECORDING => Some(TrayEvent::ToggleDisplayRecordingRequested),
            MENU_TOGGLE_RECORDING_PAUSE => Some(TrayEvent::ToggleRecordingPauseRequested),
            MENU_OPEN_HISTORY_DIRECTORY => Some(TrayEvent::OpenHistoryDirectoryRequested),
            MENU_OPEN_IMAGE => Some(TrayEvent::OpenImageRequested),
            MENU_HISTORY => Some(TrayEvent::HistoryRequested),
            MENU_SETTINGS => Some(TrayEvent::SettingsRequested),
            MENU_QUIT => Some(TrayEvent::QuitRequested),
            _ => None,
        }
    }

    /// Returns the recording menu text and whether it can be invoked for each lifecycle state.
    pub(super) fn recording_menu_presentation(state: TrayRecordingState) -> (&'static str, bool) {
        match state {
            TrayRecordingState::Idle => ("Start display recording", true),
            TrayRecordingState::Starting => ("Starting display recording...", false),
            TrayRecordingState::Recording => ("Stop display recording", true),
            TrayRecordingState::Stopping => ("Stopping display recording...", false),
            TrayRecordingState::Paused => ("Stop display recording", true),
            TrayRecordingState::Pausing => ("Pausing display recording...", false),
            TrayRecordingState::Resuming => ("Resuming display recording...", false),
        }
    }

    /// Exposes pause controls only while FFmpeg has an active recording process to control.
    pub(super) fn recording_pause_menu_presentation(
        state: TrayRecordingState,
    ) -> Option<&'static str> {
        match state {
            TrayRecordingState::Recording => Some("Pause display recording"),
            TrayRecordingState::Paused => Some("Resume display recording"),
            TrayRecordingState::Idle
            | TrayRecordingState::Starting
            | TrayRecordingState::Stopping
            | TrayRecordingState::Pausing
            | TrayRecordingState::Resuming => None,
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
        target.fill(0);
        let encoded: Vec<u16> = value.encode_utf16().collect();
        let length = encoded.len().min(target.len().saturating_sub(1));
        target[..length].copy_from_slice(&encoded[..length]);
        target[length] = 0;
    }
}

#[cfg(not(windows))]
mod platform {
    use super::{TrayEvent, TrayNotification, TrayRecordingState};
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

        pub fn is_active(&self) -> bool {
            false
        }

        pub fn notify(&self, _notification: TrayNotification) -> io::Result<()> {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "tray notifications are currently Windows-only",
            ))
        }

        pub fn set_recording_state(&self, _state: TrayRecordingState) {}
    }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::{TrayNotification, TrayService};

    #[cfg(windows)]
    #[test]
    fn tray_starts_and_stops_cleanly() {
        let (tray, _events) = TrayService::start().unwrap();
        assert!(tray.is_active());
        tray.notify(TrayNotification::new("Flash Shot", "Notification test"))
            .unwrap();
        drop(tray);
    }

    #[cfg(windows)]
    #[test]
    fn tray_clicks_request_the_popup_menu() {
        use super::platform::tray_menu_requested;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            WM_CONTEXTMENU, WM_LBUTTONUP, WM_RBUTTONUP,
        };

        assert!(tray_menu_requested(WM_LBUTTONUP));
        assert!(tray_menu_requested(WM_RBUTTONUP));
        assert!(tray_menu_requested(WM_CONTEXTMENU));
        assert!(!tray_menu_requested(0));
    }

    #[cfg(windows)]
    #[test]
    fn delayed_capture_menu_items_dispatch_their_configured_delays() {
        use super::{TrayEvent, platform::tray_event_for_command};

        assert_eq!(
            tray_event_for_command(4),
            Some(TrayEvent::DelayedCaptureRequested(3))
        );
        assert_eq!(
            tray_event_for_command(5),
            Some(TrayEvent::DelayedCaptureRequested(5))
        );
        assert_eq!(
            tray_event_for_command(6),
            Some(TrayEvent::DelayedCaptureRequested(10))
        );
    }

    #[cfg(windows)]
    #[test]
    fn full_screen_menu_item_dispatches_the_full_screen_event() {
        use super::{TrayEvent, platform::tray_event_for_command};

        assert_eq!(
            tray_event_for_command(2),
            Some(TrayEvent::FullScreenCaptureRequested)
        );
    }

    #[cfg(windows)]
    #[test]
    fn full_screen_copy_menu_item_dispatches_the_clipboard_event() {
        use super::{TrayEvent, platform::tray_event_for_command};

        assert_eq!(
            tray_event_for_command(3),
            Some(TrayEvent::FullScreenCopyRequested)
        );
    }

    #[cfg(windows)]
    #[test]
    fn screenshot_folder_menu_item_dispatches_the_directory_event() {
        use super::{TrayEvent, platform::tray_event_for_command};

        assert_eq!(
            tray_event_for_command(9),
            Some(TrayEvent::OpenHistoryDirectoryRequested)
        );
    }

    #[cfg(windows)]
    #[test]
    fn recording_menu_item_dispatches_the_recording_toggle_event() {
        use super::{TrayEvent, platform::tray_event_for_command};

        assert_eq!(
            tray_event_for_command(7),
            Some(TrayEvent::ToggleDisplayRecordingRequested)
        );
    }

    #[cfg(windows)]
    #[test]
    fn pause_menu_item_dispatches_the_pause_toggle_event() {
        use super::{TrayEvent, platform::tray_event_for_command};

        assert_eq!(
            tray_event_for_command(8),
            Some(TrayEvent::ToggleRecordingPauseRequested)
        );
    }

    #[cfg(windows)]
    #[test]
    fn recording_menu_labels_prevent_conflicting_lifecycle_operations() {
        use super::{
            TrayRecordingState,
            platform::{recording_menu_presentation, recording_pause_menu_presentation},
        };

        assert_eq!(
            recording_menu_presentation(TrayRecordingState::Idle),
            ("Start display recording", true)
        );
        assert_eq!(
            recording_menu_presentation(TrayRecordingState::Starting),
            ("Starting display recording...", false)
        );
        assert_eq!(
            recording_menu_presentation(TrayRecordingState::Recording),
            ("Stop display recording", true)
        );
        assert_eq!(
            recording_menu_presentation(TrayRecordingState::Stopping),
            ("Stopping display recording...", false)
        );
        assert_eq!(
            recording_menu_presentation(TrayRecordingState::Paused),
            ("Stop display recording", true)
        );
        assert_eq!(
            recording_menu_presentation(TrayRecordingState::Pausing),
            ("Pausing display recording...", false)
        );
        assert_eq!(
            recording_menu_presentation(TrayRecordingState::Resuming),
            ("Resuming display recording...", false)
        );
        assert_eq!(
            recording_pause_menu_presentation(TrayRecordingState::Recording),
            Some("Pause display recording")
        );
        assert_eq!(
            recording_pause_menu_presentation(TrayRecordingState::Paused),
            Some("Resume display recording")
        );
        assert_eq!(
            recording_pause_menu_presentation(TrayRecordingState::Pausing),
            None
        );
    }
}
