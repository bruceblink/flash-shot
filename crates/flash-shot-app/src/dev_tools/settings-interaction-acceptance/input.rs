//! Windows input sequence and report collection for settings navigation acceptance.

use std::{
    fs, io,
    path::PathBuf,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use super::native::{
    CursorRestore, WindowZOrderRestore, capture_step, click_navigation_item, click_update_action,
    ensure_input_idle, focus_window, send_key, snapshot, visible_window,
};
use super::{
    ActionStepReport, CleanupReport, INPUT_SETTLE_DELAY, Report, StepReport, WindowBounds,
};
use flash_shot::{
    SettingsInteractionAcceptanceCommand,
    i18n::{Locale, UiText},
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
    pub(super) commands: async_channel::Sender<SettingsInteractionAcceptanceCommand>,
}

#[cfg(windows)]
/// Navigates the settings surface, records observed GPUI state, and restores desktop side effects.
pub(super) fn run_input_probe(options: WorkerOptions) -> Result<(), Box<dyn std::error::Error>> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_DOWN, VK_F4, VK_LBUTTON, VK_MENU, VK_RETURN, VK_RIGHT, VK_SPACE,
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
    let cursor_restored = cursor_restore.restore()?;
    let input_released = [
        VK_LBUTTON, VK_DOWN, VK_F4, VK_MENU, VK_RETURN, VK_RIGHT, VK_SPACE,
    ]
    .into_iter()
    .all(|key| unsafe { GetAsyncKeyState(key as i32) >= 0 });
    let window_demoted = window_z_order.restore();
    let report = Report {
        schema: if options.exercise_app_update { 2 } else { 1 },
        status: if click_steps
            .iter()
            .chain(keyboard_steps.iter())
            .all(|step| step.passed)
            && action_steps.iter().all(|step| step.passed)
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
