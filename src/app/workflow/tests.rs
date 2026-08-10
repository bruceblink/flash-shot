use super::{
    ColorFormat, KeyboardCommand, TranslationOutcome, adjusted_number_value,
    annotation_added_status, annotation_cancelled_status, annotation_document_path,
    annotation_position, annotation_sidecar_path, capture::focused_window_selection,
    claim_idle_completion, compose_captured_displays, copy_annotated_frame_selection,
    delayed_capture_status, drawing_status, export_path, fill_alpha, fill_color, format_hsl,
    format_recording_progress, format_recording_stopping, hovered_color, intersect_rect,
    is_current_operation, keyboard_command, load_annotation_document, manual_scroll_control_bounds,
    manual_scroll_control_rect, next_annotation_counters, next_annotation_selection,
    next_quick_save_path_with_prefix, next_recording_audio_selection,
    next_recording_display_selection, ocr_language_label, ocr_support_status,
    open_annotation_project, open_image_project, pinned_size, project_image_path,
    quick_save_annotated_frame_selection_in_with_prefix,
    quick_save_annotated_frame_selection_with_fallback,
    quick_save_full_screen_frame_in_with_prefix, quick_save_with_fallback,
    recognition_start_conflict_status, recording_audio_selection_label,
    recording_discovery_conflict_status, recording_discovery_result_is_applicable,
    recording_display_selection_label, recording_start_conflict_status,
    recording_start_failure_status, recording_support_status, recording_target_label,
    resolve_pointer_selection, save_annotated_frame_selection, save_annotation_document,
    save_editable_project, smart_target_status, style_for_tool, text_annotation_with_content,
    tool_selected_status, translation_failure_status, translation_service_test_status,
    translation_support_status, with_alpha,
};
use crate::{
    domain::{
        annotation::{
            Annotation, AnnotationCommand, AnnotationDocument, AnnotationId, AnnotationKind,
            AnnotationStyle, AnnotationTool, CommandHistory,
        },
        geometry::{PhysicalPoint, PhysicalRect},
        session::CaptureSessionState,
    },
    platform::{
        capture::{CaptureFrame, DisplayCapture, PixelFormat},
        clipboard::ClipboardService,
        display::{DisplayInfo, DisplayRotation},
        window_inspector::{InspectionKind, InspectionTarget},
    },
    recording::AudioSource,
};
use gpui::Keystroke;
use std::{
    cell::RefCell,
    io::{self, BufReader},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

#[derive(Default)]
struct RecordingClipboard {
    copied: RefCell<Option<CaptureFrame>>,
}

impl ClipboardService for RecordingClipboard {
    fn copy_image(&self, frame: &CaptureFrame) -> io::Result<()> {
        self.copied.replace(Some(frame.clone()));
        Ok(())
    }

    fn copy_text(&self, _text: &str) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn copy_uses_the_pixel_correct_selected_region() {
    let frame = CaptureFrame {
        bounds: PhysicalRect {
            left: -2,
            top: 10,
            right: 1,
            bottom: 12,
        },
        width: 3,
        height: 2,
        stride: 12,
        format: PixelFormat::Bgra8,
        pixels: Arc::from([
            1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255, 13, 14, 15, 255, 16, 17, 18,
            255,
        ]),
        capture_duration: Duration::ZERO,
        cpu_copy_count: 1,
    };
    let clipboard = RecordingClipboard::default();
    let document = AnnotationDocument::new(frame.bounds).unwrap();

    copy_annotated_frame_selection(
        &frame,
        &document,
        PhysicalRect {
            left: -1,
            top: 10,
            right: 1,
            bottom: 12,
        },
        &clipboard,
    )
    .unwrap();

    let copied = clipboard.copied.borrow();
    let copied = copied.as_ref().unwrap();
    assert_eq!((copied.width, copied.height), (2, 2));
    assert_eq!(
        copied.pixels.as_ref(),
        &[4, 5, 6, 255, 7, 8, 9, 255, 13, 14, 15, 255, 16, 17, 18, 255]
    );
}

#[test]
fn annotated_copy_composites_before_cropping_the_selection() {
    let frame = CaptureFrame {
        bounds: PhysicalRect {
            left: -2,
            top: 10,
            right: 2,
            bottom: 11,
        },
        width: 4,
        height: 1,
        stride: 16,
        format: PixelFormat::Bgra8,
        pixels: Arc::from([10, 10, 10, 255].repeat(4)),
        capture_duration: Duration::ZERO,
        cpu_copy_count: 1,
    };
    let mut document = AnnotationDocument::new(frame.bounds).unwrap();
    let mut history = CommandHistory::default();
    history
        .apply(
            &mut document,
            AnnotationCommand::Insert(Annotation {
                id: AnnotationId::new(1),
                kind: AnnotationKind::Line {
                    start: PhysicalPoint { x: -1, y: 10 },
                    end: PhysicalPoint { x: 0, y: 10 },
                },
                style: AnnotationStyle {
                    stroke_rgba: 0xFF0000FF,
                    fill_rgba: None,
                    stroke_width: 1,
                    text_font_size: 24,
                },
            }),
        )
        .unwrap();
    let clipboard = RecordingClipboard::default();

    copy_annotated_frame_selection(
        &frame,
        &document,
        PhysicalRect {
            left: -1,
            top: 10,
            right: 1,
            bottom: 11,
        },
        &clipboard,
    )
    .unwrap();

    let copied = clipboard.copied.borrow();
    let copied = copied.as_ref().unwrap();
    assert_eq!((copied.width, copied.height), (2, 1));
    assert_eq!(
        copied.pixel_at(PhysicalPoint { x: -1, y: 10 }).unwrap().red,
        255
    );
    assert_eq!(
        copied.pixel_at(PhysicalPoint { x: 0, y: 10 }).unwrap().red,
        255
    );
    assert_eq!(
        frame.pixel_at(PhysicalPoint { x: -2, y: 10 }).unwrap().red,
        10
    );
}

#[test]
fn hovered_color_uses_the_frame_physical_coordinates_and_rejects_missing_samples() {
    let frame = CaptureFrame {
        bounds: PhysicalRect {
            left: -2,
            top: 10,
            right: 0,
            bottom: 11,
        },
        width: 2,
        height: 1,
        stride: 8,
        format: PixelFormat::Bgra8,
        pixels: Arc::from([5, 171, 18, 255, 7, 8, 9, 255]),
        capture_duration: Duration::ZERO,
        cpu_copy_count: 1,
    };

    assert_eq!(
        hovered_color(
            Some(&frame),
            Some(PhysicalPoint { x: -2, y: 10 }),
            ColorFormat::Hex
        ),
        Some("#12AB05".to_owned())
    );
    assert_eq!(hovered_color(Some(&frame), None, ColorFormat::Hex), None);
    assert_eq!(
        hovered_color(
            Some(&frame),
            Some(PhysicalPoint { x: 0, y: 10 }),
            ColorFormat::Hex,
        ),
        None
    );
}

#[test]
fn color_copy_formats_are_stable_and_cycle_through_all_supported_syntaxes() {
    let color = crate::platform::capture::PixelColor {
        red: 18,
        green: 171,
        blue: 5,
        alpha: 255,
    };

    assert_eq!(ColorFormat::Hex.format(color), "#12AB05");
    assert_eq!(ColorFormat::Rgb.format(color), "rgb(18, 171, 5)");
    assert_eq!(ColorFormat::Hsl.format(color), "hsl(115.3, 94.3%, 34.5%)");
    assert_eq!(format_hsl(128, 128, 128), "hsl(0, 0%, 50.2%)");
    assert_eq!(ColorFormat::Hex.next(), ColorFormat::Rgb);
    assert_eq!(ColorFormat::Rgb.next(), ColorFormat::Hsl);
    assert_eq!(ColorFormat::Hsl.next(), ColorFormat::Hex);
}

#[test]
fn keyboard_commands_cover_confirm_cancel_and_physical_nudging() {
    assert_eq!(
        keyboard_command(&Keystroke::parse("enter").unwrap()),
        Some(KeyboardCommand::Copy)
    );
    assert_eq!(
        keyboard_command(&Keystroke::parse("shift-enter").unwrap()),
        Some(KeyboardCommand::QuickSave)
    );
    assert_eq!(
        keyboard_command(&Keystroke::parse("c").unwrap()),
        Some(KeyboardCommand::CopyColor)
    );
    assert_eq!(keyboard_command(&Keystroke::parse("ctrl-c").unwrap()), None);
    assert_eq!(
        keyboard_command(&Keystroke::parse("escape").unwrap()),
        Some(KeyboardCommand::Cancel)
    );
    assert_eq!(
        keyboard_command(&Keystroke::parse("left").unwrap()),
        Some(KeyboardCommand::Nudge(-1, 0))
    );
    assert_eq!(
        keyboard_command(&Keystroke::parse("shift-down").unwrap()),
        Some(KeyboardCommand::Nudge(0, 10))
    );
    assert_eq!(
        keyboard_command(&Keystroke::parse("ctrl-left").unwrap()),
        Some(KeyboardCommand::MoveColorCursor(-1, 0))
    );
    assert_eq!(
        keyboard_command(&Keystroke::parse("ctrl-down").unwrap()),
        Some(KeyboardCommand::MoveColorCursor(0, 1))
    );
    assert_eq!(
        keyboard_command(&Keystroke::parse("ctrl-shift-left").unwrap()),
        Some(KeyboardCommand::MoveColorCursor(-1, 0))
    );
    assert_eq!(
        keyboard_command(&Keystroke::parse("ctrl-enter").unwrap()),
        None
    );
    assert_eq!(
        keyboard_command(&Keystroke::parse("ctrl-d").unwrap()),
        Some(KeyboardCommand::Duplicate)
    );
    assert_eq!(
        keyboard_command(&Keystroke::parse("ctrl-shift-]").unwrap()),
        Some(KeyboardCommand::BringForward)
    );
    assert_eq!(
        keyboard_command(&Keystroke::parse("ctrl-shift-[").unwrap()),
        Some(KeyboardCommand::SendBackward)
    );
    assert_eq!(
        keyboard_command(&Keystroke::parse("ctrl-shift-r").unwrap()),
        Some(KeyboardCommand::RotateClockwise)
    );
    assert_eq!(
        keyboard_command(&Keystroke::parse("ctrl-z").unwrap()),
        Some(KeyboardCommand::Undo)
    );
    assert_eq!(
        keyboard_command(&Keystroke::parse("ctrl-shift-z").unwrap()),
        Some(KeyboardCommand::Redo)
    );
    assert_eq!(
        keyboard_command(&Keystroke::parse("delete").unwrap()),
        Some(KeyboardCommand::Delete)
    );
    assert_eq!(
        keyboard_command(&Keystroke::parse("backspace").unwrap()),
        Some(KeyboardCommand::Delete)
    );
    assert_eq!(
        keyboard_command(&Keystroke::parse("tab").unwrap()),
        Some(KeyboardCommand::SelectNextAnnotation)
    );
    assert_eq!(
        keyboard_command(&Keystroke::parse("shift-tab").unwrap()),
        Some(KeyboardCommand::SelectPreviousAnnotation)
    );
    assert_eq!(
        keyboard_command(&Keystroke::parse("r").unwrap()),
        Some(KeyboardCommand::SelectTool(Some(AnnotationTool::Rectangle)))
    );
    assert_eq!(
        keyboard_command(&Keystroke::parse("t").unwrap()),
        Some(KeyboardCommand::SelectTool(Some(AnnotationTool::Text)))
    );
    assert_eq!(
        keyboard_command(&Keystroke::parse("s").unwrap()),
        Some(KeyboardCommand::SelectTool(None))
    );
    assert_eq!(keyboard_command(&Keystroke::parse("ctrl-r").unwrap()), None);
}

#[test]
fn annotation_selection_cycles_in_layer_order() {
    let annotations = [
        AnnotationId::new(1),
        AnnotationId::new(2),
        AnnotationId::new(3),
    ];

    assert_eq!(
        next_annotation_selection(&annotations, None, false),
        Some(AnnotationId::new(1))
    );
    assert_eq!(
        next_annotation_selection(&annotations, Some(AnnotationId::new(1)), false),
        Some(AnnotationId::new(2))
    );
    assert_eq!(
        next_annotation_selection(&annotations, Some(AnnotationId::new(3)), false),
        Some(AnnotationId::new(1))
    );
    assert_eq!(
        next_annotation_selection(&annotations, None, true),
        Some(AnnotationId::new(3))
    );
    assert_eq!(
        next_annotation_selection(&annotations, Some(AnnotationId::new(1)), true),
        Some(AnnotationId::new(3))
    );
    assert_eq!(annotation_position(&annotations, AnnotationId::new(2)), 2);
    assert_eq!(next_annotation_selection(&[], None, false), None);
}

#[test]
fn number_marker_adjustment_clamps_to_the_supported_range() {
    assert_eq!(adjusted_number_value(7, 2), 9);
    assert_eq!(adjusted_number_value(1, -1), 1);
    assert_eq!(adjusted_number_value(u32::MAX, 1), u32::MAX);
}

#[test]
fn freehand_tool_has_specific_user_feedback() {
    use crate::domain::annotation::AnnotationTool;

    assert_eq!(
        tool_selected_status(AnnotationTool::Freehand),
        "Freehand tool selected"
    );
    assert_eq!(
        drawing_status(AnnotationTool::Freehand),
        "Drawing freehand..."
    );
    assert_eq!(
        annotation_added_status(Some(AnnotationTool::Freehand)),
        "Freehand stroke added"
    );
    assert_eq!(
        annotation_cancelled_status(Some(AnnotationTool::Freehand)),
        "Freehand stroke cancelled"
    );
}

#[test]
fn watermark_tool_has_specific_user_feedback() {
    use crate::domain::annotation::AnnotationTool;

    assert_eq!(
        tool_selected_status(AnnotationTool::Watermark),
        "Watermark tool selected"
    );
    assert_eq!(
        drawing_status(AnnotationTool::Watermark),
        "Placing watermark..."
    );
    assert_eq!(
        annotation_added_status(Some(AnnotationTool::Watermark)),
        "Watermark added"
    );
    assert_eq!(
        annotation_cancelled_status(Some(AnnotationTool::Watermark)),
        "Watermark cancelled"
    );
}

#[test]
fn text_edit_replaces_content_without_changing_annotation_identity_or_style() {
    let style = AnnotationStyle {
        stroke_rgba: 0xFFCC00FF,
        fill_rgba: None,
        stroke_width: 6,
        text_font_size: 24,
    };
    let text = Annotation {
        id: AnnotationId::new(7),
        kind: AnnotationKind::Text {
            origin: PhysicalPoint { x: 12, y: 16 },
            content: "Before".to_owned(),
        },
        style,
    };
    let watermark = Annotation {
        id: AnnotationId::new(8),
        kind: AnnotationKind::Watermark {
            origin: PhysicalPoint { x: 20, y: 24 },
            content: "Old mark".to_owned(),
        },
        style,
    };

    assert_eq!(
        text_annotation_with_content(text.clone(), "After".to_owned()).unwrap(),
        Annotation {
            id: text.id,
            kind: AnnotationKind::Text {
                origin: PhysicalPoint { x: 12, y: 16 },
                content: "After".to_owned(),
            },
            style,
        }
    );
    assert_eq!(
        text_annotation_with_content(watermark, "New mark".to_owned())
            .unwrap()
            .kind,
        AnnotationKind::Watermark {
            origin: PhysicalPoint { x: 20, y: 24 },
            content: "New mark".to_owned(),
        }
    );
    assert!(
        text_annotation_with_content(
            Annotation {
                id: AnnotationId::new(9),
                kind: AnnotationKind::Rectangle {
                    bounds: PhysicalRect {
                        left: 0,
                        top: 0,
                        right: 10,
                        bottom: 10,
                    },
                },
                style,
            },
            "not text".to_owned(),
        )
        .is_none()
    );
}

#[test]
fn highlight_tool_has_specific_user_feedback_and_translucent_style() {
    use crate::domain::annotation::{AnnotationStyle, AnnotationTool};

    assert_eq!(
        tool_selected_status(AnnotationTool::Highlight),
        "Highlight tool selected"
    );
    assert_eq!(
        drawing_status(AnnotationTool::Highlight),
        "Drawing highlight..."
    );
    assert_eq!(
        annotation_added_status(Some(AnnotationTool::Highlight)),
        "Highlight added"
    );
    assert_eq!(
        style_for_tool(
            AnnotationTool::Highlight,
            AnnotationStyle {
                stroke_rgba: 0xFFCC00FF,
                fill_rgba: Some(0xFFFFFFFF),
                stroke_width: 10,
                text_font_size: 24,
            },
        ),
        AnnotationStyle {
            stroke_rgba: 0xFFCC0066,
            fill_rgba: None,
            stroke_width: 1,
            text_font_size: 24,
        }
    );
}

#[test]
fn mosaic_tool_has_specific_user_feedback() {
    use crate::domain::annotation::AnnotationTool;

    assert_eq!(
        tool_selected_status(AnnotationTool::Mosaic),
        "Mosaic tool selected"
    );
    assert_eq!(drawing_status(AnnotationTool::Mosaic), "Drawing mosaic...");
    assert_eq!(
        annotation_added_status(Some(AnnotationTool::Mosaic)),
        "Mosaic added"
    );
    assert_eq!(
        annotation_cancelled_status(Some(AnnotationTool::Mosaic)),
        "Mosaic cancelled"
    );
}

#[test]
fn blur_tool_has_specific_user_feedback() {
    use crate::domain::annotation::AnnotationTool;

    assert_eq!(
        tool_selected_status(AnnotationTool::Blur),
        "Blur tool selected"
    );
    assert_eq!(drawing_status(AnnotationTool::Blur), "Drawing blur...");
    assert_eq!(
        annotation_added_status(Some(AnnotationTool::Blur)),
        "Blur added"
    );
    assert_eq!(
        annotation_cancelled_status(Some(AnnotationTool::Blur)),
        "Blur cancelled"
    );
}

#[test]
fn fill_color_preserves_rgb_and_uses_transparent_alpha() {
    assert_eq!(fill_color(0xFF3B30FF), 0xFF3B3066);
    assert_eq!(fill_color(0xFF3B3080), 0xFF3B3033);
}

#[test]
fn opacity_preserves_rgb_and_scales_the_shape_fill() {
    assert_eq!(with_alpha(0xFF3B30FF, 128), 0xFF3B3080);
    assert_eq!(fill_alpha(255), 0x66);
    assert_eq!(fill_alpha(128), 0x33);
}

#[test]
fn pinned_window_size_matches_the_image_without_downscaling() {
    let small = pinned_size(100.0, 80.0);
    assert_eq!(f32::from(small.width), 100.0);
    assert_eq!(f32::from(small.height), 80.0);

    let large = pinned_size(1_280.0, 720.0);
    assert_eq!(f32::from(large.width), 1_280.0);
    assert_eq!(f32::from(large.height), 720.0);
}

#[test]
fn delayed_capture_status_reports_each_remaining_second() {
    let remaining = (1..=10)
        .rev()
        .map(delayed_capture_status)
        .collect::<Vec<_>>();

    assert_eq!(
        remaining.first().unwrap(),
        "Capture scheduled in 10 seconds"
    );
    assert_eq!(remaining.last().unwrap(), "Capture scheduled in 1 seconds");
    assert_eq!(remaining.len(), 10);
}

#[test]
fn stale_idle_completion_releases_its_slot_without_overriding_new_state() {
    let mut active = Some(12);
    assert!(claim_idle_completion(
        &mut active,
        12,
        12,
        CaptureSessionState::Idle
    ));
    assert_eq!(active, None);

    let mut superseded = Some(12);
    assert!(!claim_idle_completion(
        &mut superseded,
        13,
        12,
        CaptureSessionState::Idle
    ));
    assert_eq!(superseded, None);

    let mut newer_task = Some(13);
    assert!(!claim_idle_completion(
        &mut newer_task,
        13,
        12,
        CaptureSessionState::Idle
    ));
    assert_eq!(newer_task, Some(13));

    let mut active_capture = Some(12);
    assert!(!claim_idle_completion(
        &mut active_capture,
        12,
        12,
        CaptureSessionState::Capturing
    ));
    assert_eq!(active_capture, None);
}

#[test]
fn captured_display_composition_reuses_one_frame_without_an_extra_copy() {
    let bounds = PhysicalRect {
        left: 0,
        top: 0,
        right: 2,
        bottom: 1,
    };
    let frame = CaptureFrame {
        bounds,
        width: 2,
        height: 1,
        stride: 8,
        format: PixelFormat::Bgra8,
        pixels: Arc::from(vec![1, 2, 3, 255, 4, 5, 6, 255]),
        capture_duration: Duration::ZERO,
        cpu_copy_count: 1,
    };
    let captures = [DisplayCapture {
        display: DisplayInfo {
            id: "primary".to_owned(),
            platform_id: 1,
            physical_bounds: bounds,
            work_area: bounds,
            dpi_x: 96,
            dpi_y: 96,
            scale_factor: 1.0,
            primary: true,
            rotation: DisplayRotation::Landscape,
            bits_per_pixel: 32,
        },
        frame: frame.clone(),
    }];

    assert_eq!(
        compose_captured_displays(&captures).unwrap().pixels,
        frame.pixels
    );
}

#[test]
fn save_writes_the_selected_region_as_png() {
    let directory = std::env::temp_dir().join(format!(
        "flash-shot-workflow-save-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let path = directory.join("selection.png");
    let frame = CaptureFrame {
        bounds: PhysicalRect {
            left: 0,
            top: 0,
            right: 2,
            bottom: 1,
        },
        width: 2,
        height: 1,
        stride: 8,
        format: PixelFormat::Bgra8,
        pixels: Arc::from([1, 2, 3, 255, 4, 5, 6, 255]),
        capture_duration: Duration::ZERO,
        cpu_copy_count: 1,
    };

    let document = AnnotationDocument::new(frame.bounds).unwrap();
    save_annotated_frame_selection(
        &frame,
        &document,
        PhysicalRect {
            left: 1,
            top: 0,
            right: 2,
            bottom: 1,
        },
        path.clone(),
    )
    .unwrap();

    let decoder = png::Decoder::new(BufReader::new(std::fs::File::open(&path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut output = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut output).unwrap();
    assert_eq!((info.width, info.height), (1, 1));
    assert_eq!(&output[..info.buffer_size()], &[6, 5, 4, 255]);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn annotated_save_and_quick_save_encode_the_composited_selection() {
    let directory = std::env::temp_dir().join(format!(
        "flash-shot-annotated-save-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let path = directory.join("selection.png");
    let frame = CaptureFrame {
        bounds: PhysicalRect {
            left: 0,
            top: 0,
            right: 3,
            bottom: 1,
        },
        width: 3,
        height: 1,
        stride: 12,
        format: PixelFormat::Bgra8,
        pixels: Arc::from([0, 0, 0, 255].repeat(3)),
        capture_duration: Duration::ZERO,
        cpu_copy_count: 1,
    };
    let mut document = AnnotationDocument::new(frame.bounds).unwrap();
    let mut history = CommandHistory::default();
    history
        .apply(
            &mut document,
            AnnotationCommand::Insert(Annotation {
                id: AnnotationId::new(2),
                kind: AnnotationKind::Line {
                    start: PhysicalPoint { x: 1, y: 0 },
                    end: PhysicalPoint { x: 2, y: 0 },
                },
                style: AnnotationStyle {
                    stroke_rgba: 0x00FF00FF,
                    fill_rgba: None,
                    stroke_width: 1,
                    text_font_size: 24,
                },
            }),
        )
        .unwrap();
    let selection = PhysicalRect {
        left: 1,
        top: 0,
        right: 3,
        bottom: 1,
    };

    save_annotated_frame_selection(&frame, &document, selection, path.clone()).unwrap();
    let decoder = png::Decoder::new(BufReader::new(std::fs::File::open(&path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut output = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut output).unwrap();
    assert_eq!((info.width, info.height), (2, 1));
    assert_eq!(
        &output[..info.buffer_size()],
        &[0, 255, 0, 255, 0, 255, 0, 255]
    );

    let quick = quick_save_annotated_frame_selection_in_with_prefix(
        &frame,
        &document,
        selection,
        &directory,
        "FlashShot",
        1_725_000_000_123,
    )
    .unwrap();
    assert_eq!(quick, directory.join("FlashShot-1725000000123.png"));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn export_path_defaults_to_png_and_preserves_supported_formats() {
    assert_eq!(
        export_path(PathBuf::from("capture")),
        PathBuf::from("capture.png")
    );
    assert_eq!(
        export_path(PathBuf::from("capture.jpg")),
        PathBuf::from("capture.jpg")
    );
    assert_eq!(
        export_path(PathBuf::from("capture.PNG")),
        PathBuf::from("capture.PNG")
    );
    assert_eq!(
        export_path(PathBuf::from("capture.webp")),
        PathBuf::from("capture.webp")
    );
}

#[test]
fn annotation_document_path_uses_a_json_extension() {
    assert_eq!(
        annotation_document_path(PathBuf::from("capture")),
        PathBuf::from("capture.annotations.json")
    );
    assert_eq!(
        annotation_document_path(PathBuf::from("capture.JSON")),
        PathBuf::from("capture.JSON")
    );
}

#[test]
fn annotation_document_save_writes_valid_versioned_json() {
    let directory = std::env::temp_dir().join(format!(
        "flash-shot-annotation-document-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("capture.annotations.json");
    let document = AnnotationDocument::new(PhysicalRect {
        left: 0,
        top: 0,
        right: 10,
        bottom: 10,
    })
    .unwrap();

    save_annotation_document(&document, path.clone()).unwrap();
    assert_eq!(
        AnnotationDocument::from_json(&std::fs::read_to_string(&path).unwrap()).unwrap(),
        document
    );
    assert!(!path.with_extension("json.tmp").exists());
    std::fs::write(&path, "stale annotation document").unwrap();
    save_annotation_document(&document, path.clone()).unwrap();
    assert_eq!(
        AnnotationDocument::from_json(&std::fs::read_to_string(&path).unwrap()).unwrap(),
        document
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn editable_project_saves_original_png_and_rebased_annotation_sidecar() {
    let directory = std::env::temp_dir().join(format!(
        "flash-shot-editable-project-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let image_path = directory.join("capture.png");
    let frame = CaptureFrame {
        bounds: PhysicalRect {
            left: -10,
            top: 20,
            right: -8,
            bottom: 21,
        },
        width: 2,
        height: 1,
        stride: 8,
        format: PixelFormat::Bgra8,
        pixels: Arc::from([1, 2, 3, 255, 4, 5, 6, 255]),
        capture_duration: Duration::ZERO,
        cpu_copy_count: 1,
    };
    let mut document = AnnotationDocument::new(frame.bounds).unwrap();
    let mut history = CommandHistory::default();
    history
        .apply(
            &mut document,
            AnnotationCommand::Insert(Annotation {
                id: AnnotationId::new(1),
                kind: AnnotationKind::Line {
                    start: PhysicalPoint { x: -10, y: 20 },
                    end: PhysicalPoint { x: -8, y: 20 },
                },
                style: AnnotationStyle::default(),
            }),
        )
        .unwrap();

    save_editable_project(&frame, &document, image_path.clone()).unwrap();
    let reopened = CaptureFrame::open_png(&image_path).unwrap();
    assert_eq!(reopened.bounds.left, 0);
    assert_eq!(reopened.bounds.top, 0);
    assert_eq!((reopened.width, reopened.height), (2, 1));
    let sidecar = annotation_sidecar_path(&image_path);
    let loaded = load_annotation_document(&sidecar, reopened.bounds).unwrap();
    assert_eq!(
        loaded.annotation(AnnotationId::new(1)).unwrap().bounds(),
        PhysicalRect {
            left: 0,
            top: 0,
            right: 2,
            bottom: 0,
        }
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn open_image_project_restores_a_valid_sidecar_and_tolerates_a_bad_one() {
    let directory = std::env::temp_dir().join(format!(
        "flash-shot-open-project-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let image_path = directory.join("capture.png");
    let frame = CaptureFrame {
        bounds: PhysicalRect {
            left: 0,
            top: 0,
            right: 2,
            bottom: 1,
        },
        width: 2,
        height: 1,
        stride: 8,
        format: PixelFormat::Bgra8,
        pixels: Arc::from([0, 0, 0, 255, 0, 0, 0, 255]),
        capture_duration: Duration::ZERO,
        cpu_copy_count: 1,
    };
    let document = AnnotationDocument::new(frame.bounds).unwrap();
    save_editable_project(&frame, &document, image_path.clone()).unwrap();

    let (_, _, loaded, warning) = open_image_project(&image_path).unwrap();
    assert_eq!(loaded, Some(document));
    assert_eq!(warning, None);

    std::fs::write(annotation_sidecar_path(&image_path), "not json").unwrap();
    let (_, _, loaded, warning) = open_image_project(&image_path).unwrap();
    assert_eq!(loaded, None);
    assert!(warning.unwrap().contains("could not load"));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn opening_annotation_project_requires_the_matching_png_and_sidecar_name() {
    let directory = std::env::temp_dir().join(format!(
        "flash-shot-open-annotation-project-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let image_path = directory.join("capture.png");
    let frame = CaptureFrame {
        bounds: PhysicalRect {
            left: 0,
            top: 0,
            right: 2,
            bottom: 1,
        },
        width: 2,
        height: 1,
        stride: 8,
        format: PixelFormat::Bgra8,
        pixels: Arc::from([0, 0, 0, 255, 0, 0, 0, 255]),
        capture_duration: Duration::ZERO,
        cpu_copy_count: 1,
    };
    let document = AnnotationDocument::new(frame.bounds).unwrap();
    save_editable_project(&frame, &document, image_path.clone()).unwrap();
    let sidecar = annotation_sidecar_path(&image_path);

    assert_eq!(project_image_path(&sidecar).unwrap(), image_path);
    assert!(project_image_path(&directory.join("capture.json")).is_err());
    let (opened_path, opened_frame, opened_document) = open_annotation_project(&sidecar).unwrap();
    assert_eq!(opened_path, image_path);
    assert_eq!(opened_frame.bounds, document.canvas_bounds());
    assert_eq!(opened_document, document);

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn annotation_document_load_requires_the_current_frame_canvas() {
    let directory = std::env::temp_dir().join(format!(
        "flash-shot-annotation-load-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("capture.annotations.json");
    let document = AnnotationDocument::new(PhysicalRect {
        left: 0,
        top: 0,
        right: 10,
        bottom: 10,
    })
    .unwrap();
    save_annotation_document(&document, path.clone()).unwrap();

    assert_eq!(
        load_annotation_document(&path, document.canvas_bounds()).unwrap(),
        document
    );
    assert!(
        load_annotation_document(
            &path,
            PhysicalRect {
                left: 0,
                top: 0,
                right: 11,
                bottom: 10,
            }
        )
        .is_err()
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn loaded_annotation_counters_continue_existing_ids_and_sequence_numbers() {
    let mut document = AnnotationDocument::new(PhysicalRect {
        left: 0,
        top: 0,
        right: 20,
        bottom: 20,
    })
    .unwrap();
    let mut history = CommandHistory::default();
    history
        .apply(
            &mut document,
            AnnotationCommand::Insert(Annotation {
                id: AnnotationId::new(8),
                kind: AnnotationKind::Number {
                    center: PhysicalPoint { x: 10, y: 10 },
                    value: 3,
                },
                style: AnnotationStyle::default(),
            }),
        )
        .unwrap();
    assert_eq!(next_annotation_counters(&document), (9, 4));
}

#[test]
fn quick_save_names_are_timestamped_and_do_not_overwrite_existing_files() {
    let directory = PathBuf::from("Pictures").join("Flash Shot");
    let timestamp_ms = 1_725_000_000_123_u128;
    let first = next_quick_save_path_with_prefix(&directory, "FlashShot", timestamp_ms, |_| false);

    assert_eq!(first, directory.join("FlashShot-1725000000123.png"));

    let second = next_quick_save_path_with_prefix(&directory, "FlashShot", timestamp_ms, |path| {
        path.file_name()
            .is_some_and(|name| name == "FlashShot-1725000000123.png")
    });
    assert_eq!(second, directory.join("FlashShot-1725000000123-2.png"));
}

#[test]
fn quick_save_retries_in_a_fallback_directory_after_the_selected_root_fails() {
    let root = std::env::temp_dir().join(format!(
        "flash-shot-quick-save-fallback-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let selected = root.join("selected");
    let fallback = root.join("fallback");

    let result = quick_save_with_fallback(&selected, Some(&fallback), |directory| {
        if directory == selected {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "selected root unavailable",
            ));
        }
        std::fs::create_dir_all(directory)?;
        Ok(directory.join("FlashShot-1.png"))
    })
    .unwrap();

    assert_eq!(result, fallback.join("FlashShot-1.png"));
    assert!(!selected.exists());
    assert!(fallback.is_dir());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn annotated_quick_save_fallback_writes_the_capture_to_the_recovery_root() {
    let root = std::env::temp_dir().join(format!(
        "flash-shot-annotated-fallback-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let selected = root.join("selected").join("missing\0root");
    let fallback = root.join("fallback");
    let frame = CaptureFrame {
        bounds: PhysicalRect {
            left: 0,
            top: 0,
            right: 1,
            bottom: 1,
        },
        width: 1,
        height: 1,
        stride: 4,
        format: PixelFormat::Bgra8,
        pixels: Arc::from([1, 2, 3, 255]),
        capture_duration: Duration::ZERO,
        cpu_copy_count: 1,
    };
    let document = AnnotationDocument::new(frame.bounds).unwrap();

    let path = quick_save_annotated_frame_selection_with_fallback(
        &frame,
        &document,
        frame.bounds,
        &selected,
        Some(&fallback),
        "FlashShot",
    )
    .unwrap();

    assert_eq!(path.parent(), Some(fallback.as_path()));
    assert!(path.is_file());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn quick_save_prefix_is_part_of_the_collision_resistant_name() {
    let directory = PathBuf::from("Pictures").join("Flash Shot");
    assert_eq!(
        next_quick_save_path_with_prefix(&directory, "Release_Notes", 42, |_| false),
        directory.join("Release_Notes-42.png")
    );
}

#[test]
fn configured_quick_save_prefix_is_used_for_collision_safe_paths() {
    let directory = PathBuf::from("Pictures").join("Configured Flash Shot");
    assert_eq!(
        next_quick_save_path_with_prefix(&directory, "Release_Notes", 42, |_| false),
        directory.join("Release_Notes-42.png")
    );
}

#[test]
fn recording_status_uses_ffmpeg_progress_without_exposing_process_output() {
    assert_eq!(
        format_recording_progress(
            "selected area",
            crate::recording::RecordingProgress {
                output_time_us: Some(3_900_000),
                frame: Some(117),
                finished: false,
            }
        ),
        "Recording selected area: 3s, 117 frames"
    );
}

#[test]
fn recording_stop_status_names_the_target_during_late_progress() {
    assert_eq!(
        format_recording_stopping("display"),
        "Stopping display recording..."
    );
}

#[test]
fn recording_start_failures_name_the_available_recovery_path() {
    let missing = std::io::Error::new(std::io::ErrorKind::NotFound, "ffmpeg.exe");
    assert!(recording_start_failure_status(&missing).contains("FLASH_SHOT_FFMPEG"));

    let unsupported = std::io::Error::new(std::io::ErrorKind::Unsupported, "ddagrab unavailable");
    assert!(recording_start_failure_status(&unsupported).contains("ddagrab or gdigrab"));
}

#[test]
fn overlay_recording_actions_explain_active_starting_and_stopping_conflicts() {
    assert_eq!(
        recording_start_conflict_status(true, false, false),
        Some("Stop the current recording before starting another")
    );
    assert_eq!(
        recording_start_conflict_status(false, true, false),
        Some("Screen recording startup is already in progress...")
    );
    assert_eq!(
        recording_start_conflict_status(true, false, true),
        Some("Screen recording is already stopping...")
    );
    assert_eq!(recording_start_conflict_status(false, false, false), None);
}

#[test]
fn recording_start_waits_for_source_discovery_to_finish() {
    assert_eq!(
        recording_discovery_conflict_status(true, false),
        Some("Wait for recording source discovery to finish...")
    );
    assert_eq!(
        recording_discovery_conflict_status(false, true),
        Some("Wait for recording source discovery to finish...")
    );
    assert_eq!(recording_discovery_conflict_status(false, false), None);
}

#[test]
fn stale_recording_discovery_results_cannot_replace_new_lifecycle_state() {
    assert!(recording_discovery_result_is_applicable(
        4, 4, false, false, false
    ));
    assert!(!recording_discovery_result_is_applicable(
        5, 4, false, false, false
    ));
    assert!(!recording_discovery_result_is_applicable(
        4, 4, true, false, false
    ));
    assert!(!recording_discovery_result_is_applicable(
        4, 4, false, true, false
    ));
    assert!(!recording_discovery_result_is_applicable(
        4, 4, false, false, true
    ));
}

#[test]
fn recording_support_probe_reuses_actionable_missing_ffmpeg_guidance() {
    let missing = std::io::Error::new(std::io::ErrorKind::NotFound, "ffmpeg.exe");

    assert!(recording_support_status(Err(&missing)).contains("FLASH_SHOT_FFMPEG"));
}

#[test]
fn recording_status_identifies_each_capture_target() {
    assert_eq!(
        recording_target_label(&crate::recording::RecordingTarget::Display {
            bounds: PhysicalRect {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080,
            },
        }),
        "display"
    );
    assert_eq!(
        recording_target_label(&crate::recording::RecordingTarget::Window {
            title: "Editor".to_owned(),
        }),
        "window"
    );
    assert_eq!(
        recording_target_label(&crate::recording::RecordingTarget::Region {
            bounds: PhysicalRect {
                left: 10,
                top: 10,
                right: 100,
                bottom: 100,
            },
        }),
        "selected area"
    );
}

#[test]
fn recording_audio_selection_cycles_from_auto_to_off_then_local_sources() {
    let sources = [
        AudioSource::Microphone {
            device: "USB Mic".to_owned(),
        },
        AudioSource::SystemAudio {
            device: "default".to_owned(),
        },
    ];
    let off = next_recording_audio_selection(super::RecordingAudioSelection::Automatic, &sources);
    assert_eq!(off, super::RecordingAudioSelection::Disabled);
    let microphone = next_recording_audio_selection(off, &sources);
    assert_eq!(
        microphone,
        super::RecordingAudioSelection::Source(sources[0].clone())
    );
    assert_eq!(recording_audio_selection_label(&microphone), "mic: USB Mic");
    assert_eq!(
        next_recording_audio_selection(
            super::RecordingAudioSelection::Source(sources[1].clone()),
            &sources,
        ),
        super::RecordingAudioSelection::Automatic
    );
}

#[test]
fn recording_display_selection_cycles_in_stable_primary_first_order() {
    let display = |id: &str, left, top, width, height, primary| DisplayInfo {
        id: id.to_owned(),
        platform_id: 0,
        physical_bounds: PhysicalRect {
            left,
            top,
            right: left + width,
            bottom: top + height,
        },
        work_area: PhysicalRect {
            left,
            top,
            right: left + width,
            bottom: top + height,
        },
        dpi_x: 96,
        dpi_y: 96,
        scale_factor: 1.0,
        rotation: DisplayRotation::Landscape,
        bits_per_pixel: 32,
        primary,
    };
    let displays = [
        display("secondary", -2560, -100, 2560, 1440, false),
        display("primary", 0, 0, 1920, 1080, true),
    ];
    let selected =
        next_recording_display_selection(super::RecordingDisplaySelection::Primary, &displays);
    assert_eq!(
        selected,
        super::RecordingDisplaySelection::Display {
            id: "primary".to_owned(),
            label: "1 (1920x1080)".to_owned(),
        }
    );
    let secondary = next_recording_display_selection(selected, &displays);
    assert_eq!(
        secondary,
        super::RecordingDisplaySelection::Display {
            id: "secondary".to_owned(),
            label: "2 (2560x1440)".to_owned(),
        }
    );
    assert_eq!(
        recording_display_selection_label(&secondary),
        "display 2 (2560x1440)"
    );
    assert_eq!(
        next_recording_display_selection(secondary, &displays),
        super::RecordingDisplaySelection::Primary
    );
}

#[test]
fn quick_save_writes_the_selected_png_to_the_default_style_directory() {
    let directory = std::env::temp_dir().join(format!(
        "flash-shot-quick-save-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let frame = CaptureFrame {
        bounds: PhysicalRect {
            left: 0,
            top: 0,
            right: 2,
            bottom: 1,
        },
        width: 2,
        height: 1,
        stride: 8,
        format: PixelFormat::Bgra8,
        pixels: Arc::from([1, 2, 3, 255, 4, 5, 6, 255]),
        capture_duration: Duration::ZERO,
        cpu_copy_count: 1,
    };

    let document = AnnotationDocument::new(frame.bounds).unwrap();
    let path = quick_save_annotated_frame_selection_in_with_prefix(
        &frame,
        &document,
        PhysicalRect {
            left: 1,
            top: 0,
            right: 2,
            bottom: 1,
        },
        &directory,
        "FlashShot",
        1_725_000_000_123,
    )
    .unwrap();

    assert_eq!(path, directory.join("FlashShot-1725000000123.png"));
    let decoder = png::Decoder::new(BufReader::new(std::fs::File::open(&path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut output = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut output).unwrap();
    assert_eq!((info.width, info.height), (1, 1));
    assert_eq!(&output[..info.buffer_size()], &[6, 5, 4, 255]);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn full_screen_quick_save_writes_the_entire_png_with_the_managed_name() {
    let directory = std::env::temp_dir().join(format!(
        "flash-shot-full-screen-quick-save-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let frame = CaptureFrame {
        bounds: PhysicalRect {
            left: -1,
            top: 4,
            right: 1,
            bottom: 5,
        },
        width: 2,
        height: 1,
        stride: 8,
        format: PixelFormat::Bgra8,
        pixels: Arc::from([1, 2, 3, 255, 4, 5, 6, 255]),
        capture_duration: Duration::ZERO,
        cpu_copy_count: 1,
    };

    let path = quick_save_full_screen_frame_in_with_prefix(
        &frame,
        &directory,
        "FlashShot",
        1_725_000_000_123,
    )
    .unwrap();

    assert_eq!(path, directory.join("FlashShot-1725000000123.png"));
    let decoder = png::Decoder::new(BufReader::new(std::fs::File::open(&path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut output = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut output).unwrap();
    assert_eq!((info.width, info.height), (2, 1));
    assert_eq!(&output[..info.buffer_size()], &[3, 2, 1, 255, 6, 5, 4, 255]);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn inspected_targets_are_clipped_to_the_captured_desktop() {
    assert_eq!(
        intersect_rect(
            PhysicalRect {
                left: -2200,
                top: 100,
                right: -200,
                bottom: 900,
            },
            PhysicalRect {
                left: -1920,
                top: 0,
                right: 1920,
                bottom: 1080,
            },
        ),
        Some(PhysicalRect {
            left: -1920,
            top: 100,
            right: -200,
            bottom: 900,
        })
    );
}

#[test]
fn display_window_bounds_convert_physical_pixels_with_monitor_scale() {
    let display = crate::platform::display::DisplayInfo {
        id: "secondary".to_owned(),
        platform_id: 42,
        physical_bounds: PhysicalRect {
            left: -2560,
            top: -200,
            right: 0,
            bottom: 1240,
        },
        work_area: PhysicalRect {
            left: -2560,
            top: -200,
            right: 0,
            bottom: 1200,
        },
        dpi_x: 144,
        dpi_y: 144,
        scale_factor: 1.5,
        rotation: crate::platform::display::DisplayRotation::Landscape,
        bits_per_pixel: 32,
        primary: false,
    };

    let bounds = super::display_window_bounds(&display);

    assert_eq!(f32::from(bounds.origin.x), -2560.0 / 1.5);
    assert_eq!(f32::from(bounds.origin.y), -200.0 / 1.5);
    assert_eq!(f32::from(bounds.size.width), 2560.0 / 1.5);
    assert_eq!(f32::from(bounds.size.height), 1440.0 / 1.5);
}

#[test]
fn manual_scroll_controls_prefer_space_below_the_selected_viewport() {
    let work_area = PhysicalRect {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1040,
    };
    let selection = PhysicalRect {
        left: 100,
        top: 100,
        right: 800,
        bottom: 600,
    };

    let controls = manual_scroll_control_rect(selection, work_area, 520, 136);

    assert_eq!(controls.top, 612);
    assert_eq!(controls.bottom, 748);
    assert_eq!(controls.left, 190);
    assert_eq!(controls.right, 710);
}

#[test]
fn manual_scroll_controls_move_above_a_viewport_near_the_taskbar() {
    let work_area = PhysicalRect {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1040,
    };
    let selection = PhysicalRect {
        left: 600,
        top: 700,
        right: 1300,
        bottom: 1000,
    };

    let controls = manual_scroll_control_rect(selection, work_area, 520, 136);

    assert_eq!(controls.top, 552);
    assert_eq!(controls.bottom, 688);
    assert!(controls.right <= work_area.right);
    assert!(controls.bottom <= work_area.bottom);
}

#[test]
fn manual_scroll_controls_stay_inside_the_work_area_when_selection_fills_it() {
    let work_area = PhysicalRect {
        left: -1920,
        top: 0,
        right: 0,
        bottom: 1040,
    };

    let controls = manual_scroll_control_rect(work_area, work_area, 520, 136);

    assert!(controls.left >= work_area.left);
    assert!(controls.top >= work_area.top);
    assert!(controls.right <= work_area.right);
    assert!(controls.bottom <= work_area.bottom);
}

#[test]
fn manual_scroll_control_bounds_keep_logical_size_on_scaled_displays() {
    let display = DisplayInfo {
        id: "scaled".to_owned(),
        platform_id: 8,
        physical_bounds: PhysicalRect {
            left: 0,
            top: 0,
            right: 2560,
            bottom: 1440,
        },
        work_area: PhysicalRect {
            left: 0,
            top: 0,
            right: 2560,
            bottom: 1392,
        },
        dpi_x: 144,
        dpi_y: 144,
        scale_factor: 1.5,
        rotation: DisplayRotation::Landscape,
        bits_per_pixel: 32,
        primary: true,
    };
    let selection = PhysicalRect {
        left: 300,
        top: 150,
        right: 1500,
        bottom: 900,
    };

    let bounds = manual_scroll_control_bounds(selection, &[display]).unwrap();

    assert_eq!(f32::from(bounds.origin.x), 340.0);
    assert_eq!(f32::from(bounds.origin.y), 608.0);
    assert_eq!(f32::from(bounds.size.width), 520.0);
    assert_eq!(f32::from(bounds.size.height), 176.0);
}

#[test]
fn overlay_drag_clamps_to_virtual_desktop_edges() {
    let bounds = PhysicalRect {
        left: -1920,
        top: -200,
        right: 2560,
        bottom: 1440,
    };

    assert_eq!(
        super::clamp_physical_point(PhysicalPoint { x: -3000, y: 2000 }, bounds),
        PhysicalPoint { x: -1920, y: 1440 }
    );
}

#[test]
fn focused_window_selection_clips_a_partly_offscreen_window_and_rejects_missing_targets() {
    let desktop = PhysicalRect {
        left: -1920,
        top: 0,
        right: 1920,
        bottom: 1080,
    };
    assert_eq!(
        focused_window_selection(
            Some(PhysicalRect {
                left: -2100,
                top: 100,
                right: 400,
                bottom: 900,
            }),
            desktop,
        ),
        Some(PhysicalRect {
            left: -1920,
            top: 100,
            right: 400,
            bottom: 900,
        })
    );
    assert_eq!(focused_window_selection(None, desktop), None);
}

#[test]
fn click_jitter_uses_smart_target_but_drag_keeps_free_selection() {
    let target = InspectionTarget {
        bounds: PhysicalRect {
            left: 100,
            top: 100,
            right: 500,
            bottom: 400,
        },
        kind: InspectionKind::Control,
    };
    assert_eq!(
        resolve_pointer_selection(
            PhysicalRect {
                left: 200,
                top: 200,
                right: 202,
                bottom: 201,
            },
            Some(target),
        ),
        Some(target.bounds)
    );

    let drag = PhysicalRect {
        left: 200,
        top: 200,
        right: 240,
        bottom: 260,
    };
    assert_eq!(resolve_pointer_selection(drag, Some(target)), Some(drag));
}

#[test]
fn smart_target_status_includes_target_kind_bounds_and_pixel_details() {
    let target = InspectionTarget {
        bounds: PhysicalRect {
            left: -200,
            top: 50,
            right: 300,
            bottom: 250,
        },
        kind: InspectionKind::Control,
    };

    assert_eq!(
        smart_target_status(target, PhysicalPoint { x: 12, y: 34 }, "#AABBCC".to_owned()),
        "Control: 500 x 200 px | (12, 34) #AABBCC"
    );
}

#[test]
fn stale_background_completion_is_ignored_after_a_new_operation_starts() {
    assert!(is_current_operation(4, 4));
    assert!(!is_current_operation(5, 4));
}

#[test]
fn cancelling_scroll_advances_the_operation_generation() {
    let cancelled_generation = super::scrolling::next_operation_generation(4);

    assert_eq!(cancelled_generation, 5);
    assert!(!is_current_operation(cancelled_generation, 4));
}

#[test]
fn translation_failure_messages_identify_the_recovery_step() {
    assert!(
        translation_failure_status(&TranslationOutcome::OcrUnavailable)
            .contains("Install Tesseract")
    );
    assert!(
        translation_failure_status(&TranslationOutcome::OcrFailed("bad image".to_owned()))
            .contains("recognize text")
    );
    assert!(
        translation_failure_status(&TranslationOutcome::ServiceFailed("timeout".to_owned()))
            .contains("Check the endpoint")
    );
}

#[test]
fn recognition_requests_report_overlapping_work_without_replacing_the_first_task() {
    assert_eq!(recognition_start_conflict_status(false), None);
    assert_eq!(
        recognition_start_conflict_status(true),
        Some("Recognition is already in progress")
    );
}

#[test]
fn translation_support_status_keeps_disabled_configuration_local_and_actionable() {
    assert!(translation_support_status(Ok(None)).contains("FLASH_SHOT_TRANSLATION_ENDPOINT"));

    let invalid = std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "translation endpoint must use HTTPS",
    );
    assert!(translation_support_status(Err(invalid)).contains("needs attention"));
}

#[test]
fn translation_service_test_status_reports_readiness_without_returning_text() {
    let success = Ok("  Bonjour  ".to_owned());
    assert_eq!(
        translation_service_test_status(&success),
        "Translation service ready (7 characters)"
    );

    let empty = Ok(String::new());
    assert!(translation_service_test_status(&empty).contains("returned no text"));

    let failure = Err(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "request timed out",
    ));
    assert!(translation_service_test_status(&failure).contains("Check the endpoint"));
}

#[test]
fn ocr_language_labels_make_each_saved_preset_readable() {
    assert_eq!(ocr_language_label(None), "automatic");
    assert_eq!(ocr_language_label(Some("eng")), "English");
    assert_eq!(ocr_language_label(Some("chi_sim")), "Simplified Chinese");
    assert_eq!(
        ocr_language_label(Some("eng+chi_sim")),
        "English + Simplified Chinese"
    );
    assert_eq!(ocr_language_label(Some("unknown")), "automatic");
}

#[test]
fn ocr_support_probe_names_the_local_installation_recovery_step() {
    let missing = std::io::Error::new(std::io::ErrorKind::NotFound, "tesseract.exe");

    assert!(ocr_support_status(Err(&missing)).contains("FLASH_SHOT_TESSERACT"));
}
