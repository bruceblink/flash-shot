//! Per-display borderless capture overlays backed by the shared capture session.

use std::{
    io,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{
    App, Bounds, Context, ElementInputHandler, Entity, FocusHandle, Focusable, FontWeight,
    KeyDownEvent, Keystroke, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit,
    Pixels, Render, RenderImage, Subscription, TextAlign, TextRun, Window,
    WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions, canvas, div, fill, img,
    point, prelude::*, px, rgba, size,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::{
    AnnotationToolGroup, FlashShotApp,
    overlay_toolbar::{
        WorkspaceButtonConfig, WorkspaceButtonTone, icon, workspace_icon_button,
        workspace_separator, workspace_surface, workspace_swatch, workspace_text_button,
        workspace_text_button_with_aria,
    },
    workflow::{inspection_kind_label, selection_dimension_label},
};
use crate::{
    domain::{
        annotation::{
            Annotation, AnnotationId, AnnotationKind, AnnotationTool, SEQUENCE_MARKER_RADIUS,
            arrow_head_points, normalized_text_annotation_content,
        },
        geometry::{PhysicalPoint, PhysicalRect},
        selection::{PreviewTransform, SelectionDrag, ViewPoint, ViewRect},
    },
    history::ScreenshotHistory,
    i18n::{Locale, UiText},
    performance::PerformanceRecorder,
    platform::{
        capture::{CaptureFrame, PixelFormat},
        cursor,
        display::{DisplayInfo, DisplayRotation},
        window_inspector::{InspectionKind, InspectionTarget},
    },
    settings::UserSettings,
    theme::{ThemeColors, ThemeMetrics, ThemeMode},
};

const OVERLAY_EDGE_INSET: f32 = ThemeMetrics::OVERLAY_EDGE_INSET;
// Keep fallback controls above a scaled Windows taskbar when the borderless
// overlay extends over the full display rather than the working area.
const OVERLAY_BOTTOM_SAFE_INSET: f32 = ThemeMetrics::OVERLAY_BOTTOM_SAFE_INSET;
const OVERLAY_ACTION_BAR_GAP: f32 = ThemeMetrics::WORKSPACE_SELECTION_GAP;
const OVERLAY_ACTION_ITEM_GAP: f32 = ThemeMetrics::WORKSPACE_TOOLBAR_GAP;
const OVERLAY_ACTION_ITEM_HEIGHT: f32 = ThemeMetrics::WORKSPACE_TOOLBAR_HEIGHT;
const OVERLAY_ACTION_BAR_PADDING: f32 = ThemeMetrics::WORKSPACE_TOOLBAR_PADDING;
const OVERLAY_ACTION_BAR_BORDER: f32 = ThemeMetrics::WORKSPACE_SEPARATOR_WIDTH;
const OVERLAY_SECONDARY_MENU_GAP: f32 = ThemeMetrics::WORKSPACE_POPOVER_GAP;
const OVERLAY_RECOGNITION_PREVIEW_HEIGHT: f32 = 64.0;
const OVERLAY_RECOGNITION_STATUS_HEIGHT: f32 = 30.0;
const OVERLAY_RECOGNITION_PREVIEW_LIMIT: usize = 240;
const OVERLAY_DIMENSION_LABEL_WIDTH: f32 = 112.0;
const OVERLAY_DIMENSION_LABEL_HEIGHT: f32 = 26.0;
const OVERLAY_DIMENSION_LABEL_GAP: f32 = 8.0;
const OVERLAY_STATUS_ESTIMATED_HEIGHT: f32 = 42.0;
const OVERLAY_SMART_TARGET_HUD_WIDTH: f32 = 224.0;
const OVERLAY_SMART_TARGET_HUD_HEIGHT: f32 = 26.0;
const OVERLAY_SMART_TARGET_HUD_GAP: f32 = 8.0;
const ANNOTATION_TOOL_ESTIMATED_WIDTH: f32 = ThemeMetrics::WORKSPACE_TOOL_CELL_WIDTH;
const ANNOTATION_TOOL_ICON_WIDTH: f32 = ThemeMetrics::WORKSPACE_ICON_BUTTON_HIT_AREA;
const ANNOTATION_ACTION_HEIGHT: f32 = ThemeMetrics::WORKSPACE_STYLE_ROW_HEIGHT;
const ANNOTATION_TOOL_ROW_HEIGHT: f32 = ThemeMetrics::WORKSPACE_TOOL_ROW_HEIGHT;
const ANNOTATION_TOOL_GAP: f32 = ThemeMetrics::WORKSPACE_TOOL_GAP;
const ANNOTATION_TOOL_PALETTE_GAP: f32 = ThemeMetrics::SPACE_1;
const ANNOTATION_TOOLBAR_PADDING: f32 = ThemeMetrics::WORKSPACE_ANNOTATION_PADDING;
const ANNOTATION_TOOL_PALETTE_HEIGHT: f32 = ANNOTATION_TOOL_ICON_WIDTH
    + ANNOTATION_TOOLBAR_PADDING * 2.0
    + ANNOTATION_TOOL_GROUP_POPUP_BORDER * 2.0;
// Keep the measured palette count aligned with the buttons rendered below so compact locales do
// not reserve an unused wrapped row.
const ANNOTATION_TOOL_PALETTE_ITEMS: usize = 6;
const ANNOTATION_TOOL_GROUP_COUNT: usize = 4;
const ANNOTATION_TOOL_GROUP_MAX_ITEMS: usize = 3;
const ANNOTATION_TOOL_GROUP_POPUP_PADDING: f32 = ThemeMetrics::WORKSPACE_TOOLBAR_PADDING;
const ANNOTATION_TOOL_GROUP_POPUP_BORDER: f32 = ThemeMetrics::WORKSPACE_SEPARATOR_WIDTH;
const ANNOTATION_STYLE_PANEL_GAP: f32 = ThemeMetrics::WORKSPACE_POPOVER_GAP;
const ANNOTATION_STYLE_CONTROL_HEIGHT: f32 = ThemeMetrics::WORKSPACE_STYLE_ROW_HEIGHT;
const ANNOTATION_STYLE_CONTROL_GAP: f32 = ThemeMetrics::SPACE_1;
const ANNOTATION_STYLE_ROW_PADDING: f32 = ThemeMetrics::SPACE_1;
const ANNOTATION_STYLE_VALUE_WIDTH: f32 = ThemeMetrics::WORKSPACE_STYLE_VALUE_WIDTH;
const ANNOTATION_STYLE_OPACITY_WIDTH: f32 = ThemeMetrics::WORKSPACE_STYLE_OPACITY_WIDTH;
const ANNOTATION_STYLE_FILL_WIDTH: f32 = ThemeMetrics::WORKSPACE_STYLE_FILL_WIDTH;
const ANNOTATION_LAYERS_WIDTH: f32 = 180.0;
const ANNOTATION_LAYERS_PREFERRED_HEIGHT: f32 = 200.0;
const ANNOTATION_TOOLBAR_MAX_WIDTH: f32 = 900.0;
const OVERLAY_MORE_ACTIONS_ID: &str = "overlay-more-actions";
const SECONDARY_ACTION_COUNT: usize = 14;
// Chinese Save Editable, QR, and OCR labels need more room than their English counterparts.
const OVERLAY_MORE_ACTION_WIDTHS: [f32; 11] = [
    138.0, 128.0, 143.0, 92.0, 91.0, 72.0, 84.0, 92.0, 81.0, 101.0, 126.0,
];
const OVERLAY_RECOGNITION_ACTION_WIDTHS: [f32; 2] = [76.0, 92.0];
const OVERLAY_RETRY_ACTION_WIDTHS: [f32; 1] = [126.0];
const ANNOTATION_COLORS: [u32; 5] = [0xFF3B30FF, 0xFFCC00FF, 0x34C759FF, 0x007AFFFF, 0xAF52DEFF];
const ANNOTATION_WIDTHS: [u32; 5] = [1, 3, 4, 6, 10];
const ANNOTATION_FONT_SIZES: [u32; 4] = [16, 24, 32, 48];
const ANNOTATION_OPACITIES: [u8; 4] = [255, 192, 128, 64];
const MAGNIFIER_RADIUS: i32 = 4;
const MAGNIFIER_CELL_SIZE: f32 = 12.0;
const MAGNIFIER_GAP: f32 = 18.0;
const MIN_ANNOTATION_VIEW_FONT_SIZE: f32 = 8.0;
const MAX_ANNOTATION_VIEW_FONT_SIZE: f32 = 96.0;

const TEXT_TOOL_GROUP_TOOLS: &[AnnotationTool] = &[
    AnnotationTool::Text,
    AnnotationTool::Watermark,
    AnnotationTool::Number,
];
const SHAPE_TOOL_GROUP_TOOLS: &[AnnotationTool] =
    &[AnnotationTool::Rectangle, AnnotationTool::Ellipse];
const LINE_TOOL_GROUP_TOOLS: &[AnnotationTool] = &[
    AnnotationTool::Line,
    AnnotationTool::Arrow,
    AnnotationTool::Freehand,
];
const OBSCURE_TOOL_GROUP_TOOLS: &[AnnotationTool] = &[AnnotationTool::Blur, AnnotationTool::Mosaic];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AnnotationToolGroupSpec {
    group: AnnotationToolGroup,
    label: UiText,
    tooltip: UiText,
    tools: &'static [AnnotationTool],
}

const ANNOTATION_TOOL_GROUP_SPECS: [AnnotationToolGroupSpec; ANNOTATION_TOOL_GROUP_COUNT] = [
    AnnotationToolGroupSpec {
        group: AnnotationToolGroup::Text,
        label: UiText::OverlayTextGroup,
        tooltip: UiText::OverlayTextGroupTooltip,
        tools: TEXT_TOOL_GROUP_TOOLS,
    },
    AnnotationToolGroupSpec {
        group: AnnotationToolGroup::Shape,
        label: UiText::OverlayShapeGroup,
        tooltip: UiText::OverlayShapeGroupTooltip,
        tools: SHAPE_TOOL_GROUP_TOOLS,
    },
    AnnotationToolGroupSpec {
        group: AnnotationToolGroup::Line,
        label: UiText::OverlayLineGroup,
        tooltip: UiText::OverlayLineGroupTooltip,
        tools: LINE_TOOL_GROUP_TOOLS,
    },
    AnnotationToolGroupSpec {
        group: AnnotationToolGroup::Obscure,
        label: UiText::OverlayObscureGroup,
        tooltip: UiText::OverlayObscureGroupTooltip,
        tools: OBSCURE_TOOL_GROUP_TOOLS,
    },
];

/// Names the less-frequent actions at the exact point where users discover them.
fn secondary_action_tooltip(locale: Locale, action_id: &str) -> &'static str {
    match action_id {
        "scroll" => locale.text(UiText::OverlayScrollShotTooltip),
        "qr" => locale.text(UiText::OverlayQrTooltip),
        "ocr" => locale.text(UiText::OverlayOcrTooltip),
        "translate" => locale.text(UiText::OverlayTranslateTooltip),
        "record-area" => locale.text(UiText::OverlayRecordAreaTooltip),
        "record-window" => locale.text(UiText::OverlayRecordWindowTooltip),
        _ => "",
    }
}

/// Describes the stable focus order for commands revealed by the More panel.
///
/// The menu wraps by available width and conditionally renders recognition actions, so a semantic
/// order keeps arrow-key navigation predictable without depending on the current row layout. Tab
/// and Shift+Tab remain the overlay's annotation-navigation shortcuts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SecondaryAction {
    SaveAnnotations,
    SaveEditable,
    OpenAnnotations,
    QuickSave,
    ScrollShot,
    Qr,
    Ocr,
    CopyColor,
    Translate,
    RecordArea,
    RecordWindow,
    RetryRecognition,
    CopyRecognition,
    ClearRecognition,
}

impl SecondaryAction {
    /// Returns this action's menu-local focus position, leaving optional actions in a stable place.
    const fn index(self) -> usize {
        match self {
            Self::SaveAnnotations => 0,
            Self::SaveEditable => 1,
            Self::OpenAnnotations => 2,
            Self::QuickSave => 3,
            Self::ScrollShot => 4,
            Self::Qr => 5,
            Self::Ocr => 6,
            Self::CopyColor => 7,
            Self::Translate => 8,
            Self::RecordArea => 9,
            Self::RecordWindow => 10,
            Self::RetryRecognition => 11,
            Self::CopyRecognition => 12,
            Self::ClearRecognition => 13,
        }
    }
}

/// Lists the commands that are always rendered in the expanded More panel.
const ALWAYS_VISIBLE_SECONDARY_ACTIONS: [SecondaryAction; 11] = [
    SecondaryAction::SaveAnnotations,
    SecondaryAction::SaveEditable,
    SecondaryAction::OpenAnnotations,
    SecondaryAction::QuickSave,
    SecondaryAction::ScrollShot,
    SecondaryAction::Qr,
    SecondaryAction::Ocr,
    SecondaryAction::CopyColor,
    SecondaryAction::Translate,
    SecondaryAction::RecordArea,
    SecondaryAction::RecordWindow,
];

/// Owns one rendered More menu's focus order, including optional recognition commands.
#[derive(Clone)]
struct SecondaryActionNavigation {
    current: SecondaryAction,
    visible_actions: Vec<SecondaryAction>,
    focus_handles: [FocusHandle; SECONDARY_ACTION_COUNT],
}

impl SecondaryActionNavigation {
    /// Rebinds this navigation model to the action that receives a key event.
    fn for_action(&self, current: SecondaryAction) -> Self {
        Self {
            current,
            visible_actions: self.visible_actions.clone(),
            focus_handles: self.focus_handles.clone(),
        }
    }

    /// Returns the persistent handle for the currently rendered action.
    fn focus_handle(&self) -> FocusHandle {
        self.focus_handles[self.current.index()].clone()
    }

    /// Focuses the first or last visible item when the More trigger receives an arrow key.
    fn focus_edge(
        &self,
        direction: SecondaryActionFocusDirection,
        window: &mut Window,
        cx: &mut App,
    ) {
        let target = match direction {
            SecondaryActionFocusDirection::Next => self.visible_actions.first(),
            SecondaryActionFocusDirection::Previous => self.visible_actions.last(),
        };
        if let Some(target) = target {
            self.focus_handles[target.index()].focus(window, cx);
        }
    }

    /// Keeps arrow navigation inside the visible menu instead of moving to another toolbar control.
    fn handle_key_down(&self, event: &KeyDownEvent, window: &mut Window, cx: &mut App) {
        match secondary_action_focus_direction(&event.keystroke) {
            Some(direction) => {
                if let Some(target) =
                    secondary_action_focus_target(self.current, &self.visible_actions, direction)
                {
                    self.focus_handles[target.index()].focus(window, cx);
                }
                cx.stop_propagation();
            }
            None => stop_overlay_action_key_propagation(event, window, cx),
        }
    }
}

impl AnnotationToolGroup {
    const fn index(self) -> usize {
        match self {
            Self::Text => 0,
            Self::Shape => 1,
            Self::Line => 2,
            Self::Obscure => 3,
        }
    }

    const fn spec(self) -> AnnotationToolGroupSpec {
        ANNOTATION_TOOL_GROUP_SPECS[self.index()]
    }
}

/// Keeps tool-group keyboard traversal local to the currently materialized popover.
#[derive(Clone)]
struct AnnotationToolGroupNavigation {
    tools: &'static [AnnotationTool],
    current: AnnotationTool,
    focus_handles: [FocusHandle; ANNOTATION_TOOL_GROUP_MAX_ITEMS],
}

impl AnnotationToolGroupNavigation {
    fn focus_handle(&self, tool: AnnotationTool) -> Option<FocusHandle> {
        self.tools
            .iter()
            .position(|candidate| *candidate == tool)
            .and_then(|index| self.focus_handles.get(index).cloned())
    }

    /// Keeps arrow-key traversal inside a group while plain Enter/Space remains a button click.
    fn handle_key_down(&self, event: &KeyDownEvent, window: &mut Window, cx: &mut App) {
        if let Some(direction) = annotation_tool_group_focus_direction(&event.keystroke) {
            if let Some(target) =
                annotation_tool_group_focus_target(self.current, self.tools, direction)
                && let Some(focus_handle) = self.focus_handle(target)
            {
                focus_handle.focus(window, cx);
            }
            cx.stop_propagation();
            return;
        }
        stop_overlay_action_key_propagation(event, window, cx);
    }
}

/// Names the local arrow-key direction used by a tool-group popover.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AnnotationToolGroupFocusDirection {
    Next,
    Previous,
}

/// Maps plain arrow keys to local tool-group focus traversal.
fn annotation_tool_group_focus_direction(
    keystroke: &Keystroke,
) -> Option<AnnotationToolGroupFocusDirection> {
    if keystroke.modifiers.modified() {
        return None;
    }

    match keystroke.key.as_str() {
        "down" | "right" => Some(AnnotationToolGroupFocusDirection::Next),
        "up" | "left" => Some(AnnotationToolGroupFocusDirection::Previous),
        _ => None,
    }
}

/// Chooses the next or previous child without allowing focus to escape the active group.
fn annotation_tool_group_focus_target(
    current: AnnotationTool,
    tools: &[AnnotationTool],
    direction: AnnotationToolGroupFocusDirection,
) -> Option<AnnotationTool> {
    if tools.is_empty() {
        return None;
    }
    let current_index = tools.iter().position(|tool| *tool == current)?;
    let target_index = match direction {
        AnnotationToolGroupFocusDirection::Next => (current_index + 1) % tools.len(),
        AnnotationToolGroupFocusDirection::Previous => {
            current_index.checked_sub(1).unwrap_or(tools.len() - 1)
        }
    };
    tools.get(target_index).copied()
}

/// Maps each existing annotation tool to the catalog label used by its group child.
const fn annotation_tool_ui_text(tool: AnnotationTool) -> UiText {
    match tool {
        AnnotationTool::Watermark => UiText::OverlayWatermark,
        AnnotationTool::Text => UiText::OverlayText,
        AnnotationTool::Number => UiText::OverlayNumber,
        AnnotationTool::Blur => UiText::OverlayBlur,
        AnnotationTool::Mosaic => UiText::OverlayMosaic,
        AnnotationTool::Highlight => UiText::OverlayHighlight,
        AnnotationTool::Rectangle => UiText::OverlayRectangle,
        AnnotationTool::Ellipse => UiText::OverlayEllipse,
        AnnotationTool::Line => UiText::OverlayLine,
        AnnotationTool::Arrow => UiText::OverlayArrow,
        AnnotationTool::Freehand => UiText::OverlayFreehand,
    }
}

/// Builds one fixed-size More action with a stable keyboard position and focus ring.
#[allow(clippy::too_many_arguments)]
fn secondary_action_button(
    id: impl Into<gpui::ElementId>,
    navigation: SecondaryActionNavigation,
    label: &'static str,
    width: f32,
    colors: ThemeColors,
    primary: bool,
    tooltip: Option<&'static str>,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let focus_handle = navigation.focus_handle();
    let tone = if primary {
        WorkspaceButtonTone::Primary
    } else {
        WorkspaceButtonTone::Neutral
    };
    workspace_text_button(
        id,
        label,
        WorkspaceButtonConfig::text(
            Some(width),
            OVERLAY_ACTION_ITEM_HEIGHT,
            colors,
            tone,
            primary,
            true,
            tooltip,
        ),
        on_click,
    )
    .track_focus(&focus_handle)
    .on_key_down(move |event, window, cx| navigation.handle_key_down(event, window, cx))
}

/// Keeps the primary capture commands and their keyboard equivalents discoverable.
fn primary_action_tooltip(locale: Locale, action_id: &str) -> &'static str {
    match action_id {
        "draw" => locale.text(UiText::OverlayMarkTooltip),
        "copy" => locale.text(UiText::OverlayCopyTooltip),
        "save" => locale.text(UiText::OverlaySaveTooltip),
        "cancel" => locale.text(UiText::OverlayCancelTooltip),
        _ => "",
    }
}

/// Keeps the More/Less control's element identity stable while its label follows menu state, so
/// GPUI can retain keyboard focus when the secondary action menu is expanded or collapsed.
fn more_actions_button_label(locale: Locale, show_more_actions: bool) -> &'static str {
    locale.text(if show_more_actions {
        UiText::OverlayLess
    } else {
        UiText::OverlayMore
    })
}

/// Keeps a focused action from also invoking an overlay-wide command.
///
/// GPUI converts unmodified Enter and Space key releases into keyboard click events for focused
/// buttons. Stopping their key-down propagation lets that click stay local while preserving
/// Shift+Enter for the overlay's quick-save shortcut.
fn stop_overlay_action_key_propagation(event: &KeyDownEvent, _window: &mut Window, cx: &mut App) {
    if should_stop_overlay_action_key_propagation(&event.keystroke) {
        cx.stop_propagation();
    }
}

/// Identifies standard focused-button activation keys that must not reach overlay shortcuts.
fn should_stop_overlay_action_key_propagation(keystroke: &Keystroke) -> bool {
    !keystroke.modifiers.modified() && matches!(keystroke.key.as_str(), "enter" | "space")
}

/// Recognizes the explicit More-menu shortcut without shadowing annotation tool shortcuts.
fn more_actions_shortcut(keystroke: &Keystroke) -> bool {
    keystroke.key == "m"
        && keystroke.modifiers.alt
        && !keystroke.modifiers.shift
        && !keystroke.modifiers.control
        && !keystroke.modifiers.platform
        && !keystroke.modifiers.function
}

/// Recognizes Escape while More is open so it closes the menu before cancelling the capture.
fn close_more_actions_shortcut(keystroke: &Keystroke) -> bool {
    keystroke.key == "escape" && !keystroke.modifiers.modified()
}

/// Selects the local arrow-key movement allowed while a More action has focus.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SecondaryActionFocusDirection {
    Next,
    Previous,
}

/// Maps plain arrow keys to menu-local focus traversal without taking annotation shortcuts.
fn secondary_action_focus_direction(
    keystroke: &Keystroke,
) -> Option<SecondaryActionFocusDirection> {
    if keystroke.modifiers.modified() {
        return None;
    }

    match keystroke.key.as_str() {
        "down" | "right" => Some(SecondaryActionFocusDirection::Next),
        "up" | "left" => Some(SecondaryActionFocusDirection::Previous),
        _ => None,
    }
}

/// Chooses the next visible command and wraps inside the More menu instead of the whole window.
fn secondary_action_focus_target(
    current: SecondaryAction,
    visible_actions: &[SecondaryAction],
    direction: SecondaryActionFocusDirection,
) -> Option<SecondaryAction> {
    if visible_actions.is_empty() {
        return None;
    }
    let current_index = visible_actions
        .iter()
        .position(|action| *action == current)?;
    let target_index = match direction {
        SecondaryActionFocusDirection::Next => (current_index + 1) % visible_actions.len(),
        SecondaryActionFocusDirection::Previous => current_index
            .checked_sub(1)
            .unwrap_or(visible_actions.len() - 1),
    };
    visible_actions.get(target_index).copied()
}

/// Enters the expanded More menu from its trigger without using global tab traversal.
fn handle_more_actions_key_down(
    show_more_actions: bool,
    navigation: &SecondaryActionNavigation,
    event: &KeyDownEvent,
    window: &mut Window,
    cx: &mut App,
) {
    if show_more_actions && let Some(direction) = secondary_action_focus_direction(&event.keystroke)
    {
        navigation.focus_edge(direction, window, cx);
        cx.stop_propagation();
        return;
    }
    stop_overlay_action_key_propagation(event, window, cx);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SelectionCursor {
    Crosshair,
    Move,
    ResizeNwse,
    ResizeNesw,
}

#[derive(Clone, Copy)]
enum AnnotationActionTone {
    Neutral,
    Primary,
    Destructive,
}

/// Builds one annotation action with consistent spacing, focus feedback, semantic color, and a
/// non-interactive disabled state. Disabled controls stay visible so editing history cannot move
/// the drawing tools underneath the pointer.
fn annotation_action_button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<gpui::SharedString>,
    colors: ThemeColors,
    tone: AnnotationActionTone,
    enabled: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let tone = match tone {
        AnnotationActionTone::Neutral => WorkspaceButtonTone::Neutral,
        AnnotationActionTone::Primary => WorkspaceButtonTone::Primary,
        AnnotationActionTone::Destructive => WorkspaceButtonTone::Destructive,
    };
    workspace_text_button(
        id,
        label,
        WorkspaceButtonConfig::text(
            None,
            ANNOTATION_ACTION_HEIGHT,
            colors,
            tone,
            matches!(tone, WorkspaceButtonTone::Primary),
            enabled,
            None,
        ),
        on_click,
    )
    .on_key_down(stop_overlay_action_key_propagation)
}

/// Builds one fixed-size annotation tool button so the toolbar layout uses the rendered hitbox.
fn annotation_tool_button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<gpui::SharedString>,
    colors: ThemeColors,
    active: bool,
    width: f32,
    tooltip: Option<&'static str>,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    workspace_text_button(
        id,
        label,
        WorkspaceButtonConfig::text(
            Some(width),
            ANNOTATION_TOOL_ROW_HEIGHT,
            colors,
            if active {
                WorkspaceButtonTone::Primary
            } else {
                WorkspaceButtonTone::Neutral
            },
            active,
            true,
            tooltip,
        ),
        on_click,
    )
}

/// Builds one compact annotation launcher; the full tool name stays available to screen readers
/// and in the hover tooltip so the drawing row can use stable icon-sized hit targets.
fn annotation_icon_button(
    id: impl Into<gpui::ElementId>,
    icon: super::overlay_toolbar::WorkspaceIcon,
    label: &'static str,
    tooltip: &'static str,
    colors: ThemeColors,
    active: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    workspace_icon_button(
        id,
        icon,
        label,
        WorkspaceButtonConfig::icon(
            colors,
            if active {
                WorkspaceButtonTone::Primary
            } else {
                WorkspaceButtonTone::Neutral
            },
            active,
            true,
            tooltip,
        ),
        on_click,
    )
    .on_key_down(stop_overlay_action_key_propagation)
}

/// Returns one stable vector icon for the four annotation tool groups shown in the compact palette.
const fn annotation_tool_group_icon(
    group: AnnotationToolGroup,
) -> super::overlay_toolbar::WorkspaceIcon {
    match group {
        AnnotationToolGroup::Text => icon::TEXT,
        AnnotationToolGroup::Shape => icon::SHAPE,
        AnnotationToolGroup::Line => icon::LINE,
        AnnotationToolGroup::Obscure => icon::OBSCURE,
    }
}

/// Keeps acceptance selectors and accessibility ids stable as group contents evolve.
const fn annotation_tool_key(tool: AnnotationTool) -> &'static str {
    match tool {
        AnnotationTool::Text => "text",
        AnnotationTool::Watermark => "watermark",
        AnnotationTool::Number => "number",
        AnnotationTool::Blur => "blur",
        AnnotationTool::Mosaic => "mosaic",
        AnnotationTool::Highlight => "highlight",
        AnnotationTool::Rectangle => "rectangle",
        AnnotationTool::Ellipse => "ellipse",
        AnnotationTool::Line => "line",
        AnnotationTool::Arrow => "arrow",
        AnnotationTool::Freehand => "freehand",
    }
}

/// Gives each group trigger a stable semantic id independent of its localized label.
const fn annotation_tool_group_key(group: AnnotationToolGroup) -> &'static str {
    match group {
        AnnotationToolGroup::Text => "text",
        AnnotationToolGroup::Shape => "shape",
        AnnotationToolGroup::Line => "line",
        AnnotationToolGroup::Obscure => "obscure",
    }
}

/// Builds one compact style choice with the shared toolbar focus, hover, tooltip, and hitbox
/// behavior. The visible value stays short while the tooltip describes the control group.
fn annotation_style_button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<gpui::SharedString>,
    aria_label: impl Into<gpui::SharedString>,
    config: WorkspaceButtonConfig,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    workspace_text_button_with_aria(id, label, aria_label, config, on_click)
        .text_xs()
        .px_1()
        .on_key_down(stop_overlay_action_key_propagation)
}

pub(super) struct CaptureOverlay {
    app: Entity<FlashShotApp>,
    display: DisplayInfo,
    preview: Arc<RenderImage>,
    // Only full-display overlays may continue a captured mouse drag onto another monitor.
    allow_cross_display_drag: bool,
    // The selection canvas owns only its own left-button gesture; capture-phase mouse-up must
    // still finish it after Win32 routes the release outside the display-sized client hitbox.
    selection_pointer_active: bool,
    // The workflow generation prevents queued input from a closed overlay changing a later
    // capture session after the user presses Capture again.
    operation_generation: u64,
    // Raw Windows pointer messages can arrive faster than the display refreshes. Retain only the
    // latest sample until the next frame so selection work and redraw notifications stay bounded.
    selection_updates: FrameInputBatch<PendingSelectionUpdate>,
    focus_handle: FocusHandle,
    more_actions_focus_handle: FocusHandle,
    secondary_action_focus_handles: [FocusHandle; SECONDARY_ACTION_COUNT],
    annotation_tool_group_trigger_focus_handles: [FocusHandle; ANNOTATION_TOOL_GROUP_COUNT],
    annotation_tool_group_item_focus_handles:
        [[FocusHandle; ANNOTATION_TOOL_GROUP_MAX_ITEMS]; ANNOTATION_TOOL_GROUP_COUNT],
    topmost_requested: bool,
    annotation_arrange_actions_for: Option<AnnotationId>,
    _app_observation: Subscription,
}

/// Captures the newest pointer sample that arrived before one rendered frame.
///
/// A new sample replaces the previous one while a callback is already scheduled. The generation
/// makes an old callback harmless after a new gesture or mouse-up discards its pending sample.
/// `push` returns a generation exactly once per pending frame, which is the testable scheduling
/// contract used by the real GPUI `on_mouse_move` listener below.
#[derive(Debug)]
struct FrameInputBatch<T> {
    latest: Option<T>,
    generation: u64,
    scheduled: bool,
}

impl<T> Default for FrameInputBatch<T> {
    fn default() -> Self {
        Self {
            latest: None,
            generation: 0,
            scheduled: false,
        }
    }
}

impl<T> FrameInputBatch<T> {
    /// Stores the latest input and returns a generation only when this frame needs a new callback.
    fn push(&mut self, input: T) -> Option<u64> {
        self.latest = Some(input);
        if self.scheduled {
            None
        } else {
            self.scheduled = true;
            Some(self.generation)
        }
    }

    /// Takes the latest sample only when the callback still belongs to the active gesture.
    fn take(&mut self, generation: u64) -> Option<T> {
        if self.scheduled && self.generation == generation {
            self.scheduled = false;
            self.latest.take()
        } else {
            None
        }
    }

    /// Cancels an outstanding frame callback before a new gesture or final mouse position wins.
    fn invalidate(&mut self) {
        self.latest = None;
        self.scheduled = false;
        self.generation = self.generation.wrapping_add(1);
    }
}

/// Contains the pointer state that must stay internally consistent when one frame applies it.
#[derive(Clone, Copy, Debug)]
struct PendingSelectionUpdate {
    operation_generation: u64,
    hover_point: Option<PhysicalPoint>,
    dragging_point: Option<PhysicalPoint>,
    preserve_aspect_ratio: bool,
    resize_from_center: bool,
}

impl CaptureOverlay {
    pub(super) fn new(
        app: Entity<FlashShotApp>,
        display: DisplayInfo,
        preview: Arc<RenderImage>,
        operation_generation: u64,
        allow_cross_display_drag: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let observation = cx.observe(&app, |_, _, cx| cx.notify());
        Self {
            app,
            display,
            preview,
            allow_cross_display_drag,
            selection_pointer_active: false,
            operation_generation,
            selection_updates: FrameInputBatch::default(),
            focus_handle: cx.focus_handle(),
            more_actions_focus_handle: cx.focus_handle().tab_stop(false),
            secondary_action_focus_handles: std::array::from_fn(|index| {
                cx.focus_handle().tab_stop(true).tab_index(index as isize)
            }),
            annotation_tool_group_trigger_focus_handles: std::array::from_fn(|_| cx.focus_handle()),
            annotation_tool_group_item_focus_handles: std::array::from_fn(|_| {
                std::array::from_fn(|_| cx.focus_handle())
            }),
            topmost_requested: false,
            annotation_arrange_actions_for: None,
            _app_observation: observation,
        }
    }

    fn transform(&self, viewport: Bounds<Pixels>) -> Option<PreviewTransform> {
        PreviewTransform::contain(self.display.physical_bounds, view_rect(viewport))
    }

    fn begin_selection(
        &mut self,
        event: &MouseDownEvent,
        viewport: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let operation_generation = self.operation_generation;
        if !accepts_overlay_input(operation_generation, self.app.read(cx).operation_generation) {
            return;
        }
        let frame_bounds = self
            .app
            .read(cx)
            .frame
            .as_ref()
            .map(|frame| frame.bounds)
            .unwrap_or(self.display.physical_bounds);
        let screen_pointer = self
            .allow_cross_display_drag
            .then(|| cursor::position().ok())
            .flatten();
        let Some(point) = self.transform(viewport).and_then(|transform| {
            selection_point_from_view_or_screen(
                transform,
                event.position,
                self.allow_cross_display_drag,
                screen_pointer,
                frame_bounds,
            )
        }) else {
            return;
        };
        self.selection_pointer_active = true;
        // A new gesture must not consume a hover or drag sample that belonged to the prior one.
        self.selection_updates.invalidate();
        let resize_handle = self
            .app
            .read(cx)
            .selection_drag
            .selection()
            .and_then(|selection| {
                self.transform(viewport)?.resize_handle_at(
                    selection,
                    view_point(event.position),
                    10.0,
                )
            });
        let annotation_resize_handle = self.app.read(cx).selected_annotation.and_then(|id| {
            let annotation = self
                .app
                .read(cx)
                .annotation_document
                .as_ref()?
                .annotation(id)?;
            self.transform(viewport)?.resize_handle_at(
                annotation.bounds(),
                view_point(event.position),
                10.0,
            )
        });
        let app = self.app.clone();
        cx.defer(move |cx| {
            app.update(cx, |app, cx| {
                if !accepts_overlay_input(operation_generation, app.operation_generation) {
                    return;
                }
                app.begin_overlay_selection(point, resize_handle, annotation_resize_handle);
                cx.notify();
            })
        });
    }

    fn update_selection(
        &mut self,
        event: &MouseMoveEvent,
        viewport: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let operation_generation = self.operation_generation;
        if !accepts_overlay_input(operation_generation, self.app.read(cx).operation_generation) {
            self.selection_updates.invalidate();
            return;
        }
        let Some(transform) = self.transform(viewport) else {
            return;
        };
        let view = view_point(event.position);
        let point = transform.view_to_pixel(view);
        let preserve_aspect_ratio = event.modifiers.shift;
        let resize_from_center = event.modifiers.alt;
        let frame_bounds = self
            .app
            .read(cx)
            .frame
            .as_ref()
            .map(|frame| frame.bounds)
            .unwrap_or(self.display.physical_bounds);
        let dragging_point = event
            .dragging()
            .then(|| {
                let screen_pointer = self
                    .allow_cross_display_drag
                    .then(|| cursor::position().ok())
                    .flatten();
                selection_point_from_view_or_screen(
                    transform,
                    event.position,
                    self.allow_cross_display_drag,
                    screen_pointer,
                    frame_bounds,
                )
            })
            .flatten();
        let update = PendingSelectionUpdate {
            operation_generation,
            hover_point: point,
            dragging_point,
            preserve_aspect_ratio,
            resize_from_center,
        };
        let Some(frame_generation) = self.selection_updates.push(update) else {
            return;
        };
        // This is the only deferred callback for all moves received before the next paint. The
        // callback applies the most recent sample, not the first one that happened to arrive.
        cx.on_next_frame(window, move |this, _, cx| {
            if let Some(update) = this.selection_updates.take(frame_generation) {
                this.apply_selection_update(update, cx);
            }
        });
        // `on_next_frame` needs a paint to occur. GPUI deduplicates same-entity notifications in
        // this event cycle, so repeated raw moves keep one queued callback and one invalidation.
        cx.notify();
    }

    /// Applies one frame's newest pointer sample to the shared capture model.
    ///
    /// The overlay owns scheduling while `FlashShotApp` owns the actual selection and hover state;
    /// checking both generations keeps a closed overlay from changing a later capture session.
    fn apply_selection_update(&mut self, update: PendingSelectionUpdate, cx: &mut Context<Self>) {
        if !accepts_overlay_input(
            update.operation_generation,
            self.app.read(cx).operation_generation,
        ) {
            return;
        }
        let app = self.app.clone();
        app.update(cx, |app, cx| {
            if !accepts_overlay_input(update.operation_generation, app.operation_generation) {
                return;
            }
            app.update_overlay_hover(update.hover_point, cx);
            if let Some(point) = update.dragging_point {
                app.update_overlay_selection(
                    point,
                    update.preserve_aspect_ratio,
                    update.resize_from_center,
                    cx,
                );
            }
        });
    }

    fn finish_selection(
        &mut self,
        event: &MouseUpEvent,
        viewport: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.selection_pointer_active = false;
        let operation_generation = self.operation_generation;
        if !accepts_overlay_input(operation_generation, self.app.read(cx).operation_generation) {
            self.selection_updates.invalidate();
            return;
        }
        // The final mouse position wins over any queued frame update from this gesture, so it
        // cannot overwrite the selection after the button is released.
        self.selection_updates.invalidate();
        let frame_bounds = self
            .app
            .read(cx)
            .frame
            .as_ref()
            .map(|frame| frame.bounds)
            .unwrap_or(self.display.physical_bounds);
        let point = self.transform(viewport).and_then(|transform| {
            let screen_pointer = self
                .allow_cross_display_drag
                .then(|| cursor::position().ok())
                .flatten();
            selection_point_from_view_or_screen(
                transform,
                event.position,
                self.allow_cross_display_drag,
                screen_pointer,
                frame_bounds,
            )
        });
        let Some(point) = point else { return };
        let app = self.app.clone();
        let preserve_aspect_ratio = event.modifiers.shift;
        let resize_from_center = event.modifiers.alt;
        let copy_on_double_click = capture_double_click(event.click_count);
        cx.defer(move |cx| {
            app.update(cx, |app, cx| {
                if !accepts_overlay_input(operation_generation, app.operation_generation) {
                    return;
                }
                app.finish_overlay_selection(
                    point,
                    preserve_aspect_ratio,
                    resize_from_center,
                    copy_on_double_click,
                    cx,
                )
            })
        });
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let operation_generation = self.operation_generation;
        if !accepts_overlay_input(operation_generation, self.app.read(cx).operation_generation) {
            return;
        }
        // Copy cancellation has priority over More and text-editor Escape handling. The copied
        // pixels are already frozen, so this leaves the user in their current editing context.
        if event.keystroke.key == "escape"
            && !event.keystroke.modifiers.modified()
            && self.app.read(cx).selection_copy_owns_escape()
        {
            self.app.update(cx, |app, cx| app.cancel_selection_copy(cx));
            cx.stop_propagation();
            return;
        }
        if close_more_actions_shortcut(&event.keystroke)
            && self.close_annotation_tool_group_from_keyboard(window, cx)
        {
            cx.stop_propagation();
            return;
        }
        if close_more_actions_shortcut(&event.keystroke)
            && self.close_more_actions_from_keyboard(window, cx)
        {
            cx.stop_propagation();
            return;
        }
        if more_actions_shortcut(&event.keystroke)
            && self.open_more_actions_from_keyboard(window, cx)
        {
            cx.stop_propagation();
            return;
        }
        let app = self.app.clone();
        let event = event.clone();
        cx.defer(move |cx| {
            app.update(cx, |app, cx| {
                if !accepts_overlay_input(operation_generation, app.operation_generation) {
                    return;
                }
                if !app.handle_text_edit_key(&event.keystroke, cx) {
                    app.handle_key_down(&event, cx);
                }
            })
        });
    }

    fn annotation_tool_group_trigger_focus_handle(
        &self,
        group: AnnotationToolGroup,
    ) -> FocusHandle {
        self.annotation_tool_group_trigger_focus_handles[group.index()].clone()
    }

    fn annotation_tool_group_item_focus_handle(
        &self,
        group: AnnotationToolGroup,
        index: usize,
    ) -> FocusHandle {
        self.annotation_tool_group_item_focus_handles[group.index()][index].clone()
    }

    /// Opens or closes a tool group and moves focus after the next render, so the new child exists.
    fn toggle_annotation_tool_group_from_trigger(
        &mut self,
        group: AnnotationToolGroup,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let operation_generation = self.operation_generation;
        let owner = self.display.id.clone();
        let app = self.app.clone();
        let is_open = {
            let app = app.read(cx);
            app.annotation_tool_group == Some(group)
                && app.annotation_tool_group_owner.as_deref() == Some(owner.as_str())
        };
        let focus_handle = if is_open {
            self.annotation_tool_group_trigger_focus_handle(group)
        } else {
            self.annotation_tool_group_item_focus_handle(group, 0)
        };
        cx.defer(move |cx| {
            app.update(cx, |app, cx| {
                if accepts_overlay_input(operation_generation, app.operation_generation) {
                    app.toggle_annotation_tool_group(&owner, group, cx);
                }
            });
        });
        cx.on_next_frame(window, move |_, window, cx| focus_handle.focus(window, cx));
    }

    /// Selects one materialized child exactly once, closes its owner, and returns focus to the
    /// trigger so the next keyboard action stays in the screenshot workspace.
    fn select_annotation_tool_from_group(
        &mut self,
        group: AnnotationToolGroup,
        tool: AnnotationTool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let operation_generation = self.operation_generation;
        let owner = self.display.id.clone();
        let app = self.app.clone();
        let trigger_focus = self.annotation_tool_group_trigger_focus_handle(group);
        cx.defer(move |cx| {
            app.update(cx, |app, cx| {
                if !accepts_overlay_input(operation_generation, app.operation_generation)
                    || app.annotation_tool_group != Some(group)
                    || app.annotation_tool_group_owner.as_deref() != Some(owner.as_str())
                {
                    return;
                }
                app.select_annotation_tool(tool, cx);
                app.close_annotation_tool_group();
            });
        });
        cx.on_next_frame(window, move |_, window, cx| trigger_focus.focus(window, cx));
    }

    /// Closes the one group owned by this overlay and restores trigger focus for Escape handling.
    fn close_annotation_tool_group_from_keyboard(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let (group, owner) = {
            let app = self.app.read(cx);
            (
                app.annotation_tool_group,
                app.annotation_tool_group_owner.as_deref() == Some(self.display.id.as_str()),
            )
        };
        let Some(group) = group.filter(|_| owner) else {
            return false;
        };
        let operation_generation = self.operation_generation;
        let app = self.app.clone();
        let trigger_focus = self.annotation_tool_group_trigger_focus_handle(group);
        cx.defer(move |cx| {
            app.update(cx, |app, cx| {
                if accepts_overlay_input(operation_generation, app.operation_generation)
                    && app.annotation_tool_group == Some(group)
                {
                    app.close_annotation_tool_group();
                    cx.notify();
                }
            });
        });
        cx.on_next_frame(window, move |_, window, cx| trigger_focus.focus(window, cx));
        true
    }

    /// Handles a child button's local arrows and Escape without leaking them to canvas shortcuts.
    fn handle_annotation_tool_group_item_key_down(
        &mut self,
        group: AnnotationToolGroup,
        current: AnnotationTool,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if close_more_actions_shortcut(&event.keystroke)
            && self.close_annotation_tool_group_from_keyboard(window, cx)
        {
            cx.stop_propagation();
            return;
        }
        let navigation = AnnotationToolGroupNavigation {
            tools: group.spec().tools,
            current,
            focus_handles: self.annotation_tool_group_item_focus_handles[group.index()].clone(),
        };
        navigation.handle_key_down(event, window, cx);
    }

    /// Lets a focused group trigger enter its children with arrows while keeping activation local.
    fn handle_annotation_tool_group_trigger_key_down(
        &mut self,
        group: AnnotationToolGroup,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let is_open = {
            let app = self.app.read(cx);
            app.annotation_tool_group == Some(group)
                && app.annotation_tool_group_owner.as_deref() == Some(self.display.id.as_str())
        };
        if close_more_actions_shortcut(&event.keystroke)
            && is_open
            && self.close_annotation_tool_group_from_keyboard(window, cx)
        {
            cx.stop_propagation();
            return;
        }
        if is_open && let Some(direction) = annotation_tool_group_focus_direction(&event.keystroke)
        {
            let tools = group.spec().tools;
            let target = match direction {
                AnnotationToolGroupFocusDirection::Next => tools.first().copied(),
                AnnotationToolGroupFocusDirection::Previous => tools.last().copied(),
            };
            if let Some(target) = target
                && let Some(focus_handle) = self.annotation_tool_group_item_focus_handles
                    [group.index()]
                .iter()
                .zip(tools.iter())
                .find_map(|(focus_handle, candidate)| {
                    (*candidate == target).then(|| focus_handle.clone())
                })
            {
                focus_handle.focus(window, cx);
            }
            cx.stop_propagation();
            return;
        }
        stop_overlay_action_key_propagation(event, window, cx);
    }

    /// Dismisses a group before a canvas gesture can start and returns focus to that canvas.
    fn dismiss_annotation_tool_group_from_pointer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_handle.focus(window, cx);
        let operation_generation = self.operation_generation;
        let app = self.app.clone();
        cx.defer(move |cx| {
            app.update(cx, |app, cx| {
                if accepts_overlay_input(operation_generation, app.operation_generation) {
                    app.close_annotation_tool_group();
                    cx.notify();
                }
            });
        });
    }

    /// Dismisses More before a canvas gesture can start and returns focus to that canvas.
    fn dismiss_more_actions_from_pointer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
        let operation_generation = self.operation_generation;
        let app = self.app.clone();
        cx.defer(move |cx| {
            app.update(cx, |app, cx| {
                if accepts_overlay_input(operation_generation, app.operation_generation) {
                    app.close_overlay_more_actions();
                    cx.notify();
                }
            });
        });
    }

    /// Opens More from the keyboard and waits one frame before focusing its first rendered action.
    fn open_more_actions_from_keyboard(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let owns_toolbar = {
            let app = self.app.read(cx);
            app.text_edit().is_none()
                && app.session.selection().is_some_and(|selection| {
                    owns_selection_toolbar(selection, self.display.physical_bounds)
                })
        };
        if !owns_toolbar {
            return false;
        }

        self.app.update(cx, |app, cx| {
            if !app.overlay_more_actions {
                app.toggle_overlay_more_actions(cx);
            }
        });
        let first_action = self.secondary_action_focus_handle(SecondaryAction::SaveAnnotations);
        cx.on_next_frame(window, move |_, window, cx| first_action.focus(window, cx));
        true
    }

    /// Closes More before Escape reaches the capture-wide cancellation shortcut.
    fn close_more_actions_from_keyboard(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let can_close = {
            let app = self.app.read(cx);
            app.text_edit().is_none() && app.overlay_more_actions
        };
        if !can_close {
            return false;
        }

        self.app
            .update(cx, |app, cx| app.toggle_overlay_more_actions(cx));
        let more_actions = self.more_actions_focus_handle.clone();
        cx.on_next_frame(window, move |_, window, cx| more_actions.focus(window, cx));
        true
    }

    /// Returns the persistent focus handle for a menu action across conditional re-renders.
    fn secondary_action_focus_handle(&self, action: SecondaryAction) -> FocusHandle {
        self.secondary_action_focus_handles[action.index()].clone()
    }

    /// Restores focus to a persistent More action after a transient action removes itself.
    fn focus_secondary_action_after_render(
        &self,
        action: SecondaryAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let action_focus = self.secondary_action_focus_handle(action);
        cx.on_next_frame(window, move |_, window, cx| action_focus.focus(window, cx));
    }
}

/// Accepts deferred overlay input only while it still belongs to the active workflow.
const fn accepts_overlay_input(overlay_generation: u64, current_generation: u64) -> bool {
    overlay_generation == current_generation
}

/// Opens one disposable overlay with a seeded visual state for native screenshot acceptance.
pub(super) fn open_ui_acceptance(
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
    let preview = super::render_image::render_image_from_capture(&frame)?.image;
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
fn overlay_ui_acceptance_frame(bounds: PhysicalRect) -> io::Result<CaptureFrame> {
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
fn overlay_ui_acceptance_target(bounds: PhysicalRect, kind: InspectionKind) -> InspectionTarget {
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
fn overlay_ui_acceptance_selection(
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

impl Focusable for CaptureOverlay {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for CaptureOverlay {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.topmost_requested
            && let Ok(handle) = window.window_handle()
            && let RawWindowHandle::Win32(handle) = handle.as_raw()
        {
            self.topmost_requested = true;
            let hwnd = handle.hwnd.get();
            // Keep the active capture overlay above ordinary application windows so its
            // selection controls remain available until the capture is completed or cancelled.
            cx.defer(move |_| {
                if let Err(error) = crate::platform::window_visibility::make_topmost(hwnd) {
                    log::warn!(target: "flash_shot::overlay", "overlay_topmost_failed error={error}");
                }
            });
        }
        let display_bounds = self.display.physical_bounds;
        let app = self.app.read(cx);
        let colors = app.colors;
        let workspace_colors = ThemeColors::for_mode(ThemeMode::Dark);
        let locale = app.settings.locale;
        // The session owns the committed selection. Keep rendering it after the
        // drag has ended, even if a late pointer event clears transient UI state.
        let selection = visible_selection(app.selection_drag, app.session.selection());
        let inspection_target = app.inspection_target;
        let annotations = app
            .annotation_document
            .as_ref()
            .map(|document| document.annotations().to_vec())
            .unwrap_or_default();
        let layer_annotations = annotations.clone();
        let annotation_preview = app
            .annotation_document
            .as_ref()
            .and_then(|document| app.annotation_editor.preview(document.canvas_bounds()));
        let text_edit = app.text_edit().cloned();
        let text_edit_annotation = app.text_edit_annotation();
        let selected_annotation = app.selected_annotation;
        let selected_annotation_object =
            selected_annotation.and_then(|id| app.annotation_document.as_ref()?.annotation(id));
        let can_delete = selected_annotation.is_some();
        let selected_tool = app.annotation_tool;
        let can_edit_text = selected_annotation_object.is_some_and(|annotation| {
            matches!(
                annotation.kind,
                AnnotationKind::Text { .. } | AnnotationKind::Watermark { .. }
            )
        });
        let can_rotate =
            selected_annotation_object.is_some_and(Annotation::supports_clockwise_rotation);
        let style_capabilities =
            annotation_style_capabilities(selected_tool, selected_annotation_object);
        let selected_number = selected_annotation.and_then(|id| {
            app.annotation_document
                .as_ref()?
                .annotation(id)
                .and_then(|annotation| match annotation.kind {
                    AnnotationKind::Number { value, .. } => Some(value),
                    _ => None,
                })
        });
        let annotation_color = app.annotation_style.stroke_rgba;
        let annotation_width = app.annotation_style.stroke_width;
        let annotation_font_size = app.annotation_style.text_font_size;
        let annotation_opacity = (app.annotation_style.stroke_rgba & 0xFF) as u8;
        let fill_enabled = app.annotation_style.fill_rgba.is_some();
        let can_undo = app.annotation_history.undo_len() > 0;
        let can_redo = app.annotation_history.redo_len() > 0;
        let status = app.status.clone();
        let show_more_actions = app.overlay_more_actions;
        let recognition_result = app.recognition_result.clone();
        let recognition_retry = app.recognition_retry;
        let recognition_in_flight = app.recognition_in_flight;
        let mut visible_secondary_actions = ALWAYS_VISIBLE_SECONDARY_ACTIONS.to_vec();
        if recognition_retry.is_some() {
            visible_secondary_actions.push(SecondaryAction::RetryRecognition);
        }
        if recognition_result.is_some() {
            visible_secondary_actions.extend([
                SecondaryAction::CopyRecognition,
                SecondaryAction::ClearRecognition,
            ]);
        }
        let secondary_navigation = SecondaryActionNavigation {
            current: SecondaryAction::SaveAnnotations,
            visible_actions: visible_secondary_actions,
            focus_handles: self.secondary_action_focus_handles.clone(),
        };
        let more_actions_navigation = secondary_navigation.clone();
        let hover_pixel = app.hover_pixel;
        let frame = app.frame.clone();
        let viewport = local_viewport(window);
        self.annotation_arrange_actions_for =
            arrange_context_for_selection(self.annotation_arrange_actions_for, selected_annotation);
        let show_annotation_arrange_actions = self.annotation_arrange_actions_for.is_some();
        let annotation_toolbar_items = annotation_toolbar_items(
            can_delete,
            can_edit_text,
            selected_number.is_some(),
            can_rotate,
            show_annotation_arrange_actions,
        );
        let transform = self.transform(viewport);
        let selected_on_display =
            selection.and_then(|selection| intersect(selection, display_bounds));
        let annotation_tool_width = if locale == Locale::SimplifiedChinese {
            ThemeMetrics::WORKSPACE_TOOL_CELL_WIDTH_COMPACT
        } else {
            ANNOTATION_TOOL_ESTIMATED_WIDTH
        };
        let owns_action_toolbar =
            selection.is_some_and(|selection| owns_selection_toolbar(selection, display_bounds));
        let show_annotation_controls =
            annotation_controls_visible(app.overlay_annotation_controls, selection, display_bounds);
        let annotation_tool_group = app.annotation_tool_group.filter(|_| {
            app.annotation_tool_group_owner.as_deref() == Some(self.display.id.as_str())
        });
        let show_annotation_tool_group_dismiss = app.annotation_tool_group.is_some();
        let show_annotation_style = show_annotation_controls && style_capabilities.has_controls();
        let layout_snapshot = workspace_layout_snapshot(WorkspaceLayoutInput {
            selection,
            display_bounds,
            transform,
            viewport,
            hover_pixel,
            inspection_target,
            show_annotation_controls,
            annotation_toolbar_items,
            annotation_style_capabilities: style_capabilities,
            annotation_tool_group,
            annotation_tool_width,
            has_recognition_result: recognition_result.is_some(),
            has_recognition_retry: recognition_retry.is_some(),
            recognition_in_flight,
        });
        let action_layout = layout_snapshot.action_toolbar;
        let annotation_layout = layout_snapshot.annotation_toolbar;
        let annotation_layer_layout = layout_snapshot.annotation_layer;
        let show_annotation_layers = owns_action_toolbar
            && !layer_annotations.is_empty()
            && (!show_annotation_controls || annotation_layer_layout.is_some());
        let annotation_layer_top = annotation_layer_layout
            .map(|layout| layout.top)
            .unwrap_or(OVERLAY_EDGE_INSET);
        let annotation_layer_max_height = annotation_layer_layout
            .map(|layout| layout.max_height)
            .unwrap_or_else(|| {
                (view_rect(viewport).height - annotation_layer_top - OVERLAY_BOTTOM_SAFE_INSET)
                    .max(80.0)
            });
        let secondary_menu = layout_snapshot.secondary_menu;
        let dimension_layout = layout_snapshot.dimension;
        let status_inset = layout_snapshot.status_inset;
        let smart_target_hud = layout_snapshot.smart_target_hud;
        let target_on_display = selection
            .is_none()
            .then(|| inspection_target.and_then(|target| intersect(target.bounds, display_bounds)))
            .flatten();
        let has_selection = selection.is_some();
        let selection_copy_in_progress = app.selection_copy_is_active();
        // Copy owns a frozen snapshot, not this editor. Keep the normal action bar available so
        // large PNG/DIB encoding never turns a simple Copy into a workflow dead end.
        let can_export = has_selection && owns_action_toolbar;
        let show_action_toolbar = !has_selection || owns_action_toolbar;
        let selection_cursor = selection_cursor(
            selection,
            transform,
            view_point(window.mouse_position()),
            selected_tool.is_some(),
            app.selection_drag.is_moving(),
        );
        let text_input_focus = self.focus_handle.clone();
        let text_input_app = self.app.clone();

        div()
            .size_full()
            .relative()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .bg(colors.background)
            .child(
                img(self.preview.clone())
                    .size_full()
                    .object_fit(ObjectFit::Fill),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .cursor_crosshair()
                    .when(selection_cursor == SelectionCursor::Move, |overlay| {
                        overlay.cursor_move()
                    })
                    .when(selection_cursor == SelectionCursor::ResizeNwse, |overlay| {
                        overlay.cursor_nwse_resize()
                    })
                    .when(selection_cursor == SelectionCursor::ResizeNesw, |overlay| {
                        overlay.cursor_nesw_resize()
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event, window, cx| {
                            this.focus_handle.focus(window, cx);
                            this.begin_selection(event, local_viewport(window), cx);
                        }),
                    )
                    .on_mouse_move(cx.listener(move |this, event, window, cx| {
                        let viewport = local_viewport(window);
                        this.update_selection(event, viewport, window, cx)
                    }))
                    .capture_any_mouse_up(cx.listener(
                        move |this, event: &MouseUpEvent, window, cx| {
                            if event.button == MouseButton::Left && this.selection_pointer_active {
                                this.finish_selection(event, local_viewport(window), cx);
                            }
                        },
                    ))
                    .child(
                        canvas(
                            move |bounds, _, _| {
                                (bounds, transform, selected_on_display, target_on_display)
                            },
                            move |bounds, (_, transform, selection, target), window, _| {
                                paint_selection_mask(
                                    window, bounds, transform, selection, target, colors,
                                )
                            },
                        )
                        .size_full(),
                    ),
            )
            .child(
                canvas(
                    move |bounds, _, _| {
                        (
                            bounds,
                            transform,
                            annotations,
                            annotation_preview,
                            selected_annotation,
                            text_edit_annotation,
                            text_edit,
                        )
                    },
                    move |bounds,
                          (
                        _,
                        transform,
                        annotations,
                        preview,
                        selected_annotation,
                        text_edit_annotation,
                        text_edit,
                    ),
                          window,
                          cx| {
                        // GPUI only accepts native IME registration during paint. Keeping it in
                        // this canvas lets text and watermark edits receive input without
                        // triggering a paint-phase assertion from the Windows backend.
                        if text_edit.is_some() {
                            window.handle_input(
                                &text_input_focus,
                                ElementInputHandler::new(bounds, text_input_app.clone()),
                                cx,
                            );
                        }
                        paint_annotations(
                            window,
                            transform,
                            &annotations,
                            preview.as_ref(),
                            AnnotationPaintState {
                                selected: selected_annotation,
                                hidden: text_edit_annotation,
                            },
                            colors,
                            cx,
                        );
                        if let Some(edit) = text_edit {
                            paint_text_annotation(
                                window,
                                transform.unwrap_or_else(|| unreachable!()),
                                edit.origin,
                                &edit.content,
                                0xFFFFFFFF,
                                annotation_font_size,
                                cx,
                            );
                        }
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .bottom_0(),
            )
            .child(
                canvas(
                    move |bounds, _, _| (bounds, transform, hover_pixel, frame),
                    move |viewport, (_, transform, hover_pixel, frame), window, _| {
                        paint_magnifier(window, viewport, transform, hover_pixel, frame.as_ref())
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .bottom_0(),
            )
            .when_some(dimension_layout, |overlay, layout| {
                let selection = selected_on_display.expect("dimension layout requires selection");
                overlay.child(
                    div()
                        .id("overlay-selection-dimensions")
                        .occlude()
                        .absolute()
                        .left(px(layout.left))
                        .top(px(layout.top))
                        .w(px(OVERLAY_DIMENSION_LABEL_WIDTH))
                        .h(px(OVERLAY_DIMENSION_LABEL_HEIGHT))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_sm()
                        .bg(rgba(0x0B0D10E6))
                        .border_1()
                        .border_color(colors.accent)
                        .text_color(colors.overlay_text)
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .shadow_lg()
                        .child(selection_dimension_label(locale, selection)),
                )
            })
            .when_some(smart_target_hud, |overlay, layout| {
                // Keep this read-only HUD out of hit testing so selection gestures still reach
                // the full-screen canvas behind it.
                overlay.child(
                    div()
                        .absolute()
                        .left(px(layout.left))
                        .top(px(layout.top))
                        .w(px(layout.width))
                        .h(px(OVERLAY_SMART_TARGET_HUD_HEIGHT))
                        .px_2()
                        .flex()
                        .items_center()
                        .rounded_sm()
                        .border_1()
                        .border_color(colors.accent)
                        .bg(rgba(0x0B0D10E6))
                        .text_color(colors.overlay_text)
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .shadow_lg()
                        .child(
                            div()
                                .w_full()
                                .min_w(px(0.0))
                                .text_ellipsis()
                                .child(smart_target_hud_label(locale, layout.target)),
                        ),
                )
            })
            .when_some(annotation_layout, |overlay, layout| {
                overlay.child(
                    div()
                        .id("overlay-marking-panel")
                        .occlude()
                        .absolute()
                        .left(px(layout.left))
                        .top(px(layout.top))
                        .w(px(layout.width))
                        .h(px(layout.height)),
                )
            })
            .when(show_annotation_layers, |overlay| {
                overlay.child(
                    div()
                        .id("overlay-layers")
                        .occlude()
                        .absolute()
                        .when_some(annotation_layer_layout, |layers, layout| {
                            layers.left(px(layout.left))
                        })
                        .when(annotation_layer_layout.is_none(), |layers| {
                            layers.right(px(OVERLAY_EDGE_INSET))
                        })
                        .top(px(annotation_layer_top))
                        .w(px(ANNOTATION_LAYERS_WIDTH))
                        .max_h(px(annotation_layer_max_height))
                        .overflow_y_scroll()
                        .p_2()
                        .bg(workspace_colors.toolbar_surface)
                        .border_1()
                        .border_color(workspace_colors.toolbar_border)
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_sm()
                                .text_color(workspace_colors.overlay_muted)
                                .child(locale.text(UiText::OverlayLayers)),
                        )
                        .children(layer_annotations.iter().rev().enumerate().map(
                            |(reverse_index, annotation)| {
                                let id = annotation.id;
                                let position = layer_annotations.len() - reverse_index;
                                let is_selected = selected_annotation == Some(id);
                                div()
                                    .id(format!("overlay-layer-{}", id.value()))
                                    .px_2()
                                    .py_1()
                                    .bg(if is_selected {
                                        workspace_colors.accent
                                    } else {
                                        workspace_colors.toolbar_surface
                                    })
                                    .text_color(if is_selected {
                                        workspace_colors.background
                                    } else {
                                        workspace_colors.text
                                    })
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let app = this.app.clone();
                                        cx.defer(move |cx| {
                                            app.update(cx, |app, cx| {
                                                app.select_annotation_layer(id, cx);
                                            });
                                        });
                                    }))
                                    .child(annotation_layer_entry_label(
                                        locale,
                                        position,
                                        &annotation.kind,
                                    ))
                            },
                        )),
                )
            })
            .when(show_annotation_tool_group_dismiss, |overlay| {
                overlay.child(
                    div()
                        .id("overlay-annotation-tool-group-dismiss")
                        .occlude()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, window, cx| {
                                this.dismiss_annotation_tool_group_from_pointer(window, cx);
                            }),
                        ),
                )
            })
            .when(show_more_actions, |overlay| {
                overlay.child(
                    div()
                        .id("overlay-more-actions-dismiss")
                        .occlude()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, window, cx| {
                                this.dismiss_more_actions_from_pointer(window, cx);
                            }),
                        ),
                )
            })
            .when(
                show_annotation_controls
                    && (annotation_tool_group.is_some() || selected_annotation.is_some()),
                |overlay| {
                overlay.child(
                    div()
                        .id("overlay-annotation-tools")
                        .absolute()
                        .when_some(annotation_layout, |tools, layout| {
                            tools
                                .left(px(layout.left))
                                .top(px(layout.tools_top))
                                .w(px(layout.tools_width))
                        })
                        .when(annotation_layout.is_none(), |tools| {
                            tools
                                .left(px(OVERLAY_EDGE_INSET))
                                .right(px(OVERLAY_EDGE_INSET))
                                .top(px(OVERLAY_EDGE_INSET))
                        })
                        .flex_col()
                        .gap(px(ANNOTATION_TOOL_GAP))
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        // Reserve the same row height as the inline palette. The old standalone
                        // palette is intentionally absent so the reference toolbar stays one row;
                        // the child popover below still uses this anchor when a group is open.
                        .child(div().h(px(ANNOTATION_TOOL_PALETTE_HEIGHT)))
                        .when_some(annotation_tool_group, |tools, group| {
                            let popup_width = layout_snapshot
                                .annotation_tool_group
                                .map(|layout| layout.width)
                                .unwrap_or_else(|| {
                                    annotation_tool_group_popover_width(
                                        annotation_layout
                                            .map(|layout| layout.tools_width)
                                            .unwrap_or_else(|| {
                                                (view_rect(viewport).width
                                                    - OVERLAY_EDGE_INSET * 2.0)
                                                    .clamp(1.0, ANNOTATION_TOOLBAR_MAX_WIDTH)
                                            }),
                                        group,
                                        annotation_tool_width,
                                    )
                                });
                            tools.child(
                                workspace_surface(workspace_colors, true)
                                    .id(format!(
                                        "overlay-tool-group-popover-{}",
                                        annotation_tool_group_key(group)
                                    ))
                                    .occlude()
                                    .w(px(popup_width))
                                    .p(px(ANNOTATION_TOOL_GROUP_POPUP_PADDING))
                                    .flex()
                                    .flex_wrap()
                                    .gap(px(ANNOTATION_TOOL_GAP))
                                    .children(group.spec().tools.iter().copied().enumerate().map(
                                        |(index, tool)| {
                                            let focus_handle = self
                                                .annotation_tool_group_item_focus_handle(
                                                    group, index,
                                                );
                                            let active = selected_tool == Some(tool);
                                            annotation_tool_button(
                                                format!(
                                                    "overlay-tool-group-{}-{}",
                                                    annotation_tool_group_key(group),
                                                    annotation_tool_key(tool)
                                                ),
                                                locale.text(annotation_tool_ui_text(tool)),
                                                workspace_colors,
                                                active,
                                                annotation_tool_width,
                                                None,
                                                cx.listener(move |this, _, window, cx| {
                                                    this.select_annotation_tool_from_group(
                                                        group, tool, window, cx,
                                                    );
                                                }),
                                            )
                                            .track_focus(&focus_handle)
                                            .on_key_down(cx.listener(
                                                move |this, event, window, cx| {
                                                    this.handle_annotation_tool_group_item_key_down(
                                                        group, tool, event, window, cx,
                                                    );
                                                },
                                            ))
                                        },
                                    )),
                            )
                        })
                        .when_some(selected_annotation, |tools, selected_id| {
                            tools.child(
                                div()
                                    .id("overlay-selection-context")
                                    .w_full()
                                    .pt_2()
                                    .border_t_1()
                                    .border_color(workspace_colors.toolbar_border)
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .h(px(ANNOTATION_ACTION_HEIGHT))
                                            .px_2()
                                            .flex()
                                            .items_center()
                                            .text_color(workspace_colors.overlay_muted)
                                            .text_xs()
                                            .child(locale.text(UiText::OverlaySelected)),
                                    )
                                    .child(annotation_action_button(
                                        "overlay-delete",
                                        locale.text(UiText::OverlayDelete),
                                        workspace_colors,
                                        AnnotationActionTone::Destructive,
                                        true,
                                        cx.listener(|this, _, _, cx| {
                                            let app = this.app.clone();
                                            cx.defer(move |cx| {
                                                app.update(cx, |app, cx| {
                                                    app.delete_selected_annotation(cx);
                                                });
                                            });
                                        }),
                                    ))
                                    .when(can_edit_text, |actions| {
                                        actions.child(annotation_action_button(
                                            "overlay-edit-text",
                                            locale.text(UiText::OverlayEditText),
                                            workspace_colors,
                                            AnnotationActionTone::Primary,
                                            true,
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.edit_selected_text_annotation(cx);
                                                    });
                                                });
                                            }),
                                        ))
                                    })
                                    .when_some(selected_number, |actions, value| {
                                        actions
                                            .child(annotation_action_button(
                                                "overlay-number-decrement",
                                                "-",
                                                workspace_colors,
                                                AnnotationActionTone::Neutral,
                                                true,
                                                cx.listener(|this, _, _, cx| {
                                                    let app = this.app.clone();
                                                    cx.defer(move |cx| {
                                                        app.update(cx, |app, cx| {
                                                            app.adjust_selected_number(-1, cx);
                                                        });
                                                    });
                                                }),
                                            ))
                                            .child(
                                                div()
                                                    .h(px(ANNOTATION_ACTION_HEIGHT))
                                                    .px_2()
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .rounded_md()
                                                    .bg(workspace_colors.toolbar_elevated)
                                                    .text_color(workspace_colors.text)
                                                    .child(value.to_string()),
                                            )
                                            .child(annotation_action_button(
                                                "overlay-number-increment",
                                                "+",
                                                workspace_colors,
                                                AnnotationActionTone::Neutral,
                                                true,
                                                cx.listener(|this, _, _, cx| {
                                                    let app = this.app.clone();
                                                    cx.defer(move |cx| {
                                                        app.update(cx, |app, cx| {
                                                            app.adjust_selected_number(1, cx);
                                                        });
                                                    });
                                                }),
                                            ))
                                    })
                                    .child(annotation_action_button(
                                        "overlay-duplicate",
                                        locale.text(UiText::OverlayDuplicate),
                                        workspace_colors,
                                        AnnotationActionTone::Neutral,
                                        true,
                                        cx.listener(|this, _, _, cx| {
                                            let app = this.app.clone();
                                            cx.defer(move |cx| {
                                                app.update(cx, |app, cx| {
                                                    app.duplicate_selected_annotation(cx);
                                                });
                                            });
                                        }),
                                    ))
                                    .child(annotation_action_button(
                                        "overlay-selection-arrange-toggle",
                                        locale.text(UiText::OverlayArrange),
                                        workspace_colors,
                                        if show_annotation_arrange_actions {
                                            AnnotationActionTone::Primary
                                        } else {
                                            AnnotationActionTone::Neutral
                                        },
                                        true,
                                        cx.listener(move |this, _, _, cx| {
                                            this.annotation_arrange_actions_for = if this
                                                .annotation_arrange_actions_for
                                                == Some(selected_id)
                                            {
                                                None
                                            } else {
                                                Some(selected_id)
                                            };
                                            cx.notify();
                                        }),
                                    )),
                            )
                        })
                        .when(show_annotation_arrange_actions, |tools| {
                            tools.child(
                                div()
                                    .id("overlay-selection-arrange-context")
                                    .w_full()
                                    .pt_2()
                                    .border_t_1()
                                    .border_color(workspace_colors.toolbar_border)
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .h(px(ANNOTATION_ACTION_HEIGHT))
                                            .px_2()
                                            .flex()
                                            .items_center()
                                            .text_color(workspace_colors.overlay_muted)
                                            .text_xs()
                                            .child(locale.text(UiText::OverlayArrange)),
                                    )
                                    .when(can_rotate, |actions| {
                                        actions.child(annotation_action_button(
                                            "overlay-rotate-clockwise",
                                            locale.text(UiText::OverlayRotate90),
                                            workspace_colors,
                                            AnnotationActionTone::Neutral,
                                            true,
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.rotate_selected_annotation_clockwise(
                                                            cx,
                                                        );
                                                    });
                                                });
                                            }),
                                        ))
                                    })
                                    .child(annotation_action_button(
                                        "overlay-bring-forward",
                                        locale.text(UiText::OverlayBringForward),
                                        workspace_colors,
                                        AnnotationActionTone::Neutral,
                                        true,
                                        cx.listener(|this, _, _, cx| {
                                            let app = this.app.clone();
                                            cx.defer(move |cx| {
                                                app.update(cx, |app, cx| {
                                                    app.bring_selected_annotation_forward(cx);
                                                });
                                            });
                                        }),
                                    ))
                                    .child(annotation_action_button(
                                        "overlay-send-backward",
                                        locale.text(UiText::OverlaySendBackward),
                                        workspace_colors,
                                        AnnotationActionTone::Neutral,
                                        true,
                                        cx.listener(|this, _, _, cx| {
                                            let app = this.app.clone();
                                            cx.defer(move |cx| {
                                                app.update(cx, |app, cx| {
                                                    app.send_selected_annotation_backward(cx);
                                                });
                                            });
                                        }),
                                    ))
                                    .child(annotation_action_button(
                                        "overlay-bring-to-front",
                                        locale.text(UiText::OverlayBringToFront),
                                        workspace_colors,
                                        AnnotationActionTone::Neutral,
                                        true,
                                        cx.listener(|this, _, _, cx| {
                                            let app = this.app.clone();
                                            cx.defer(move |cx| {
                                                app.update(cx, |app, cx| {
                                                    app.bring_selected_annotation_to_front(cx);
                                                });
                                            });
                                        }),
                                    ))
                                    .child(annotation_action_button(
                                        "overlay-send-to-back",
                                        locale.text(UiText::OverlaySendToBack),
                                        workspace_colors,
                                        AnnotationActionTone::Neutral,
                                        true,
                                        cx.listener(|this, _, _, cx| {
                                            let app = this.app.clone();
                                            cx.defer(move |cx| {
                                                app.update(cx, |app, cx| {
                                                    app.send_selected_annotation_to_back(cx);
                                                });
                                            });
                                        }),
                                    )),
                            )
                        }),
                )
            })
            .when(show_annotation_style, |overlay| {
                let style_row = workspace_surface(workspace_colors, false)
                    .id("overlay-annotation-style-row")
                    .occlude()
                    .absolute()
                    .when_some(annotation_layout, |style, layout| {
                        style
                            .left(px(layout.style_left))
                            .top(px(layout.style_top))
                            .w(px(layout.tools_width))
                            .h(px(layout.style_height))
                    })
                    .when(annotation_layout.is_none(), |style| {
                        style
                            .left(px(OVERLAY_EDGE_INSET))
                            .right(px(OVERLAY_EDGE_INSET))
                            .top(px(OVERLAY_EDGE_INSET))
                    })
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(ANNOTATION_STYLE_CONTROL_GAP))
                    .p(px(ANNOTATION_STYLE_ROW_PADDING))
                    .children(
                        [
                            style_capabilities.color.then(|| {
                                div()
                                    .id("overlay-style-colors")
                                    .flex()
                                    .items_center()
                                    .gap(px(ANNOTATION_STYLE_CONTROL_GAP))
                                    .children(ANNOTATION_COLORS.into_iter().map(|color| {
                                        workspace_swatch(
                                            format!("overlay-color-{color:08x}"),
                                            gpui::Hsla::from(rgba(color)),
                                            color == annotation_color,
                                            locale.text(UiText::AnnotationColorSelected),
                                            locale.text(UiText::AnnotationColorSelected),
                                            workspace_colors,
                                            cx.listener(move |this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.select_annotation_color(color, cx)
                                                    });
                                                });
                                            }),
                                        )
                                    }))
                            }),
                            style_capabilities.fill.then(|| {
                                div()
                                    .id("overlay-style-fill-group")
                                    .border_l_1()
                                    .border_color(workspace_colors.toolbar_border)
                                    .pl_2()
                                    .child(annotation_style_button(
                                        "overlay-fill",
                                        locale.text(UiText::OverlayFill),
                                        locale.text(UiText::OverlayFill),
                                        WorkspaceButtonConfig::text(
                                            Some(ANNOTATION_STYLE_FILL_WIDTH),
                                            ANNOTATION_STYLE_CONTROL_HEIGHT,
                                            workspace_colors,
                                            WorkspaceButtonTone::Neutral,
                                            fill_enabled,
                                            true,
                                            Some(locale.text(UiText::OverlayFill)),
                                        ),
                                        cx.listener(|this, _, _, cx| {
                                            let app = this.app.clone();
                                            cx.defer(move |cx| {
                                                app.update(cx, |app, cx| {
                                                    app.toggle_annotation_fill(cx)
                                                });
                                            });
                                        }),
                                    ))
                            }),
                            style_capabilities.width.then(|| {
                                div()
                                    .id("overlay-style-widths")
                                    .border_l_1()
                                    .border_color(workspace_colors.toolbar_border)
                                    .pl_2()
                                    .flex()
                                    .items_center()
                                    .gap(px(ANNOTATION_STYLE_CONTROL_GAP))
                                    .children(ANNOTATION_WIDTHS.into_iter().map(|width| {
                                        let value = width.to_string();
                                        let aria_label = locale.format_template(
                                            UiText::AnnotationWidth,
                                            &[("width", &value)],
                                        );
                                        annotation_style_button(
                                            format!("overlay-width-{width}"),
                                            value,
                                            aria_label,
                                            WorkspaceButtonConfig::text(
                                                Some(ANNOTATION_STYLE_VALUE_WIDTH),
                                                ANNOTATION_STYLE_CONTROL_HEIGHT,
                                                workspace_colors,
                                                WorkspaceButtonTone::Neutral,
                                                width == annotation_width,
                                                true,
                                                Some(locale.text(UiText::AnnotationWidthLabel)),
                                            ),
                                            cx.listener(move |this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.select_annotation_width(width, cx)
                                                    });
                                                });
                                            }),
                                        )
                                    }))
                            }),
                            style_capabilities.opacity.then(|| {
                                div()
                                    .id("overlay-style-opacity")
                                    .border_l_1()
                                    .border_color(workspace_colors.toolbar_border)
                                    .pl_2()
                                    .flex()
                                    .items_center()
                                    .gap(px(ANNOTATION_STYLE_CONTROL_GAP))
                                    .children(ANNOTATION_OPACITIES.into_iter().map(|opacity| {
                                        let percent = (u16::from(opacity) * 100 / 255).to_string();
                                        let aria_label = locale.format_template(
                                            UiText::AnnotationOpacity,
                                            &[("percent", &percent)],
                                        );
                                        annotation_style_button(
                                            format!("overlay-opacity-{opacity}"),
                                            annotation_opacity_value_label(locale, opacity),
                                            aria_label,
                                            WorkspaceButtonConfig::text(
                                                Some(ANNOTATION_STYLE_OPACITY_WIDTH),
                                                ANNOTATION_STYLE_CONTROL_HEIGHT,
                                                workspace_colors,
                                                WorkspaceButtonTone::Neutral,
                                                opacity == annotation_opacity,
                                                true,
                                                Some(locale.text(UiText::AnnotationOpacityLabel)),
                                            ),
                                            cx.listener(move |this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.select_annotation_opacity(opacity, cx)
                                                    });
                                                });
                                            }),
                                        )
                                    }))
                            }),
                            style_capabilities.font_size.then(|| {
                                div()
                                    .id("overlay-style-font-sizes")
                                    .border_l_1()
                                    .border_color(workspace_colors.toolbar_border)
                                    .pl_2()
                                    .flex()
                                    .items_center()
                                    .gap(px(ANNOTATION_STYLE_CONTROL_GAP))
                                    .children(ANNOTATION_FONT_SIZES.into_iter().map(|font_size| {
                                        let value = font_size.to_string();
                                        let aria_label = locale.format_template(
                                            UiText::AnnotationTextSize,
                                            &[("size", &value)],
                                        );
                                        annotation_style_button(
                                            format!("overlay-font-size-{font_size}"),
                                            value,
                                            aria_label,
                                            WorkspaceButtonConfig::text(
                                                Some(ANNOTATION_STYLE_VALUE_WIDTH),
                                                ANNOTATION_STYLE_CONTROL_HEIGHT,
                                                workspace_colors,
                                                WorkspaceButtonTone::Neutral,
                                                font_size == annotation_font_size,
                                                true,
                                                Some(locale.text(UiText::AnnotationTextSizeLabel)),
                                            ),
                                            cx.listener(move |this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.select_annotation_font_size(
                                                            font_size, cx,
                                                        )
                                                    });
                                                });
                                            }),
                                        )
                                    }))
                            }),
                        ]
                        .into_iter()
                        .flatten(),
                    );
                overlay.child(style_row)
            })
            .child(
                div()
                    .id("overlay-status")
                    .occlude()
                    .absolute()
                    .left(px(18.0))
                    .bottom(px(status_inset))
                    .px_3()
                    .py_2()
                    .rounded_sm()
                    .border_1()
                    .border_color(rgba(0xFFFFFF24))
                    .bg(rgba(0x0B0D10E6))
                    .text_color(colors.overlay_text)
                    .text_sm()
                    .shadow_lg()
                    .child(status),
            )
            .child(
                div()
                    // These controls sit over the full-screen selection canvas. Their hitbox must
                    // keep a command click from also starting or completing a selection underneath.
                    .id("overlay-actions")
                    .occlude()
                    .absolute()
                    .when(!show_action_toolbar, |actions| actions.hidden())
                    .when_some(action_layout, |actions, layout| {
                        actions
                            .left(px(layout.left))
                            .top(px(layout.top))
                            .w(px(layout.width))
                    })
                    .when(action_layout.is_none(), |actions| {
                        actions
                            .right(px(OVERLAY_EDGE_INSET))
                            .bottom(px(OVERLAY_BOTTOM_SAFE_INSET))
                    })
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap(px(OVERLAY_ACTION_ITEM_GAP))
                    .p(px(OVERLAY_ACTION_BAR_PADDING))
                    .rounded(px(ThemeMetrics::default().radius_md))
                    .border_1()
                    .border_color(workspace_colors.toolbar_border)
                    .bg(workspace_colors.toolbar_surface)
                    .shadow_lg()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .when(can_export, |actions| {
                        actions
                            .when(show_annotation_controls, |actions| {
                                actions.child(
                                    div()
                                        .id("overlay-inline-annotation-tools")
                                        .flex()
                                        .items_center()
                                        .gap(px(ANNOTATION_TOOL_PALETTE_GAP))
                                        .child(annotation_icon_button(
                                            "overlay-tool-selection",
                                            icon::MOVE,
                                            locale.text(UiText::OverlaySelect),
                                            locale.text(UiText::OverlaySelect),
                                            workspace_colors,
                                            selected_tool.is_none(),
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.select_selection_tool(cx);
                                                    });
                                                });
                                             }),
                                         ))
                                         .child(workspace_separator(
                                             "overlay-tool-selection-separator",
                                             workspace_colors,
                                         ))
                                         .children(ANNOTATION_TOOL_GROUP_SPECS.iter().copied().map(
                                            |spec| {
                                                let group = spec.group;
                                                let active = annotation_tool_group == Some(group)
                                                    || selected_tool.is_some_and(|tool| {
                                                        spec.tools.contains(&tool)
                                                    });
                                                let focus_handle = self
                                                    .annotation_tool_group_trigger_focus_handle(group);
                                                annotation_icon_button(
                                                    format!(
                                                        "overlay-tool-group-{}",
                                                        annotation_tool_group_key(group)
                                                    ),
                                                    annotation_tool_group_icon(group),
                                                    locale.text(spec.label),
                                                    locale.text(spec.tooltip),
                                                    workspace_colors,
                                                    active,
                                                    cx.listener(move |this, _, window, cx| {
                                                        this.toggle_annotation_tool_group_from_trigger(
                                                            group, window, cx,
                                                        );
                                                    }),
                                                )
                                                .track_focus(&focus_handle)
                                                .on_key_down(cx.listener(
                                                    move |this, event, window, cx| {
                                                        this.handle_annotation_tool_group_trigger_key_down(
                                                            group, event, window, cx,
                                                        );
                                                    },
                                                ))
                                            },
                                        ))
                                        .child(annotation_icon_button(
                                            "overlay-tool-highlight",
                                            icon::MARK,
                                            locale.text(UiText::OverlayHighlight),
                                            locale.text(UiText::OverlayHighlight),
                                            workspace_colors,
                                            selected_tool == Some(AnnotationTool::Highlight),
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.select_annotation_tool(
                                                            AnnotationTool::Highlight,
                                                            cx,
                                                        );
                                                    });
                                                });
                                            }),
                                        )),
                                )
                            })
                            .when(show_annotation_controls, |actions| {
                                actions.child(workspace_separator(
                                    "overlay-action-context-separator",
                                    workspace_colors,
                                ))
                            })
                            .when(show_annotation_controls, |actions| {
                                actions.child(
                                    div()
                                        .id("overlay-annotation-context-actions")
                                        .flex()
                                        .items_center()
                                        .gap(px(OVERLAY_ACTION_ITEM_GAP))
                                        .child(workspace_icon_button(
                                            "overlay-undo",
                                            icon::UNDO,
                                            locale.text(UiText::OverlayUndo),
                                            WorkspaceButtonConfig::icon(
                                                workspace_colors,
                                                WorkspaceButtonTone::Neutral,
                                                false,
                                                can_undo,
                                                locale.text(UiText::OverlayUndo),
                                            ),
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.undo_annotation(cx)
                                                    });
                                                });
                                            }),
                                        ))
                                        .child(workspace_icon_button(
                                            "overlay-redo",
                                            icon::REDO,
                                            locale.text(UiText::OverlayRedo),
                                            WorkspaceButtonConfig::icon(
                                                workspace_colors,
                                                WorkspaceButtonTone::Neutral,
                                                false,
                                                can_redo,
                                                locale.text(UiText::OverlayRedo),
                                            ),
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.redo_annotation(cx)
                                                    });
                                                });
                                            }),
                                        )),
                                )
                            })
                            .when(!show_annotation_controls, |actions| actions.child(
                                div()
                                    .id("overlay-mode-actions")
                                    .flex()
                                    .items_center()
                                    .child(
                                        workspace_icon_button(
                                            "overlay-annotation-controls",
                                            icon::MARK,
                                            locale.text(UiText::OverlayMark),
                                            WorkspaceButtonConfig::icon(
                                                workspace_colors,
                                                WorkspaceButtonTone::Neutral,
                                                show_annotation_controls,
                                                true,
                                                primary_action_tooltip(locale, "draw"),
                                            ),
                                            cx.listener(|this, _, window, cx| {
                                                // Return focus to the overlay canvas after opening the
                                                // annotation palette so keyboard tools remain available
                                                // without requiring an extra click on the image.
                                                this.focus_handle.focus(window, cx);
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.toggle_overlay_annotation_controls(cx)
                                                    })
                                                });
                                            }),
                                        )
                                        .on_key_down(stop_overlay_action_key_propagation),
                                    ),
                            ))
                            .when(show_annotation_controls, |actions| {
                                actions.child(workspace_separator(
                                    "overlay-action-result-separator",
                                    workspace_colors,
                                ))
                            })
                            .child(
                                div()
                                    .id("overlay-result-actions")
                                    .flex()
                                    .items_center()
                                    .gap(px(OVERLAY_ACTION_ITEM_GAP))
                                    .child(
                                        workspace_icon_button(
                                            "overlay-pin",
                                            icon::PIN,
                                            locale.text(UiText::OverlayPin),
                                            WorkspaceButtonConfig::icon(
                                                workspace_colors,
                                                WorkspaceButtonTone::Neutral,
                                                false,
                                                true,
                                                locale.text(UiText::OverlayPinTooltip),
                                            ),
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| app.pin_selection(cx))
                                                });
                                            }),
                                        )
                                        .on_key_down(stop_overlay_action_key_propagation),
                                    )
                                    .child(
                                        workspace_icon_button(
                                            "overlay-save",
                                            icon::SAVE,
                                            locale.text(UiText::OverlaySave),
                                            WorkspaceButtonConfig::icon(
                                                workspace_colors,
                                                WorkspaceButtonTone::Neutral,
                                                false,
                                                true,
                                                primary_action_tooltip(locale, "save"),
                                            ),
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| app.save_selection(cx))
                                                });
                                            }),
                                        )
                                        .on_key_down(stop_overlay_action_key_propagation),
                                    )
                                    .child(
                                        workspace_icon_button(
                                            OVERLAY_MORE_ACTIONS_ID,
                                            icon::MORE,
                                            more_actions_button_label(locale, show_more_actions),
                                            WorkspaceButtonConfig::icon(
                                                workspace_colors,
                                                WorkspaceButtonTone::Neutral,
                                                show_more_actions,
                                                true,
                                                if show_more_actions {
                                                    locale.text(UiText::OverlayHideMoreTooltip)
                                                } else {
                                                    locale.text(UiText::OverlayShowMoreTooltip)
                                                },
                                            ),
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.toggle_overlay_more_actions(cx)
                                                    })
                                                });
                                            }),
                                        )
                                        .track_focus(&self.more_actions_focus_handle)
                                        .aria_keyshortcuts("Alt+M")
                                        .aria_expanded(show_more_actions)
                                        .on_key_down(
                                            move |event, window, cx| {
                                                handle_more_actions_key_down(
                                                    show_more_actions,
                                                    &more_actions_navigation,
                                                    event,
                                                    window,
                                                    cx,
                                                );
                                            },
                                        ),
                                    )
                                    .child(
                                        workspace_icon_button(
                                            "overlay-cancel",
                                            icon::CANCEL,
                                            locale.text(UiText::OverlayCancel),
                                            WorkspaceButtonConfig::icon(
                                                workspace_colors,
                                                WorkspaceButtonTone::Destructive,
                                                false,
                                                true,
                                                primary_action_tooltip(locale, "cancel"),
                                            ),
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| app.reset(cx))
                                                });
                                            }),
                                        )
                                        .on_key_down(stop_overlay_action_key_propagation),
                                    )
                                    .child(
                                        workspace_icon_button(
                                            "overlay-copy",
                                            icon::COPY,
                                            locale.text(UiText::OverlayCopy),
                                            WorkspaceButtonConfig::icon(
                                                workspace_colors,
                                                WorkspaceButtonTone::Primary,
                                                false,
                                                !selection_copy_in_progress,
                                                if selection_copy_in_progress {
                                                    locale.text(UiText::OverlayCopyingTooltip)
                                                } else {
                                                    primary_action_tooltip(locale, "copy")
                                                },
                                            ),
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| app.copy_selection(cx))
                                                });
                                            }),
                                        )
                                        .on_key_down(stop_overlay_action_key_propagation),
                                    ),
                            )
                            .when(show_more_actions, |actions| {
                                actions.child(
                                    workspace_surface(workspace_colors, false)
                                        .id("overlay-secondary-actions")
                                        .tab_group()
                                        .occlude()
                                        .absolute()
                                        .when_some(secondary_menu, |menu, layout| {
                                            menu.w(px(layout.width)).left(px(layout.left))
                                        })
                                        .when_some(secondary_menu, |menu, layout| {
                                            if layout.opens_above {
                                                menu.bottom(px(secondary_action_menu_offset()))
                                            } else {
                                                menu.top(px(secondary_action_menu_offset()))
                                            }
                                        })
                                        .p(px(OVERLAY_ACTION_BAR_PADDING))
                                        .flex()
                                        .flex_wrap()
                                        .justify_end()
                                        .gap(px(OVERLAY_ACTION_ITEM_GAP))
                                        .child(secondary_action_button(
                                            "overlay-save-annotations",
                                            secondary_navigation
                                                .for_action(SecondaryAction::SaveAnnotations),
                                            locale.text(UiText::OverlaySaveAnnotations),
                                            OVERLAY_MORE_ACTION_WIDTHS[0],
                                            workspace_colors,
                                            false,
                                            None,
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.save_annotation_document(cx)
                                                    })
                                                });
                                            }),
                                        ))
                                        .child(secondary_action_button(
                                            "overlay-save-editable-project",
                                            secondary_navigation
                                                .for_action(SecondaryAction::SaveEditable),
                                            locale.text(UiText::OverlaySaveEditable),
                                            OVERLAY_MORE_ACTION_WIDTHS[1],
                                            workspace_colors,
                                            false,
                                            None,
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.save_editable_project(cx)
                                                    })
                                                });
                                            }),
                                        ))
                                        .child(secondary_action_button(
                                            "overlay-open-annotations",
                                            secondary_navigation
                                                .for_action(SecondaryAction::OpenAnnotations),
                                            locale.text(UiText::OverlayOpenAnnotations),
                                            OVERLAY_MORE_ACTION_WIDTHS[2],
                                            workspace_colors,
                                            false,
                                            None,
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.open_annotation_document(cx)
                                                    })
                                                });
                                            }),
                                        ))
                                        .child(secondary_action_button(
                                            "overlay-quick-save",
                                            secondary_navigation
                                                .for_action(SecondaryAction::QuickSave),
                                            locale.text(UiText::OverlayQuickSave),
                                            OVERLAY_MORE_ACTION_WIDTHS[3],
                                            workspace_colors,
                                            true,
                                            Some(locale.text(UiText::OverlayQuickSaveTooltip)),
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.quick_save_selection(cx)
                                                    })
                                                });
                                            }),
                                        ))
                                        .child(secondary_action_button(
                                            "overlay-manual-scroll",
                                            secondary_navigation
                                                .for_action(SecondaryAction::ScrollShot),
                                            locale.text(UiText::OverlayScrollShot),
                                            OVERLAY_MORE_ACTION_WIDTHS[4],
                                            workspace_colors,
                                            false,
                                            Some(secondary_action_tooltip(locale, "scroll")),
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.start_manual_scroll(cx)
                                                    })
                                                });
                                            }),
                                        ))
                                        .child(secondary_action_button(
                                            "overlay-qr",
                                            secondary_navigation.for_action(SecondaryAction::Qr),
                                            locale.text(UiText::OverlayQr),
                                            OVERLAY_MORE_ACTION_WIDTHS[5],
                                            workspace_colors,
                                            false,
                                            Some(secondary_action_tooltip(locale, "qr")),
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.recognize_qr_selection(cx)
                                                    })
                                                });
                                            }),
                                        ))
                                        .child(secondary_action_button(
                                            "overlay-ocr",
                                            secondary_navigation.for_action(SecondaryAction::Ocr),
                                            locale.text(UiText::OverlayOcr),
                                            OVERLAY_MORE_ACTION_WIDTHS[6],
                                            workspace_colors,
                                            false,
                                            Some(secondary_action_tooltip(locale, "ocr")),
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.recognize_text_selection(cx)
                                                    })
                                                });
                                            }),
                                        ))
                                        .child(secondary_action_button(
                                            "overlay-copy-color",
                                            secondary_navigation
                                                .for_action(SecondaryAction::CopyColor),
                                            locale.text(UiText::OverlayCopyColor),
                                            OVERLAY_MORE_ACTION_WIDTHS[7],
                                            workspace_colors,
                                            false,
                                            Some(locale.text(UiText::OverlayCopyColorTooltip)),
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.copy_hover_color(cx)
                                                    })
                                                });
                                            }),
                                        ))
                                        .child(secondary_action_button(
                                            "overlay-translate",
                                            secondary_navigation
                                                .for_action(SecondaryAction::Translate),
                                            locale.text(UiText::OverlayTranslate),
                                            OVERLAY_MORE_ACTION_WIDTHS[8],
                                            workspace_colors,
                                            false,
                                            Some(secondary_action_tooltip(locale, "translate")),
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.translate_selection(cx)
                                                    })
                                                });
                                            }),
                                        ))
                                        .child(secondary_action_button(
                                            "overlay-record-area",
                                            secondary_navigation
                                                .for_action(SecondaryAction::RecordArea),
                                            locale.text(UiText::OverlayRecordArea),
                                            OVERLAY_MORE_ACTION_WIDTHS[9],
                                            workspace_colors,
                                            false,
                                            Some(secondary_action_tooltip(locale, "record-area")),
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.start_region_recording(cx)
                                                    })
                                                });
                                            }),
                                        ))
                                        .child(secondary_action_button(
                                            "overlay-record-window",
                                            secondary_navigation
                                                .for_action(SecondaryAction::RecordWindow),
                                            locale.text(UiText::OverlayRecordWindow),
                                            OVERLAY_MORE_ACTION_WIDTHS[10],
                                            workspace_colors,
                                            false,
                                            Some(secondary_action_tooltip(locale, "record-window")),
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.start_selected_window_recording(cx)
                                                    })
                                                });
                                            }),
                                        ))
                                        .when(recognition_in_flight, |actions| {
                                            actions.child(
                                                div()
                                                    .id("overlay-recognition-progress")
                                                    .w_full()
                                                    .h(px(OVERLAY_RECOGNITION_STATUS_HEIGHT))
                                                    .px_2()
                                                    .flex()
                                                    .items_center()
                                                    .rounded_sm()
                                                    .bg(workspace_colors.toolbar_surface)
                                                    .text_xs()
                                                    .text_color(workspace_colors.overlay_muted)
                                                    .child(
                                                        locale.text(
                                                            UiText::OverlayRecognizingSelection,
                                                        ),
                                                    ),
                                            )
                                        })
                                        .when_some(recognition_retry, |actions, retry| {
                                            let retry_label =
                                                recognition_retry_label(locale, retry);
                                            actions.child(secondary_action_button(
                                                "overlay-retry-recognition",
                                                secondary_navigation
                                                    .for_action(SecondaryAction::RetryRecognition),
                                                retry_label,
                                                OVERLAY_RETRY_ACTION_WIDTHS[0],
                                                workspace_colors,
                                                true,
                                                Some(
                                                    locale.text(
                                                        UiText::OverlayRetryRecognitionTooltip,
                                                    ),
                                                ),
                                                cx.listener(move |this, _, window, cx| {
                                                    let app = this.app.clone();
                                                    cx.defer(move |cx| {
                                                        app.update(cx, |app, cx| {
                                                            app.retry_recognition(retry, cx)
                                                        })
                                                    });
                                                    this.focus_secondary_action_after_render(
                                                        SecondaryAction::SaveAnnotations,
                                                        window,
                                                        cx,
                                                    );
                                                }),
                                            ))
                                        })
                                        .when_some(recognition_result, |actions, result| {
                                            actions
                                                .child(
                                                    div()
                                                        .id("overlay-recognition-preview")
                                                        .w_full()
                                                        .h(px(OVERLAY_RECOGNITION_PREVIEW_HEIGHT))
                                                        .p_2()
                                                        .flex()
                                                        .flex_col()
                                                        .gap_1()
                                                        .overflow_hidden()
                                                        .border_1()
                                                        .border_color(rgba(0xFFFFFF24))
                                                        .bg(rgba(0x0B0D10E6))
                                                        .child(
                                                            div()
                                                                .text_xs()
                                                                .text_color(
                                                                    workspace_colors.overlay_muted,
                                                                )
                                                                .child(result.title.clone()),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_sm()
                                                                .text_color(
                                                                    workspace_colors.overlay_text,
                                                                )
                                                                .child(recognition_result_preview(
                                                                    &result.text,
                                                                )),
                                                        ),
                                                )
                                                .child(secondary_action_button(
                                                    "overlay-copy-recognition",
                                                    secondary_navigation.for_action(
                                                        SecondaryAction::CopyRecognition,
                                                    ),
                                                    locale.text(UiText::OverlayCopyText),
                                                    OVERLAY_RECOGNITION_ACTION_WIDTHS[0],
                                                    workspace_colors,
                                                    false,
                                                    Some(
                                                        locale.text(UiText::OverlayCopyTextTooltip),
                                                    ),
                                                    cx.listener(|this, _, _, cx| {
                                                        let app = this.app.clone();
                                                        cx.defer(move |cx| {
                                                            app.update(cx, |app, cx| {
                                                                app.copy_recognition_result(cx)
                                                            })
                                                        });
                                                    }),
                                                ))
                                                .child(secondary_action_button(
                                                    "overlay-clear-recognition",
                                                    secondary_navigation.for_action(
                                                        SecondaryAction::ClearRecognition,
                                                    ),
                                                    locale.text(UiText::OverlayClearResult),
                                                    OVERLAY_RECOGNITION_ACTION_WIDTHS[1],
                                                    workspace_colors,
                                                    false,
                                                    Some(
                                                        locale.text(
                                                            UiText::OverlayClearResultTooltip,
                                                        ),
                                                    ),
                                                    cx.listener(|this, _, window, cx| {
                                                        let app = this.app.clone();
                                                        cx.defer(move |cx| {
                                                            app.update(cx, |app, cx| {
                                                                app.clear_recognition_result(cx)
                                                            })
                                                        });
                                                        this.focus_secondary_action_after_render(
                                                            SecondaryAction::SaveAnnotations,
                                                            window,
                                                            cx,
                                                        );
                                                    }),
                                                ))
                                        }),
                                )
                            })
                    })
                    .when(!has_selection, |actions| {
                        actions.child(
                            div()
                                .id("overlay-cancel")
                                .px_3()
                                .py_2()
                                .bg(colors.panel)
                                .text_color(colors.text)
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    let app = this.app.clone();
                                    cx.defer(move |cx| app.update(cx, |app, cx| app.reset(cx)));
                                }))
                                .child(locale.text(UiText::OverlayCancel)),
                        )
                    }),
            )
    }
}

fn paint_selection_mask(
    window: &mut Window,
    viewport: Bounds<Pixels>,
    transform: Option<PreviewTransform>,
    selection: Option<PhysicalRect>,
    target: Option<PhysicalRect>,
    colors: ThemeColors,
) {
    let Some(transform) = transform else {
        window.paint_quad(fill(viewport, rgba(0x00000066)));
        return;
    };
    let Some(selection) = selection else {
        window.paint_quad(fill(viewport, rgba(0x00000066)));
        if let Some(target) = target {
            paint_outline(window, transform, target, colors.accent, 1);
        }
        return;
    };
    let start = transform.physical_to_view(PhysicalPoint {
        x: selection.left,
        y: selection.top,
    });
    let end = transform.physical_to_view(PhysicalPoint {
        x: selection.right,
        y: selection.bottom,
    });
    let selection_bounds = Bounds::new(
        point(px(start.x), px(start.y)),
        size(px(end.x - start.x), px(end.y - start.y)),
    );
    let shade = rgba(0x00000066);
    window.paint_quad(fill(
        Bounds::new(
            viewport.origin,
            size(
                viewport.size.width,
                selection_bounds.origin.y - viewport.origin.y,
            ),
        ),
        shade,
    ));
    window.paint_quad(fill(
        Bounds::new(
            point(viewport.origin.x, selection_bounds.bottom()),
            size(
                viewport.size.width,
                viewport.bottom() - selection_bounds.bottom(),
            ),
        ),
        shade,
    ));
    window.paint_quad(fill(
        Bounds::new(
            point(viewport.origin.x, selection_bounds.origin.y),
            size(
                selection_bounds.origin.x - viewport.origin.x,
                selection_bounds.size.height,
            ),
        ),
        shade,
    ));
    window.paint_quad(fill(
        Bounds::new(
            point(selection_bounds.right(), selection_bounds.origin.y),
            size(
                viewport.right() - selection_bounds.right(),
                selection_bounds.size.height,
            ),
        ),
        shade,
    ));
    window.paint_quad(gpui::outline(
        selection_bounds,
        colors.accent,
        gpui::BorderStyle::Solid,
    ));
    // These are the same corners used by PreviewTransform::resize_handle_at,
    // so the visible affordance matches the physical-pixel hit targets.
    paint_resize_handles(window, transform, selection, colors.accent);
}

fn paint_magnifier(
    window: &mut Window,
    viewport: Bounds<Pixels>,
    transform: Option<PreviewTransform>,
    hover_pixel: Option<PhysicalPoint>,
    frame: Option<&crate::platform::capture::CaptureFrame>,
) {
    let (Some(transform), Some(center), Some(frame)) = (transform, hover_pixel, frame) else {
        return;
    };
    if !frame.bounds.contains(center) {
        return;
    }

    let view_center = transform.physical_to_view(center);
    let grid_cells = (MAGNIFIER_RADIUS * 2 + 1) as f32;
    let grid_size = grid_cells * MAGNIFIER_CELL_SIZE;
    let origin = magnifier_origin(view_center, viewport, grid_size);
    let panel = Bounds::new(
        point(px(origin.x - 4.0), px(origin.y - 4.0)),
        size(px(grid_size + 8.0), px(grid_size + 8.0)),
    );
    window.paint_quad(fill(panel, rgba(0x111827F2)));
    window.paint_quad(gpui::outline(
        panel,
        rgba(0xF4F6F8FF),
        gpui::BorderStyle::Solid,
    ));

    for row in -MAGNIFIER_RADIUS..=MAGNIFIER_RADIUS {
        for column in -MAGNIFIER_RADIUS..=MAGNIFIER_RADIUS {
            let sample_point = PhysicalPoint {
                x: center
                    .x
                    .saturating_add(column)
                    .clamp(frame.bounds.left, frame.bounds.right.saturating_sub(1)),
                y: center
                    .y
                    .saturating_add(row)
                    .clamp(frame.bounds.top, frame.bounds.bottom.saturating_sub(1)),
            };
            let Some(color) = frame.pixel_at(sample_point) else {
                continue;
            };
            let cell = Bounds::new(
                point(
                    px(origin.x + (column + MAGNIFIER_RADIUS) as f32 * MAGNIFIER_CELL_SIZE),
                    px(origin.y + (row + MAGNIFIER_RADIUS) as f32 * MAGNIFIER_CELL_SIZE),
                ),
                size(px(MAGNIFIER_CELL_SIZE), px(MAGNIFIER_CELL_SIZE)),
            );
            window.paint_quad(fill(cell, rgba(color.rgba_u32())));
            window.paint_quad(gpui::outline(
                cell,
                if row == 0 && column == 0 {
                    rgba(0xFFFFFFFF)
                } else {
                    rgba(0x00000055)
                },
                gpui::BorderStyle::Solid,
            ));
        }
    }
}

fn magnifier_origin(view_center: ViewPoint, viewport: Bounds<Pixels>, grid_size: f32) -> ViewPoint {
    let min_x = f32::from(viewport.origin.x) + 4.0;
    let min_y = f32::from(viewport.origin.y) + 4.0;
    let max_x = (f32::from(viewport.right()) - grid_size - 4.0).max(min_x);
    let max_y = (f32::from(viewport.bottom()) - grid_size - 4.0).max(min_y);
    ViewPoint {
        x: (view_center.x + MAGNIFIER_GAP).clamp(min_x, max_x),
        y: (view_center.y + MAGNIFIER_GAP).clamp(min_y, max_y),
    }
}

#[derive(Clone, Copy)]
struct AnnotationPaintState {
    selected: Option<AnnotationId>,
    hidden: Option<AnnotationId>,
}

fn paint_annotations(
    window: &mut Window,
    transform: Option<PreviewTransform>,
    annotations: &[Annotation],
    preview: Option<&Annotation>,
    state: AnnotationPaintState,
    colors: ThemeColors,
    cx: &mut gpui::App,
) {
    let Some(transform) = transform else {
        return;
    };
    for annotation in annotations
        .iter()
        .filter(|annotation| {
            Some(annotation.id) != preview.map(|preview| preview.id)
                && Some(annotation.id) != state.hidden
        })
        .chain(preview)
    {
        let color = rgba(annotation.style.stroke_rgba).into();
        match annotation.kind {
            AnnotationKind::Watermark {
                origin,
                ref content,
            } => paint_text_annotation(
                window,
                transform,
                origin,
                content,
                annotation.style.stroke_rgba,
                annotation.text_font_size(),
                cx,
            ),
            AnnotationKind::Text {
                origin,
                ref content,
            } => paint_text_annotation(
                window,
                transform,
                origin,
                content,
                annotation.style.stroke_rgba,
                annotation.text_font_size(),
                cx,
            ),
            AnnotationKind::Number { center, value } => paint_number_marker(
                window,
                transform,
                center,
                value,
                annotation.style.stroke_rgba,
            ),
            AnnotationKind::Blur { bounds } => {
                paint_rect_fill(window, transform, bounds, rgba(0xCBD5E188));
                paint_outline(window, transform, bounds, colors.muted, 1);
            }
            AnnotationKind::Mosaic { bounds } => {
                paint_rect_fill(window, transform, bounds, rgba(0x11182799));
                paint_mosaic_grid(window, transform, bounds, colors.muted);
            }
            AnnotationKind::Highlight { bounds } => {
                paint_rect_fill(
                    window,
                    transform,
                    bounds,
                    rgba(annotation.style.stroke_rgba),
                );
            }
            AnnotationKind::Rectangle { bounds } => {
                if let Some(fill_color) = annotation.style.fill_rgba {
                    paint_rect_fill(window, transform, bounds, rgba(fill_color));
                }
                paint_outline(window, transform, bounds, color, annotation.stroke_width())
            }
            AnnotationKind::Ellipse { bounds } => paint_ellipse_outline(
                window,
                transform,
                bounds,
                color,
                annotation.stroke_width(),
                annotation.style.fill_rgba.map(rgba),
            ),
            AnnotationKind::Line { start, end } => paint_line(
                window,
                transform,
                start,
                end,
                color,
                annotation.stroke_width(),
            ),
            AnnotationKind::Arrow { start, end } => paint_arrow(
                window,
                transform,
                start,
                end,
                color,
                annotation.stroke_width(),
            ),
            AnnotationKind::Freehand { ref points } => {
                paint_freehand(window, transform, points, color, annotation.stroke_width())
            }
        }
        if Some(annotation.id) == state.selected {
            paint_outline(window, transform, annotation.bounds(), colors.success, 1);
            paint_resize_handles(window, transform, annotation.bounds(), colors.success);
        }
    }
}

fn annotation_layer_label(locale: Locale, kind: &AnnotationKind) -> &'static str {
    match kind {
        AnnotationKind::Watermark { .. } => locale.text(UiText::OverlayWatermark),
        AnnotationKind::Text { .. } => locale.text(UiText::OverlayText),
        AnnotationKind::Number { .. } => locale.text(UiText::OverlayNumber),
        AnnotationKind::Blur { .. } => locale.text(UiText::OverlayBlur),
        AnnotationKind::Mosaic { .. } => locale.text(UiText::OverlayMosaic),
        AnnotationKind::Highlight { .. } => locale.text(UiText::OverlayHighlight),
        AnnotationKind::Rectangle { .. } => locale.text(UiText::OverlayRectangle),
        AnnotationKind::Ellipse { .. } => locale.text(UiText::OverlayEllipse),
        AnnotationKind::Line { .. } => locale.text(UiText::OverlayLine),
        AnnotationKind::Arrow { .. } => locale.text(UiText::OverlayArrow),
        AnnotationKind::Freehand { .. } => locale.text(UiText::OverlayFreehand),
    }
}

/// Formats one visible layer row through the active catalog while preserving its stable number.
fn annotation_layer_entry_label(locale: Locale, position: usize, kind: &AnnotationKind) -> String {
    let position = position.to_string();
    let kind = annotation_layer_label(locale, kind);
    locale.format_template(
        UiText::OverlayLayerLabel,
        &[("position", &position), ("kind", kind)],
    )
}

/// Formats the compact opacity value through the active catalog without changing its width.
fn annotation_opacity_value_label(locale: Locale, opacity: u8) -> String {
    let percent = (u16::from(opacity) * 100 / 255).to_string();
    locale.format_template(UiText::AnnotationOpacityValue, &[("percent", &percent)])
}

#[cfg(test)]
fn is_text_annotation(annotation: &Annotation) -> bool {
    matches!(
        annotation.kind,
        AnnotationKind::Text { .. } | AnnotationKind::Watermark { .. }
    )
}

fn paint_text_annotation(
    window: &mut Window,
    transform: PreviewTransform,
    origin: PhysicalPoint,
    content: &str,
    color: u32,
    font_size: u32,
    cx: &mut gpui::App,
) {
    let view_origin = transform.physical_to_view(origin);
    let physical_font_size = i32::try_from(font_size.clamp(1, 96)).unwrap_or(96);
    let view_font_size = (transform
        .physical_to_view(PhysicalPoint {
            x: origin.x,
            y: origin.y.saturating_add(physical_font_size),
        })
        .y
        - view_origin.y)
        .abs()
        .clamp(MIN_ANNOTATION_VIEW_FONT_SIZE, MAX_ANNOTATION_VIEW_FONT_SIZE);
    let content = normalized_text_annotation_content(content);
    if content.is_empty() {
        return;
    }
    let style = window.text_style();
    let run = TextRun {
        len: content.len(),
        font: style.font(),
        color: rgba(color).into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window
        .text_system()
        .shape_line(content.into(), px(view_font_size), &[run], None);
    let _ = line.paint(
        point(px(view_origin.x), px(view_origin.y)),
        px(view_font_size * 1.25),
        TextAlign::Left,
        None,
        window,
        cx,
    );
}

fn paint_number_marker(
    window: &mut Window,
    transform: PreviewTransform,
    center: PhysicalPoint,
    value: u32,
    color: u32,
) {
    let view_center = transform.physical_to_view(center);
    let radius = (transform
        .physical_to_view(PhysicalPoint {
            x: center.x.saturating_add(SEQUENCE_MARKER_RADIUS),
            y: center.y,
        })
        .x
        - view_center.x)
        .abs();
    if radius <= 0.0 {
        return;
    }
    let mut path = gpui::PathBuilder::fill();
    const SEGMENTS: u32 = 32;
    for index in 0..=SEGMENTS {
        let angle = std::f32::consts::TAU * index as f32 / SEGMENTS as f32;
        let point = point(
            px(view_center.x + radius * angle.cos()),
            px(view_center.y + radius * angle.sin()),
        );
        if index == 0 {
            path.move_to(point);
        } else {
            path.line_to(point);
        }
    }
    path.close();
    if let Ok(path) = path.build() {
        window.paint_path(path, rgba(color));
    }
    paint_number_label(
        window,
        view_center,
        value,
        (radius / SEQUENCE_MARKER_RADIUS as f32).max(0.5),
    );
}

fn paint_number_label(window: &mut Window, center: ViewPoint, value: u32, scale: f32) {
    const DIGITS: [[u8; 5]; 10] = [
        [0b111, 0b101, 0b101, 0b101, 0b111],
        [0b010, 0b110, 0b010, 0b010, 0b111],
        [0b111, 0b001, 0b111, 0b100, 0b111],
        [0b111, 0b001, 0b111, 0b001, 0b111],
        [0b101, 0b101, 0b111, 0b001, 0b001],
        [0b111, 0b100, 0b111, 0b001, 0b111],
        [0b111, 0b100, 0b111, 0b101, 0b111],
        [0b111, 0b001, 0b010, 0b010, 0b010],
        [0b111, 0b101, 0b111, 0b101, 0b111],
        [0b111, 0b101, 0b111, 0b001, 0b111],
    ];
    let digits = value.to_string();
    let width = digits.len() as f32 * 4.0 - 1.0;
    let left = center.x - width * scale / 2.0;
    let top = center.y - 2.5 * scale;
    for (digit_index, digit) in digits.bytes().enumerate() {
        let Some(rows) = digit
            .checked_sub(b'0')
            .and_then(|index| DIGITS.get(index as usize))
        else {
            continue;
        };
        for (row, bits) in rows.iter().enumerate() {
            for column in 0..3 {
                if bits & (1 << (2 - column)) != 0 {
                    window.paint_quad(fill(
                        Bounds::new(
                            point(
                                px(left + (digit_index as f32 * 4.0 + column as f32) * scale),
                                px(top + row as f32 * scale),
                            ),
                            size(px(scale.max(1.0)), px(scale.max(1.0))),
                        ),
                        rgba(0xFFFFFFFF),
                    ));
                }
            }
        }
    }
}

fn paint_mosaic_grid(
    window: &mut Window,
    transform: PreviewTransform,
    bounds: PhysicalRect,
    color: gpui::Hsla,
) {
    const BLOCK_SIZE: i32 = 10;
    for x in (bounds.left..=bounds.right).step_by(BLOCK_SIZE as usize) {
        paint_line(
            window,
            transform,
            PhysicalPoint { x, y: bounds.top },
            PhysicalPoint {
                x,
                y: bounds.bottom,
            },
            color,
            1,
        );
    }
    for y in (bounds.top..=bounds.bottom).step_by(BLOCK_SIZE as usize) {
        paint_line(
            window,
            transform,
            PhysicalPoint { x: bounds.left, y },
            PhysicalPoint { x: bounds.right, y },
            color,
            1,
        );
    }
}

fn paint_rect_fill(
    window: &mut Window,
    transform: PreviewTransform,
    bounds: PhysicalRect,
    color: gpui::Rgba,
) {
    let start = transform.physical_to_view(PhysicalPoint {
        x: bounds.left,
        y: bounds.top,
    });
    let end = transform.physical_to_view(PhysicalPoint {
        x: bounds.right,
        y: bounds.bottom,
    });
    window.paint_quad(fill(
        Bounds::new(
            point(px(start.x), px(start.y)),
            size(px(end.x - start.x), px(end.y - start.y)),
        ),
        color,
    ));
}

fn paint_resize_handles(
    window: &mut Window,
    transform: PreviewTransform,
    bounds: PhysicalRect,
    color: gpui::Hsla,
) {
    const HANDLE_SIZE: f32 = 8.0;
    for physical_point in resize_handle_points(bounds) {
        let view_point = transform.physical_to_view(physical_point);
        window.paint_quad(fill(
            Bounds::new(
                point(
                    px(view_point.x - HANDLE_SIZE / 2.0),
                    px(view_point.y - HANDLE_SIZE / 2.0),
                ),
                size(px(HANDLE_SIZE), px(HANDLE_SIZE)),
            ),
            color,
        ));
    }
}

fn resize_handle_points(bounds: PhysicalRect) -> [PhysicalPoint; 4] {
    [
        PhysicalPoint {
            x: bounds.left,
            y: bounds.top,
        },
        PhysicalPoint {
            x: bounds.right,
            y: bounds.top,
        },
        PhysicalPoint {
            x: bounds.left,
            y: bounds.bottom,
        },
        PhysicalPoint {
            x: bounds.right,
            y: bounds.bottom,
        },
    ]
}

#[cfg(test)]
fn outline_shape_bounds(annotation: &Annotation) -> Option<PhysicalRect> {
    match annotation.kind {
        AnnotationKind::Blur { bounds }
        | AnnotationKind::Mosaic { bounds }
        | AnnotationKind::Highlight { bounds }
        | AnnotationKind::Rectangle { bounds }
        | AnnotationKind::Ellipse { bounds } => Some(bounds),
        _ => None,
    }
}

fn paint_outline(
    window: &mut Window,
    transform: PreviewTransform,
    rect: PhysicalRect,
    color: gpui::Hsla,
    stroke_width: u32,
) {
    let start = transform.physical_to_view(PhysicalPoint {
        x: rect.left,
        y: rect.top,
    });
    let end = transform.physical_to_view(PhysicalPoint {
        x: rect.right,
        y: rect.bottom,
    });
    let mut path = gpui::PathBuilder::stroke(px(stroke_width.max(1) as f32));
    path.move_to(point(px(start.x), px(start.y)));
    path.line_to(point(px(end.x), px(start.y)));
    path.line_to(point(px(end.x), px(end.y)));
    path.line_to(point(px(start.x), px(end.y)));
    path.close();
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

fn paint_line(
    window: &mut Window,
    transform: PreviewTransform,
    start: PhysicalPoint,
    end: PhysicalPoint,
    color: gpui::Hsla,
    stroke_width: u32,
) {
    let start = transform.physical_to_view(start);
    let end = transform.physical_to_view(end);
    let mut path = gpui::PathBuilder::stroke(px(stroke_width.max(1) as f32));
    path.move_to(point(px(start.x), px(start.y)));
    path.line_to(point(px(end.x), px(end.y)));
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

fn paint_arrow(
    window: &mut Window,
    transform: PreviewTransform,
    start: PhysicalPoint,
    end: PhysicalPoint,
    color: gpui::Hsla,
    stroke_width: u32,
) {
    paint_line(window, transform, start, end, color, stroke_width);
    let arrow_head_size = f64::from(stroke_width.div_ceil(2).max(3)) * 4.0;
    let (left, right) = arrow_head_points(start, end, arrow_head_size);
    for point in [left, right].into_iter().flatten() {
        paint_line(window, transform, end, point, color, stroke_width);
    }
}

fn paint_freehand(
    window: &mut Window,
    transform: PreviewTransform,
    points: &[PhysicalPoint],
    color: gpui::Hsla,
    stroke_width: u32,
) {
    for segment in points.windows(2) {
        paint_line(
            window,
            transform,
            segment[0],
            segment[1],
            color,
            stroke_width,
        );
    }
}

fn paint_ellipse_outline(
    window: &mut Window,
    transform: PreviewTransform,
    rect: PhysicalRect,
    color: gpui::Hsla,
    stroke_width: u32,
    fill_color: Option<gpui::Rgba>,
) {
    let start = transform.physical_to_view(PhysicalPoint {
        x: rect.left,
        y: rect.top,
    });
    let end = transform.physical_to_view(PhysicalPoint {
        x: rect.right,
        y: rect.bottom,
    });
    let center_x = (start.x + end.x) / 2.0;
    let center_y = (start.y + end.y) / 2.0;
    let radius_x = (end.x - start.x).abs() / 2.0;
    let radius_y = (end.y - start.y).abs() / 2.0;
    if radius_x == 0.0 || radius_y == 0.0 {
        return;
    }
    if let Some(fill_color) = fill_color {
        let mut path = gpui::PathBuilder::fill();
        for index in 0..=SEGMENTS {
            let angle = std::f32::consts::TAU * index as f32 / SEGMENTS as f32;
            let point = point(
                px(center_x + radius_x * angle.cos()),
                px(center_y + radius_y * angle.sin()),
            );
            if index == 0 {
                path.move_to(point);
            } else {
                path.line_to(point);
            }
        }
        path.close();
        if let Ok(path) = path.build() {
            window.paint_path(path, fill_color);
        }
    }
    let mut path = gpui::PathBuilder::stroke(px(stroke_width.max(1) as f32));
    const SEGMENTS: u32 = 32;
    for index in 0..=SEGMENTS {
        let angle = std::f32::consts::TAU * index as f32 / SEGMENTS as f32;
        let point = point(
            px(center_x + radius_x * angle.cos()),
            px(center_y + radius_y * angle.sin()),
        );
        if index == 0 {
            path.move_to(point);
        } else {
            path.line_to(point);
        }
    }
    path.close();
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

fn view_rect(bounds: Bounds<Pixels>) -> ViewRect {
    ViewRect {
        left: f32::from(bounds.origin.x),
        top: f32::from(bounds.origin.y),
        width: f32::from(bounds.size.width),
        height: f32::from(bounds.size.height),
    }
}

fn local_viewport(window: &Window) -> Bounds<Pixels> {
    Bounds::new(point(px(0.0), px(0.0)), window.bounds().size)
}

fn view_point(position: gpui::Point<Pixels>) -> ViewPoint {
    ViewPoint {
        x: f32::from(position.x),
        y: f32::from(position.y),
    }
}

fn clamp_to_view(transform: PreviewTransform, position: gpui::Point<Pixels>) -> ViewPoint {
    let fitted = transform.fitted_view();
    ViewPoint {
        x: f32::from(position.x).clamp(fitted.left, fitted.right()),
        y: f32::from(position.y).clamp(fitted.top, fitted.bottom()),
    }
}

/// Uses global physical pixels for full-display overlays so off-screen Win32 client insets do not
/// shift selection edges; windowed image editors continue to use their local preview transform.
fn selection_point_from_view_or_screen(
    transform: PreviewTransform,
    position: gpui::Point<Pixels>,
    allow_cross_display_drag: bool,
    screen_pointer: Option<PhysicalPoint>,
    selection_bounds: PhysicalRect,
) -> Option<PhysicalPoint> {
    let view = view_point(position);
    if allow_cross_display_drag {
        let display_bounds = transform.image_bounds();
        let local_point = transform
            .view_to_physical(clamp_to_view(transform, position))
            .map(|point| map_display_point_to_frame(point, display_bounds, selection_bounds));
        let screen_point = screen_pointer
            .map(|point| map_display_point_to_frame(point, display_bounds, selection_bounds));
        match (screen_pointer, screen_point, local_point) {
            (Some(raw_screen), Some(screen), Some(local)) => Some(PhysicalPoint {
                // Prefer the local GPUI edge sample when the native pointer is just inside a
                // borderless client inset; keep the global point for interior/cross-display drag.
                x: if raw_screen.x >= display_bounds.left
                    && raw_screen.x < display_bounds.right
                    && (local.x == selection_bounds.right
                        || raw_screen.x >= display_bounds.right.saturating_sub(8))
                {
                    local.x
                } else {
                    screen.x
                },
                y: if raw_screen.y >= display_bounds.top
                    && raw_screen.y < display_bounds.bottom
                    && (local.y == selection_bounds.bottom
                        || raw_screen.y >= display_bounds.bottom.saturating_sub(8))
                {
                    local.y
                } else {
                    screen.y
                },
            }),
            (_, Some(point), None) | (None, None, Some(point)) => Some(point),
            (None, Some(point), Some(_)) => Some(point),
            (Some(_), None, Some(point)) => Some(point),
            (Some(_), None, None) => None,
            (None, None, None) => None,
        }
        .map(|point| snap_selection_pointer_to_frame_edge(point, selection_bounds))
    } else if transform.fitted_view().contains(view) {
        transform.view_to_physical(view)
    } else {
        transform.view_to_physical(clamp_to_view(transform, position))
    }
}

/// Maps a display-space pointer into the captured frame when capture and display dimensions differ.
fn map_display_point_to_frame(
    point: PhysicalPoint,
    display_bounds: PhysicalRect,
    frame_bounds: PhysicalRect,
) -> PhysicalPoint {
    let map_axis = |value: i32,
                    display_start: i32,
                    display_extent: u32,
                    frame_start: i32,
                    frame_extent: u32| {
        if value < display_start {
            return frame_start.saturating_add(value.saturating_sub(display_start));
        }
        let display_end = display_start.saturating_add(display_extent as i32);
        if value >= display_end {
            return frame_start
                .saturating_add(frame_extent as i32)
                .saturating_add(value.saturating_sub(display_end));
        }
        let offset = value
            .saturating_sub(display_start)
            .clamp(0, display_extent.saturating_sub(1) as i32);
        frame_start.saturating_add(
            (i64::from(offset) * i64::from(frame_extent) / i64::from(display_extent.max(1))) as i32,
        )
    };
    PhysicalPoint {
        x: map_axis(
            point.x,
            display_bounds.left,
            display_bounds.width(),
            frame_bounds.left,
            frame_bounds.width(),
        ),
        y: map_axis(
            point.y,
            display_bounds.top,
            display_bounds.height(),
            frame_bounds.top,
            frame_bounds.height(),
        ),
    }
}

/// Treats the final addressable pixel as the exclusive edge when a physical drag reaches a frame border.
fn snap_selection_pointer_to_frame_edge(
    point: PhysicalPoint,
    bounds: PhysicalRect,
) -> PhysicalPoint {
    PhysicalPoint {
        x: if point.x == bounds.right.saturating_sub(1) {
            bounds.right
        } else {
            point.x
        },
        y: if point.y == bounds.bottom.saturating_sub(1) {
            bounds.bottom
        } else {
            point.y
        },
    }
}

fn intersect(left: PhysicalRect, right: PhysicalRect) -> Option<PhysicalRect> {
    let result = PhysicalRect {
        left: left.left.max(right.left),
        top: left.top.max(right.top),
        right: left.right.min(right.right),
        bottom: left.bottom.min(right.bottom),
    };
    (result.width() > 0 && result.height() > 0).then_some(result)
}

fn visible_selection(
    drag: SelectionDrag,
    committed_selection: Option<PhysicalRect>,
) -> Option<PhysicalRect> {
    // The local drag retains the editing bounds after mouse-up. Prefer it so
    // switching to an annotation tool cannot make the selection frame vanish.
    drag.selection()
        .filter(|selection| selection.width() > 0 && selection.height() > 0)
        .or(committed_selection)
}

/// Limits the fast completion gesture to the platform's second left-button click.
fn capture_double_click(click_count: usize) -> bool {
    click_count == 2
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ActionToolbarLayout {
    left: f32,
    top: f32,
    width: f32,
    height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct AnnotationToolbarLayout {
    left: f32,
    top: f32,
    width: f32,
    height: f32,
    tools_width: f32,
    tools_height: f32,
    tools_top: f32,
    style_left: f32,
    style_top: f32,
    style_height: f32,
    action_toolbar: ActionToolbarLayout,
    actions_above_tools: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AnnotationToolbarItems {
    selection_context: usize,
    arrange_context: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct AnnotationToolbarLayoutOptions {
    items: AnnotationToolbarItems,
    style_width: f32,
    style_height: f32,
    annotation_tool_group: Option<AnnotationToolGroup>,
    tool_estimated_width: f32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct AnnotationStyleCapabilities {
    color: bool,
    fill: bool,
    width: bool,
    opacity: bool,
    font_size: bool,
}

impl AnnotationStyleCapabilities {
    const EMPTY: Self = Self {
        color: false,
        fill: false,
        width: false,
        opacity: false,
        font_size: false,
    };

    const fn has_controls(self) -> bool {
        self.color || self.fill || self.width || self.opacity || self.font_size
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct AnnotationLayerLayout {
    left: f32,
    top: f32,
    max_height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SelectionDimensionLayout {
    left: f32,
    top: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SmartTargetHudLayout {
    target: InspectionTarget,
    left: f32,
    top: f32,
    width: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct WorkspaceSafeArea {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct WorkspaceSelectionAnchor {
    top_left: ViewPoint,
    bottom_right: ViewPoint,
}

impl WorkspaceSelectionAnchor {
    /// Converts one non-empty physical selection into the shared view-space anchor.
    ///
    /// Rejecting zero-area rectangles here keeps every workspace surface on the same validity
    /// rule. Callers must use this anchor instead of repeating physical-to-view conversions.
    fn from_selection(selection: PhysicalRect, transform: PreviewTransform) -> Option<Self> {
        if selection.width() == 0 || selection.height() == 0 {
            return None;
        }

        Some(Self {
            top_left: transform.physical_to_view(PhysicalPoint {
                x: selection.left,
                y: selection.top,
            }),
            bottom_right: transform.physical_to_view(PhysicalPoint {
                x: selection.right,
                y: selection.bottom,
            }),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SecondaryMenuLayout {
    left: f32,
    width: f32,
    height: f32,
    opens_above: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct AnnotationToolGroupLayout {
    left: f32,
    top: f32,
    width: f32,
    height: f32,
}

/// Captures one immutable workspace layout so every surface uses the same selection anchor.
///
/// The snapshot keeps the main row, context rows, transient menus, HUDs, and safe area together;
/// callers must not recalculate one of these values during rendering or input dispatch.
#[derive(Clone, Copy, Debug, PartialEq)]
struct WorkspaceLayoutSnapshot {
    safe_area: WorkspaceSafeArea,
    selection_anchor: Option<WorkspaceSelectionAnchor>,
    action_toolbar: Option<ActionToolbarLayout>,
    annotation_toolbar: Option<AnnotationToolbarLayout>,
    annotation_layer: Option<AnnotationLayerLayout>,
    secondary_menu: Option<SecondaryMenuLayout>,
    annotation_tool_group: Option<AnnotationToolGroupLayout>,
    dimension: Option<SelectionDimensionLayout>,
    smart_target_hud: Option<SmartTargetHudLayout>,
    status_inset: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct WorkspaceLayoutInput {
    selection: Option<PhysicalRect>,
    display_bounds: PhysicalRect,
    transform: Option<PreviewTransform>,
    viewport: Bounds<Pixels>,
    hover_pixel: Option<PhysicalPoint>,
    inspection_target: Option<InspectionTarget>,
    show_annotation_controls: bool,
    annotation_toolbar_items: AnnotationToolbarItems,
    annotation_style_capabilities: AnnotationStyleCapabilities,
    annotation_tool_group: Option<AnnotationToolGroup>,
    annotation_tool_width: f32,
    has_recognition_result: bool,
    has_recognition_retry: bool,
    recognition_in_flight: bool,
}

/// Maps a drawing tool to the style values that its renderer actually consumes.
///
/// Effect tools intentionally return no controls because their appearance is fixed by the
/// renderer; exposing unrelated values would create a misleading empty settings surface.
const fn annotation_style_capabilities_for_tool(
    tool: AnnotationTool,
) -> AnnotationStyleCapabilities {
    match tool {
        AnnotationTool::Watermark | AnnotationTool::Text => AnnotationStyleCapabilities {
            color: true,
            opacity: true,
            font_size: true,
            ..AnnotationStyleCapabilities::EMPTY
        },
        AnnotationTool::Number => AnnotationStyleCapabilities {
            color: true,
            opacity: true,
            ..AnnotationStyleCapabilities::EMPTY
        },
        AnnotationTool::Blur | AnnotationTool::Mosaic => AnnotationStyleCapabilities::EMPTY,
        AnnotationTool::Highlight => AnnotationStyleCapabilities {
            color: true,
            opacity: true,
            ..AnnotationStyleCapabilities::EMPTY
        },
        AnnotationTool::Rectangle | AnnotationTool::Ellipse => AnnotationStyleCapabilities {
            color: true,
            fill: true,
            width: true,
            opacity: true,
            ..AnnotationStyleCapabilities::EMPTY
        },
        AnnotationTool::Line | AnnotationTool::Arrow | AnnotationTool::Freehand => {
            AnnotationStyleCapabilities {
                color: true,
                width: true,
                opacity: true,
                ..AnnotationStyleCapabilities::EMPTY
            }
        }
    }
}

/// Maps a selected annotation to the same capabilities as the tool that created it.
const fn annotation_style_capabilities_for_kind(
    kind: &AnnotationKind,
) -> AnnotationStyleCapabilities {
    match kind {
        AnnotationKind::Watermark { .. } => {
            annotation_style_capabilities_for_tool(AnnotationTool::Watermark)
        }
        AnnotationKind::Text { .. } => annotation_style_capabilities_for_tool(AnnotationTool::Text),
        AnnotationKind::Number { .. } => {
            annotation_style_capabilities_for_tool(AnnotationTool::Number)
        }
        AnnotationKind::Blur { .. } => annotation_style_capabilities_for_tool(AnnotationTool::Blur),
        AnnotationKind::Mosaic { .. } => {
            annotation_style_capabilities_for_tool(AnnotationTool::Mosaic)
        }
        AnnotationKind::Highlight { .. } => {
            annotation_style_capabilities_for_tool(AnnotationTool::Highlight)
        }
        AnnotationKind::Rectangle { .. } => {
            annotation_style_capabilities_for_tool(AnnotationTool::Rectangle)
        }
        AnnotationKind::Ellipse { .. } => {
            annotation_style_capabilities_for_tool(AnnotationTool::Ellipse)
        }
        AnnotationKind::Line { .. } => annotation_style_capabilities_for_tool(AnnotationTool::Line),
        AnnotationKind::Arrow { .. } => {
            annotation_style_capabilities_for_tool(AnnotationTool::Arrow)
        }
        AnnotationKind::Freehand { .. } => {
            annotation_style_capabilities_for_tool(AnnotationTool::Freehand)
        }
    }
}

/// Chooses capabilities for the active drawing tool or the selected document object.
fn annotation_style_capabilities(
    tool: Option<AnnotationTool>,
    selected_annotation: Option<&Annotation>,
) -> AnnotationStyleCapabilities {
    tool.map(annotation_style_capabilities_for_tool)
        .or_else(|| {
            selected_annotation
                .map(|annotation| annotation_style_capabilities_for_kind(&annotation.kind))
        })
        .unwrap_or_default()
}

/// Measures all style groups at their natural width so the dock only grows for controls the
/// current annotation tool can actually use.
fn annotation_style_row_width(capabilities: AnnotationStyleCapabilities) -> f32 {
    if !capabilities.has_controls() {
        return 0.0;
    }
    annotation_style_group_widths(capabilities)
        .into_iter()
        .flatten()
        .sum::<f32>()
        + capabilities_group_count(capabilities).saturating_sub(1) as f32
            * ANNOTATION_STYLE_CONTROL_GAP
        + ANNOTATION_STYLE_ROW_PADDING * 2.0
        + ANNOTATION_TOOL_GROUP_POPUP_BORDER * 2.0
}

/// Finds the narrowest style row that preserves the row count allowed by the safe viewport width.
/// This keeps wrapped controls compact instead of leaving a wide empty strip to their right.
fn annotation_style_row_preferred_width(
    available_width: f32,
    capabilities: AnnotationStyleCapabilities,
) -> f32 {
    let natural_width = annotation_style_row_width(capabilities);
    if natural_width == 0.0 || natural_width <= available_width {
        return natural_width;
    }

    let target_height = annotation_style_row_height(available_width, capabilities);
    let minimum_group_width = annotation_style_group_widths(capabilities)
        .into_iter()
        .flatten()
        .fold(0.0_f32, f32::max)
        + ANNOTATION_STYLE_ROW_PADDING * 2.0
        + ANNOTATION_TOOL_GROUP_POPUP_BORDER * 2.0;
    let mut lower = minimum_group_width.min(available_width);
    let mut upper = available_width;
    for _ in 0..16 {
        let candidate = (lower + upper) / 2.0;
        if annotation_style_row_height(candidate, capabilities) <= target_height {
            upper = candidate;
        } else {
            lower = candidate;
        }
    }
    upper.ceil().min(available_width)
}

/// Returns the rendered width of each optional style group, including its leading divider.
fn annotation_style_group_widths(capabilities: AnnotationStyleCapabilities) -> [Option<f32>; 5] {
    const COLOR_GROUP_WIDTH: f32 =
        ThemeMetrics::WORKSPACE_SWATCH_SIZE * 5.0 + ANNOTATION_STYLE_CONTROL_GAP * 4.0;
    const WIDTH_GROUP_WIDTH: f32 =
        5.0 * ANNOTATION_STYLE_VALUE_WIDTH + ANNOTATION_STYLE_CONTROL_GAP * 4.0;
    const OPACITY_GROUP_WIDTH: f32 =
        4.0 * ANNOTATION_STYLE_OPACITY_WIDTH + ANNOTATION_STYLE_CONTROL_GAP * 3.0;
    const FONT_GROUP_WIDTH: f32 = WIDTH_GROUP_WIDTH;
    const GROUP_SEPARATOR_WIDTH: f32 =
        ThemeMetrics::WORKSPACE_SEPARATOR_WIDTH + ThemeMetrics::SPACE_2;

    [
        capabilities.color.then_some(COLOR_GROUP_WIDTH),
        capabilities
            .fill
            .then_some(ANNOTATION_STYLE_FILL_WIDTH + GROUP_SEPARATOR_WIDTH),
        capabilities
            .width
            .then_some(WIDTH_GROUP_WIDTH + GROUP_SEPARATOR_WIDTH),
        capabilities
            .opacity
            .then_some(OPACITY_GROUP_WIDTH + GROUP_SEPARATOR_WIDTH),
        capabilities
            .font_size
            .then_some(FONT_GROUP_WIDTH + GROUP_SEPARATOR_WIDTH),
    ]
}

/// Counts visible style groups so natural width includes only the separators actually rendered.
fn capabilities_group_count(capabilities: AnnotationStyleCapabilities) -> usize {
    usize::from(capabilities.color)
        + usize::from(capabilities.fill)
        + usize::from(capabilities.width)
        + usize::from(capabilities.opacity)
        + usize::from(capabilities.font_size)
}

/// Measures the attached style row using the same fixed control groups that the renderer wraps.
///
/// The estimate is intentionally conservative for localized labels and button padding. Returning
/// zero for an unsupported tool also removes the row's gap from the surrounding layout.
fn annotation_style_row_height(width: f32, capabilities: AnnotationStyleCapabilities) -> f32 {
    if !capabilities.has_controls() {
        return 0.0;
    }
    let group_widths = annotation_style_group_widths(capabilities);
    let available_width =
        (width - ANNOTATION_STYLE_ROW_PADDING * 2.0 - ANNOTATION_TOOL_GROUP_POPUP_BORDER * 2.0)
            .max(1.0);
    let mut rows: usize = 1;
    let mut row_width = 0.0;
    for group_width in group_widths.into_iter().flatten() {
        let next_width = if row_width == 0.0 {
            group_width
        } else {
            row_width + ANNOTATION_STYLE_CONTROL_GAP + group_width
        };
        if row_width > 0.0 && next_width > available_width {
            rows += 1;
            row_width = group_width;
        } else {
            row_width = next_width;
        }
    }
    ANNOTATION_STYLE_ROW_PADDING * 2.0
        + rows as f32 * ANNOTATION_STYLE_CONTROL_HEIGHT
        + rows.saturating_sub(1) as f32 * ANNOTATION_STYLE_CONTROL_GAP
        + ANNOTATION_TOOL_GROUP_POPUP_BORDER * 2.0
}

/// Retains the expanded Arrange group only while its originating annotation remains selected.
/// This prevents a later selection from unexpectedly opening a dense group of layer commands.
fn arrange_context_for_selection(
    expanded_for: Option<AnnotationId>,
    selected_annotation: Option<AnnotationId>,
) -> Option<AnnotationId> {
    expanded_for.filter(|id| Some(*id) == selected_annotation)
}

/// Counts the contextual controls that belong beside a selected annotation.
/// The drawing palette remains a separate stable section; arrange controls only consume space
/// after the user explicitly expands them.
fn annotation_toolbar_items(
    has_selected_annotation: bool,
    can_edit_text: bool,
    has_selected_number: bool,
    can_rotate: bool,
    show_arrange_context: bool,
) -> AnnotationToolbarItems {
    if !has_selected_annotation {
        return AnnotationToolbarItems {
            selection_context: 0,
            arrange_context: 0,
        };
    }

    AnnotationToolbarItems {
        // Selected label, Delete, Duplicate, and the Arrange toggle are always available.
        selection_context: 4 + usize::from(can_edit_text) + usize::from(has_selected_number) * 3,
        // Arrange label, four layer ordering commands, and an optional rotation command.
        arrange_context: usize::from(show_arrange_context) * (5 + usize::from(can_rotate)),
    }
}

/// Reserves enough vertical space for the annotation toolbar before style controls are placed.
/// Widths are deliberately conservative so localized or contextual labels cannot overlap the
/// rows below even when GPUI wraps them earlier than expected.
#[cfg(test)]
fn annotation_toolbar_height(viewport: Bounds<Pixels>, items: AnnotationToolbarItems) -> f32 {
    let viewport = view_rect(viewport);
    annotation_toolbar_height_for_width(
        (viewport.width - OVERLAY_EDGE_INSET * 2.0).max(1.0),
        items,
        None,
        ANNOTATION_TOOL_ESTIMATED_WIDTH,
    )
}

/// Measures the compact icon palette including its rounded surface and padding.
fn annotation_tool_palette_width() -> f32 {
    ANNOTATION_TOOL_PALETTE_ITEMS as f32 * ANNOTATION_TOOL_ICON_WIDTH
        + ANNOTATION_TOOL_PALETTE_ITEMS.saturating_sub(1) as f32 * ANNOTATION_TOOL_PALETTE_GAP
        + ANNOTATION_TOOLBAR_PADDING * 2.0
        + ANNOTATION_TOOL_GROUP_POPUP_BORDER * 2.0
}

/// Chooses a natural dock width from the visible tool icons, style groups, context actions, and
/// result toolbar, then clamps that width to the display's safe horizontal area.
fn annotation_toolbar_preferred_width(
    viewport_width: f32,
    action_toolbar_width: f32,
    items: AnnotationToolbarItems,
    style_width: f32,
    tool_estimated_width: f32,
) -> f32 {
    let context_width = |item_count: usize| {
        if item_count == 0 {
            0.0
        } else {
            item_count as f32 * tool_estimated_width
                + item_count.saturating_sub(1) as f32 * ANNOTATION_TOOL_GAP
                + ANNOTATION_TOOLBAR_PADDING * 2.0
                + ANNOTATION_TOOL_GROUP_POPUP_BORDER * 2.0
        }
    };
    let natural_width = action_toolbar_width
        .max(annotation_tool_palette_width())
        .max(style_width)
        .max(context_width(items.selection_context))
        .max(context_width(items.arrange_context));
    let available_width =
        (viewport_width - OVERLAY_EDGE_INSET * 2.0).clamp(1.0, ANNOTATION_TOOLBAR_MAX_WIDTH);
    natural_width.min(available_width)
}

/// Computes the actual icon-palette height so selection-anchored group popovers start below it.
fn annotation_tool_palette_height(width: f32) -> f32 {
    let content_width =
        (width - ANNOTATION_TOOLBAR_PADDING * 2.0 - ANNOTATION_TOOL_GROUP_POPUP_BORDER * 2.0)
            .max(1.0);
    let columns = (((content_width + ANNOTATION_TOOL_PALETTE_GAP)
        / (ANNOTATION_TOOL_ICON_WIDTH + ANNOTATION_TOOL_PALETTE_GAP))
        .floor() as usize)
        .max(1);
    let rows = ANNOTATION_TOOL_PALETTE_ITEMS.div_ceil(columns);
    rows as f32 * ANNOTATION_TOOL_ICON_WIDTH
        + rows.saturating_sub(1) as f32 * ANNOTATION_TOOL_PALETTE_GAP
        + ANNOTATION_TOOLBAR_PADDING * 2.0
        + ANNOTATION_TOOL_GROUP_POPUP_BORDER * 2.0
}

/// Computes a compact popover width from the group's actual children and the current locale's
/// button estimate, keeping the transient surface from spanning the whole annotation toolbar.
fn annotation_tool_group_popover_width(
    width: f32,
    group: AnnotationToolGroup,
    tool_estimated_width: f32,
) -> f32 {
    let available_width = width.max(1.0);
    let tools = group.spec().tools;
    let natural_width = tools.len() as f32 * tool_estimated_width
        + tools.len().saturating_sub(1) as f32 * ANNOTATION_TOOL_GAP
        + ANNOTATION_TOOL_GROUP_POPUP_PADDING * 2.0
        + ANNOTATION_TOOL_GROUP_POPUP_BORDER * 2.0;
    natural_width.min(available_width)
}

/// Computes wrapped rows for the materialized group using the same dimensions as its renderer.
fn annotation_tool_group_popover_height(
    width: f32,
    group: AnnotationToolGroup,
    tool_estimated_width: f32,
) -> f32 {
    let popup_width = annotation_tool_group_popover_width(width, group, tool_estimated_width);
    let content_width = (popup_width
        - ANNOTATION_TOOL_GROUP_POPUP_PADDING * 2.0
        - ANNOTATION_TOOL_GROUP_POPUP_BORDER * 2.0)
        .max(1.0);
    let columns = (((content_width + ANNOTATION_TOOL_GAP)
        / (tool_estimated_width + ANNOTATION_TOOL_GAP))
        .floor() as usize)
        .max(1);
    let rows = group.spec().tools.len().div_ceil(columns);
    rows as f32 * ANNOTATION_TOOL_ROW_HEIGHT
        + rows.saturating_sub(1) as f32 * ANNOTATION_TOOL_GAP
        + ANNOTATION_TOOL_GROUP_POPUP_PADDING * 2.0
        + ANNOTATION_TOOL_GROUP_POPUP_BORDER * 2.0
}

/// Measures each visible toolbar section independently so a selection context cannot reorder the
/// drawing palette or reserve rows until it is actually expanded.
fn annotation_toolbar_height_for_width(
    width: f32,
    items: AnnotationToolbarItems,
    annotation_tool_group: Option<AnnotationToolGroup>,
    tool_estimated_width: f32,
) -> f32 {
    let content_width = width.max(1.0);
    let palette_columns = (((content_width
        - ANNOTATION_TOOLBAR_PADDING * 2.0
        - ANNOTATION_TOOL_GROUP_POPUP_BORDER * 2.0
        + ANNOTATION_TOOL_PALETTE_GAP)
        / (ANNOTATION_TOOL_ICON_WIDTH + ANNOTATION_TOOL_PALETTE_GAP))
        .floor() as usize)
        .max(1);
    let context_columns = (((content_width + ANNOTATION_TOOL_GAP)
        / (tool_estimated_width + ANNOTATION_TOOL_GAP))
        .floor() as usize)
        .max(1);
    let rows_height = |item_count: usize, columns: usize, item_height: f32, gap: f32| {
        let rows = item_count.max(1).div_ceil(columns);
        rows as f32 * item_height + rows.saturating_sub(1) as f32 * gap
    };
    let palette_height = rows_height(
        ANNOTATION_TOOL_PALETTE_ITEMS,
        palette_columns,
        ANNOTATION_TOOL_ICON_WIDTH,
        ANNOTATION_TOOL_PALETTE_GAP,
    ) + ANNOTATION_TOOLBAR_PADDING * 2.0
        + ANNOTATION_TOOL_GROUP_POPUP_BORDER * 2.0;
    let selection_context_height = if items.selection_context > 0 {
        rows_height(
            items.selection_context,
            context_columns,
            ANNOTATION_TOOL_ROW_HEIGHT,
            ANNOTATION_TOOL_GAP,
        ) + ANNOTATION_TOOLBAR_PADDING
            + ANNOTATION_TOOL_GROUP_POPUP_BORDER
    } else {
        0.0
    };
    let arrange_context_height = if items.arrange_context > 0 {
        rows_height(
            items.arrange_context,
            context_columns,
            ANNOTATION_TOOL_ROW_HEIGHT,
            ANNOTATION_TOOL_GAP,
        ) + ANNOTATION_TOOLBAR_PADDING
            + ANNOTATION_TOOL_GROUP_POPUP_BORDER
    } else {
        0.0
    };
    let tool_group_height = annotation_tool_group
        .map(|group| annotation_tool_group_popover_height(width, group, tool_estimated_width))
        .unwrap_or(0.0);
    let tool_group_gap = annotation_tool_group
        .map(|_| ANNOTATION_TOOL_GAP)
        .unwrap_or(0.0);
    palette_height
        + tool_group_height
        + tool_group_gap
        + selection_context_height
        + arrange_context_height
        + usize::from(items.selection_context > 0) as f32 * ANNOTATION_TOOL_GAP
        + usize::from(items.arrange_context > 0) as f32 * ANNOTATION_TOOL_GAP
}

/// Keeps marking modes, the attached style row, and selection commands in one dock beside the
/// selection. The style row always follows the tool palette, so it cannot become a detached side
/// panel at wide widths.
#[cfg(test)]
fn annotation_toolbar_layout(
    selection_anchor: Option<WorkspaceSelectionAnchor>,
    viewport: Bounds<Pixels>,
    action_toolbar: Option<ActionToolbarLayout>,
    items: AnnotationToolbarItems,
    style_width: f32,
    style_height: f32,
    annotation_tool_group: Option<AnnotationToolGroup>,
) -> Option<AnnotationToolbarLayout> {
    annotation_toolbar_layout_with_tool_width(
        selection_anchor,
        viewport,
        action_toolbar,
        AnnotationToolbarLayoutOptions {
            items,
            style_width,
            style_height,
            annotation_tool_group,
            tool_estimated_width: ANNOTATION_TOOL_ESTIMATED_WIDTH,
        },
    )
}

/// Lays out the compact icon dock using natural visible widths and the shared selection anchor.
fn annotation_toolbar_layout_with_tool_width(
    selection_anchor: Option<WorkspaceSelectionAnchor>,
    viewport: Bounds<Pixels>,
    action_toolbar: Option<ActionToolbarLayout>,
    options: AnnotationToolbarLayoutOptions,
) -> Option<AnnotationToolbarLayout> {
    let selection_anchor = selection_anchor?;
    let action_toolbar = action_toolbar?;
    let viewport = view_rect(viewport);
    let width = annotation_toolbar_preferred_width(
        viewport.width,
        action_toolbar.width,
        options.items,
        options.style_width,
        options.tool_estimated_width,
    );
    let tools_width = width;
    let tools_height = annotation_toolbar_height_for_width(
        tools_width,
        options.items,
        options.annotation_tool_group,
        options.tool_estimated_width,
    )
    .max(action_toolbar.height);
    let style_gap = if options.style_height > 0.0 {
        ANNOTATION_STYLE_PANEL_GAP
    } else {
        0.0
    };
    // In marking mode the palette and result actions share one horizontal surface. The style
    // row remains attached below it only when the selected tool exposes renderer-backed values.
    let tools_and_style_height = tools_height + style_gap + options.style_height;
    let total_height = tools_and_style_height;
    let top_left = selection_anchor.top_left;
    let bottom_right = selection_anchor.bottom_right;
    let left_min = viewport.left + OVERLAY_EDGE_INSET;
    let left_max = (viewport.right() - OVERLAY_EDGE_INSET - width).max(left_min);
    let left = (bottom_right.x - width).clamp(left_min, left_max);
    let top_min = viewport.top + OVERLAY_EDGE_INSET;
    let top_max = (viewport.bottom() - OVERLAY_BOTTOM_SAFE_INSET - total_height).max(top_min);
    let below_selection = bottom_right.y + OVERLAY_ACTION_BAR_GAP;
    let above_selection = top_left.y - OVERLAY_ACTION_BAR_GAP - total_height;
    let can_fit_below = below_selection <= top_max;
    let can_fit_above = above_selection >= top_min;
    let (top, actions_above_tools) = if can_fit_below {
        (below_selection, false)
    } else if can_fit_above {
        (above_selection, true)
    } else {
        let room_below = viewport.bottom() - OVERLAY_BOTTOM_SAFE_INSET - below_selection;
        let room_above = top_left.y - OVERLAY_ACTION_BAR_GAP - top_min;
        if room_below >= room_above {
            (below_selection.clamp(top_min, top_max), false)
        } else {
            (above_selection.clamp(top_min, top_max), true)
        }
    };
    let tools_top = top;
    let action_top = top;
    let style_left = left;
    let style_top = tools_top + tools_height + style_gap;
    Some(AnnotationToolbarLayout {
        left,
        top,
        width,
        height: total_height,
        tools_width,
        tools_height,
        tools_top,
        style_left,
        style_top,
        style_height: options.style_height,
        action_toolbar: ActionToolbarLayout {
            left,
            top: action_top,
            width,
            height: action_toolbar.height,
        },
        actions_above_tools,
    })
}

/// Places layer management after the marking panel so it never covers tool or style controls.
fn annotation_layer_layout(
    toolbar: AnnotationToolbarLayout,
    viewport: Bounds<Pixels>,
) -> Option<AnnotationLayerLayout> {
    let viewport = view_rect(viewport);
    let requested_left = toolbar.left + toolbar.width - ANNOTATION_LAYERS_WIDTH;
    let left_min = viewport.left + OVERLAY_EDGE_INSET;
    let left_max = (viewport.right() - OVERLAY_EDGE_INSET - ANNOTATION_LAYERS_WIDTH).max(left_min);
    let top_min = viewport.top + OVERLAY_EDGE_INSET;
    let safe_bottom = viewport.bottom() - OVERLAY_BOTTOM_SAFE_INSET;
    let below_top = toolbar.top + toolbar.height + ANNOTATION_STYLE_PANEL_GAP;
    let (top, max_height) = if below_top + 80.0 <= safe_bottom {
        (below_top, safe_bottom - below_top)
    } else {
        let available_above = toolbar.top - ANNOTATION_STYLE_PANEL_GAP - top_min;
        if available_above < 80.0 {
            return None;
        }
        let top = (toolbar.top - ANNOTATION_STYLE_PANEL_GAP - ANNOTATION_LAYERS_PREFERRED_HEIGHT)
            .max(top_min);
        (top, toolbar.top - ANNOTATION_STYLE_PANEL_GAP - top)
    };
    Some(AnnotationLayerLayout {
        left: requested_left.clamp(left_min, left_max),
        top,
        max_height: max_height.max(80.0),
    })
}

/// Positions the pixel-size readout near the selection without covering export controls.
///
/// When an edge selection lifts the toolbar above itself, the label moves into the gap above that
/// toolbar instead of colliding with it or disappearing under the bottom taskbar-safe area.
fn selection_dimension_label_layout(
    selection_anchor: Option<WorkspaceSelectionAnchor>,
    viewport: Bounds<Pixels>,
    action_toolbar: Option<ActionToolbarLayout>,
) -> Option<SelectionDimensionLayout> {
    let selection_anchor = selection_anchor?;
    let viewport = view_rect(viewport);
    let top_left = selection_anchor.top_left;
    let bottom_right = selection_anchor.bottom_right;
    let left_min = viewport.left + OVERLAY_EDGE_INSET;
    let left_max =
        (viewport.right() - OVERLAY_EDGE_INSET - OVERLAY_DIMENSION_LABEL_WIDTH).max(left_min);
    let left = top_left.x.clamp(left_min, left_max);
    let above = top_left.y - OVERLAY_DIMENSION_LABEL_HEIGHT - OVERLAY_DIMENSION_LABEL_GAP;
    let below = bottom_right.y + OVERLAY_DIMENSION_LABEL_GAP;
    let top_min = viewport.top + OVERLAY_EDGE_INSET;
    let top_max = viewport.bottom() - OVERLAY_BOTTOM_SAFE_INSET - OVERLAY_DIMENSION_LABEL_HEIGHT;
    let overlaps_toolbar = |top: f32| {
        action_toolbar.is_some_and(|toolbar| {
            top < toolbar.top + toolbar.height && top + OVERLAY_DIMENSION_LABEL_HEIGHT > toolbar.top
        })
    };
    let above_toolbar = action_toolbar.map(|toolbar| {
        (toolbar.top - OVERLAY_DIMENSION_LABEL_HEIGHT - OVERLAY_DIMENSION_LABEL_GAP).max(top_min)
    });
    let top = if above >= top_min && !overlaps_toolbar(above) {
        above
    } else if below <= top_max && !overlaps_toolbar(below) {
        below
    } else if let Some(above_toolbar) = above_toolbar.filter(|top| !overlaps_toolbar(*top)) {
        above_toolbar
    } else if above >= top_min {
        above
    } else {
        below.min(top_max)
    };
    Some(SelectionDimensionLayout { left, top })
}

/// Places a candidate HUD only on the display under the current pointer and only before selection.
fn smart_target_hud_layout(
    selection: Option<PhysicalRect>,
    hover_pixel: Option<PhysicalPoint>,
    target: Option<InspectionTarget>,
    display_bounds: PhysicalRect,
    transform: Option<PreviewTransform>,
    viewport: Bounds<Pixels>,
) -> Option<SmartTargetHudLayout> {
    if selection.is_some() {
        return None;
    }
    let hover_pixel = hover_pixel?;
    let target = target.filter(|target| target.bounds.contains(hover_pixel))?;
    if !display_bounds.contains(hover_pixel) {
        return None;
    }
    let visible_bounds = intersect(target.bounds, display_bounds)?;
    let transform = transform?;
    let viewport = view_rect(viewport);
    let top_left = transform.physical_to_view(PhysicalPoint {
        x: visible_bounds.left,
        y: visible_bounds.top,
    });
    let bottom_right = transform.physical_to_view(PhysicalPoint {
        x: visible_bounds.right,
        y: visible_bounds.bottom,
    });
    let left_min = viewport.left + OVERLAY_EDGE_INSET;
    let width =
        OVERLAY_SMART_TARGET_HUD_WIDTH.min((viewport.width - OVERLAY_EDGE_INSET * 2.0).max(1.0));
    let left_max = (viewport.right() - OVERLAY_EDGE_INSET - width).max(left_min);
    let top_min = viewport.top + OVERLAY_EDGE_INSET;
    let top_max = (viewport.bottom() - OVERLAY_BOTTOM_SAFE_INSET - OVERLAY_SMART_TARGET_HUD_HEIGHT)
        .max(top_min);
    let above = top_left.y - OVERLAY_SMART_TARGET_HUD_HEIGHT - OVERLAY_SMART_TARGET_HUD_GAP;
    let below = bottom_right.y + OVERLAY_SMART_TARGET_HUD_GAP;
    let top = if above >= top_min {
        above.min(top_max)
    } else if below <= top_max {
        below
    } else {
        above.max(top_min).min(top_max)
    };
    Some(SmartTargetHudLayout {
        target,
        left: top_left.x.clamp(left_min, left_max),
        top,
        width,
    })
}

/// Formats the compact smart-target HUD label without exposing a window title.
fn smart_target_hud_label(locale: Locale, target: InspectionTarget) -> String {
    let kind = inspection_kind_label(locale, target.kind);
    let width = target.bounds.width().to_string();
    let height = target.bounds.height().to_string();
    locale.format_template(
        UiText::SmartTargetLabel,
        &[("kind", kind), ("width", &width), ("height", &height)],
    )
}

/// Places the stable main row near the selection using only shared icon geometry and safe bounds.
fn action_toolbar_layout(
    selection_anchor: Option<WorkspaceSelectionAnchor>,
    viewport: Bounds<Pixels>,
    show_annotation_controls: bool,
) -> Option<ActionToolbarLayout> {
    let selection_anchor = selection_anchor?;
    let viewport = view_rect(viewport);
    let available_width = (viewport.width - OVERLAY_EDGE_INSET * 2.0).max(1.0);
    let width = action_toolbar_natural_width(show_annotation_controls).min(available_width);
    let height = action_toolbar_height(width, show_annotation_controls);
    let selection_top = selection_anchor.top_left.y;
    let selection_bottom = selection_anchor.bottom_right.y;
    let selection_right = selection_anchor.bottom_right.x;
    let left_min = viewport.left + OVERLAY_EDGE_INSET;
    let left_limit = (viewport.right() - OVERLAY_EDGE_INSET - width).max(left_min);
    let left = (selection_right - width).clamp(left_min, left_limit);
    let lowest_top = (viewport.bottom() - OVERLAY_BOTTOM_SAFE_INSET - height)
        .max(viewport.top + OVERLAY_EDGE_INSET);
    let below = selection_bottom + OVERLAY_ACTION_BAR_GAP;
    let above = selection_top - height - OVERLAY_ACTION_BAR_GAP;
    let top = if below <= lowest_top {
        below
    } else {
        above.max(viewport.top + OVERLAY_EDGE_INSET).min(lowest_top)
    };
    Some(ActionToolbarLayout {
        left,
        top,
        width,
        height,
    })
}

/// Computes every workspace surface from one selection/display snapshot.
///
/// The renderer and interaction tests consume this result instead of independently deciding
/// where the action row, style rows, More menu, and transient HUD should move.
fn workspace_layout_snapshot(input: WorkspaceLayoutInput) -> WorkspaceLayoutSnapshot {
    let WorkspaceLayoutInput {
        selection,
        display_bounds,
        transform,
        viewport,
        hover_pixel,
        inspection_target,
        show_annotation_controls,
        annotation_toolbar_items,
        annotation_style_capabilities,
        annotation_tool_group,
        annotation_tool_width,
        has_recognition_result,
        has_recognition_retry,
        recognition_in_flight,
    } = input;
    let viewport_rect = view_rect(viewport);
    let safe_area = WorkspaceSafeArea {
        left: viewport_rect.left + OVERLAY_EDGE_INSET,
        top: viewport_rect.top + OVERLAY_EDGE_INSET,
        right: viewport_rect.right() - OVERLAY_EDGE_INSET,
        bottom: viewport_rect.bottom() - OVERLAY_BOTTOM_SAFE_INSET,
    };
    let selected_on_display = selection.and_then(|selection| intersect(selection, display_bounds));
    let selection_anchor = selected_on_display
        .zip(transform)
        .and_then(|(selection, transform)| {
            WorkspaceSelectionAnchor::from_selection(selection, transform)
        });
    let owns_action_toolbar =
        selection.is_some_and(|selection| owns_selection_toolbar(selection, display_bounds));
    let base_action_layout =
        action_toolbar_layout(selection_anchor, viewport, show_annotation_controls);
    let available_toolbar_width =
        (viewport_rect.width - OVERLAY_EDGE_INSET * 2.0).clamp(1.0, ANNOTATION_TOOLBAR_MAX_WIDTH);
    let style_width = annotation_style_row_preferred_width(
        available_toolbar_width,
        annotation_style_capabilities,
    );
    let toolbar_width = annotation_toolbar_preferred_width(
        viewport_rect.width,
        base_action_layout.map(|layout| layout.width).unwrap_or(0.0),
        annotation_toolbar_items,
        style_width,
        annotation_tool_width,
    );
    let style_height = annotation_style_row_height(toolbar_width, annotation_style_capabilities);
    let annotation_layout = show_annotation_controls
        .then(|| {
            annotation_toolbar_layout_with_tool_width(
                selection_anchor,
                viewport,
                base_action_layout,
                AnnotationToolbarLayoutOptions {
                    items: annotation_toolbar_items,
                    style_width,
                    style_height,
                    annotation_tool_group,
                    tool_estimated_width: annotation_tool_width,
                },
            )
        })
        .flatten();
    let action_layout = annotation_layout
        .map(|layout| layout.action_toolbar)
        .or(base_action_layout);
    let annotation_layer = owns_action_toolbar
        .then(|| annotation_layout.and_then(|layout| annotation_layer_layout(layout, viewport)))
        .flatten();
    let secondary_menu = action_layout.map(|layout| {
        let width = secondary_action_menu_width(
            viewport_rect.width,
            has_recognition_result,
            has_recognition_retry,
        );
        let height = secondary_action_menu_height(
            width,
            has_recognition_result,
            has_recognition_retry,
            recognition_in_flight,
        );
        let opens_above = if let Some(marking) = annotation_layout {
            let menu_offset = secondary_action_menu_offset();
            let above = layout.top - menu_offset - height;
            let below = layout.top + layout.height + menu_offset + height;
            if marking.actions_above_tools && above >= safe_area.top {
                true
            } else if !marking.actions_above_tools && below <= safe_area.bottom {
                false
            } else {
                secondary_menu_opens_above(layout, viewport, height)
            }
        } else {
            secondary_menu_opens_above(layout, viewport, height)
        };
        SecondaryMenuLayout {
            left: secondary_action_menu_left(layout, width, viewport),
            width,
            height,
            opens_above,
        }
    });
    let annotation_tool_group =
        annotation_layout
            .zip(annotation_tool_group)
            .map(|(layout, group)| AnnotationToolGroupLayout {
                left: layout.left,
                top: layout.tools_top
                    + annotation_tool_palette_height(layout.tools_width)
                    + ANNOTATION_TOOL_GAP,
                width: annotation_tool_group_popover_width(
                    layout.tools_width,
                    group,
                    annotation_tool_width,
                ),
                height: annotation_tool_group_popover_height(
                    layout.tools_width,
                    group,
                    annotation_tool_width,
                ),
            });
    let dimension = selection_dimension_label_layout(selection_anchor, viewport, action_layout);
    let smart_target_hud = smart_target_hud_layout(
        selection,
        hover_pixel,
        inspection_target,
        display_bounds,
        transform,
        viewport,
    );
    let status_inset = annotation_layout
        .filter(|layout| layout.style_height > 0.0 && !layout.actions_above_tools)
        .map(|layout| status_bottom_inset_for_stacked_annotation(layout, dimension, viewport))
        .unwrap_or_else(|| status_bottom_inset(action_layout.is_none()));
    WorkspaceLayoutSnapshot {
        safe_area,
        selection_anchor,
        action_toolbar: action_layout,
        annotation_toolbar: annotation_layout,
        annotation_layer,
        secondary_menu,
        annotation_tool_group,
        dimension,
        smart_target_hud,
        status_inset,
    }
}

/// Assigns one cross-display selection to the screen nearest its export controls.
fn owns_selection_toolbar(selection: PhysicalRect, display_bounds: PhysicalRect) -> bool {
    selection.width() > 0
        && selection.height() > 0
        && display_bounds.contains(PhysicalPoint {
            x: selection.right.saturating_sub(1),
            y: selection.bottom.saturating_sub(1),
        })
}

/// Limits annotation controls to the display that owns the committed selection's export actions.
fn annotation_controls_visible(
    requested: bool,
    selection: Option<PhysicalRect>,
    display_bounds: PhysicalRect,
) -> bool {
    requested
        && selection.is_some_and(|selection| owns_selection_toolbar(selection, display_bounds))
}

/// Chooses the side with room for the detached secondary menu without moving the main toolbar.
fn secondary_menu_opens_above(
    toolbar: ActionToolbarLayout,
    viewport: Bounds<Pixels>,
    menu_height: f32,
) -> bool {
    let viewport = view_rect(viewport);
    let menu_offset = secondary_action_menu_offset();
    let top = toolbar.top - menu_offset - menu_height;
    let bottom = toolbar.top + toolbar.height + menu_offset + menu_height;
    top >= viewport.top + OVERLAY_EDGE_INSET
        || bottom > viewport.bottom() - OVERLAY_BOTTOM_SAFE_INSET
}

/// Keeps the detached More panel clear of the main toolbar's padded border.
fn secondary_action_menu_offset() -> f32 {
    OVERLAY_ACTION_ITEM_HEIGHT + OVERLAY_ACTION_BAR_PADDING * 2.0 + OVERLAY_SECONDARY_MENU_GAP
}

/// Lifts status feedback above the fallback action bar so narrow overlays stay readable.
fn status_bottom_inset(uses_fallback_action_bar: bool) -> f32 {
    if uses_fallback_action_bar {
        OVERLAY_BOTTOM_SAFE_INSET + OVERLAY_ACTION_ITEM_HEIGHT + OVERLAY_ACTION_BAR_GAP
    } else {
        OVERLAY_BOTTOM_SAFE_INSET
    }
}

/// Keeps status feedback in the reserved gap before a stacked style row and its action bar.
fn status_bottom_inset_for_stacked_annotation(
    layout: AnnotationToolbarLayout,
    dimension_layout: Option<SelectionDimensionLayout>,
    viewport: Bounds<Pixels>,
) -> f32 {
    let viewport = view_rect(viewport);
    let blocking_top = dimension_layout
        .map(|layout| layout.top)
        .unwrap_or(layout.top);
    let desired_top = blocking_top - OVERLAY_STATUS_ESTIMATED_HEIGHT - OVERLAY_ACTION_BAR_GAP;
    let top = desired_top.max(viewport.top + OVERLAY_EDGE_INSET);
    (viewport.bottom() - top - OVERLAY_STATUS_ESTIMATED_HEIGHT).max(0.0)
}

/// Computes the menu height so recognition feedback has room without covering the capture toolbar.
fn secondary_action_menu_height(
    width: f32,
    has_recognition_result: bool,
    has_recognition_retry: bool,
    recognition_in_flight: bool,
) -> f32 {
    let widths = secondary_action_widths(has_recognition_result, has_recognition_retry);
    action_toolbar_height_for(width, widths.iter().copied())
        + if has_recognition_result {
            OVERLAY_RECOGNITION_PREVIEW_HEIGHT + OVERLAY_ACTION_ITEM_GAP
        } else {
            0.0
        }
        + if recognition_in_flight {
            OVERLAY_RECOGNITION_STATUS_HEIGHT + OVERLAY_ACTION_ITEM_GAP
        } else {
            0.0
        }
}

/// Returns the visible More actions in their stable layout order for height and width estimates.
fn secondary_action_widths(has_recognition_result: bool, has_recognition_retry: bool) -> Vec<f32> {
    let mut widths = OVERLAY_MORE_ACTION_WIDTHS.to_vec();
    if has_recognition_result {
        widths.extend(OVERLAY_RECOGNITION_ACTION_WIDTHS);
    }
    if has_recognition_retry {
        widths.extend(OVERLAY_RETRY_ACTION_WIDTHS);
    }
    widths
}

/// Finds a compact preferred More-panel width that keeps its common actions to four rows.
fn secondary_action_menu_preferred_width(
    has_recognition_result: bool,
    has_recognition_retry: bool,
) -> f32 {
    let widths = secondary_action_widths(has_recognition_result, has_recognition_retry);
    let minimum_width = widths.iter().copied().fold(0.0, f32::max)
        + OVERLAY_ACTION_BAR_PADDING * 2.0
        + OVERLAY_ACTION_BAR_BORDER * 2.0;
    let maximum_width = widths.iter().sum::<f32>()
        + widths.len().saturating_sub(1) as f32 * OVERLAY_ACTION_ITEM_GAP
        + OVERLAY_ACTION_BAR_PADDING * 2.0
        + OVERLAY_ACTION_BAR_BORDER * 2.0;
    let mut width = minimum_width.ceil();
    while width <= maximum_width {
        if action_toolbar_row_count(width, widths.iter().copied()) <= 4 {
            return width;
        }
        width += 1.0;
    }
    maximum_width
}

/// Caps the More panel by the current safe viewport while preserving its preferred content width.
fn secondary_action_menu_width(
    viewport_width: f32,
    has_recognition_result: bool,
    has_recognition_retry: bool,
) -> f32 {
    let available_width = (viewport_width - OVERLAY_EDGE_INSET * 2.0).max(1.0);
    secondary_action_menu_preferred_width(has_recognition_result, has_recognition_retry)
        .min(available_width)
}

/// Centers the More panel near its trigger while clamping it to the overlay's safe horizontal area.
fn secondary_action_menu_left(
    toolbar: ActionToolbarLayout,
    menu_width: f32,
    viewport: Bounds<Pixels>,
) -> f32 {
    let viewport = view_rect(viewport);
    let left_min = viewport.left + OVERLAY_EDGE_INSET - toolbar.left;
    let left_max = viewport.right() - OVERLAY_EDGE_INSET - menu_width - toolbar.left;
    ((toolbar.width - menu_width) / 2.0).clamp(left_min, left_max)
}

/// Gives the retry control a precise action without exposing internal failure categories.
fn recognition_retry_label(locale: Locale, retry: super::RecognitionRetry) -> &'static str {
    match retry {
        super::RecognitionRetry::Ocr => locale.text(UiText::OverlayRetryOcr),
        super::RecognitionRetry::Translation => locale.text(UiText::OverlayRetryTranslation),
    }
}

/// Bounds visible OCR, QR, and translation text so a result never covers the screenshot controls.
fn recognition_result_preview(text: &str) -> String {
    let mut preview: String = text
        .chars()
        .take(OVERLAY_RECOGNITION_PREVIEW_LIMIT)
        .collect();
    if text
        .chars()
        .nth(OVERLAY_RECOGNITION_PREVIEW_LIMIT)
        .is_some()
    {
        preview.push_str("...");
    }
    preview
}

/// Lists the visible main-row group widths, including the inline annotation palette in marking mode.
fn action_toolbar_item_widths(show_annotation_controls: bool) -> Vec<f32> {
    if show_annotation_controls {
        let context_width =
            2.0 * ThemeMetrics::WORKSPACE_ICON_BUTTON_HIT_AREA + OVERLAY_ACTION_ITEM_GAP;
        let annotation_width = ANNOTATION_TOOL_PALETTE_ITEMS as f32 * ANNOTATION_TOOL_ICON_WIDTH
            + ANNOTATION_TOOL_PALETTE_ITEMS.saturating_sub(1) as f32 * ANNOTATION_TOOL_PALETTE_GAP
            + ThemeMetrics::WORKSPACE_SEPARATOR_WIDTH;
        let result_width =
            5.0 * ThemeMetrics::WORKSPACE_ICON_BUTTON_HIT_AREA + 4.0 * OVERLAY_ACTION_ITEM_GAP;
        vec![
            context_width,
            ThemeMetrics::WORKSPACE_SEPARATOR_WIDTH,
            annotation_width,
            ThemeMetrics::WORKSPACE_SEPARATOR_WIDTH,
            result_width,
        ]
    } else {
        vec![
            ThemeMetrics::WORKSPACE_ICON_BUTTON_HIT_AREA,
            5.0 * ThemeMetrics::WORKSPACE_ICON_BUTTON_HIT_AREA + 4.0 * OVERLAY_ACTION_ITEM_GAP,
        ]
    }
}

/// Measures the visible main row from shared geometry instead of a per-action width table.
fn action_toolbar_natural_width(show_annotation_controls: bool) -> f32 {
    let widths = action_toolbar_item_widths(show_annotation_controls);
    widths.iter().sum::<f32>()
        + widths.len().saturating_sub(1) as f32 * OVERLAY_ACTION_ITEM_GAP
        + OVERLAY_ACTION_BAR_PADDING * 2.0
        + OVERLAY_ACTION_BAR_BORDER * 2.0
}

/// Computes wrapped-row height from the shared icon hit size when a narrow viewport requires it.
fn action_toolbar_height(width: f32, show_annotation_controls: bool) -> f32 {
    action_toolbar_height_for(width, action_toolbar_item_widths(show_annotation_controls))
}

/// Computes the total toolbar height from wrapped rows and shared vertical tokens.
fn action_toolbar_height_for(width: f32, widths: impl IntoIterator<Item = f32>) -> f32 {
    let rows = action_toolbar_row_count(width, widths);
    rows as f32 * OVERLAY_ACTION_ITEM_HEIGHT
        + rows.saturating_sub(1) as f32 * OVERLAY_ACTION_ITEM_GAP
        + OVERLAY_ACTION_BAR_PADDING * 2.0
        + OVERLAY_ACTION_BAR_BORDER * 2.0
}

/// Counts flex rows using the same padded content width as the rendered toolbar.
fn action_toolbar_row_count(width: f32, widths: impl IntoIterator<Item = f32>) -> u32 {
    let mut rows = 1_u32;
    let mut row_width = 0.0;
    let content_width =
        (width - OVERLAY_ACTION_BAR_PADDING * 2.0 - OVERLAY_ACTION_BAR_BORDER * 2.0).max(1.0);
    for item_width in widths {
        let next_width = if row_width == 0.0 {
            item_width
        } else {
            row_width + OVERLAY_ACTION_ITEM_GAP + item_width
        };
        if row_width > 0.0 && next_width > content_width {
            rows = rows.saturating_add(1);
            row_width = item_width;
        } else {
            row_width = next_width;
        }
    }
    rows
}

/// Chooses pointer feedback without letting selection movement override an active drawing tool.
fn selection_cursor(
    selection: Option<PhysicalRect>,
    transform: Option<PreviewTransform>,
    pointer: ViewPoint,
    annotation_tool_active: bool,
    moving: bool,
) -> SelectionCursor {
    if annotation_tool_active {
        return SelectionCursor::Crosshair;
    }
    if moving {
        return SelectionCursor::Move;
    }
    let Some((selection, transform)) = selection.zip(transform) else {
        return SelectionCursor::Crosshair;
    };
    if let Some(handle) = transform.resize_handle_at(selection, pointer, 10.0) {
        return match handle {
            crate::domain::selection::ResizeHandle::TopLeft
            | crate::domain::selection::ResizeHandle::BottomRight => SelectionCursor::ResizeNwse,
            crate::domain::selection::ResizeHandle::TopRight
            | crate::domain::selection::ResizeHandle::BottomLeft => SelectionCursor::ResizeNesw,
        };
    }
    if transform
        .view_to_physical(pointer)
        .is_some_and(|point| selection.contains(point))
    {
        SelectionCursor::Move
    } else {
        SelectionCursor::Crosshair
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ANNOTATION_TOOL_ESTIMATED_WIDTH, ANNOTATION_TOOL_GAP, ANNOTATION_TOOL_GROUP_POPUP_BORDER,
        ANNOTATION_TOOL_GROUP_POPUP_PADDING, ANNOTATION_TOOL_GROUP_SPECS,
        ANNOTATION_TOOL_ROW_HEIGHT, ANNOTATION_WIDTHS, ActionToolbarLayout,
        AnnotationStyleCapabilities, AnnotationToolGroup, AnnotationToolbarLayout, FrameInputBatch,
        MAGNIFIER_CELL_SIZE, MAGNIFIER_RADIUS, OVERLAY_ACTION_BAR_GAP, OVERLAY_ACTION_BAR_PADDING,
        OVERLAY_ACTION_ITEM_HEIGHT, OVERLAY_BOTTOM_SAFE_INSET, OVERLAY_EDGE_INSET,
        OVERLAY_MORE_ACTION_WIDTHS, OVERLAY_MORE_ACTIONS_ID, OVERLAY_RECOGNITION_PREVIEW_LIMIT,
        OVERLAY_SECONDARY_MENU_GAP, OVERLAY_STATUS_ESTIMATED_HEIGHT, SecondaryAction,
        SecondaryActionFocusDirection, SelectionCursor, SelectionDimensionLayout,
        SmartTargetHudLayout, WorkspaceLayoutInput, WorkspaceSelectionAnchor,
        accepts_overlay_input, action_toolbar_height, action_toolbar_layout,
        action_toolbar_natural_width, action_toolbar_row_count, annotation_controls_visible,
        annotation_layer_entry_label, annotation_layer_label, annotation_opacity_value_label,
        annotation_style_capabilities_for_tool, annotation_style_row_height,
        annotation_style_row_preferred_width, annotation_style_row_width,
        annotation_tool_group_focus_direction, annotation_tool_group_focus_target,
        annotation_tool_group_popover_height, annotation_tool_group_popover_width,
        annotation_tool_palette_width, annotation_toolbar_height, annotation_toolbar_items,
        annotation_toolbar_layout, annotation_toolbar_preferred_width,
        arrange_context_for_selection, arrow_head_points, capture_double_click,
        close_more_actions_shortcut, intersect, is_text_annotation, magnifier_origin,
        more_actions_button_label, more_actions_shortcut, outline_shape_bounds,
        overlay_ui_acceptance_frame, overlay_ui_acceptance_selection, overlay_ui_acceptance_target,
        owns_selection_toolbar, primary_action_tooltip, recognition_result_preview,
        recognition_retry_label, resize_handle_points, secondary_action_focus_direction,
        secondary_action_focus_target, secondary_action_menu_height, secondary_action_menu_left,
        secondary_action_menu_width, secondary_action_tooltip, secondary_menu_opens_above,
        selection_cursor, selection_dimension_label_layout, selection_point_from_view_or_screen,
        should_stop_overlay_action_key_propagation, smart_target_hud_label,
        smart_target_hud_layout, snap_selection_pointer_to_frame_edge, status_bottom_inset,
        status_bottom_inset_for_stacked_annotation, view_rect, visible_selection,
        workspace_layout_snapshot,
    };
    use crate::domain::{
        annotation::{Annotation, AnnotationId, AnnotationKind, AnnotationStyle, AnnotationTool},
        geometry::{PhysicalPoint, PhysicalRect},
        selection::{PreviewTransform, SelectionDrag, ViewPoint, ViewRect},
    };
    use crate::i18n::Locale;
    use crate::platform::capture::PixelFormat;
    use crate::platform::window_inspector::{InspectionKind, InspectionTarget};
    use gpui::{Bounds, Keystroke, Pixels, point, px, size};

    /// Builds the same shared anchor used by production layout code.
    fn workspace_anchor(
        selection: PhysicalRect,
        transform: Option<PreviewTransform>,
    ) -> Option<WorkspaceSelectionAnchor> {
        transform
            .and_then(|transform| WorkspaceSelectionAnchor::from_selection(selection, transform))
    }

    /// Creates a selection-only snapshot input for deterministic boundary-layout tests.
    fn workspace_layout_input(
        selection: PhysicalRect,
        display_bounds: PhysicalRect,
        transform: Option<PreviewTransform>,
        viewport: Bounds<Pixels>,
    ) -> WorkspaceLayoutInput {
        WorkspaceLayoutInput {
            selection: Some(selection),
            display_bounds,
            transform,
            viewport,
            hover_pixel: None,
            inspection_target: None,
            show_annotation_controls: false,
            annotation_toolbar_items: annotation_toolbar_items(false, false, false, false, false),
            annotation_style_capabilities: AnnotationStyleCapabilities::EMPTY,
            annotation_tool_group: None,
            annotation_tool_width: ANNOTATION_TOOL_ESTIMATED_WIDTH,
            has_recognition_result: false,
            has_recognition_retry: false,
            recognition_in_flight: false,
        }
    }

    #[test]
    fn queued_input_only_applies_to_the_overlay_generation_that_created_it() {
        assert!(accepts_overlay_input(42, 42));
        assert!(!accepts_overlay_input(42, 43));
        assert!(!accepts_overlay_input(u64::MAX, 0));
    }

    #[test]
    fn frame_input_batch_applies_only_the_latest_sample_once_per_frame() {
        let mut batch = FrameInputBatch::default();

        assert_eq!(batch.push(10), Some(0));
        assert_eq!(batch.push(20), None);
        assert_eq!(batch.push(30), None);
        assert_eq!(batch.take(0), Some(30));
        assert_eq!(batch.take(0), None);
        assert_eq!(batch.push(40), Some(0));
    }

    #[test]
    fn invalidated_frame_input_cannot_consume_the_next_gesture() {
        let mut batch = FrameInputBatch::default();
        let stale_generation = batch.push("old gesture").unwrap();

        batch.invalidate();
        let current_generation = batch.push("new gesture").unwrap();

        assert_ne!(stale_generation, current_generation);
        assert_eq!(batch.take(stale_generation), None);
        assert_eq!(batch.take(current_generation), Some("new gesture"));
    }

    #[test]
    fn frame_input_batch_bounds_a_burst_to_one_scheduled_callback() {
        let mut batch = FrameInputBatch::default();
        let mut callbacks = 0;

        for sample in 0..1_000 {
            if batch.push(sample).is_some() {
                callbacks += 1;
            }
        }

        assert_eq!(callbacks, 1);
        assert_eq!(batch.take(0), Some(999));
    }

    #[test]
    fn full_display_drag_uses_global_physical_points_and_preserves_letterboxing() {
        let bounds = PhysicalRect {
            left: 0,
            top: 0,
            right: 2560,
            bottom: 1440,
        };
        let transform = PreviewTransform::contain(
            bounds,
            ViewRect {
                left: 0.0,
                top: 0.0,
                width: 2560.0,
                height: 1440.0,
            },
        )
        .unwrap();

        assert_eq!(
            selection_point_from_view_or_screen(
                transform,
                point(px(563.0), px(288.0)),
                true,
                Some(PhysicalPoint { x: 999, y: 999 }),
                bounds,
            ),
            Some(PhysicalPoint { x: 999, y: 999 })
        );
        assert_eq!(
            selection_point_from_view_or_screen(
                transform,
                point(px(-4.0), px(1444.0)),
                true,
                Some(PhysicalPoint { x: -20, y: 1500 }),
                bounds,
            ),
            Some(PhysicalPoint { x: -20, y: 1500 })
        );
        assert_eq!(
            selection_point_from_view_or_screen(
                transform,
                point(px(-4.0), px(1444.0)),
                false,
                Some(PhysicalPoint { x: -20, y: 1500 }),
                bounds,
            ),
            Some(PhysicalPoint { x: 0, y: 1440 })
        );
        assert_eq!(
            selection_point_from_view_or_screen(
                transform,
                point(px(-4.0), px(1444.0)),
                true,
                None,
                bounds,
            ),
            Some(PhysicalPoint { x: 0, y: 1440 })
        );
    }

    #[test]
    fn final_addressable_frame_pixel_maps_to_the_exclusive_selection_edge() {
        let bounds = PhysicalRect {
            left: -1920,
            top: -120,
            right: 0,
            bottom: 960,
        };

        assert_eq!(
            snap_selection_pointer_to_frame_edge(PhysicalPoint { x: -1, y: 959 }, bounds,),
            PhysicalPoint { x: 0, y: 960 }
        );
        assert_eq!(
            snap_selection_pointer_to_frame_edge(PhysicalPoint { x: -2, y: 958 }, bounds,),
            PhysicalPoint { x: -2, y: 958 }
        );
    }

    #[test]
    fn clips_shared_selection_to_each_display() {
        let selection = PhysicalRect {
            left: -200,
            top: 100,
            right: 300,
            bottom: 500,
        };
        let display = PhysicalRect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };

        assert_eq!(
            intersect(selection, display),
            Some(PhysicalRect {
                left: 0,
                top: 100,
                right: 300,
                bottom: 500,
            })
        );
    }

    #[test]
    fn only_the_display_containing_the_selection_end_owns_its_actions() {
        let left_display = PhysicalRect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        let right_display = PhysicalRect {
            left: 1920,
            top: 0,
            right: 3840,
            bottom: 1080,
        };
        let selection = PhysicalRect {
            left: 1600,
            top: 200,
            right: 2200,
            bottom: 600,
        };

        assert!(!owns_selection_toolbar(selection, left_display));
        assert!(owns_selection_toolbar(selection, right_display));
    }

    #[test]
    fn cross_display_marking_controls_have_one_owner() {
        let left_display = PhysicalRect {
            left: -1920,
            top: 0,
            right: 0,
            bottom: 1080,
        };
        let right_display = PhysicalRect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        let selection = PhysicalRect {
            left: -420,
            top: 160,
            right: 320,
            bottom: 680,
        };

        assert!(!annotation_controls_visible(
            true,
            Some(selection),
            left_display
        ));
        assert!(annotation_controls_visible(
            true,
            Some(selection),
            right_display
        ));
        assert!(!annotation_controls_visible(
            false,
            Some(selection),
            right_display
        ));
        assert!(!annotation_controls_visible(true, None, right_display));
    }

    #[test]
    fn secondary_action_tooltips_explain_each_advanced_workflow() {
        for action in [
            "scroll",
            "qr",
            "ocr",
            "translate",
            "record-area",
            "record-window",
        ] {
            assert!(!secondary_action_tooltip(Locale::English, action).is_empty());
            assert!(!secondary_action_tooltip(Locale::SimplifiedChinese, action).is_empty());
        }
        assert_eq!(secondary_action_tooltip(Locale::English, "unknown"), "");
        assert_eq!(
            secondary_action_tooltip(Locale::SimplifiedChinese, "scroll"),
            "滚动并拼接多个视口以截取长页面"
        );
    }

    #[test]
    fn primary_action_tooltips_expose_capture_shortcuts_and_intent() {
        assert!(primary_action_tooltip(Locale::English, "copy").contains("Enter"));
        assert!(primary_action_tooltip(Locale::English, "save").contains("Ctrl+S"));
        assert!(primary_action_tooltip(Locale::English, "cancel").contains("Escape"));
        assert!(primary_action_tooltip(Locale::SimplifiedChinese, "copy").contains("Enter"));
        assert!(!primary_action_tooltip(Locale::English, "draw").is_empty());
        assert_eq!(primary_action_tooltip(Locale::English, "unknown"), "");
    }

    #[test]
    fn more_actions_control_keeps_focus_identity_when_label_changes() {
        assert_eq!(OVERLAY_MORE_ACTIONS_ID, "overlay-more-actions");
        assert_eq!(more_actions_button_label(Locale::English, false), "More");
        assert_eq!(more_actions_button_label(Locale::English, true), "Less");
        assert_eq!(
            more_actions_button_label(Locale::SimplifiedChinese, false),
            "更多"
        );
        assert_eq!(
            more_actions_button_label(Locale::SimplifiedChinese, true),
            "收起"
        );
    }

    #[test]
    fn more_actions_shortcuts_are_explicit_and_do_not_shadow_other_commands() {
        assert!(more_actions_shortcut(&Keystroke::parse("alt-m").unwrap()));
        for key in ["m", "shift-alt-m", "ctrl-alt-m", "alt-r"] {
            assert!(
                !more_actions_shortcut(&Keystroke::parse(key).unwrap()),
                "{key} must not open More"
            );
        }

        assert!(close_more_actions_shortcut(
            &Keystroke::parse("escape").unwrap()
        ));
        for key in ["shift-escape", "ctrl-escape", "enter"] {
            assert!(
                !close_more_actions_shortcut(&Keystroke::parse(key).unwrap()),
                "{key} must not close More"
            );
        }
    }

    #[test]
    fn focused_more_actions_keep_activation_local_and_directional_navigation_explicit() {
        for key in ["enter", "space"] {
            assert!(should_stop_overlay_action_key_propagation(
                &Keystroke::parse(key).unwrap()
            ));
        }
        for key in ["shift-enter", "ctrl-enter", "shift-space", "escape"] {
            assert!(!should_stop_overlay_action_key_propagation(
                &Keystroke::parse(key).unwrap()
            ));
        }

        assert_eq!(
            secondary_action_focus_direction(&Keystroke::parse("down").unwrap()),
            Some(SecondaryActionFocusDirection::Next)
        );
        assert_eq!(
            secondary_action_focus_direction(&Keystroke::parse("left").unwrap()),
            Some(SecondaryActionFocusDirection::Previous)
        );
        for key in ["tab", "shift-tab", "shift-down", "ctrl-right", "enter"] {
            assert_eq!(
                secondary_action_focus_direction(&Keystroke::parse(key).unwrap()),
                None,
                "{key} must not be captured by More navigation"
            );
        }
    }

    #[test]
    fn annotation_tool_groups_materialize_expected_tools_and_local_navigation() {
        assert_eq!(ANNOTATION_TOOL_GROUP_SPECS.len(), 4);
        assert_eq!(
            ANNOTATION_TOOL_GROUP_SPECS[0].tools,
            &[
                AnnotationTool::Text,
                AnnotationTool::Watermark,
                AnnotationTool::Number,
            ]
        );
        assert_eq!(
            ANNOTATION_TOOL_GROUP_SPECS[1].tools,
            &[AnnotationTool::Rectangle, AnnotationTool::Ellipse]
        );
        assert_eq!(
            ANNOTATION_TOOL_GROUP_SPECS[2].tools,
            &[
                AnnotationTool::Line,
                AnnotationTool::Arrow,
                AnnotationTool::Freehand,
            ]
        );
        assert_eq!(
            ANNOTATION_TOOL_GROUP_SPECS[3].tools,
            &[AnnotationTool::Blur, AnnotationTool::Mosaic]
        );
        assert_eq!(
            annotation_tool_group_focus_direction(&Keystroke::parse("right").unwrap()),
            Some(super::AnnotationToolGroupFocusDirection::Next)
        );
        assert_eq!(
            annotation_tool_group_focus_direction(&Keystroke::parse("up").unwrap()),
            Some(super::AnnotationToolGroupFocusDirection::Previous)
        );
        for key in ["tab", "shift-right", "ctrl-down", "enter", "escape"] {
            assert_eq!(
                annotation_tool_group_focus_direction(&Keystroke::parse(key).unwrap()),
                None,
                "{key} must stay outside group arrow navigation"
            );
        }
        assert_eq!(
            annotation_tool_group_focus_target(
                AnnotationTool::Text,
                ANNOTATION_TOOL_GROUP_SPECS[0].tools,
                super::AnnotationToolGroupFocusDirection::Previous,
            ),
            Some(AnnotationTool::Number)
        );
        assert_eq!(
            annotation_tool_group_focus_target(
                AnnotationTool::Number,
                ANNOTATION_TOOL_GROUP_SPECS[0].tools,
                super::AnnotationToolGroupFocusDirection::Next,
            ),
            Some(AnnotationTool::Text)
        );
    }

    #[test]
    fn annotation_tool_group_popover_layout_matches_fixed_rendered_cells() {
        let wide_width = 900.0;
        let tool_width = ANNOTATION_TOOL_ESTIMATED_WIDTH;
        assert_eq!(
            annotation_tool_group_popover_width(wide_width, AnnotationToolGroup::Text, tool_width,),
            3.0 * tool_width
                + 2.0 * ANNOTATION_TOOL_GAP
                + 2.0 * ANNOTATION_TOOL_GROUP_POPUP_PADDING
                + 2.0 * ANNOTATION_TOOL_GROUP_POPUP_BORDER
        );
        assert_eq!(
            annotation_tool_group_popover_height(wide_width, AnnotationToolGroup::Text, tool_width,),
            ANNOTATION_TOOL_ROW_HEIGHT
                + 2.0 * ANNOTATION_TOOL_GROUP_POPUP_PADDING
                + 2.0 * ANNOTATION_TOOL_GROUP_POPUP_BORDER
        );

        let narrow_width = 324.0;
        assert_eq!(
            annotation_tool_group_popover_height(
                narrow_width,
                AnnotationToolGroup::Text,
                tool_width,
            ),
            2.0 * ANNOTATION_TOOL_ROW_HEIGHT
                + ANNOTATION_TOOL_GAP
                + 2.0 * ANNOTATION_TOOL_GROUP_POPUP_PADDING
                + 2.0 * ANNOTATION_TOOL_GROUP_POPUP_BORDER
        );
    }

    #[test]
    fn secondary_actions_have_a_stable_focus_order_when_optional_items_appear() {
        assert_eq!(SecondaryAction::SaveAnnotations.index(), 0);
        assert_eq!(SecondaryAction::ScrollShot.index(), 4);
        assert_eq!(SecondaryAction::RecordWindow.index(), 10);
        assert_eq!(SecondaryAction::RetryRecognition.index(), 11);
        assert_eq!(SecondaryAction::CopyRecognition.index(), 12);
        assert_eq!(SecondaryAction::ClearRecognition.index(), 13);
    }

    #[test]
    fn more_action_arrow_navigation_wraps_only_visible_actions() {
        let visible = [
            SecondaryAction::SaveAnnotations,
            SecondaryAction::ScrollShot,
            SecondaryAction::CopyRecognition,
        ];

        assert_eq!(
            secondary_action_focus_target(
                SecondaryAction::SaveAnnotations,
                &visible,
                SecondaryActionFocusDirection::Previous,
            ),
            Some(SecondaryAction::CopyRecognition)
        );
        assert_eq!(
            secondary_action_focus_target(
                SecondaryAction::ScrollShot,
                &visible,
                SecondaryActionFocusDirection::Next,
            ),
            Some(SecondaryAction::CopyRecognition)
        );
        assert_eq!(
            secondary_action_focus_target(
                SecondaryAction::CopyRecognition,
                &visible,
                SecondaryActionFocusDirection::Next,
            ),
            Some(SecondaryAction::SaveAnnotations)
        );
        assert_eq!(
            secondary_action_focus_target(
                SecondaryAction::Ocr,
                &visible,
                SecondaryActionFocusDirection::Next,
            ),
            None
        );
    }

    #[test]
    fn empty_selection_never_owns_a_toolbar() {
        let display = PhysicalRect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };

        assert!(!owns_selection_toolbar(
            PhysicalRect {
                left: 300,
                top: 200,
                right: 300,
                bottom: 600,
            },
            display
        ));
    }

    #[test]
    fn only_the_second_click_triggers_fast_capture_completion() {
        assert!(!capture_double_click(1));
        assert!(capture_double_click(2));
        assert!(!capture_double_click(3));
    }

    #[test]
    fn magnifier_stays_inside_the_overlay_at_viewport_edges() {
        let viewport = Bounds::new(point(px(0.0), px(0.0)), size(px(300.0), px(200.0)));
        let grid_size = (MAGNIFIER_RADIUS * 2 + 1) as f32 * MAGNIFIER_CELL_SIZE;

        assert_eq!(
            magnifier_origin(ViewPoint { x: 295.0, y: 195.0 }, viewport, grid_size),
            ViewPoint { x: 188.0, y: 88.0 }
        );
        assert_eq!(
            magnifier_origin(ViewPoint { x: 0.0, y: 0.0 }, viewport, grid_size),
            ViewPoint { x: 18.0, y: 18.0 }
        );
    }

    #[test]
    fn shape_bounds_helper_selects_rectangle_and_ellipse_but_not_line_geometry() {
        let rectangle = Annotation {
            id: AnnotationId::new(1),
            kind: AnnotationKind::Rectangle {
                bounds: PhysicalRect {
                    left: 10,
                    top: 20,
                    right: 30,
                    bottom: 40,
                },
            },
            style: AnnotationStyle::default(),
        };
        let ellipse = Annotation {
            id: AnnotationId::new(2),
            kind: AnnotationKind::Ellipse {
                bounds: PhysicalRect {
                    left: 20,
                    top: 30,
                    right: 40,
                    bottom: 50,
                },
            },
            style: AnnotationStyle::default(),
        };
        let line = Annotation {
            id: AnnotationId::new(3),
            kind: AnnotationKind::Line {
                start: PhysicalPoint { x: 10, y: 20 },
                end: PhysicalPoint { x: 30, y: 40 },
            },
            style: AnnotationStyle::default(),
        };

        assert_eq!(
            outline_shape_bounds(&rectangle),
            Some(PhysicalRect {
                left: 10,
                top: 20,
                right: 30,
                bottom: 40,
            })
        );
        assert_eq!(
            outline_shape_bounds(&ellipse),
            Some(PhysicalRect {
                left: 20,
                top: 30,
                right: 40,
                bottom: 50,
            })
        );
        assert_eq!(outline_shape_bounds(&line), None);
    }

    #[test]
    fn annotation_layer_labels_cover_every_drawable_kind() {
        assert_eq!(
            annotation_layer_label(
                Locale::English,
                &AnnotationKind::Text {
                    origin: PhysicalPoint { x: 0, y: 0 },
                    content: "Note".to_owned(),
                }
            ),
            "Text"
        );
        assert_eq!(
            annotation_layer_label(
                Locale::English,
                &AnnotationKind::Freehand {
                    points: vec![PhysicalPoint { x: 0, y: 0 }, PhysicalPoint { x: 1, y: 1 }],
                }
            ),
            "Freehand"
        );
        assert_eq!(
            annotation_layer_label(
                Locale::SimplifiedChinese,
                &AnnotationKind::Freehand {
                    points: vec![PhysicalPoint { x: 0, y: 0 }, PhysicalPoint { x: 1, y: 1 }],
                }
            ),
            "画笔"
        );
    }

    #[test]
    fn annotation_layer_entry_labels_localize_the_position_and_kind_template() {
        let text = AnnotationKind::Text {
            origin: PhysicalPoint { x: 0, y: 0 },
            content: "Note".to_owned(),
        };

        assert_eq!(
            annotation_layer_entry_label(Locale::English, 3, &text),
            "3. Text"
        );
        assert_eq!(
            annotation_layer_entry_label(Locale::SimplifiedChinese, 3, &text),
            "3. 文字"
        );
    }

    #[test]
    fn annotation_opacity_value_uses_the_active_catalog_for_its_dynamic_value() {
        assert_eq!(annotation_opacity_value_label(Locale::English, 128), "50%");
        assert_eq!(
            annotation_opacity_value_label(Locale::SimplifiedChinese, 128),
            "50%"
        );
    }

    #[test]
    fn text_annotation_helper_excludes_non_text_annotations() {
        let text = Annotation {
            id: AnnotationId::new(1),
            kind: AnnotationKind::Text {
                origin: PhysicalPoint { x: 0, y: 0 },
                content: "Note".to_owned(),
            },
            style: AnnotationStyle::default(),
        };
        let line = Annotation {
            id: AnnotationId::new(2),
            kind: AnnotationKind::Line {
                start: PhysicalPoint { x: 0, y: 0 },
                end: PhysicalPoint { x: 1, y: 1 },
            },
            style: AnnotationStyle::default(),
        };

        assert!(is_text_annotation(&text));
        assert!(!is_text_annotation(&line));
    }

    #[test]
    fn arrow_head_uses_two_symmetric_wings_and_skips_zero_length_arrows() {
        let start = PhysicalPoint { x: 10, y: 20 };
        let end = PhysicalPoint { x: 30, y: 20 };
        let (left, right) = arrow_head_points(start, end, 12.0);

        assert_eq!(left, Some(PhysicalPoint { x: 20, y: 14 }));
        assert_eq!(right, Some(PhysicalPoint { x: 20, y: 26 }));
        assert_eq!(arrow_head_points(end, end, 12.0), (None, None));
    }

    #[test]
    fn vertical_arrow_heads_stay_behind_the_endpoint() {
        let start = PhysicalPoint { x: 10, y: 10 };
        let end = PhysicalPoint { x: 10, y: 30 };
        let (left, right) = arrow_head_points(start, end, 12.0);

        assert_eq!(left, Some(PhysicalPoint { x: 16, y: 20 }));
        assert_eq!(right, Some(PhysicalPoint { x: 4, y: 20 }));
    }

    #[test]
    fn reversed_arrow_heads_still_point_at_the_logical_endpoint() {
        let start = PhysicalPoint { x: 30, y: 20 };
        let end = PhysicalPoint { x: 10, y: 20 };
        let (left, right) = arrow_head_points(start, end, 12.0);

        assert_eq!(left, Some(PhysicalPoint { x: 20, y: 26 }));
        assert_eq!(right, Some(PhysicalPoint { x: 20, y: 14 }));
    }

    #[test]
    fn retained_selection_stays_visible_after_the_drag_finishes() {
        let committed = PhysicalRect {
            left: 10,
            top: 20,
            right: 110,
            bottom: 120,
        };
        let mut drag = SelectionDrag::default();
        drag.select(committed);

        assert!(!drag.is_dragging());
        assert_eq!(visible_selection(drag, Some(committed)), Some(committed));
    }

    #[test]
    fn retained_selection_stays_visible_while_an_annotation_is_active() {
        let committed = PhysicalRect {
            left: 10,
            top: 20,
            right: 110,
            bottom: 120,
        };
        let retained = PhysicalRect {
            left: 200,
            top: 300,
            right: 400,
            bottom: 500,
        };
        let mut drag = SelectionDrag::default();
        drag.select(retained);

        assert!(!drag.is_dragging());
        assert_eq!(visible_selection(drag, Some(committed)), Some(retained));
    }

    #[test]
    fn active_drag_overrides_the_previous_committed_selection() {
        let committed = PhysicalRect {
            left: 10,
            top: 20,
            right: 110,
            bottom: 120,
        };
        let current = PhysicalRect {
            left: 200,
            top: 300,
            right: 400,
            bottom: 500,
        };
        let mut drag = SelectionDrag::default();
        drag.begin(PhysicalPoint {
            x: current.left,
            y: current.top,
        });
        drag.update(PhysicalPoint {
            x: current.right,
            y: current.bottom,
        });

        assert!(drag.is_dragging());
        assert_eq!(visible_selection(drag, Some(committed)), Some(current));
    }

    #[test]
    fn committed_selection_uses_move_cursor_inside_and_resize_cursor_at_corners() {
        let image = PhysicalRect {
            left: 0,
            top: 0,
            right: 1000,
            bottom: 500,
        };
        let selection = PhysicalRect {
            left: 200,
            top: 100,
            right: 800,
            bottom: 400,
        };
        let transform = PreviewTransform::contain(
            image,
            crate::domain::selection::ViewRect {
                left: 0.0,
                top: 0.0,
                width: 1000.0,
                height: 500.0,
            },
        )
        .unwrap();

        assert_eq!(
            selection_cursor(
                Some(selection),
                Some(transform),
                ViewPoint { x: 500.0, y: 250.0 },
                false,
                false,
            ),
            SelectionCursor::Move
        );
        assert_eq!(
            selection_cursor(
                Some(selection),
                Some(transform),
                ViewPoint { x: 200.0, y: 100.0 },
                false,
                false,
            ),
            SelectionCursor::ResizeNwse
        );
        assert_eq!(
            selection_cursor(
                Some(selection),
                Some(transform),
                ViewPoint { x: 100.0, y: 50.0 },
                false,
                false,
            ),
            SelectionCursor::Crosshair
        );
        assert_eq!(
            selection_cursor(
                Some(selection),
                Some(transform),
                ViewPoint { x: 500.0, y: 250.0 },
                true,
                false,
            ),
            SelectionCursor::Crosshair
        );
    }

    #[test]
    fn zero_sized_drag_keeps_the_committed_selection_visible() {
        let committed = PhysicalRect {
            left: 10,
            top: 20,
            right: 110,
            bottom: 120,
        };
        let mut drag = SelectionDrag::default();
        drag.begin(PhysicalPoint { x: 300, y: 400 });

        assert!(drag.is_dragging());
        assert_eq!(visible_selection(drag, Some(committed)), Some(committed));
    }

    #[test]
    fn workspace_snapshot_rejects_zero_area_selection_surfaces() {
        let viewport = Bounds::new(point(px(0.0), px(0.0)), size(px(420.0), px(420.0)));
        let bounds = PhysicalRect {
            left: 0,
            top: 0,
            right: 420,
            bottom: 420,
        };
        let transform = PreviewTransform::contain(bounds, super::view_rect(viewport));

        for selection in [
            PhysicalRect {
                left: 210,
                top: 120,
                right: 210,
                bottom: 300,
            },
            PhysicalRect {
                left: 120,
                top: 210,
                right: 300,
                bottom: 210,
            },
        ] {
            let snapshot = workspace_layout_snapshot(workspace_layout_input(
                selection, bounds, transform, viewport,
            ));

            assert_eq!(snapshot.selection_anchor, None);
            assert_eq!(snapshot.action_toolbar, None);
            assert_eq!(snapshot.annotation_toolbar, None);
            assert_eq!(snapshot.annotation_layer, None);
            assert_eq!(snapshot.secondary_menu, None);
            assert_eq!(snapshot.annotation_tool_group, None);
            assert_eq!(snapshot.dimension, None);
            assert_eq!(snapshot.smart_target_hud, None);
        }
    }

    #[test]
    fn workspace_snapshot_keeps_tiny_fullscreen_and_edge_selections_inside_safe_area() {
        let viewport = Bounds::new(point(px(0.0), px(0.0)), size(px(420.0), px(420.0)));
        let bounds = PhysicalRect {
            left: 0,
            top: 0,
            right: 420,
            bottom: 420,
        };
        let transform = PreviewTransform::contain(bounds, super::view_rect(viewport));
        let selections = [
            (
                "one-pixel",
                PhysicalRect {
                    left: 210,
                    top: 210,
                    right: 211,
                    bottom: 211,
                },
            ),
            ("fullscreen", bounds),
            (
                "top",
                PhysicalRect {
                    left: 120,
                    top: 0,
                    right: 300,
                    bottom: 80,
                },
            ),
            (
                "bottom",
                PhysicalRect {
                    left: 120,
                    top: 340,
                    right: 300,
                    bottom: 420,
                },
            ),
            (
                "left",
                PhysicalRect {
                    left: 0,
                    top: 120,
                    right: 80,
                    bottom: 300,
                },
            ),
            (
                "right",
                PhysicalRect {
                    left: 340,
                    top: 120,
                    right: 420,
                    bottom: 300,
                },
            ),
        ];

        for (name, selection) in selections {
            let snapshot = workspace_layout_snapshot(workspace_layout_input(
                selection, bounds, transform, viewport,
            ));
            let anchor = snapshot
                .selection_anchor
                .unwrap_or_else(|| panic!("{name} selection should have one anchor"));
            let toolbar = snapshot
                .action_toolbar
                .unwrap_or_else(|| panic!("{name} selection should have export actions"));
            let dimension = snapshot
                .dimension
                .unwrap_or_else(|| panic!("{name} selection should have a size HUD"));

            assert_eq!(anchor, workspace_anchor(selection, transform).unwrap());
            assert!(toolbar.left >= snapshot.safe_area.left, "{name}");
            assert!(
                toolbar.left + toolbar.width <= snapshot.safe_area.right,
                "{name}"
            );
            assert!(toolbar.top >= snapshot.safe_area.top, "{name}");
            assert!(
                toolbar.top + toolbar.height <= snapshot.safe_area.bottom,
                "{name}"
            );
            assert!(dimension.left >= snapshot.safe_area.left, "{name}");
            assert!(dimension.top >= snapshot.safe_area.top, "{name}");
            assert!(
                dimension.top + super::OVERLAY_DIMENSION_LABEL_HEIGHT <= snapshot.safe_area.bottom,
                "{name}"
            );
            assert!(
                dimension.top + super::OVERLAY_DIMENSION_LABEL_HEIGHT <= toolbar.top
                    || dimension.top >= toolbar.top + toolbar.height,
                "{name} size HUD overlaps the action toolbar"
            );
        }
    }

    #[test]
    fn workspace_snapshot_keeps_edge_surfaces_on_one_selection_anchor() {
        let viewport = Bounds::new(point(px(0.0), px(0.0)), size(px(420.0), px(420.0)));
        let bounds = PhysicalRect {
            left: 0,
            top: 0,
            right: 420,
            bottom: 420,
        };
        let transform = PreviewTransform::contain(bounds, super::view_rect(viewport));
        let selection = overlay_ui_acceptance_selection(
            bounds,
            crate::OverlayUiAcceptanceSelectionPlacement::BottomRight,
        );
        let snapshot = workspace_layout_snapshot(WorkspaceLayoutInput {
            selection: Some(selection),
            display_bounds: bounds,
            transform,
            viewport,
            hover_pixel: None,
            inspection_target: None,
            show_annotation_controls: false,
            annotation_toolbar_items: annotation_toolbar_items(false, false, false, false, false),
            annotation_style_capabilities: AnnotationStyleCapabilities::EMPTY,
            annotation_tool_group: None,
            annotation_tool_width: ANNOTATION_TOOL_ESTIMATED_WIDTH,
            has_recognition_result: false,
            has_recognition_retry: false,
            recognition_in_flight: false,
        });

        assert_eq!(snapshot.safe_area.left, OVERLAY_EDGE_INSET);
        assert_eq!(snapshot.safe_area.top, OVERLAY_EDGE_INSET);
        assert_eq!(snapshot.safe_area.right, 402.0);
        assert_eq!(snapshot.safe_area.bottom, 324.0);
        assert_eq!(
            snapshot.selection_anchor,
            Some(super::WorkspaceSelectionAnchor {
                top_left: ViewPoint { x: 242.0, y: 312.0 },
                bottom_right: ViewPoint { x: 402.0, y: 408.0 },
            })
        );
        assert_eq!(
            snapshot.action_toolbar,
            Some(ActionToolbarLayout {
                left: 142.0,
                top: 250.0,
                width: 260.0,
                height: 50.0,
            })
        );
        let menu = snapshot.secondary_menu.expect("selection should own More");
        assert_eq!(menu.width, 352.0);
        assert_eq!(menu.height, 176.0);
        assert!(menu.opens_above);
        assert_eq!(menu.left, -92.0);
        assert_eq!(
            snapshot.dimension,
            Some(SelectionDimensionLayout {
                left: 242.0,
                top: 216.0,
            })
        );
    }

    #[test]
    fn workspace_snapshot_shares_group_geometry_with_annotation_toolbar() {
        let viewport = Bounds::new(point(px(0.0), px(0.0)), size(px(1280.0), px(720.0)));
        let bounds = PhysicalRect {
            left: 0,
            top: 0,
            right: 1280,
            bottom: 720,
        };
        let transform = PreviewTransform::contain(bounds, super::view_rect(viewport));
        let selection = PhysicalRect {
            left: 300,
            top: 200,
            right: 1000,
            bottom: 400,
        };
        let snapshot = workspace_layout_snapshot(WorkspaceLayoutInput {
            selection: Some(selection),
            display_bounds: bounds,
            transform,
            viewport,
            hover_pixel: None,
            inspection_target: None,
            show_annotation_controls: true,
            annotation_toolbar_items: annotation_toolbar_items(false, false, false, false, false),
            annotation_style_capabilities: AnnotationStyleCapabilities::EMPTY,
            annotation_tool_group: Some(AnnotationToolGroup::Text),
            annotation_tool_width: ANNOTATION_TOOL_ESTIMATED_WIDTH,
            has_recognition_result: false,
            has_recognition_retry: false,
            recognition_in_flight: false,
        });

        let toolbar = snapshot
            .annotation_toolbar
            .expect("marking mode should own a toolbar");
        let group = snapshot
            .annotation_tool_group
            .expect("the selected group should be materialized");
        assert_eq!(toolbar.action_toolbar, snapshot.action_toolbar.unwrap());
        assert_eq!(group.left, toolbar.left);
        assert_eq!(
            group.width,
            annotation_tool_group_popover_width(
                toolbar.tools_width,
                AnnotationToolGroup::Text,
                ANNOTATION_TOOL_ESTIMATED_WIDTH,
            )
        );
        assert_eq!(
            group.height,
            annotation_tool_group_popover_height(
                toolbar.tools_width,
                AnnotationToolGroup::Text,
                ANNOTATION_TOOL_ESTIMATED_WIDTH,
            )
        );
        assert!(group.top >= toolbar.tools_top);
        assert!(group.top + group.height <= toolbar.tools_top + toolbar.tools_height);
    }

    #[test]
    fn compact_action_toolbar_stays_near_the_selection_and_above_the_taskbar_safe_area() {
        let viewport = Bounds::new(point(px(0.0), px(0.0)), size(px(1280.0), px(720.0)));
        let transform = PreviewTransform::contain(
            PhysicalRect {
                left: 0,
                top: 0,
                right: 1280,
                bottom: 720,
            },
            super::view_rect(viewport),
        );
        let selection = PhysicalRect {
            left: 900,
            top: 580,
            right: 1200,
            bottom: 700,
        };

        assert_eq!(
            action_toolbar_layout(workspace_anchor(selection, transform), viewport, false),
            Some(ActionToolbarLayout {
                left: 940.0,
                top: 518.0,
                width: 260.0,
                height: 50.0,
            })
        );
    }

    #[test]
    fn marking_toolbar_groups_selection_actions_under_nearby_tools() {
        let viewport = Bounds::new(point(px(0.0), px(0.0)), size(px(1280.0), px(720.0)));
        let transform = PreviewTransform::contain(
            PhysicalRect {
                left: 0,
                top: 0,
                right: 1280,
                bottom: 720,
            },
            super::view_rect(viewport),
        );
        let selection = PhysicalRect {
            left: 300,
            top: 200,
            right: 1000,
            bottom: 400,
        };
        let primary_actions = ActionToolbarLayout {
            left: 722.0,
            top: 412.0,
            width: 558.0,
            height: 50.0,
        };

        let layout = annotation_toolbar_layout(
            workspace_anchor(selection, transform),
            viewport,
            Some(primary_actions),
            annotation_toolbar_items(false, false, false, false, false),
            0.0,
            0.0,
            None,
        )
        .expect("selection with actions should position marking tools");

        assert_eq!(layout.left, 442.0);
        assert_eq!(layout.width, 558.0);
        assert_eq!(layout.tools_width, 558.0);
        assert_eq!(layout.tools_height, 50.0);
        assert_eq!(layout.height, 50.0);
        assert!((layout.top - 412.0).abs() < 0.01);
        assert_eq!(layout.tools_top, layout.top);
        assert_eq!(layout.style_top, layout.tools_top + layout.tools_height);
        assert_eq!(layout.style_left, layout.left);
        assert_eq!(layout.action_toolbar.left, 442.0);
        assert!((layout.action_toolbar.top - 412.0).abs() < 0.01);
        assert_eq!(layout.action_toolbar.width, primary_actions.width);
        assert_eq!(layout.action_toolbar.height, primary_actions.height);
        assert!(!layout.actions_above_tools);
        assert!((layout.top - (selection.bottom as f32 + OVERLAY_ACTION_BAR_GAP)).abs() < 0.01);
        assert!(layout.top + layout.height <= 720.0 - OVERLAY_BOTTOM_SAFE_INSET);
    }

    #[test]
    fn marking_toolbar_flips_above_a_bottom_edge_selection() {
        let viewport = Bounds::new(point(px(0.0), px(0.0)), size(px(1280.0), px(720.0)));
        let transform = PreviewTransform::contain(
            PhysicalRect {
                left: 0,
                top: 0,
                right: 1280,
                bottom: 720,
            },
            super::view_rect(viewport),
        );
        let selection = PhysicalRect {
            left: 900,
            top: 580,
            right: 1200,
            bottom: 700,
        };
        let primary_actions = ActionToolbarLayout {
            left: 642.0,
            top: 518.0,
            width: 558.0,
            height: 50.0,
        };

        let layout = annotation_toolbar_layout(
            workspace_anchor(selection, transform),
            viewport,
            Some(primary_actions),
            annotation_toolbar_items(false, false, false, false, false),
            0.0,
            0.0,
            None,
        )
        .expect("selection with actions should position marking tools");

        assert_eq!(layout.top, 518.0);
        assert_eq!(layout.tools_top, 518.0);
        assert_eq!(layout.style_top, layout.tools_top + layout.tools_height);
        assert_eq!(layout.action_toolbar.top, layout.top);
        assert!(layout.actions_above_tools);
        assert!(layout.top >= 18.0);
        assert!(
            layout.tools_top + layout.tools_height + OVERLAY_ACTION_BAR_GAP >= selection.top as f32
        );
    }

    #[test]
    fn selection_dimensions_stay_visible_and_avoid_a_toolbar_below_the_selection() {
        let viewport = Bounds::new(point(px(0.0), px(0.0)), size(px(1280.0), px(720.0)));
        let transform = PreviewTransform::contain(
            PhysicalRect {
                left: 0,
                top: 0,
                right: 1280,
                bottom: 720,
            },
            super::view_rect(viewport),
        );
        let selection = PhysicalRect {
            left: 100,
            top: 300,
            right: 600,
            bottom: 500,
        };
        let toolbar = ActionToolbarLayout {
            left: 18.0,
            top: 508.0,
            width: 260.0,
            height: 42.0,
        };

        assert_eq!(
            selection_dimension_label_layout(
                workspace_anchor(selection, transform),
                viewport,
                Some(toolbar),
            ),
            Some(SelectionDimensionLayout {
                left: 100.0,
                top: 266.0,
            })
        );
    }

    #[test]
    fn bottom_right_acceptance_selection_lifts_actions_and_keeps_dimensions_clear() {
        let viewport = Bounds::new(point(px(0.0), px(0.0)), size(px(420.0), px(420.0)));
        let bounds = PhysicalRect {
            left: 0,
            top: 0,
            right: 420,
            bottom: 420,
        };
        let transform = PreviewTransform::contain(bounds, super::view_rect(viewport));
        let selection = overlay_ui_acceptance_selection(
            bounds,
            crate::OverlayUiAcceptanceSelectionPlacement::BottomRight,
        );
        assert_eq!(
            selection,
            PhysicalRect {
                left: 242,
                top: 312,
                right: 402,
                bottom: 408,
            }
        );

        let toolbar =
            action_toolbar_layout(workspace_anchor(selection, transform), viewport, false).unwrap();
        assert_eq!(
            toolbar,
            ActionToolbarLayout {
                left: 142.0,
                top: 250.0,
                width: 260.0,
                height: 50.0,
            }
        );
        assert!(toolbar.top + toolbar.height + OVERLAY_ACTION_BAR_GAP <= selection.top as f32);

        let menu_width =
            secondary_action_menu_width(super::view_rect(viewport).width, false, false);
        assert_eq!(menu_width, 352.0);
        let menu_height = secondary_action_menu_height(menu_width, false, false, false);
        assert_eq!(menu_height, 176.0);
        assert!(secondary_menu_opens_above(toolbar, viewport, menu_height));
        let menu_top = toolbar.top
            - (OVERLAY_ACTION_ITEM_HEIGHT
                + OVERLAY_ACTION_BAR_PADDING * 2.0
                + OVERLAY_SECONDARY_MENU_GAP)
            - menu_height;
        assert!(menu_top >= OVERLAY_EDGE_INSET);

        assert_eq!(
            selection_dimension_label_layout(
                workspace_anchor(selection, transform),
                viewport,
                Some(toolbar),
            ),
            Some(SelectionDimensionLayout {
                left: 242.0,
                top: 216.0,
            })
        );
    }

    #[test]
    fn real_display_narrow_edge_marking_dock_stays_above_the_selection() {
        let viewport = Bounds::new(point(px(0.0), px(0.0)), size(px(2560.0), px(1440.0)));
        let bounds = PhysicalRect {
            left: 0,
            top: 0,
            right: 2560,
            bottom: 1440,
        };
        let transform = PreviewTransform::contain(bounds, super::view_rect(viewport));
        let selection = overlay_ui_acceptance_selection(
            bounds,
            crate::OverlayUiAcceptanceSelectionPlacement::BottomRight,
        );
        assert_eq!(
            selection,
            PhysicalRect {
                left: 2382,
                top: 1332,
                right: 2542,
                bottom: 1428,
            }
        );

        let primary =
            action_toolbar_layout(workspace_anchor(selection, transform), viewport, true).unwrap();
        assert_eq!(
            primary,
            ActionToolbarLayout {
                left: 1983.0,
                top: 1270.0,
                width: 559.0,
                height: 50.0,
            }
        );
        let marking = annotation_toolbar_layout(
            workspace_anchor(selection, transform),
            viewport,
            Some(primary),
            annotation_toolbar_items(false, false, false, false, false),
            0.0,
            0.0,
            None,
        )
        .unwrap();
        assert_eq!(marking.left, 1983.0);
        assert_eq!(marking.top, 1270.0);
        assert_eq!(marking.width, 559.0);
        assert_eq!(marking.height, 50.0);
        assert_eq!(marking.tools_width, 559.0);
        assert_eq!(marking.tools_top, 1270.0);
        assert_eq!(marking.style_left, marking.left);
        assert_eq!(marking.style_top, 1320.0);
        assert_eq!(marking.action_toolbar.left, primary.left);
        assert_eq!(marking.action_toolbar.top, 1270.0);
        assert!(marking.actions_above_tools);
        assert!(marking.top >= OVERLAY_EDGE_INSET);
        assert!(marking.top + marking.height + OVERLAY_ACTION_BAR_GAP <= selection.top as f32);
    }

    #[test]
    fn smart_target_hud_belongs_to_the_display_under_the_pointer() {
        let left_display = PhysicalRect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        let right_display = PhysicalRect {
            left: 1920,
            top: 0,
            right: 3840,
            bottom: 1080,
        };
        let target = InspectionTarget {
            bounds: PhysicalRect {
                left: 1800,
                top: 100,
                right: 2200,
                bottom: 500,
            },
            kind: InspectionKind::Window,
        };
        let viewport = Bounds::new(point(px(0.0), px(0.0)), size(px(1920.0), px(1080.0)));
        let left_transform = PreviewTransform::contain(left_display, super::view_rect(viewport));
        let right_transform = PreviewTransform::contain(right_display, super::view_rect(viewport));

        assert_eq!(
            smart_target_hud_layout(
                None,
                Some(PhysicalPoint { x: 1850, y: 200 }),
                Some(target),
                left_display,
                left_transform,
                viewport,
            ),
            Some(SmartTargetHudLayout {
                target,
                left: 1678.0,
                top: 66.0,
                width: 224.0,
            })
        );
        assert_eq!(
            smart_target_hud_layout(
                None,
                Some(PhysicalPoint { x: 1850, y: 200 }),
                Some(target),
                right_display,
                right_transform,
                viewport,
            ),
            None
        );
        assert_eq!(
            smart_target_hud_layout(
                None,
                Some(PhysicalPoint { x: 2000, y: 200 }),
                Some(target),
                right_display,
                right_transform,
                viewport,
            ),
            Some(SmartTargetHudLayout {
                target,
                left: 18.0,
                top: 66.0,
                width: 224.0,
            })
        );
    }

    #[test]
    fn smart_target_hud_flips_and_clamps_at_viewport_edges() {
        let display = PhysicalRect {
            left: 0,
            top: 0,
            right: 1280,
            bottom: 720,
        };
        let viewport = Bounds::new(point(px(0.0), px(0.0)), size(px(1280.0), px(720.0)));
        let transform = PreviewTransform::contain(display, super::view_rect(viewport));
        let top_target = InspectionTarget {
            bounds: PhysicalRect {
                left: 1100,
                top: 10,
                right: 1250,
                bottom: 64,
            },
            kind: InspectionKind::Control,
        };
        let bottom_target = InspectionTarget {
            bounds: PhysicalRect {
                left: 1100,
                top: 660,
                right: 1250,
                bottom: 700,
            },
            kind: InspectionKind::Control,
        };

        assert_eq!(
            smart_target_hud_layout(
                None,
                Some(PhysicalPoint { x: 1150, y: 20 }),
                Some(top_target),
                display,
                transform,
                viewport,
            ),
            Some(SmartTargetHudLayout {
                target: top_target,
                left: 1038.0,
                top: 72.0,
                width: 224.0,
            })
        );
        assert_eq!(
            smart_target_hud_layout(
                None,
                Some(PhysicalPoint { x: 1150, y: 680 }),
                Some(bottom_target),
                display,
                transform,
                viewport,
            ),
            Some(SmartTargetHudLayout {
                target: bottom_target,
                left: 1038.0,
                top: 598.0,
                width: 224.0,
            })
        );
    }

    #[test]
    fn smart_target_hud_hides_after_selection_or_stale_hover() {
        let display = PhysicalRect {
            left: 0,
            top: 0,
            right: 1280,
            bottom: 720,
        };
        let viewport = Bounds::new(point(px(0.0), px(0.0)), size(px(1280.0), px(720.0)));
        let transform = PreviewTransform::contain(display, super::view_rect(viewport));
        let target = InspectionTarget {
            bounds: PhysicalRect {
                left: 100,
                top: 200,
                right: 600,
                bottom: 500,
            },
            kind: InspectionKind::Control,
        };

        assert_eq!(
            smart_target_hud_layout(
                Some(PhysicalRect {
                    left: 110,
                    top: 210,
                    right: 200,
                    bottom: 300,
                }),
                Some(PhysicalPoint { x: 150, y: 250 }),
                Some(target),
                display,
                transform,
                viewport,
            ),
            None
        );
        assert_eq!(
            smart_target_hud_layout(None, None, Some(target), display, transform, viewport),
            None
        );
        assert_eq!(
            smart_target_hud_layout(
                None,
                Some(PhysicalPoint { x: 900, y: 250 }),
                Some(target),
                display,
                transform,
                viewport,
            ),
            None
        );
    }

    #[test]
    fn smart_target_hud_labels_the_full_detected_bounds() {
        assert_eq!(
            smart_target_hud_label(
                Locale::English,
                InspectionTarget {
                    bounds: PhysicalRect {
                        left: 1800,
                        top: 100,
                        right: 2200,
                        bottom: 500,
                    },
                    kind: InspectionKind::Window,
                }
            ),
            "Window | 400 x 400 px"
        );
        assert_eq!(
            smart_target_hud_label(
                Locale::SimplifiedChinese,
                InspectionTarget {
                    bounds: PhysicalRect {
                        left: 1800,
                        top: 100,
                        right: 2200,
                        bottom: 500,
                    },
                    kind: InspectionKind::Window,
                },
            ),
            "窗口 | 400 x 400 像素"
        );
        assert_eq!(
            smart_target_hud_label(
                Locale::English,
                InspectionTarget {
                    bounds: PhysicalRect {
                        left: 20,
                        top: 30,
                        right: 500,
                        bottom: 150,
                    },
                    kind: InspectionKind::Control,
                }
            ),
            "Control | 480 x 120 px"
        );
    }

    #[test]
    fn overlay_ui_acceptance_fixture_preserves_frame_and_seeded_geometry() {
        let bounds = PhysicalRect {
            left: 0,
            top: 0,
            right: 1280,
            bottom: 720,
        };
        let frame = overlay_ui_acceptance_frame(bounds).expect("fixture frame should be valid");
        assert_eq!(frame.bounds, bounds);
        assert_eq!(frame.width, 1280);
        assert_eq!(frame.height, 720);
        assert_eq!(frame.stride, 5120);
        assert_eq!(frame.format, PixelFormat::Bgra8);
        assert!(frame.validate().is_ok());

        let target = overlay_ui_acceptance_target(bounds, InspectionKind::Control);
        assert_eq!(target.kind, InspectionKind::Control);
        assert!(bounds.contains(PhysicalPoint {
            x: target.bounds.left,
            y: target.bounds.top,
        }));
        assert!(bounds.contains(PhysicalPoint {
            x: target.bounds.right - 1,
            y: target.bounds.bottom - 1,
        }));

        let selection = overlay_ui_acceptance_selection(
            bounds,
            crate::OverlayUiAcceptanceSelectionPlacement::Centered,
        );
        assert!(selection.width() > 0 && selection.height() > 0);
        assert!(bounds.contains(PhysicalPoint {
            x: selection.left,
            y: selection.top,
        }));
        assert!(bounds.contains(PhysicalPoint {
            x: selection.right - 1,
            y: selection.bottom - 1,
        }));

        let bottom_right = overlay_ui_acceptance_selection(
            bounds,
            crate::OverlayUiAcceptanceSelectionPlacement::BottomRight,
        );
        assert_eq!(bottom_right.width(), 160);
        assert_eq!(bottom_right.height(), 96);
        assert_eq!(
            bottom_right.right,
            bounds.right - OVERLAY_EDGE_INSET.round() as i32
        );
        assert_eq!(
            bottom_right.bottom,
            bounds.bottom - OVERLAY_ACTION_BAR_GAP.round() as i32
        );
    }

    #[test]
    fn secondary_action_menu_keeps_the_main_toolbar_stable_on_small_overlays() {
        let viewport = Bounds::new(point(px(0.0), px(0.0)), size(px(360.0), px(720.0)));
        let transform = PreviewTransform::contain(
            PhysicalRect {
                left: 0,
                top: 0,
                right: 360,
                bottom: 720,
            },
            super::view_rect(viewport),
        );
        let selection = PhysicalRect {
            left: 20,
            top: 400,
            right: 340,
            bottom: 600,
        };

        assert_eq!(action_toolbar_height(324.0, false), 50.0);
        assert_eq!(action_toolbar_height(288.0, false), 50.0);
        assert_eq!(action_toolbar_natural_width(false), 260.0);
        assert_eq!(action_toolbar_natural_width(true), 559.0);
        assert_eq!(action_toolbar_height(358.0, true), 92.0);
        assert_eq!(action_toolbar_height(559.0, true), 50.0);
        assert_eq!(secondary_action_menu_width(420.0, false, false), 352.0);
        assert_eq!(
            action_toolbar_row_count(352.0, OVERLAY_MORE_ACTION_WIDTHS),
            4
        );
        assert_eq!(secondary_action_menu_width(360.0, false, false), 324.0);
        let layout =
            action_toolbar_layout(workspace_anchor(selection, transform), viewport, false).unwrap();
        assert_eq!(
            secondary_action_menu_left(
                layout,
                secondary_action_menu_width(360.0, false, false),
                viewport,
            ),
            -62.0
        );
        assert_eq!(layout.left, 80.0);
        assert!((layout.top - 338.0).abs() < 0.01);
        assert_eq!(layout.width, 260.0);
        assert_eq!(layout.height, 50.0);
        assert_eq!(
            secondary_action_menu_height(324.0, false, false, false),
            218.0
        );
        assert_eq!(
            secondary_action_menu_height(324.0, true, false, false),
            288.0
        );
        assert!(secondary_action_menu_height(324.0, false, true, false) >= 196.0);
        assert_eq!(
            secondary_action_menu_height(324.0, false, false, true),
            254.0
        );
        assert!(secondary_menu_opens_above(
            layout,
            viewport,
            secondary_action_menu_height(layout.width, false, false, false)
        ));
    }

    #[test]
    fn annotation_toolbar_reserves_wrapped_rows_on_narrow_overlays() {
        let wide = Bounds::new(point(px(0.0), px(0.0)), size(px(1920.0), px(1080.0)));
        let narrow = Bounds::new(point(px(0.0), px(0.0)), size(px(360.0), px(720.0)));
        let stable_items = annotation_toolbar_items(false, false, false, false, false);
        let selected_items = annotation_toolbar_items(true, true, true, true, false);
        let expanded_items = annotation_toolbar_items(true, true, true, true, true);

        assert_eq!(stable_items.selection_context, 0);
        assert_eq!(stable_items.arrange_context, 0);
        assert_eq!(selected_items.selection_context, 8);
        assert_eq!(selected_items.arrange_context, 0);
        assert_eq!(expanded_items.arrange_context, 6);
        assert_eq!(annotation_toolbar_height(wide, stable_items), 46.0);
        assert_eq!(annotation_toolbar_height(narrow, stable_items), 46.0);
        assert_eq!(annotation_toolbar_height(narrow, selected_items), 219.0);
        assert!(annotation_toolbar_height(narrow, expanded_items) > 219.0);
    }

    #[test]
    fn annotation_dock_grows_only_for_visible_controls() {
        let empty_style = AnnotationStyleCapabilities::EMPTY;
        let shape_style = annotation_style_capabilities_for_tool(AnnotationTool::Rectangle);
        let empty_items = annotation_toolbar_items(false, false, false, false, false);

        assert_eq!(annotation_tool_palette_width(), 246.0);
        assert_eq!(annotation_style_row_width(empty_style), 0.0);
        assert_eq!(
            annotation_toolbar_preferred_width(
                1280.0,
                358.0,
                empty_items,
                0.0,
                ANNOTATION_TOOL_ESTIMATED_WIDTH,
            ),
            358.0
        );
        assert!(annotation_style_row_width(shape_style) > 550.0);
        let wrapped_style_width = annotation_style_row_preferred_width(484.0, shape_style);
        assert!((350.0..420.0).contains(&wrapped_style_width));
        assert_eq!(
            annotation_style_row_height(wrapped_style_width, shape_style),
            annotation_style_row_height(484.0, shape_style)
        );
        assert_eq!(
            annotation_toolbar_preferred_width(
                520.0,
                358.0,
                empty_items,
                wrapped_style_width,
                ANNOTATION_TOOL_ESTIMATED_WIDTH,
            ),
            387.0
        );
    }

    #[test]
    fn annotation_style_capabilities_only_expose_renderer_backed_values() {
        assert!(ANNOTATION_WIDTHS.contains(&4));
        let text = annotation_style_capabilities_for_tool(AnnotationTool::Text);
        assert_eq!(
            text,
            AnnotationStyleCapabilities {
                color: true,
                opacity: true,
                font_size: true,
                ..AnnotationStyleCapabilities::EMPTY
            }
        );
        assert!(annotation_style_capabilities_for_tool(AnnotationTool::Rectangle).fill);
        assert!(annotation_style_capabilities_for_tool(AnnotationTool::Line).width);
        assert!(annotation_style_capabilities_for_tool(AnnotationTool::Number).color);
        assert_eq!(
            annotation_style_capabilities_for_tool(AnnotationTool::Blur),
            AnnotationStyleCapabilities::EMPTY
        );
        assert_eq!(
            annotation_style_row_height(
                360.0,
                annotation_style_capabilities_for_tool(AnnotationTool::Rectangle)
            ),
            114.0
        );
        assert_eq!(
            annotation_style_row_height(360.0, AnnotationStyleCapabilities::EMPTY),
            0.0
        );
    }

    #[test]
    fn arrange_context_closes_when_annotation_selection_changes() {
        let first = AnnotationId::new(1);
        let second = AnnotationId::new(2);

        assert_eq!(
            arrange_context_for_selection(Some(first), Some(first)),
            Some(first)
        );
        assert_eq!(
            arrange_context_for_selection(Some(first), Some(second)),
            None
        );
        assert_eq!(arrange_context_for_selection(Some(first), None), None);
        assert_eq!(arrange_context_for_selection(None, Some(first)), None);
    }

    #[test]
    fn status_feedback_moves_above_the_fallback_action_bar() {
        assert_eq!(
            status_bottom_inset(true),
            OVERLAY_BOTTOM_SAFE_INSET + OVERLAY_ACTION_ITEM_HEIGHT + OVERLAY_ACTION_BAR_GAP
        );
        assert_eq!(status_bottom_inset(false), OVERLAY_BOTTOM_SAFE_INSET);
    }

    #[test]
    fn stacked_status_feedback_stays_above_the_dimension_label() {
        let viewport = Bounds::new(point(px(0.0), px(0.0)), size(px(520.0), px(640.0)));
        let annotation = AnnotationToolbarLayout {
            left: 18.0,
            top: 300.0,
            width: 484.0,
            height: 300.0,
            tools_width: 484.0,
            tools_height: 240.0,
            tools_top: 300.0,
            style_left: 18.0,
            style_top: 548.0,
            style_height: 40.0,
            action_toolbar: ActionToolbarLayout {
                left: 242.0,
                top: 682.0,
                width: 260.0,
                height: 50.0,
            },
            actions_above_tools: false,
        };
        let dimension = Some(SelectionDimensionLayout {
            left: 18.0,
            top: 540.0,
        });

        let bottom_inset =
            status_bottom_inset_for_stacked_annotation(annotation, dimension, viewport);
        let status_top =
            view_rect(viewport).bottom() - bottom_inset - OVERLAY_STATUS_ESTIMATED_HEIGHT;

        assert_eq!(status_top, 486.0);
        assert_eq!(status_top + OVERLAY_STATUS_ESTIMATED_HEIGHT, 528.0);
        assert!(status_top + OVERLAY_STATUS_ESTIMATED_HEIGHT + OVERLAY_ACTION_BAR_GAP <= 540.0);
    }

    #[test]
    fn recognition_preview_keeps_short_content_and_bounds_long_results() {
        assert_eq!(recognition_result_preview("short result"), "short result");

        let long = "x".repeat(OVERLAY_RECOGNITION_PREVIEW_LIMIT + 1);
        let preview = recognition_result_preview(&long);
        assert_eq!(
            preview.chars().count(),
            OVERLAY_RECOGNITION_PREVIEW_LIMIT + 3
        );
        assert!(preview.ends_with("..."));
    }

    #[test]
    fn recognition_retry_labels_name_the_failed_action() {
        assert_eq!(
            recognition_retry_label(Locale::English, super::super::RecognitionRetry::Ocr),
            "Retry OCR"
        );
        assert_eq!(
            recognition_retry_label(Locale::English, super::super::RecognitionRetry::Translation),
            "Retry translation"
        );
        assert_eq!(
            recognition_retry_label(
                Locale::SimplifiedChinese,
                super::super::RecognitionRetry::Translation
            ),
            "重试翻译"
        );
    }

    #[test]
    fn selection_resize_handles_cover_all_four_corners() {
        let selection = PhysicalRect {
            left: -400,
            top: 50,
            right: 800,
            bottom: 600,
        };

        assert_eq!(
            resize_handle_points(selection),
            [
                PhysicalPoint { x: -400, y: 50 },
                PhysicalPoint { x: 800, y: 50 },
                PhysicalPoint { x: -400, y: 600 },
                PhysicalPoint { x: 800, y: 600 },
            ]
        );
    }
}
