//! Windows input sequence and report collection for settings navigation acceptance.

use std::{
    fs, io,
    path::PathBuf,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use super::native::{
    CursorRestore, NativeWindow, WindowZOrderRestore, capture_step,
    click_library_cancel_single_remove, click_library_clear_search, click_library_clear_selection,
    click_library_confirm_single_remove, click_library_copy, click_library_filter_all,
    click_library_filter_selection, click_library_format, click_library_open, click_library_remove,
    click_library_retention, click_library_search, click_library_select_all_filtered,
    click_navigation_item, click_record_idle_toggle, click_record_support, click_record_toggle,
    click_system_language, click_system_theme, click_update_action, ensure_input_idle,
    focus_window, send_key, send_unicode_text, snapshot, visible_window, visible_window_except,
};
use super::{
    ActionStepReport, CleanupReport, INPUT_SETTLE_DELAY, LibraryOpenCopyReport,
    LibraryRetentionReport, LibrarySearchSelectionReport, LibrarySingleDeleteReport,
    PinAppearanceReport, RecordingSuccessReport, Report, StepReport, WindowBounds,
};
use flash_shot::{
    SettingsInteractionAcceptanceCommand, SettingsInteractionState,
    i18n::{Locale, UiText},
    platform::{capture::CaptureFrame, clipboard::SystemClipboard},
    theme::ThemeMode,
};

#[cfg(windows)]
pub(super) struct WorkerOptions {
    pub(super) output_dir: PathBuf,
    pub(super) width: i32,
    pub(super) height: i32,
    pub(super) timeout: Duration,
    pub(super) settle: Duration,
    pub(super) locale: Locale,
    pub(super) theme: ThemeMode,
    pub(super) exercise_app_update: bool,
    pub(super) exercise_record_support: bool,
    pub(super) exercise_record_start: bool,
    pub(super) exercise_record_success: bool,
    pub(super) exercise_library_format: bool,
    pub(super) exercise_library_search_selection: bool,
    pub(super) exercise_library_single_delete: bool,
    pub(super) exercise_library_open_copy: bool,
    pub(super) exercise_library_retention: bool,
    pub(super) library_remove_paths: Option<(PathBuf, PathBuf)>,
    pub(super) library_open_path: Option<PathBuf>,
    pub(super) library_retention_paths: Option<Vec<PathBuf>>,
    pub(super) exercise_pin_appearance: bool,
    pub(super) commands: async_channel::Sender<SettingsInteractionAcceptanceCommand>,
}

#[cfg(windows)]
/// Navigates the settings surface, records observed GPUI state, and restores desktop side effects.
pub(super) fn run_input_probe(options: WorkerOptions) -> Result<(), Box<dyn std::error::Error>> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_DOWN, VK_ESCAPE, VK_F4, VK_LBUTTON, VK_MENU, VK_RETURN, VK_RIGHT,
        VK_SPACE,
    };

    let deadline = Instant::now() + options.timeout;
    let window = loop {
        if Instant::now() >= deadline {
            return Err(
                io::Error::new(io::ErrorKind::TimedOut, "settings window did not appear").into(),
            );
        }
        if let Some(window) = visible_window()? {
            break window;
        }
        thread::sleep(Duration::from_millis(50));
    };
    let mut window_z_order = WindowZOrderRestore {
        window,
        restored: false,
    };
    ensure_input_idle()?;
    let cursor_restore = CursorRestore::capture()?;
    focus_window(window)?;
    thread::sleep(options.settle);
    let compact = options.width < 700;
    let mut click_steps = Vec::new();
    for (index, expected) in ["capture", "library", "record", "app"]
        .into_iter()
        .enumerate()
    {
        focus_window(window)?;
        click_navigation_item(window, compact, index)?;
        thread::sleep(INPUT_SETTLE_DELAY);
        let observed = snapshot(&options.commands)?;
        let screenshot = capture_step(&window, &options.output_dir, &format!("click-{expected}"))?;
        let passed = observed.section == expected
            && observed.locale == options.locale.label()
            && observed.theme == options.theme.label();
        click_steps.push(StepReport {
            action: format!("mouse-click-{expected}"),
            expected_section: expected.to_owned(),
            observed_section: observed.section,
            observed_locale: observed.locale,
            observed_theme: observed.theme,
            passed,
            screenshot,
        });
    }
    let mut keyboard_steps = Vec::new();
    let key = if compact { VK_RIGHT } else { VK_DOWN };
    for expected in ["capture", "library", "record", "app", "capture"] {
        focus_window(window)?;
        send_key(window, key)?;
        thread::sleep(INPUT_SETTLE_DELAY);
        let observed = snapshot(&options.commands)?;
        let screenshot = capture_step(&window, &options.output_dir, &format!("key-{expected}"))?;
        let passed = observed.section == expected
            && observed.locale == options.locale.label()
            && observed.theme == options.theme.label();
        keyboard_steps.push(StepReport {
            action: format!("keyboard-{}", if compact { "right" } else { "down" }),
            expected_section: expected.to_owned(),
            observed_section: observed.section,
            observed_locale: observed.locale,
            observed_theme: observed.theme,
            passed,
            screenshot,
        });
    }
    for (key, name) in [(VK_RETURN, "enter"), (VK_SPACE, "space")] {
        focus_window(window)?;
        send_key(window, key)?;
        thread::sleep(INPUT_SETTLE_DELAY);
        let observed = snapshot(&options.commands)?;
        let screenshot = capture_step(&window, &options.output_dir, &format!("key-{name}"))?;
        let passed = observed.section == "capture"
            && observed.locale == options.locale.label()
            && observed.theme == options.theme.label();
        keyboard_steps.push(StepReport {
            action: format!("keyboard-{name}"),
            expected_section: "capture".to_owned(),
            observed_section: observed.section,
            observed_locale: observed.locale,
            observed_theme: observed.theme,
            passed,
            screenshot,
        });
    }
    let mut pin_appearance = None;
    if options.exercise_pin_appearance {
        // Keep three production Pins open while real App-page clicks change appearance. Each
        // snapshot crosses the GPUI command channel so the report proves the existing windows
        // updated, rather than only proving that the settings buttons changed their labels.
        focus_window(window)?;
        click_navigation_item(window, compact, 3)?;
        thread::sleep(INPUT_SETTLE_DELAY);
        let expected_initial = options.locale.label();
        let expected_theme = options.theme.label();
        let initial = wait_for_settings_state(&options.commands, options.timeout, |state| {
            state.section == "app"
                && state.pinned_window_count == 3
                && state.pinned_appearances.len() == 3
                && state
                    .pinned_appearances
                    .iter()
                    .all(|pin| pin.locale == expected_initial && pin.theme == expected_theme)
        })
        .map_err(|error| {
            io::Error::other(format!(
                "{error}; expected three Pins with initial {expected_initial}/{expected_theme}"
            ))
        })?;
        thread::sleep(options.settle);
        let before_screenshot = capture_step(&window, &options.output_dir, "appearance-before")?;

        focus_window(window)?;
        click_system_theme(window, compact)?;
        let after_theme = wait_for_settings_state(&options.commands, options.timeout, |state| {
            state.section == "app"
                && state.locale == expected_initial
                && state.theme != expected_theme
                && !state.pinned_appearances.is_empty()
                && state
                    .pinned_appearances
                    .iter()
                    .all(|pin| pin.locale == state.locale && pin.theme == state.theme)
        })?;
        thread::sleep(options.settle);
        let after_theme_screenshot =
            capture_step(&window, &options.output_dir, "appearance-after-theme")?;

        focus_window(window)?;
        click_system_language(window, compact)?;
        let after_locale = wait_for_settings_state(&options.commands, options.timeout, |state| {
            state.section == "app"
                && state.locale != expected_initial
                && state.theme == after_theme.theme
                && !state.pinned_appearances.is_empty()
                && state
                    .pinned_appearances
                    .iter()
                    .all(|pin| pin.locale == state.locale && pin.theme == state.theme)
        })?;
        thread::sleep(options.settle);
        let after_locale_screenshot =
            capture_step(&window, &options.output_dir, "appearance-after-locale")?;
        let final_locale = after_locale.locale.clone();
        let final_theme = after_locale.theme.clone();
        let all_pins_updated = after_locale
            .pinned_appearances
            .iter()
            .all(|pin| pin.locale == final_locale && pin.theme == final_theme);
        pin_appearance = Some(PinAppearanceReport {
            initial_locale: initial.locale,
            initial_theme: initial.theme,
            after_theme_locale: after_theme.locale,
            after_theme: after_theme.theme,
            after_locale: final_locale,
            final_theme,
            pin_count: after_locale.pinned_appearances.len(),
            all_pins_updated,
            screenshots: vec![
                before_screenshot,
                after_theme_screenshot,
                after_locale_screenshot,
            ],
        });
    }
    let mut action_steps = Vec::new();
    if options.exercise_app_update {
        focus_window(window)?;
        click_navigation_item(window, compact, 3)?;
        thread::sleep(INPUT_SETTLE_DELAY);
        let before = snapshot(&options.commands)?;
        let before_screenshot = capture_step(&window, &options.output_dir, "action-app-before")?;
        let expected_status = options
            .locale
            .text(UiText::UpdateCheckInProgress)
            .to_owned();
        let before_passed = before.section == "app"
            && before.update_check_in_flight
            && before.status == expected_status;
        action_steps.push(ActionStepReport {
            action: "mouse-click-app-update-before".to_owned(),
            expected_section: "app".to_owned(),
            observed_section: before.section,
            expected_busy: true,
            observed_busy: before.update_check_in_flight,
            expected_status: expected_status.clone(),
            observed_status: before.status,
            passed: before_passed,
            screenshot: before_screenshot,
        });

        focus_window(window)?;
        click_update_action(window, compact)?;
        thread::sleep(INPUT_SETTLE_DELAY);
        let after = snapshot(&options.commands)?;
        let after_screenshot = capture_step(&window, &options.output_dir, "action-app-after")?;
        let cancelled_status = options.locale.text(UiText::UpdateCheckCancelled).to_owned();
        let after_passed = after.section == "app"
            && !after.update_check_in_flight
            && after.status == cancelled_status;
        action_steps.push(ActionStepReport {
            action: "mouse-click-app-update-cancel".to_owned(),
            expected_section: "app".to_owned(),
            observed_section: after.section,
            expected_busy: false,
            observed_busy: after.update_check_in_flight,
            expected_status: cancelled_status,
            observed_status: after.status,
            passed: after_passed,
            screenshot: after_screenshot,
        });
    }
    if options.exercise_record_support {
        focus_window(window)?;
        click_navigation_item(window, compact, 2)?;
        thread::sleep(INPUT_SETTLE_DELAY);
        let before = snapshot(&options.commands)?;
        let before_screenshot = capture_step(&window, &options.output_dir, "action-record-before")?;
        let expected_status = options
            .locale
            .text(UiText::RecordingSupportCheckInProgress)
            .to_owned();
        let before_passed = before.section == "record"
            && before.recording_support_check_in_flight
            && before.status == expected_status;
        action_steps.push(ActionStepReport {
            action: "mouse-click-record-support-before".to_owned(),
            expected_section: "record".to_owned(),
            observed_section: before.section,
            expected_busy: true,
            observed_busy: before.recording_support_check_in_flight,
            expected_status: expected_status.clone(),
            observed_status: before.status,
            passed: before_passed,
            screenshot: before_screenshot,
        });

        focus_window(window)?;
        click_record_support(window, compact)?;
        thread::sleep(INPUT_SETTLE_DELAY);
        let after = snapshot(&options.commands)?;
        let after_screenshot = capture_step(&window, &options.output_dir, "action-record-after")?;
        let cancelled_status = options
            .locale
            .text(UiText::RecordingSupportCheckCancelled)
            .to_owned();
        let after_passed = after.section == "record"
            && !after.recording_support_check_in_flight
            && after.status == cancelled_status;
        action_steps.push(ActionStepReport {
            action: "mouse-click-record-support-cancel".to_owned(),
            expected_section: "record".to_owned(),
            observed_section: after.section,
            expected_busy: false,
            observed_busy: after.recording_support_check_in_flight,
            expected_status: cancelled_status,
            observed_status: after.status,
            passed: after_passed,
            screenshot: after_screenshot,
        });
    }
    if options.exercise_record_start {
        focus_window(window)?;
        click_navigation_item(window, compact, 2)?;
        thread::sleep(INPUT_SETTLE_DELAY);
        let before = snapshot(&options.commands)?;
        let before_screenshot =
            capture_step(&window, &options.output_dir, "action-record-start-before")?;
        let expected_status = options
            .locale
            .text(UiText::RecordingPreparingDisplay)
            .to_owned();
        let before_passed = before.section == "record"
            && before.recording_start_in_flight
            && before.status == expected_status;
        action_steps.push(ActionStepReport {
            action: "mouse-click-record-start-before".to_owned(),
            expected_section: "record".to_owned(),
            observed_section: before.section,
            expected_busy: true,
            observed_busy: before.recording_start_in_flight,
            expected_status: expected_status.clone(),
            observed_status: before.status,
            passed: before_passed,
            screenshot: before_screenshot,
        });

        focus_window(window)?;
        click_record_toggle(window, compact)?;
        thread::sleep(INPUT_SETTLE_DELAY);
        let after = snapshot(&options.commands)?;
        let after_screenshot =
            capture_step(&window, &options.output_dir, "action-record-start-after")?;
        let cancelled_status = options
            .locale
            .text(UiText::RecordingStartupCancelled)
            .to_owned();
        let after_passed = after.section == "record"
            && !after.recording_start_in_flight
            && after.status == cancelled_status;
        action_steps.push(ActionStepReport {
            action: "mouse-click-record-start-cancel".to_owned(),
            expected_section: "record".to_owned(),
            observed_section: after.section,
            expected_busy: false,
            observed_busy: after.recording_start_in_flight,
            expected_status: cancelled_status,
            observed_status: after.status,
            passed: after_passed,
            screenshot: after_screenshot,
        });
    }
    let mut recording_success = None;
    if options.exercise_record_success {
        focus_window(window)?;
        click_navigation_item(window, compact, 2)?;
        thread::sleep(INPUT_SETTLE_DELAY);
        let before = snapshot(&options.commands)?;
        let before_screenshot =
            capture_step(&window, &options.output_dir, "action-record-success-before")?;
        let before_passed = before.section == "record"
            && !before.recording_active
            && !before.recording_start_in_flight
            && !before.recording_stopping;
        action_steps.push(ActionStepReport {
            action: "mouse-click-record-success-before".to_owned(),
            expected_section: "record".to_owned(),
            observed_section: before.section,
            expected_busy: false,
            observed_busy: before.recording_start_in_flight,
            expected_status: "recording idle".to_owned(),
            observed_status: before.status,
            passed: before_passed,
            screenshot: before_screenshot,
        });

        focus_window(window)?;
        click_record_idle_toggle(window, compact)?;
        let active = wait_for_settings_state(&options.commands, options.timeout, |state| {
            state.section == "record"
                && state.recording_active
                && !state.recording_start_in_flight
                && !state.recording_stopping
                && state.recording_target.as_deref() == Some("display")
        })?;
        let active_screenshot =
            capture_step(&window, &options.output_dir, "action-record-success-active")?;
        let active_passed = active.recording_active
            && !active.recording_start_in_flight
            && active.recording_target.as_deref() == Some("display");
        action_steps.push(ActionStepReport {
            action: "mouse-click-record-success-active".to_owned(),
            expected_section: "record".to_owned(),
            observed_section: active.section.clone(),
            expected_busy: true,
            observed_busy: active.recording_start_in_flight || active.recording_active,
            expected_status: "recording display".to_owned(),
            observed_status: active.status.clone(),
            passed: active_passed,
            screenshot: active_screenshot,
        });

        thread::sleep(Duration::from_millis(1_200));
        focus_window(window)?;
        click_record_idle_toggle(window, compact)?;
        let stopping = wait_for_settings_state(&options.commands, options.timeout, |state| {
            state.section == "record" && state.recording_stopping
        })?;
        let stopping_screenshot = capture_step(
            &window,
            &options.output_dir,
            "action-record-success-stopping",
        )?;
        action_steps.push(ActionStepReport {
            action: "mouse-click-record-success-stop".to_owned(),
            expected_section: "record".to_owned(),
            observed_section: stopping.section.clone(),
            expected_busy: true,
            observed_busy: stopping.recording_stopping,
            expected_status: "recording stopping".to_owned(),
            observed_status: stopping.status.clone(),
            passed: stopping.recording_stopping,
            screenshot: stopping_screenshot,
        });

        let saved = wait_for_settings_state(&options.commands, options.timeout, |state| {
            state.section == "record"
                && !state.recording_active
                && !state.recording_start_in_flight
                && !state.recording_stopping
                && recording_saved_status(options.locale, &state.status)
        })?;
        let saved_screenshot =
            capture_step(&window, &options.output_dir, "action-record-success-saved")?;
        let output =
            wait_for_single_recording(&options.output_dir.join("recordings"), options.timeout)?;
        action_steps.push(ActionStepReport {
            action: "mouse-click-record-success-saved".to_owned(),
            expected_section: "record".to_owned(),
            observed_section: saved.section,
            expected_busy: false,
            observed_busy: saved.recording_active || saved.recording_stopping,
            expected_status: "recording saved".to_owned(),
            observed_status: saved.status,
            passed: output.1 > 0,
            screenshot: saved_screenshot.clone(),
        });
        recording_success = Some(RecordingSuccessReport {
            target: "display".to_owned(),
            active_observed: active.recording_active,
            stopping_observed: stopping.recording_stopping,
            progress_frames: active.recording_progress_frames,
            output_path: output.0.to_string_lossy().into_owned(),
            output_exists: output.0.is_file(),
            output_bytes: output.1,
            screenshots: vec![
                "action-record-success-before.png".to_owned(),
                "action-record-success-active.png".to_owned(),
                "action-record-success-stopping.png".to_owned(),
                "action-record-success-saved.png".to_owned(),
            ],
        });
    }
    if options.exercise_library_format {
        focus_window(window)?;
        click_navigation_item(window, compact, 1)?;
        thread::sleep(INPUT_SETTLE_DELAY);
        let before = snapshot(&options.commands)?;
        let before_screenshot =
            capture_step(&window, &options.output_dir, "action-library-before")?;
        let before_passed = before.section == "library"
            && before.export_format == "PNG"
            && before.status
                == options
                    .locale
                    .ready_with_shortcut("Ctrl+Shift+Print Screen");
        action_steps.push(ActionStepReport {
            action: "mouse-click-library-format-before".to_owned(),
            expected_section: "library".to_owned(),
            observed_section: before.section,
            expected_busy: false,
            observed_busy: false,
            expected_status: options
                .locale
                .ready_with_shortcut("Ctrl+Shift+Print Screen"),
            observed_status: before.status,
            passed: before_passed,
            screenshot: before_screenshot,
        });

        focus_window(window)?;
        click_library_format(window, compact)?;
        thread::sleep(INPUT_SETTLE_DELAY);
        let after = snapshot(&options.commands)?;
        let after_screenshot = capture_step(&window, &options.output_dir, "action-library-after")?;
        let expected_status = options
            .locale
            .format_template(UiText::ExportFormatChanged, &[("format", "JPEG")]);
        let after_passed = after.section == "library"
            && after.export_format == "JPEG"
            && after.status == expected_status;
        action_steps.push(ActionStepReport {
            action: "mouse-click-library-format-cycle".to_owned(),
            expected_section: "library".to_owned(),
            observed_section: after.section,
            expected_busy: false,
            observed_busy: false,
            expected_status,
            observed_status: after.status,
            passed: after_passed,
            screenshot: after_screenshot,
        });
    }
    let library_search_selection = if options.exercise_library_search_selection {
        Some(exercise_library_search_selection(
            &options, window, compact,
        )?)
    } else {
        None
    };
    let library_single_delete = if options.exercise_library_single_delete {
        Some(exercise_library_single_delete(&options, window, compact)?)
    } else {
        None
    };
    let library_open_copy = if options.exercise_library_open_copy {
        Some(exercise_library_open_copy(&options, window, compact)?)
    } else {
        None
    };
    let library_retention = if options.exercise_library_retention {
        Some(exercise_library_retention(&options, window, compact)?)
    } else {
        None
    };
    let cursor_restored = cursor_restore.restore()?;
    let input_released = [
        VK_ESCAPE, VK_LBUTTON, VK_DOWN, VK_F4, VK_MENU, VK_RETURN, VK_RIGHT, VK_SPACE,
    ]
    .into_iter()
    .all(|key| unsafe { GetAsyncKeyState(key as i32) >= 0 });
    let window_demoted = window_z_order.restore();
    let report = Report {
        schema: if options.exercise_pin_appearance {
            4
        } else if options.exercise_app_update
            || options.exercise_record_support
            || options.exercise_record_start
            || options.exercise_record_success
            || options.exercise_library_format
            || options.exercise_library_search_selection
            || options.exercise_library_single_delete
            || options.exercise_library_open_copy
            || options.exercise_library_retention
        {
            if options.exercise_library_retention {
                8
            } else if options.exercise_library_open_copy {
                7
            } else if options.exercise_library_search_selection {
                6
            } else if options.exercise_library_single_delete {
                5
            } else if options.exercise_record_success {
                3
            } else {
                2
            }
        } else {
            1
        },
        status: if click_steps
            .iter()
            .chain(keyboard_steps.iter())
            .all(|step| step.passed)
            && action_steps.iter().all(|step| step.passed)
            && pin_appearance
                .as_ref()
                .is_none_or(|report| report.pin_count == 3 && report.all_pins_updated)
            && library_single_delete.as_ref().is_none_or(|report| {
                report.single_confirmation_observed
                    && report.cancel_preserved_entries
                    && report.confirmation_reopened
                    && report.target_removed
                    && report.neighbor_preserved
                    && report.remaining_entries == 1
            })
            && library_search_selection.as_ref().is_none_or(|report| {
                report.initial_entries == 3
                    && report.selection_filter_entries == 2
                    && report.query_entries == 1
                    && report.selected_after_query == 1
                    && report.entries_after_clearing_query == 2
                    && report.selection_preserved_after_clearing_query
                    && report.selection_cleared
                    && report.all_filter_entries == 3
            })
            && library_open_copy.as_ref().is_none_or(|report| {
                report.initial_entries == 1
                    && report.copy_status_observed
                    && report.clipboard_width == 96
                    && report.clipboard_height == 64
                    && report.clipboard_pixels_match
                    && report.open_result_observed
                    && report.editor_window_observed
                    && report.selection_matches_fixture
                    && report.cancel_returned_to_idle
                    && report.editor_window_closed
                    && report.history_file_preserved
            })
            && library_retention.as_ref().is_none_or(|report| {
                report.initial_entries == 12
                    && report.initial_limit == 30
                    && report.applied_limits == [100, 300, 10]
                    && report.entry_counts_after_updates == [12, 12, 10]
                    && report.oldest_entries_removed
                    && report.newest_entries_preserved
                    && report.final_limit == 10
                    && report.final_entries == 10
            })
            && cursor_restored
            && input_released
            && window_demoted
        {
            "passed"
        } else {
            "failed"
        },
        input_authorized: true,
        locale: options.locale.label(),
        theme: options.theme.label(),
        window_width: options.width,
        window_height: options.height,
        window_bounds: WindowBounds {
            left: window.left,
            top: window.top,
            right: window.right,
            bottom: window.bottom,
        },
        dpi: window.dpi,
        scale_factor: window.dpi as f32 / 96.0,
        click_steps,
        keyboard_steps,
        action_steps,
        recording_success,
        library_search_selection,
        library_single_delete,
        library_open_copy,
        library_retention,
        pin_appearance,
        cleanup: CleanupReport {
            cursor_restored,
            input_released,
            window_demoted,
            process_exit_requested: true,
        },
    };
    let report_path = options.output_dir.join("report.json");
    fs::write(report_path, serde_json::to_vec_pretty(&report)?)?;
    let (quit_tx, quit_rx) = mpsc::sync_channel(1);
    options
        .commands
        .send_blocking(SettingsInteractionAcceptanceCommand::Quit(quit_tx))?;
    quit_rx.recv_timeout(Duration::from_secs(2))?;
    if report.status == "passed" {
        Ok(())
    } else {
        Err(io::Error::other("one or more settings input steps failed").into())
    }
}

#[cfg(windows)]
/// Exercises source filtering, filename search, selection, and the matching clear recovery action.
fn exercise_library_search_selection(
    options: &WorkerOptions,
    window: NativeWindow,
    compact: bool,
) -> io::Result<LibrarySearchSelectionReport> {
    focus_window(window)?;
    click_navigation_item(window, compact, 1)?;
    let initial = wait_for_settings_state(&options.commands, options.timeout, |state| {
        state.section == "library"
            && state.history_entry_count == 3
            && state.history_filtered_entry_count == 3
            && state.history_filter == "all"
            && state.history_search_query.is_empty()
    })?;
    thread::sleep(options.settle);
    let mut screenshots = vec![capture_step(
        &window,
        &options.output_dir,
        "library-search-before",
    )?];

    focus_window(window)?;
    click_library_filter_selection(window)?;
    let filtered = wait_for_settings_state(&options.commands, options.timeout, |state| {
        state.section == "library"
            && state.history_filter == "selections"
            && state.history_filtered_entry_count == 2
    })?;
    thread::sleep(options.settle);
    screenshots.push(capture_step(
        &window,
        &options.output_dir,
        "library-search-source-filter",
    )?);

    focus_window(window)?;
    click_library_search(window)?;
    wait_for_settings_state(&options.commands, options.timeout, |state| {
        state.section == "library" && state.history_search_active
    })?;
    thread::sleep(INPUT_SETTLE_DELAY);
    send_unicode_text(window, "needle")?;
    thread::sleep(INPUT_SETTLE_DELAY);
    screenshots.push(capture_step(
        &window,
        &options.output_dir,
        "library-search-input",
    )?);
    let searched = wait_for_settings_state(&options.commands, options.timeout, |state| {
        state.section == "library"
            && state.history_filter == "selections"
            && state.history_search_query == "needle"
            && state.history_filtered_entry_count == 1
    })?;
    thread::sleep(options.settle);
    screenshots.push(capture_step(
        &window,
        &options.output_dir,
        "library-search-query",
    )?);

    focus_window(window)?;
    click_library_select_all_filtered(window)?;
    let selected = wait_for_settings_state(&options.commands, options.timeout, |state| {
        state.section == "library"
            && state.history_selected_count == 1
            && state.history_search_query == "needle"
            && state.history_filtered_entry_count == 1
    })?;
    thread::sleep(options.settle);
    screenshots.push(capture_step(
        &window,
        &options.output_dir,
        "library-search-selected",
    )?);

    focus_window(window)?;
    click_library_clear_search(window)?;
    let search_cleared = wait_for_settings_state(&options.commands, options.timeout, |state| {
        state.section == "library"
            && state.history_filter == "selections"
            && state.history_search_query.is_empty()
            && state.history_filtered_entry_count == 2
            && state.history_selected_count == 1
    })?;
    thread::sleep(options.settle);
    screenshots.push(capture_step(
        &window,
        &options.output_dir,
        "library-search-cleared",
    )?);

    focus_window(window)?;
    click_library_clear_selection(window)?;
    let selection_cleared = wait_for_settings_state(&options.commands, options.timeout, |state| {
        state.section == "library"
            && state.history_selected_count == 0
            && state.status == options.locale.text(UiText::HistorySelectionCleared)
    })?;
    thread::sleep(options.settle);
    screenshots.push(capture_step(
        &window,
        &options.output_dir,
        "library-search-selection-cleared",
    )?);

    focus_window(window)?;
    click_library_filter_all(window)?;
    let all_filter = wait_for_settings_state(&options.commands, options.timeout, |state| {
        state.section == "library"
            && state.history_filter == "all"
            && state.history_filtered_entry_count == 3
    })?;
    thread::sleep(options.settle);
    screenshots.push(capture_step(
        &window,
        &options.output_dir,
        "library-search-all-restored",
    )?);

    Ok(LibrarySearchSelectionReport {
        initial_entries: initial.history_entry_count,
        selection_filter_entries: filtered.history_filtered_entry_count,
        query_entries: searched.history_filtered_entry_count,
        selected_after_query: selected.history_selected_count,
        entries_after_clearing_query: search_cleared.history_filtered_entry_count,
        selection_preserved_after_clearing_query: search_cleared.history_selected_count == 1,
        selection_cleared: selection_cleared.history_selected_count == 0,
        all_filter_entries: all_filter.history_filtered_entry_count,
        screenshots,
    })
}

#[cfg(windows)]
/// Uses real mouse input to cancel and then confirm one row-level Library removal.
fn exercise_library_single_delete(
    options: &WorkerOptions,
    window: NativeWindow,
    compact: bool,
) -> io::Result<LibrarySingleDeleteReport> {
    let (target, neighbor) = options
        .library_remove_paths
        .as_ref()
        .ok_or_else(|| io::Error::other("single-delete fixture paths were not provided"))?;
    focus_window(window)?;
    click_navigation_item(window, compact, 1)?;
    let initial = wait_for_settings_state(&options.commands, options.timeout, |state| {
        state.section == "library"
            && state.history_entry_count == 2
            && !state.history_clear_confirmation
    })?;
    thread::sleep(options.settle);
    let initial_screenshot = capture_step(&window, &options.output_dir, "library-delete-before")?;

    focus_window(window)?;
    click_library_remove(window, compact)?;
    let confirmation = wait_for_settings_state(&options.commands, options.timeout, |state| {
        state.section == "library"
            && state.history_entry_count == 2
            && state.history_clear_confirmation
            && state.history_clear_scope == "single"
    })?;
    thread::sleep(options.settle);
    let confirmation_screenshot =
        capture_step(&window, &options.output_dir, "library-delete-confirm")?;
    let single_confirmation_observed = confirmation.history_clear_scope == "single";

    focus_window(window)?;
    click_library_cancel_single_remove(window, compact)?;
    let cancelled = wait_for_settings_state(&options.commands, options.timeout, |state| {
        state.section == "library"
            && state.history_entry_count == 2
            && !state.history_clear_confirmation
            && state.status == options.locale.text(UiText::HistoryRemoveCancelled)
    })?;
    thread::sleep(options.settle);
    let cancelled_screenshot =
        capture_step(&window, &options.output_dir, "library-delete-cancelled")?;
    let cancel_preserved_entries =
        cancelled.history_entry_count == 2 && target.is_file() && neighbor.is_file();

    focus_window(window)?;
    click_library_remove(window, compact)?;
    let reopened = wait_for_settings_state(&options.commands, options.timeout, |state| {
        state.section == "library"
            && state.history_entry_count == 2
            && state.history_clear_confirmation
            && state.history_clear_scope == "single"
    })?;
    let confirmation_reopened =
        reopened.history_clear_confirmation && reopened.history_clear_scope == "single";
    thread::sleep(options.settle);
    let reopened_screenshot =
        capture_step(&window, &options.output_dir, "library-delete-confirm-again")?;

    focus_window(window)?;
    click_library_confirm_single_remove(window, compact)?;
    let removed = wait_for_settings_state(&options.commands, options.timeout, |state| {
        state.section == "library"
            && state.history_entry_count == 1
            && !state.history_clear_confirmation
    })?;
    thread::sleep(options.settle);
    let removed_screenshot = capture_step(&window, &options.output_dir, "library-delete-removed")?;

    Ok(LibrarySingleDeleteReport {
        initial_entries: initial.history_entry_count,
        single_confirmation_observed,
        cancel_preserved_entries,
        confirmation_reopened,
        target_removed: !target.exists(),
        neighbor_preserved: neighbor.is_file(),
        remaining_entries: removed.history_entry_count,
        screenshots: vec![
            initial_screenshot,
            confirmation_screenshot,
            cancelled_screenshot,
            reopened_screenshot,
            removed_screenshot,
        ],
    })
}

#[cfg(windows)]
/// Copies a retained image, opens it in the production editor, and verifies Escape recovery.
fn exercise_library_open_copy(
    options: &WorkerOptions,
    window: NativeWindow,
    compact: bool,
) -> io::Result<LibraryOpenCopyReport> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;

    let path = options
        .library_open_path
        .as_ref()
        .ok_or_else(|| io::Error::other("Library open/copy fixture path was not provided"))?;
    let original_bytes = fs::read(path)?;
    let expected_frame = CaptureFrame::open_png(path)?;
    focus_window(window)?;
    click_navigation_item(window, compact, 1)?;
    let initial = wait_for_settings_state(&options.commands, options.timeout, |state| {
        state.section == "library"
            && state.history_entry_count == 1
            && state.history_filtered_entry_count == 1
            && state.capture_session_state == "Idle"
    })?;
    thread::sleep(options.settle);
    let mut screenshots = vec![capture_step(
        &window,
        &options.output_dir,
        "library-open-copy-before",
    )?];

    focus_window(window)?;
    click_library_copy(window)?;
    let copied = wait_for_settings_state(&options.commands, options.timeout, |state| {
        state.section == "library"
            && state.history_entry_count == 1
            && state.capture_session_state == "Idle"
            && state.status == options.locale.text(UiText::HistoryCopiedToClipboard)
    })?;
    thread::sleep(options.settle);
    screenshots.push(capture_step(
        &window,
        &options.output_dir,
        "library-open-copy-copied",
    )?);
    let clipboard_frame = SystemClipboard.read_image()?;
    let clipboard_pixels_match = clipboard_frame.width == expected_frame.width
        && clipboard_frame.height == expected_frame.height
        && clipboard_frame.stride == expected_frame.stride
        && clipboard_frame.pixels.as_ref() == expected_frame.pixels.as_ref();

    focus_window(window)?;
    click_library_open(window)?;
    let opened = wait_for_settings_state(&options.commands, options.timeout, |state| {
        state.section == "library"
            && state.history_entry_count == 1
            && state.capture_session_state == "Selecting"
            && state.capture_selection_bounds
                == Some((
                    expected_frame.bounds.left,
                    expected_frame.bounds.top,
                    expected_frame.bounds.right,
                    expected_frame.bounds.bottom,
                ))
            && state.overlay_window_count == 1
    })?;
    let editor_window = wait_for_window_except(window, options.timeout)?;
    thread::sleep(options.settle);
    screenshots.push(capture_step(
        &editor_window,
        &options.output_dir,
        "library-open-editor",
    )?);

    send_key(editor_window, VK_ESCAPE)?;
    let cancelled = wait_for_settings_state(&options.commands, options.timeout, |state| {
        state.history_entry_count == 1
            && state.capture_session_state == "Idle"
            && state.overlay_window_count == 0
    })?;
    let editor_window_closed = wait_for_no_window_except(window, options.timeout)?;
    let history_file_preserved = path.is_file() && fs::read(path)? == original_bytes;

    Ok(LibraryOpenCopyReport {
        initial_entries: initial.history_entry_count,
        copy_status_observed: copied.status
            == options.locale.text(UiText::HistoryCopiedToClipboard),
        clipboard_width: clipboard_frame.width,
        clipboard_height: clipboard_frame.height,
        clipboard_pixels_match,
        open_result_observed: opened.capture_session_state == "Selecting"
            && opened.overlay_window_count == 1,
        editor_window_observed: editor_window.handle != window.handle,
        selection_matches_fixture: opened.capture_selection_bounds
            == Some((
                expected_frame.bounds.left,
                expected_frame.bounds.top,
                expected_frame.bounds.right,
                expected_frame.bounds.bottom,
            )),
        cancel_returned_to_idle: cancelled.capture_session_state == "Idle"
            && cancelled.overlay_window_count == 0,
        editor_window_closed,
        history_file_preserved,
        screenshots,
    })
}

#[cfg(windows)]
/// Changes retention through the real Library control and verifies oldest-file pruning.
fn exercise_library_retention(
    options: &WorkerOptions,
    window: NativeWindow,
    compact: bool,
) -> io::Result<LibraryRetentionReport> {
    let paths = options
        .library_retention_paths
        .as_ref()
        .ok_or_else(|| io::Error::other("Library retention fixture paths were not provided"))?;
    if paths.len() != 12 {
        return Err(io::Error::other(format!(
            "Library retention requires 12 fixture images, received {}",
            paths.len()
        )));
    }

    focus_window(window)?;
    click_navigation_item(window, compact, 1)?;
    let initial = wait_for_settings_state(&options.commands, options.timeout, |state| {
        state.section == "library"
            && state.history_entry_count == 12
            && state.history_limit == 30
            && state.history_retention_target.is_none()
    })?;
    thread::sleep(options.settle);
    let mut screenshots = vec![capture_step(
        &window,
        &options.output_dir,
        "library-retention-before",
    )?];
    let mut applied_limits = Vec::with_capacity(3);
    let mut entry_counts_after_updates = Vec::with_capacity(3);

    for target in [100_u16, 300, 10] {
        focus_window(window)?;
        click_library_retention(window, options.locale)?;
        let expected_entries = if target == 10 { 10 } else { 12 };
        let expected_status = options.locale.format_template(
            UiText::HistoryRetentionUpdated,
            &[("count", &target.to_string())],
        );
        let updated = wait_for_settings_state(&options.commands, options.timeout, |state| {
            state.section == "library"
                && state.history_limit == target
                && state.history_retention_target.is_none()
                && state.history_entry_count == expected_entries
                && state.status == expected_status
        })?;
        thread::sleep(options.settle);
        screenshots.push(capture_step(
            &window,
            &options.output_dir,
            &format!("library-retention-{target}"),
        )?);
        applied_limits.push(updated.history_limit);
        entry_counts_after_updates.push(updated.history_entry_count);
    }

    let oldest_entries_removed = paths.iter().take(2).all(|path| !path.exists());
    let newest_entries_preserved = paths.iter().skip(2).all(|path| path.is_file());
    let final_state =
        snapshot(&options.commands).map_err(|error| io::Error::other(error.to_string()))?;
    Ok(LibraryRetentionReport {
        initial_entries: initial.history_entry_count,
        initial_limit: initial.history_limit,
        applied_limits,
        entry_counts_after_updates,
        oldest_entries_removed,
        newest_entries_preserved,
        final_limit: final_state.history_limit,
        final_entries: final_state.history_entry_count,
        screenshots,
    })
}

#[cfg(windows)]
/// Polls the production settings snapshot until the requested lifecycle predicate becomes true.
fn wait_for_settings_state(
    commands: &async_channel::Sender<SettingsInteractionAcceptanceCommand>,
    timeout: Duration,
    predicate: impl Fn(&SettingsInteractionState) -> bool,
) -> io::Result<SettingsInteractionState> {
    let deadline = Instant::now() + timeout;
    loop {
        let state = snapshot(commands).map_err(|error| io::Error::other(error.to_string()))?;
        if predicate(&state) {
            return Ok(state);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "settings state did not reach the expected lifecycle: {} (section={}, entries={}, filtered_entries={}, selected={}, filter={}, query={:?}, search_active={}, history_limit={}, retention_target={:?}, clear_confirmation={}, clear_scope={}, session={}, selection={:?}, overlays={}, pinned_window_count={}, pinned_appearances={})",
                    state.status,
                    state.section,
                    state.history_entry_count,
                    state.history_filtered_entry_count,
                    state.history_selected_count,
                    state.history_filter,
                    state.history_search_query,
                    state.history_search_active,
                    state.history_limit,
                    state.history_retention_target,
                    state.history_clear_confirmation,
                    state.history_clear_scope,
                    state.capture_session_state,
                    state.capture_selection_bounds,
                    state.overlay_window_count,
                    state.pinned_window_count,
                    state.pinned_appearances.len()
                ),
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(windows)]
/// Waits for one visible production editor window while excluding the hidden settings surface.
fn wait_for_window_except(
    settings_window: NativeWindow,
    timeout: Duration,
) -> io::Result<NativeWindow> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(window) = visible_window_except(settings_window, 420, 350)? {
            return Ok(window);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Library Open did not create a visible editor window",
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(windows)]
/// Confirms that Escape closed the editor and left no other visible large window in this process.
fn wait_for_no_window_except(settings_window: NativeWindow, timeout: Duration) -> io::Result<bool> {
    let deadline = Instant::now() + timeout;
    loop {
        if visible_window_except(settings_window, 420, 350)?.is_none() {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(windows)]
/// Recognizes the localized production status emitted after FFmpeg publishes an MP4.
fn recording_saved_status(locale: Locale, status: &str) -> bool {
    let prefix = locale.text(UiText::RecordingSaved).replace("{path}", "");
    status.starts_with(prefix.trim_end())
}

#[cfg(windows)]
/// Waits for one non-empty MP4 in the isolated recording directory and returns its size.
fn wait_for_single_recording(
    directory: &std::path::Path,
    timeout: Duration,
) -> io::Result<(PathBuf, u64)> {
    let deadline = Instant::now() + timeout;
    loop {
        let mut paths = fs::read_dir(directory)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .filter(|path| {
                        path.extension()
                            .and_then(|extension| extension.to_str())
                            .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        paths.sort();
        if paths.len() == 1 {
            let path = paths.pop().expect("one recording path was checked");
            if let Ok(metadata) = fs::metadata(&path)
                && metadata.len() > 0
            {
                return Ok((path, metadata.len()));
            }
        } else if paths.len() > 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("recording directory contains {} MP4 files", paths.len()),
            ));
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "recording MP4 did not become readable in {}",
                    directory.display()
                ),
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}
