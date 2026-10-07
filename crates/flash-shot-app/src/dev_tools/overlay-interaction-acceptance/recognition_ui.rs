//! Real-input OCR and QR workflows for the Windows Release acceptance runner.

use super::*;

const OCR_FIXTURE_FILE: &str = "recognition-ocr-fixture.png";
const QR_FIXTURE_FILE: &str = "recognition-qr-fixture.png";

/// Opens isolated image fixtures through the production history reader, then exercises real More actions.
pub(super) fn execute(context: &WorkerContext, report: &mut AcceptanceReport) -> io::Result<()> {
    if !context.use_system_clipboard {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "recognition UI acceptance requires the explicit system-clipboard opt-in",
        ));
    }
    let ocr_path = save_fixture(context, OCR_FIXTURE_FILE, &ocr_acceptance_frame()?)?;
    let qr_path = save_fixture(context, QR_FIXTURE_FILE, &qr_acceptance_frame()?)?;
    let controller = wait_for_controller(context.timeout)?;
    focus_owned_window(controller, context.timeout)?;
    report.controller_window = Some(controller.report());
    record_step(
        report,
        &context.report_path,
        "recognition_controller_ready",
        controller,
        None,
    )?;

    request_history_open(context, ocr_path.clone())?;
    let ocr_ready = wait_for_editor_state(context, "recognition OCR editor")?;
    let mut editor =
        wait_for_finished_image_overlay(controller.handle, ptr::null_mut(), context.timeout)?;
    focus_owned_window(editor, context.timeout)?;
    let ocr_source = capture_evidence(context, "00-recognition-ocr-source.png", editor)?;
    record_step(
        report,
        &context.report_path,
        "recognition_ocr_source",
        editor,
        Some(&ocr_source),
    )?;
    let ocr_selection = ocr_ready
        .selection
        .ok_or_else(|| io::Error::other("OCR acceptance editor has no committed selection"))?;

    click_action(
        context,
        report,
        editor,
        WorkspaceMoreAction::Ocr,
        "recognition_ocr",
        "01-recognition-ocr-more.png",
    )?;
    let ocr_failure = wait_for_capture_state(context, "recognition OCR retry state", |state| {
        !state.recognition_in_flight
            && state.recognition_retry.as_deref() == Some("ocr")
            && state.selection == Some(ocr_selection)
            && state.overlay_count == 1
    })?;
    if ocr_failure.status != context.locale.text(UiText::RecognitionOcrUnavailable) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "injected OCR startup failure did not expose the localized retry status",
        ));
    }
    capture_step(
        context,
        report,
        editor,
        "02-recognition-ocr-failed.png",
        "recognition_ocr_failure",
    )?;

    click_action(
        context,
        report,
        editor,
        WorkspaceMoreAction::RetryRecognition,
        "recognition_ocr_retry",
        "03-recognition-ocr-retry-menu.png",
    )?;
    let ocr_result = wait_for_capture_state(context, "recognition OCR retry result", |state| {
        !state.recognition_in_flight
            && state.recognition_retry.is_none()
            && state.recognition_result_length.is_some()
            && state.overlay_count == 1
    })?;
    let ocr_result_length = ocr_result
        .recognition_result_length
        .ok_or_else(|| io::Error::other("OCR retry produced no result text"))?;
    if ocr_result_length == 0
        || ocr_result.status != context.locale.text(UiText::RecognitionTextCompleted)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "OCR retry did not produce localized nonempty text",
        ));
    }
    capture_step(
        context,
        report,
        editor,
        "04-recognition-ocr-result.png",
        "recognition_ocr_result",
    )?;

    let ocr_clipboard_before = unsafe { GetClipboardSequenceNumber() };
    click_action(
        context,
        report,
        editor,
        WorkspaceMoreAction::CopyRecognition,
        "recognition_ocr_copy",
        "05-recognition-ocr-copy-menu.png",
    )?;
    let ocr_copy_status = context.locale.format_template(
        UiText::RecognitionResultCopied,
        &[("title", context.locale.text(UiText::RecognitionTextTitle))],
    );
    let ocr_copied = wait_for_capture_state(context, "recognition OCR copy", |state| {
        state.status == ocr_copy_status && !state.clipboard_write_active
    })?;
    let ocr_clipboard_after = unsafe { GetClipboardSequenceNumber() };
    let ocr_clipboard = read_system_clipboard_text()?;
    let ocr_clipboard_matches = ocr_clipboard_after != ocr_clipboard_before
        && normalize_text(&ocr_clipboard).contains(&normalize_text(OCR_FIXTURE_TEXT));
    if !ocr_clipboard_matches || ocr_copied.selection != Some(ocr_selection) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "OCR Copy text did not match its fixture or changed selection (clipboard sequence {} -> {}, result length {})",
                ocr_clipboard_before, ocr_clipboard_after, ocr_result_length
            ),
        ));
    }
    click_action(
        context,
        report,
        editor,
        WorkspaceMoreAction::ClearRecognition,
        "recognition_ocr_clear",
        "06-recognition-ocr-clear-menu.png",
    )?;
    wait_for_capture_state(context, "recognition OCR result clear", |state| {
        state.recognition_result_length.is_none()
            && state.recognition_retry.is_none()
            && !state.recognition_in_flight
            && state.selection == Some(ocr_selection)
    })?;
    let ocr_cancelled = cancel_editor(context, report, editor, "recognition_ocr_cancel")?;

    request_history_open(context, qr_path.clone())?;
    let qr_ready = wait_for_editor_state(context, "recognition QR editor")?;
    editor = wait_for_finished_image_overlay(controller.handle, ptr::null_mut(), context.timeout)?;
    focus_owned_window(editor, context.timeout)?;
    let qr_source = capture_evidence(context, "07-recognition-qr-source.png", editor)?;
    record_step(
        report,
        &context.report_path,
        "recognition_qr_source",
        editor,
        Some(&qr_source),
    )?;
    let qr_selection = qr_ready
        .selection
        .ok_or_else(|| io::Error::other("QR acceptance editor has no committed selection"))?;
    click_action(
        context,
        report,
        editor,
        WorkspaceMoreAction::Qr,
        "recognition_qr",
        "08-recognition-qr-more.png",
    )?;
    let qr_result = wait_for_capture_state(context, "recognition QR result", |state| {
        !state.recognition_in_flight
            && state.recognition_retry.is_none()
            && state.recognition_result_length.is_some()
            && state.overlay_count == 1
    })?;
    let qr_result_length = qr_result
        .recognition_result_length
        .ok_or_else(|| io::Error::other("QR recognition produced no result"))?;
    if qr_result_length != QR_FIXTURE_TEXT.chars().count()
        || qr_result.status
            != context
                .locale
                .format_template(UiText::RecognitionQrFound, &[("count", "1")])
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "QR recognition did not report the one expected fixture code",
        ));
    }
    capture_step(
        context,
        report,
        editor,
        "09-recognition-qr-result.png",
        "recognition_qr_result",
    )?;

    let qr_clipboard_before = unsafe { GetClipboardSequenceNumber() };
    click_action(
        context,
        report,
        editor,
        WorkspaceMoreAction::CopyRecognition,
        "recognition_qr_copy",
        "10-recognition-qr-copy-menu.png",
    )?;
    let qr_copy_status = context.locale.format_template(
        UiText::RecognitionResultCopied,
        &[("title", context.locale.text(UiText::RecognitionQrCode))],
    );
    let qr_copied = wait_for_capture_state(context, "recognition QR text copy", |state| {
        state.status == qr_copy_status && !state.clipboard_write_active
    })?;
    let qr_clipboard_after = unsafe { GetClipboardSequenceNumber() };
    let qr_clipboard = read_system_clipboard_text()?;
    let qr_clipboard_matches = qr_clipboard_after != qr_clipboard_before
        && qr_clipboard.trim() == QR_FIXTURE_TEXT
        && qr_copied.selection == Some(qr_selection);
    if !qr_clipboard_matches {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "QR Copy text did not match its fixture or changed selection (clipboard sequence {} -> {})",
                qr_clipboard_before, qr_clipboard_after
            ),
        ));
    }
    let qr_cancelled = cancel_editor(context, report, editor, "recognition_qr_cancel")?;
    unsafe { ShowWindow(controller.handle, SW_HIDE) };
    wait_for_window_gone(
        controller.handle,
        context.timeout,
        "recognition controller hide",
    )?;
    let visible_process_windows = process_windows()?.len();
    if visible_process_windows != 0 {
        return Err(io::Error::other(format!(
            "recognition UI acceptance left {visible_process_windows} visible process window(s)"
        )));
    }
    let cleanup = wait_for_capture_state(context, "recognition acceptance cleanup", |state| {
        state.session_state == "idle"
            && state.selection.is_none()
            && state.overlay_count == 0
            && state.recognition_result_length.is_none()
            && state.recognition_retry.is_none()
            && !state.recognition_in_flight
            && state.capture_preflight_ready
    })?;
    report.recognition = Some(RecognitionReport {
        ocr_fixture: relative(context, &ocr_path),
        qr_fixture: relative(context, &qr_path),
        ocr_failure_status: ocr_failure.status,
        ocr_failure_selection_preserved: ocr_failure.selection == Some(ocr_selection),
        ocr_retry_available: ocr_failure.recognition_retry.as_deref() == Some("ocr"),
        ocr_retry_succeeded: ocr_result_length > 0,
        ocr_result_length,
        ocr_clipboard_matches_fixture: ocr_clipboard_matches,
        ocr_cancelled,
        qr_result_length,
        qr_clipboard_matches_fixture: qr_clipboard_matches,
        qr_cancelled,
        cleanup: CleanupReport {
            session_state: cleanup.session_state,
            overlay_count: cleanup.overlay_count,
            pinned_count: cleanup.pinned_count,
            capture_teardown_pending: cleanup.capture_teardown_pending,
            visible_process_windows,
            capture_preflight_ready: cleanup.capture_preflight_ready,
        },
    });
    write_report(&context.report_path, report)
}

fn save_fixture(
    context: &WorkerContext,
    file_name: &str,
    frame: &CaptureFrame,
) -> io::Result<PathBuf> {
    let history_root = context.session_root.join("history");
    fs::create_dir_all(&history_root)?;
    let path = history_root.join(file_name);
    frame.save_png(&path)?;
    ensure_path_within(&path, &context.session_root)?;
    Ok(path)
}

fn request_history_open(context: &WorkerContext, path: PathBuf) -> io::Result<()> {
    let path = path.canonicalize()?;
    let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
    context
        .interaction_commands
        .send_blocking(OverlayInteractionAcceptanceCommand::OpenHistoryImage {
            path,
            reply: reply_tx,
        })
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "history-open command channel closed",
            )
        })?;
    reply_rx
        .recv_timeout(context.timeout)
        .map_err(|error| match error {
            std::sync::mpsc::RecvTimeoutError::Timeout => io::Error::new(
                io::ErrorKind::TimedOut,
                "history-open command reply timed out",
            ),
            std::sync::mpsc::RecvTimeoutError::Disconnected => io::Error::new(
                io::ErrorKind::BrokenPipe,
                "history-open command reply channel disconnected",
            ),
        })?
        .map_err(io::Error::other)
}

fn wait_for_editor_state(
    context: &WorkerContext,
    stage: &str,
) -> io::Result<OverlayInteractionCaptureState> {
    wait_for_capture_state(context, stage, |state| {
        state.session_state == "selecting"
            && state.selection.is_some()
            && state.overlay_count == 1
            && !state.recognition_in_flight
    })
}

fn click_action(
    context: &WorkerContext,
    report: &mut AcceptanceReport,
    editor: NativeWindow,
    action: WorkspaceMoreAction,
    action_step: &'static str,
    menu_screenshot: &str,
) -> io::Result<()> {
    let mut state = query_capture_state(context, context.timeout)?;
    if !state.more_actions_visible {
        let selection = state
            .selection
            .ok_or_else(|| io::Error::other("recognition editor lost its selection before More"))?;
        let point = scroll_toolbar_more_point_for_capture_selection(
            editor.handle,
            selection,
            selection,
            state.annotation_controls_visible,
        )?;
        let foreground =
            inject_settled_mouse_click(editor.handle, point, Duration::from_millis(75))?;
        state = wait_for_capture_state(context, "recognition More panel", |state| {
            state.session_state == "selecting"
                && state.more_actions_visible
                && state.overlay_count == 1
        })?;
        thread::sleep(context.settle_delay.min(Duration::from_millis(300)));
        let evidence = capture_evidence(context, menu_screenshot, editor)?;
        record_step(
            report,
            &context.report_path,
            "recognition_more_actions",
            foreground,
            Some(&evidence),
        )?;
    }
    let menu_client_bounds = state
        .secondary_menu_bounds
        .ok_or_else(|| io::Error::other("recognition More panel did not expose its bounds"))?;
    let client_bounds = client_bounds_for_window(editor.handle)?;
    let menu_bounds = translated_rect(menu_client_bounds, client_bounds.left, client_bounds.top)?;
    let point = more_menu_action_screen_point(
        menu_bounds,
        editor.dpi,
        action,
        state.recognition_result_length.is_some(),
        state.recognition_retry.is_some(),
    )?;
    let foreground = inject_settled_mouse_click(editor.handle, point, Duration::from_millis(75))?;
    record_step(report, &context.report_path, action_step, foreground, None)
}

fn capture_step(
    context: &WorkerContext,
    report: &mut AcceptanceReport,
    editor: NativeWindow,
    screenshot: &str,
    action: &'static str,
) -> io::Result<()> {
    focus_owned_window(editor, context.timeout)?;
    thread::sleep(context.settle_delay.min(Duration::from_millis(300)));
    let evidence = capture_evidence(context, screenshot, editor)?;
    record_step(
        report,
        &context.report_path,
        action,
        guard_foreground(editor.handle)?,
        Some(&evidence),
    )
}

fn cancel_editor(
    context: &WorkerContext,
    report: &mut AcceptanceReport,
    editor: NativeWindow,
    action: &'static str,
) -> io::Result<bool> {
    let mut state = query_capture_state(context, context.timeout)?;
    if state.more_actions_visible {
        let selection = state.selection.ok_or_else(|| {
            io::Error::other("recognition editor lost its selection before cancel")
        })?;
        let point = scroll_toolbar_more_point_for_capture_selection(
            editor.handle,
            selection,
            selection,
            state.annotation_controls_visible,
        )?;
        let foreground =
            inject_settled_mouse_click(editor.handle, point, Duration::from_millis(75))?;
        record_step(
            report,
            &context.report_path,
            "recognition_more_closed",
            foreground,
            None,
        )?;
        state = wait_for_capture_state(context, "recognition More close", |state| {
            !state.more_actions_visible
        })?;
    }
    if state.overlay_count > 0 {
        let foreground = inject_key(editor.handle, VK_ESCAPE)?;
        record_step(report, &context.report_path, action, foreground, None)?;
    }
    wait_for_window_gone(editor.handle, context.timeout, "recognition editor Escape")?;
    let state = wait_for_capture_state(context, "recognition editor cancel cleanup", |state| {
        state.session_state == "idle"
            && state.selection.is_none()
            && state.overlay_count == 0
            && state.recognition_result_length.is_none()
            && state.recognition_retry.is_none()
            && !state.recognition_in_flight
            && state.capture_preflight_ready
    })?;
    Ok(state.selection.is_none() && state.overlay_count == 0)
}

fn normalize_text(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_uppercase()
}

fn relative(context: &WorkerContext, path: &Path) -> String {
    path.strip_prefix(&context.session_root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}
