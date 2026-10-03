//! Shared visual primitives for the screenshot workspace.

use gpui::prelude::*;
use gpui::{
    App, Bounds, ClickEvent, Context, PathBuilder, Pixels, Render, SharedString, Stateful, Window,
    canvas, div, point, px,
};
use gpui::{Div, Hsla, Role};

use crate::{
    domain::annotation::AnnotationTool,
    i18n::UiText,
    theme::{ThemeColors, ThemeMetrics},
};

/// Names the visual emphasis used by a workspace control.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkspaceButtonTone {
    Neutral,
    Primary,
    Destructive,
}

/// Collects geometry, semantic color, state, and tooltip data shared by workspace buttons.
pub(crate) struct WorkspaceButtonConfig {
    pub(crate) width: Option<f32>,
    pub(crate) height: f32,
    pub(crate) colors: ThemeColors,
    pub(crate) tone: WorkspaceButtonTone,
    pub(crate) active: bool,
    pub(crate) enabled: bool,
    pub(crate) tooltip: Option<SharedString>,
    pub(crate) icon_size: Option<f32>,
}

/// Identifies the small line icon rendered inside a screenshot-workspace button.
///
/// Keeping the icon geometry in one enum gives every control the same 16px canvas, stroke width,
/// and baseline. The enum is deliberately local to the workspace so it does not become a general
/// application icon dependency or change the accessible labels that already describe each action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkspaceIcon {
    DragHandle,
    Move,
    Text,
    Shape,
    Line,
    Highlight,
    Obscure,
    Undo,
    Redo,
    Pin,
    Copy,
    Save,
    More,
    Cancel,
    Scroll,
    Qr,
    Ocr,
    Translate,
    Record,
    Arrow,
    Ellipse,
    Freehand,
    Mosaic,
    Number,
    Watermark,
}

/// Names the annotation tool groups that can open a workspace popover.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AnnotationToolGroup {
    Text,
    Shape,
    Line,
    Obscure,
}

impl AnnotationToolGroup {
    /// Returns the stable focus slot used by one overlay.
    pub(crate) const fn index(self) -> usize {
        match self {
            Self::Text => 0,
            Self::Shape => 1,
            Self::Line => 2,
            Self::Obscure => 3,
        }
    }

    /// Returns the stable semantic key used by the popover element ID.
    pub(crate) const fn id(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Shape => "shape",
            Self::Line => "line",
            Self::Obscure => "obscure",
        }
    }

    /// Lists groups in the order used by overlay focus storage.
    pub(crate) const fn catalog() -> &'static [Self; 4] {
        &[Self::Text, Self::Shape, Self::Line, Self::Obscure]
    }
}

/// Describes one directly visible annotation action and its related-tool membership.
///
/// This catalog owns the drawing row order, stable element key, icon, accessible label key, and
/// group-local child order. The overlay renders it while the native acceptance runner uses its
/// index to target the same production hitbox.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WorkspaceAnnotationToolSpec {
    tool: AnnotationTool,
    index: usize,
    id: &'static str,
    #[cfg(feature = "dev-tools")]
    acceptance_step: &'static str,
    icon: WorkspaceIcon,
    label: UiText,
    group: Option<AnnotationToolGroup>,
    group_order: Option<u8>,
}

impl WorkspaceAnnotationToolSpec {
    /// Returns the domain tool selected by this workspace action.
    pub(crate) const fn tool(self) -> AnnotationTool {
        self.tool
    }

    /// Returns the stable action key used by the production element ID and acceptance report.
    pub(crate) const fn id(self) -> &'static str {
        self.id
    }

    /// Returns the stable native-runner step name for this action.
    #[cfg(feature = "dev-tools")]
    pub(crate) const fn acceptance_step(self) -> &'static str {
        self.acceptance_step
    }

    /// Returns the icon rendered for this action.
    pub(crate) const fn icon(self) -> WorkspaceIcon {
        self.icon
    }

    /// Returns the localized label key used by the button and its related popover.
    pub(crate) const fn label(self) -> UiText {
        self.label
    }

    /// Returns the related-tool group, if repeat activation can open a popover.
    pub(crate) const fn group(self) -> Option<AnnotationToolGroup> {
        self.group
    }

    /// Returns the index of this direct action in the annotation toolbar.
    #[cfg(any(test, feature = "dev-tools"))]
    pub(crate) const fn index(self) -> usize {
        self.index
    }

    /// Returns the stable direct-action order used by the screenshot workspace.
    pub(crate) const fn catalog() -> &'static [Self; 11] {
        &WORKSPACE_ANNOTATION_TOOLS
    }

    /// Looks up the complete visual descriptor for a domain annotation tool.
    pub(crate) fn for_tool(tool: AnnotationTool) -> Self {
        Self::catalog()
            .iter()
            .copied()
            .find(|spec| spec.tool == tool)
            .expect("every domain annotation tool must have a workspace catalog entry")
    }

    /// Returns a group's child tools in their stable local keyboard and popover order.
    pub(crate) fn group_tools(
        group: AnnotationToolGroup,
    ) -> [Option<AnnotationTool>; MAX_GROUP_TOOL_COUNT] {
        let mut tools = [None; MAX_GROUP_TOOL_COUNT];
        for spec in Self::catalog() {
            if spec.group == Some(group)
                && let Some(index) = spec.group_order
            {
                tools[usize::from(index)] = Some(spec.tool);
            }
        }
        tools
    }

    /// Returns the number of children that are actually present in a group.
    pub(crate) fn group_tool_count(group: AnnotationToolGroup) -> usize {
        Self::group_tools(group).iter().flatten().count()
    }
}

pub(crate) const MAX_GROUP_TOOL_COUNT: usize = 3;
const WORKSPACE_ANNOTATION_TOOLS: [WorkspaceAnnotationToolSpec; 11] = [
    WorkspaceAnnotationToolSpec {
        tool: AnnotationTool::Rectangle,
        index: 0,
        id: "rectangle",
        #[cfg(feature = "dev-tools")]
        acceptance_step: "annotation_toolbar_rectangle_selected",
        icon: WorkspaceIcon::Shape,
        label: UiText::OverlayRectangle,
        group: Some(AnnotationToolGroup::Shape),
        group_order: Some(0),
    },
    WorkspaceAnnotationToolSpec {
        tool: AnnotationTool::Ellipse,
        index: 1,
        id: "ellipse",
        #[cfg(feature = "dev-tools")]
        acceptance_step: "annotation_toolbar_ellipse_selected",
        icon: WorkspaceIcon::Ellipse,
        label: UiText::OverlayEllipse,
        group: Some(AnnotationToolGroup::Shape),
        group_order: Some(1),
    },
    WorkspaceAnnotationToolSpec {
        tool: AnnotationTool::Arrow,
        index: 2,
        id: "arrow",
        #[cfg(feature = "dev-tools")]
        acceptance_step: "annotation_toolbar_arrow_selected",
        icon: WorkspaceIcon::Arrow,
        label: UiText::OverlayArrow,
        group: Some(AnnotationToolGroup::Line),
        group_order: Some(1),
    },
    WorkspaceAnnotationToolSpec {
        tool: AnnotationTool::Line,
        index: 3,
        id: "line",
        #[cfg(feature = "dev-tools")]
        acceptance_step: "annotation_toolbar_line_selected",
        icon: WorkspaceIcon::Line,
        label: UiText::OverlayLine,
        group: Some(AnnotationToolGroup::Line),
        group_order: Some(0),
    },
    WorkspaceAnnotationToolSpec {
        tool: AnnotationTool::Freehand,
        index: 4,
        id: "freehand",
        #[cfg(feature = "dev-tools")]
        acceptance_step: "annotation_toolbar_freehand_selected",
        icon: WorkspaceIcon::Freehand,
        label: UiText::OverlayFreehand,
        group: Some(AnnotationToolGroup::Line),
        group_order: Some(2),
    },
    WorkspaceAnnotationToolSpec {
        tool: AnnotationTool::Highlight,
        index: 5,
        id: "highlight",
        #[cfg(feature = "dev-tools")]
        acceptance_step: "annotation_toolbar_highlight_selected",
        icon: WorkspaceIcon::Highlight,
        label: UiText::OverlayHighlight,
        group: None,
        group_order: None,
    },
    WorkspaceAnnotationToolSpec {
        tool: AnnotationTool::Text,
        index: 6,
        id: "text",
        #[cfg(feature = "dev-tools")]
        acceptance_step: "annotation_toolbar_text_selected",
        icon: WorkspaceIcon::Text,
        label: UiText::OverlayText,
        group: Some(AnnotationToolGroup::Text),
        group_order: Some(0),
    },
    WorkspaceAnnotationToolSpec {
        tool: AnnotationTool::Number,
        index: 7,
        id: "number",
        #[cfg(feature = "dev-tools")]
        acceptance_step: "annotation_toolbar_number_selected",
        icon: WorkspaceIcon::Number,
        label: UiText::OverlayNumber,
        group: Some(AnnotationToolGroup::Text),
        group_order: Some(2),
    },
    WorkspaceAnnotationToolSpec {
        tool: AnnotationTool::Blur,
        index: 8,
        id: "blur",
        #[cfg(feature = "dev-tools")]
        acceptance_step: "annotation_toolbar_blur_selected",
        icon: WorkspaceIcon::Obscure,
        label: UiText::OverlayBlur,
        group: Some(AnnotationToolGroup::Obscure),
        group_order: Some(0),
    },
    WorkspaceAnnotationToolSpec {
        tool: AnnotationTool::Mosaic,
        index: 9,
        id: "mosaic",
        #[cfg(feature = "dev-tools")]
        acceptance_step: "annotation_toolbar_mosaic_selected",
        icon: WorkspaceIcon::Mosaic,
        label: UiText::OverlayMosaic,
        group: Some(AnnotationToolGroup::Obscure),
        group_order: Some(1),
    },
    WorkspaceAnnotationToolSpec {
        tool: AnnotationTool::Watermark,
        index: 10,
        id: "watermark",
        #[cfg(feature = "dev-tools")]
        acceptance_step: "annotation_toolbar_watermark_selected",
        icon: WorkspaceIcon::Watermark,
        label: UiText::OverlayWatermark,
        group: Some(AnnotationToolGroup::Text),
        group_order: Some(1),
    },
];

/// Names the small line icons used by the settings navigation rail and compact section picker.
///
/// These icons are kept separate from screenshot-workspace actions so a settings change cannot
/// silently alter the toolbar's semantic catalog or its acceptance order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SettingsIcon {
    Capture,
    Library,
    Record,
    App,
}

/// Identifies the result actions that make up the compact screenshot toolbar.
///
/// The order is part of the screenshot workspace contract: Pin, Save, More, Cancel, Copy.
/// Keeping the IDs and icons in one catalog lets the renderer and acceptance review refer to the
/// same action vocabulary while each action keeps its existing handler and state semantics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkspaceResultAction {
    Pin,
    Save,
    More,
    Cancel,
    Copy,
}

/// Identifies the low-frequency actions promoted into the wide Snow-style toolbar rail.
///
/// The catalog owns the visible order, stable element IDs, and icon mapping. Renderers and
/// acceptance geometry can therefore consume one action vocabulary when the toolbar changes shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkspaceInlineAction {
    Scroll,
    Qr,
    Ocr,
    Translate,
    RecordArea,
    RecordWindow,
}

impl WorkspaceInlineAction {
    /// Returns the stable element ID used by the production toolbar.
    pub(crate) const fn id(self) -> &'static str {
        match self {
            Self::Scroll => "overlay-manual-scroll-inline",
            Self::Qr => "overlay-qr-inline",
            Self::Ocr => "overlay-ocr-inline",
            Self::Translate => "overlay-translate-inline",
            Self::RecordArea => "overlay-record-area-inline",
            Self::RecordWindow => "overlay-record-window-inline",
        }
    }

    /// Returns the line icon associated with this inline action.
    pub(crate) const fn icon(self) -> WorkspaceIcon {
        match self {
            Self::Scroll => WorkspaceIcon::Scroll,
            Self::Qr => WorkspaceIcon::Qr,
            Self::Ocr => WorkspaceIcon::Ocr,
            Self::Translate => WorkspaceIcon::Translate,
            Self::RecordArea | Self::RecordWindow => WorkspaceIcon::Record,
        }
    }

    /// Returns the key used by the overlay to resolve localized tooltip text.
    pub(crate) const fn tooltip_key(self) -> &'static str {
        match self {
            Self::Scroll => "scroll",
            Self::Qr => "qr",
            Self::Ocr => "ocr",
            Self::Translate => "translate",
            Self::RecordArea => "record-area",
            Self::RecordWindow => "record-window",
        }
    }

    /// Lists the visible order used by the wide screenshot workspace toolbar.
    pub(crate) const fn catalog() -> &'static [Self; 6] {
        &[
            Self::Scroll,
            Self::Qr,
            Self::Ocr,
            Self::Translate,
            Self::RecordArea,
            Self::RecordWindow,
        ]
    }

    /// Measures the complete inline group, including the gaps between icon hit areas.
    pub(crate) const fn total_width(gap: f32) -> f32 {
        let count = Self::catalog().len() as f32;
        count * ThemeMetrics::WORKSPACE_ICON_BUTTON_HIT_AREA + (count - 1.0) * gap
    }
}

/// Identifies actions shown by the expanded More panel.
///
/// This is the low-frequency counterpart of the main toolbar catalogs. It owns the stable focus
/// order, element IDs, visibility groups, and localized-label width budget while the overlay keeps
/// the business handlers and recognition state transitions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkspaceMoreAction {
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

impl WorkspaceMoreAction {
    /// Returns the stable menu-local focus index.
    pub(crate) const fn index(self) -> usize {
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

    /// Returns the stable element ID used by the More panel.
    pub(crate) const fn id(self) -> &'static str {
        match self {
            Self::SaveAnnotations => "overlay-save-annotations",
            Self::SaveEditable => "overlay-save-editable-project",
            Self::OpenAnnotations => "overlay-open-annotations",
            Self::QuickSave => "overlay-quick-save",
            Self::ScrollShot => "overlay-manual-scroll",
            Self::Qr => "overlay-qr",
            Self::Ocr => "overlay-ocr",
            Self::CopyColor => "overlay-copy-color",
            Self::Translate => "overlay-translate",
            Self::RecordArea => "overlay-record-area",
            Self::RecordWindow => "overlay-record-window",
            Self::RetryRecognition => "overlay-retry-recognition",
            Self::CopyRecognition => "overlay-copy-recognition",
            Self::ClearRecognition => "overlay-clear-recognition",
        }
    }

    /// Returns the width budget used to keep localized More actions readable.
    pub(crate) const fn width(self) -> f32 {
        match self {
            Self::SaveAnnotations => 138.0,
            Self::SaveEditable => 128.0,
            Self::OpenAnnotations => 143.0,
            Self::QuickSave => 92.0,
            Self::ScrollShot => 91.0,
            Self::Qr => 72.0,
            Self::Ocr => 84.0,
            Self::CopyColor => 92.0,
            Self::Translate => 81.0,
            Self::RecordArea => 101.0,
            Self::RecordWindow => 126.0,
            Self::RetryRecognition => 126.0,
            Self::CopyRecognition => 76.0,
            Self::ClearRecognition => 92.0,
        }
    }

    /// Lists every possible action in stable focus order.
    pub(crate) const fn catalog() -> &'static [Self; 14] {
        &[
            Self::SaveAnnotations,
            Self::SaveEditable,
            Self::OpenAnnotations,
            Self::QuickSave,
            Self::ScrollShot,
            Self::Qr,
            Self::Ocr,
            Self::CopyColor,
            Self::Translate,
            Self::RecordArea,
            Self::RecordWindow,
            Self::RetryRecognition,
            Self::CopyRecognition,
            Self::ClearRecognition,
        ]
    }

    /// Lists the actions visible whenever More is expanded.
    pub(crate) const fn always_visible_catalog() -> &'static [Self; 11] {
        &[
            Self::SaveAnnotations,
            Self::SaveEditable,
            Self::OpenAnnotations,
            Self::QuickSave,
            Self::ScrollShot,
            Self::Qr,
            Self::Ocr,
            Self::CopyColor,
            Self::Translate,
            Self::RecordArea,
            Self::RecordWindow,
        ]
    }
}

impl WorkspaceResultAction {
    /// Returns the stable position used by the toolbar interaction plan.
    #[cfg(feature = "dev-tools")]
    pub(crate) const fn index(self) -> usize {
        match self {
            Self::Pin => 0,
            Self::Save => 1,
            Self::More => 2,
            Self::Cancel => 3,
            Self::Copy => 4,
        }
    }

    /// Returns the stable element ID used by the production toolbar.
    pub(crate) const fn id(self) -> &'static str {
        match self {
            Self::Pin => "overlay-pin",
            Self::Save => "overlay-save",
            Self::More => "overlay-more-actions",
            Self::Cancel => "overlay-cancel",
            Self::Copy => "overlay-copy",
        }
    }

    /// Returns the line icon associated with this result action.
    pub(crate) const fn icon(self) -> WorkspaceIcon {
        match self {
            Self::Pin => WorkspaceIcon::Pin,
            Self::Save => WorkspaceIcon::Save,
            Self::More => WorkspaceIcon::More,
            Self::Cancel => WorkspaceIcon::Cancel,
            Self::Copy => WorkspaceIcon::Copy,
        }
    }

    /// Lists the compact toolbar order used by the screenshot workspace.
    pub(crate) const fn catalog() -> &'static [Self; 5] {
        &[Self::Pin, Self::Save, Self::More, Self::Cancel, Self::Copy]
    }

    /// Measures the complete result group, including the gaps between icon hit areas.
    pub(crate) const fn total_width(gap: f32) -> f32 {
        let count = Self::catalog().len() as f32;
        count * ThemeMetrics::WORKSPACE_ICON_BUTTON_HIT_AREA + (count - 1.0) * gap
    }
}

#[cfg(test)]
impl WorkspaceIcon {
    /// Returns the stable semantic name used by layout probes and accessibility review.
    ///
    /// Keeping this catalog beside the painter prevents a visual icon from silently losing its
    /// action identity when a toolbar row is reordered or a localized label changes.
    pub(crate) const fn stable_id(self) -> &'static str {
        match self {
            Self::DragHandle => "drag-handle",
            Self::Move => "move",
            Self::Text => "text",
            Self::Shape => "shape",
            Self::Line => "line",
            Self::Highlight => "highlight",
            Self::Obscure => "obscure",
            Self::Undo => "undo",
            Self::Redo => "redo",
            Self::Pin => "pin",
            Self::Copy => "copy",
            Self::Save => "save",
            Self::More => "more",
            Self::Cancel => "cancel",
            Self::Scroll => "scroll",
            Self::Qr => "qr",
            Self::Ocr => "ocr",
            Self::Translate => "translate",
            Self::Record => "record",
            Self::Arrow => "arrow",
            Self::Ellipse => "ellipse",
            Self::Freehand => "freehand",
            Self::Mosaic => "mosaic",
            Self::Number => "number",
            Self::Watermark => "watermark",
        }
    }

    /// Lists every icon that can appear in a screenshot workspace surface.
    ///
    /// The order is the review order, not a promise about every responsive toolbar row. Callers
    /// still derive placement from the active workflow, while this list gives tests one complete
    /// source for icon and accessibility coverage.
    pub(crate) const fn catalog() -> &'static [Self; 25] {
        &[
            Self::DragHandle,
            Self::Move,
            Self::Text,
            Self::Shape,
            Self::Line,
            Self::Highlight,
            Self::Obscure,
            Self::Undo,
            Self::Redo,
            Self::Pin,
            Self::Copy,
            Self::Save,
            Self::More,
            Self::Cancel,
            Self::Scroll,
            Self::Qr,
            Self::Ocr,
            Self::Translate,
            Self::Record,
            Self::Arrow,
            Self::Ellipse,
            Self::Freehand,
            Self::Mosaic,
            Self::Number,
            Self::Watermark,
        ]
    }
}

impl WorkspaceButtonConfig {
    /// Creates an icon configuration using the workspace's stable toolbar hit height.
    pub(crate) fn icon(
        colors: ThemeColors,
        tone: WorkspaceButtonTone,
        active: bool,
        enabled: bool,
        tooltip: impl Into<SharedString>,
    ) -> Self {
        Self {
            width: Some(ThemeMetrics::WORKSPACE_ICON_BUTTON_HIT_AREA),
            height: ThemeMetrics::WORKSPACE_ICON_BUTTON_HIT_AREA,
            colors,
            tone,
            active,
            enabled,
            tooltip: Some(tooltip.into()),
            icon_size: Some(ThemeMetrics::default().workspace_icon_size),
        }
    }

    /// Creates a text configuration for a caller-owned compact control height.
    pub(crate) fn text(
        width: Option<f32>,
        height: f32,
        colors: ThemeColors,
        tone: WorkspaceButtonTone,
        active: bool,
        enabled: bool,
        tooltip: Option<&'static str>,
    ) -> Self {
        Self {
            width,
            height,
            colors,
            tone,
            active,
            enabled,
            tooltip: tooltip.map(SharedString::from),
            icon_size: None,
        }
    }
}

/// Names the vector icons used by the overlay call sites.
pub(crate) mod icon {
    use super::WorkspaceIcon;

    pub(crate) const MOVE: WorkspaceIcon = WorkspaceIcon::Move;
    pub(crate) const MARK: WorkspaceIcon = WorkspaceIcon::Highlight;
    pub(crate) const UNDO: WorkspaceIcon = WorkspaceIcon::Undo;
    pub(crate) const REDO: WorkspaceIcon = WorkspaceIcon::Redo;
}

const ICON_STROKE_WIDTH: f32 = 1.6;
const ICON_INSET: f32 = 1.5;

/// Builds the shared surface used by the main row, style row, and transient popovers.
pub(crate) fn workspace_surface(colors: ThemeColors, elevated: bool) -> Div {
    let metrics = ThemeMetrics::default();
    div()
        .rounded(px(metrics.radius_md))
        .border_1()
        .border_color(colors.toolbar_border)
        .bg(if elevated {
            colors.toolbar_elevated
        } else {
            colors.toolbar_surface
        })
        .shadow_lg()
}

/// Builds a shared toolbar divider so adjacent action groups keep a clear visual boundary.
pub(crate) fn workspace_separator(
    id: impl Into<gpui::ElementId>,
    colors: ThemeColors,
) -> Stateful<Div> {
    let metrics = ThemeMetrics::default();
    div()
        .id(id)
        .w(px(metrics.workspace_separator_width))
        .h(px(metrics.workspace_separator_height))
        .bg(colors.toolbar_border)
}

/// Builds the decorative grip at the start of the Snow-style toolbar rail.
///
/// The grip is intentionally not focusable or clickable: the capture overlay owns positioning and
/// the existing selection gestures remain the only drag interaction. Keeping it decorative gives
/// the long icon row the same visual starting edge as Snow Shot without changing input routing.
pub(crate) fn workspace_drag_handle(colors: ThemeColors) -> Stateful<Div> {
    let metrics = ThemeMetrics::default();
    div()
        .id("overlay-toolbar-drag-handle")
        .w(px(ThemeMetrics::WORKSPACE_TOOLBAR_DRAG_HANDLE_WIDTH))
        .h(px(metrics.workspace_icon_button_hit_area))
        .flex()
        .items_center()
        .justify_center()
        .child(workspace_icon_element(
            WorkspaceIcon::DragHandle,
            colors.overlay_muted,
        ))
}

/// Builds one icon-first workspace action with shared focus, hover, busy, and accessibility state.
pub(crate) fn workspace_icon_button(
    id: impl Into<gpui::ElementId>,
    icon: WorkspaceIcon,
    aria_label: impl Into<SharedString>,
    config: WorkspaceButtonConfig,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    workspace_button(
        id,
        WorkspaceButtonContent::Icon(icon),
        aria_label,
        config,
        on_click,
    )
}

/// Builds a text action for secondary menus and annotation context controls.
pub(crate) fn workspace_text_button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    config: WorkspaceButtonConfig,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let label = label.into();
    workspace_button(
        id,
        WorkspaceButtonContent::Text(label.clone()),
        label,
        config,
        on_click,
    )
}

/// Builds a text action whose compact visible value has a fuller accessible name.
pub(crate) fn workspace_text_button_with_aria(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    aria_label: impl Into<SharedString>,
    config: WorkspaceButtonConfig,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    workspace_button(
        id,
        WorkspaceButtonContent::Text(label.into()),
        aria_label,
        config,
        on_click,
    )
}

enum WorkspaceButtonContent {
    Text(SharedString),
    Icon(WorkspaceIcon),
}

/// Builds the shared button shell used by icon and text workspace controls.
fn workspace_button(
    id: impl Into<gpui::ElementId>,
    content: WorkspaceButtonContent,
    aria_label: impl Into<SharedString>,
    config: WorkspaceButtonConfig,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let metrics = ThemeMetrics::default();
    let aria_label = aria_label.into();
    let WorkspaceButtonConfig {
        width,
        height,
        colors,
        tone,
        active,
        enabled,
        tooltip,
        icon_size,
    } = config;
    let (background, foreground, border) = button_colors(colors, tone, active, enabled);
    let button = div()
        .id(id)
        .role(Role::Button)
        .aria_label(aria_label)
        .when_some(width, |button, width| button.w(px(width)))
        .h(px(height))
        .when(icon_size.is_some(), |button| button.px_0())
        .when(icon_size.is_none(), |button| button.px_3())
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(metrics.radius_sm))
        .border_1()
        .border_color(border)
        .bg(background)
        .text_color(foreground)
        .when_some(icon_size, |button, icon_size| {
            button.text_size(px(icon_size))
        })
        .when(icon_size.is_none(), |button| button.text_sm())
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .when_some(tooltip, |button, tooltip| {
            button.tooltip(move |_, cx| {
                cx.new(|_| WorkspaceTooltip {
                    text: tooltip.clone(),
                    colors,
                })
                .into()
            })
        })
        .focus_visible(move |style| style.border_color(colors.toolbar_focus))
        .hover(move |style| {
            if !enabled {
                return style;
            }
            let (background, foreground, border) = button_hover_colors(colors, tone, active);
            style
                .bg(background)
                .text_color(foreground)
                .border_color(border)
        })
        .active(move |style| {
            if !enabled {
                return style;
            }
            let (background, foreground, border) = button_active_colors(colors, tone);
            style
                .bg(background)
                .text_color(foreground)
                .border_color(border)
        })
        .when(enabled, |button| {
            button.focusable().cursor_pointer().on_click(on_click)
        });
    let mut button = match content {
        WorkspaceButtonContent::Text(content) => button.child(content),
        WorkspaceButtonContent::Icon(icon) => {
            button.child(workspace_icon_element(icon, foreground))
        }
    };
    if !enabled {
        button = button.cursor_not_allowed();
    }
    button
}

/// Creates a fixed-size canvas for one line icon so glyph-specific font metrics cannot move the
/// visual center of adjacent buttons. The canvas is decorative; the parent button owns semantics.
fn workspace_icon_element(icon: WorkspaceIcon, color: Hsla) -> impl IntoElement {
    let size = ThemeMetrics::default().workspace_icon_size;
    canvas(
        move |_, _, _| (icon, color),
        move |bounds, (icon, color), window, _| {
            paint_workspace_icon(window, bounds, icon, color);
        },
    )
    .w(px(size))
    .h(px(size))
    .flex_none()
}

/// Creates a fixed-size decorative canvas for one settings navigation icon. The surrounding
/// navigation item keeps the visible label and keyboard semantics, so the canvas is visual only.
pub(crate) fn settings_icon_element(icon: SettingsIcon, color: Hsla) -> impl IntoElement {
    let size = ThemeMetrics::default().workspace_icon_size;
    canvas(
        move |_, _, _| (icon, color),
        move |bounds, (icon, color), window, _| {
            paint_settings_icon(window, bounds, icon, color);
        },
    )
    .w(px(size))
    .h(px(size))
    .flex_none()
}

/// Paints a compact line icon using one coordinate system and one stroke width.
fn paint_workspace_icon(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    icon: WorkspaceIcon,
    color: Hsla,
) {
    let left = f32::from(bounds.origin.x) + ICON_INSET;
    let top = f32::from(bounds.origin.y) + ICON_INSET;
    let right = f32::from(bounds.origin.x + bounds.size.width) - ICON_INSET;
    let bottom = f32::from(bounds.origin.y + bounds.size.height) - ICON_INSET;
    let center_x = (left + right) / 2.0;
    let center_y = (top + bottom) / 2.0;

    match icon {
        WorkspaceIcon::DragHandle => {
            for offset in [-4.0, 0.0, 4.0] {
                draw_icon_dot(window, color, center_x, center_y + offset, 1.15);
                draw_icon_dot(window, color, center_x + 4.0, center_y + offset, 1.15);
            }
        }
        WorkspaceIcon::Move => {
            draw_icon_stroke(window, color, &[(left, center_y), (right, center_y)]);
            draw_icon_stroke(window, color, &[(center_x, top), (center_x, bottom)]);
            draw_icon_stroke(
                window,
                color,
                &[
                    (left, center_y),
                    (left + 3.0, center_y - 3.0),
                    (left + 3.0, center_y + 3.0),
                ],
            );
            draw_icon_stroke(
                window,
                color,
                &[
                    (right, center_y),
                    (right - 3.0, center_y - 3.0),
                    (right - 3.0, center_y + 3.0),
                ],
            );
            draw_icon_stroke(
                window,
                color,
                &[
                    (center_x, top),
                    (center_x - 3.0, top + 3.0),
                    (center_x + 3.0, top + 3.0),
                ],
            );
            draw_icon_stroke(
                window,
                color,
                &[
                    (center_x, bottom),
                    (center_x - 3.0, bottom - 3.0),
                    (center_x + 3.0, bottom - 3.0),
                ],
            );
        }
        WorkspaceIcon::Text => {
            draw_icon_stroke(
                window,
                color,
                &[(left + 2.0, top + 2.0), (right - 2.0, top + 2.0)],
            );
            draw_icon_stroke(
                window,
                color,
                &[(center_x, top + 2.0), (center_x, bottom - 2.0)],
            );
        }
        WorkspaceIcon::Shape => {
            draw_icon_closed_stroke(
                window,
                color,
                &[
                    (left + 2.0, top + 2.0),
                    (right - 2.0, top + 2.0),
                    (right - 2.0, bottom - 2.0),
                    (left + 2.0, bottom - 2.0),
                ],
            );
        }
        WorkspaceIcon::Line => {
            draw_icon_stroke(
                window,
                color,
                &[(left + 2.0, bottom - 2.0), (right - 2.0, top + 2.0)],
            );
        }
        WorkspaceIcon::Highlight => {
            draw_icon_stroke(
                window,
                color,
                &[(left + 3.0, bottom - 3.0), (right - 3.0, top + 3.0)],
            );
            draw_icon_stroke(
                window,
                color,
                &[(left + 2.0, bottom - 6.0), (left + 5.0, bottom - 3.0)],
            );
            draw_icon_stroke(
                window,
                color,
                &[(right - 6.0, top + 3.0), (right - 3.0, top + 6.0)],
            );
        }
        WorkspaceIcon::Obscure => {
            draw_icon_closed_stroke(
                window,
                color,
                &[
                    (left + 2.0, top + 2.0),
                    (right - 2.0, top + 2.0),
                    (right - 2.0, bottom - 2.0),
                    (left + 2.0, bottom - 2.0),
                ],
            );
            draw_icon_stroke(
                window,
                color,
                &[(center_x, top + 2.0), (center_x, bottom - 2.0)],
            );
            draw_icon_stroke(
                window,
                color,
                &[(left + 2.0, center_y), (right - 2.0, center_y)],
            );
        }
        WorkspaceIcon::Undo | WorkspaceIcon::Redo => {
            let direction = if icon == WorkspaceIcon::Undo {
                -1.0
            } else {
                1.0
            };
            let points = (0..=12)
                .map(|index| {
                    let angle = std::f32::consts::PI * (index as f32 / 12.0);
                    (
                        center_x + direction * angle.cos() * 5.0,
                        center_y + angle.sin() * 5.0,
                    )
                })
                .collect::<Vec<_>>();
            draw_icon_stroke(window, color, &points);
            let arrow_x = center_x + direction * 5.0;
            draw_icon_stroke(
                window,
                color,
                &[
                    (arrow_x, center_y),
                    (arrow_x - direction * 3.0, center_y - 2.5),
                    (arrow_x - direction * 3.0, center_y + 2.5),
                ],
            );
        }
        WorkspaceIcon::Pin => {
            draw_icon_closed_stroke(
                window,
                color,
                &[
                    (center_x - 3.5, top + 2.0),
                    (center_x + 3.5, top + 2.0),
                    (center_x + 2.5, center_y + 1.0),
                    (center_x, center_y + 3.5),
                    (center_x - 2.5, center_y + 1.0),
                ],
            );
            draw_icon_stroke(
                window,
                color,
                &[(center_x, center_y + 3.5), (center_x, bottom - 1.5)],
            );
        }
        WorkspaceIcon::Copy => {
            draw_icon_closed_stroke(
                window,
                color,
                &[
                    (left + 4.0, top + 4.0),
                    (right - 1.0, top + 4.0),
                    (right - 1.0, bottom - 1.0),
                    (left + 4.0, bottom - 1.0),
                ],
            );
            draw_icon_closed_stroke(
                window,
                color,
                &[
                    (left + 1.0, top + 1.0),
                    (right - 4.0, top + 1.0),
                    (right - 4.0, bottom - 4.0),
                    (left + 1.0, bottom - 4.0),
                ],
            );
        }
        WorkspaceIcon::Save => {
            draw_icon_closed_stroke(
                window,
                color,
                &[
                    (left + 2.0, top + 1.0),
                    (right - 2.0, top + 1.0),
                    (right - 2.0, bottom - 1.0),
                    (left + 2.0, bottom - 1.0),
                ],
            );
            draw_icon_stroke(
                window,
                color,
                &[(left + 4.0, top + 1.0), (left + 4.0, top + 5.0)],
            );
            draw_icon_stroke(
                window,
                color,
                &[(right - 4.0, top + 1.0), (right - 4.0, top + 5.0)],
            );
            draw_icon_closed_stroke(
                window,
                color,
                &[
                    (left + 4.0, center_y + 1.0),
                    (right - 4.0, center_y + 1.0),
                    (right - 4.0, bottom - 2.0),
                    (left + 4.0, bottom - 2.0),
                ],
            );
        }
        WorkspaceIcon::More => {
            for offset in [-4.0, 0.0, 4.0] {
                draw_icon_dot(window, color, center_x + offset, center_y, 1.2);
            }
        }
        WorkspaceIcon::Cancel => {
            draw_icon_stroke(
                window,
                color,
                &[(left + 3.0, top + 3.0), (right - 3.0, bottom - 3.0)],
            );
            draw_icon_stroke(
                window,
                color,
                &[(right - 3.0, top + 3.0), (left + 3.0, bottom - 3.0)],
            );
        }
        WorkspaceIcon::Scroll => {
            draw_icon_stroke(
                window,
                color,
                &[(center_x, top + 1.5), (center_x, bottom - 1.5)],
            );
            draw_icon_stroke(
                window,
                color,
                &[
                    (center_x, top + 1.5),
                    (center_x - 3.0, top + 4.5),
                    (center_x + 3.0, top + 4.5),
                ],
            );
            draw_icon_stroke(
                window,
                color,
                &[
                    (center_x, bottom - 1.5),
                    (center_x - 3.0, bottom - 4.5),
                    (center_x + 3.0, bottom - 4.5),
                ],
            );
        }
        WorkspaceIcon::Qr => {
            for (x, y) in [
                (left + 2.0, top + 2.0),
                (right - 5.0, top + 2.0),
                (left + 2.0, bottom - 5.0),
            ] {
                draw_icon_closed_stroke(
                    window,
                    color,
                    &[(x, y), (x + 3.0, y), (x + 3.0, y + 3.0), (x, y + 3.0)],
                );
            }
            draw_icon_dot(window, color, right - 3.0, bottom - 3.0, 1.3);
        }
        WorkspaceIcon::Ocr => {
            draw_icon_stroke(
                window,
                color,
                &[
                    (left + 2.0, top + 3.0),
                    (left + 2.0, top + 1.0),
                    (left + 5.0, top + 1.0),
                ],
            );
            draw_icon_stroke(
                window,
                color,
                &[
                    (right - 2.0, top + 3.0),
                    (right - 2.0, top + 1.0),
                    (right - 5.0, top + 1.0),
                ],
            );
            draw_icon_stroke(
                window,
                color,
                &[
                    (left + 2.0, bottom - 3.0),
                    (left + 2.0, bottom - 1.0),
                    (left + 5.0, bottom - 1.0),
                ],
            );
            draw_icon_stroke(
                window,
                color,
                &[
                    (right - 2.0, bottom - 3.0),
                    (right - 2.0, bottom - 1.0),
                    (right - 5.0, bottom - 1.0),
                ],
            );
            draw_icon_stroke(
                window,
                color,
                &[(left + 5.0, center_y), (right - 5.0, center_y)],
            );
        }
        WorkspaceIcon::Translate => {
            draw_icon_stroke(
                window,
                color,
                &[(left + 2.0, top + 3.0), (right - 2.0, top + 3.0)],
            );
            draw_icon_stroke(
                window,
                color,
                &[(center_x - 2.0, top + 1.0), (center_x - 2.0, top + 6.0)],
            );
            draw_icon_stroke(
                window,
                color,
                &[(left + 4.0, top + 6.0), (left + 2.0, bottom - 2.0)],
            );
            draw_icon_stroke(
                window,
                color,
                &[(left + 2.0, bottom - 2.0), (left + 6.0, bottom - 2.0)],
            );
            draw_icon_stroke(
                window,
                color,
                &[(right - 5.0, top + 8.0), (right - 2.0, bottom - 2.0)],
            );
            draw_icon_stroke(
                window,
                color,
                &[(right - 8.0, bottom - 4.0), (right - 2.0, bottom - 4.0)],
            );
        }
        WorkspaceIcon::Record => {
            draw_icon_circle_stroke(window, color, center_x, center_y, 4.5);
            draw_icon_dot(window, color, center_x, center_y, 2.0);
        }
        WorkspaceIcon::Arrow => {
            draw_icon_stroke(
                window,
                color,
                &[(left + 2.0, bottom - 2.0), (right - 2.0, top + 2.0)],
            );
            draw_icon_stroke(
                window,
                color,
                &[
                    (right - 2.0, top + 2.0),
                    (right - 6.0, top + 2.0),
                    (right - 2.0, top + 6.0),
                ],
            );
        }
        WorkspaceIcon::Ellipse => {
            draw_icon_circle_stroke(window, color, center_x, center_y, 6.0);
        }
        WorkspaceIcon::Freehand => {
            draw_icon_stroke(
                window,
                color,
                &[
                    (left + 1.5, center_y + 2.5),
                    (left + 4.0, center_y - 2.5),
                    (center_x - 1.0, center_y + 1.5),
                    (center_x + 2.0, center_y - 3.0),
                    (right - 1.5, center_y + 1.0),
                ],
            );
        }
        WorkspaceIcon::Mosaic => {
            for (x, y) in [
                (left + 1.0, top + 1.0),
                (center_x + 1.0, top + 1.0),
                (left + 1.0, center_y + 1.0),
                (center_x + 1.0, center_y + 1.0),
            ] {
                draw_icon_closed_stroke(
                    window,
                    color,
                    &[(x, y), (x + 5.0, y), (x + 5.0, y + 5.0), (x, y + 5.0)],
                );
            }
        }
        WorkspaceIcon::Number => {
            draw_icon_circle_stroke(window, color, center_x, center_y, 6.0);
            draw_icon_stroke(
                window,
                color,
                &[
                    (center_x - 1.0, center_y - 2.0),
                    (center_x + 1.0, center_y - 3.5),
                    (center_x + 1.0, center_y + 3.0),
                ],
            );
            draw_icon_stroke(
                window,
                color,
                &[
                    (center_x - 2.0, center_y + 3.5),
                    (center_x + 3.0, center_y + 3.5),
                ],
            );
        }
        WorkspaceIcon::Watermark => {
            draw_icon_closed_stroke(
                window,
                color,
                &[
                    (left + 2.0, top + 2.0),
                    (right - 2.0, top + 2.0),
                    (right - 2.0, bottom - 2.0),
                    (left + 2.0, bottom - 2.0),
                ],
            );
            draw_icon_stroke(
                window,
                color,
                &[
                    (left + 3.0, center_y - 2.0),
                    (left + 5.0, center_y + 2.0),
                    (center_x, center_y - 1.0),
                    (right - 5.0, center_y + 2.0),
                    (right - 3.0, center_y - 2.0),
                ],
            );
        }
    }
}

/// Paints the four settings icons on the same coordinate grid as the screenshot toolbar icons.
fn paint_settings_icon(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    icon: SettingsIcon,
    color: Hsla,
) {
    let left = f32::from(bounds.origin.x) + ICON_INSET;
    let top = f32::from(bounds.origin.y) + ICON_INSET;
    let right = f32::from(bounds.origin.x + bounds.size.width) - ICON_INSET;
    let bottom = f32::from(bounds.origin.y + bounds.size.height) - ICON_INSET;
    let center_x = (left + right) / 2.0;
    let center_y = (top + bottom) / 2.0;

    match icon {
        SettingsIcon::Capture => {
            draw_icon_stroke(
                window,
                color,
                &[(left, top + 4.0), (left, top), (left + 4.0, top)],
            );
            draw_icon_stroke(
                window,
                color,
                &[(right - 4.0, top), (right, top), (right, top + 4.0)],
            );
            draw_icon_stroke(
                window,
                color,
                &[(left, bottom - 4.0), (left, bottom), (left + 4.0, bottom)],
            );
            draw_icon_stroke(
                window,
                color,
                &[
                    (right - 4.0, bottom),
                    (right, bottom),
                    (right, bottom - 4.0),
                ],
            );
            draw_icon_dot(window, color, center_x, center_y, 1.4);
        }
        SettingsIcon::Library => {
            draw_icon_closed_stroke(
                window,
                color,
                &[
                    (left + 2.0, top + 3.0),
                    (right - 2.0, top + 3.0),
                    (right - 2.0, bottom - 1.0),
                    (left + 2.0, bottom - 1.0),
                ],
            );
            draw_icon_stroke(
                window,
                color,
                &[(left + 4.0, top + 1.0), (right, top + 1.0)],
            );
        }
        SettingsIcon::Record => {
            draw_icon_circle_stroke(window, color, center_x, center_y, 5.5);
            draw_icon_dot(window, color, center_x, center_y, 2.3);
        }
        SettingsIcon::App => {
            for (y, knob_x) in [
                (top + 2.0, left + 4.0),
                (center_y, right - 4.0),
                (bottom - 2.0, left + 7.0),
            ] {
                draw_icon_stroke(window, color, &[(left + 1.0, y), (right - 1.0, y)]);
                draw_icon_dot(window, color, knob_x, y, 1.5);
            }
        }
    }
}

fn draw_icon_stroke(window: &mut Window, color: Hsla, points: &[(f32, f32)]) {
    if points.len() < 2 {
        return;
    }
    let mut path = PathBuilder::stroke(px(ICON_STROKE_WIDTH));
    for (index, (x, y)) in points.iter().copied().enumerate() {
        let point = point(px(x), px(y));
        if index == 0 {
            path.move_to(point);
        } else {
            path.line_to(point);
        }
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

fn draw_icon_closed_stroke(window: &mut Window, color: Hsla, points: &[(f32, f32)]) {
    if points.len() < 3 {
        return;
    }
    let mut closed = points.to_vec();
    closed.push(points[0]);
    draw_icon_stroke(window, color, &closed);
}

fn draw_icon_dot(window: &mut Window, color: Hsla, center_x: f32, center_y: f32, radius: f32) {
    let mut path = PathBuilder::fill();
    for index in 0..=16 {
        let angle = std::f32::consts::TAU * index as f32 / 16.0;
        let point = point(
            px(center_x + radius * angle.cos()),
            px(center_y + radius * angle.sin()),
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

fn draw_icon_circle_stroke(
    window: &mut Window,
    color: Hsla,
    center_x: f32,
    center_y: f32,
    radius: f32,
) {
    let points = (0..=16)
        .map(|index| {
            let angle = std::f32::consts::TAU * index as f32 / 16.0;
            (
                center_x + radius * angle.cos(),
                center_y + radius * angle.sin(),
            )
        })
        .collect::<Vec<_>>();
    draw_icon_stroke(window, color, &points);
}

/// Builds a compact color swatch with a stable hit area and an accessible name.
pub(crate) fn workspace_swatch(
    id: impl Into<gpui::ElementId>,
    color: Hsla,
    selected: bool,
    aria_label: impl Into<SharedString>,
    tooltip: impl Into<SharedString>,
    colors: ThemeColors,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let metrics = ThemeMetrics::default();
    let aria_label = aria_label.into();
    let tooltip = tooltip.into();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(aria_label)
        .w(px(metrics.workspace_swatch_size))
        .h(px(metrics.workspace_swatch_size))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(metrics.radius_sm))
        .border_2()
        .border_color(if selected {
            colors.toolbar_focus
        } else {
            colors.toolbar_border
        })
        .bg(color)
        .cursor_pointer()
        .tooltip(move |_, cx| {
            cx.new(|_| WorkspaceTooltip {
                text: tooltip.clone(),
                colors,
            })
            .into()
        })
        .focusable()
        .focus_visible(move |style| style.border_color(colors.toolbar_focus))
        .hover(move |style| style.border_color(colors.toolbar_focus))
        .on_click(on_click)
}

/// Supplies the compact tooltip used by icon-first controls.
pub(crate) struct WorkspaceTooltip {
    text: SharedString,
    colors: ThemeColors,
}

impl Render for WorkspaceTooltip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let metrics = ThemeMetrics::default();
        div()
            .px(px(metrics.workspace_toolbar_padding))
            .py_1()
            .rounded(px(metrics.radius_sm))
            .bg(self.colors.toolbar_elevated)
            .border_1()
            .border_color(self.colors.toolbar_border)
            .text_color(self.colors.text)
            .text_xs()
            .shadow_lg()
            .child(self.text.clone())
    }
}

/// Resolves the resting colors for a workspace control, including disabled and selected states.
fn button_colors(
    colors: ThemeColors,
    tone: WorkspaceButtonTone,
    active: bool,
    enabled: bool,
) -> (Hsla, Hsla, Hsla) {
    if !enabled {
        return (
            colors.toolbar_surface,
            colors.text_disabled,
            colors.toolbar_surface,
        );
    }
    match tone {
        WorkspaceButtonTone::Primary => (
            if active {
                colors.toolbar_active
            } else {
                colors.accent
            },
            colors.background,
            colors.accent,
        ),
        WorkspaceButtonTone::Destructive => (colors.danger, colors.background, colors.danger),
        WorkspaceButtonTone::Neutral => (
            if active {
                colors.toolbar_hover
            } else {
                // Resting controls share the continuous toolbar surface. The target workspace
                // uses the active, hover, and focus states to reveal affordances without turning
                // every icon into a separate tile.
                colors.toolbar_surface
            },
            colors.text,
            if active {
                colors.toolbar_focus
            } else {
                colors.toolbar_surface
            },
        ),
    }
}

/// Resolves hover colors while preserving the semantic tone of the action.
fn button_hover_colors(
    colors: ThemeColors,
    tone: WorkspaceButtonTone,
    active: bool,
) -> (Hsla, Hsla, Hsla) {
    match tone {
        WorkspaceButtonTone::Primary => (
            if active {
                colors.toolbar_hover
            } else {
                colors.accent_hover
            },
            colors.background,
            colors.toolbar_focus,
        ),
        WorkspaceButtonTone::Destructive => (colors.toolbar_hover, colors.danger, colors.danger),
        WorkspaceButtonTone::Neutral => (colors.toolbar_hover, colors.text, colors.toolbar_focus),
    }
}

/// Resolves pressed colors after the shared button has already confirmed it is enabled.
fn button_active_colors(colors: ThemeColors, tone: WorkspaceButtonTone) -> (Hsla, Hsla, Hsla) {
    match tone {
        WorkspaceButtonTone::Primary => (
            colors.accent_pressed,
            colors.background,
            colors.accent_pressed,
        ),
        WorkspaceButtonTone::Destructive => (colors.danger, colors.background, colors.danger),
        WorkspaceButtonTone::Neutral => (colors.toolbar_hover, colors.text, colors.toolbar_focus),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AnnotationToolGroup, WorkspaceAnnotationToolSpec, WorkspaceButtonTone, WorkspaceIcon,
        WorkspaceInlineAction, WorkspaceMoreAction, WorkspaceResultAction, button_colors,
    };
    use crate::{
        domain::annotation::AnnotationTool,
        i18n::UiText,
        theme::{ThemeColors, ThemeMetrics, ThemeMode},
    };

    #[test]
    fn resting_neutral_buttons_blend_into_the_toolbar_surface() {
        for mode in [ThemeMode::Dark, ThemeMode::Light] {
            let colors = ThemeColors::for_mode(mode);
            let (background, foreground, border) =
                button_colors(colors, WorkspaceButtonTone::Neutral, false, true);

            assert_eq!(background, colors.toolbar_surface);
            assert_eq!(foreground, colors.text);
            assert_eq!(border, colors.toolbar_surface);
        }
    }

    #[test]
    fn active_neutral_buttons_keep_a_visible_focus_edge() {
        for mode in [ThemeMode::Dark, ThemeMode::Light] {
            let colors = ThemeColors::for_mode(mode);
            let (background, foreground, border) =
                button_colors(colors, WorkspaceButtonTone::Neutral, true, true);

            assert_eq!(background, colors.toolbar_hover);
            assert_eq!(foreground, colors.text);
            assert_eq!(border, colors.toolbar_focus);
        }
    }

    #[test]
    fn workspace_icon_catalog_has_unique_stable_ids() {
        let icons = WorkspaceIcon::catalog();
        let ids = icons.map(WorkspaceIcon::stable_id);

        assert_eq!(
            ids,
            [
                "drag-handle",
                "move",
                "text",
                "shape",
                "line",
                "highlight",
                "obscure",
                "undo",
                "redo",
                "pin",
                "copy",
                "save",
                "more",
                "cancel",
                "scroll",
                "qr",
                "ocr",
                "translate",
                "record",
                "arrow",
                "ellipse",
                "freehand",
                "mosaic",
                "number",
                "watermark",
            ]
        );
        for (index, id) in ids.iter().enumerate() {
            assert!(ids[index + 1..].iter().all(|other| other != id));
        }
    }

    #[test]
    fn result_action_catalog_matches_the_compact_toolbar_contract() {
        let actions = WorkspaceResultAction::catalog();

        assert_eq!(
            actions.map(WorkspaceResultAction::id),
            [
                "overlay-pin",
                "overlay-save",
                "overlay-more-actions",
                "overlay-cancel",
                "overlay-copy",
            ]
        );
        assert_eq!(
            actions.map(WorkspaceResultAction::icon),
            [
                WorkspaceIcon::Pin,
                WorkspaceIcon::Save,
                WorkspaceIcon::More,
                WorkspaceIcon::Cancel,
                WorkspaceIcon::Copy,
            ]
        );
    }

    #[test]
    fn inline_action_catalog_matches_the_wide_toolbar_contract() {
        let actions = WorkspaceInlineAction::catalog();

        assert_eq!(
            actions.map(WorkspaceInlineAction::id),
            [
                "overlay-manual-scroll-inline",
                "overlay-qr-inline",
                "overlay-ocr-inline",
                "overlay-translate-inline",
                "overlay-record-area-inline",
                "overlay-record-window-inline",
            ]
        );
        assert_eq!(
            actions.map(WorkspaceInlineAction::icon),
            [
                WorkspaceIcon::Scroll,
                WorkspaceIcon::Qr,
                WorkspaceIcon::Ocr,
                WorkspaceIcon::Translate,
                WorkspaceIcon::Record,
                WorkspaceIcon::Record,
            ]
        );
        assert_eq!(
            actions.map(WorkspaceInlineAction::tooltip_key),
            [
                "scroll",
                "qr",
                "ocr",
                "translate",
                "record-area",
                "record-window",
            ]
        );
        assert_eq!(
            WorkspaceInlineAction::total_width(ThemeMetrics::WORKSPACE_TOOLBAR_GAP),
            6.0 * ThemeMetrics::WORKSPACE_ICON_BUTTON_HIT_AREA
                + 5.0 * ThemeMetrics::WORKSPACE_TOOLBAR_GAP
        );
    }

    #[test]
    fn annotation_tool_catalog_owns_order_icons_ids_labels_and_groups() {
        let tools = WorkspaceAnnotationToolSpec::catalog();

        assert_eq!(
            tools.map(|spec| spec.tool()),
            [
                AnnotationTool::Rectangle,
                AnnotationTool::Ellipse,
                AnnotationTool::Arrow,
                AnnotationTool::Line,
                AnnotationTool::Freehand,
                AnnotationTool::Highlight,
                AnnotationTool::Text,
                AnnotationTool::Number,
                AnnotationTool::Blur,
                AnnotationTool::Mosaic,
                AnnotationTool::Watermark,
            ]
        );
        assert_eq!(
            tools.map(|spec| spec.id()),
            [
                "rectangle",
                "ellipse",
                "arrow",
                "line",
                "freehand",
                "highlight",
                "text",
                "number",
                "blur",
                "mosaic",
                "watermark",
            ]
        );
        assert_eq!(
            tools.map(|spec| spec.icon().stable_id()),
            [
                "shape",
                "ellipse",
                "arrow",
                "line",
                "freehand",
                "highlight",
                "text",
                "number",
                "obscure",
                "mosaic",
                "watermark",
            ]
        );
        assert_eq!(
            tools.map(|spec| spec.index()),
            [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
        );
        assert_eq!(
            tools.map(|spec| spec.label()),
            [
                UiText::OverlayRectangle,
                UiText::OverlayEllipse,
                UiText::OverlayArrow,
                UiText::OverlayLine,
                UiText::OverlayFreehand,
                UiText::OverlayHighlight,
                UiText::OverlayText,
                UiText::OverlayNumber,
                UiText::OverlayBlur,
                UiText::OverlayMosaic,
                UiText::OverlayWatermark,
            ]
        );
        for (index, spec) in tools.iter().enumerate() {
            assert_eq!(WorkspaceAnnotationToolSpec::for_tool(spec.tool()), *spec);
            assert_eq!(spec.index(), index);
        }
    }

    #[cfg(feature = "dev-tools")]
    #[test]
    fn annotation_tool_catalog_keeps_native_acceptance_steps_stable() {
        assert_eq!(
            WorkspaceAnnotationToolSpec::catalog().map(|spec| spec.acceptance_step()),
            [
                "annotation_toolbar_rectangle_selected",
                "annotation_toolbar_ellipse_selected",
                "annotation_toolbar_arrow_selected",
                "annotation_toolbar_line_selected",
                "annotation_toolbar_freehand_selected",
                "annotation_toolbar_highlight_selected",
                "annotation_toolbar_text_selected",
                "annotation_toolbar_number_selected",
                "annotation_toolbar_blur_selected",
                "annotation_toolbar_mosaic_selected",
                "annotation_toolbar_watermark_selected",
            ]
        );
    }

    #[test]
    fn annotation_tool_group_catalog_matches_popover_keyboard_order() {
        assert_eq!(
            AnnotationToolGroup::catalog().map(AnnotationToolGroup::index),
            [0, 1, 2, 3]
        );
        assert_eq!(
            AnnotationToolGroup::catalog().map(AnnotationToolGroup::id),
            ["text", "shape", "line", "obscure"]
        );
        assert_eq!(
            WorkspaceAnnotationToolSpec::group_tools(AnnotationToolGroup::Text),
            [
                Some(AnnotationTool::Text),
                Some(AnnotationTool::Watermark),
                Some(AnnotationTool::Number),
            ]
        );
        assert_eq!(
            WorkspaceAnnotationToolSpec::group_tools(AnnotationToolGroup::Shape),
            [
                Some(AnnotationTool::Rectangle),
                Some(AnnotationTool::Ellipse),
                None,
            ]
        );
        assert_eq!(
            WorkspaceAnnotationToolSpec::group_tools(AnnotationToolGroup::Line),
            [
                Some(AnnotationTool::Line),
                Some(AnnotationTool::Arrow),
                Some(AnnotationTool::Freehand),
            ]
        );
        assert_eq!(
            WorkspaceAnnotationToolSpec::group_tools(AnnotationToolGroup::Obscure),
            [
                Some(AnnotationTool::Blur),
                Some(AnnotationTool::Mosaic),
                None,
            ]
        );
        assert_eq!(
            AnnotationToolGroup::catalog().map(WorkspaceAnnotationToolSpec::group_tool_count),
            [3, 2, 3, 2]
        );
    }

    #[test]
    fn more_action_catalog_matches_the_expanded_panel_contract() {
        let actions = WorkspaceMoreAction::catalog();

        assert_eq!(
            actions.map(WorkspaceMoreAction::id),
            [
                "overlay-save-annotations",
                "overlay-save-editable-project",
                "overlay-open-annotations",
                "overlay-quick-save",
                "overlay-manual-scroll",
                "overlay-qr",
                "overlay-ocr",
                "overlay-copy-color",
                "overlay-translate",
                "overlay-record-area",
                "overlay-record-window",
                "overlay-retry-recognition",
                "overlay-copy-recognition",
                "overlay-clear-recognition",
            ]
        );
        assert_eq!(WorkspaceMoreAction::always_visible_catalog().len(), 11);
        assert_eq!(
            actions.map(WorkspaceMoreAction::index),
            [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13]
        );
        assert_eq!(
            actions.map(WorkspaceMoreAction::width),
            [
                138.0, 128.0, 143.0, 92.0, 91.0, 72.0, 84.0, 92.0, 81.0, 101.0, 126.0, 126.0, 76.0,
                92.0,
            ]
        );
    }
}
