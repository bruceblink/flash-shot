//! GPUI window orchestration and multi-display capture previews.

use super::*;

// Keeps every manual-scroll command on one row after the incomplete-session guidance was added.
const MANUAL_SCROLL_CONTROL_WIDTH: f32 = 440.0;

pub(super) fn open_capture_overlays(
    app: gpui::Entity<FlashShotApp>,
    displays: Vec<CapturedDisplayPreview>,
    pipeline: CapturePipelineMeasurement,
    cx: &mut gpui::App,
) {
    if app.read(cx).session.state() != CaptureSessionState::Selecting {
        return;
    }
    let mut windows = Vec::with_capacity(displays.len());
    for display in displays {
        let bounds = display_window_bounds(&display.display);
        let display_id = DisplayId::new(display.display.platform_id);
        let info = display.display;
        let primary = info.primary;
        let preview = display.preview;
        let performance = app.read(cx).performance.clone();
        let primary_pipeline = primary.then_some(pipeline);
        match cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: None,
                focus: primary,
                show: true,
                kind: WindowKind::PopUp,
                is_movable: false,
                is_resizable: false,
                is_minimizable: false,
                display_id: Some(display_id),
                window_background: WindowBackgroundAppearance::Opaque,
                window_min_size: None,
                ..Default::default()
            },
            {
                let app = app.clone();
                move |window, cx| {
                    if let Some(pipeline) = primary_pipeline {
                        window.on_next_frame(move |_, _| {
                            performance.record_capture_pipeline(pipeline.finish(Instant::now()));
                        });
                    }
                    let overlay = cx.new(|cx| CaptureOverlay::new(app, info, preview, cx));
                    if primary {
                        overlay.read(cx).focus_handle(cx).focus(window, cx);
                    }
                    overlay
                }
            },
        ) {
            Ok(window) => windows.push(window),
            Err(error) => {
                close_overlay_windows(windows, cx);
                let message = format!("Capture overlay failed: {error}");
                app.update(cx, |app, cx| {
                    let _ = app.session.fail(message.clone());
                    app.status = message;
                    app.return_to_background();
                    cx.notify();
                });
                log::warn!(target: "flash_shot::overlay", "overlay_open_failed error={error}");
                return;
            }
        }
    }
    app.update(cx, |app, _| app.overlay_windows = windows);
    cx.activate(true);
}

pub(super) fn open_image_overlay(
    app: gpui::Entity<FlashShotApp>,
    bounds: PhysicalRect,
    cx: &mut gpui::App,
) {
    if app.read(cx).session.state() != CaptureSessionState::Selecting {
        return;
    }
    let Some(preview) = app.read(cx).preview.clone() else {
        return;
    };
    let display = crate::platform::display::DisplayInfo {
        id: "opened-image".to_owned(),
        platform_id: 0,
        physical_bounds: bounds,
        work_area: bounds,
        dpi_x: 96,
        dpi_y: 96,
        scale_factor: 1.0,
        rotation: crate::platform::display::DisplayRotation::Landscape,
        bits_per_pixel: 32,
        primary: true,
    };
    let window_size = pinned_size(bounds.width() as f32, bounds.height() as f32);
    let overlay_app = app.clone();
    match cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::centered(window_size, cx)),
            titlebar: Some(gpui::TitlebarOptions {
                title: Some("Flash Shot - Edit Image".into()),
                ..Default::default()
            }),
            focus: true,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: true,
            is_resizable: true,
            is_minimizable: false,
            window_background: WindowBackgroundAppearance::Opaque,
            window_min_size: Some(size(px(480.0), px(360.0))),
            ..Default::default()
        },
        move |window, cx| {
            let overlay = cx.new(|cx| CaptureOverlay::new(overlay_app, display, preview, cx));
            overlay.read(cx).focus_handle(cx).focus(window, cx);
            overlay
        },
    ) {
        Ok(window) => {
            app.update(cx, |app, _| app.overlay_windows = vec![window]);
            cx.activate(true);
        }
        Err(error) => {
            let message = format!("Image editor window failed: {error}");
            app.update(cx, |app, cx| {
                let _ = app.session.fail(message.clone());
                app.status = message;
                app.return_to_background();
                cx.notify();
            });
            log::warn!(target: "flash_shot::image", "image_editor_open_failed error={error}");
        }
    }
}

pub(super) fn open_manual_scroll_control(app: gpui::Entity<FlashShotApp>, cx: &mut gpui::App) {
    if app.read(cx).manual_scroll.state() != crate::scroll::ManualScrollState::Collecting {
        return;
    }
    let control_app = app.clone();
    match cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::centered(
                size(px(MANUAL_SCROLL_CONTROL_WIDTH), px(120.0)),
                cx,
            )),
            titlebar: Some(gpui::TitlebarOptions {
                title: Some("Flash Shot - Manual Scroll".into()),
                ..Default::default()
            }),
            focus: true,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: true,
            is_resizable: false,
            is_minimizable: false,
            window_background: WindowBackgroundAppearance::Opaque,
            ..Default::default()
        },
        move |window, cx| {
            let close_app = control_app.clone();
            window.on_window_should_close(cx, move |_, cx| {
                close_app.update(cx, |app, cx| app.manual_scroll_control_closed(cx));
                true
            });
            let control = cx.new(|cx| ManualScrollControl::new(control_app, cx));
            control.read(cx).focus_handle(cx).focus(window, cx);
            control
        },
    ) {
        Ok(window) => app.update(cx, |app, _| app.scroll_window = Some(window)),
        Err(error) => {
            app.update(cx, |app, cx| {
                let _ = app.manual_scroll.cancel();
                let _ = app.manual_scroll.reset();
                app.manual_scroll_selection = None;
                app.manual_scroll_capture_in_flight = false;
                app.status = format!("Could not open manual scroll controls: {error}");
                app.return_to_background();
                cx.notify();
            });
            log::warn!(target: "flash_shot::scroll", "manual_scroll_control_open_failed error={error}");
        }
    }
}

pub(super) fn close_overlay_windows(
    windows: Vec<gpui::WindowHandle<CaptureOverlay>>,
    cx: &mut gpui::App,
) {
    for window in windows {
        let _ = window.update(cx, |_, window, _| window.remove_window());
    }
}

/// Extracts the HWND of a GPUI window for the small set of native visibility controls.
pub(super) fn native_window_handle(window: &gpui::Window) -> Option<isize> {
    HasWindowHandle::window_handle(window)
        .ok()
        .and_then(|handle| match handle.as_raw() {
            RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
            _ => None,
        })
}

pub(super) struct CapturedDesktopPreview {
    pub(super) capture: crate::platform::capture::VirtualDesktopCapture,
    pub(super) workspace_preview: crate::app::render_image::CaptureRenderImage,
    pub(super) displays: Vec<CapturedDisplayPreview>,
    pub(super) render_upload_copy_count: u32,
}

#[derive(Clone, Copy)]
pub(super) struct CapturePipelineMeasurement {
    pub(super) started_at: Instant,
    pub(super) frame_ready_at: Instant,
    pub(super) platform_capture: std::time::Duration,
    pub(super) display_count: usize,
    pub(super) frame_width: u32,
    pub(super) frame_height: u32,
    pub(super) capture_cpu_copy_count: u32,
    pub(super) render_upload_copy_count: u32,
    pub(super) overlay_image_count: usize,
    pub(super) overlay_upload_bytes: usize,
    pub(super) workspace_upload_bytes: usize,
}

impl CapturePipelineMeasurement {
    fn finish(self, overlay_frame_at: Instant) -> CapturePipelineSample {
        CapturePipelineSample {
            shortcut_to_frame_ready: self.frame_ready_at.duration_since(self.started_at),
            shortcut_to_overlay_frame: overlay_frame_at.duration_since(self.started_at),
            platform_capture: self.platform_capture,
            display_count: self.display_count,
            frame_width: self.frame_width,
            frame_height: self.frame_height,
            capture_cpu_copy_count: self.capture_cpu_copy_count,
            render_upload_copy_count: self.render_upload_copy_count,
            overlay_image_count: self.overlay_image_count,
            overlay_upload_bytes: self.overlay_upload_bytes,
            workspace_upload_bytes: self.workspace_upload_bytes,
        }
    }
}

pub(super) struct CapturedDisplayPreview {
    pub(super) display: crate::platform::display::DisplayInfo,
    pub(super) preview: Arc<RenderImage>,
    pub(super) upload_bytes: usize,
}

pub(super) fn capture_virtual_desktop_preview(
    include_cursor: bool,
) -> std::io::Result<CapturedDesktopPreview> {
    let display_captures = capture_displays_with_options(CaptureOptions { include_cursor })?;
    let frame = compose_captured_displays(&display_captures)?;
    let displays = display_captures
        .into_iter()
        .map(|capture| {
            let preview = render_image_from_capture(&capture.frame)?;
            Ok(CapturedDisplayPreview {
                display: capture.display,
                preview: preview.image,
                upload_bytes: preview.upload_bytes,
            })
        })
        .collect::<std::io::Result<Vec<_>>>()?;
    let workspace_preview = if displays.len() == 1 {
        // The main workspace and the only overlay show identical pixels. Reuse
        // the decoded image instead of allocating and uploading it a second time.
        crate::app::render_image::CaptureRenderImage {
            image: displays[0].preview.clone(),
            upload_bytes: 0,
        }
    } else {
        render_image_from_capture(&frame)?
    };
    let render_upload_copy_count =
        displays.len() as u32 + u32::from(workspace_preview.upload_bytes != 0);
    Ok(CapturedDesktopPreview {
        capture: crate::platform::capture::VirtualDesktopCapture {
            display_count: displays.len(),
            frame,
        },
        workspace_preview,
        displays,
        render_upload_copy_count,
    })
}

pub(super) fn capture_virtual_desktop_frame(include_cursor: bool) -> std::io::Result<CaptureFrame> {
    let display_captures = capture_displays_with_options(CaptureOptions { include_cursor })?;
    compose_captured_displays(&display_captures)
}

pub(super) fn compose_captured_displays(
    display_captures: &[DisplayCapture],
) -> std::io::Result<CaptureFrame> {
    match display_captures {
        [capture] => Ok(capture.frame.clone()),
        captures => compose_virtual_desktop(captures),
    }
}

pub(super) fn display_window_bounds(
    display: &crate::platform::display::DisplayInfo,
) -> Bounds<Pixels> {
    let scale = display.scale_factor.max(1.0);
    Bounds::new(
        point(
            px(display.physical_bounds.left as f32 / scale),
            px(display.physical_bounds.top as f32 / scale),
        ),
        size(
            px(display.physical_bounds.width() as f32 / scale),
            px(display.physical_bounds.height() as f32 / scale),
        ),
    )
}
