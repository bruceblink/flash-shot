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
        MAX_GROUP_TOOL_COUNT, WorkspaceAnnotationToolSpec, WorkspaceButtonConfig,
        WorkspaceButtonTone, WorkspaceInlineAction, WorkspaceMoreAction, WorkspaceResultAction,
        icon, workspace_drag_handle, workspace_icon_button, workspace_separator, workspace_surface,
        workspace_swatch, workspace_text_button, workspace_text_button_with_aria,
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

#[path = "overlay_paint.rs"]
mod overlay_paint;
use overlay_paint::*;
#[path = "overlay_selection.rs"]
mod overlay_selection;
use overlay_selection::*;
#[path = "overlay_layout.rs"]
mod overlay_layout;
use overlay_layout::*;
#[path = "overlay_controls.rs"]
mod overlay_controls;
use overlay_controls::*;
#[path = "overlay_input.rs"]
mod overlay_input;
use overlay_input::*;
#[path = "overlay_acceptance.rs"]
mod overlay_acceptance;
#[path = "overlay_focus.rs"]
mod overlay_focus;
use overlay_focus::*;
#[path = "overlay_events.rs"]
mod overlay_events;
#[path = "overlay_render.rs"]
mod overlay_render;
pub(in crate::app) use overlay_acceptance::open_ui_acceptance;
#[cfg(test)]
use overlay_acceptance::{
    overlay_ui_acceptance_frame, overlay_ui_acceptance_selection, overlay_ui_acceptance_target,
};

const OVERLAY_EDGE_INSET: f32 = ThemeMetrics::OVERLAY_EDGE_INSET;
// Keep fallback controls above a scaled Windows taskbar when the borderless
// overlay extends over the full display rather than the working area.
const OVERLAY_BOTTOM_SAFE_INSET: f32 = ThemeMetrics::OVERLAY_BOTTOM_SAFE_INSET;
const OVERLAY_ACTION_BAR_GAP: f32 = ThemeMetrics::WORKSPACE_SELECTION_GAP;
const OVERLAY_ACTION_ITEM_GAP: f32 = ThemeMetrics::WORKSPACE_TOOLBAR_GAP;
const OVERLAY_TOOLBAR_DRAG_HANDLE_WIDTH: f32 = ThemeMetrics::WORKSPACE_TOOLBAR_DRAG_HANDLE_WIDTH;
const OVERLAY_ACTION_ITEM_HEIGHT: f32 = ThemeMetrics::WORKSPACE_TOOLBAR_HEIGHT;
const OVERLAY_ACTION_BAR_PADDING: f32 = ThemeMetrics::WORKSPACE_TOOLBAR_VERTICAL_PADDING;
const OVERLAY_ACTION_BAR_HORIZONTAL_PADDING: f32 =
    ThemeMetrics::WORKSPACE_TOOLBAR_HORIZONTAL_PADDING;
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
// Keep the measured palette count aligned with the directly visible annotation actions.
const ANNOTATION_TOOL_PALETTE_ITEMS: usize = WorkspaceAnnotationToolSpec::catalog().len();
const ANNOTATION_TOOL_GROUP_COUNT: usize = AnnotationToolGroup::catalog().len();
const ANNOTATION_TOOL_GROUP_MAX_ITEMS: usize = MAX_GROUP_TOOL_COUNT;
const ANNOTATION_TOOL_GROUP_POPUP_PADDING: f32 = ThemeMetrics::WORKSPACE_TOOLBAR_PADDING;
const ANNOTATION_TOOL_GROUP_POPUP_BORDER: f32 = ThemeMetrics::WORKSPACE_SEPARATOR_WIDTH;
const ANNOTATION_STYLE_PANEL_GAP: f32 = ThemeMetrics::WORKSPACE_POPOVER_GAP;
const ANNOTATION_STYLE_CONTROL_HEIGHT: f32 = ThemeMetrics::WORKSPACE_STYLE_ROW_HEIGHT;
const INLINE_SECONDARY_ACTIONS_MIN_VIEWPORT_WIDTH: f32 = 640.0;
const ANNOTATION_STYLE_CONTROL_GAP: f32 = ThemeMetrics::SPACE_1;
const ANNOTATION_STYLE_ROW_PADDING: f32 = ThemeMetrics::SPACE_1;
const ANNOTATION_STYLE_VALUE_WIDTH: f32 = ThemeMetrics::WORKSPACE_STYLE_VALUE_WIDTH;
const ANNOTATION_STYLE_OPACITY_WIDTH: f32 = ThemeMetrics::WORKSPACE_STYLE_OPACITY_WIDTH;
const ANNOTATION_STYLE_FILL_WIDTH: f32 = ThemeMetrics::WORKSPACE_STYLE_FILL_WIDTH;
const ANNOTATION_LAYERS_WIDTH: f32 = 180.0;
const ANNOTATION_LAYERS_PREFERRED_HEIGHT: f32 = 200.0;
const ANNOTATION_TOOLBAR_MAX_WIDTH: f32 = 1200.0;
const ANNOTATION_COLORS: [u32; 5] = [0xFF3B30FF, 0xFFCC00FF, 0x34C759FF, 0x007AFFFF, 0xAF52DEFF];
const ANNOTATION_WIDTHS: [u32; 5] = [1, 3, 4, 6, 10];
const ANNOTATION_FONT_SIZES: [u32; 4] = [16, 24, 32, 48];
const ANNOTATION_OPACITIES: [u8; 4] = [255, 192, 128, 64];
const MAGNIFIER_RADIUS: i32 = 4;
const MAGNIFIER_CELL_SIZE: f32 = 12.0;
const MAGNIFIER_GAP: f32 = 18.0;
const MIN_ANNOTATION_VIEW_FONT_SIZE: f32 = 8.0;
const MAX_ANNOTATION_VIEW_FONT_SIZE: f32 = 96.0;

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
    secondary_action_focus_handles: [FocusHandle; WorkspaceMoreAction::catalog().len()],
    annotation_tool_group_trigger_focus_handles: [FocusHandle; ANNOTATION_TOOL_GROUP_COUNT],
    annotation_tool_group_item_focus_handles:
        [[FocusHandle; ANNOTATION_TOOL_GROUP_MAX_ITEMS]; ANNOTATION_TOOL_GROUP_COUNT],
    toolbar_drag: Option<ToolbarDragGesture>,
    manual_toolbar_position: Option<ManualToolbarPosition>,
    last_workspace_snapshot: Option<WorkspaceLayoutSnapshot>,
    last_workspace_scale_factor: f32,
    topmost_requested: bool,
    annotation_arrange_actions_for: Option<AnnotationId>,
    _app_observation: Subscription,
}

/// Accepts deferred overlay input only while it still belongs to the active workflow.
const fn accepts_overlay_input(overlay_generation: u64, current_generation: u64) -> bool {
    overlay_generation == current_generation
}

#[cfg(test)]
mod tests {
    use super::super::overlay_toolbar::{WorkspaceAnnotationToolSpec, WorkspaceIcon};
    use super::{
        ANNOTATION_TOOL_ESTIMATED_WIDTH, ANNOTATION_TOOL_GAP, ANNOTATION_TOOL_GROUP_POPUP_BORDER,
        ANNOTATION_TOOL_GROUP_POPUP_PADDING, ANNOTATION_TOOL_ROW_HEIGHT, ANNOTATION_WIDTHS,
        ActionToolbarLayout, AnnotationStyleCapabilities, AnnotationToolGroup,
        AnnotationToolbarLayout, FrameInputBatch, MAGNIFIER_CELL_SIZE, MAGNIFIER_RADIUS,
        ManualToolbarPosition, OVERLAY_ACTION_BAR_GAP, OVERLAY_ACTION_ITEM_HEIGHT,
        OVERLAY_BOTTOM_SAFE_INSET, OVERLAY_EDGE_INSET, OVERLAY_RECOGNITION_PREVIEW_LIMIT,
        OVERLAY_STATUS_ESTIMATED_HEIGHT, SecondaryAction, SecondaryActionFocusDirection,
        SelectionCursor, SelectionDimensionLayout, SmartTargetHudLayout, ToolbarDragGesture,
        WorkspaceLayoutInput, WorkspaceResultAction, WorkspaceSelectionAnchor,
        accepts_overlay_input, action_toolbar_height, action_toolbar_layout,
        action_toolbar_natural_width, action_toolbar_row_count, annotation_controls_visible,
        annotation_layer_entry_label, annotation_layer_label, annotation_number_value_label,
        annotation_opacity_value_label, annotation_style_capabilities_for_tool,
        annotation_style_row_height, annotation_style_row_preferred_width,
        annotation_style_row_width, annotation_text_size_value_label,
        annotation_tool_group_focus_direction, annotation_tool_group_focus_target,
        annotation_tool_group_popover_height, annotation_tool_group_popover_width,
        annotation_tool_key, annotation_tool_palette_width, annotation_tool_tooltip,
        annotation_toolbar_height, annotation_toolbar_items, annotation_toolbar_layout,
        annotation_toolbar_preferred_width, annotation_width_value_label,
        apply_manual_toolbar_position, arrange_context_for_selection, arrow_head_points,
        capture_double_click, close_more_actions_shortcut, intersect, is_text_annotation,
        magnifier_origin, more_actions_button_label, more_actions_shortcut, outline_shape_bounds,
        overlay_ui_acceptance_frame, overlay_ui_acceptance_selection, overlay_ui_acceptance_target,
        owns_selection_toolbar, position_workspace_for_more_menu, primary_action_tooltip,
        recognition_result_preview, recognition_retry_label, resize_handle_points,
        secondary_action_focus_direction, secondary_action_focus_target, secondary_action_menu_gap,
        secondary_action_menu_height, secondary_action_menu_left, secondary_action_menu_top_offset,
        secondary_action_menu_width, secondary_action_tooltip, secondary_action_widths,
        secondary_menu_opens_above, selection_cursor, selection_dimension_label_layout,
        selection_point_from_view_or_screen, should_stop_overlay_action_key_propagation,
        smart_target_hud_label, smart_target_hud_layout, snap_selection_pointer_to_frame_edge,
        status_bottom_inset, status_bottom_inset_for_stacked_annotation,
        toolbar_position_for_pointer, view_rect, visible_selection, workspace_layout_snapshot,
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
        assert_eq!(WorkspaceResultAction::More.id(), "overlay-more-actions");
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
    fn annotation_toolbar_exposes_every_supported_tool_with_a_stable_icon_order() {
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
            tools.map(|spec| annotation_tool_key(spec.tool())),
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
            tools.map(|spec| spec.icon()).map(WorkspaceIcon::stable_id),
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
        assert!(
            annotation_tool_tooltip(
                Locale::English,
                WorkspaceAnnotationToolSpec::for_tool(AnnotationTool::Rectangle)
            )
            .contains("Activate the selected tool again")
        );
        assert!(
            annotation_tool_tooltip(
                Locale::SimplifiedChinese,
                WorkspaceAnnotationToolSpec::for_tool(AnnotationTool::Rectangle)
            )
            .contains("再次点击当前工具")
        );
    }

    #[test]
    fn annotation_tool_groups_materialize_expected_tools_and_local_navigation() {
        assert_eq!(AnnotationToolGroup::catalog().len(), 4);
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
                &WorkspaceAnnotationToolSpec::group_tools(AnnotationToolGroup::Text),
                super::AnnotationToolGroupFocusDirection::Previous,
            ),
            Some(AnnotationTool::Number)
        );
        assert_eq!(
            annotation_tool_group_focus_target(
                AnnotationTool::Number,
                &WorkspaceAnnotationToolSpec::group_tools(AnnotationToolGroup::Text),
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
    fn annotation_numeric_style_values_use_the_active_catalog() {
        assert_eq!(annotation_width_value_label(Locale::English, 4), "4");
        assert_eq!(
            annotation_text_size_value_label(Locale::SimplifiedChinese, 24),
            "24"
        );
    }

    #[test]
    fn annotation_number_value_uses_the_active_catalog() {
        assert_eq!(annotation_number_value_label(Locale::English, 12), "12");
        assert_eq!(
            annotation_number_value_label(Locale::SimplifiedChinese, 12),
            "12"
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
        let toolbar = snapshot
            .action_toolbar
            .expect("selection should own actions");
        assert_eq!(
            toolbar,
            ActionToolbarLayout {
                left: 148.0,
                top: 258.0,
                width: 254.0,
                height: 42.0,
            }
        );
        let menu = snapshot.secondary_menu.expect("selection should own More");
        assert_eq!(menu.width, 358.0);
        assert_eq!(menu.height, 147.0);
        assert_eq!(menu.left, -104.0);
        let menu_top = toolbar.top + menu.top_offset;
        assert!(menu_top >= snapshot.safe_area.top);
        assert!(menu_top + menu.height <= snapshot.safe_area.bottom);
        assert_eq!(
            snapshot.dimension,
            Some(SelectionDimensionLayout {
                left: 242.0,
                top: 224.0,
            })
        );
    }

    #[test]
    fn workspace_snapshot_keeps_centered_more_panel_inside_a_minimum_viewport() {
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
            crate::OverlayUiAcceptanceSelectionPlacement::Centered,
        );
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
            annotation_tool_group: None,
            annotation_tool_width: ANNOTATION_TOOL_ESTIMATED_WIDTH,
            has_recognition_result: false,
            has_recognition_retry: false,
            recognition_in_flight: false,
        });
        let toolbar = snapshot
            .action_toolbar
            .expect("selection should own actions");
        let menu = snapshot.secondary_menu.expect("selection should own More");
        let menu_top = toolbar.top + menu.top_offset;
        let toolbar_bottom = toolbar.top + toolbar.height;
        let menu_is_above = menu_top + menu.height <= toolbar.top - secondary_action_menu_gap();
        let menu_is_below = menu_top >= toolbar_bottom + secondary_action_menu_gap();

        assert!(menu_top >= snapshot.safe_area.top);
        assert!(menu_top + menu.height <= snapshot.safe_area.bottom);
        assert!(menu_is_above || menu_is_below);
        assert!(toolbar.left + menu.left >= snapshot.safe_area.left);
        assert!(toolbar.left + menu.left + menu.width <= snapshot.safe_area.right);
    }

    #[test]
    fn secondary_menu_uses_the_roomier_side_when_neither_side_fits() {
        let viewport = Bounds::new(point(px(0.0), px(0.0)), size(px(420.0), px(420.0)));
        let toolbar = ActionToolbarLayout {
            left: 40.0,
            top: 115.0,
            width: 200.0,
            height: 42.0,
        };
        let menu_height = 162.0;

        assert!(!secondary_menu_opens_above(toolbar, viewport, menu_height));
        let top_offset = secondary_action_menu_top_offset(toolbar, viewport, menu_height, false);
        let menu_top = toolbar.top + top_offset;
        let safe_bottom = 420.0 - OVERLAY_BOTTOM_SAFE_INSET;
        assert!(menu_top >= OVERLAY_EDGE_INSET);
        assert!(menu_top >= toolbar.top + toolbar.height);
        assert!(menu_top + menu_height <= safe_bottom);
    }

    #[test]
    fn expanded_more_repositions_wrapped_toolbar_without_covering_its_actions() {
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
        let mut input = workspace_layout_input(selection, bounds, transform, viewport);
        input.show_annotation_controls = true;
        input.annotation_style_capabilities =
            annotation_style_capabilities_for_tool(AnnotationTool::Rectangle);
        let mut snapshot = workspace_layout_snapshot(input);
        let initial_toolbar = snapshot
            .action_toolbar
            .expect("bottom-right selection should own actions");
        assert!(snapshot.secondary_menu.is_some());
        assert!(initial_toolbar.height > OVERLAY_ACTION_ITEM_HEIGHT);

        position_workspace_for_more_menu(&mut snapshot, Some(selection), viewport);

        let toolbar = snapshot
            .action_toolbar
            .expect("repositioned workspace should retain its actions");
        let menu = snapshot
            .secondary_menu
            .expect("repositioned workspace should retain More");
        let menu_top = toolbar.top + menu.top_offset;
        let toolbar_bottom = toolbar.top + toolbar.height;
        assert_ne!(toolbar.top, initial_toolbar.top);
        assert!(toolbar.top >= snapshot.safe_area.top);
        assert!(toolbar_bottom <= snapshot.safe_area.bottom);
        assert!(menu_top >= snapshot.safe_area.top);
        assert!(menu_top + menu.height <= snapshot.safe_area.bottom);
        if menu.opens_above {
            assert!(menu_top + menu.height <= toolbar.top - secondary_action_menu_gap());
        } else {
            assert!(menu_top >= toolbar_bottom + secondary_action_menu_gap());
        }
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
    fn toolbar_drag_clamps_to_the_overlay_safe_area() {
        let viewport = Bounds::new(point(px(0.0), px(0.0)), size(px(1280.0), px(720.0)));
        let selection = PhysicalRect {
            left: 200,
            top: 180,
            right: 640,
            bottom: 420,
        };
        let gesture = ToolbarDragGesture {
            pointer_start: ViewPoint { x: 240.0, y: 220.0 },
            toolbar_start: ActionToolbarLayout {
                left: 200.0,
                top: 220.0,
                width: 320.0,
                height: 50.0,
            },
            selection,
        };

        let top_left = toolbar_position_for_pointer(
            gesture,
            ViewPoint {
                x: -800.0,
                y: -800.0,
            },
            viewport,
        );
        assert_eq!(top_left.left, OVERLAY_EDGE_INSET);
        assert_eq!(top_left.top, OVERLAY_EDGE_INSET);

        let bottom_right = toolbar_position_for_pointer(
            gesture,
            ViewPoint {
                x: 2_000.0,
                y: 2_000.0,
            },
            viewport,
        );
        assert_eq!(
            bottom_right.left,
            1280.0 - OVERLAY_EDGE_INSET - gesture.toolbar_start.width
        );
        assert_eq!(
            bottom_right.top,
            720.0 - OVERLAY_BOTTOM_SAFE_INSET - gesture.toolbar_start.height
        );
        assert_eq!(bottom_right.selection, selection);
    }

    #[test]
    fn manual_toolbar_position_moves_attached_workspace_surfaces_together() {
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
            right: 1_000,
            bottom: 400,
        };
        let mut snapshot = workspace_layout_snapshot(WorkspaceLayoutInput {
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
        let base = snapshot.action_toolbar.unwrap();
        let manual = ManualToolbarPosition {
            selection,
            left: base.left + 120.0,
            top: base.top + 24.0,
        };

        apply_manual_toolbar_position(&mut snapshot, manual, viewport);

        let moved = snapshot.action_toolbar.unwrap();
        assert_eq!(moved.left - base.left, 120.0);
        assert_eq!(moved.top - base.top, 24.0);
        assert_eq!(snapshot.annotation_toolbar.unwrap().action_toolbar, moved);
        let group = snapshot.annotation_tool_group.unwrap();
        let base_group = workspace_layout_snapshot(WorkspaceLayoutInput {
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
        })
        .annotation_tool_group
        .unwrap();
        assert_eq!(group.left - base_group.left, 120.0);
        assert_eq!(group.top - base_group.top, 24.0);
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
                left: 728.0,
                top: 526.0,
                width: 472.0,
                height: 42.0,
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
                left: 148.0,
                top: 258.0,
                width: 254.0,
                height: 42.0,
            }
        );
        assert!(toolbar.top + toolbar.height + OVERLAY_ACTION_BAR_GAP <= selection.top as f32);

        let menu_width =
            secondary_action_menu_width(super::view_rect(viewport).width, false, false);
        assert_eq!(menu_width, 358.0);
        let menu_height = secondary_action_menu_height(menu_width, false, false, false);
        assert_eq!(menu_height, 147.0);
        assert!(secondary_menu_opens_above(toolbar, viewport, menu_height));
        let menu_top = toolbar.top - secondary_action_menu_gap() - menu_height;
        assert!(menu_top >= OVERLAY_EDGE_INSET);

        assert_eq!(
            selection_dimension_label_layout(
                workspace_anchor(selection, transform),
                viewport,
                Some(toolbar),
            ),
            Some(SelectionDimensionLayout {
                left: 242.0,
                top: 224.0,
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
                left: 1607.0,
                top: 1278.0,
                width: 935.0,
                height: 42.0,
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
        assert_eq!(marking.left, 1607.0);
        assert_eq!(marking.top, 1278.0);
        assert_eq!(marking.width, 935.0);
        assert_eq!(marking.height, 42.0);
        assert_eq!(marking.tools_width, 935.0);
        assert_eq!(marking.tools_top, 1278.0);
        assert_eq!(marking.style_left, marking.left);
        assert_eq!(marking.style_top, 1320.0);
        assert_eq!(marking.action_toolbar.left, primary.left);
        assert_eq!(marking.action_toolbar.top, 1278.0);
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

        assert_eq!(action_toolbar_height(324.0, false), 77.0);
        assert_eq!(action_toolbar_height(288.0, false), 112.0);
        assert_eq!(action_toolbar_natural_width(false), 472.0);
        assert_eq!(action_toolbar_natural_width(true), 935.0);
        assert_eq!(action_toolbar_height(358.0, true), 147.0);
        assert_eq!(action_toolbar_height(559.0, true), 77.0);
        assert_eq!(action_toolbar_height(587.0, true), 77.0);
        assert_eq!(action_toolbar_height(846.0, true), 77.0);
        assert_eq!(secondary_action_menu_width(420.0, false, false), 358.0);
        assert_eq!(
            action_toolbar_row_count(368.0, secondary_action_widths(false, false)),
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
            -68.0
        );
        assert_eq!(layout.left, 86.0);
        assert!((layout.top - 346.0).abs() < 0.01);
        assert_eq!(layout.width, 254.0);
        assert_eq!(layout.height, 42.0);
        assert_eq!(
            secondary_action_menu_height(324.0, false, false, false),
            182.0
        );
        assert_eq!(
            secondary_action_menu_height(324.0, true, false, false),
            284.0
        );
        assert_eq!(
            secondary_action_menu_height(324.0, false, true, false),
            182.0
        );
        assert_eq!(
            secondary_action_menu_height(324.0, false, false, true),
            215.0
        );
        assert_eq!(
            secondary_action_widths(true, true),
            vec![
                SecondaryAction::SaveAnnotations.width(),
                SecondaryAction::SaveEditable.width(),
                SecondaryAction::OpenAnnotations.width(),
                SecondaryAction::QuickSave.width(),
                SecondaryAction::ScrollShot.width(),
                SecondaryAction::Qr.width(),
                SecondaryAction::Ocr.width(),
                SecondaryAction::CopyColor.width(),
                SecondaryAction::Translate.width(),
                SecondaryAction::RecordArea.width(),
                SecondaryAction::RecordWindow.width(),
                SecondaryAction::RetryRecognition.width(),
                SecondaryAction::CopyRecognition.width(),
                SecondaryAction::ClearRecognition.width(),
            ]
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
        assert_eq!(annotation_toolbar_height(wide, stable_items), 42.0);
        assert_eq!(annotation_toolbar_height(narrow, stable_items), 78.0);
        assert_eq!(annotation_toolbar_height(narrow, selected_items), 251.0);
        assert!(annotation_toolbar_height(narrow, expanded_items) > 251.0);
    }

    #[test]
    fn annotation_dock_grows_only_for_visible_controls() {
        let empty_style = AnnotationStyleCapabilities::EMPTY;
        let shape_style = annotation_style_capabilities_for_tool(AnnotationTool::Rectangle);
        let empty_items = annotation_toolbar_items(false, false, false, false, false);

        assert_eq!(annotation_tool_palette_width(), 402.0);
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
            102.0
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
