//! Copy workflow orchestration for the Windows Release acceptance runner.

use super::*;

#[cfg(windows)]
/// Measures the production Copy path without running unrelated Save, Pin, and recapture actions.
///
/// This is the batch scenario: it still opens a real overlay, commits a real mouse selection,
/// drives the requested Copy gesture, validates the external clipboard consumer, and sends Escape.
pub(super) fn execute_only(
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

    let copy = execute_copy_interaction(
        context,
        report,
        controller,
        interaction_plan_for_window,
        None,
    )?;
    let final_state = wait_for_capture_state(context, "Copy-only final cleanup", |state| {
        state.session_state == "idle"
            && state.selection.is_none()
            && state.overlay_count == 0
            && state.pinned_count == 0
            && !state.capture_teardown_pending
            && state.capture_preflight_ready
    })?;
    let visible_process_windows = process_windows()?.len();
    if visible_process_windows != 0 {
        return Err(io::Error::other(format!(
            "Copy-only cleanup left {visible_process_windows} visible process window(s)"
        )));
    }
    report.capture_actions = Some(CaptureActionReport {
        initial_requested_selection: None,
        recapture_requested_selection: None,
        nudge: None,
        save: None,
        pin: None,
        copy,
        lifecycle: None,
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
/// Sends Copy and Escape back-to-back, then proves cancellation before the injected sink commits.
pub(super) fn execute_cancellation_race(
    context: &WorkerContext,
    report: &mut AcceptanceReport,
) -> io::Result<()> {
    if context.use_system_clipboard {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Copy cancellation race requires the injected clipboard sink",
        ));
    }
    let copy_results = context.copy_results.as_ref().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Copy cancellation race has no injected clipboard sink",
        )
    })?;
    if copy_results.try_recv().is_ok() {
        return Err(io::Error::other(
            "Copy cancellation race sink contained an unexpected earlier frame",
        ));
    }
    let copy_race = context.copy_race.as_ref().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Copy cancellation race has no deterministic commit checkpoint",
        )
    })?;

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

    let (overlay, plan, selection, requested_selection, _source) =
        begin_selected_overlay(context, controller)?;
    thread::sleep(context.settle_delay);
    focus_owned_window(overlay, context.timeout)?;
    let selected = capture_evidence(context, "12-copy-cancellation-race-selection.png", overlay)?;
    record_step(
        report,
        &context.report_path,
        "copy_cancellation_race_selection_ready",
        guard_foreground(overlay.handle)?,
        Some(&selected),
    )?;

    // Wait for the worker to reach the reversible checkpoint, then inject Escape without a
    // reporting write or settle delay between the two real inputs.
    let (copy_foreground, _copy_started_qpc) =
        inject_copy_trigger(overlay.handle, plan.copy, context.copy_trigger)?;
    let checkpoint_reached_before_escape =
        wait_for_copy_race_checkpoint(context, copy_race, selection, context.timeout)?;
    let escape_foreground = inject_key(overlay.handle, VK_ESCAPE)?;
    // Persist both input records only after Escape has been delivered, so report I/O cannot
    // widen the Copy-to-Escape interval being measured.
    record_step(
        report,
        &context.report_path,
        "copy_cancellation_race_trigger",
        copy_foreground,
        None,
    )?;
    record_step(
        report,
        &context.report_path,
        "copy_cancellation_race_escape",
        escape_foreground,
        None,
    )?;

    let checkpoint_reached =
        wait_for_copy_race_release(context, copy_race, selection, context.timeout)?;
    let cancellation_status = context
        .locale
        .text(UiText::CopyCancelledBeforeClipboardChanged)
        .to_owned();
    let cancellation_state =
        wait_for_capture_state(context, "Copy cancellation completion", |state| {
            state.session_state == "selecting"
                && state.selection == Some(selection)
                && state.overlay_count == 1
                && !state.selection_copy_active
                && !state.clipboard_write_active
                && !state.capture_teardown_pending
                && state.capture_preflight_ready
                && state.status == cancellation_status
        })?;
    let clipboard_frame_received = match copy_results.try_recv() {
        Ok(_) => true,
        Err(mpsc::TryRecvError::Empty) => false,
        Err(mpsc::TryRecvError::Disconnected) => {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Copy cancellation race sink disconnected",
            ));
        }
    };
    if clipboard_frame_received {
        return Err(io::Error::other(
            "Escape did not win before the injected clipboard sink commit",
        ));
    }
    let selection_copy_lease_released = !cancellation_state.selection_copy_active;
    let clipboard_write_lease_released = !cancellation_state.clipboard_write_active;
    let status_reports_cancellation = cancellation_state.status == cancellation_status
        && !cancellation_state.status.starts_with("Selection copied")
        && !cancellation_state.status.starts_with("Copy failed:");
    if !selection_copy_lease_released
        || !clipboard_write_lease_released
        || !status_reports_cancellation
    {
        return Err(io::Error::other(
            "Copy cancellation race did not leave cancellation state cleanly",
        ));
    }
    thread::sleep(context.settle_delay);
    focus_owned_window(overlay, context.timeout)?;
    let cancelled = capture_evidence(context, "13-copy-cancellation-race-cancelled.png", overlay)?;
    record_step(
        report,
        &context.report_path,
        "copy_cancellation_race_cancelled",
        guard_foreground(overlay.handle)?,
        Some(&cancelled),
    )?;

    let cleanup_foreground = focus_and_inject_key(overlay, VK_ESCAPE, context.timeout)?;
    wait_for_window_gone(overlay.handle, context.timeout, "Copy cancellation cleanup")?;
    let cleanup_state = wait_for_capture_state(context, "Copy cancellation cleanup", |state| {
        state.session_state == "idle"
            && state.selection.is_none()
            && state.overlay_count == 0
            && state.pinned_count == 0
            && !state.capture_teardown_pending
            && state.background_tasks_idle
            && state.capture_preflight_ready
    })?;
    record_step(
        report,
        &context.report_path,
        "copy_cancellation_race_cleanup",
        cleanup_foreground,
        None,
    )?;
    ensure_capture_input_released()?;
    let visible_process_windows = process_windows()?.len();
    if visible_process_windows != 0 {
        return Err(io::Error::other(format!(
            "Copy cancellation cleanup left {visible_process_windows} visible process window(s)"
        )));
    }
    let final_capture_state = CaptureStateEvidence::from_state(&cleanup_state);
    let cleanup = CleanupReport {
        session_state: cleanup_state.session_state,
        overlay_count: cleanup_state.overlay_count,
        pinned_count: cleanup_state.pinned_count,
        capture_teardown_pending: cleanup_state.capture_teardown_pending,
        visible_process_windows,
        capture_preflight_ready: cleanup_state.capture_preflight_ready,
    };
    let race_report = CopyCancellationRaceReport {
        trigger: context.copy_trigger.label(),
        action: match context.copy_trigger {
            CopyTriggerOption::Toolbar => "toolbar_click",
            CopyTriggerOption::Enter => "enter_key",
        },
        sink: "isolated_observer",
        requested_selection,
        selection,
        escape_injected_immediately: true,
        checkpoint_reached_before_escape,
        checkpoint_reached_before_commit: checkpoint_reached,
        escape_won_before_clipboard_commit: !clipboard_frame_received,
        clipboard_frame_received,
        selection_copy_lease_released,
        clipboard_write_lease_released,
        status_reports_cancellation,
        cancellation_status,
        cancellation_state: CaptureStateEvidence::from_state(&cancellation_state),
        final_capture_state,
        cleanup_status: cleanup_state.status,
        cleanup,
    };
    println!(
        "copy_cancellation_race_evidence: {}",
        serde_json::to_string(&race_report).map_err(io::Error::other)?
    );
    report.copy_cancellation_race = Some(race_report);
    write_report(&context.report_path, report)
}

#[cfg(windows)]
/// Holds the real clipboard through one failed Copy, releases it, and proves the same selection
/// can be copied again with all native formats and resources intact.
pub(super) fn execute_contention_retry(
    context: &WorkerContext,
    report: &mut AcceptanceReport,
) -> io::Result<()> {
    let controller = wait_for_controller(context.timeout)?;
    focus_owned_window(controller, context.timeout)?;
    let (overlay, plan, selection, requested_selection, source) =
        begin_selected_overlay(context, controller)?;
    thread::sleep(context.settle_delay);
    let selected = capture_evidence(context, "12-clipboard-contention-selection.png", overlay)?;
    record_step(
        report,
        &context.report_path,
        "clipboard_contention_selection_ready",
        guard_foreground(overlay.handle)?,
        Some(&selected),
    )?;

    let mut consumer = ClipboardConsumer::launch(&context.session_root, context.timeout)?;
    let consumer_ready_before_input = consumer.ready_path.is_file();
    let clipboard_sequence_before = consumer.arm()?;
    let consumer_observing_before_input = wait_for_path(
        &consumer.observing_path,
        context.timeout,
        "clipboard consumer observing marker",
    )?;
    let mut holder = ClipboardContentionHolder::launch(&context.session_root, context.timeout)?;
    let holder_held_before_input = holder.held_path.is_file();
    // The holder must not mutate the user's existing clipboard before the first production input.
    let before_input_sequence = unsafe { GetClipboardSequenceNumber() };
    if before_input_sequence != clipboard_sequence_before {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "system clipboard changed before contention input ({clipboard_sequence_before} -> {before_input_sequence})"
            ),
        ));
    }

    let (failure_foreground, _failure_started_qpc) =
        inject_copy_trigger(overlay.handle, plan.copy, context.copy_trigger)?;
    record_step(
        report,
        &context.report_path,
        "clipboard_contention_failure_trigger",
        failure_foreground,
        None,
    )?;
    let failure_state = wait_for_capture_state(context, "clipboard contention failure", |state| {
        state.session_state == "selecting"
            && state.selection == Some(selection)
            && state.overlay_count == 1
            && !state.selection_copy_active
            && !state.clipboard_write_active
            && !state.capture_teardown_pending
            && state.capture_preflight_ready
            && state.status.starts_with("Copy failed:")
    })?;
    let failure_clipboard_unchanged =
        unsafe { GetClipboardSequenceNumber() } == clipboard_sequence_before;
    if !failure_clipboard_unchanged {
        return Err(io::Error::other(
            "failed Copy changed the system clipboard before the retry",
        ));
    }
    let failure_selection_preserved = failure_state.selection == Some(selection);
    if !failure_selection_preserved {
        return Err(io::Error::other(
            "clipboard contention failure discarded the editable selection",
        ));
    }
    // Let GPUI paint the settled failure message before taking the user-visible evidence shot.
    thread::sleep(context.settle_delay);
    let failure = capture_evidence(context, "13-clipboard-contention-failed.png", overlay)?;
    record_step(
        report,
        &context.report_path,
        "clipboard_contention_failure_reported",
        guard_foreground(overlay.handle)?,
        Some(&failure),
    )?;

    holder.release_and_wait(context.timeout)?;
    let holder_released_before_retry = holder.stopped && holder.released_path.is_file();
    if !holder_released_before_retry {
        return Err(io::Error::other(
            "clipboard contention holder did not confirm release before retry",
        ));
    }

    let (retry_foreground, retry_started_qpc) =
        inject_copy_trigger(overlay.handle, plan.copy, context.copy_trigger)?;
    record_step(
        report,
        &context.report_path,
        "clipboard_contention_retry_trigger",
        retry_foreground,
        None,
    )?;
    let consumer_result =
        consumer.wait_result_with_probe(context.timeout, |remaining| match query_capture_state(
            context,
            remaining.min(Duration::from_millis(100)),
        ) {
            Ok(state)
                if state.status.starts_with("Copy failed:") && !state.selection_copy_active =>
            {
                Err(io::Error::other(state.status))
            }
            Ok(_) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::TimedOut && !remaining.is_zero() => Ok(()),
            Err(error) => Err(error),
        })?;
    let png_path = PathBuf::from(&consumer_result.png_path);
    let dib_path = PathBuf::from(&consumer_result.dib_path);
    let consumer_image_path = PathBuf::from(&consumer_result.consumer_image_path);
    for path in [&png_path, &dib_path, &consumer_image_path] {
        ensure_path_within(path, &context.session_root)?;
    }
    ensure_path_within(&consumer.result_path, &context.session_root)?;
    let copied = CaptureFrame::open_png(&consumer_image_path)?;
    let png = fs::read(&png_path)?;
    let dib = fs::read(&dib_path)?;
    if consumer_result.previous_sequence != clipboard_sequence_before
        || consumer_result.png_bytes != png.len()
        || consumer_result.dib_bytes != dib.len()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "clipboard contention retry result does not match its launch sequence or artifact sizes",
        ));
    }
    let clipboard_sequence_after = unsafe { GetClipboardSequenceNumber() };
    if clipboard_sequence_after != consumer_result.observed_sequence {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "system clipboard changed after contention retry consumer read ({} -> {clipboard_sequence_after})",
                consumer_result.observed_sequence
            ),
        ));
    }
    let retry_state =
        wait_for_capture_state(context, "clipboard contention retry completion", |state| {
            selection_copy_completed_in_editor(state, selection, context.locale)
        })?;
    if retry_state.selection != Some(selection) {
        return Err(io::Error::other(
            "clipboard contention retry no longer reports its editable selection",
        ));
    }
    validate_frame_dimensions(&copied, selection, "clipboard contention retry image")?;
    let consumer_image_content =
        validate_same_pixel_content(&source, &copied, "clipboard contention retry image")?;
    let decoded_png = CaptureFrame::open_png(&png_path)?;
    validate_frame_dimensions(&decoded_png, selection, "clipboard contention retry PNG")?;
    let png_content =
        validate_same_pixel_content(&source, &decoded_png, "clipboard contention retry PNG")?;
    let decoded_dib = decode_clipboard_dib(&dib)?;
    validate_frame_dimensions(&decoded_dib, selection, "clipboard contention retry CF_DIB")?;
    let dib_content =
        validate_same_pixel_content(&source, &decoded_dib, "clipboard contention retry CF_DIB")?;
    let input_to_consumer_readable_ms =
        qpc_elapsed_ms(retry_started_qpc, consumer_result.consumer_read_qpc_ticks)?;
    if !consumer_ready_before_input
        || !consumer_observing_before_input
        || !consumer.stopped
        || clipboard_sequence_after == clipboard_sequence_before
    {
        return Err(io::Error::other(
            "clipboard contention retry consumer was not ready, did not observe a sequence change, or was not reaped",
        ));
    }
    let final_sequence = unsafe { GetClipboardSequenceNumber() };
    if final_sequence != clipboard_sequence_after {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "system clipboard changed during contention retry cleanup ({clipboard_sequence_after} -> {final_sequence})"
            ),
        ));
    }
    let editor_retained_after_retry =
        selection_copy_completed_in_editor(&retry_state, selection, context.locale);
    let retry = capture_evidence(
        context,
        "14-clipboard-contention-retry-complete.png",
        overlay,
    )?;
    record_step(
        report,
        &context.report_path,
        "clipboard_contention_retry_complete",
        guard_foreground(overlay.handle)?,
        Some(&retry),
    )?;

    let cleanup_foreground = focus_and_inject_key(overlay, VK_ESCAPE, context.timeout)?;
    wait_for_window_gone(
        overlay.handle,
        context.timeout,
        "clipboard contention Escape cleanup",
    )?;
    let cleanup_state =
        wait_for_capture_state(context, "clipboard contention Escape cleanup", |state| {
            state.session_state == "idle"
                && state.selection.is_none()
                && state.overlay_count == 0
                && state.pinned_count == 0
                && !state.capture_teardown_pending
                && state.background_tasks_idle
                && state.capture_preflight_ready
        })?;
    record_step(
        report,
        &context.report_path,
        "clipboard_contention_retry_cleanup",
        cleanup_foreground,
        None,
    )?;
    ensure_capture_input_released()?;
    let visible_process_windows = process_windows()?.len();
    if visible_process_windows != 0 {
        return Err(io::Error::other(format!(
            "clipboard contention retry cleanup left {visible_process_windows} visible process window(s)"
        )));
    }
    let relative = |path: &Path| {
        path.strip_prefix(&context.session_root)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned()
    };
    report.clipboard_contention_retry = Some(ClipboardContentionRetryReport {
        trigger: context.copy_trigger.label(),
        action: match context.copy_trigger {
            CopyTriggerOption::Toolbar => "toolbar_click",
            CopyTriggerOption::Enter => "enter_key",
        },
        requested_selection,
        selection,
        holder_process_id: holder.process_id,
        holder_held_before_input,
        clipboard_sequence_before,
        failure_status: failure_state.status,
        failure_selection_preserved,
        failure_clipboard_unchanged,
        holder_released_before_retry,
        clipboard_sequence_after,
        clipboard_sequence_changed: clipboard_sequence_after != clipboard_sequence_before,
        copied_bounds: copied.bounds,
        width: copied.width,
        height: copied.height,
        png_path: relative(&png_path),
        dib_path: relative(&dib_path),
        consumer_image_path: relative(&consumer_image_path),
        png_bytes: png.len(),
        dib_bytes: dib.len(),
        png_content,
        dib_content,
        consumer_image_content,
        timing_clock: "windows_qpc",
        timing_boundary: "retry_button_down_batch_to_consumer_decoded_image",
        input_to_consumer_readable_ms,
        consumer_result_path: relative(&consumer.result_path),
        consumer_ready_before_input,
        consumer_observing_before_input,
        consumer_cleaned_up: consumer.stopped,
        editor_retained_after_retry,
        cleanup_after_escape: cleanup_state.session_state == "idle"
            && cleanup_state.selection.is_none()
            && cleanup_state.overlay_count == 0
            && !cleanup_state.capture_teardown_pending
            && cleanup_state.capture_preflight_ready,
        cleanup: CleanupReport {
            session_state: cleanup_state.session_state,
            overlay_count: cleanup_state.overlay_count,
            pinned_count: cleanup_state.pinned_count,
            capture_teardown_pending: cleanup_state.capture_teardown_pending,
            visible_process_windows,
            capture_preflight_ready: cleanup_state.capture_preflight_ready,
        },
    });
    write_report(&context.report_path, report)
}

#[cfg(windows)]
/// Waits until Copy has reached the injected sink checkpoint before delivering competing Escape.
fn wait_for_copy_race_checkpoint(
    context: &WorkerContext,
    copy_race: &OverlayInteractionCopyRace,
    selection: PhysicalRect,
    timeout: Duration,
) -> io::Result<bool> {
    let copying_status = context.locale.text(UiText::SelectionCopyInProgress);
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            copy_race.release();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Copy cancellation race did not reach the commit checkpoint before Escape",
            ));
        }
        let state = match query_capture_state(context, remaining.min(Duration::from_millis(100))) {
            Ok(state) => state,
            Err(error) if error.kind() == io::ErrorKind::TimedOut => continue,
            Err(error) => {
                copy_race.release();
                return Err(error);
            }
        };
        if copy_race.checkpoint_reached()
            && state.session_state == "selecting"
            && state.selection == Some(selection)
            && state.overlay_count == 1
            && state.selection_copy_active
            && state.status == copying_status
        {
            return Ok(true);
        }
        thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(windows)]
/// Waits until Escape has reached the cancellation state before releasing a blocked sink worker.
fn wait_for_copy_race_release(
    context: &WorkerContext,
    copy_race: &OverlayInteractionCopyRace,
    selection: PhysicalRect,
    timeout: Duration,
) -> io::Result<bool> {
    let cancelling_status = context.locale.text(UiText::SelectionCopyCancelling);
    let waiting_status = context.locale.text(UiText::SelectionCopyWaitingForCommit);
    let cancelled_status = context
        .locale
        .text(UiText::CopyCancelledBeforeClipboardChanged);
    let deadline = Instant::now() + timeout;
    let mut last_state = String::from("no capture state observed");
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            copy_race.release();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "Copy cancellation race did not reach a safe release point; last_state={last_state}"
                ),
            ));
        }
        let state = match query_capture_state(context, remaining.min(Duration::from_millis(100))) {
            Ok(state) => state,
            Err(error) if error.kind() == io::ErrorKind::TimedOut => continue,
            Err(error) => {
                copy_race.release();
                return Err(error);
            }
        };
        last_state = format!(
            "session={} selection={:?} status={:?} selection_copy_active={} clipboard_write_active={} checkpoint_reached={}",
            state.session_state,
            state.selection,
            state.status,
            state.selection_copy_active,
            state.clipboard_write_active,
            copy_race.checkpoint_reached(),
        );
        let cancellation_requested = state.selection == Some(selection)
            && [cancelling_status, waiting_status, cancelled_status]
                .into_iter()
                .any(|status| state.status == status);
        if copy_race.checkpoint_reached() && cancellation_requested {
            copy_race.release();
            return Ok(true);
        }
        let worker_finished = state.selection == Some(selection)
            && !state.selection_copy_active
            && !state.clipboard_write_active
            && state.status == cancelled_status;
        if worker_finished {
            copy_race.release();
            return Ok(false);
        }
        thread::sleep(Duration::from_millis(5));
    }
}
