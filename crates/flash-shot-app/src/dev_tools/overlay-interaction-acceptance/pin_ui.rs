//! Pin workflow orchestration for the Windows Release acceptance runner.

use super::*;

#[cfg(windows)]
/// Creates three Pins through the real toolbar, drags one, then captures while all three coexist.
pub(super) fn execute(context: &WorkerContext, report: &mut AcceptanceReport) -> io::Result<()> {
    let controller = wait_for_controller(context.timeout)?;
    focus_owned_window(controller, context.timeout)?;
    report.controller_window = Some(controller.report());
    record_step(
        report,
        &context.report_path,
        "pins_controller_ready",
        controller,
        None,
    )?;

    // Keep every selected source deterministic and visibly non-flat while the real overlay is
    // active. The fixture is no-activate and process-isolated, so it cannot receive the toolbar
    // clicks; it is lowered after each source sample and shut down before Pin layout checks.
    let fixture_bounds = pin_coexist_fixture_bounds(context.display.physical_bounds)?;
    let mut source_fixture = ScrollWindowFixture::launch(fixture_bounds, context.timeout)?;
    let source_fixture_window = source_fixture.report()?;
    focus_owned_window(controller, context.timeout)?;

    let creation_actions = ["pin_one_created", "pin_two_created", "pin_three_created"];
    let selection_actions = [
        "pin_one_selection_ready",
        "pin_two_selection_ready",
        "pin_three_selection_ready",
    ];
    let selection_files = [
        "00-pin-one-selection.png",
        "01-pin-two-selection.png",
        "02-pin-three-selection.png",
    ];
    let mut handles = Vec::with_capacity(PIN_COEXIST_COUNT);
    let mut sources = Vec::with_capacity(PIN_COEXIST_COUNT);
    let mut pin_reports = Vec::with_capacity(PIN_COEXIST_COUNT);
    for (index, action) in creation_actions.into_iter().enumerate() {
        if index > 0 {
            raise_scroll_fixture_for_capture(source_fixture.target, fixture_bounds)?;
        }
        let (overlay, plan, selection, requested_selection, source) =
            begin_selected_overlay_with_plan(
                context,
                controller,
                |handle| pin_coexist_interaction_plan_for_window(handle, index),
                context.timeout.min(Duration::from_secs(1)),
            )?;
        thread::sleep(context.settle_delay);
        validate_scroll_fixture_frame(&source, fixture_bounds, 0)?;
        let selected = capture_evidence(context, selection_files[index], overlay)?;
        record_step(
            report,
            &context.report_path,
            selection_actions[index],
            guard_foreground(overlay.handle)?,
            Some(&selected),
        )?;
        lower_scroll_fixture(source_fixture.target)?;
        let foreground = inject_mouse_click(overlay.handle, plan.pin)?;
        let state = wait_for_capture_state(context, action, |state| {
            state.session_state == "idle"
                && state.overlay_count == 0
                && state.pinned_count == index + 1
                && state.pinned_source_bounds == Some(selection)
                && state.capture_preflight_ready
        })?;
        wait_for_overlay_teardown(
            overlay.handle,
            context.display.physical_bounds,
            context.timeout,
            "Pin creation",
        )?;
        let content = query_capture_content(context, context.timeout.min(Duration::from_secs(1)))?;
        if content.pins.len() != index + 1 {
            return Err(io::Error::other(format!(
                "{action} exposed {} Pin source frame(s), expected {}",
                content.pins.len(),
                index + 1
            )));
        }
        let pinned = content
            .pins
            .last()
            .ok_or_else(|| io::Error::other("new Pin did not expose its source frame"))?
            .clone();
        let pixel_match = validate_same_pixel_content(&source, &pinned, action)?;
        let pin = wait_for_new_pin(controller.handle, &handles, context.timeout)?;
        pin_reports.push(PinReport {
            requested_selection,
            selection,
            source_bounds: state
                .pinned_source_bounds
                .ok_or_else(|| io::Error::other("new Pin source bounds disappeared"))?,
            window: pin.report(),
            content: pixel_match,
        });
        handles.push(pin.handle);
        sources.push(pinned);
        record_step(report, &context.report_path, action, foreground, None)?;
    }

    source_fixture.shutdown(context.timeout.min(Duration::from_secs(3)))?;
    let source_fixture_cleaned_up = source_fixture.stopped;
    if !source_fixture_cleaned_up {
        return Err(io::Error::other(
            "Pin source fixture did not report a completed cleanup",
        ));
    }

    let initial_windows = owned_windows(&handles)?;
    let layout = horizontal_pin_layout(context.display.physical_bounds, &initial_windows)?;
    for (window, bounds) in initial_windows.iter().zip(&layout) {
        move_owned_window(window.handle, *bounds)?;
    }
    thread::sleep(context.settle_delay);
    let arranged_windows = owned_windows(&handles)?;
    validate_exact_window_bounds(&arranged_windows, &layout, "arranged Pin windows")?;
    focus_owned_window(arranged_windows[0], context.timeout)?;
    validate_pin_sources(context, &sources, "arranged Pins")?;
    let before_region =
        window_union_with_margin(&arranged_windows, context.display.physical_bounds, 20)?;
    let before = capture_region_evidence(
        context,
        "03-pins-before-capture.png",
        arranged_windows[0],
        before_region,
    )?;
    record_step(
        report,
        &context.report_path,
        "pins_arranged",
        arranged_windows[0],
        Some(&before),
    )?;

    let drag_before = arranged_windows[0].bounds;
    let drag_start = PhysicalPoint {
        x: drag_before.left + drag_before.width() as i32 / 2,
        y: drag_before.bottom - 24,
    };
    let drag_end = PhysicalPoint {
        x: drag_start.x + 48,
        y: drag_start.y - 40,
    };
    let injected_drag = inject_native_window_drag(handles[0], drag_start, drag_end)?;
    let drag_after = wait_for_window_drag(
        handles[0],
        drag_before,
        injected_drag.start,
        injected_drag.end,
        context.timeout,
    )?;
    let dragged_windows = owned_windows(&handles)?;
    validate_non_overlapping_windows(&dragged_windows, context.display.physical_bounds)?;
    let dragged_region =
        window_union_with_margin(&dragged_windows, context.display.physical_bounds, 20)?;
    let dragged = capture_region_evidence(
        context,
        "04-pin-pointer-drag.png",
        dragged_windows[0],
        dragged_region,
    )?;
    record_step(
        report,
        &context.report_path,
        "pin_pointer_drag",
        injected_drag.foreground,
        Some(&dragged),
    )?;
    let expected_delta_x = injected_drag.end.x - injected_drag.start.x;
    let expected_delta_y = injected_drag.end.y - injected_drag.start.y;
    let pointer_drag = WindowDragReport {
        handle: handles[0] as usize,
        before: drag_before,
        after: drag_after,
        pointer_start: injected_drag.start,
        pointer_end: injected_drag.end,
        expected_delta_x,
        expected_delta_y,
        actual_delta_x: drag_after.left - drag_before.left,
        actual_delta_y: drag_after.top - drag_before.top,
    };
    let lock_mode = exercise_pin_lock_mode(context, report, handles[0])?;

    // The default Pins coexistence probe uses an injected sink and never touches user data.
    // An explicit system-clipboard flag opts into one real Ctrl+C on the first Pin so the
    // production asynchronous Pin copy worker and shared clipboard lease are exercised too.
    let system_clipboard_copy = if context.use_system_clipboard {
        Some(execute_pin_system_clipboard_copy(
            context,
            report,
            owned_window(handles[0])?,
            &sources[0],
        )?)
    } else {
        None
    };

    validate_pin_sources(context, &sources, "Pins before coexistence capture")?;
    let baseline_windows = owned_windows(&handles)?;
    focus_owned_window(baseline_windows[0], context.timeout)?;
    let foreground = inject_capture_shortcut(baseline_windows[0].handle)?;
    record_step(
        report,
        &context.report_path,
        "pins_capture_shortcut",
        foreground,
        None,
    )?;
    let overlay = wait_for_overlay(
        controller.handle,
        context.display.physical_bounds,
        context.timeout,
    )?;
    // Settings is hidden by an asynchronous production callback. Do not drag until it is gone,
    // otherwise the source frame can contain the controller or a stale native dialog.
    wait_for_window_gone(
        controller.handle,
        context.timeout,
        "capture overlay hides Settings",
    )?;
    focus_owned_window(overlay, context.timeout)?;
    thread::sleep(context.settle_delay);
    let plan = interaction_plan_for_window(overlay.handle)?;
    // The full-display overlay consumes physical screen pixels directly. Reuse the same
    // display-coordinate path as Pin creation so the measured gesture and committed selection do
    // not apply the native client origin twice.
    let capture_drag = inject_mouse_drag_in_display_pixels(
        overlay.handle,
        plan.drag_start,
        plan.drag_end,
        context.display.physical_bounds,
    )?;
    let active = wait_for_capture_state(context, "capture with three Pins", |state| {
        state.session_state == "selecting"
            && state.selection.is_some()
            && state.overlay_count == 1
            && state.pinned_count == PIN_COEXIST_COUNT
    })?;
    let committed_capture_selection = active
        .selection
        .ok_or_else(|| io::Error::other("coexistence capture selection disappeared"))?;
    validate_selection_geometry(
        capture_drag.selection,
        committed_capture_selection,
        "capture with three Pins",
    )?;
    validate_pin_sources(context, &sources, "Pins during coexistence capture")?;
    let windows_during_capture = owned_windows(&handles)?;
    validate_same_windows(
        &baseline_windows,
        &windows_during_capture,
        "active coexistence overlay",
    )?;
    thread::sleep(context.settle_delay);
    let overlay_evidence = capture_evidence(context, "05-overlay-with-pins.png", overlay)?;
    record_step(
        report,
        &context.report_path,
        "pins_overlay_selection",
        capture_drag.foreground,
        Some(&overlay_evidence),
    )?;

    let foreground = inject_mouse_click(overlay.handle, plan.cancel)?;
    wait_for_window_gone(overlay.handle, context.timeout, "Pin coexistence Cancel")?;
    record_step(
        report,
        &context.report_path,
        "pins_overlay_cancel",
        foreground,
        None,
    )?;
    let after_cancel = wait_for_capture_state(context, "Pin coexistence Cancel", |state| {
        state.overlay_count == 0
            && state.pinned_count == PIN_COEXIST_COUNT
            && state.capture_preflight_ready
            && matches!(
                state.session_state.as_str(),
                "idle" | "completed" | "cancelled"
            )
    })?;
    validate_pin_sources(context, &sources, "Pins after coexistence Cancel")?;
    let windows_after_cancel = owned_windows(&handles)?;
    validate_same_windows(
        &baseline_windows,
        &windows_after_cancel,
        "coexistence Cancel",
    )?;
    focus_owned_window(windows_after_cancel[0], context.timeout)?;
    let after_region =
        window_union_with_margin(&windows_after_cancel, context.display.physical_bounds, 20)?;
    let after = capture_region_evidence(
        context,
        "06-pins-after-cancel.png",
        windows_after_cancel[0],
        after_region,
    )?;
    record_step(
        report,
        &context.report_path,
        "pins_survived_cancel",
        windows_after_cancel[0],
        Some(&after),
    )?;

    let pointer_pin = owned_window(handles[0])?;
    focus_owned_window(pointer_pin, context.timeout)?;
    let foreground = inject_mouse_click(
        pointer_pin.handle,
        pin_close_button_point(
            client_bounds_for_window(pointer_pin.handle)?,
            pointer_pin.dpi as f32 / WINDOWS_BASE_DPI,
        )?,
    )?;
    wait_for_window_gone(
        pointer_pin.handle,
        context.timeout,
        "Pin persistent Close button",
    )?;
    wait_for_capture_state(context, "Pin persistent Close button", |state| {
        state.overlay_count == 0
            && state.pinned_count == PIN_COEXIST_COUNT - 1
            && state.capture_preflight_ready
    })?;
    record_step(
        report,
        &context.report_path,
        "pin_one_close_button",
        foreground,
        None,
    )?;

    let close_actions = ["pin_two_escape", "pin_three_escape"];
    for (index, (handle, action)) in handles[1..].iter().zip(close_actions).enumerate() {
        let pin = owned_window(*handle)?;
        focus_owned_window(pin, context.timeout)?;
        let foreground = inject_key(pin.handle, VK_ESCAPE)?;
        wait_for_window_gone(pin.handle, context.timeout, action)?;
        let remaining = PIN_COEXIST_COUNT - index - 2;
        wait_for_capture_state(context, action, |state| {
            state.overlay_count == 0
                && state.pinned_count == remaining
                && state.capture_preflight_ready
        })?;
        record_step(report, &context.report_path, action, foreground, None)?;
    }

    let final_state = wait_for_capture_state(context, "Pin coexistence cleanup", |state| {
        state.overlay_count == 0 && state.pinned_count == 0 && state.capture_preflight_ready
    })?;
    let visible_process_windows = process_windows()?.len();
    if visible_process_windows != 0 {
        return Err(io::Error::other(format!(
            "Pin coexistence cleanup left {visible_process_windows} visible process window(s)"
        )));
    }
    report.pins_coexist = Some(PinsCoexistReport {
        source_fixture_process_id: source_fixture.process_id,
        source_fixture_window,
        source_fixture_cleaned_up,
        pins: pin_reports,
        arranged_windows: arranged_windows
            .into_iter()
            .map(NativeWindow::report)
            .collect(),
        pointer_drag,
        lock_mode,
        requested_capture_selection: capture_drag.selection,
        committed_capture_selection,
        windows_during_capture: windows_during_capture
            .into_iter()
            .map(NativeWindow::report)
            .collect(),
        windows_after_cancel: windows_after_cancel
            .into_iter()
            .map(NativeWindow::report)
            .collect(),
        sources_unchanged_during_capture: true,
        sources_unchanged_after_cancel: true,
        pins_during_capture: active.pinned_count,
        pins_after_cancel: after_cancel.pinned_count,
        closed_with_pointer: 1,
        closed_with_escape: PIN_COEXIST_COUNT - 1,
        system_clipboard_copy,
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

#[cfg(windows)]
/// Tests the Pin lock with real keyboard and pointer input and records both geometry states.
fn exercise_pin_lock_mode(
    context: &WorkerContext,
    report: &mut AcceptanceReport,
    handle: *mut c_void,
) -> io::Result<PinLockReport> {
    let initial = owned_window(handle)?;
    focus_owned_window(initial, context.timeout)?;
    let foreground = inject_ctrl_key(handle, PIN_LOCK_KEY, "L")?;
    let locked_state = wait_for_capture_state(context, "Pin lock shortcut", |state| {
        state.pinned_locked_states.first() == Some(&true)
    })?;
    if locked_state.pinned_count != PIN_COEXIST_COUNT {
        return Err(io::Error::other(
            "Pin lock shortcut changed the registered window count",
        ));
    }
    record_step(
        report,
        &context.report_path,
        "pin_lock_enabled",
        foreground,
        None,
    )?;
    thread::sleep(context.settle_delay);

    let locked_window = owned_window(handle)?;
    let locked_bounds = locked_window.bounds;
    let locked_screenshot = capture_evidence(context, "pin-lock-enabled.png", locked_window)?;
    record_step(
        report,
        &context.report_path,
        "pin_lock_visual_state",
        locked_window,
        Some(&locked_screenshot),
    )?;

    let bounds_before_locked_drag = locked_bounds;
    let locked_drag_start = PhysicalPoint {
        x: locked_bounds.left + locked_bounds.width() as i32 / 2,
        y: locked_bounds.bottom - 24,
    };
    let locked_drag_end = PhysicalPoint {
        x: locked_drag_start.x + 36,
        y: locked_drag_start.y - 24,
    };
    let _ = inject_native_window_drag(handle, locked_drag_start, locked_drag_end)?;
    thread::sleep(context.settle_delay);
    let bounds_after_locked_drag = owned_window(handle)?.bounds;
    let locked_drag_blocked = bounds_after_locked_drag == bounds_before_locked_drag;
    if !locked_drag_blocked {
        return Err(io::Error::other(format!(
            "locked Pin changed bounds during drag input: {bounds_before_locked_drag:?} -> {bounds_after_locked_drag:?}"
        )));
    }

    let foreground = inject_ctrl_key(handle, PIN_ZOOM_IN_KEY, "Equals")?;
    thread::sleep(context.settle_delay);
    let bounds_after_locked_zoom = owned_window(handle)?.bounds;
    let locked_zoom_blocked = bounds_after_locked_zoom == bounds_after_locked_drag;
    if !locked_zoom_blocked {
        return Err(io::Error::other(format!(
            "locked Pin changed bounds during zoom input: {bounds_after_locked_drag:?} -> {bounds_after_locked_zoom:?}"
        )));
    }
    record_step(
        report,
        &context.report_path,
        "pin_lock_blocks_geometry",
        foreground,
        None,
    )?;

    let foreground = inject_ctrl_key(handle, PIN_LOCK_KEY, "L")?;
    let unlocked_state = wait_for_capture_state(context, "Pin unlock shortcut", |state| {
        state.pinned_locked_states.first() == Some(&false)
    })?;
    if unlocked_state.pinned_count != PIN_COEXIST_COUNT {
        return Err(io::Error::other(
            "Pin unlock shortcut changed the registered window count",
        ));
    }
    thread::sleep(context.settle_delay);
    let unlocked_window = owned_window(handle)?;
    record_step(
        report,
        &context.report_path,
        "pin_lock_disabled",
        foreground,
        Some(&capture_evidence(
            context,
            "pin-lock-disabled.png",
            unlocked_window,
        )?),
    )?;

    let bounds_before_unlocked_drag = unlocked_window.bounds;
    let drag_start = PhysicalPoint {
        x: bounds_before_unlocked_drag.left + bounds_before_unlocked_drag.width() as i32 / 2,
        y: bounds_before_unlocked_drag.bottom - 24,
    };
    let drag_end = PhysicalPoint {
        x: drag_start.x + 36,
        y: drag_start.y - 24,
    };
    let drag = inject_native_window_drag(handle, drag_start, drag_end)?;
    let bounds_after_unlocked_drag = wait_for_window_drag(
        handle,
        bounds_before_unlocked_drag,
        drag.start,
        drag.end,
        context.timeout,
    )?;
    let unlocked_drag_restored = window_drag_matches(
        bounds_before_unlocked_drag,
        bounds_after_unlocked_drag,
        drag.start,
        drag.end,
        2,
    );
    if !unlocked_drag_restored {
        return Err(io::Error::other(
            "Pin movement did not resume after unlocking",
        ));
    }
    record_step(
        report,
        &context.report_path,
        "pin_unlocked_drag_restored",
        drag.foreground,
        None,
    )?;

    let bounds_before_unlocked_zoom = owned_window(handle)?.bounds;
    let foreground = inject_ctrl_key(handle, PIN_ZOOM_IN_KEY, "Equals")?;
    let deadline = Instant::now() + context.timeout;
    let bounds_after_unlocked_zoom = loop {
        let bounds = owned_window(handle)?.bounds;
        if bounds.width() > bounds_before_unlocked_zoom.width()
            && bounds.height() > bounds_before_unlocked_zoom.height()
        {
            break bounds;
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Pin zoom did not resume after unlocking",
            ));
        }
        thread::sleep(Duration::from_millis(25));
    };
    record_step(
        report,
        &context.report_path,
        "pin_unlocked_zoom_restored",
        foreground,
        None,
    )?;

    Ok(PinLockReport {
        lock_shortcut: "Ctrl+L",
        lock_state_observed: true,
        bounds_before_locked_drag,
        bounds_after_locked_drag,
        locked_drag_blocked,
        bounds_before_locked_zoom: bounds_after_locked_drag,
        bounds_after_locked_zoom,
        locked_zoom_blocked,
        unlock_state_observed: true,
        bounds_before_unlocked_drag,
        bounds_after_unlocked_drag,
        unlocked_drag_restored,
        bounds_before_unlocked_zoom,
        bounds_after_unlocked_zoom,
        unlocked_zoom_restored: true,
    })
}

#[cfg(windows)]
/// Exercises one real Pin Ctrl+C and verifies PNG, CF_DIB, and consumer pixels without hiding
/// the clipboard side effect behind the in-memory acceptance sink.
fn execute_pin_system_clipboard_copy(
    context: &WorkerContext,
    report: &mut AcceptanceReport,
    pin: NativeWindow,
    source: &CaptureFrame,
) -> io::Result<PinSystemClipboardCopyReport> {
    if context.copy_results.is_some() {
        return Err(io::Error::other(
            "Pin system clipboard copy requires the real clipboard path",
        ));
    }
    let mut consumer = ClipboardConsumer::launch(&context.session_root, context.timeout)?;
    let consumer_ready_before_click = consumer.ready_path.is_file();
    let clipboard_sequence_before = consumer.arm()?;
    let consumer_observing_before_click = wait_for_path(
        &consumer.observing_path,
        context.timeout,
        "Pin clipboard consumer observing marker",
    )?;
    // SAFETY: this call only reads the monotonic Windows clipboard change counter.
    let before_input_sequence = unsafe { GetClipboardSequenceNumber() };
    if before_input_sequence != clipboard_sequence_before {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "system clipboard changed before Pin Copy input ({clipboard_sequence_before} -> {before_input_sequence})"
            ),
        ));
    }
    let copy_started_qpc = qpc_ticks()?;
    let foreground = inject_ctrl_c(pin.handle)?;
    let consumer_result = consumer.wait_result(context.timeout)?;
    if consumer_result.previous_sequence != clipboard_sequence_before {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Pin clipboard consumer observed a mismatched launch sequence",
        ));
    }
    for path in [
        Path::new(&consumer_result.png_path),
        Path::new(&consumer_result.dib_path),
        Path::new(&consumer_result.consumer_image_path),
    ] {
        ensure_path_within(path, &context.session_root)?;
    }

    let copied = CaptureFrame::open_png(&consumer_result.consumer_image_path)?;
    let clipboard_png = fs::read(&consumer_result.png_path)?;
    let clipboard_dib = fs::read(&consumer_result.dib_path)?;
    let copy_state = wait_for_capture_state(context, "Pin clipboard lease release", |state| {
        !state.clipboard_write_active
    })?;
    thread::sleep(context.settle_delay);
    let evidence = capture_evidence(context, "04a-pin-system-copy.png", pin)?;
    record_step(
        report,
        &context.report_path,
        "pin_system_clipboard_copy",
        foreground,
        Some(&evidence),
    )?;

    // SAFETY: this call only reads the monotonic Windows clipboard change counter.
    let clipboard_sequence_after = unsafe { GetClipboardSequenceNumber() };
    if clipboard_sequence_after != consumer_result.observed_sequence {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "system clipboard changed after Pin Copy ({:?} -> {clipboard_sequence_after})",
                consumer_result.observed_sequence
            ),
        ));
    }
    validate_frame_dimensions(&copied, source.bounds, "Pin system clipboard image")?;
    let consumer_image_content =
        validate_same_pixel_content(source, &copied, "Pin system clipboard image")?;
    let png_path = PathBuf::from(&consumer_result.png_path);
    let decoded_png = CaptureFrame::open_png(&png_path)?;
    validate_frame_dimensions(&decoded_png, source.bounds, "Pin clipboard PNG format")?;
    let png_content = validate_same_pixel_content(source, &decoded_png, "Pin clipboard PNG")?;
    let decoded_dib = decode_clipboard_dib(&clipboard_dib)?;
    validate_frame_dimensions(&decoded_dib, source.bounds, "Pin clipboard CF_DIB format")?;
    let dib_content = validate_same_pixel_content(source, &decoded_dib, "Pin clipboard CF_DIB")?;
    if clipboard_sequence_after == clipboard_sequence_before
        || !consumer_ready_before_click
        || !consumer_observing_before_click
        || !consumer.stopped
    {
        return Err(io::Error::other(
            "Pin clipboard consumer was not ready, did not observe a sequence change, or was not reaped",
        ));
    }
    Ok(PinSystemClipboardCopyReport {
        source_bounds: source.bounds,
        copied_bounds: copied.bounds,
        width: copied.width,
        height: copied.height,
        clipboard_sequence_before,
        clipboard_sequence_after,
        clipboard_sequence_changed: clipboard_sequence_after != clipboard_sequence_before,
        png_format_available: true,
        dib_format_available: true,
        png_bytes: clipboard_png.len(),
        dib_bytes: clipboard_dib.len(),
        png_content,
        dib_content,
        consumer_image_content: Box::new(consumer_image_content),
        timing_clock: "windows_qpc",
        timing_boundary: "ctrl_c_batch_to_consumer_decoded_image",
        input_to_consumer_readable_ms: qpc_elapsed_ms(
            copy_started_qpc,
            consumer_result.consumer_read_qpc_ticks,
        )?,
        consumer_result_path: consumer
            .result_path
            .strip_prefix(&context.session_root)
            .unwrap_or(&consumer.result_path)
            .to_string_lossy()
            .into_owned(),
        consumer_ready_before_click,
        consumer_observing_before_click,
        consumer_cleaned_up: consumer.stopped,
        shared_clipboard_lease_released: !copy_state.clipboard_write_active,
    })
}
