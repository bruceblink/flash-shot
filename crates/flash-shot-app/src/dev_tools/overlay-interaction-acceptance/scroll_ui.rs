//! Scroll screenshot orchestration for the Windows Release acceptance runner.

use super::*;

#[cfg(windows)]
/// Drives More -> Scroll shot -> real auto-scroll capture -> Finish, then proves teardown.
pub(super) fn execute(context: &WorkerContext, report: &mut AcceptanceReport) -> io::Result<()> {
    let display = context.display.physical_bounds;
    let fixture_bounds = PhysicalRect {
        left: display.left.saturating_add(120),
        top: display.top.saturating_add(100),
        right: display.right.saturating_sub(120),
        bottom: display.bottom.saturating_sub(100),
    };
    let mut controller = wait_for_controller(context.timeout)?;
    let controller_right = display.right.saturating_sub(20);
    let controller_bottom = display.bottom.saturating_sub(20);
    move_owned_window(
        controller.handle,
        PhysicalRect {
            left: controller_right.saturating_sub(controller.bounds.width() as i32),
            top: controller_bottom.saturating_sub(controller.bounds.height() as i32),
            right: controller_right,
            bottom: controller_bottom,
        },
    )?;
    controller = owned_window(controller.handle)?;
    focus_owned_window(controller, context.timeout)?;
    report.controller_window = Some(controller.report());
    record_step(
        report,
        &context.report_path,
        "scroll_roundtrip_controller_ready",
        controller,
        None,
    )?;
    // Launch the external target only after the acceptance controller owns foreground input; the
    // no-activate fixture style keeps that ownership while still receiving wheel hit testing.
    let mut fixture = ScrollWindowFixture::launch(fixture_bounds, context.timeout)?;
    let fixture_window = fixture.report()?;
    focus_owned_window(controller, context.timeout)?;

    let foreground = inject_capture_shortcut(controller.handle)?;
    record_step(
        report,
        &context.report_path,
        "scroll_roundtrip_capture_shortcut",
        foreground,
        None,
    )?;
    let overlay = wait_for_overlay(controller.handle, display, context.timeout)?;
    focus_owned_window(overlay, context.timeout)?;
    thread::sleep(context.settle_delay);
    let plan = scroll_roundtrip_interaction_plan_for_window(overlay.handle)?;
    // The production capture overlay consumes global physical points while it spans the full
    // display. Keep the acceptance request in that same coordinate space; remapping through the
    // borderless client rectangle would apply the native inset a second time.
    let drag = inject_mouse_drag_in_display_pixels(
        overlay.handle,
        plan.drag_start,
        plan.drag_end,
        display,
    )?;
    let selected = capture_evidence(context, "00-scroll-selected.png", drag.foreground)?;
    record_step(
        report,
        &context.report_path,
        "scroll_roundtrip_selection",
        drag.foreground,
        Some(&selected),
    )?;
    let initial_state = wait_for_capture_state(context, "scroll roundtrip selection", |state| {
        state.session_state == "selecting" && state.selection.is_some() && state.overlay_count == 1
    })?;
    let initial_selection = initial_state
        .selection
        .ok_or_else(|| io::Error::other("scroll roundtrip selection disappeared"))?;
    validate_selection_geometry(
        drag.selection,
        initial_selection,
        "scroll roundtrip selection",
    )?;
    if !rect_contains_rect(fixture.target_bounds, initial_selection) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "scroll fixture {:?} does not contain the full selection {initial_selection:?}",
                fixture.target_bounds
            ),
        ));
    }
    // Recompute the action row from the committed selection. The initial drag plan is only a
    // safe viewport seed; the production toolbar may clamp above/below or against the display
    // edge after GPUI normalizes the final selection geometry.
    let mut plan =
        interaction_plan_for_capture_selection(overlay.handle, display, initial_selection)?;
    plan.more = scroll_toolbar_more_point_for_capture_selection(
        overlay.handle,
        display,
        initial_selection,
        initial_state.annotation_controls_visible,
    )?;

    let initial_frame = query_capture_content(context, context.timeout)?
        .selection
        .ok_or_else(|| {
            io::Error::other("scroll roundtrip did not expose the initial selection frame")
        })?;
    validate_scroll_fixture_frame(&initial_frame, fixture.target_bounds, 0)?;
    let initial_frame_report =
        save_scroll_frame_report(context, "01-scroll-initial-frame.png", initial_frame)?;
    lower_scroll_fixture(fixture.target)?;
    focus_owned_window(overlay, context.timeout)?;
    raise_owned_window_above_topmost(overlay)?;
    let more_hit = unsafe {
        WindowFromPoint(POINT {
            x: plan.more.x,
            y: plan.more.y,
        })
    };
    if more_hit != overlay.handle {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "scroll roundtrip More point {:?} resolves to {:?}, expected overlay {:?}",
                plan.more, more_hit, overlay.handle
            ),
        ));
    }

    let foreground = inject_mouse_click(overlay.handle, plan.more)?;
    let _more_state = wait_for_capture_state(context, "scroll roundtrip More", |state| {
        state.session_state == "selecting" && state.more_actions_visible && state.overlay_count == 1
    })?;
    thread::sleep(context.settle_delay);
    let more = capture_evidence(context, "02-scroll-more.png", overlay)?;
    ensure_evidence_changed(
        &selected,
        &more,
        "Scroll roundtrip More did not change the overlay",
    )?;
    record_step(
        report,
        &context.report_path,
        "scroll_roundtrip_more",
        foreground,
        Some(&more),
    )?;
    let menu_client_bounds = _more_state.secondary_menu_bounds.ok_or_else(|| {
        io::Error::other("scroll More state did not expose the rendered menu bounds")
    })?;
    let client_bounds = client_bounds_for_window(overlay.handle)?;
    let menu_bounds = translated_rect(menu_client_bounds, client_bounds.left, client_bounds.top)?;
    let scroll_point = scroll_menu_item_screen_point(menu_bounds, overlay.dpi, 4)?;
    if !overlay.bounds.contains(scroll_point) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("computed Scroll shot point {scroll_point:?} escaped overlay"),
        ));
    }
    let foreground = inject_mouse_click(overlay.handle, scroll_point)?;
    record_step(
        report,
        &context.report_path,
        "scroll_roundtrip_scroll_shot",
        foreground,
        None,
    )?;
    wait_for_window_gone(overlay.handle, context.timeout, "Scroll shot")?;
    let ready = wait_for_capture_state(context, "scroll roundtrip ready", |state| {
        state.overlay_count == 0
            && !state.more_actions_visible
            && !state.annotation_controls_visible
            && state.status == "Scrolling screenshot ready. One frame captured."
    })?;
    let mut scroll_control = wait_for_scroll_control(controller.handle, context.timeout)?;
    let preferred_top = initial_selection.bottom.saturating_add(12);
    let preferred_bottom = preferred_top.saturating_add(scroll_control.bounds.height() as i32);
    if preferred_bottom <= display.bottom.saturating_sub(12) {
        move_owned_window(
            scroll_control.handle,
            PhysicalRect {
                left: scroll_control.bounds.left,
                top: preferred_top,
                right: scroll_control.bounds.right,
                bottom: preferred_bottom,
            },
        )?;
        scroll_control = owned_window(scroll_control.handle)?;
    }
    if rects_overlap(initial_selection, scroll_control.bounds) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "scroll control {:?} overlaps capture selection {initial_selection:?}",
                scroll_control.bounds
            ),
        ));
    }
    focus_owned_window(scroll_control, context.timeout)?;
    let scroll_control_report = scroll_control.report();
    set_fixture_window_bounds(fixture.target, fixture.target_bounds, true)?;
    raise_scroll_fixture_for_capture(fixture.target, fixture.target_bounds)?;
    // Keep the foreground control above the fixture while it remains visible. When auto capture
    // hides the control, the fixture stays topmost so the desktop app behind it cannot replace
    // the pixels that the production capture backend samples.
    raise_owned_window_above_topmost(scroll_control)?;
    let wheel_routing = preflight_scroll_input(initial_selection, fixture.target)?;

    let foreground = inject_scroll_auto_capture(scroll_control.handle)?;
    record_step(
        report,
        &context.report_path,
        "scroll_roundtrip_auto_capture",
        foreground,
        None,
    )?;
    let auto = wait_for_capture_state(context, "scroll roundtrip auto capture", |state| {
        state.overlay_count == 0
            && (state.status.starts_with("Captured scroll frame 2 (")
                || state
                    .status
                    .starts_with("That frame did not overlap the previous one:"))
    })?;
    if !auto.status.starts_with("Captured scroll frame 2 (") {
        let fixture_offset = fixture.offset()?;
        let failed_frame = SystemCaptureBackend.capture(initial_selection)?;
        let diagnostic_path = context
            .session_root
            .join("screenshots")
            .join("03-scroll-failed-frame.png");
        failed_frame.save_png(&diagnostic_path)?;
        let fixture_validation =
            validate_scroll_fixture_frame(&failed_frame, fixture.target_bounds, fixture_offset);
        return Err(io::Error::other(format!(
            "auto-scroll append was rejected: {}; fixture_offset={fixture_offset}; diagnostic_frame={}; fixture_validation={fixture_validation:?}",
            auto.status,
            diagnostic_path.display()
        )));
    }
    thread::sleep(context.settle_delay);
    let second_frame_report = capture_scroll_region_evidence(
        context,
        "03-scroll-second-frame.png",
        initial_selection,
        fixture.target,
        fixture.target_bounds,
        SCROLL_FIXTURE_SCROLL_STEP * 3,
    )?;
    if second_frame_report.fingerprint == initial_frame_report.fingerprint {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "auto scroll completed but the fixture viewport did not change",
        ));
    }
    lower_scroll_fixture(fixture.target)?;

    focus_owned_window(scroll_control, context.timeout)?;
    thread::sleep(context.settle_delay);
    scroll_control = owned_window(scroll_control.handle)?;
    let control_ready = capture_evidence(context, "03-scroll-control-ready.png", scroll_control)?;
    record_step(
        report,
        &context.report_path,
        "scroll_roundtrip_control_ready",
        scroll_control,
        Some(&control_ready),
    )?;
    let finish_point = scroll_finish_button_screen_point(scroll_control.bounds)?;
    let foreground = inject_settled_mouse_click(
        scroll_control.handle,
        finish_point,
        Duration::from_millis(150),
    )?;
    record_step(
        report,
        &context.report_path,
        "scroll_roundtrip_finish",
        foreground,
        None,
    )?;
    let finished = wait_for_capture_state(
        context,
        &format!(
            "scroll roundtrip Finish at {finish_point:?} in {:?}",
            scroll_control.bounds
        ),
        |state| {
            state.overlay_count == 1
                && state.selection.is_some_and(|selection| {
                    selection.width() == initial_selection.width()
                        && selection.height() > initial_selection.height()
                })
                && !state.more_actions_visible
                && !state.annotation_controls_visible
                && state.manual_scroll_state == "idle"
                && !state.manual_scroll_finish_in_flight
        },
    )?;
    let stitched_selection = finished
        .selection
        .ok_or_else(|| io::Error::other("finished scrolling selection disappeared"))?;
    if stitched_selection.height() <= initial_selection.height() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "stitched selection {stitched_selection:?} did not grow beyond viewport {initial_selection:?}"
            ),
        ));
    }
    wait_for_window_gone(
        scroll_control.handle,
        context.timeout,
        "scroll Finish control close",
    )?;
    let stitched_overlay =
        wait_for_finished_image_overlay(controller.handle, scroll_control.handle, context.timeout)?;
    focus_owned_window(stitched_overlay, context.timeout)?;
    let stitched_evidence =
        capture_evidence(context, "04-scroll-stitched-overlay.png", stitched_overlay)?;
    record_step(
        report,
        &context.report_path,
        "scroll_roundtrip_stitched_overlay",
        stitched_overlay,
        Some(&stitched_evidence),
    )?;
    let stitched_source = query_capture_content(context, context.timeout)?
        .selection
        .ok_or_else(|| io::Error::other("finished scroll editor did not expose stitched pixels"))?;
    validate_frame_dimensions(
        &stitched_source,
        stitched_selection,
        "stitched scroll editor source",
    )?;
    if stitched_source.bounds != stitched_selection {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "stitched source bounds {:?} do not match editor selection {stitched_selection:?}",
                stitched_source.bounds
            ),
        ));
    }
    let export_plan = interaction_plan_for_capture_selection(
        stitched_overlay.handle,
        stitched_source.bounds,
        stitched_selection,
    )?;
    let export = execute_scroll_export(
        context,
        report,
        controller,
        stitched_overlay,
        export_plan,
        stitched_selection,
        &stitched_source,
    )?;
    let final_state = wait_for_capture_state(
        context,
        "scroll roundtrip cleanup",
        scroll_roundtrip_cleanup_complete,
    )?;
    ensure_input_keys_released(&[
        (VK_CONTROL, "Control"),
        (VK_MENU, "Alt"),
        (VK_F24, "F24"),
        (VK_SHIFT, "Shift"),
        (VK_SPACE, "Space"),
        (VK_RETURN, "Enter"),
        (VK_ESCAPE, "Escape"),
        (VK_LBUTTON, "left mouse button"),
    ])?;
    unsafe { ShowWindow(controller.handle, SW_HIDE) };
    wait_for_window_gone(
        controller.handle,
        context.timeout,
        "scroll roundtrip controller hide",
    )?;
    fixture.shutdown(context.timeout.min(Duration::from_secs(3)))?;
    let visible_process_windows = process_windows()?.len();
    if visible_process_windows != 0 {
        return Err(io::Error::other(format!(
            "scroll roundtrip cleanup left {visible_process_windows} visible process window(s)"
        )));
    }
    report.scroll_roundtrip = Some(ScrollRoundtripReport {
        fixture_process_id: fixture.process_id,
        fixture_window,
        wheel_routing,
        requested_selection: drag.selection,
        initial_selection,
        scroll_control: scroll_control_report,
        ready_status: ready.status,
        initial_frame: initial_frame_report,
        auto_capture_status: auto.status,
        second_frame: second_frame_report,
        finish_status: finished.status,
        stitched_selection,
        stitched_height_increased: stitched_selection.height() > initial_selection.height(),
        export,
        manual_scroll_cleanup: ManualScrollCleanupReport {
            state: final_state.manual_scroll_state.clone(),
            frame_count: final_state.manual_scroll_frame_count,
            can_finish: final_state.manual_scroll_can_finish,
            capture_in_flight: final_state.manual_scroll_capture_in_flight,
            finish_in_flight: final_state.manual_scroll_finish_in_flight,
            auto_capture_pending: final_state.manual_scroll_auto_capture_pending,
            selection: final_state.manual_scroll_selection,
            more_actions_visible: final_state.more_actions_visible,
            annotation_controls_visible: final_state.annotation_controls_visible,
        },
        cleanup: CleanupReport {
            session_state: final_state.session_state,
            overlay_count: final_state.overlay_count,
            pinned_count: final_state.pinned_count,
            capture_teardown_pending: final_state.capture_teardown_pending,
            visible_process_windows,
            capture_preflight_ready: final_state.capture_preflight_ready,
        },
    });
    write_report(&context.report_path, report)
}
