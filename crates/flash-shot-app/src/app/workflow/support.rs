//! Pure workflow state, formatting, and interaction helpers.

use super::*;
use crate::i18n::{Locale, UiText};

pub(in crate::app) fn tool_selected_status(tool: AnnotationTool) -> &'static str {
    match tool {
        AnnotationTool::Text => "Text tool selected",
        AnnotationTool::Watermark => "Watermark tool selected",
        AnnotationTool::Number => "Number tool selected",
        AnnotationTool::Blur => "Blur tool selected",
        AnnotationTool::Mosaic => "Mosaic tool selected",
        AnnotationTool::Highlight => "Highlight tool selected",
        AnnotationTool::Rectangle => "Rectangle tool selected",
        AnnotationTool::Ellipse => "Ellipse tool selected",
        AnnotationTool::Line => "Line tool selected",
        AnnotationTool::Arrow => "Arrow tool selected",
        AnnotationTool::Freehand => "Freehand tool selected",
    }
}

pub(in crate::app) fn drawing_status(tool: AnnotationTool) -> &'static str {
    match tool {
        AnnotationTool::Text => "Editing text...",
        AnnotationTool::Watermark => "Placing watermark...",
        AnnotationTool::Number => "Placing number...",
        AnnotationTool::Blur => "Drawing blur...",
        AnnotationTool::Mosaic => "Drawing mosaic...",
        AnnotationTool::Highlight => "Drawing highlight...",
        AnnotationTool::Rectangle => "Drawing rectangle...",
        AnnotationTool::Ellipse => "Drawing ellipse...",
        AnnotationTool::Line => "Drawing line...",
        AnnotationTool::Arrow => "Drawing arrow...",
        AnnotationTool::Freehand => "Drawing freehand...",
    }
}

pub(in crate::app) fn annotation_added_status(tool: Option<AnnotationTool>) -> &'static str {
    match tool {
        Some(AnnotationTool::Text) => "Text added",
        Some(AnnotationTool::Watermark) => "Watermark added",
        Some(AnnotationTool::Number) => "Number added",
        Some(AnnotationTool::Blur) => "Blur added",
        Some(AnnotationTool::Mosaic) => "Mosaic added",
        Some(AnnotationTool::Highlight) => "Highlight added",
        Some(AnnotationTool::Rectangle) => "Rectangle added",
        Some(AnnotationTool::Ellipse) => "Ellipse added",
        Some(AnnotationTool::Line) => "Line added",
        Some(AnnotationTool::Arrow) => "Arrow added",
        Some(AnnotationTool::Freehand) => "Freehand stroke added",
        _ => "Annotation added",
    }
}

pub(in crate::app) fn annotation_cancelled_status(tool: Option<AnnotationTool>) -> &'static str {
    match tool {
        Some(AnnotationTool::Text) => "Text cancelled",
        Some(AnnotationTool::Watermark) => "Watermark cancelled",
        Some(AnnotationTool::Number) => "Number cancelled",
        Some(AnnotationTool::Blur) => "Blur cancelled",
        Some(AnnotationTool::Mosaic) => "Mosaic cancelled",
        Some(AnnotationTool::Highlight) => "Highlight cancelled",
        Some(AnnotationTool::Rectangle) => "Rectangle cancelled",
        Some(AnnotationTool::Ellipse) => "Ellipse cancelled",
        Some(AnnotationTool::Line) => "Line cancelled",
        Some(AnnotationTool::Arrow) => "Arrow cancelled",
        Some(AnnotationTool::Freehand) => "Freehand stroke cancelled",
        _ => "Annotation cancelled",
    }
}

pub(in crate::app) fn is_current_operation(current: u64, completed: u64) -> bool {
    current == completed
}

/// Releases the completed task's slot and accepts its result only while its workflow is current.
/// A superseded completion must still clear its own slot or future capture requests stay blocked.
pub(in crate::app) fn claim_idle_completion(
    active_generation: &mut Option<u64>,
    current_generation: u64,
    completion_generation: u64,
    session_state: CaptureSessionState,
) -> bool {
    if *active_generation != Some(completion_generation) {
        return false;
    }
    *active_generation = None;
    is_current_operation(current_generation, completion_generation)
        && session_state == CaptureSessionState::Idle
}

pub(in crate::app) fn next_history_limit(current: u16) -> u16 {
    match current {
        10 => 30,
        30 => 100,
        100 => 300,
        _ => 10,
    }
}

pub(in crate::app) fn delayed_capture_status(remaining_seconds: u8) -> String {
    format!("Capture scheduled in {remaining_seconds} seconds")
}

pub(in crate::app) fn clamp_physical_point(
    point: crate::domain::geometry::PhysicalPoint,
    bounds: PhysicalRect,
) -> crate::domain::geometry::PhysicalPoint {
    crate::domain::geometry::PhysicalPoint {
        x: point.x.clamp(bounds.left, bounds.right),
        y: point.y.clamp(bounds.top, bounds.bottom),
    }
}

pub(in crate::app) fn utf16_offset(text: &str, byte_offset: usize) -> usize {
    text[..byte_offset].chars().map(char::len_utf16).sum()
}

pub(in crate::app) fn byte_offset(text: &str, utf16_offset: usize) -> usize {
    let mut bytes = 0;
    let mut units = 0;
    for character in text.chars() {
        if units >= utf16_offset {
            break;
        }
        units += character.len_utf16();
        bytes += character.len_utf8();
    }
    bytes
}

pub(in crate::app) fn range_to_utf16(text: &str, range: &Range<usize>) -> Range<usize> {
    utf16_offset(text, range.start)..utf16_offset(text, range.end)
}

pub(in crate::app) fn range_from_utf16(text: &str, range: &Range<usize>) -> Range<usize> {
    byte_offset(text, range.start)..byte_offset(text, range.end)
}

pub(in crate::app) fn previous_char_boundary(text: &str, offset: usize) -> usize {
    text.char_indices()
        .rev()
        .find_map(|(index, _)| (index < offset).then_some(index))
        .unwrap_or(0)
}

pub(in crate::app) fn next_char_boundary(text: &str, offset: usize) -> usize {
    text.char_indices()
        .find_map(|(index, _)| (index > offset).then_some(index))
        .unwrap_or(text.len())
}

pub(in crate::app) fn selection_status(selection: PhysicalRect) -> String {
    format!(
        "Selection: {} x {} physical pixels",
        selection.width(),
        selection.height()
    )
}

pub(in crate::app) fn smart_target_status(
    target: InspectionTarget,
    point: PhysicalPoint,
    color: String,
) -> String {
    let kind = match target.kind {
        InspectionKind::Control => "Control",
        InspectionKind::Window => "Window",
    };
    format!(
        "{kind}: {} x {} px | ({}, {}) {color}",
        target.bounds.width(),
        target.bounds.height(),
        point.x,
        point.y,
    )
}

pub(in crate::app) fn fill_color(stroke_rgba: u32) -> u32 {
    with_alpha(stroke_rgba, fill_alpha(stroke_rgba as u8))
}

pub(in crate::app) fn pinned_size(image_width: f32, image_height: f32) -> gpui::Size<Pixels> {
    let width = image_width.max(1.0);
    let height = image_height.max(1.0);
    size(px(width), px(height))
}

pub(in crate::app) fn with_alpha(color: u32, alpha: u8) -> u32 {
    (color & 0xFFFFFF00) | u32::from(alpha)
}

pub(in crate::app) fn fill_alpha(stroke_alpha: u8) -> u8 {
    (u16::from(stroke_alpha) * 0x66 / 255) as u8
}

pub(in crate::app) fn style_for_tool(
    tool: AnnotationTool,
    style: crate::domain::annotation::AnnotationStyle,
) -> crate::domain::annotation::AnnotationStyle {
    if tool == AnnotationTool::Highlight {
        crate::domain::annotation::AnnotationStyle {
            stroke_rgba: fill_color(style.stroke_rgba),
            fill_rgba: None,
            stroke_width: 1,
            text_font_size: style.text_font_size,
        }
    } else {
        style
    }
}

pub(in crate::app) fn text_annotation_with_content(
    annotation: Annotation,
    content: String,
) -> Option<Annotation> {
    let content = crate::domain::annotation::normalized_text_annotation_content(&content);
    let kind = match annotation.kind {
        AnnotationKind::Text { origin, .. } => AnnotationKind::Text { origin, content },
        AnnotationKind::Watermark { origin, .. } => AnnotationKind::Watermark { origin, content },
        _ => return None,
    };
    Some(Annotation {
        id: annotation.id,
        kind,
        style: annotation.style,
    })
}

pub(in crate::app) fn intersect_rect(
    left: PhysicalRect,
    right: PhysicalRect,
) -> Option<PhysicalRect> {
    let intersection = PhysicalRect {
        left: left.left.max(right.left),
        top: left.top.max(right.top),
        right: left.right.min(right.right),
        bottom: left.bottom.min(right.bottom),
    };
    (intersection.width() > 0 && intersection.height() > 0).then_some(intersection)
}

pub(in crate::app) fn resolve_pointer_selection(
    dragged: PhysicalRect,
    smart_target: Option<InspectionTarget>,
) -> Option<PhysicalRect> {
    const CLICK_TOLERANCE: u32 = 3;
    if dragged.width() <= CLICK_TOLERANCE && dragged.height() <= CLICK_TOLERANCE {
        smart_target.map(|target| target.bounds)
    } else if dragged.width() > 0 && dragged.height() > 0 {
        Some(dragged)
    } else {
        None
    }
}

/// Formats the exact RGB value at the active overlay pointer, if it still belongs to this frame.
pub(in crate::app) fn hovered_color(
    frame: Option<&CaptureFrame>,
    hover_pixel: Option<PhysicalPoint>,
    format: ColorFormat,
) -> Option<String> {
    frame?
        .pixel_at(hover_pixel?)
        .map(|color| format.format(color))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::app) enum ColorFormat {
    Hex,
    Rgb,
    Hsl,
}

impl ColorFormat {
    pub(in crate::app) const fn from_setting(value: u8) -> Self {
        match value {
            1 => Self::Rgb,
            2 => Self::Hsl,
            _ => Self::Hex,
        }
    }

    pub(in crate::app) const fn setting_value(self) -> u8 {
        match self {
            Self::Hex => 0,
            Self::Rgb => 1,
            Self::Hsl => 2,
        }
    }

    pub(in crate::app) const fn next(self) -> Self {
        match self {
            Self::Hex => Self::Rgb,
            Self::Rgb => Self::Hsl,
            Self::Hsl => Self::Hex,
        }
    }

    pub(in crate::app) const fn label(self) -> &'static str {
        match self {
            Self::Hex => "HEX",
            Self::Rgb => "RGB",
            Self::Hsl => "HSL",
        }
    }

    pub(in crate::app) fn format(self, color: crate::platform::capture::PixelColor) -> String {
        match self {
            Self::Hex => color.hex_rgb(),
            Self::Rgb => format!("rgb({}, {}, {})", color.red, color.green, color.blue),
            Self::Hsl => format_hsl(color.red, color.green, color.blue),
        }
    }
}

pub(in crate::app) fn format_hsl(red: u8, green: u8, blue: u8) -> String {
    let red = f32::from(red) / 255.0;
    let green = f32::from(green) / 255.0;
    let blue = f32::from(blue) / 255.0;
    let minimum = red.min(green).min(blue);
    let maximum = red.max(green).max(blue);
    let lightness = (minimum + maximum) / 2.0;
    let delta = maximum - minimum;
    if delta == 0.0 {
        return format!("hsl(0, 0%, {:.1}%)", lightness * 100.0);
    }
    let saturation = delta / (1.0 - (2.0 * lightness - 1.0).abs());
    let hue = if maximum == red {
        60.0 * ((green - blue) / delta).rem_euclid(6.0)
    } else if maximum == green {
        60.0 * ((blue - red) / delta + 2.0)
    } else {
        60.0 * ((red - green) / delta + 4.0)
    };
    format!(
        "hsl({hue:.1}, {:.1}%, {:.1}%)",
        saturation * 100.0,
        lightness * 100.0
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::app) enum KeyboardCommand {
    Undo,
    Redo,
    Duplicate,
    BringForward,
    SendBackward,
    RotateClockwise,
    SelectNextAnnotation,
    SelectPreviousAnnotation,
    Delete,
    Cancel,
    Copy,
    Save,
    QuickSave,
    CopyColor,
    MoveColorCursor(i32, i32),
    Nudge(i32, i32),
    SelectTool(Option<AnnotationTool>),
}

pub(in crate::app) enum SaveOutcome {
    Saved { path: PathBuf, managed: bool },
    Cancelled,
    Failed(String),
}

pub(in crate::app) enum OpenImageOutcome {
    Opened {
        path: PathBuf,
        frame: CaptureFrame,
        preview: Arc<RenderImage>,
        document: Option<AnnotationDocument>,
        document_warning: Option<String>,
    },
    Cancelled,
    Failed(String),
}

/// Separates local OCR and remote translation failures so the overlay can suggest the right fix.
pub(in crate::app) enum TranslationOutcome {
    Completed(String),
    PreparationFailed(String),
    OcrUnavailable,
    OcrFailed(String),
    ServiceFailed(String),
}

/// Runs the selection pipeline outside the UI thread while retaining the failure stage.
pub(in crate::app) fn translate_selected_frame(
    frame: CaptureFrame,
    document: AnnotationDocument,
    selection: PhysicalRect,
    config: crate::translation::TranslationConfig,
    ocr_language: Option<String>,
) -> TranslationOutcome {
    let frame = match frame
        .composite_annotations(&document)
        .and_then(|frame| frame.crop(selection))
    {
        Ok(frame) => frame,
        Err(error) => return TranslationOutcome::PreparationFailed(error.to_string()),
    };
    let text = match crate::ocr::recognize_with_language(&frame, ocr_language.as_deref()) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return TranslationOutcome::OcrUnavailable;
        }
        Err(error) => return TranslationOutcome::OcrFailed(error.to_string()),
    };
    match crate::translation::translate(&config, &text) {
        Ok(text) => TranslationOutcome::Completed(text),
        Err(error) => TranslationOutcome::ServiceFailed(error.to_string()),
    }
}

/// Formats the persisted OCR choice so status messages never expose a raw optional value.
pub(in crate::app) fn ocr_language_label(locale: Locale, language: Option<&str>) -> &'static str {
    let key = match language {
        None => UiText::OcrLanguageAutomatic,
        Some("eng") => UiText::OcrLanguageEnglish,
        Some("chi_sim") => UiText::OcrLanguageSimplifiedChinese,
        Some("eng+chi_sim") => UiText::OcrLanguageEnglishSimplifiedChinese,
        Some(_) => UiText::OcrLanguageAutomatic,
    };
    locale.text(key)
}

/// Turns a local OCR probe into a concise readiness result with a concrete recovery action.
pub(in crate::app) fn ocr_support_status(
    locale: Locale,
    result: Result<&crate::ocr::OcrSupport, &std::io::Error>,
) -> String {
    match result {
        Ok(support) if support.language_available() => locale.format_template(
            UiText::OcrSupportReady,
            &[
                ("version", support.version()),
                (
                    "language",
                    ocr_language_label(locale, Some(support.language())),
                ),
            ],
        ),
        Ok(support) => locale.format_template(
            UiText::OcrSupportLanguageMissing,
            &[(
                "language",
                ocr_language_label(locale, Some(support.language())),
            )],
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => locale.format_template(
            UiText::OcrSupportUnavailable,
            &[("error", &error.to_string())],
        ),
        Err(error) => locale.format_template(
            UiText::OcrSupportCheckFailed,
            &[("error", &error.to_string())],
        ),
    }
}

/// Describes only local translation configuration so checking support never contacts a service.
pub(in crate::app) fn translation_support_status(
    locale: Locale,
    result: std::io::Result<Option<crate::translation::TranslationConfig>>,
) -> String {
    match result {
        Ok(Some(config)) => locale.format_template(
            UiText::TranslationSupportReady,
            &[("language", config.target_language())],
        ),
        Ok(None) => locale.text(UiText::TranslationSupportDisabled).to_owned(),
        Err(error) => locale.format_template(
            UiText::TranslationSupportNeedsAttention,
            &[("error", &error.to_string())],
        ),
    }
}

/// Turns an explicit fixed-text service probe into a safe status message without exposing the
/// returned translation. The settings action only sends the phrase "Flash Shot" and reports its
/// character count, so users can verify connectivity without putting screenshot text on the wire.
pub(in crate::app) fn translation_service_test_status(
    locale: Locale,
    result: &std::io::Result<String>,
) -> String {
    match result {
        Ok(text) if !text.trim().is_empty() => locale.format_template(
            UiText::TranslationServiceReady,
            &[("count", &text.trim().chars().count().to_string())],
        ),
        Ok(_) => locale.text(UiText::TranslationServiceNoText).to_owned(),
        Err(error) => translation_failure_status(
            locale,
            &TranslationOutcome::ServiceFailed(error.to_string()),
        ),
    }
}

/// Turns each translation-stage failure into a recovery action instead of a generic error.
pub(in crate::app) fn translation_failure_status(
    locale: Locale,
    outcome: &TranslationOutcome,
) -> String {
    match outcome {
        TranslationOutcome::PreparationFailed(error) => {
            locale.format_template(UiText::TranslationPreparationFailed, &[("error", error)])
        }
        TranslationOutcome::OcrUnavailable => {
            locale.text(UiText::RecognitionOcrUnavailable).to_owned()
        }
        TranslationOutcome::OcrFailed(error) => {
            locale.format_template(UiText::TranslationOcrFailed, &[("error", error)])
        }
        TranslationOutcome::ServiceFailed(error) => {
            locale.format_template(UiText::TranslationServiceFailed, &[("error", error)])
        }
        TranslationOutcome::Completed(_) => String::new(),
    }
}

pub(in crate::app) fn keyboard_command(keystroke: &Keystroke) -> Option<KeyboardCommand> {
    let modifiers = keystroke.modifiers;
    if modifiers.secondary()
        && !modifiers.alt
        && !modifiers.platform
        && !modifiers.function
        && keystroke.key == "z"
    {
        return Some(if modifiers.shift {
            KeyboardCommand::Redo
        } else {
            KeyboardCommand::Undo
        });
    }
    if modifiers.secondary()
        && !modifiers.alt
        && !modifiers.platform
        && !modifiers.function
        && keystroke.key == "d"
    {
        return Some(KeyboardCommand::Duplicate);
    }
    if modifiers.secondary()
        && modifiers.shift
        && !modifiers.alt
        && !modifiers.platform
        && !modifiers.function
        && keystroke.key == "]"
    {
        return Some(KeyboardCommand::BringForward);
    }
    if modifiers.secondary()
        && modifiers.shift
        && !modifiers.alt
        && !modifiers.platform
        && !modifiers.function
        && keystroke.key == "["
    {
        return Some(KeyboardCommand::SendBackward);
    }
    if modifiers.secondary()
        && modifiers.shift
        && !modifiers.alt
        && !modifiers.platform
        && !modifiers.function
        && keystroke.key == "r"
    {
        return Some(KeyboardCommand::RotateClockwise);
    }
    // Keep the familiar standard-save shortcut separate from Shift+Enter's
    // quick-save path, which intentionally skips the native destination dialog.
    if modifiers.control
        && !modifiers.shift
        && !modifiers.alt
        && !modifiers.platform
        && !modifiers.function
        && keystroke.key == "s"
    {
        return Some(KeyboardCommand::Save);
    }
    if modifiers.control && !modifiers.alt && !modifiers.platform && !modifiers.function {
        return match keystroke.key.as_str() {
            "left" => Some(KeyboardCommand::MoveColorCursor(-1, 0)),
            "right" => Some(KeyboardCommand::MoveColorCursor(1, 0)),
            "up" => Some(KeyboardCommand::MoveColorCursor(0, -1)),
            "down" => Some(KeyboardCommand::MoveColorCursor(0, 1)),
            _ => None,
        };
    }
    if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
        return None;
    }
    match keystroke.key.as_str() {
        "a" => Some(KeyboardCommand::SelectTool(Some(AnnotationTool::Arrow))),
        "b" => Some(KeyboardCommand::SelectTool(Some(AnnotationTool::Blur))),
        "c" => Some(KeyboardCommand::CopyColor),
        "e" => Some(KeyboardCommand::SelectTool(Some(AnnotationTool::Ellipse))),
        "h" => Some(KeyboardCommand::SelectTool(Some(AnnotationTool::Highlight))),
        "l" => Some(KeyboardCommand::SelectTool(Some(AnnotationTool::Line))),
        "m" => Some(KeyboardCommand::SelectTool(Some(AnnotationTool::Mosaic))),
        "n" => Some(KeyboardCommand::SelectTool(Some(AnnotationTool::Number))),
        "p" => Some(KeyboardCommand::SelectTool(Some(AnnotationTool::Freehand))),
        "r" => Some(KeyboardCommand::SelectTool(Some(AnnotationTool::Rectangle))),
        "s" => Some(KeyboardCommand::SelectTool(None)),
        "t" => Some(KeyboardCommand::SelectTool(Some(AnnotationTool::Text))),
        "w" => Some(KeyboardCommand::SelectTool(Some(AnnotationTool::Watermark))),
        "tab" if modifiers.shift => Some(KeyboardCommand::SelectPreviousAnnotation),
        "tab" => Some(KeyboardCommand::SelectNextAnnotation),
        "delete" | "backspace" if !modifiers.shift => Some(KeyboardCommand::Delete),
        "escape" if !modifiers.shift => Some(KeyboardCommand::Cancel),
        "enter" if !modifiers.shift => Some(KeyboardCommand::Copy),
        "enter" if modifiers.shift => Some(KeyboardCommand::QuickSave),
        "left" => Some(KeyboardCommand::Nudge(
            if modifiers.shift { -10 } else { -1 },
            0,
        )),
        "right" => Some(KeyboardCommand::Nudge(
            if modifiers.shift { 10 } else { 1 },
            0,
        )),
        "up" => Some(KeyboardCommand::Nudge(
            0,
            if modifiers.shift { -10 } else { -1 },
        )),
        "down" => Some(KeyboardCommand::Nudge(
            0,
            if modifiers.shift { 10 } else { 1 },
        )),
        _ => None,
    }
}

pub(in crate::app) fn next_annotation_selection(
    annotations: &[AnnotationId],
    selected: Option<AnnotationId>,
    reverse: bool,
) -> Option<AnnotationId> {
    let len = annotations.len();
    let current = selected.and_then(|id| annotations.iter().position(|candidate| *candidate == id));
    let index = match (current, reverse) {
        (Some(index), false) => (index + 1) % len,
        (Some(0), true) => len - 1,
        (Some(index), true) => index - 1,
        (None, false) => 0,
        (None, true) => len - 1,
    };
    annotations.get(index).copied()
}

pub(in crate::app) fn annotation_position(
    annotations: &[AnnotationId],
    selected: AnnotationId,
) -> usize {
    annotations
        .iter()
        .position(|candidate| *candidate == selected)
        .map_or(0, |index| index + 1)
}

pub(in crate::app) fn adjusted_number_value(value: u32, delta: i32) -> u32 {
    i64::from(value)
        .saturating_add(i64::from(delta))
        .clamp(1, i64::from(u32::MAX)) as u32
}
