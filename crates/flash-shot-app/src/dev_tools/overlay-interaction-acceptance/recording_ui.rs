//! Recording workflow orchestration for the Windows Release acceptance runner.

use super::*;

#[cfg(windows)]
/// Drives a real Record-page startup failure, retry, pause/resume cycle, and MP4 verification.
pub(super) fn execute_failure_retry(
    context: &WorkerContext,
    report: &mut AcceptanceReport,
) -> io::Result<()> {
    let controller = wait_for_controller(context.timeout)?;
    focus_owned_window(controller, context.timeout)?;
    report.controller_window = Some(controller.report());
    record_step(
        report,
        &context.report_path,
        "controller_ready",
        controller,
        None,
    )?;

    context
        .interaction_commands
        .send_blocking(OverlayInteractionAcceptanceCommand::ShowRecordingSettings)
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "Record page command closed"))?;
    let controller = wait_for_owned_window_visible(controller.handle, context.timeout)?;
    focus_owned_window(controller, context.timeout)?;
    thread::sleep(context.settle_delay);
    let controls = recording_control_plan_for_window(controller.handle, context.locale)?;
    let idle = capture_evidence(context, "01-recording-idle.png", controller)?;
    record_step(
        report,
        &context.report_path,
        "record_page_visible",
        controller,
        Some(&idle),
    )?;

    let foreground = inject_mouse_click(controller.handle, controls.stop)?;
    record_step(
        report,
        &context.report_path,
        "recording_start_failure_click",
        foreground,
        None,
    )?;
    let failed = wait_for_recording_failure(context, "recording startup failure")?;
    let failure_state_cleared = !failed.active
        && !failed.starting
        && !failed.stopping
        && !failed.paused
        && failed.target.is_none();
    if !failure_state_cleared {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("recording failure left lifecycle state active: {failed:?}"),
        ));
    }
    if !failed
        .status
        .contains("acceptance-injected recording startup failure")
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "Record page did not surface the injected startup failure: {}",
                failed.status
            ),
        ));
    }
    // The worker state can report failure before GPUI paints the user-visible error message.
    thread::sleep(context.settle_delay);
    let failed_evidence = capture_evidence(
        context,
        "02-recording-failed.png",
        guard_foreground(controller.handle)?,
    )?;
    record_step(
        report,
        &context.report_path,
        "recording_start_failed",
        foreground,
        Some(&failed_evidence),
    )?;
    record_recording_state(report, &context.report_path, "failed", failed.clone())?;

    let foreground = inject_mouse_click(controller.handle, controls.stop)?;
    record_step(
        report,
        &context.report_path,
        "recording_retry_click",
        foreground,
        None,
    )?;
    let active = wait_for_recording_state(context, "retried display recording", |state| {
        state.active
            && !state.starting
            && !state.stopping
            && state.target.as_deref() == Some("display")
            && state.progress_frame >= 10
            && state.progress_time_us > 0
    })?;
    let expected_source_bounds = context.display.physical_bounds;
    let reported_source_bounds = active.target_bounds.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "retried display recording did not report physical source bounds",
        )
    })?;
    if reported_source_bounds != expected_source_bounds {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "display recording reported {reported_source_bounds:?}, expected {expected_source_bounds:?}"
            ),
        ));
    }
    let reference_timestamp_seconds = active.progress_time_us as f64 / 1_000_000.0;
    let recording_reference = SystemCaptureBackend.capture(expected_source_bounds)?;
    if recording_reference.bounds != expected_source_bounds {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "recording reference captured {:?}, expected {expected_source_bounds:?}",
                recording_reference.bounds
            ),
        ));
    }
    let reference_path = context
        .session_root
        .join("screenshots")
        .join("recording-source-reference.png");
    recording_reference.save_png(&reference_path)?;
    let mut maximum_progress_frame = active.progress_frame;
    report.recording_failure_retry = Some(RecordingFailureRetryReport {
        failure_status: failed.status,
        failure_state_cleared,
        retry_target: "display",
        retry_started: true,
        retry_target_bounds: reported_source_bounds,
    });
    record_recording_state(
        report,
        &context.report_path,
        "retry_recording",
        active.clone(),
    )?;
    let retried_evidence = capture_evidence(
        context,
        "03-recording-retried.png",
        guard_foreground(controller.handle)?,
    )?;
    record_step(
        report,
        &context.report_path,
        "recording_retry_started",
        controller,
        Some(&retried_evidence),
    )?;

    let foreground = inject_mouse_click(controller.handle, controls.pause_or_resume)?;
    let paused = wait_for_recording_state(context, "paused retried recording", |state| {
        state.active && state.paused && !state.stopping
    })?;
    maximum_progress_frame = maximum_progress_frame.max(paused.progress_frame);
    record_recording_state(report, &context.report_path, "paused", paused)?;
    thread::sleep(context.settle_delay.max(PAUSE_STABILITY_INTERVAL));
    let paused_before =
        wait_for_recording_state(context, "settled paused retried recording", |state| {
            state.active && state.paused && !state.stopping
        })?;
    maximum_progress_frame = maximum_progress_frame.max(paused_before.progress_frame);
    record_recording_state(
        report,
        &context.report_path,
        "paused_stability_start",
        paused_before.clone(),
    )?;
    thread::sleep(PAUSE_STABILITY_INTERVAL);
    let paused_after = query_recording_state(context, context.timeout.min(Duration::from_secs(1)))?;
    validate_paused_progress(&paused_before, &paused_after)?;
    maximum_progress_frame = maximum_progress_frame.max(paused_after.progress_frame);
    record_recording_state(
        report,
        &context.report_path,
        "paused_stability_end",
        paused_after.clone(),
    )?;
    let paused_evidence = capture_evidence(
        context,
        "04-paused.png",
        guard_foreground(controller.handle)?,
    )?;
    record_step(
        report,
        &context.report_path,
        "pause_click",
        foreground,
        Some(&paused_evidence),
    )?;

    let foreground = inject_mouse_click(controller.handle, controls.pause_or_resume)?;
    let resumed = wait_for_recording_state(context, "resumed retried recording", |state| {
        state.active
            && !state.paused
            && !state.stopping
            && state.progress_frame > paused_after.progress_frame
            && state.progress_time_us > paused_after.progress_time_us
    })?;
    maximum_progress_frame = maximum_progress_frame.max(resumed.progress_frame);
    record_recording_state(report, &context.report_path, "resumed", resumed)?;
    thread::sleep(context.settle_delay);
    let resumed_evidence = capture_evidence(
        context,
        "05-resumed.png",
        guard_foreground(controller.handle)?,
    )?;
    record_step(
        report,
        &context.report_path,
        "resume_click",
        foreground,
        Some(&resumed_evidence),
    )?;

    let foreground = inject_mouse_click(controller.handle, controls.stop)?;
    record_step(report, &context.report_path, "stop_click", foreground, None)?;
    let stopping = wait_for_recording_state(context, "stopping retried recording", |state| {
        state.stopping
    })?;
    maximum_progress_frame = maximum_progress_frame.max(stopping.progress_frame);
    record_recording_state(report, &context.report_path, "stopping", stopping)?;
    let saved = wait_for_recording_state(context, "saved retried recording", recording_saved)?;
    maximum_progress_frame = maximum_progress_frame.max(saved.progress_frame);
    record_recording_state(report, &context.report_path, "saved", saved)?;
    thread::sleep(context.settle_delay);
    let saved_evidence = capture_evidence(
        context,
        "06-saved.png",
        guard_foreground(controller.handle)?,
    )?;
    record_step(
        report,
        &context.report_path,
        "saved_visible",
        controller,
        Some(&saved_evidence),
    )?;

    let output = single_recording_output(&context.session_root.join("recordings"))?;
    let output_bytes = fs::metadata(&output)?.len();
    if output_bytes == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "retried recording MP4 is empty",
        ));
    }
    let capabilities = discover()?;
    let media = probe_media(capabilities.executable(), &output)?;
    validate_recorded_media(expected_source_bounds, &media)?;
    let decoded_path = context
        .session_root
        .join("screenshots")
        .join("recording-decoded-frame.png");
    extract_video_frame(
        capabilities.executable(),
        &output,
        reference_timestamp_seconds,
        &decoded_path,
    )?;
    let decoded_frame = CaptureFrame::open_png(&decoded_path)?;
    let content_comparison =
        validate_recording_frame_content(&recording_reference, &decoded_frame)?;
    let content = recording_content_report(
        "screenshots/recording-source-reference.png".to_owned(),
        "screenshots/recording-decoded-frame.png".to_owned(),
        reference_timestamp_seconds,
        content_comparison,
    );
    let relative_output = output
        .strip_prefix(&context.session_root)
        .unwrap_or(&output)
        .to_string_lossy()
        .into_owned();
    report.recording = Some(RecordingReport {
        target: "display",
        requested_selection: expected_source_bounds,
        source_bounds: expected_source_bounds,
        reported_source_bounds,
        window_title: None,
        output: relative_output,
        output_bytes,
        ffmpeg_version: capabilities.version().to_owned(),
        codec_name: media.codec_name,
        width: media.width,
        height: media.height,
        duration_seconds: media.duration_seconds,
        pause_observed: true,
        resume_observed: true,
        maximum_progress_frame,
        content,
        window_dynamics: None,
    });
    write_report(&context.report_path, report)
}

#[cfg(windows)]
/// Drives the real overlay entry plus Record-page pause, resume, stop, and MP4 verification.
pub(super) fn execute(context: &WorkerContext, report: &mut AcceptanceReport) -> io::Result<()> {
    let target = context
        .record_target
        .ok_or_else(|| io::Error::other("recording workflow requires a target"))?;
    let controller = wait_for_controller(context.timeout)?;
    focus_owned_window(controller, context.timeout)?;
    report.controller_window = Some(controller.report());
    record_step(
        report,
        &context.report_path,
        "controller_ready",
        controller,
        None,
    )?;

    let foreground = inject_capture_shortcut(controller.handle)?;
    record_step(
        report,
        &context.report_path,
        "capture_shortcut",
        foreground,
        None,
    )?;
    let overlay = wait_for_overlay(
        controller.handle,
        context.display.physical_bounds,
        context.timeout,
    )?;
    focus_owned_window(overlay, context.timeout)?;
    thread::sleep(context.settle_delay);
    let plan = recording_interaction_plan_for_window(
        overlay,
        controller,
        context.display.physical_bounds,
    )?;

    let drag = inject_mouse_drag_in_display_pixels(
        overlay.handle,
        plan.drag_start,
        plan.drag_end,
        context.display.physical_bounds,
    )?;
    let foreground = drag.foreground;
    thread::sleep(context.settle_delay);
    let selected = capture_evidence(context, "01-selected.png", overlay)?;
    record_step(
        report,
        &context.report_path,
        "selection_drag",
        foreground,
        Some(&selected),
    )?;
    let committed_state = wait_for_capture_state(context, "recording selection", |state| {
        state.session_state == "selecting" && state.selection.is_some() && state.overlay_count == 1
    })?;
    let committed_selection = committed_state
        .selection
        .ok_or_else(|| io::Error::other("recording selection disappeared"))?;
    validate_selection_geometry(
        drag.selection,
        committed_selection,
        "recording overlay selection",
    )?;
    let mut plan = plan;
    let toolbar_bounds = committed_state
        .action_toolbar_bounds
        .ok_or_else(|| io::Error::other("recording selection did not expose toolbar bounds"))?;
    plan.more =
        toolbar_result_action_point_100_percent(toolbar_bounds, WorkspaceResultAction::More);
    let mut window_fixture = None;
    let (expected_source_bounds, window_title) = match target {
        RecordTargetOption::Area => (committed_selection, None),
        RecordTargetOption::Window => {
            let fixture = RecordingWindowFixture::launch(committed_selection, context.timeout)?;
            fixture.hide_backdrop()?;
            focus_owned_window(overlay, context.timeout)?;
            thread::sleep(context.settle_delay);
            let center = PhysicalPoint {
                x: committed_selection.left + committed_selection.width() as i32 / 2,
                y: committed_selection.top + committed_selection.height() as i32 / 2,
            };
            let window_target = SystemWindowInspector
                .window_capture_target_at(center)?
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        "no external titled window is visible beneath the selection center",
                    )
                })?;
            if window_target.title != fixture.target_title
                || window_target.bounds != fixture.initial_bounds
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "window inspector selected {:?} at {:?}, expected fixture {:?} at {:?}",
                        window_target.title,
                        window_target.bounds,
                        fixture.target_title,
                        fixture.initial_bounds
                    ),
                ));
            }
            let bounds = fixture.initial_bounds;
            let title = fixture.target_title.clone();
            fixture.show_backdrop()?;
            thread::sleep(context.settle_delay);
            window_fixture = Some(fixture);
            (bounds, Some(title))
        }
    };

    let foreground = inject_mouse_click(overlay.handle, plan.more)?;
    let more_state = wait_for_capture_state(context, "recording More panel", |state| {
        state.session_state == "selecting" && state.more_actions_visible && state.overlay_count == 1
    })?;
    thread::sleep(context.settle_delay);
    let more = capture_evidence(context, "02-more.png", overlay)?;
    ensure_evidence_changed(&selected, &more, "More did not change the overlay")?;
    record_step(
        report,
        &context.report_path,
        "more",
        foreground,
        Some(&more),
    )?;

    let menu_client_bounds = more_state
        .secondary_menu_bounds
        .ok_or_else(|| io::Error::other("recording More panel did not expose its bounds"))?;
    let client_bounds = client_bounds_for_window(overlay.handle)?;
    let menu_bounds = translated_rect(menu_client_bounds, client_bounds.left, client_bounds.top)?;
    let record_action = match target {
        RecordTargetOption::Area => WorkspaceMoreAction::RecordArea,
        RecordTargetOption::Window => WorkspaceMoreAction::RecordWindow,
    };
    let record_point =
        more_menu_action_screen_point(menu_bounds, overlay.dpi, record_action, false, false)?;
    let foreground = inject_mouse_click(overlay.handle, record_point)?;
    record_step(
        report,
        &context.report_path,
        match target {
            RecordTargetOption::Area => "record_area_click",
            RecordTargetOption::Window => "record_window_click",
        },
        foreground,
        None,
    )?;
    wait_for_window_gone(
        overlay.handle,
        context.timeout,
        match target {
            RecordTargetOption::Area => "Record area",
            RecordTargetOption::Window => "Record window",
        },
    )?;

    let active = wait_for_recording_state(context, "active recording", |state| {
        state.active
            && !state.starting
            && !state.stopping
            && state.target.as_deref() == Some(target.label())
            && state.progress_frame >= 10
            && state.progress_time_us > 0
    })?;
    let reported_source_bounds = active.target_bounds.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "active recording did not report physical source bounds",
        )
    })?;
    validate_recording_target_bounds(target, expected_source_bounds, reported_source_bounds)?;
    let reference_timestamp_seconds = active.progress_time_us as f64 / 1_000_000.0;
    let recording_reference = SystemCaptureBackend.capture(expected_source_bounds)?;
    if recording_reference.bounds != expected_source_bounds {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "recording reference captured {:?}, expected {expected_source_bounds:?}",
                recording_reference.bounds
            ),
        ));
    }
    let reference_path = context
        .session_root
        .join("screenshots")
        .join("recording-source-reference.png");
    recording_reference.save_png(&reference_path)?;
    let mut maximum_progress_frame = active.progress_frame;
    record_recording_state(report, &context.report_path, "recording", active.clone())?;
    let mut pending_window_phases = Vec::new();
    let mut window_fixture_seed = None;
    if let Some(fixture) = window_fixture.as_ref() {
        let initial_hold_after = active
            .progress_time_us
            .checked_add(WINDOW_PHASE_HOLD_US)
            .ok_or_else(|| io::Error::other("initial recording timestamp overflow"))?;
        let initial_held = wait_for_recording_state(context, "initial window hold", |state| {
            state.active
                && !state.paused
                && !state.stopping
                && state.progress_time_us >= initial_hold_after
        })?;
        let initial_held_bounds = initial_held.target_bounds.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "initial window hold lost physical source bounds",
            )
        })?;
        validate_recording_target_bounds(
            RecordTargetOption::Window,
            expected_source_bounds,
            initial_held_bounds,
        )?;
        maximum_progress_frame = maximum_progress_frame.max(initial_held.progress_frame);

        let moved = fixture.move_target()?;
        pending_window_phases.push(capture_recording_window_phase(
            context,
            report,
            "window_moved",
            "recording-window-moved",
            expected_source_bounds,
            moved,
        )?);
        let resized = fixture.resize_target()?;
        pending_window_phases.push(capture_recording_window_phase(
            context,
            report,
            "window_resized",
            "recording-window-resized",
            expected_source_bounds,
            resized,
        )?);
        let occluded = fixture.occlude_target()?;
        pending_window_phases.push(capture_recording_window_phase(
            context,
            report,
            "window_occluded",
            "recording-window-occluded",
            expected_source_bounds,
            occluded,
        )?);
        let minimized = fixture.minimize_target(context.timeout)?;
        pending_window_phases.push(capture_recording_window_phase(
            context,
            report,
            "window_minimized",
            "recording-window-minimized",
            expected_source_bounds,
            minimized,
        )?);
        maximum_progress_frame = pending_window_phases
            .iter()
            .fold(maximum_progress_frame, |maximum, phase| {
                maximum.max(phase.maximum_progress_frame)
            });
        fixture.hide_all()?;
        window_fixture_seed = Some(RecordingWindowFixtureReportSeed {
            process_id: fixture.process_id,
            target_title: fixture.target_title.clone(),
            initial_bounds: fixture.initial_bounds,
            moved_bounds: fixture.moved_bounds,
            resized_bounds: fixture.resized_bounds,
        });
    }

    context
        .interaction_commands
        .send_blocking(OverlayInteractionAcceptanceCommand::ShowRecordingSettings)
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "Record page command closed"))?;
    let controller = wait_for_owned_window_visible(controller.handle, context.timeout)?;
    focus_owned_window(controller, context.timeout)?;
    thread::sleep(context.settle_delay);
    let controls = recording_control_plan_for_window(controller.handle, context.locale)?;
    let recording = capture_evidence(context, "03-recording.png", controller)?;
    record_step(
        report,
        &context.report_path,
        "record_page_visible",
        controller,
        Some(&recording),
    )?;

    let foreground = inject_mouse_click(controller.handle, controls.pause_or_resume)?;
    let paused = wait_for_recording_state(context, "paused recording", |state| {
        state.active && state.paused && !state.stopping
    })?;
    maximum_progress_frame = maximum_progress_frame.max(paused.progress_frame);
    record_recording_state(report, &context.report_path, "paused", paused)?;
    thread::sleep(context.settle_delay.max(PAUSE_STABILITY_INTERVAL));
    let paused_before = wait_for_recording_state(context, "settled paused recording", |state| {
        state.active && state.paused && !state.stopping
    })?;
    maximum_progress_frame = maximum_progress_frame.max(paused_before.progress_frame);
    record_recording_state(
        report,
        &context.report_path,
        "paused_stability_start",
        paused_before.clone(),
    )?;
    thread::sleep(PAUSE_STABILITY_INTERVAL);
    let paused_after = query_recording_state(context, context.timeout.min(Duration::from_secs(1)))?;
    validate_paused_progress(&paused_before, &paused_after)?;
    maximum_progress_frame = maximum_progress_frame.max(paused_after.progress_frame);
    record_recording_state(
        report,
        &context.report_path,
        "paused_stability_end",
        paused_after.clone(),
    )?;
    let paused_evidence = capture_evidence(
        context,
        "04-paused.png",
        guard_foreground(controller.handle)?,
    )?;
    record_step(
        report,
        &context.report_path,
        "pause_click",
        foreground,
        Some(&paused_evidence),
    )?;

    let foreground = inject_mouse_click(controller.handle, controls.pause_or_resume)?;
    let resumed = wait_for_recording_state(context, "resumed recording", |state| {
        state.active
            && !state.paused
            && !state.stopping
            && state.progress_frame > paused_after.progress_frame
            && state.progress_time_us > paused_after.progress_time_us
    })?;
    maximum_progress_frame = maximum_progress_frame.max(resumed.progress_frame);
    record_recording_state(report, &context.report_path, "resumed", resumed)?;
    thread::sleep(context.settle_delay);
    let resumed_evidence = capture_evidence(
        context,
        "05-resumed.png",
        guard_foreground(controller.handle)?,
    )?;
    record_step(
        report,
        &context.report_path,
        "resume_click",
        foreground,
        Some(&resumed_evidence),
    )?;

    let foreground = inject_mouse_click(controller.handle, controls.stop)?;
    record_step(report, &context.report_path, "stop_click", foreground, None)?;
    let stopping = wait_for_recording_state(context, "stopping recording", |state| state.stopping)?;
    maximum_progress_frame = maximum_progress_frame.max(stopping.progress_frame);
    record_recording_state(report, &context.report_path, "stopping", stopping)?;

    // Stopping may finish between two frames, so it is recorded from production state rather than
    // attaching that label to a screenshot which might already contain the Saved UI.
    let saved = wait_for_recording_state(context, "saved recording", recording_saved)?;
    maximum_progress_frame = maximum_progress_frame.max(saved.progress_frame);
    record_recording_state(report, &context.report_path, "saved", saved)?;
    thread::sleep(context.settle_delay);
    let saved_evidence = capture_evidence(
        context,
        "06-saved.png",
        guard_foreground(controller.handle)?,
    )?;
    record_step(
        report,
        &context.report_path,
        "saved_visible",
        guard_foreground(controller.handle)?,
        Some(&saved_evidence),
    )?;

    let fixture_cleaned_up = if let Some(fixture) = window_fixture.as_mut() {
        fixture.shutdown(context.timeout.min(Duration::from_secs(3)))?;
        true
    } else {
        false
    };

    let output = single_recording_output(&context.session_root.join("recordings"))?;
    let output_bytes = fs::metadata(&output)?.len();
    if output_bytes == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recorded MP4 is empty",
        ));
    }
    let capabilities = discover()?;
    let media = probe_media(capabilities.executable(), &output)?;
    validate_recorded_media(expected_source_bounds, &media)?;
    let (content, window_phases, timeline_sample_count) = if window_fixture_seed.is_some() {
        finalize_recording_window_timeline(
            capabilities.executable(),
            &output,
            &context.session_root,
            &recording_reference,
            pending_window_phases,
        )?
    } else {
        if !pending_window_phases.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "area recording unexpectedly produced window dynamic phases",
            ));
        }
        let decoded_path = context
            .session_root
            .join("screenshots")
            .join("recording-decoded-frame.png");
        extract_video_frame(
            capabilities.executable(),
            &output,
            reference_timestamp_seconds,
            &decoded_path,
        )?;
        let decoded_frame = CaptureFrame::open_png(&decoded_path)?;
        let content_comparison =
            validate_recording_frame_content(&recording_reference, &decoded_frame)?;
        (
            recording_content_report(
                "screenshots/recording-source-reference.png".to_owned(),
                "screenshots/recording-decoded-frame.png".to_owned(),
                reference_timestamp_seconds,
                content_comparison,
            ),
            Vec::new(),
            0,
        )
    };
    let window_dynamics = match window_fixture_seed {
        Some(seed) => {
            if window_phases.len() != 4 || !fixture_cleaned_up {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "window recording did not finish four dynamic phases and helper cleanup",
                ));
            }
            let source_bounds_fixed = window_phases
                .iter()
                .all(|phase| phase.reported_source_bounds == expected_source_bounds);
            if !source_bounds_fixed {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "window recording source bounds changed during a dynamic phase",
                ));
            }
            let mut fingerprints = Vec::with_capacity(window_phases.len() + 1);
            fingerprints.push(content.reference_fingerprint.as_str());
            fingerprints.extend(
                window_phases
                    .iter()
                    .map(|phase| phase.content.reference_fingerprint.as_str()),
            );
            validate_recording_phase_fingerprints(&fingerprints)?;
            Some(RecordingWindowDynamicsReport {
                fixture_process_id: seed.process_id,
                target_title: seed.target_title,
                initial_target_bounds: seed.initial_bounds,
                moved_target_bounds: seed.moved_bounds,
                resized_target_bounds: seed.resized_bounds,
                source_bounds_fixed,
                fixture_cleaned_up,
                timeline_frames_per_second: WINDOW_TIMELINE_FRAMES_PER_SECOND,
                timeline_sample_count,
                phases: window_phases,
            })
        }
        None => None,
    };
    let relative_output = output
        .strip_prefix(&context.session_root)
        .unwrap_or(&output)
        .to_string_lossy()
        .into_owned();
    report.recording = Some(RecordingReport {
        target: target.label(),
        requested_selection: drag.selection,
        source_bounds: expected_source_bounds,
        reported_source_bounds,
        window_title,
        output: relative_output,
        output_bytes,
        ffmpeg_version: capabilities.version().to_owned(),
        codec_name: media.codec_name,
        width: media.width,
        height: media.height,
        duration_seconds: media.duration_seconds,
        pause_observed: true,
        resume_observed: true,
        maximum_progress_frame,
        content,
        window_dynamics,
    });
    write_report(&context.report_path, report)
}

#[cfg(windows)]
/// Chooses a recording region outside the controller so opening the Record page cannot change
/// the pixels used by the desktop-reference oracle.
fn recording_interaction_plan_for_window(
    overlay: NativeWindow,
    controller: NativeWindow,
    display: PhysicalRect,
) -> io::Result<InteractionPlan> {
    let client = client_bounds_for_window(overlay.handle)?;
    let scale = overlay.dpi as f32 / WINDOWS_BASE_DPI;
    let (width, height) = overlay_logical_size(client, scale)?;
    let left = display.left.saturating_add(64);
    let right_limit = controller.bounds.left.saturating_sub(32);
    let right = left.saturating_add(520).min(right_limit);
    let top = display.top.saturating_add(128);
    let bottom = top
        .saturating_add(320)
        .min(display.bottom.saturating_sub(64));
    if right - left < 240 || bottom - top < 120 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "display has no safe recording region outside controller {:?}",
                controller.bounds
            ),
        ));
    }
    let logical_point = |point: PhysicalPoint| {
        (
            (point.x - client.left) as f32 / scale,
            (point.y - client.top) as f32 / scale,
        )
    };
    interaction_plan_for_logical_selection(
        client,
        scale,
        width,
        height,
        logical_point(PhysicalPoint { x: left, y: top }),
        logical_point(PhysicalPoint {
            x: right,
            y: bottom,
        }),
    )
}
