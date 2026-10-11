//! Native acceptance for the Snow-style window/control smart-target toggle.

use super::*;

#[derive(serde::Serialize)]
pub(super) struct SmartTargetReport {
    pub fixture_process_id: u32,
    pub fixture_window: WindowReport,
    pub control_point: PhysicalPoint,
    pub control_target_kind: Option<String>,
    pub control_target_bounds: Option<PhysicalRect>,
    pub window_target_kind: Option<String>,
    pub window_target_bounds: Option<PhysicalRect>,
    pub committed_selection: Option<PhysicalRect>,
    pub preference_reset_after_recapture: bool,
    pub cleanup: CleanupReport,
}

/// Exercises default control targeting, Alt+W window targeting, click-to-window selection, and
/// the preference reset performed by a fresh Capture lifecycle.
pub(super) fn execute(context: &WorkerContext, report: &mut AcceptanceReport) -> io::Result<()> {
    let fixture_bounds = smart_fixture_bounds(context.display.physical_bounds)?;
    let mut fixture = SmartTargetFixture::launch(fixture_bounds, context.timeout)?;
    let fixture_window = fixture.report()?;
    let control_point = fixture.control_point();
    let controller = wait_for_controller(context.timeout)?;
    report.controller_window = Some(controller.report());

    context
        .interaction_commands
        .send_blocking(OverlayInteractionAcceptanceCommand::ShowCaptureSettings)
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "capture command channel closed"))?;
    let controller = wait_for_owned_window_visible(controller.handle, context.timeout)?;
    focus_and_inject_capture_shortcut(controller, context.timeout)?;
    let overlay = wait_for_overlay(
        controller.handle,
        context.display.physical_bounds,
        context.timeout,
    )?;
    wait_for_window_gone(
        controller.handle,
        context.timeout,
        "smart-target capture hides Settings",
    )?;
    focus_owned_window(overlay, context.timeout)?;
    fixture.raise_topmost()?;
    raise_owned_window_above_topmost(overlay)?;
    thread::sleep(context.settle_delay);

    inject_mouse_move(overlay.handle, control_point)?;
    let control_state = wait_for_capture_state(context, "control smart target", |state| {
        state.session_state == "selecting"
            && state.overlay_count == 1
            && state.smart_target_preference == "control"
            && state.inspection_target_kind.as_deref() == Some("control")
            && state.inspection_target_bounds.is_some_and(|bounds| {
                bounds.contains(control_point) && rect_contains_rect(fixture_bounds, bounds)
            })
    })?;
    let control_evidence = capture_evidence(context, "smart-target-control.png", overlay)?;
    record_step(
        report,
        &context.report_path,
        "smart_target_control_ready",
        guard_foreground(overlay.handle)?,
        Some(&control_evidence),
    )?;

    inject_alt_w(overlay.handle)?;
    let window_state = wait_for_capture_state(context, "window smart target", |state| {
        state.session_state == "selecting"
            && state.overlay_count == 1
            && state.smart_target_preference == "window"
            && state.inspection_target_kind.as_deref() == Some("window")
            && state.inspection_target_bounds == Some(fixture_bounds)
    })?;
    let window_evidence = capture_evidence(context, "smart-target-window.png", overlay)?;
    record_step(
        report,
        &context.report_path,
        "smart_target_window_ready",
        guard_foreground(overlay.handle)?,
        Some(&window_evidence),
    )?;

    inject_mouse_click(overlay.handle, control_point)?;
    let selected = wait_for_capture_state(context, "window target selection", |state| {
        state.session_state == "selecting"
            && state.overlay_count == 1
            && state.selection == Some(fixture_bounds)
    })?;
    record_step(
        report,
        &context.report_path,
        "smart_target_window_committed",
        guard_foreground(overlay.handle)?,
        None,
    )?;

    inject_key(overlay.handle, VK_ESCAPE)?;
    wait_for_capture_state(context, "smart-target first cleanup", |state| {
        state.session_state == "idle" && state.overlay_count == 0 && state.capture_preflight_ready
    })?;

    context
        .interaction_commands
        .send_blocking(OverlayInteractionAcceptanceCommand::ShowCaptureSettings)
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "capture command channel closed"))?;
    let controller = wait_for_owned_window_visible(controller.handle, context.timeout)?;
    focus_and_inject_capture_shortcut(controller, context.timeout)?;
    let recapture_overlay = wait_for_overlay(
        controller.handle,
        context.display.physical_bounds,
        context.timeout,
    )?;
    wait_for_window_gone(
        controller.handle,
        context.timeout,
        "smart-target recapture hides Settings",
    )?;
    focus_owned_window(recapture_overlay, context.timeout)?;
    fixture.raise_topmost()?;
    raise_owned_window_above_topmost(recapture_overlay)?;
    inject_mouse_move(recapture_overlay.handle, control_point)?;
    let reset_state = wait_for_capture_state(context, "smart-target preference reset", |state| {
        state.session_state == "selecting"
            && state.overlay_count == 1
            && state.smart_target_preference == "control"
            && state.inspection_target_kind.as_deref() == Some("control")
    })?;
    let preference_reset_after_recapture = reset_state.smart_target_preference == "control";
    record_step(
        report,
        &context.report_path,
        "smart_target_preference_reset",
        guard_foreground(recapture_overlay.handle)?,
        None,
    )?;
    inject_key(recapture_overlay.handle, VK_ESCAPE)?;
    wait_for_capture_state(context, "smart-target recapture cleanup", |state| {
        state.session_state == "idle" && state.overlay_count == 0 && state.capture_preflight_ready
    })?;

    fixture.shutdown(context.timeout.min(Duration::from_secs(3)))?;
    let final_state = query_capture_state(context, context.timeout)?;
    let visible_process_windows = process_windows()?.len();
    if final_state.session_state != "idle"
        || final_state.overlay_count != 0
        || final_state.pinned_count != 0
        || final_state.capture_teardown_pending
        || !final_state.background_tasks_idle
        || !final_state.capture_preflight_ready
        || visible_process_windows != 0
        || !preference_reset_after_recapture
    {
        return Err(io::Error::other(format!(
            "smart-target cleanup failed: state={final_state:?}, visible_process_windows={visible_process_windows}, preference_reset_after_recapture={preference_reset_after_recapture}"
        )));
    }

    report.smart_target = Some(SmartTargetReport {
        fixture_process_id: fixture.process_id,
        fixture_window,
        control_point,
        control_target_kind: control_state.inspection_target_kind,
        control_target_bounds: control_state.inspection_target_bounds,
        window_target_kind: window_state.inspection_target_kind,
        window_target_bounds: window_state.inspection_target_bounds,
        committed_selection: selected.selection,
        preference_reset_after_recapture,
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

fn smart_fixture_bounds(display: PhysicalRect) -> io::Result<PhysicalRect> {
    let width = 640_i32;
    let height = 360_i32;
    // Keep the fixture away from the pinned video window that may be present in the shared
    // desktop. The pointer must resolve to this fixture in both the control and window passes.
    const EDGE_MARGIN: i32 = 64;
    let left = display.left + EDGE_MARGIN;
    let top = display.bottom - height - EDGE_MARGIN;
    let bounds = PhysicalRect {
        left,
        top,
        right: left + width,
        bottom: top + height,
    };
    rect_inside_display(bounds, display)
        .then_some(bounds)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "display is too small for smart-target fixture",
            )
        })
}
