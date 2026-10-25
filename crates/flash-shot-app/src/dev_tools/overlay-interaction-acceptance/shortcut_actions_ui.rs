//! Native acceptance for the supported global Capture action matrix.

use super::*;

#[derive(serde::Serialize)]
pub(super) struct ShortcutActionsReport {
    pub capture_shortcut: &'static str,
    pub full_screen_shortcut: &'static str,
    pub focused_window_shortcut: &'static str,
    pub capture_started_without_selection: bool,
    pub full_screen_selection: Option<PhysicalRect>,
    pub focused_window_fixture: WindowReport,
    pub focused_window_selection: Option<PhysicalRect>,
    pub input_released: bool,
    pub fixture_stopped: bool,
    pub cleanup: CleanupReport,
}

/// Exercises Capture, full-screen, and focused-window hotkeys with real Win32 input.
pub(super) fn execute(context: &WorkerContext, report: &mut AcceptanceReport) -> io::Result<()> {
    let fixture_bounds = shortcut_fixture_bounds(context.display.physical_bounds)?;
    let mut fixture = SmartTargetFixture::launch(fixture_bounds, context.timeout)?;
    let fixture_window = fixture.report()?;
    let controller = wait_for_controller(context.timeout)?;
    report.controller_window = Some(controller.report());

    let capture_overlay = open_settings_and_capture(context, controller, VK_F24, "F24")?;
    let capture_state = wait_for_capture_state(context, "global Capture action", |state| {
        state.session_state == "selecting" && state.overlay_count == 1 && state.selection.is_none()
    })?;
    let capture_step_evidence = capture_evidence(context, "shortcut-capture.png", capture_overlay)?;
    record_step(
        report,
        &context.report_path,
        "shortcut_capture_action",
        guard_foreground(capture_overlay.handle)?,
        Some(&capture_step_evidence),
    )?;
    close_overlay(context, capture_overlay, "global Capture cleanup")?;

    let full_screen_overlay = open_settings_and_capture(context, controller, VK_F23, "F23")?;
    let full_screen_state =
        wait_for_capture_state(context, "global full-screen action", |state| {
            state.session_state == "selecting"
                && state.overlay_count == 1
                && state.selection == Some(context.display.physical_bounds)
        })?;
    let full_screen_evidence =
        capture_evidence(context, "shortcut-full-screen.png", full_screen_overlay)?;
    record_step(
        report,
        &context.report_path,
        "shortcut_full_screen_action",
        guard_foreground(full_screen_overlay.handle)?,
        Some(&full_screen_evidence),
    )?;
    close_overlay(context, full_screen_overlay, "global full-screen cleanup")?;

    let controller = show_capture_settings(context, controller)?;
    fixture.focus(context.timeout)?;
    inject_global_shortcut_from_external_window(fixture.target, fixture.process_id, VK_F22, "F22")?;
    let focused_overlay = wait_for_overlay(
        controller.handle,
        context.display.physical_bounds,
        context.timeout,
    )?;
    wait_for_window_gone(
        controller.handle,
        context.timeout,
        "focused-window shortcut hides Settings",
    )?;
    focus_owned_window(focused_overlay, context.timeout)?;
    let focused_state = wait_for_capture_state(context, "global focused-window action", |state| {
        state.session_state == "selecting"
            && state.overlay_count == 1
            && state.selection == Some(fixture_bounds)
    })?;
    let focused_evidence =
        capture_evidence(context, "shortcut-focused-window.png", focused_overlay)?;
    record_step(
        report,
        &context.report_path,
        "shortcut_focused_window_action",
        guard_foreground(focused_overlay.handle)?,
        Some(&focused_evidence),
    )?;
    close_overlay(context, focused_overlay, "global focused-window cleanup")?;

    let input_released = ensure_input_keys_released(&[
        (VK_CONTROL, "Control"),
        (VK_MENU, "Alt"),
        (VK_F22, "F22"),
        (VK_F23, "F23"),
        (VK_F24, "F24"),
    ])
    .is_ok();
    fixture.shutdown(context.timeout.min(Duration::from_secs(3)))?;
    let fixture_stopped = fixture.stopped;
    let final_state = query_capture_state(context, context.timeout)?;
    let visible_process_windows = process_windows()?.len();
    if final_state.session_state != "idle"
        || final_state.overlay_count != 0
        || final_state.pinned_count != 0
        || final_state.capture_teardown_pending
        || !final_state.background_tasks_idle
        || !final_state.capture_preflight_ready
        || visible_process_windows != 0
        || !input_released
        || !fixture_stopped
    {
        return Err(io::Error::other(format!(
            "global shortcut action cleanup failed: state={final_state:?}, visible_process_windows={visible_process_windows}, input_released={input_released}, fixture_stopped={fixture_stopped}"
        )));
    }

    report.shortcut_actions = Some(ShortcutActionsReport {
        capture_shortcut: CAPTURE_SHORTCUT,
        full_screen_shortcut: FULL_SCREEN_ACCEPTANCE_SHORTCUT,
        focused_window_shortcut: FOCUSED_WINDOW_ACCEPTANCE_SHORTCUT,
        capture_started_without_selection: capture_state.selection.is_none(),
        full_screen_selection: full_screen_state.selection,
        focused_window_fixture: fixture_window,
        focused_window_selection: focused_state.selection,
        input_released,
        fixture_stopped,
        cleanup: CleanupReport {
            session_state: final_state.session_state,
            overlay_count: final_state.overlay_count,
            pinned_count: final_state.pinned_count,
            capture_teardown_pending: final_state.capture_teardown_pending,
            visible_process_windows,
            capture_preflight_ready: final_state.capture_preflight_ready,
        },
    });
    Ok(())
}

fn shortcut_fixture_bounds(display: PhysicalRect) -> io::Result<PhysicalRect> {
    let width = 640_i32;
    let height = 360_i32;
    const EDGE_MARGIN: i32 = 64;
    let bounds = PhysicalRect {
        left: display.left + EDGE_MARGIN,
        top: display.bottom - height - EDGE_MARGIN,
        right: display.left + EDGE_MARGIN + width,
        bottom: display.bottom - EDGE_MARGIN,
    };
    rect_inside_display(bounds, display)
        .then_some(bounds)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "display is too small for global-shortcut fixture",
            )
        })
}

fn show_capture_settings(
    context: &WorkerContext,
    controller: NativeWindow,
) -> io::Result<NativeWindow> {
    context
        .interaction_commands
        .send_blocking(OverlayInteractionAcceptanceCommand::ShowCaptureSettings)
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "capture command channel closed"))?;
    wait_for_owned_window_visible(controller.handle, context.timeout)
}

fn open_settings_and_capture(
    context: &WorkerContext,
    controller: NativeWindow,
    virtual_key: u16,
    key_label: &'static str,
) -> io::Result<NativeWindow> {
    let controller = show_capture_settings(context, controller)?;
    if virtual_key == VK_F24 {
        focus_and_inject_capture_shortcut(controller, context.timeout)?;
    } else {
        focus_owned_window(controller, context.timeout)?;
        inject_global_shortcut(controller.handle, virtual_key, key_label)?;
    }
    let overlay = wait_for_overlay(
        controller.handle,
        context.display.physical_bounds,
        context.timeout,
    )?;
    wait_for_window_gone(
        controller.handle,
        context.timeout,
        "global shortcut hides Settings",
    )?;
    focus_owned_window(overlay, context.timeout)?;
    Ok(overlay)
}

fn close_overlay(context: &WorkerContext, overlay: NativeWindow, stage: &str) -> io::Result<()> {
    thread::sleep(context.settle_delay);
    if let Err(error) = focus_and_inject_key(overlay, VK_ESCAPE, context.timeout) {
        let foreground = unsafe { GetForegroundWindow() };
        let mut process_id = 0;
        if !foreground.is_null() {
            unsafe { GetWindowThreadProcessId(foreground, &mut process_id) };
        }
        return Err(io::Error::new(
            error.kind(),
            format!(
                "{stage} could not inject Escape: {error}; expected HWND={:?}, foreground HWND={foreground:?}, foreground PID={process_id}",
                overlay.handle
            ),
        ));
    }
    wait_for_capture_state(context, stage, |state| {
        state.session_state == "idle" && state.overlay_count == 0 && state.capture_preflight_ready
    })?;
    Ok(())
}
