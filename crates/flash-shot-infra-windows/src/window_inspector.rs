//! Window and UI-control hit testing behind a platform-neutral boundary.

use std::io;

use flash_shot_domain::domain::geometry::{PhysicalPoint, PhysicalRect};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InspectionKind {
    Control,
    Window,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InspectionTarget {
    pub bounds: PhysicalRect,
    pub kind: InspectionKind,
}

/// Identifies a visible top-level window and the exact desktop pixels used for recording it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowCaptureTarget {
    pub title: String,
    pub bounds: PhysicalRect,
}

pub trait WindowInspector {
    fn target_at(&self, point: PhysicalPoint) -> io::Result<Option<InspectionTarget>>;
    fn window_capture_target_at(
        &self,
        point: PhysicalPoint,
    ) -> io::Result<Option<WindowCaptureTarget>>;
    fn focused_window_target(&self) -> io::Result<Option<InspectionTarget>>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemWindowInspector;

impl WindowInspector for SystemWindowInspector {
    fn target_at(&self, point: PhysicalPoint) -> io::Result<Option<InspectionTarget>> {
        platform::target_at(point)
    }

    fn window_capture_target_at(
        &self,
        point: PhysicalPoint,
    ) -> io::Result<Option<WindowCaptureTarget>> {
        platform::window_capture_target_at(point)
    }

    fn focused_window_target(&self) -> io::Result<Option<InspectionTarget>> {
        platform::focused_window_target()
    }
}

impl SystemWindowInspector {
    /// Finds one visible window by title for the recording acceptance probe.
    ///
    /// The production overlay resolves from a user-selected point instead; this title lookup keeps
    /// the command-line probe on the same physical-bounds recording path without guessing among
    /// duplicate visible windows.
    pub fn visible_window_with_title(title: &str) -> io::Result<Option<WindowCaptureTarget>> {
        platform::visible_window_with_title(title)
    }
}

#[cfg(windows)]
mod platform {
    use super::{InspectionKind, InspectionTarget, WindowCaptureTarget};
    use flash_shot_domain::domain::geometry::{PhysicalPoint, PhysicalRect};
    use std::io;
    use windows::{
        Win32::{
            Foundation::{HWND, LPARAM, RECT},
            System::{
                Com::{
                    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
                    CoUninitialize,
                },
                Threading::GetCurrentProcessId,
            },
            UI::{
                Accessibility::{CUIAutomation8, IUIAutomation, IUIAutomationElement},
                WindowsAndMessaging::{
                    EnumWindows, GetForegroundWindow, GetSystemMetrics, GetWindowRect,
                    GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
                    SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
                },
            },
        },
        core::Error,
    };

    pub fn target_at(point: PhysicalPoint) -> io::Result<Option<InspectionTarget>> {
        let Some((window, window_bounds)) = window_at(point)? else {
            return Ok(None);
        };
        if let Ok(Some(control_bounds)) = uia_control_at(window, point)
            && control_bounds != window_bounds
        {
            return Ok(Some(InspectionTarget {
                bounds: control_bounds,
                kind: InspectionKind::Control,
            }));
        }
        Ok(Some(InspectionTarget {
            bounds: window_bounds,
            kind: InspectionKind::Window,
        }))
    }

    /// Resolves both a top-level window title and its physical bounds from one selected point.
    pub fn window_capture_target_at(
        point: PhysicalPoint,
    ) -> io::Result<Option<WindowCaptureTarget>> {
        let Some((window, bounds)) = window_at(point)? else {
            return Ok(None);
        };
        Ok(window_capture_target(window, bounds))
    }

    /// Finds one visible exact-title match, rejecting ambiguity for repeatable CLI acceptance.
    pub fn visible_window_with_title(title: &str) -> io::Result<Option<WindowCaptureTarget>> {
        struct Search<'a> {
            title: &'a str,
            found: Option<WindowCaptureTarget>,
            ambiguous: bool,
        }

        unsafe extern "system" fn callback(window: HWND, parameter: LPARAM) -> windows::core::BOOL {
            // SAFETY: EnumWindows passes the Search pointer supplied by visible_window_with_title.
            let search = unsafe { &mut *(parameter.0 as *mut Search<'_>) };
            // A minimized window has no stable visible desktop content to record.
            if !unsafe { IsWindowVisible(window) }.as_bool()
                || unsafe { IsIconic(window) }.as_bool()
            {
                return true.into();
            }
            let mut rect = RECT::default();
            if unsafe { GetWindowRect(window, &mut rect) }.is_err() {
                return true.into();
            }
            let Some(target) = window_capture_target(window, physical_rect(rect)) else {
                return true.into();
            };
            if target.title != search.title {
                return true.into();
            }
            if search.found.is_some() {
                search.ambiguous = true;
                return false.into();
            }
            search.found = Some(target);
            true.into()
        }

        let title = title.trim();
        if title.is_empty() {
            return Ok(None);
        }
        let mut search = Search {
            title,
            found: None,
            ambiguous: false,
        };
        let result = unsafe {
            EnumWindows(
                Some(callback),
                LPARAM((&mut search as *mut Search<'_>) as isize),
            )
        };
        if let Err(error) = result
            && search.found.is_none()
        {
            return Err(windows_error(error));
        }
        if search.ambiguous {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("more than one visible window matches title '{title}'"),
            ));
        }
        Ok(search.found)
    }

    fn window_capture_target(window: HWND, bounds: PhysicalRect) -> Option<WindowCaptureTarget> {
        let bounds = clip_to_virtual_desktop(bounds)?;
        let mut buffer = vec![0_u16; 512];
        // SAFETY: buffer is writable and its declared capacity matches the supplied length.
        let length = unsafe { GetWindowTextW(window, &mut buffer) };
        if length == 0 {
            return None;
        }
        let title = String::from_utf16_lossy(&buffer[..length as usize]);
        (!title.trim().is_empty()).then_some(WindowCaptureTarget { title, bounds })
    }

    /// Clips a Win32 outer-window rectangle to the pixels that can actually be sampled.
    fn clip_to_virtual_desktop(bounds: PhysicalRect) -> Option<PhysicalRect> {
        let left = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
        let top = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
        let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
        let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
        let right = left.checked_add(width)?;
        let bottom = top.checked_add(height)?;
        let clipped = PhysicalRect {
            left: bounds.left.max(left),
            top: bounds.top.max(top),
            right: bounds.right.min(right),
            bottom: bounds.bottom.min(bottom),
        };
        (clipped.left < clipped.right && clipped.top < clipped.bottom).then_some(clipped)
    }

    /// Resolves the visible foreground window without using pointer position or UI Automation.
    pub fn focused_window_target() -> io::Result<Option<InspectionTarget>> {
        // SAFETY: GetForegroundWindow returns either a borrowed HWND or a null handle.
        let window = unsafe { GetForegroundWindow() };
        if window.0.is_null() || !unsafe { IsWindowVisible(window) }.as_bool() {
            return Ok(None);
        }
        let mut process_id = 0;
        // SAFETY: process_id points to writable storage for the duration of the call.
        unsafe { GetWindowThreadProcessId(window, Some(&mut process_id)) };
        // SAFETY: returns the identifier of the current process.
        if process_id == unsafe { GetCurrentProcessId() } {
            return Ok(None);
        }
        let mut rect = RECT::default();
        // SAFETY: rect is valid writable storage for GetWindowRect.
        unsafe { GetWindowRect(window, &mut rect) }.map_err(windows_error)?;
        let bounds = physical_rect(rect);
        Ok(
            (bounds.width() > 0 && bounds.height() > 0).then_some(InspectionTarget {
                bounds,
                kind: InspectionKind::Window,
            }),
        )
    }

    fn window_at(point: PhysicalPoint) -> io::Result<Option<(HWND, PhysicalRect)>> {
        struct Search {
            point: PhysicalPoint,
            process_id: u32,
            found: Option<(HWND, PhysicalRect)>,
        }

        unsafe extern "system" fn callback(window: HWND, parameter: LPARAM) -> windows::core::BOOL {
            // SAFETY: EnumWindows passes the Search pointer supplied by window_at.
            let search = unsafe { &mut *(parameter.0 as *mut Search) };
            if !unsafe { IsWindowVisible(window) }.as_bool() {
                return true.into();
            }
            let mut process_id = 0;
            unsafe { GetWindowThreadProcessId(window, Some(&mut process_id)) };
            if process_id == search.process_id {
                return true.into();
            }
            let mut rect = RECT::default();
            if unsafe { GetWindowRect(window, &mut rect) }.is_err() {
                return true.into();
            }
            let bounds = physical_rect(rect);
            if bounds.width() == 0 || bounds.height() == 0 || !bounds.contains(search.point) {
                return true.into();
            }
            search.found = Some((window, bounds));
            false.into()
        }

        let mut search = Search {
            point,
            // SAFETY: returns the identifier of the current process.
            process_id: unsafe { GetCurrentProcessId() },
            found: None,
        };
        // SAFETY: callback only accesses search during this synchronous enumeration.
        let result = unsafe {
            EnumWindows(
                Some(callback),
                LPARAM((&mut search as *mut Search) as isize),
            )
        };
        if let Err(error) = result
            && search.found.is_none()
        {
            return Err(windows_error(error));
        }
        Ok(search.found)
    }

    fn uia_control_at(
        window: HWND,
        point: PhysicalPoint,
    ) -> windows::core::Result<Option<PhysicalRect>> {
        let _com = ComGuard::initialize()?;
        let bounds = {
            let automation: IUIAutomation =
                unsafe { CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER) }?;
            let root = unsafe { automation.ElementFromHandle(window) }?;
            let walker = unsafe { automation.ControlViewWalker() }?;
            let control = deepest_control_at(&walker, root, point)?;
            unsafe { control.CurrentBoundingRectangle() }?
        };
        let bounds = physical_rect(bounds);
        // UI Automation can report a descendant bounding rectangle that no longer matches the
        // queried point after a live window changes. Returning it would make callers highlight a
        // different control, so fall back to the verified top-level window in that case.
        Ok((bounds.width() > 0 && bounds.height() > 0 && bounds.contains(point)).then_some(bounds))
    }

    fn deepest_control_at(
        walker: &windows::Win32::UI::Accessibility::IUIAutomationTreeWalker,
        root: IUIAutomationElement,
        point: PhysicalPoint,
    ) -> windows::core::Result<IUIAutomationElement> {
        let mut current = root;
        loop {
            let mut child = unsafe { walker.GetFirstChildElement(&current) }.ok();
            let mut match_child = None;
            while let Some(candidate) = child {
                let bounds = unsafe { candidate.CurrentBoundingRectangle() }?;
                if physical_rect(bounds).contains(point) {
                    match_child = Some(candidate);
                    break;
                }
                child = unsafe { walker.GetNextSiblingElement(&candidate) }.ok();
            }
            let Some(child) = match_child else {
                return Ok(current);
            };
            current = child;
        }
    }

    struct ComGuard;

    impl ComGuard {
        fn initialize() -> windows::core::Result<Self> {
            let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            result.ok()?;
            Ok(Self)
        }
    }

    impl Drop for ComGuard {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }

    const fn physical_rect(rect: RECT) -> PhysicalRect {
        PhysicalRect {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        }
    }

    fn windows_error(error: Error) -> io::Error {
        io::Error::other(error)
    }
}

#[cfg(not(windows))]
mod platform {
    use super::{InspectionTarget, WindowCaptureTarget};
    use flash_shot_domain::domain::geometry::PhysicalPoint;
    use std::io;

    pub fn target_at(_point: PhysicalPoint) -> io::Result<Option<InspectionTarget>> {
        Ok(None)
    }

    pub fn window_capture_target_at(
        _point: PhysicalPoint,
    ) -> io::Result<Option<WindowCaptureTarget>> {
        Ok(None)
    }

    pub fn visible_window_with_title(_title: &str) -> io::Result<Option<WindowCaptureTarget>> {
        Ok(None)
    }

    pub fn focused_window_target() -> io::Result<Option<InspectionTarget>> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::{InspectionKind, InspectionTarget};
    #[cfg(windows)]
    use super::{SystemWindowInspector, WindowInspector};
    use flash_shot_domain::domain::geometry::{PhysicalPoint, PhysicalRect};

    #[test]
    fn inspection_target_preserves_virtual_desktop_coordinates() {
        let target = InspectionTarget {
            bounds: PhysicalRect {
                left: -1200,
                top: 100,
                right: -200,
                bottom: 900,
            },
            kind: InspectionKind::Window,
        };

        assert!(target.bounds.contains(PhysicalPoint { x: -500, y: 400 }));
    }

    #[cfg(windows)]
    #[test]
    fn system_inspector_finds_a_real_desktop_target() {
        use crate::display::{DisplayProvider, SystemDisplayProvider, virtual_desktop_bounds};

        let displays = SystemDisplayProvider.displays().unwrap();
        let bounds = virtual_desktop_bounds(&displays).unwrap();
        let point = PhysicalPoint {
            x: bounds.left + bounds.width() as i32 / 2,
            y: bounds.top + bounds.height() as i32 / 2,
        };

        let target = SystemWindowInspector.target_at(point).unwrap().unwrap();

        assert!(target.bounds.contains(point));
        assert!(target.bounds.width() > 0);
        assert!(target.bounds.height() > 0);
    }
}
