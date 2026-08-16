//! Isolated, no-input acceptance for the native multi-Pin lifecycle.

#[cfg(windows)]
mod windows {
    use std::{
        ffi::c_void,
        fs, io,
        mem::size_of,
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };

    use gpui::{
        App, AppContext, AsyncApp, Entity, WindowBackgroundAppearance, WindowBounds, WindowHandle,
        WindowKind, WindowOptions, px, size,
    };
    use windows_sys::Win32::{
        Foundation::RECT,
        System::{
            ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
            Threading::{GetCurrentProcess, GetCurrentProcessId},
        },
        UI::WindowsAndMessaging::{
            GWL_EXSTYLE, GetForegroundWindow, GetLayeredWindowAttributes, GetWindowLongPtrW,
            GetWindowRect, GetWindowThreadProcessId, IsWindow, IsWindowVisible, LWA_ALPHA,
            SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos, WS_EX_LAYERED,
        },
    };

    use super::super::{
        FlashShotApp,
        pinned::{PinnedImage, native_window_handle},
    };
    use crate::{
        PinLifecycleAcceptanceOptions,
        domain::geometry::PhysicalRect,
        history::{HistorySource, ScreenshotHistory},
        performance::PerformanceRecorder,
        platform::{
            capture::{CaptureBackend, CaptureFrame, PixelFormat, SystemCaptureBackend},
            clipboard::ClipboardService,
        },
        settings::UserSettings,
    };

    const PIN_COUNT: usize = 3;
    const PIN_WIDTH: u32 = 360;
    const PIN_HEIGHT: u32 = 240;
    const PIN_GAP: i32 = 80;
    const PIN_LAYOUT_MARGIN: i32 = 50;
    const PIN_LAYOUT_TOP: i32 = 80;
    const PIN_LAYOUT_BOTTOM: i32 = 40;
    const PIN_SCREENSHOT_MARGIN: i32 = 20;

    #[derive(serde::Serialize)]
    struct PinLifecycleReport {
        schema_version: u32,
        test: &'static str,
        status: String,
        process_id: u32,
        isolated_profile: String,
        system_services_disabled: bool,
        display: DisplayReport,
        windows: Vec<PinWindowReport>,
        zoom: Option<GeometryChangeReport>,
        opacity: Option<OpacityReport>,
        copy: Option<CopyReport>,
        save: Option<SaveReport>,
        solo: Option<VisibilityReport>,
        show_all: Option<VisibilityReport>,
        show_all_preserved_focus: Option<bool>,
        registered_windows_after_solo: Option<usize>,
        registered_windows_after_show_all: Option<usize>,
        live_windows_after_close: Option<usize>,
        capture_preflight_ready: Option<bool>,
        soak: Option<PinSoakReport>,
        screenshots: Vec<String>,
        error: Option<String>,
    }

    #[derive(serde::Serialize)]
    struct DisplayReport {
        id: String,
        bounds: PhysicalRect,
        dpi_x: u32,
        dpi_y: u32,
        scale_factor: f32,
    }

    #[derive(serde::Serialize)]
    struct PinWindowReport {
        handle: usize,
        process_id: u32,
        original_bounds: PhysicalRect,
        arranged_bounds: PhysicalRect,
    }

    #[derive(serde::Serialize)]
    struct GeometryChangeReport {
        handle: usize,
        before: PhysicalRect,
        after: PhysicalRect,
    }

    #[derive(serde::Serialize)]
    struct OpacityReport {
        handle: usize,
        expected_alpha: u8,
        observed_alpha: u8,
    }

    #[derive(serde::Serialize)]
    struct CopyReport {
        calls: usize,
        complete_frame_equal: bool,
        background_worker_completed: bool,
        shared_lease_released: bool,
    }

    #[derive(serde::Serialize)]
    struct SaveReport {
        path: String,
        source: &'static str,
        file_exists: bool,
    }

    #[derive(serde::Serialize)]
    struct VisibilityReport {
        visible: Vec<bool>,
        visible_count: usize,
    }

    #[derive(serde::Serialize)]
    struct PinSoakReport {
        requested_duration_ms: u128,
        elapsed_ms: u128,
        cycles_completed: usize,
        registry_min: usize,
        registry_max: usize,
        frame_checks: usize,
        preflight_checks: usize,
        focus_preserved: bool,
        working_set_start_bytes: u64,
        working_set_end_bytes: u64,
        working_set_peak_bytes: u64,
        working_set_growth_bytes: i128,
        private_commit_start_bytes: u64,
        private_commit_end_bytes: u64,
        private_commit_peak_bytes: u64,
        private_commit_growth_bytes: i128,
        samples: Vec<PinSoakSampleReport>,
    }

    #[derive(serde::Serialize)]
    struct PinSoakSampleReport {
        cycle: usize,
        elapsed_ms: u128,
        invoking_pin_index: usize,
        registered_windows: usize,
        solo_visible_count: usize,
        show_all_visible_count: usize,
        source_frames_equal: bool,
        bounds_valid: bool,
        capture_preflight_ready: bool,
        focus_preserved: bool,
        working_set_bytes: u64,
        private_commit_bytes: u64,
        native_bounds: Vec<PhysicalRect>,
    }

    struct ProcessMemorySample {
        working_set_bytes: u64,
        private_commit_bytes: u64,
    }

    #[derive(Default)]
    struct RecordingClipboard {
        frames: Mutex<Vec<CaptureFrame>>,
    }

    impl RecordingClipboard {
        fn frames(&self) -> io::Result<Vec<CaptureFrame>> {
            self.frames
                .lock()
                .map(|frames| frames.clone())
                .map_err(|_| io::Error::other("acceptance clipboard lock was poisoned"))
        }
    }

    impl ClipboardService for RecordingClipboard {
        fn copy_image(&self, frame: &CaptureFrame) -> io::Result<()> {
            self.frames
                .lock()
                .map_err(|_| io::Error::other("acceptance clipboard lock was poisoned"))?
                .push(frame.clone());
            Ok(())
        }

        fn copy_text(&self, _text: &str) -> io::Result<()> {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "pin acceptance does not copy text",
            ))
        }
    }

    /// Opens one hidden controller and starts the isolated lifecycle after GPUI owns the app.
    pub(crate) fn open(
        performance: PerformanceRecorder,
        history: ScreenshotHistory,
        settings: UserSettings,
        settings_path: PathBuf,
        acceptance: PinLifecycleAcceptanceOptions,
        cx: &mut App,
    ) -> Result<(), Box<dyn std::error::Error>> {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(420.0), px(420.0)), cx)),
                show: false,
                focus: false,
                kind: WindowKind::PopUp,
                window_background: WindowBackgroundAppearance::Opaque,
                ..Default::default()
            },
            move |_, cx| {
                let app = cx.new(|cx| {
                    FlashShotApp::new_for_acceptance(
                        performance,
                        history,
                        settings,
                        settings_path,
                        cx,
                    )
                });
                let acceptance_app = app.clone();
                cx.defer(move |cx| {
                    acceptance_app.update(cx, |app, cx| {
                        app.start_pin_lifecycle_acceptance(acceptance, cx)
                    });
                });
                app
            },
        )?;
        Ok(())
    }

    impl FlashShotApp {
        /// Opens three production Pin windows, then runs the no-input checks on the GPUI task loop.
        fn start_pin_lifecycle_acceptance(
            &mut self,
            acceptance: PinLifecycleAcceptanceOptions,
            cx: &mut gpui::Context<Self>,
        ) {
            let watchdog_timeout = acceptance.timeout;
            let timeout_marker = acceptance.session_root.join("timeout.txt");
            // The watchdog bounds the complete native lifecycle, including GPUI scheduling and
            // desktop capture, rather than only the async Save operation.
            std::thread::spawn(move || {
                std::thread::sleep(watchdog_timeout);
                let _ = fs::write(
                    timeout_marker,
                    format!(
                        "Pin lifecycle acceptance exceeded {} ms",
                        watchdog_timeout.as_millis()
                    ),
                );
                std::process::exit(1);
            });

            let frames = acceptance_frames();
            for frame in frames.iter().cloned() {
                self.open_pinned_frame(
                    frame,
                    "Pin lifecycle acceptance window opened",
                    None,
                    false,
                    cx,
                );
            }

            cx.spawn(move |this: gpui::WeakEntity<Self>, cx: &mut AsyncApp| {
                let mut cx = cx.clone();
                async move {
                    let mut report = PinLifecycleReport::new(&acceptance);
                    let report_path = acceptance.session_root.join("report.json");
                    let _ = write_report(&report_path, &report);
                    let outcome = match this.upgrade() {
                        Some(app) => {
                            run_lifecycle(app, frames, &acceptance, &mut report, &mut cx).await
                        }
                        None => Err(io::Error::other(
                            "pin acceptance controller closed before the lifecycle started",
                        )),
                    };
                    match &outcome {
                        Ok(()) => report.status = "passed".to_owned(),
                        Err(error) => {
                            report.status = "failed".to_owned();
                            report.error = Some(error.to_string());
                        }
                    }
                    let report_result = write_report(&report_path, &report);
                    if let Err(error) = &report_result {
                        log::error!(target: "flash_shot::acceptance", "pin_report_write_failed error={error}");
                    }
                    let exit_code = if outcome.is_ok() && report_result.is_ok() {
                        0
                    } else {
                        1
                    };
                    std::process::exit(exit_code);
                }
            })
            .detach();
        }
    }

    impl PinLifecycleReport {
        fn new(acceptance: &PinLifecycleAcceptanceOptions) -> Self {
            Self {
                schema_version: 4,
                test: "pin_lifecycle_acceptance",
                status: "running".to_owned(),
                process_id: unsafe { GetCurrentProcessId() },
                isolated_profile: acceptance.session_root.to_string_lossy().into_owned(),
                system_services_disabled: false,
                display: DisplayReport {
                    id: acceptance.display.id.clone(),
                    bounds: acceptance.display.physical_bounds,
                    dpi_x: acceptance.display.dpi_x,
                    dpi_y: acceptance.display.dpi_y,
                    scale_factor: acceptance.display.scale_factor,
                },
                windows: Vec::new(),
                zoom: None,
                opacity: None,
                copy: None,
                save: None,
                solo: None,
                show_all: None,
                show_all_preserved_focus: None,
                registered_windows_after_solo: None,
                registered_windows_after_show_all: None,
                live_windows_after_close: None,
                capture_preflight_ready: None,
                soak: None,
                screenshots: Vec::new(),
                error: None,
            }
        }
    }

    /// Exercises window geometry, opacity, copy, save, visibility, and close in one bounded run.
    async fn run_lifecycle(
        app: Entity<FlashShotApp>,
        frames: Vec<CaptureFrame>,
        acceptance: &PinLifecycleAcceptanceOptions,
        report: &mut PinLifecycleReport,
        cx: &mut AsyncApp,
    ) -> io::Result<()> {
        let system_services_disabled =
            app.update(cx, |app, _| app.system_services_disabled_for_acceptance());
        report.system_services_disabled = system_services_disabled;
        if !system_services_disabled {
            return Err(io::Error::other(
                "acceptance app unexpectedly owns a tray or global shortcut service",
            ));
        }
        cx.background_executor()
            .timer(acceptance.settle_delay)
            .await;
        let handles = app.update(
            cx,
            |app, cx| -> io::Result<Vec<WindowHandle<PinnedImage>>> {
                app.prune_closed_pinned_windows(cx);
                if app.pinned_windows.len() != PIN_COUNT {
                    return Err(io::Error::other(format!(
                        "expected {PIN_COUNT} Pin windows, found {}",
                        app.pinned_windows.len()
                    )));
                }
                for handle in &app.pinned_windows {
                    handle
                        .update(cx, |pin, _, cx| pin.show_controls_for_acceptance(cx))
                        .map_err(|error| io::Error::other(error.to_string()))?;
                }
                Ok(app.pinned_windows.clone())
            },
        )?;

        let native_handles = native_handles(&handles, cx)?;
        let original_bounds = native_handles
            .iter()
            .map(|handle| owned_window_bounds(*handle))
            .collect::<io::Result<Vec<_>>>()?;
        arrange_windows(
            &native_handles,
            acceptance.display.physical_bounds,
            acceptance.display.scale_factor,
        )?;
        cx.background_executor()
            .timer(acceptance.settle_delay)
            .await;
        let arranged_bounds = native_handles
            .iter()
            .map(|handle| owned_window_bounds(*handle))
            .collect::<io::Result<Vec<_>>>()?;
        validate_window_layout(&arranged_bounds, acceptance.display.physical_bounds)?;
        report.windows = native_handles
            .iter()
            .enumerate()
            .map(|(index, handle)| PinWindowReport {
                handle: *handle as usize,
                process_id: unsafe { GetCurrentProcessId() },
                original_bounds: original_bounds[index],
                arranged_bounds: arranged_bounds[index],
            })
            .collect();

        let initial_path = acceptance.session_root.join("screenshots/pins-initial.png");
        capture_windows(
            &arranged_bounds,
            acceptance.display.physical_bounds,
            &initial_path,
            cx,
        )
        .await?;
        report
            .screenshots
            .push("screenshots/pins-initial.png".to_owned());

        let zoom_before = owned_window_bounds(native_handles[0])?;
        handles[0]
            .update(cx, |pin, window, cx| pin.zoom(1.25, window, cx))
            .map_err(|error| io::Error::other(error.to_string()))?;
        handles[1]
            .update(cx, |pin, window, cx| pin.cycle_opacity(window, cx))
            .map_err(|error| io::Error::other(error.to_string()))?;
        let clipboard = Arc::new(RecordingClipboard::default());
        let injected_clipboard: Arc<dyn ClipboardService + Send + Sync> = clipboard.clone();
        app.update(cx, |app, _| {
            app.image_clipboard = injected_clipboard;
        });
        handles[2]
            .update(cx, |pin, _, cx| {
                pin.copy_image(cx);
                // A rapid second command must reuse the visible in-flight state, not enqueue
                // another clipboard write behind the first one.
                pin.copy_image(cx);
            })
            .map_err(|error| io::Error::other(error.to_string()))?;
        let copied = wait_for_pinned_copy(
            &app,
            &handles[2],
            clipboard.as_ref(),
            acceptance.timeout,
            cx,
        )
        .await?;
        cx.background_executor()
            .timer(acceptance.settle_delay)
            .await;
        let post_zoom_bounds = native_handles
            .iter()
            .map(|handle| owned_window_bounds(*handle))
            .collect::<io::Result<Vec<_>>>()?;
        validate_window_layout(&post_zoom_bounds, acceptance.display.physical_bounds)?;
        let zoom_after = post_zoom_bounds[0];
        if zoom_after.width() <= zoom_before.width() || zoom_after.height() <= zoom_before.height()
        {
            return Err(io::Error::other(
                "Pin zoom did not enlarge the native window",
            ));
        }
        report.zoom = Some(GeometryChangeReport {
            handle: native_handles[0] as usize,
            before: zoom_before,
            after: zoom_after,
        });

        let observed_alpha = owned_window_alpha(native_handles[1])?;
        if observed_alpha != 191 {
            return Err(io::Error::other(format!(
                "Pin opacity was {observed_alpha}, expected 191"
            )));
        }
        report.opacity = Some(OpacityReport {
            handle: native_handles[1] as usize,
            expected_alpha: 191,
            observed_alpha,
        });

        let complete_frame_equal = copied.len() == 1 && frames_equal(&copied[0], &frames[2]);
        if !complete_frame_equal {
            return Err(io::Error::other(
                "in-memory Pin copy did not preserve the complete source frame",
            ));
        }
        report.copy = Some(CopyReport {
            calls: copied.len(),
            complete_frame_equal,
            background_worker_completed: true,
            shared_lease_released: true,
        });

        handles[0]
            .update(cx, |pin, _, cx| pin.save_image(cx))
            .map_err(|error| io::Error::other(error.to_string()))?;
        let saved = wait_for_pinned_save(&app, acceptance.timeout, cx).await?;
        let isolated_root = fs::canonicalize(&acceptance.session_root)?;
        let saved_path = fs::canonicalize(&saved.path)?;
        if !saved_path.starts_with(&isolated_root) || !saved_path.is_file() {
            return Err(io::Error::other(
                "Pin save escaped the isolated profile or did not create a file",
            ));
        }
        report.save = Some(SaveReport {
            path: saved_path.to_string_lossy().into_owned(),
            source: "pinned",
            file_exists: true,
        });

        handles[0]
            .update(cx, |pin, window, cx| {
                pin.hide_other_pinned_images(window, cx)
            })
            .map_err(|error| io::Error::other(error.to_string()))?;
        cx.background_executor()
            .timer(acceptance.settle_delay)
            .await;
        let solo_visibility = native_handles
            .iter()
            .map(|handle| owned_window_visible(*handle))
            .collect::<io::Result<Vec<_>>>()?;
        if solo_visibility != [true, false, false] {
            return Err(io::Error::other(format!(
                "Solo visibility was {solo_visibility:?}, expected [true, false, false]"
            )));
        }
        report.solo = Some(visibility_report(solo_visibility));
        let registered_after_solo = app.update(cx, |app, _| app.pinned_windows.len());
        if registered_after_solo != PIN_COUNT {
            return Err(io::Error::other(format!(
                "Solo left {registered_after_solo} registered Pins, expected {PIN_COUNT}"
            )));
        }
        report.registered_windows_after_solo = Some(registered_after_solo);

        let foreground_before_show_all = foreground_window_handle()?;

        handles[0]
            .update(cx, |pin, window, cx| pin.show_all_pinned_images(window, cx))
            .map_err(|error| io::Error::other(error.to_string()))?;
        cx.background_executor()
            .timer(acceptance.settle_delay)
            .await;
        let shown_visibility = native_handles
            .iter()
            .map(|handle| owned_window_visible(*handle))
            .collect::<io::Result<Vec<_>>>()?;
        if shown_visibility != [true, true, true] {
            return Err(io::Error::other(format!(
                "Show all visibility was {shown_visibility:?}, expected all Pins visible"
            )));
        }
        report.show_all = Some(visibility_report(shown_visibility));
        let foreground_after_show_all = foreground_window_handle()?;
        let show_all_preserved_focus = foreground_after_show_all == foreground_before_show_all;
        report.show_all_preserved_focus = Some(show_all_preserved_focus);
        if !show_all_preserved_focus {
            return Err(io::Error::other(format!(
                "Show all changed foreground HWND from {foreground_before_show_all} to {foreground_after_show_all}"
            )));
        }
        let registered_after_show_all = app.update(cx, |app, _| app.pinned_windows.len());
        if registered_after_show_all != PIN_COUNT {
            return Err(io::Error::other(format!(
                "Show all left {registered_after_show_all} registered Pins, expected {PIN_COUNT}"
            )));
        }
        report.registered_windows_after_show_all = Some(registered_after_show_all);

        if acceptance.soak_duration > Duration::ZERO {
            report.soak =
                Some(run_pin_soak(&app, &handles, &native_handles, &frames, acceptance, cx).await?);
            for handle in &handles {
                handle
                    .update(cx, |pin, _, cx| pin.show_controls_for_acceptance(cx))
                    .map_err(|error| io::Error::other(error.to_string()))?;
            }
            cx.background_executor()
                .timer(acceptance.settle_delay)
                .await;
            let soak_bounds = native_handles
                .iter()
                .map(|handle| owned_window_bounds(*handle))
                .collect::<io::Result<Vec<_>>>()?;
            validate_window_layout(&soak_bounds, acceptance.display.physical_bounds)?;
            let soak_path = acceptance
                .session_root
                .join("screenshots/pins-soak-final.png");
            capture_windows(
                &soak_bounds,
                acceptance.display.physical_bounds,
                &soak_path,
                cx,
            )
            .await?;
            report
                .screenshots
                .push("screenshots/pins-soak-final.png".to_owned());
        }

        handles[1]
            .update(cx, |pin, window, cx| pin.close(window, cx))
            .map_err(|error| io::Error::other(error.to_string()))?;
        cx.background_executor()
            .timer(acceptance.settle_delay)
            .await;
        let live_windows = app.update(cx, |app, _| app.pinned_windows.len());
        if live_windows != 2 {
            return Err(io::Error::other(format!(
                "closing one Pin left {live_windows} live windows, expected 2"
            )));
        }
        report.live_windows_after_close = Some(live_windows);
        let live_windows_after_prune = app.update(cx, |app, cx| {
            app.prune_closed_pinned_windows(cx);
            app.pinned_windows.len()
        });
        if live_windows_after_prune != 2 {
            return Err(io::Error::other(format!(
                "defensive Pin pruning left {live_windows_after_prune} windows, expected 2"
            )));
        }
        let capture_preflight_ready = app.update(cx, |app, _| app.capture_preflight_ready());
        if !capture_preflight_ready {
            return Err(io::Error::other(
                "capture preflight was blocked while two Pins remained open",
            ));
        }
        report.capture_preflight_ready = Some(true);

        for handle in [&handles[0], &handles[2]] {
            handle
                .update(cx, |pin, _, cx| pin.show_controls_for_acceptance(cx))
                .map_err(|error| io::Error::other(error.to_string()))?;
        }
        cx.background_executor()
            .timer(acceptance.settle_delay)
            .await;
        let final_bounds = [native_handles[0], native_handles[2]]
            .into_iter()
            .map(owned_window_bounds)
            .collect::<io::Result<Vec<_>>>()?;
        let final_path = acceptance.session_root.join("screenshots/pins-final.png");
        capture_windows(
            &final_bounds,
            acceptance.display.physical_bounds,
            &final_path,
            cx,
        )
        .await?;
        report
            .screenshots
            .push("screenshots/pins-final.png".to_owned());
        Ok(())
    }

    /// Repeats production Solo/Show all transitions while proving every Pin remains registered,
    /// content-stable, in bounds, focus-preserving, and compatible with a new Capture request.
    async fn run_pin_soak(
        app: &Entity<FlashShotApp>,
        handles: &[WindowHandle<PinnedImage>],
        native_handles: &[isize],
        frames: &[CaptureFrame],
        acceptance: &PinLifecycleAcceptanceOptions,
        cx: &mut AsyncApp,
    ) -> io::Result<PinSoakReport> {
        let started = Instant::now();
        let mut samples = Vec::new();
        let mut registry_min = PIN_COUNT;
        let mut registry_max = PIN_COUNT;
        let mut frame_checks = 0;
        let mut preflight_checks = 0;
        let baseline_memory = process_memory_sample()?;
        let mut last_memory = ProcessMemorySample {
            working_set_bytes: baseline_memory.working_set_bytes,
            private_commit_bytes: baseline_memory.private_commit_bytes,
        };
        let mut working_set_peak_bytes = baseline_memory.working_set_bytes;
        let mut private_commit_peak_bytes = baseline_memory.private_commit_bytes;

        while started.elapsed() < acceptance.soak_duration {
            let cycle = samples.len() + 1;
            let invoking_pin_index = (cycle - 1) % PIN_COUNT;
            handles[invoking_pin_index]
                .update(cx, |pin, window, cx| {
                    pin.hide_other_pinned_images(window, cx)
                })
                .map_err(|error| io::Error::other(error.to_string()))?;
            cx.background_executor()
                .timer(acceptance.settle_delay)
                .await;
            let solo_visibility = native_handles
                .iter()
                .map(|handle| owned_window_visible(*handle))
                .collect::<io::Result<Vec<_>>>()?;
            let solo_visible_count = solo_visibility.iter().filter(|visible| **visible).count();
            if solo_visible_count != 1 || !solo_visibility[invoking_pin_index] {
                return Err(io::Error::other(format!(
                    "Pin soak cycle {cycle} Solo visibility was {solo_visibility:?}"
                )));
            }
            let registered_after_solo = app.update(cx, |app, _| app.pinned_windows.len());
            if registered_after_solo != PIN_COUNT {
                return Err(io::Error::other(format!(
                    "Pin soak cycle {cycle} Solo left {registered_after_solo} registered windows"
                )));
            }
            let foreground_before = foreground_window_handle()?;

            handles[invoking_pin_index]
                .update(cx, |pin, window, cx| pin.show_all_pinned_images(window, cx))
                .map_err(|error| io::Error::other(error.to_string()))?;
            cx.background_executor()
                .timer(acceptance.settle_delay)
                .await;
            let shown_visibility = native_handles
                .iter()
                .map(|handle| owned_window_visible(*handle))
                .collect::<io::Result<Vec<_>>>()?;
            let show_all_visible_count =
                shown_visibility.iter().filter(|visible| **visible).count();
            if show_all_visible_count != PIN_COUNT {
                return Err(io::Error::other(format!(
                    "Pin soak cycle {cycle} Show all visibility was {shown_visibility:?}"
                )));
            }

            let registered_windows = app.update(cx, |app, _| app.pinned_windows.len());
            registry_min = registry_min.min(registered_windows);
            registry_max = registry_max.max(registered_windows);
            if registered_windows != PIN_COUNT {
                return Err(io::Error::other(format!(
                    "Pin soak cycle {cycle} left {registered_windows} registered windows"
                )));
            }
            let foreground_after = foreground_window_handle()?;
            let focus_preserved = foreground_after == foreground_before;
            if !focus_preserved {
                return Err(io::Error::other(format!(
                    "Pin soak cycle {cycle} changed foreground HWND from {foreground_before} to {foreground_after}"
                )));
            }

            let current_frames = handles
                .iter()
                .map(|handle| {
                    handle
                        .update(cx, |pin, _, _| pin.frame_for_acceptance())
                        .map_err(|error| io::Error::other(error.to_string()))
                })
                .collect::<io::Result<Vec<_>>>()?;
            let source_frames_equal = current_frames
                .iter()
                .zip(frames)
                .all(|(current, expected)| frames_equal(current, expected));
            if !source_frames_equal {
                return Err(io::Error::other(format!(
                    "Pin soak cycle {cycle} changed an immutable source frame"
                )));
            }
            frame_checks += current_frames.len();

            let native_bounds = native_handles
                .iter()
                .map(|handle| owned_window_bounds(*handle))
                .collect::<io::Result<Vec<_>>>()?;
            validate_window_layout(&native_bounds, acceptance.display.physical_bounds)?;
            let capture_preflight_ready = app.update(cx, |app, _| app.capture_preflight_ready());
            if !capture_preflight_ready {
                return Err(io::Error::other(format!(
                    "Pin soak cycle {cycle} blocked Capture preflight"
                )));
            }
            preflight_checks += 1;
            last_memory = process_memory_sample()?;
            working_set_peak_bytes = working_set_peak_bytes.max(last_memory.working_set_bytes);
            private_commit_peak_bytes =
                private_commit_peak_bytes.max(last_memory.private_commit_bytes);
            samples.push(PinSoakSampleReport {
                cycle,
                elapsed_ms: started.elapsed().as_millis(),
                invoking_pin_index,
                registered_windows,
                solo_visible_count,
                show_all_visible_count,
                source_frames_equal,
                bounds_valid: true,
                capture_preflight_ready,
                focus_preserved,
                working_set_bytes: last_memory.working_set_bytes,
                private_commit_bytes: last_memory.private_commit_bytes,
                native_bounds,
            });
        }

        if samples.is_empty() {
            return Err(io::Error::other(
                "Pin soak duration completed without a lifecycle sample",
            ));
        }
        Ok(PinSoakReport {
            requested_duration_ms: acceptance.soak_duration.as_millis(),
            elapsed_ms: started.elapsed().as_millis(),
            cycles_completed: samples.len(),
            registry_min,
            registry_max,
            frame_checks,
            preflight_checks,
            focus_preserved: samples.iter().all(|sample| sample.focus_preserved),
            working_set_start_bytes: baseline_memory.working_set_bytes,
            working_set_end_bytes: last_memory.working_set_bytes,
            working_set_peak_bytes,
            working_set_growth_bytes: i128::from(last_memory.working_set_bytes)
                - i128::from(baseline_memory.working_set_bytes),
            private_commit_start_bytes: baseline_memory.private_commit_bytes,
            private_commit_end_bytes: last_memory.private_commit_bytes,
            private_commit_peak_bytes,
            private_commit_growth_bytes: i128::from(last_memory.private_commit_bytes)
                - i128::from(baseline_memory.private_commit_bytes),
            samples,
        })
    }

    /// Reads process resource counters without imposing a machine-specific pass/fail threshold.
    fn process_memory_sample() -> io::Result<ProcessMemorySample> {
        let mut counters = PROCESS_MEMORY_COUNTERS {
            cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            ..Default::default()
        };
        if unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(ProcessMemorySample {
            working_set_bytes: counters.WorkingSetSize as u64,
            private_commit_bytes: counters.PagefileUsage as u64,
        })
    }

    struct SavedPin {
        path: PathBuf,
    }

    /// Waits for the production Pin worker and app-wide clipboard lease to finish together.
    async fn wait_for_pinned_copy(
        app: &Entity<FlashShotApp>,
        pin: &WindowHandle<PinnedImage>,
        clipboard: &RecordingClipboard,
        timeout: Duration,
        cx: &mut AsyncApp,
    ) -> io::Result<Vec<CaptureFrame>> {
        let deadline = Instant::now() + timeout;
        loop {
            let pin_in_flight = pin
                .update(cx, |pin, _, _| pin.copy_in_flight_for_acceptance())
                .map_err(|error| io::Error::other(error.to_string()))?;
            let lease_in_flight = app.update(cx, |app, _| app.clipboard_write_lease.is_some());
            let frames = clipboard.frames()?;
            if !pin_in_flight && !lease_in_flight {
                if frames.len() == 1 {
                    return Ok(frames);
                }
                return Err(io::Error::other(format!(
                    "Pin Copy finished with {} clipboard writes; expected exactly one",
                    frames.len()
                )));
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Pin Copy worker or shared clipboard lease did not finish before timeout",
                ));
            }
            cx.background_executor()
                .timer(Duration::from_millis(25))
                .await;
        }
    }

    /// Waits for the real async save worker and requires a Pinned history entry before continuing.
    async fn wait_for_pinned_save(
        app: &Entity<FlashShotApp>,
        timeout: Duration,
        cx: &mut AsyncApp,
    ) -> io::Result<SavedPin> {
        let deadline = Instant::now() + timeout;
        loop {
            let state = app.update(cx, |app, _| {
                let entry = app
                    .history
                    .entries()
                    .iter()
                    .find(|entry| entry.source == HistorySource::Pinned)
                    .map(|entry| entry.path.clone());
                (app.pinned_save_in_flight, entry)
            });
            if !state.0
                && let Some(path) = state.1
            {
                return Ok(SavedPin { path });
            }
            if !state.0 && state.1.is_none() {
                return Err(io::Error::other(
                    "Pin save finished without a Pinned history entry",
                ));
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Pin save did not finish before the acceptance timeout",
                ));
            }
            cx.background_executor()
                .timer(Duration::from_millis(25))
                .await;
        }
    }

    fn native_handles(
        windows: &[WindowHandle<PinnedImage>],
        cx: &mut AsyncApp,
    ) -> io::Result<Vec<isize>> {
        windows
            .iter()
            .map(|window| {
                window
                    .update(cx, |_, window, _| native_window_handle(window))
                    .map_err(|error| io::Error::other(error.to_string()))?
                    .ok_or_else(|| io::Error::other("Pin native window handle is unavailable"))
            })
            .collect()
    }

    /// Places measured native HWND extents in a DPI-scaled row without changing their size.
    fn arrange_windows(
        handles: &[isize],
        display: PhysicalRect,
        scale_factor: f32,
    ) -> io::Result<()> {
        let bounds = handles
            .iter()
            .map(|handle| owned_window_bounds(*handle))
            .collect::<io::Result<Vec<_>>>()?;
        let positions = pin_layout_positions(&bounds, display, scale_factor)?;
        for (handle, (left, top)) in handles.iter().zip(positions) {
            let window = owned_window(*handle)?;
            // SAFETY: window is process-owned and flags preserve size, focus, and z-order.
            if unsafe {
                SetWindowPos(
                    window,
                    std::ptr::null_mut(),
                    left,
                    top,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER,
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }

    /// Computes physical positions from measured outer sizes so high-DPI Pins cannot overlap.
    fn pin_layout_positions(
        windows: &[PhysicalRect],
        display: PhysicalRect,
        scale_factor: f32,
    ) -> io::Result<Vec<(i32, i32)>> {
        if windows.len() != PIN_COUNT {
            return Err(io::Error::other(format!(
                "expected {PIN_COUNT} Pin bounds, found {}",
                windows.len()
            )));
        }
        let display_width = i64::from(display.right) - i64::from(display.left);
        let display_height = i64::from(display.bottom) - i64::from(display.top);
        if display_width <= 0 || display_height <= 0 {
            return Err(io::Error::other("acceptance display bounds are empty"));
        }

        let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
            scale_factor
        } else {
            1.0
        };
        let scaled = |value: i32| i64::from(((value as f32 * scale).round() as i32).max(1));
        let gap = scaled(PIN_GAP);
        let side_margin = scaled(PIN_LAYOUT_MARGIN);
        let top_margin = scaled(PIN_LAYOUT_TOP);
        let bottom_margin = scaled(PIN_LAYOUT_BOTTOM);

        let mut widths = Vec::with_capacity(windows.len());
        let mut max_height = 0_i64;
        for window in windows {
            let width = i64::from(window.right) - i64::from(window.left);
            let height = i64::from(window.bottom) - i64::from(window.top);
            if width <= 0 || height <= 0 {
                return Err(io::Error::other("Pin native window bounds are empty"));
            }
            widths.push(width);
            max_height = max_height.max(height);
        }

        let required_width = widths.iter().sum::<i64>()
            + gap * (windows.len().saturating_sub(1) as i64)
            + side_margin * 2;
        let required_height = top_margin + max_height + bottom_margin;
        if display_width < required_width || display_height < required_height {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!(
                    "display must provide {required_width}x{required_height} physical pixels for three-Pin acceptance"
                ),
            ));
        }

        let top = i64::from(display.top) + top_margin;
        let mut left = i64::from(display.left) + side_margin;
        let mut positions = Vec::with_capacity(windows.len());
        for width in widths {
            positions.push((
                i32::try_from(left)
                    .map_err(|_| io::Error::other("Pin layout x-coordinate overflowed"))?,
                i32::try_from(top)
                    .map_err(|_| io::Error::other("Pin layout y-coordinate overflowed"))?,
            ));
            left += width + gap;
        }
        Ok(positions)
    }

    /// Rejects clipped or overlapping evidence before a screenshot can hide the layout defect.
    fn validate_window_layout(windows: &[PhysicalRect], display: PhysicalRect) -> io::Result<()> {
        for window in windows {
            if window.left < display.left
                || window.top < display.top
                || window.right > display.right
                || window.bottom > display.bottom
            {
                return Err(io::Error::other(
                    "Pin native window extends outside the acceptance display",
                ));
            }
        }
        for (index, left) in windows.iter().enumerate() {
            for right in windows.iter().skip(index + 1) {
                if left.left < right.right
                    && left.right > right.left
                    && left.top < right.bottom
                    && left.bottom > right.top
                {
                    return Err(io::Error::other("Pin native windows overlap"));
                }
            }
        }
        Ok(())
    }

    async fn capture_windows(
        windows: &[PhysicalRect],
        display: PhysicalRect,
        output: &Path,
        cx: &AsyncApp,
    ) -> io::Result<()> {
        let bounds = union_bounds(windows, display)?;
        let output = output.to_owned();
        cx.background_executor()
            .spawn(async move {
                let frame = SystemCaptureBackend.capture(bounds)?;
                frame.save_png(output)
            })
            .await
    }

    fn union_bounds(windows: &[PhysicalRect], display: PhysicalRect) -> io::Result<PhysicalRect> {
        let first = windows
            .first()
            .copied()
            .ok_or_else(|| io::Error::other("no Pin windows are available to capture"))?;
        let bounds = windows
            .iter()
            .skip(1)
            .fold(first, |bounds, window| PhysicalRect {
                left: bounds.left.min(window.left),
                top: bounds.top.min(window.top),
                right: bounds.right.max(window.right),
                bottom: bounds.bottom.max(window.bottom),
            });
        let clipped = PhysicalRect {
            left: bounds
                .left
                .saturating_sub(PIN_SCREENSHOT_MARGIN)
                .max(display.left),
            top: bounds
                .top
                .saturating_sub(PIN_SCREENSHOT_MARGIN)
                .max(display.top),
            right: bounds
                .right
                .saturating_add(PIN_SCREENSHOT_MARGIN)
                .min(display.right),
            bottom: bounds
                .bottom
                .saturating_add(PIN_SCREENSHOT_MARGIN)
                .min(display.bottom),
        };
        if clipped.right <= clipped.left || clipped.bottom <= clipped.top {
            return Err(io::Error::other("Pin screenshot bounds are empty"));
        }
        Ok(clipped)
    }

    fn visibility_report(visible: Vec<bool>) -> VisibilityReport {
        VisibilityReport {
            visible_count: visible.iter().filter(|visible| **visible).count(),
            visible,
        }
    }

    fn frames_equal(left: &CaptureFrame, right: &CaptureFrame) -> bool {
        left.bounds == right.bounds
            && left.width == right.width
            && left.height == right.height
            && left.stride == right.stride
            && left.format == right.format
            && left.pixels == right.pixels
    }

    fn owned_window(handle: isize) -> io::Result<*mut c_void> {
        let window = handle as *mut c_void;
        if unsafe { IsWindow(window) } == 0 {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "Pin native window is unavailable",
            ));
        }
        let mut process_id = 0;
        unsafe { GetWindowThreadProcessId(window, &mut process_id) };
        if process_id != unsafe { GetCurrentProcessId() } {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Pin native window belongs to another process",
            ));
        }
        Ok(window)
    }

    fn foreground_window_handle() -> io::Result<isize> {
        let window = unsafe { GetForegroundWindow() };
        if window.is_null() {
            return Err(io::Error::other("Windows reported no foreground HWND"));
        }
        Ok(window as isize)
    }

    fn owned_window_bounds(handle: isize) -> io::Result<PhysicalRect> {
        let window = owned_window(handle)?;
        let mut rect = RECT::default();
        if unsafe { GetWindowRect(window, &mut rect) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if rect.right <= rect.left || rect.bottom <= rect.top {
            return Err(io::Error::other("Pin native window bounds are empty"));
        }
        Ok(PhysicalRect {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        })
    }

    fn owned_window_visible(handle: isize) -> io::Result<bool> {
        let window = owned_window(handle)?;
        Ok(unsafe { IsWindowVisible(window) } != 0)
    }

    fn owned_window_alpha(handle: isize) -> io::Result<u8> {
        let window = owned_window(handle)?;
        let style = unsafe { GetWindowLongPtrW(window, GWL_EXSTYLE) };
        if style & WS_EX_LAYERED as isize == 0 {
            return Ok(255);
        }
        let mut color_key = 0;
        let mut alpha = 255;
        let mut flags = 0;
        if unsafe { GetLayeredWindowAttributes(window, &mut color_key, &mut alpha, &mut flags) }
            == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(if flags & LWA_ALPHA != 0 { alpha } else { 255 })
    }

    fn acceptance_frames() -> Vec<CaptureFrame> {
        (0..PIN_COUNT).map(acceptance_frame).collect()
    }

    /// Builds distinct patterned BGRA frames so screenshots prove three independent Pin surfaces.
    fn acceptance_frame(index: usize) -> CaptureFrame {
        let stride = PIN_WIDTH as usize * 4;
        let mut pixels = vec![0_u8; stride * PIN_HEIGHT as usize];
        let palettes = [
            ([42, 92, 140, 255], [75, 132, 184, 255]),
            ([70, 116, 72, 255], [104, 154, 105, 255]),
            ([116, 72, 126, 255], [158, 104, 168, 255]),
        ];
        let (dark, light) = palettes[index];
        for y in 0..PIN_HEIGHT as usize {
            for x in 0..PIN_WIDTH as usize {
                let color = if (x / 40 + y / 40 + index).is_multiple_of(2) {
                    light
                } else {
                    dark
                };
                let offset = y * stride + x * 4;
                pixels[offset..offset + 4].copy_from_slice(&color);
            }
        }
        CaptureFrame {
            bounds: PhysicalRect {
                left: 0,
                top: 0,
                right: PIN_WIDTH as i32,
                bottom: PIN_HEIGHT as i32,
            },
            width: PIN_WIDTH,
            height: PIN_HEIGHT,
            stride,
            format: PixelFormat::Bgra8,
            pixels: pixels.into(),
            capture_duration: Duration::ZERO,
            cpu_copy_count: 0,
        }
    }

    fn write_report(path: &Path, report: &PinLifecycleReport) -> io::Result<()> {
        let encoded = serde_json::to_vec_pretty(report).map_err(io::Error::other)?;
        fs::write(path, encoded)
    }

    #[cfg(test)]
    mod tests {
        use super::{
            PIN_COUNT, acceptance_frames, frames_equal, pin_layout_positions, union_bounds,
            validate_window_layout, visibility_report,
        };
        use crate::domain::geometry::PhysicalRect;

        #[test]
        fn acceptance_frames_are_distinct_and_valid() {
            let frames = acceptance_frames();
            assert_eq!(frames.len(), PIN_COUNT);
            for frame in &frames {
                assert!(frame.validate().is_ok());
            }
            assert!(!frames_equal(&frames[0], &frames[1]));
            assert!(!frames_equal(&frames[1], &frames[2]));
        }

        #[test]
        fn screenshot_union_adds_margin_and_clips_to_the_display() {
            let display = PhysicalRect {
                left: 0,
                top: 0,
                right: 1280,
                bottom: 720,
            };
            let windows = [
                PhysicalRect {
                    left: 10,
                    top: 30,
                    right: 370,
                    bottom: 270,
                },
                PhysicalRect {
                    left: 900,
                    top: 30,
                    right: 1260,
                    bottom: 270,
                },
            ];
            assert_eq!(
                union_bounds(&windows, display).unwrap(),
                PhysicalRect {
                    left: 0,
                    top: 10,
                    right: 1280,
                    bottom: 290,
                }
            );
        }

        #[test]
        fn visibility_report_counts_only_visible_windows() {
            let report = visibility_report(vec![true, false, true]);
            assert_eq!(report.visible_count, 2);
            assert_eq!(report.visible, [true, false, true]);
        }

        #[test]
        fn high_dpi_layout_uses_measured_native_extents_and_scaled_gaps() {
            let display = PhysicalRect {
                left: 0,
                top: 0,
                right: 3840,
                bottom: 2160,
            };
            let source = [
                PhysicalRect {
                    left: 0,
                    top: 0,
                    right: 720,
                    bottom: 480,
                },
                PhysicalRect {
                    left: 0,
                    top: 0,
                    right: 720,
                    bottom: 480,
                },
                PhysicalRect {
                    left: 0,
                    top: 0,
                    right: 720,
                    bottom: 480,
                },
            ];

            let positions = pin_layout_positions(&source, display, 2.0).unwrap();
            assert_eq!(positions, [(100, 160), (980, 160), (1860, 160)]);

            let arranged = [
                PhysicalRect {
                    left: 10,
                    top: 100,
                    right: 910,
                    bottom: 700,
                },
                PhysicalRect {
                    left: 980,
                    top: 160,
                    right: 1700,
                    bottom: 640,
                },
                PhysicalRect {
                    left: 1860,
                    top: 160,
                    right: 2580,
                    bottom: 640,
                },
            ];
            validate_window_layout(&arranged, display).unwrap();
        }
    }
}

#[cfg(windows)]
pub(crate) use windows::open;

#[cfg(not(windows))]
pub(crate) fn open(
    _performance: crate::performance::PerformanceRecorder,
    _history: crate::history::ScreenshotHistory,
    _settings: crate::settings::UserSettings,
    _settings_path: std::path::PathBuf,
    _acceptance: crate::PinLifecycleAcceptanceOptions,
    _cx: &mut gpui::App,
) -> Result<(), Box<dyn std::error::Error>> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Pin lifecycle acceptance is currently Windows-only",
    )
    .into())
}
