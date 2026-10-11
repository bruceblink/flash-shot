//! Seeded overlay fixtures and acceptance-window helpers.

use super::*;

/// Opens one disposable overlay with a seeded visual state for native screenshot acceptance.
pub(in crate::app) fn open_ui_acceptance(
    started_at: Instant,
    performance: PerformanceRecorder,
    history: ScreenshotHistory,
    mut settings: UserSettings,
    settings_path: std::path::PathBuf,
    acceptance: crate::OverlayUiAcceptanceOptions,
    cx: &mut App,
) -> Result<(), Box<dyn std::error::Error>> {
    let width = acceptance.width.max(360.0).round() as u32;
    let height = acceptance.height.max(320.0).round() as u32;
    let display = DisplayInfo {
        id: "overlay-ui-acceptance".to_owned(),
        platform_id: 0,
        physical_bounds: PhysicalRect {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        },
        work_area: PhysicalRect {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        },
        dpi_x: 96,
        dpi_y: 96,
        scale_factor: 1.0,
        rotation: DisplayRotation::Landscape,
        bits_per_pixel: 32,
        primary: true,
    };
    let frame = overlay_ui_acceptance_frame(display.physical_bounds)?;
    let preview = super::super::render_image::render_image_from_capture(&frame)?.image;
    // A disposable probe should render the overlay without registering user-facing hotkeys.
    settings.capture_shortcut_enabled = false;
    let readiness = performance.clone();
    let app = cx.new(|cx| FlashShotApp::new(performance, history, settings, settings_path, cx));
    app.update(cx, |app, cx| {
        let _ = app.session.begin();
        let _ = app.session.frames_ready();
        app.frame = Some(frame);
        app.preview = Some(preview.clone());
        match acceptance.scenario {
            crate::OverlayUiAcceptanceScenario::SmartTarget { kind } => {
                let target = overlay_ui_acceptance_target(display.physical_bounds, kind);
                app.inspection_target = Some(target);
                app.hover_pixel = Some(PhysicalPoint {
                    x: target.bounds.left + target.bounds.width() as i32 / 2,
                    y: target.bounds.top + target.bounds.height() as i32 / 2,
                });
                let width = target.bounds.width().to_string();
                let height = target.bounds.height().to_string();
                app.status = app.settings.locale.format_template(
                    UiText::OverlaySmartTargetReady,
                    &[("width", &width), ("height", &height)],
                );
            }
            crate::OverlayUiAcceptanceScenario::SelectedRegion {
                placement,
                show_more_actions,
                show_annotation_controls,
                show_annotation_tool_group,
            } => {
                let selection = overlay_ui_acceptance_selection(display.physical_bounds, placement);
                match app.session.select(selection) {
                    Ok(()) => {
                        app.selection_drag.select(selection);
                        if show_more_actions {
                            app.toggle_overlay_more_actions(cx);
                        }
                        if show_annotation_controls {
                            app.toggle_overlay_annotation_controls(cx);
                            // The marking acceptance surface must exercise the contextual row,
                            // not merely the visibility toggle. Rectangle exposes every W3
                            // control while keeping the synthetic fixture free of new geometry.
                            app.select_annotation_tool(AnnotationTool::Rectangle, cx);
                            if show_annotation_tool_group {
                                app.toggle_annotation_tool_group(
                                    &display.id,
                                    AnnotationToolGroup::Shape,
                                    cx,
                                );
                            }
                        }
                        let width = selection.width().to_string();
                        let height = selection.height().to_string();
                        app.status = app.settings.locale.format_template(
                            UiText::OverlaySelectionReady,
                            &[("width", &width), ("height", &height)],
                        );
                    }
                    Err(error) => {
                        let error_detail = error.to_string();
                        app.status = app.settings.locale.format_template(
                            UiText::OverlaySeedSelectionFailed,
                            &[("error", &error_detail)],
                        );
                    }
                }
            }
        }
        cx.notify();
    });
    let operation_generation = app.read(cx).operation_generation;
    let overlay_app = app.clone();
    let overlay_display = display.clone();
    let overlay_preview = preview.clone();
    let window = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::centered(
                size(px(width as f32), px(height as f32)),
                cx,
            )),
            titlebar: None,
            focus: true,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            window_background: WindowBackgroundAppearance::Opaque,
            ..Default::default()
        },
        move |window, cx| {
            let overlay = cx.new(|cx| {
                CaptureOverlay::new(
                    overlay_app,
                    overlay_display,
                    overlay_preview,
                    operation_generation,
                    false,
                    cx,
                )
            });
            overlay.read(cx).focus_handle(cx).focus(window, cx);
            overlay
        },
    )?;
    app.update(cx, |app, _| app.overlay_windows = vec![window]);
    readiness.record_duration("startup_to_overlay_acceptance_ready", started_at.elapsed());
    Ok(())
}

/// Builds a bounded BGRA frame so the acceptance overlay exercises the real image upload path.
pub(super) fn overlay_ui_acceptance_frame(bounds: PhysicalRect) -> io::Result<CaptureFrame> {
    let width = bounds.width();
    let height = bounds.height();
    let stride = width as usize * 4;
    let length = stride
        .checked_mul(height as usize)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "acceptance frame too large"))?;
    let mut pixels = vec![0; length];
    for y in 0..height as usize {
        for x in 0..width as usize {
            let offset = y * stride + x * 4;
            let grid = ((x / 32 + y / 32) % 2) as u8;
            pixels[offset] = 0x36 + grid * 0x10;
            pixels[offset + 1] = 0x2D + grid * 0x0C;
            pixels[offset + 2] = 0x22 + grid * 0x08;
            pixels[offset + 3] = 0xFF;
        }
    }
    Ok(CaptureFrame {
        bounds,
        width,
        height,
        stride,
        format: PixelFormat::Bgra8,
        pixels: pixels.into(),
        capture_duration: Duration::ZERO,
        cpu_copy_count: 0,
    })
}

/// Seeds a visible candidate with enough surrounding space to inspect the HUD in a screenshot.
pub(super) fn overlay_ui_acceptance_target(
    bounds: PhysicalRect,
    kind: InspectionKind,
) -> InspectionTarget {
    let width = (bounds.width() / 2).clamp(180, 720) as i32;
    let height = (bounds.height() / 4).clamp(96, 300) as i32;
    let left = bounds.left + (bounds.width() as i32 - width) / 2;
    let top = bounds.top + (bounds.height() as i32 - height) / 2;
    InspectionTarget {
        bounds: PhysicalRect {
            left,
            top,
            right: left + width,
            bottom: top + height,
        },
        kind,
    }
}

/// Seeds a compact region inside physical desktop bounds for deterministic overlay screenshots.
///
/// The bottom-right mode leaves the image near the raw display edge so production layout must
/// lift the toolbar and menu above it; the centered mode keeps dense controls easy to inspect.
pub(super) fn overlay_ui_acceptance_selection(
    bounds: PhysicalRect,
    placement: crate::OverlayUiAcceptanceSelectionPlacement,
) -> PhysicalRect {
    let (width, height) = match placement {
        crate::OverlayUiAcceptanceSelectionPlacement::Centered => (
            (bounds.width() / 2).clamp(160, 320) as i32,
            (bounds.height() / 4).clamp(96, 160) as i32,
        ),
        // A fixed small region exposes wrapping and edge-placement behavior at 420 px.
        crate::OverlayUiAcceptanceSelectionPlacement::BottomRight => (160, 96),
    };
    let (left, top) = match placement {
        crate::OverlayUiAcceptanceSelectionPlacement::Centered => (
            bounds.left + (bounds.width() as i32 - width) / 2,
            bounds.top + (bounds.height() as i32 - height) * 2 / 5,
        ),
        crate::OverlayUiAcceptanceSelectionPlacement::BottomRight => (
            bounds.right - OVERLAY_EDGE_INSET.round() as i32 - width,
            // Keep enough upper space for the full More menu while forcing the main bar above.
            bounds.bottom - OVERLAY_ACTION_BAR_GAP.round() as i32 - height,
        ),
    };
    PhysicalRect {
        left,
        top,
        right: left + width,
        bottom: top + height,
    }
}
