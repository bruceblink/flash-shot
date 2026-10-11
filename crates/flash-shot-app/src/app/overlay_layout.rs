//! Workspace layout data and geometry helpers for the capture overlay.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ActionToolbarLayout {
    pub(super) left: f32,
    pub(super) top: f32,
    pub(super) width: f32,
    pub(super) height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct AnnotationToolbarLayout {
    pub(super) left: f32,
    pub(super) top: f32,
    pub(super) width: f32,
    pub(super) height: f32,
    pub(super) tools_width: f32,
    pub(super) tools_height: f32,
    pub(super) tools_top: f32,
    pub(super) style_left: f32,
    pub(super) style_top: f32,
    pub(super) style_height: f32,
    pub(super) action_toolbar: ActionToolbarLayout,
    pub(super) actions_above_tools: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct AnnotationToolbarItems {
    pub(super) selection_context: usize,
    pub(super) arrange_context: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct AnnotationToolbarLayoutOptions {
    pub(super) items: AnnotationToolbarItems,
    pub(super) style_width: f32,
    pub(super) style_height: f32,
    pub(super) annotation_tool_group: Option<AnnotationToolGroup>,
    pub(super) tool_estimated_width: f32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct AnnotationStyleCapabilities {
    pub(super) color: bool,
    pub(super) fill: bool,
    pub(super) width: bool,
    pub(super) opacity: bool,
    pub(super) font_size: bool,
}

impl AnnotationStyleCapabilities {
    pub(super) const EMPTY: Self = Self {
        color: false,
        fill: false,
        width: false,
        opacity: false,
        font_size: false,
    };

    pub(super) const fn has_controls(self) -> bool {
        self.color || self.fill || self.width || self.opacity || self.font_size
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct AnnotationLayerLayout {
    pub(super) left: f32,
    pub(super) top: f32,
    pub(super) max_height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SelectionDimensionLayout {
    pub(super) left: f32,
    pub(super) top: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SmartTargetHudLayout {
    pub(super) target: InspectionTarget,
    pub(super) left: f32,
    pub(super) top: f32,
    pub(super) width: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct WorkspaceSafeArea {
    pub(super) left: f32,
    pub(super) top: f32,
    pub(super) right: f32,
    pub(super) bottom: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct WorkspaceSelectionAnchor {
    pub(super) top_left: ViewPoint,
    pub(super) bottom_right: ViewPoint,
}

impl WorkspaceSelectionAnchor {
    /// Converts one non-empty physical selection into the shared view-space anchor.
    ///
    /// Rejecting zero-area rectangles here keeps every workspace surface on the same validity
    /// rule. Callers must use this anchor instead of repeating physical-to-view conversions.
    pub(super) fn from_selection(
        selection: PhysicalRect,
        transform: PreviewTransform,
    ) -> Option<Self> {
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
pub(super) struct SecondaryMenuLayout {
    pub(super) left: f32,
    pub(super) width: f32,
    pub(super) height: f32,
    pub(super) top_offset: f32,
    pub(super) opens_above: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct AnnotationToolGroupLayout {
    pub(super) left: f32,
    pub(super) top: f32,
    pub(super) width: f32,
    pub(super) height: f32,
}

/// Captures one immutable workspace layout so every surface uses the same selection anchor.
///
/// The snapshot keeps the main row, context rows, transient menus, HUDs, and safe area together;
/// callers must not recalculate one of these values during rendering or input dispatch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct WorkspaceLayoutSnapshot {
    pub(super) safe_area: WorkspaceSafeArea,
    pub(super) selection_anchor: Option<WorkspaceSelectionAnchor>,
    pub(super) action_toolbar: Option<ActionToolbarLayout>,
    pub(super) annotation_toolbar: Option<AnnotationToolbarLayout>,
    pub(super) annotation_layer: Option<AnnotationLayerLayout>,
    pub(super) secondary_menu: Option<SecondaryMenuLayout>,
    pub(super) annotation_tool_group: Option<AnnotationToolGroupLayout>,
    pub(super) dimension: Option<SelectionDimensionLayout>,
    pub(super) smart_target_hud: Option<SmartTargetHudLayout>,
    pub(super) status_inset: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct WorkspaceLayoutInput {
    pub(super) selection: Option<PhysicalRect>,
    pub(super) display_bounds: PhysicalRect,
    pub(super) transform: Option<PreviewTransform>,
    pub(super) viewport: Bounds<Pixels>,
    pub(super) hover_pixel: Option<PhysicalPoint>,
    pub(super) inspection_target: Option<InspectionTarget>,
    pub(super) show_annotation_controls: bool,
    pub(super) annotation_toolbar_items: AnnotationToolbarItems,
    pub(super) annotation_style_capabilities: AnnotationStyleCapabilities,
    pub(super) annotation_tool_group: Option<AnnotationToolGroup>,
    pub(super) annotation_tool_width: f32,
    pub(super) has_recognition_result: bool,
    pub(super) has_recognition_retry: bool,
    pub(super) recognition_in_flight: bool,
}

/// Maps a drawing tool to the style values that its renderer actually consumes.
///
/// Effect tools intentionally return no controls because their appearance is fixed by the
/// renderer; exposing unrelated values would create a misleading empty settings surface.
pub(super) const fn annotation_style_capabilities_for_tool(
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
pub(super) const fn annotation_style_capabilities_for_kind(
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
pub(super) fn annotation_style_capabilities(
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
pub(super) fn annotation_style_row_width(capabilities: AnnotationStyleCapabilities) -> f32 {
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
pub(super) fn annotation_style_row_preferred_width(
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
pub(super) fn annotation_style_group_widths(
    capabilities: AnnotationStyleCapabilities,
) -> [Option<f32>; 5] {
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
pub(super) fn capabilities_group_count(capabilities: AnnotationStyleCapabilities) -> usize {
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
pub(super) fn annotation_style_row_height(
    width: f32,
    capabilities: AnnotationStyleCapabilities,
) -> f32 {
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
pub(super) fn arrange_context_for_selection(
    expanded_for: Option<AnnotationId>,
    selected_annotation: Option<AnnotationId>,
) -> Option<AnnotationId> {
    expanded_for.filter(|id| Some(*id) == selected_annotation)
}

/// Counts the contextual controls that belong beside a selected annotation.
/// The drawing palette remains a separate stable section; arrange controls only consume space
/// after the user explicitly expands them.
pub(super) fn annotation_toolbar_items(
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
pub(super) fn annotation_toolbar_height(
    viewport: Bounds<Pixels>,
    items: AnnotationToolbarItems,
) -> f32 {
    let viewport = view_rect(viewport);
    annotation_toolbar_height_for_width(
        (viewport.width - OVERLAY_EDGE_INSET * 2.0).max(1.0),
        items,
        None,
        ANNOTATION_TOOL_ESTIMATED_WIDTH,
    )
}

/// Measures the compact icon palette including its rounded surface and padding.
pub(super) fn annotation_tool_palette_width() -> f32 {
    ANNOTATION_TOOL_PALETTE_ITEMS as f32 * ANNOTATION_TOOL_ICON_WIDTH
        + ANNOTATION_TOOL_PALETTE_ITEMS.saturating_sub(1) as f32 * ANNOTATION_TOOL_PALETTE_GAP
        + ANNOTATION_TOOLBAR_PADDING * 2.0
        + ANNOTATION_TOOL_GROUP_POPUP_BORDER * 2.0
}

/// Chooses a natural dock width from the visible tool icons, style groups, context actions, and
/// result toolbar, then clamps that width to the display's safe horizontal area.
pub(super) fn annotation_toolbar_preferred_width(
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
    let palette_width = (items.selection_context > 0 || items.arrange_context > 0)
        .then(annotation_tool_palette_width)
        .unwrap_or_default();
    let natural_width = action_toolbar_width
        .max(palette_width)
        .max(style_width)
        .max(context_width(items.selection_context))
        .max(context_width(items.arrange_context));
    let available_width =
        (viewport_width - OVERLAY_EDGE_INSET * 2.0).clamp(1.0, ANNOTATION_TOOLBAR_MAX_WIDTH);
    natural_width.min(available_width)
}

/// Computes the actual icon-palette height so selection-anchored group popovers start below it.
pub(super) fn annotation_tool_palette_height(width: f32) -> f32 {
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
pub(super) fn annotation_tool_group_popover_width(
    width: f32,
    group: AnnotationToolGroup,
    tool_estimated_width: f32,
) -> f32 {
    let available_width = width.max(1.0);
    let tool_count = WorkspaceAnnotationToolSpec::group_tool_count(group);
    let natural_width = tool_count as f32 * tool_estimated_width
        + tool_count.saturating_sub(1) as f32 * ANNOTATION_TOOL_GAP
        + ANNOTATION_TOOL_GROUP_POPUP_PADDING * 2.0
        + ANNOTATION_TOOL_GROUP_POPUP_BORDER * 2.0;
    natural_width.min(available_width)
}

/// Computes wrapped rows for the materialized group using the same dimensions as its renderer.
pub(super) fn annotation_tool_group_popover_height(
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
    let rows = WorkspaceAnnotationToolSpec::group_tool_count(group).div_ceil(columns);
    rows as f32 * ANNOTATION_TOOL_ROW_HEIGHT
        + rows.saturating_sub(1) as f32 * ANNOTATION_TOOL_GAP
        + ANNOTATION_TOOL_GROUP_POPUP_PADDING * 2.0
        + ANNOTATION_TOOL_GROUP_POPUP_BORDER * 2.0
}

/// Measures each visible toolbar section independently so a selection context cannot reorder the
/// drawing palette or reserve rows until it is actually expanded.
pub(super) fn annotation_toolbar_height_for_width(
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
pub(super) fn annotation_toolbar_layout(
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
pub(super) fn annotation_toolbar_layout_with_tool_width(
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
pub(super) fn annotation_layer_layout(
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
pub(super) fn selection_dimension_label_layout(
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
pub(super) fn smart_target_hud_layout(
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
pub(super) fn smart_target_hud_label(locale: Locale, target: InspectionTarget) -> String {
    let kind = inspection_kind_label(locale, target.kind);
    let width = target.bounds.width().to_string();
    let height = target.bounds.height().to_string();
    locale.format_template(
        UiText::SmartTargetLabel,
        &[("kind", kind), ("width", &width), ("height", &height)],
    )
}

/// Places the stable main row near the selection using only shared icon geometry and safe bounds.
pub(super) fn action_toolbar_layout(
    selection_anchor: Option<WorkspaceSelectionAnchor>,
    viewport: Bounds<Pixels>,
    show_annotation_controls: bool,
) -> Option<ActionToolbarLayout> {
    let selection_anchor = selection_anchor?;
    let viewport = view_rect(viewport);
    let available_width = (viewport.width - OVERLAY_EDGE_INSET * 2.0).max(1.0);
    let include_inline_secondary = viewport.width >= INLINE_SECONDARY_ACTIONS_MIN_VIEWPORT_WIDTH;
    let natural_width = if include_inline_secondary {
        action_toolbar_natural_width(show_annotation_controls)
    } else {
        action_toolbar_width_for(show_annotation_controls, false)
    };
    let width = natural_width.min(available_width);
    let height = if include_inline_secondary {
        action_toolbar_height(width, show_annotation_controls)
    } else {
        action_toolbar_height_for(
            width,
            compact_action_toolbar_item_widths(show_annotation_controls),
        )
    };
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

/// Clamps a manually dragged toolbar to the same safe area used by automatic placement.
pub(super) fn toolbar_position_for_pointer(
    gesture: ToolbarDragGesture,
    pointer: ViewPoint,
    viewport: Bounds<Pixels>,
) -> ManualToolbarPosition {
    let viewport = view_rect(viewport);
    let delta_x = pointer.x - gesture.pointer_start.x;
    let delta_y = pointer.y - gesture.pointer_start.y;
    let left_min = viewport.left + OVERLAY_EDGE_INSET;
    let left_max =
        (viewport.right() - OVERLAY_EDGE_INSET - gesture.toolbar_start.width).max(left_min);
    let top_min = viewport.top + OVERLAY_EDGE_INSET;
    let top_max =
        (viewport.bottom() - OVERLAY_BOTTOM_SAFE_INSET - gesture.toolbar_start.height).max(top_min);
    ManualToolbarPosition {
        selection: gesture.selection,
        left: (gesture.toolbar_start.left + delta_x).clamp(left_min, left_max),
        top: (gesture.toolbar_start.top + delta_y).clamp(top_min, top_max),
    }
}

/// Moves the complete screenshot workspace rail and its short-lived attached surfaces together.
pub(super) fn apply_manual_toolbar_position(
    snapshot: &mut WorkspaceLayoutSnapshot,
    position: ManualToolbarPosition,
    viewport: Bounds<Pixels>,
) {
    let Some(base_toolbar) = snapshot.action_toolbar else {
        return;
    };
    let viewport = view_rect(viewport);
    let left_min = viewport.left + OVERLAY_EDGE_INSET;
    let left_max = (viewport.right() - OVERLAY_EDGE_INSET - base_toolbar.width).max(left_min);
    let top_min = viewport.top + OVERLAY_EDGE_INSET;
    let top_max =
        (viewport.bottom() - OVERLAY_BOTTOM_SAFE_INSET - base_toolbar.height).max(top_min);
    let left = position.left.clamp(left_min, left_max);
    let top = position.top.clamp(top_min, top_max);
    let delta_x = left - base_toolbar.left;
    let delta_y = top - base_toolbar.top;
    let moved_toolbar = ActionToolbarLayout {
        left,
        top,
        ..base_toolbar
    };
    snapshot.action_toolbar = Some(moved_toolbar);

    if let Some(mut annotation) = snapshot.annotation_toolbar {
        annotation.left += delta_x;
        annotation.top += delta_y;
        annotation.tools_top += delta_y;
        annotation.style_left += delta_x;
        annotation.style_top += delta_y;
        annotation.action_toolbar = moved_toolbar;
        snapshot.annotation_toolbar = Some(annotation);
    }
    if let Some(mut layers) = snapshot.annotation_layer {
        layers.left += delta_x;
        layers.top += delta_y;
        snapshot.annotation_layer = Some(layers);
    }
    if let Some(mut menu) = snapshot.secondary_menu {
        menu.left += delta_x;
        snapshot.secondary_menu = Some(menu);
    }
    if let Some(mut group) = snapshot.annotation_tool_group {
        group.left += delta_x;
        group.top += delta_y;
        snapshot.annotation_tool_group = Some(group);
    }
}

/// Repositions the automatic workspace rail when neither side can fit the complete More panel.
///
/// A wrapped toolbar is taller than the one-row token spacing. Move the attached surfaces as one
/// unit to the safe edge so More can use the roomier side without covering the main action row.
pub(super) fn position_workspace_for_more_menu(
    snapshot: &mut WorkspaceLayoutSnapshot,
    selection: Option<PhysicalRect>,
    viewport: Bounds<Pixels>,
) {
    let (Some(toolbar), Some(menu), Some(selection)) =
        (snapshot.action_toolbar, snapshot.secondary_menu, selection)
    else {
        return;
    };
    let safe_top = snapshot.safe_area.top;
    let safe_bottom = snapshot.safe_area.bottom;
    let menu_gap = secondary_action_menu_gap();
    let available_above = (toolbar.top - menu_gap - safe_top).max(0.0);
    let available_below = (safe_bottom - toolbar.top - toolbar.height - menu_gap).max(0.0);
    if available_above >= menu.height || available_below >= menu.height {
        return;
    }

    let stack_height = toolbar.height + menu_gap + menu.height;
    if stack_height > safe_bottom - safe_top {
        return;
    }
    let opens_above = available_above > available_below;
    let toolbar_top = if opens_above {
        safe_top + menu.height + menu_gap
    } else {
        safe_bottom - stack_height
    };
    apply_manual_toolbar_position(
        snapshot,
        ManualToolbarPosition {
            selection,
            left: toolbar.left,
            top: toolbar_top,
        },
        viewport,
    );
    if let (Some(toolbar), Some(mut menu)) = (snapshot.action_toolbar, snapshot.secondary_menu) {
        menu.left = secondary_action_menu_left(toolbar, menu.width, viewport);
        menu.opens_above = opens_above;
        menu.top_offset =
            secondary_action_menu_top_offset(toolbar, viewport, menu.height, opens_above);
        snapshot.secondary_menu = Some(menu);
    }
}

/// Computes every workspace surface from one selection/display snapshot.
///
/// The renderer and interaction tests consume this result instead of independently deciding
/// where the action row, style rows, More menu, and transient HUD should move.
pub(super) fn workspace_layout_snapshot(input: WorkspaceLayoutInput) -> WorkspaceLayoutSnapshot {
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
        let opens_above = secondary_menu_opens_above(layout, viewport, height);
        let top_offset = secondary_action_menu_top_offset(layout, viewport, height, opens_above);
        SecondaryMenuLayout {
            left: secondary_action_menu_left(layout, width, viewport),
            width,
            height,
            top_offset,
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
pub(super) fn owns_selection_toolbar(
    selection: PhysicalRect,
    display_bounds: PhysicalRect,
) -> bool {
    selection.width() > 0
        && selection.height() > 0
        && display_bounds.contains(PhysicalPoint {
            x: selection.right.saturating_sub(1),
            y: selection.bottom.saturating_sub(1),
        })
}

/// Limits annotation controls to the display that owns the committed selection's export actions.
pub(super) fn annotation_controls_visible(
    requested: bool,
    selection: Option<PhysicalRect>,
    display_bounds: PhysicalRect,
) -> bool {
    requested
        && selection.is_some_and(|selection| owns_selection_toolbar(selection, display_bounds))
}

/// Chooses the side with the most room for the detached More panel.
pub(super) fn secondary_menu_opens_above(
    toolbar: ActionToolbarLayout,
    viewport: Bounds<Pixels>,
    menu_height: f32,
) -> bool {
    let viewport = view_rect(viewport);
    let menu_gap = secondary_action_menu_gap();
    let safe_top = viewport.top + OVERLAY_EDGE_INSET;
    let safe_bottom = viewport.bottom() - OVERLAY_BOTTOM_SAFE_INSET;
    let available_above = (toolbar.top - menu_gap - safe_top).max(0.0);
    let available_below = (safe_bottom - toolbar.top - toolbar.height - menu_gap).max(0.0);
    available_above >= menu_height
        || (available_below < menu_height && available_above > available_below)
}

/// Clamps More to the safe viewport when neither side has enough space for its full height.
pub(super) fn secondary_action_menu_top_offset(
    toolbar: ActionToolbarLayout,
    viewport: Bounds<Pixels>,
    menu_height: f32,
    opens_above: bool,
) -> f32 {
    let viewport = view_rect(viewport);
    let menu_gap = secondary_action_menu_gap();
    let safe_top = viewport.top + OVERLAY_EDGE_INSET;
    let safe_bottom = viewport.bottom() - OVERLAY_BOTTOM_SAFE_INSET;
    let preferred_top = if opens_above {
        toolbar.top - menu_gap - menu_height
    } else {
        toolbar.top + toolbar.height + menu_gap
    };
    let max_top = (safe_bottom - menu_height).max(safe_top);
    preferred_top.clamp(safe_top, max_top) - toolbar.top
}

/// Keeps the detached More panel clear of the main toolbar, including wrapped toolbar rows.
pub(super) fn secondary_action_menu_gap() -> f32 {
    OVERLAY_SECONDARY_MENU_GAP
}

/// Lifts status feedback above the fallback action bar so narrow overlays stay readable.
pub(super) fn status_bottom_inset(uses_fallback_action_bar: bool) -> f32 {
    if uses_fallback_action_bar {
        OVERLAY_BOTTOM_SAFE_INSET + OVERLAY_ACTION_ITEM_HEIGHT + OVERLAY_ACTION_BAR_GAP
    } else {
        OVERLAY_BOTTOM_SAFE_INSET
    }
}

/// Keeps status feedback in the reserved gap before a stacked style row and its action bar.
pub(super) fn status_bottom_inset_for_stacked_annotation(
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
pub(super) fn secondary_action_menu_height(
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
pub(super) fn secondary_action_widths(
    has_recognition_result: bool,
    has_recognition_retry: bool,
) -> Vec<f32> {
    let mut widths = WorkspaceMoreAction::always_visible_catalog()
        .iter()
        .map(|action| action.width())
        .collect::<Vec<_>>();
    if has_recognition_retry {
        widths.push(WorkspaceMoreAction::RetryRecognition.width());
    }
    if has_recognition_result {
        widths.extend([
            WorkspaceMoreAction::CopyRecognition.width(),
            WorkspaceMoreAction::ClearRecognition.width(),
        ]);
    }
    widths
}

/// Finds a compact preferred More-panel width that keeps its common actions to four rows.
pub(super) fn secondary_action_menu_preferred_width(
    has_recognition_result: bool,
    has_recognition_retry: bool,
) -> f32 {
    let widths = secondary_action_widths(has_recognition_result, has_recognition_retry);
    let minimum_width = widths.iter().copied().fold(0.0, f32::max)
        + OVERLAY_ACTION_BAR_HORIZONTAL_PADDING * 2.0
        + OVERLAY_ACTION_BAR_BORDER * 2.0;
    let maximum_width = widths.iter().sum::<f32>()
        + widths.len().saturating_sub(1) as f32 * OVERLAY_ACTION_ITEM_GAP
        + OVERLAY_ACTION_BAR_HORIZONTAL_PADDING * 2.0
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
pub(super) fn secondary_action_menu_width(
    viewport_width: f32,
    has_recognition_result: bool,
    has_recognition_retry: bool,
) -> f32 {
    let available_width = (viewport_width - OVERLAY_EDGE_INSET * 2.0).max(1.0);
    secondary_action_menu_preferred_width(has_recognition_result, has_recognition_retry)
        .min(available_width)
}

/// Centers the More panel near its trigger while clamping it to the overlay's safe horizontal area.
pub(super) fn secondary_action_menu_left(
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
pub(super) fn recognition_retry_label(
    locale: Locale,
    retry: super::super::RecognitionRetry,
) -> &'static str {
    match retry {
        super::super::RecognitionRetry::Ocr => locale.text(UiText::OverlayRetryOcr),
        super::super::RecognitionRetry::Translation => locale.text(UiText::OverlayRetryTranslation),
    }
}

/// Bounds visible OCR, QR, and translation text so a result never covers the screenshot controls.
pub(super) fn recognition_result_preview(text: &str) -> String {
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
pub(super) fn action_toolbar_item_widths(show_annotation_controls: bool) -> Vec<f32> {
    let inline_secondary_width = WorkspaceInlineAction::total_width(OVERLAY_ACTION_ITEM_GAP);
    if show_annotation_controls {
        let context_width =
            2.0 * ThemeMetrics::WORKSPACE_ICON_BUTTON_HIT_AREA + OVERLAY_ACTION_ITEM_GAP;
        let result_width = WorkspaceResultAction::total_width(OVERLAY_ACTION_ITEM_GAP);
        let mut widths = vec![
            OVERLAY_TOOLBAR_DRAG_HANDLE_WIDTH,
            ThemeMetrics::WORKSPACE_ICON_BUTTON_HIT_AREA,
            ThemeMetrics::WORKSPACE_SEPARATOR_WIDTH,
        ];
        widths.extend([ANNOTATION_TOOL_ICON_WIDTH; ANNOTATION_TOOL_PALETTE_ITEMS]);
        widths.extend([
            ThemeMetrics::WORKSPACE_SEPARATOR_WIDTH,
            context_width,
            ThemeMetrics::WORKSPACE_SEPARATOR_WIDTH,
            inline_secondary_width,
            ThemeMetrics::WORKSPACE_SEPARATOR_WIDTH,
            result_width,
        ]);
        widths
    } else {
        vec![
            OVERLAY_TOOLBAR_DRAG_HANDLE_WIDTH,
            ThemeMetrics::WORKSPACE_ICON_BUTTON_HIT_AREA,
            ThemeMetrics::WORKSPACE_SEPARATOR_WIDTH,
            inline_secondary_width,
            ThemeMetrics::WORKSPACE_SEPARATOR_WIDTH,
            WorkspaceResultAction::total_width(OVERLAY_ACTION_ITEM_GAP),
        ]
    }
}

/// Returns the compact item groups used when a narrow window cannot fit the full Snow-style rail.
pub(super) fn compact_action_toolbar_item_widths(show_annotation_controls: bool) -> Vec<f32> {
    if show_annotation_controls {
        let context_width =
            2.0 * ThemeMetrics::WORKSPACE_ICON_BUTTON_HIT_AREA + OVERLAY_ACTION_ITEM_GAP;
        let result_width = WorkspaceResultAction::total_width(OVERLAY_ACTION_ITEM_GAP);
        let mut widths = vec![
            OVERLAY_TOOLBAR_DRAG_HANDLE_WIDTH,
            ThemeMetrics::WORKSPACE_ICON_BUTTON_HIT_AREA,
            ThemeMetrics::WORKSPACE_SEPARATOR_WIDTH,
        ];
        widths.extend([ANNOTATION_TOOL_ICON_WIDTH; ANNOTATION_TOOL_PALETTE_ITEMS]);
        widths.extend([
            ThemeMetrics::WORKSPACE_SEPARATOR_WIDTH,
            context_width,
            ThemeMetrics::WORKSPACE_SEPARATOR_WIDTH,
            result_width,
        ]);
        widths
    } else {
        vec![
            OVERLAY_TOOLBAR_DRAG_HANDLE_WIDTH,
            ThemeMetrics::WORKSPACE_ICON_BUTTON_HIT_AREA,
            WorkspaceResultAction::total_width(OVERLAY_ACTION_ITEM_GAP),
        ]
    }
}

/// Measures the toolbar using a selected set of visible groups and shared border/padding tokens.
pub(super) fn action_toolbar_width_for(
    show_annotation_controls: bool,
    include_inline_secondary: bool,
) -> f32 {
    let widths = if include_inline_secondary {
        action_toolbar_item_widths(show_annotation_controls)
    } else {
        compact_action_toolbar_item_widths(show_annotation_controls)
    };
    widths.iter().sum::<f32>()
        + widths.len().saturating_sub(1) as f32 * OVERLAY_ACTION_ITEM_GAP
        + OVERLAY_ACTION_BAR_HORIZONTAL_PADDING * 2.0
        + OVERLAY_ACTION_BAR_BORDER * 2.0
}

/// Measures the visible main row from shared geometry instead of a per-action width table.
pub(super) fn action_toolbar_natural_width(show_annotation_controls: bool) -> f32 {
    action_toolbar_width_for(show_annotation_controls, true)
}

/// Computes wrapped-row height from the shared icon hit size when a narrow viewport requires it.
pub(super) fn action_toolbar_height(width: f32, show_annotation_controls: bool) -> f32 {
    action_toolbar_height_for(width, action_toolbar_item_widths(show_annotation_controls))
}

/// Computes the total toolbar height from wrapped rows and shared vertical tokens.
pub(super) fn action_toolbar_height_for(width: f32, widths: impl IntoIterator<Item = f32>) -> f32 {
    let rows = action_toolbar_row_count(width, widths);
    rows as f32 * OVERLAY_ACTION_ITEM_HEIGHT
        + rows.saturating_sub(1) as f32 * OVERLAY_ACTION_ITEM_GAP
        + OVERLAY_ACTION_BAR_PADDING * 2.0
        + OVERLAY_ACTION_BAR_BORDER * 2.0
}

/// Counts flex rows using the same padded content width as the rendered toolbar.
pub(super) fn action_toolbar_row_count(width: f32, widths: impl IntoIterator<Item = f32>) -> u32 {
    let mut rows = 1_u32;
    let mut row_width = 0.0;
    let content_width =
        (width - OVERLAY_ACTION_BAR_HORIZONTAL_PADDING * 2.0 - OVERLAY_ACTION_BAR_BORDER * 2.0)
            .max(1.0);
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
