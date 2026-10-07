//! Win32 window discovery, focus ownership, safe input, and screenshot primitives.

use std::{
    io,
    path::Path,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use flash_shot::{SettingsInteractionAcceptanceCommand, SettingsInteractionState, i18n::Locale};

#[cfg(windows)]
/// Holds one borrowed top-level HWND plus its measured screen bounds and monitor DPI.
/// The window handle stays valid only while the isolated runner process is alive.
#[derive(Clone, Copy)]
pub(super) struct NativeWindow {
    pub(super) handle: *mut std::ffi::c_void,
    pub(super) left: i32,
    pub(super) top: i32,
    pub(super) right: i32,
    pub(super) bottom: i32,
    pub(super) dpi: u32,
}

#[cfg(windows)]
/// Raises the settings window so injected input and screenshots target the intended HWND.
pub(super) fn focus_window(window: NativeWindow) -> io::Result<()> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
        VK_MENU,
    };
    let topmost = unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SetWindowPos(
            window.handle,
            windows_sys::Win32::UI::WindowsAndMessaging::HWND_TOPMOST,
            window.left,
            window.top,
            window.right - window.left,
            window.bottom - window.top,
            windows_sys::Win32::UI::WindowsAndMessaging::SWP_SHOWWINDOW,
        ) != 0
    };
    let started = Instant::now();
    let mut assist_attempted = false;
    loop {
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::BringWindowToTop(window.handle);
            windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow(window.handle);
        }
        if unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() == window.handle
        } {
            return Ok(());
        }
        if !assist_attempted && started.elapsed() >= Duration::from_millis(250) {
            assist_attempted = true;
            if unsafe { GetAsyncKeyState(VK_MENU as i32) < 0 } {
                return Err(io::Error::other(
                    "Alt is held; foreground activation was aborted to avoid changing user input",
                ));
            }
            let alt = [
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VK_MENU,
                            ..Default::default()
                        },
                    },
                },
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VK_MENU,
                            dwFlags: KEYEVENTF_KEYUP,
                            ..Default::default()
                        },
                    },
                },
            ];
            let sent = unsafe {
                SendInput(
                    alt.len() as u32,
                    alt.as_ptr(),
                    std::mem::size_of::<INPUT>() as i32,
                )
            };
            if sent != alt.len() as u32 {
                if sent > 0 {
                    let release = alt[1];
                    unsafe {
                        SendInput(1, &release, std::mem::size_of::<INPUT>() as i32);
                    }
                }
                return Err(io::Error::other(format!(
                    "foreground activation assist accepted {sent}/2 Alt events"
                )));
            }
        }
        if started.elapsed() >= Duration::from_secs(2) {
            let foreground =
                unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
            return Err(io::Error::other(format!(
                "settings window could not become foreground (expected {:?}, observed {:?}, topmost_set={topmost})",
                window.handle, foreground
            )));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(windows)]
/// Keeps temporary z-order changes scoped to the acceptance worker, including error paths.
pub(super) struct WindowZOrderRestore {
    pub(super) window: NativeWindow,
    pub(super) restored: bool,
}

#[cfg(windows)]
impl WindowZOrderRestore {
    /// Removes temporary topmost state and records whether Windows accepted the request.
    pub(super) fn restore(&mut self) -> bool {
        if self.restored {
            return true;
        }
        let restored = set_window_not_topmost(self.window);
        self.restored = restored;
        restored
    }
}

#[cfg(windows)]
impl Drop for WindowZOrderRestore {
    fn drop(&mut self) {
        if !self.restored {
            self.restored = set_window_not_topmost(self.window);
        }
    }
}

#[cfg(windows)]
/// Removes the temporary always-on-top flag without moving or resizing the settings window.
fn set_window_not_topmost(window: NativeWindow) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        HWND_NOTOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos,
    };
    unsafe {
        SetWindowPos(
            window.handle,
            HWND_NOTOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        ) != 0
    }
}

#[cfg(windows)]
/// Requests the current settings section from the GPUI thread with a bounded reply deadline.
pub(super) fn snapshot(
    commands: &async_channel::Sender<SettingsInteractionAcceptanceCommand>,
) -> Result<SettingsInteractionState, Box<dyn std::error::Error>> {
    let (reply_tx, reply_rx) = mpsc::sync_channel(1);
    commands.send_blocking(SettingsInteractionAcceptanceCommand::Snapshot(reply_tx))?;
    Ok(reply_rx.recv_timeout(Duration::from_secs(2))?)
}

#[cfg(windows)]
/// Computes a stable click point in the horizontal navigation row or vertical rail.
pub(super) fn click_navigation_item(
    window: NativeWindow,
    compact: bool,
    index: usize,
) -> io::Result<()> {
    guard_foreground(window)?;
    let x = if compact {
        window.left + ((index as i32 * window_width(window)) / 4) + window_width(window) / 8
    } else {
        window.left + scale_logical_extent(window, 82)
    };
    let y = if compact {
        window.top + scale_logical_extent(window, 135)
    } else {
        window.top + scale_logical_extent(window, 165 + index as i32 * 52)
    };
    move_and_click(window, x, y)
}

#[cfg(windows)]
/// Clicks the App page's update action using the layout coordinates from the production page.
///
/// The acceptance window intentionally uses the documented 520px compact and 980px wide
/// layouts. Keeping the two targets explicit makes the probe fail loudly if the page geometry
/// moves instead of silently clicking another preference.
pub(super) fn click_update_action(window: NativeWindow, compact: bool) -> io::Result<()> {
    let (logical_x, logical_y) = if compact { (300, 486) } else { (453, 432) };
    guard_foreground(window)?;
    move_and_click(
        window,
        window.left + scale_logical_extent(window, logical_x),
        window.top + scale_logical_extent(window, logical_y),
    )
}

#[cfg(windows)]
/// Clicks the App page's appearance button before the live Pin language/theme assertion.
pub(super) fn click_system_theme(window: NativeWindow, compact: bool) -> io::Result<()> {
    let (logical_x, logical_y) = if compact { (300, 322) } else { (453, 269) };
    guard_foreground(window)?;
    move_and_click(
        window,
        window.left + scale_logical_extent(window, logical_x),
        window.top + scale_logical_extent(window, logical_y),
    )
}

#[cfg(windows)]
/// Clicks the App page's language button after the theme update has reached every Pin.
pub(super) fn click_system_language(window: NativeWindow, compact: bool) -> io::Result<()> {
    let (logical_x, logical_y) = if compact { (300, 378) } else { (453, 325) };
    guard_foreground(window)?;
    move_and_click(
        window,
        window.left + scale_logical_extent(window, logical_x),
        window.top + scale_logical_extent(window, logical_y),
    )
}

#[cfg(windows)]
/// Clicks the Record page's support-check action in the compact or wide settings layout.
pub(super) fn click_record_support(window: NativeWindow, compact: bool) -> io::Result<()> {
    let (logical_x, logical_y) = if compact { (82, 531) } else { (229, 441) };
    guard_foreground(window)?;
    move_and_click(
        window,
        window.left + scale_logical_extent(window, logical_x),
        window.top + scale_logical_extent(window, logical_y),
    )
}

#[cfg(windows)]
/// Clicks the Record primary toggle in the deterministic short-directory acceptance layout.
pub(super) fn click_record_toggle(window: NativeWindow, compact: bool) -> io::Result<()> {
    let (logical_x, logical_y) = if compact { (225, 530) } else { (326, 441) };
    guard_foreground(window)?;
    move_and_click(
        window,
        window.left + scale_logical_extent(window, logical_x),
        window.top + scale_logical_extent(window, logical_y),
    )
}

#[cfg(windows)]
/// Clicks the idle Record primary action, whose row moves down before a status row exists.
pub(super) fn click_record_idle_toggle(window: NativeWindow, compact: bool) -> io::Result<()> {
    let (logical_x, logical_y) = if compact { (225, 550) } else { (326, 498) };
    guard_foreground(window)?;
    move_and_click(
        window,
        window.left + scale_logical_extent(window, logical_x),
        window.top + scale_logical_extent(window, logical_y),
    )
}

#[cfg(windows)]
/// Clicks the Library export-format action after the acceptance runner fixes the history root to
/// a short path, keeping the content rows at stable coordinates in both layout modes.
pub(super) fn click_library_format(window: NativeWindow, compact: bool) -> io::Result<()> {
    let (logical_x, logical_y) = if compact { (435, 525) } else { (520, 472) };
    guard_foreground(window)?;
    move_and_click(
        window,
        window.left + scale_logical_extent(window, logical_x),
        window.top + scale_logical_extent(window, logical_y),
    )
}

#[cfg(windows)]
/// Selects source-only captures in the stable wide Library acceptance layout.
pub(super) fn click_library_filter_selection(window: NativeWindow) -> io::Result<()> {
    click_library_control(window, 290, 687)
}

#[cfg(windows)]
/// Restores all source types after the search/selection acceptance flow.
pub(super) fn click_library_filter_all(window: NativeWindow) -> io::Result<()> {
    click_library_control(window, 215, 687)
}

#[cfg(windows)]
/// Focuses the Library search field before native Unicode input is sent.
pub(super) fn click_library_search(window: NativeWindow) -> io::Result<()> {
    click_library_control(window, 500, 640)
}

#[cfg(windows)]
/// Clears the query through the visible production search-clear button.
pub(super) fn click_library_clear_search(window: NativeWindow) -> io::Result<()> {
    click_library_control(window, 956, 640)
}

#[cfg(windows)]
/// Selects every row matched by the current Library query and source filter.
pub(super) fn click_library_select_all_filtered(window: NativeWindow) -> io::Result<()> {
    click_library_control(window, 350, 766)
}

#[cfg(windows)]
/// Clears the in-memory history selection without deleting any saved screenshot.
pub(super) fn click_library_clear_selection(window: NativeWindow) -> io::Result<()> {
    click_library_control(window, 475, 766)
}

#[cfg(windows)]
/// Clicks Remove on the first visible Library row in the tall, wide single-delete acceptance view.
pub(super) fn click_library_remove(window: NativeWindow, _compact: bool) -> io::Result<()> {
    click_library_row_action(window, 490, 880)
}

#[cfg(windows)]
/// Copies the first visible retained image using its production Library row action.
pub(super) fn click_library_copy(window: NativeWindow) -> io::Result<()> {
    click_library_row_action(window, 375, 880)
}

#[cfg(windows)]
/// Opens the first visible retained image using its production Library row action.
pub(super) fn click_library_open(window: NativeWindow) -> io::Result<()> {
    click_library_row_action(window, 300, 880)
}

#[cfg(windows)]
/// Advances the Library's persistent capture-retention setting in the wide acceptance layout.
pub(super) fn click_library_retention(window: NativeWindow, locale: Locale) -> io::Result<()> {
    let logical_x = match locale {
        Locale::English => 740,
        Locale::SimplifiedChinese => 640,
    };
    click_library_control(window, logical_x, 510)
}

#[cfg(windows)]
/// Confirms the row-level removal after its inline prompt has replaced the row actions.
pub(super) fn click_library_confirm_single_remove(
    window: NativeWindow,
    _compact: bool,
) -> io::Result<()> {
    click_library_row_action(window, 850, 880)
}

#[cfg(windows)]
/// Cancels the row-level removal while leaving both managed screenshots intact.
pub(super) fn click_library_cancel_single_remove(
    window: NativeWindow,
    _compact: bool,
) -> io::Result<()> {
    click_library_row_action(window, 940, 880)
}

#[cfg(windows)]
/// Converts a documented logical point in the row action strip to physical desktop coordinates.
fn click_library_row_action(
    window: NativeWindow,
    logical_x: i32,
    logical_y: i32,
) -> io::Result<()> {
    click_library_control(window, logical_x, logical_y)
}

#[cfg(windows)]
/// Converts a stable wide-layout control point to physical desktop coordinates.
fn click_library_control(window: NativeWindow, logical_x: i32, logical_y: i32) -> io::Result<()> {
    move_and_click(
        window,
        window.left + scale_logical_extent(window, logical_x),
        window.top + scale_logical_extent(window, logical_y),
    )
}

#[cfg(windows)]
/// Sends one key press/release pair and repairs a partially accepted SendInput batch.
pub(super) fn send_key(window: NativeWindow, virtual_key: u16) -> io::Result<()> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
    };
    guard_foreground(window)?;
    let inputs = [
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: virtual_key,
                    ..Default::default()
                },
            },
        },
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: virtual_key,
                    dwFlags: KEYEVENTF_KEYUP,
                    ..Default::default()
                },
            },
        },
    ];
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    if sent == inputs.len() as u32 {
        Ok(())
    } else if sent == 0 {
        Err(io::Error::last_os_error())
    } else {
        let error = io::Error::last_os_error();
        let release = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: virtual_key,
                    dwFlags: KEYEVENTF_KEYUP,
                    ..Default::default()
                },
            },
        };
        let cleanup = unsafe { SendInput(1, &release, std::mem::size_of::<INPUT>() as i32) };
        if cleanup == 1 {
            Err(error)
        } else {
            Err(io::Error::new(
                error.kind(),
                format!(
                    "{error}; key-up cleanup failed: {}",
                    io::Error::last_os_error()
                ),
            ))
        }
    }
}

#[cfg(windows)]
/// Sends bounded Unicode text with paired key-down/up events and releases a partial key-down.
pub(super) fn send_unicode_text(window: NativeWindow, text: &str) -> io::Result<()> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, SendInput,
    };
    let units = text.encode_utf16().collect::<Vec<_>>();
    if units.is_empty() || units.len() > 64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "acceptance text must contain between 1 and 64 UTF-16 code units",
        ));
    }
    for unit in units {
        guard_foreground(window)?;
        let inputs = [
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wScan: unit,
                        dwFlags: KEYEVENTF_UNICODE,
                        ..Default::default()
                    },
                },
            },
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wScan: unit,
                        dwFlags: KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
                        ..Default::default()
                    },
                },
            },
        ];
        let sent = unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            )
        };
        if sent == inputs.len() as u32 {
            continue;
        }
        let error = io::Error::last_os_error();
        if sent == 1 {
            let release = INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wScan: unit,
                        dwFlags: KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
                        ..Default::default()
                    },
                },
            };
            if unsafe { SendInput(1, &release, std::mem::size_of::<INPUT>() as i32) } != 1 {
                return Err(io::Error::new(
                    error.kind(),
                    format!(
                        "{error}; Unicode key-up cleanup failed: {}",
                        io::Error::last_os_error()
                    ),
                ));
            }
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(windows)]
/// Moves and clicks one navigation target, releasing the button after partial input.
fn move_and_click(window: NativeWindow, x: i32, y: i32) -> io::Result<()> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEINPUT,
        SendInput,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::SetCursorPos;
    guard_foreground(window)?;
    if unsafe { SetCursorPos(x, y) } == 0 {
        return Err(io::Error::last_os_error());
    }
    guard_foreground(window)?;
    let inputs = [
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dwFlags: MOUSEEVENTF_LEFTDOWN,
                    ..Default::default()
                },
            },
        },
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dwFlags: MOUSEEVENTF_LEFTUP,
                    ..Default::default()
                },
            },
        },
    ];
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    if sent == inputs.len() as u32 {
        Ok(())
    } else if sent == 0 {
        Err(io::Error::last_os_error())
    } else {
        let error = io::Error::last_os_error();
        let release = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dwFlags: MOUSEEVENTF_LEFTUP,
                    ..Default::default()
                },
            },
        };
        let cleanup = unsafe { SendInput(1, &release, std::mem::size_of::<INPUT>() as i32) };
        if cleanup == 1 {
            Err(error)
        } else {
            Err(io::Error::new(
                error.kind(),
                format!(
                    "{error}; left-button cleanup failed: {}",
                    io::Error::last_os_error()
                ),
            ))
        }
    }
}

#[cfg(windows)]
/// Aborts global input if another desktop window took focus after the acceptance window was raised.
fn guard_foreground(window: NativeWindow) -> io::Result<()> {
    let foreground = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
    if foreground == window.handle {
        Ok(())
    } else {
        Err(io::Error::other(
            "foreground window changed; settings input was aborted",
        ))
    }
}

#[cfg(windows)]
/// Saves the visible native window pixels after an action for visual review.
pub(super) fn capture_step(
    window: &NativeWindow,
    output_dir: &Path,
    name: &str,
) -> io::Result<String> {
    use flash_shot::domain::geometry::PhysicalRect;
    use flash_shot::platform::capture::{CaptureBackend, SystemCaptureBackend};
    let path = output_dir.join(format!("{name}.png"));
    let bounds = PhysicalRect {
        left: window.left,
        top: window.top,
        right: window.right,
        bottom: window.bottom,
    };
    SystemCaptureBackend.capture(bounds)?.save_png(&path)?;
    Ok(path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned())
}

#[cfg(windows)]
fn window_width(window: NativeWindow) -> i32 {
    (window.right - window.left).max(1)
}

#[cfg(windows)]
/// Converts a logical-pixel offset to physical pixels using the window's measured monitor DPI.
fn scale_logical_extent(window: NativeWindow, logical_pixels: i32) -> i32 {
    (logical_pixels as f32 * window.dpi as f32 / 96.0).round() as i32
}

#[cfg(windows)]
/// Reads the pointer position before the probe moves this global desktop input device.
fn cursor_position() -> io::Result<(i32, i32)> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut point = POINT::default();
    if unsafe { GetCursorPos(&mut point) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok((point.x, point.y))
    }
}

#[cfg(windows)]
/// Refuses to merge injected navigation with a user-held mouse button or modifier key.
pub(super) fn ensure_input_idle() -> io::Result<()> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_CONTROL, VK_DOWN, VK_LBUTTON, VK_MENU, VK_RETURN, VK_RIGHT, VK_SHIFT,
        VK_SPACE,
    };
    let held = [
        VK_LBUTTON, VK_CONTROL, VK_DOWN, VK_MENU, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE,
    ]
    .into_iter()
    .any(|key| unsafe { GetAsyncKeyState(key as i32) < 0 });
    if held {
        Err(io::Error::other(
            "release the mouse button and modifier keys before starting input acceptance",
        ))
    } else {
        Ok(())
    }
}

#[cfg(windows)]
pub(super) struct CursorRestore {
    position: (i32, i32),
    restored: bool,
}

#[cfg(windows)]
impl CursorRestore {
    /// Saves the pointer position before the input runner moves it.
    pub(super) fn capture() -> io::Result<Self> {
        Ok(Self {
            position: cursor_position()?,
            restored: false,
        })
    }

    /// Restores the saved position only after injected mouse buttons are released.
    pub(super) fn restore(mut self) -> io::Result<bool> {
        if unsafe {
            windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(
                windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_LBUTTON as i32,
            ) < 0
        } {
            self.restored = true;
            return Err(io::Error::other(
                "cursor was not moved because the left mouse button remains held",
            ));
        }
        let restored = unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::SetCursorPos(
                self.position.0,
                self.position.1,
            ) != 0
        };
        self.restored = restored;
        if restored {
            Ok(true)
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

#[cfg(windows)]
impl Drop for CursorRestore {
    fn drop(&mut self) {
        if !self.restored
            && unsafe {
                windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(
                    windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_LBUTTON as i32,
                ) < 0
            }
        {
            return;
        }
        if !self.restored {
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::SetCursorPos(
                    self.position.0,
                    self.position.1,
                )
            };
        }
    }
}

#[cfg(windows)]
/// Finds this process's visible settings window and its physical screen bounds.
pub(super) fn visible_window() -> io::Result<Option<NativeWindow>> {
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::{LPARAM, RECT};
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible,
    };
    use windows_sys::core::BOOL;
    struct Search {
        process_id: u32,
        window: Option<NativeWindow>,
    }
    unsafe extern "system" fn callback(handle: *mut c_void, parameter: LPARAM) -> BOOL {
        let search = unsafe { &mut *(parameter as *mut Search) };
        let mut process_id = 0;
        unsafe { GetWindowThreadProcessId(handle, &mut process_id) };
        if process_id != search.process_id || unsafe { IsWindowVisible(handle) } == 0 {
            return 1;
        }
        let mut rect = RECT::default();
        if unsafe { GetWindowRect(handle, &mut rect) } != 0 {
            // Live appearance acceptance keeps three 360x240 Pins beside this settings window;
            // reject those child surfaces so input always targets the configured settings bounds.
            if rect.right - rect.left < 420 || rect.bottom - rect.top < 420 {
                return 1;
            }
            search.window = Some(NativeWindow {
                handle,
                left: rect.left,
                top: rect.top,
                right: rect.right,
                bottom: rect.bottom,
                dpi: unsafe { GetDpiForWindow(handle) }.max(96),
            });
            return 0;
        }
        1
    }
    let mut search = Search {
        process_id: unsafe { GetCurrentProcessId() },
        window: None,
    };
    unsafe { EnumWindows(Some(callback), &mut search as *mut Search as LPARAM) };
    Ok(search.window)
}

#[cfg(windows)]
/// Finds one visible window from this process while excluding a known settings HWND.
pub(super) fn visible_window_except(
    excluded: NativeWindow,
    minimum_width: i32,
    minimum_height: i32,
) -> io::Result<Option<NativeWindow>> {
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::{LPARAM, RECT};
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible,
    };
    use windows_sys::core::BOOL;

    struct Search {
        process_id: u32,
        excluded_handle: *mut c_void,
        minimum_width: i32,
        minimum_height: i32,
        windows: Vec<NativeWindow>,
    }

    unsafe extern "system" fn callback(handle: *mut c_void, parameter: LPARAM) -> BOOL {
        let search = unsafe { &mut *(parameter as *mut Search) };
        if handle == search.excluded_handle || unsafe { IsWindowVisible(handle) } == 0 {
            return 1;
        }
        let mut process_id = 0;
        unsafe { GetWindowThreadProcessId(handle, &mut process_id) };
        if process_id != search.process_id {
            return 1;
        }
        let mut rect = RECT::default();
        if unsafe { GetWindowRect(handle, &mut rect) } == 0 {
            return 1;
        }
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        if width >= search.minimum_width && height >= search.minimum_height {
            search.windows.push(NativeWindow {
                handle,
                left: rect.left,
                top: rect.top,
                right: rect.right,
                bottom: rect.bottom,
                dpi: unsafe { GetDpiForWindow(handle) }.max(96),
            });
        }
        1
    }

    let mut search = Search {
        process_id: unsafe { GetCurrentProcessId() },
        excluded_handle: excluded.handle,
        minimum_width,
        minimum_height,
        windows: Vec::new(),
    };
    if unsafe { EnumWindows(Some(callback), &mut search as *mut Search as LPARAM) } == 0 {
        return Err(io::Error::last_os_error());
    }
    match search.windows.len() {
        0 => Ok(None),
        1 => Ok(search.windows.pop()),
        count => Err(io::Error::other(format!(
            "expected at most one visible editor window, found {count}"
        ))),
    }
}
