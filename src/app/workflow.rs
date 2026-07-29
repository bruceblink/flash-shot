//! Capture, selection, and clipboard workflow orchestration.

mod annotation;
mod capture;
mod exporting;
mod file_io;
mod images;
mod pinning;
mod recognition;
mod recording;
mod scrolling;
mod settings;
mod windowing;

use file_io::*;
use windowing::*;

#[cfg(test)]
use recording::{
    format_recording_progress, next_recording_audio_selection, next_recording_display_selection,
    recording_start_failure_status, recording_support_status, recording_target_label,
};
pub(super) use recording::{recording_audio_selection_label, recording_display_selection_label};

use std::{
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use gpui::{
    AppContext, AsyncApp, Bounds, Context, DisplayId, Focusable, KeyDownEvent, Keystroke,
    PathPromptOptions, Pixels, RenderImage, WeakEntity, WindowBackgroundAppearance, WindowBounds,
    WindowKind, WindowOptions, point, px, size,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::{
    FlashShotApp, HistoryFilter, RecognitionResult, RecordingAudioSelection,
    RecordingDisplaySelection, SettingsSection,
    overlay::CaptureOverlay,
    pinned::PinnedImage,
    render_image::{history_thumbnail_frame, render_image_from_capture},
    scroll_control::ManualScrollControl,
};
use crate::{
    domain::{
        annotation::{
            Annotation, AnnotationCommand, AnnotationDocument, AnnotationId, AnnotationKind,
            AnnotationTool,
        },
        geometry::{PhysicalPoint, PhysicalRect},
        session::CaptureSessionState,
    },
    performance::CapturePipelineSample,
    platform::{
        autostart::{AutoStartService, AutoStartState, SystemAutoStart},
        capture::{
            CaptureBackend, CaptureFrame, CaptureOptions, DisplayCapture, SystemCaptureBackend,
            capture_displays_with_options, compose_virtual_desktop,
        },
        clipboard::{ClipboardService, SystemClipboard},
        directory,
        display::{DisplayProvider, SystemDisplayProvider},
        shortcut::GlobalShortcutService,
        window_inspector::{
            InspectionKind, InspectionTarget, SystemWindowInspector, WindowInspector,
        },
        window_visibility,
    },
    recording::{
        AudioSource, RecordingAudioConfig, RecordingEvent, RecordingProgress, RecordingRequest,
        RecordingTarget, discover, discover_audio_sources, start_recording,
    },
    settings::UserSettings,
    update::{UpdateAvailability, UpdateConfig},
};

impl FlashShotApp {
    /// Persists one of the supported delayed-capture choices selected in settings.
    pub(super) fn set_capture_delay(&mut self, delay_seconds: u8, cx: &mut Context<Self>) {
        let next_delay = UserSettings::normalize_capture_delay(delay_seconds);
        if next_delay != delay_seconds || self.capture_delay_seconds == next_delay {
            return;
        }
        let previous_delay = self.capture_delay_seconds;
        self.capture_delay_seconds = next_delay;
        self.settings.capture_delay_seconds = next_delay;
        if let Err(error) = self.settings.save(&self.settings_path) {
            self.capture_delay_seconds = previous_delay;
            self.settings.capture_delay_seconds = previous_delay;
            self.status = format!("Could not save capture delay: {error}");
            cx.notify();
            return;
        }
        self.status = if self.capture_delay_seconds == 0 {
            "Capture delay disabled".to_owned()
        } else {
            format!(
                "Capture delay set to {} seconds",
                self.capture_delay_seconds
            )
        };
        cx.notify();
    }

    pub(super) fn toggle_capture_cursor(&mut self, cx: &mut Context<Self>) {
        let previous_value = self.include_cursor;
        self.include_cursor = !previous_value;
        self.settings.include_cursor = self.include_cursor;
        if let Err(error) = self.settings.save(&self.settings_path) {
            self.include_cursor = previous_value;
            self.settings.include_cursor = previous_value;
            self.status = format!("Could not save cursor preference: {error}");
            cx.notify();
            return;
        }
        self.set_tray_capture_cursor_enabled(self.include_cursor);
        self.status = if self.include_cursor {
            "Capture will include the system cursor".to_owned()
        } else {
            "Capture will omit the system cursor".to_owned()
        };
        cx.notify();
    }

    pub(super) fn cycle_history_limit(&mut self, cx: &mut Context<Self>) {
        if self.history_retention_target.is_some()
            || self.history_clear_in_flight
            || self.history_clear_confirmation
            || !self.history_deletions_in_flight.is_empty()
        {
            return;
        }
        let next_limit = next_history_limit(self.settings.history_limit);
        self.history_retention_target = Some(next_limit);
        self.status = format!("Updating screenshot history retention to {next_limit}...");
        cx.notify();
        self.continue_history_retention(cx);
    }

    fn continue_history_retention(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.history_retention_target else {
            return;
        };
        let candidates = self.history.retention_candidates(usize::from(target));
        if candidates.is_empty() {
            self.finish_history_retention(cx);
            return;
        }
        let snapshot = self.history.clone();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let deletion = cx
                    .background_executor()
                    .spawn(async move { snapshot.delete_managed_paths(candidates) })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.finish_history_retention_deletion(deletion, cx)
                    });
                }
            }
        })
        .detach();
    }

    fn finish_history_retention_deletion(
        &mut self,
        deletion: crate::history::HistoryFileDeletion,
        cx: &mut Context<Self>,
    ) {
        if self.history_retention_target.is_none() {
            return;
        }
        for (path, error) in &deletion.failures {
            log::warn!(target: "flash_shot::history", "history_retention_delete_failed path={} error={error}", path.display());
        }
        if let Err(error) = self.history.forget_deleted(&deletion.deleted) {
            self.history_retention_target = None;
            self.status = format!("Could not update screenshot history index: {error}");
            cx.notify();
            return;
        }
        self.synchronize_history_preview_cache();
        if !deletion.failures.is_empty() {
            let failure_count = deletion.failures.len();
            self.history_retention_target = None;
            self.status = format!(
                "Could not remove {failure_count} capture(s); history retention was unchanged"
            );
            cx.notify();
            return;
        }
        self.continue_history_retention(cx);
    }

    fn finish_history_retention(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.history_retention_target.take() else {
            return;
        };
        if let Err(error) = self.history.set_limit_after_prune(usize::from(target)) {
            self.status = format!("Could not update screenshot history retention: {error}");
            cx.notify();
            return;
        }
        self.settings.history_limit = target;
        self.status = match self.settings.save(&self.settings_path) {
            Ok(()) => format!("Screenshot history retains the latest {target} captures"),
            Err(error) => format!(
                "History retention is {target} captures for this session but could not be saved: {error}"
            ),
        };
        cx.notify();
    }

    pub(super) fn copy_recognition_result(&mut self, cx: &mut Context<Self>) {
        let Some(result) = self.recognition_result.as_ref() else {
            return;
        };
        self.status = match SystemClipboard.copy_text(&result.text) {
            Ok(()) => format!("{} copied to clipboard", result.title),
            Err(error) => format!("Could not copy {}: {error}", result.title),
        };
        cx.notify();
    }

    /// Copies the current physical-pixel sample without changing the capture or annotation state.
    pub(super) fn copy_hover_color(&mut self, cx: &mut Context<Self>) {
        let format = ColorFormat::from_setting(self.settings.color_format);
        let Some(color) = hovered_color(self.frame.as_ref(), self.hover_pixel, format) else {
            self.status = "Move over the captured image to copy a color".to_owned();
            cx.notify();
            return;
        };
        self.status = match SystemClipboard.copy_text(&color) {
            Ok(()) => format!("{color} copied to clipboard"),
            Err(error) => format!("Could not copy {color}: {error}"),
        };
        cx.notify();
    }

    /// Cycles the saved output syntax used by the overlay's pixel color copy action.
    pub(super) fn cycle_color_format(&mut self, cx: &mut Context<Self>) {
        let previous = self.settings.color_format;
        let next = ColorFormat::from_setting(previous).next();
        self.settings.color_format = next.setting_value();
        if let Err(error) = self.settings.save(&self.settings_path) {
            self.settings.color_format = previous;
            self.status = format!("Could not save color format preference: {error}");
            cx.notify();
            return;
        }
        self.status = format!("Color copy format: {}", next.label());
        cx.notify();
    }

    pub(super) fn color_format_label(&self) -> &'static str {
        ColorFormat::from_setting(self.settings.color_format).label()
    }

    pub(super) fn clear_recognition_result(&mut self, cx: &mut Context<Self>) {
        self.recognition_result = None;
        cx.notify();
    }

    pub(super) fn show_settings_window(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.settings_window_handle
            && let Err(error) = window_visibility::restore(handle)
        {
            self.status = format!("Could not open settings: {error}");
            log::warn!(target: "flash_shot::settings", "settings_window_restore_failed error={error}");
        }
        cx.notify();
    }

    pub(super) fn show_history_window(&mut self, cx: &mut Context<Self>) {
        self.select_settings_section(SettingsSection::Files, cx);
        self.show_settings_window(cx);
    }

    pub(super) fn open_history_directory(&mut self, cx: &mut Context<Self>) {
        self.status = match crate::history::managed_history_directory()
            .and_then(|path| directory::open(&path).map(|()| path))
        {
            Ok(path) => format!("Opened screenshot folder {}", path.display()),
            Err(error) => {
                log::warn!(target: "flash_shot::history", "history_directory_open_failed error={error}");
                format!("Could not open screenshot folder: {error}")
            }
        };
        cx.notify();
    }

    pub(crate) fn hide_settings_window(&mut self) {
        if let Some(handle) = self.settings_window_handle
            && let Err(error) = window_visibility::hide(handle)
        {
            log::warn!(target: "flash_shot::settings", "settings_window_hide_failed error={error}");
        }
    }

    pub(super) fn toggle_overlay_more_actions(&mut self, cx: &mut Context<Self>) {
        self.overlay_more_actions = !self.overlay_more_actions;
        cx.notify();
    }

    pub(super) fn toggle_overlay_annotation_controls(&mut self, cx: &mut Context<Self>) {
        self.overlay_annotation_controls = !self.overlay_annotation_controls;
        cx.notify();
    }

    fn return_to_background(&mut self) {
        self.hide_settings_window();
    }
}

fn tool_selected_status(tool: AnnotationTool) -> &'static str {
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

fn drawing_status(tool: AnnotationTool) -> &'static str {
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

fn annotation_added_status(tool: Option<AnnotationTool>) -> &'static str {
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

fn annotation_cancelled_status(tool: Option<AnnotationTool>) -> &'static str {
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

fn is_current_operation(current: u64, completed: u64) -> bool {
    current == completed
}

/// Releases the completed task's slot and accepts its result only while its workflow is current.
/// A superseded completion must still clear its own slot or future capture requests stay blocked.
fn claim_idle_completion(
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

fn next_history_limit(current: u16) -> u16 {
    match current {
        10 => 30,
        30 => 100,
        100 => 300,
        _ => 10,
    }
}

fn delayed_capture_status(remaining_seconds: u8) -> String {
    format!("Capture scheduled in {remaining_seconds} seconds")
}

fn clamp_physical_point(
    point: crate::domain::geometry::PhysicalPoint,
    bounds: PhysicalRect,
) -> crate::domain::geometry::PhysicalPoint {
    crate::domain::geometry::PhysicalPoint {
        x: point.x.clamp(bounds.left, bounds.right),
        y: point.y.clamp(bounds.top, bounds.bottom),
    }
}

fn utf16_offset(text: &str, byte_offset: usize) -> usize {
    text[..byte_offset].chars().map(char::len_utf16).sum()
}

fn byte_offset(text: &str, utf16_offset: usize) -> usize {
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

fn range_to_utf16(text: &str, range: &Range<usize>) -> Range<usize> {
    utf16_offset(text, range.start)..utf16_offset(text, range.end)
}

fn range_from_utf16(text: &str, range: &Range<usize>) -> Range<usize> {
    byte_offset(text, range.start)..byte_offset(text, range.end)
}

fn previous_char_boundary(text: &str, offset: usize) -> usize {
    text.char_indices()
        .rev()
        .find_map(|(index, _)| (index < offset).then_some(index))
        .unwrap_or(0)
}

fn next_char_boundary(text: &str, offset: usize) -> usize {
    text.char_indices()
        .find_map(|(index, _)| (index > offset).then_some(index))
        .unwrap_or(text.len())
}

fn selection_status(selection: PhysicalRect) -> String {
    format!(
        "Selection: {} x {} physical pixels",
        selection.width(),
        selection.height()
    )
}

fn smart_target_status(target: InspectionTarget, point: PhysicalPoint, color: String) -> String {
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

fn fill_color(stroke_rgba: u32) -> u32 {
    with_alpha(stroke_rgba, fill_alpha(stroke_rgba as u8))
}

fn pinned_size(image_width: f32, image_height: f32) -> gpui::Size<Pixels> {
    let width = image_width.max(1.0);
    let height = image_height.max(1.0);
    size(px(width), px(height))
}

fn with_alpha(color: u32, alpha: u8) -> u32 {
    (color & 0xFFFFFF00) | u32::from(alpha)
}

fn fill_alpha(stroke_alpha: u8) -> u8 {
    (u16::from(stroke_alpha) * 0x66 / 255) as u8
}

fn style_for_tool(
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

fn text_annotation_with_content(annotation: Annotation, content: String) -> Option<Annotation> {
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

fn intersect_rect(left: PhysicalRect, right: PhysicalRect) -> Option<PhysicalRect> {
    let intersection = PhysicalRect {
        left: left.left.max(right.left),
        top: left.top.max(right.top),
        right: left.right.min(right.right),
        bottom: left.bottom.min(right.bottom),
    };
    (intersection.width() > 0 && intersection.height() > 0).then_some(intersection)
}

fn resolve_pointer_selection(
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
fn hovered_color(
    frame: Option<&CaptureFrame>,
    hover_pixel: Option<PhysicalPoint>,
    format: ColorFormat,
) -> Option<String> {
    frame?
        .pixel_at(hover_pixel?)
        .map(|color| format.format(color))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ColorFormat {
    Hex,
    Rgb,
    Hsl,
}

impl ColorFormat {
    const fn from_setting(value: u8) -> Self {
        match value {
            1 => Self::Rgb,
            2 => Self::Hsl,
            _ => Self::Hex,
        }
    }

    const fn setting_value(self) -> u8 {
        match self {
            Self::Hex => 0,
            Self::Rgb => 1,
            Self::Hsl => 2,
        }
    }

    const fn next(self) -> Self {
        match self {
            Self::Hex => Self::Rgb,
            Self::Rgb => Self::Hsl,
            Self::Hsl => Self::Hex,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Hex => "HEX",
            Self::Rgb => "RGB",
            Self::Hsl => "HSL",
        }
    }

    fn format(self, color: crate::platform::capture::PixelColor) -> String {
        match self {
            Self::Hex => color.hex_rgb(),
            Self::Rgb => format!("rgb({}, {}, {})", color.red, color.green, color.blue),
            Self::Hsl => format_hsl(color.red, color.green, color.blue),
        }
    }
}

fn format_hsl(red: u8, green: u8, blue: u8) -> String {
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
enum KeyboardCommand {
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
    QuickSave,
    CopyColor,
    MoveColorCursor(i32, i32),
    Nudge(i32, i32),
    SelectTool(Option<AnnotationTool>),
}

enum SaveOutcome {
    Saved { path: PathBuf, managed: bool },
    Cancelled,
    Failed(String),
}

enum OpenImageOutcome {
    Opened {
        path: PathBuf,
        frame: CaptureFrame,
        document: Option<AnnotationDocument>,
        document_warning: Option<String>,
    },
    Cancelled,
    Failed(String),
}

/// Separates local OCR and remote translation failures so the overlay can suggest the right fix.
enum TranslationOutcome {
    Completed(String),
    PreparationFailed(String),
    OcrUnavailable,
    OcrFailed(String),
    ServiceFailed(String),
}

/// Runs the selection pipeline outside the UI thread while retaining the failure stage.
fn translate_selected_frame(
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
pub(super) fn ocr_language_label(language: Option<&str>) -> &'static str {
    match language {
        None => "automatic",
        Some("eng") => "English",
        Some("chi_sim") => "Simplified Chinese",
        Some("eng+chi_sim") => "English + Simplified Chinese",
        Some(_) => "automatic",
    }
}

/// Turns a local OCR probe into a concise readiness result with a concrete recovery action.
fn ocr_support_status(result: Result<&crate::ocr::OcrSupport, &std::io::Error>) -> String {
    match result {
        Ok(support) if support.language_available() => format!(
            "Local OCR ready: {} with {}",
            support.version(),
            ocr_language_label(Some(support.language()))
        ),
        Ok(support) => format!(
            "Tesseract is installed but the {} language data is missing. Install that language pack or choose another OCR language.",
            ocr_language_label(Some(support.language()))
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            format!(
                "Local OCR is unavailable. Install Tesseract or set FLASH_SHOT_TESSERACT: {error}"
            )
        }
        Err(error) => format!("Could not check local OCR support: {error}"),
    }
}

/// Describes only local translation configuration so checking support never contacts a service.
fn translation_support_status(
    result: std::io::Result<Option<crate::translation::TranslationConfig>>,
) -> String {
    match result {
        Ok(Some(config)) => format!(
            "Translation ready: HTTPS endpoint configured for {}",
            config.target_language()
        ),
        Ok(None) => {
            "Translation is disabled. Set FLASH_SHOT_TRANSLATION_ENDPOINT to opt in.".to_owned()
        }
        Err(error) => format!("Translation configuration needs attention: {error}"),
    }
}

/// Turns each translation-stage failure into a recovery action instead of a generic error.
fn translation_failure_status(outcome: &TranslationOutcome) -> String {
    match outcome {
        TranslationOutcome::PreparationFailed(error) => {
            format!("Could not prepare the selection for translation: {error}")
        }
        TranslationOutcome::OcrUnavailable => {
            "Local OCR is unavailable. Install Tesseract or set FLASH_SHOT_TESSERACT.".to_owned()
        }
        TranslationOutcome::OcrFailed(error) => {
            format!("Could not recognize text for translation: {error}")
        }
        TranslationOutcome::ServiceFailed(error) => {
            format!("Translation service failed: {error}. Check the endpoint and try again.")
        }
        TranslationOutcome::Completed(_) => String::new(),
    }
}

fn keyboard_command(keystroke: &Keystroke) -> Option<KeyboardCommand> {
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

fn next_annotation_selection(
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

fn annotation_position(annotations: &[AnnotationId], selected: AnnotationId) -> usize {
    annotations
        .iter()
        .position(|candidate| *candidate == selected)
        .map_or(0, |index| index + 1)
}

fn adjusted_number_value(value: u32, delta: i32) -> u32 {
    i64::from(value)
        .saturating_add(i64::from(delta))
        .clamp(1, i64::from(u32::MAX)) as u32
}

#[cfg(test)]
mod tests;
