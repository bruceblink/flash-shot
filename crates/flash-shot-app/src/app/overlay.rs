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
            toolbar_drag: None,
            manual_toolbar_position: None,
            last_workspace_snapshot: None,
            last_workspace_scale_factor: 1.0,
            topmost_requested: false,
            annotation_arrange_actions_for: None,
            _app_observation: observation,
        }
    }

    /// Clears the manual toolbar position before a fresh selection gesture begins.
    fn reset_toolbar_position(&mut self) {
        self.toolbar_drag = None;
        self.manual_toolbar_position = None;
    }

    /// Exposes the rendered toolbar bounds to the native acceptance bridge.
    ///
    /// Layout values are local to this borderless display-sized window, so translate them by the
    /// display origin before returning physical desktop coordinates.
    pub(super) fn action_toolbar_bounds_for_acceptance(&self) -> Option<PhysicalRect> {
        let toolbar = self.last_workspace_snapshot?.action_toolbar?;
        let origin = self.display.physical_bounds;
        let scale = self.last_workspace_scale_factor.max(0.01);
        Some(PhysicalRect {
            left: origin.left + (toolbar.left * scale).round() as i32,
            top: origin.top + (toolbar.top * scale).round() as i32,
            right: origin.left + ((toolbar.left + toolbar.width) * scale).round() as i32,
            bottom: origin.top + ((toolbar.top + toolbar.height) * scale).round() as i32,
        })
    }

    /// Exposes client-relative More panel bounds so native acceptance can add the HWND origin.
    pub(super) fn secondary_menu_bounds_for_acceptance(&self) -> Option<PhysicalRect> {
        let snapshot = self.last_workspace_snapshot?;
        let toolbar = snapshot.action_toolbar?;
        let menu = snapshot.secondary_menu?;
        let scale = self.last_workspace_scale_factor.max(0.01);
        let left = toolbar.left + menu.left;
        let top = toolbar.top + menu.top_offset;
        Some(PhysicalRect {
            left: (left * scale).round() as i32,
            top: (top * scale).round() as i32,
            right: ((left + menu.width) * scale).round() as i32,
            bottom: ((top + menu.height) * scale).round() as i32,
        })
    }

    /// Starts a toolbar move without allowing the full-screen selection surface to see the click.
    fn begin_toolbar_drag(
        &mut self,
        event: &MouseDownEvent,
        toolbar: Option<ActionToolbarLayout>,
        selection: Option<PhysicalRect>,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left {
            return;
        }
        let (Some(toolbar), Some(selection)) = (toolbar, selection) else {
            return;
        };
        self.toolbar_drag = Some(ToolbarDragGesture {
            pointer_start: view_point(event.position),
            toolbar_start: toolbar,
            selection,
        });
        self.manual_toolbar_position = Some(ManualToolbarPosition {
            selection,
            left: toolbar.left,
            top: toolbar.top,
        });
        cx.stop_propagation();
        cx.notify();
    }

    /// Applies one movement sample while the toolbar owns the pointer gesture.
    fn update_toolbar_drag(
        &mut self,
        event: &MouseMoveEvent,
        viewport: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if !event.dragging() {
            self.toolbar_drag = None;
            return;
        }
        let Some(gesture) = self.toolbar_drag else {
            return;
        };
        self.manual_toolbar_position = Some(toolbar_position_for_pointer(
            gesture,
            view_point(event.position),
            viewport,
        ));
        cx.stop_propagation();
        cx.notify();
    }

    /// Commits the final pointer sample and releases the toolbar gesture.
    fn finish_toolbar_drag(
        &mut self,
        event: &MouseUpEvent,
        viewport: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left || self.toolbar_drag.is_none() {
            return;
        }
        if let Some(gesture) = self.toolbar_drag {
            self.manual_toolbar_position = Some(toolbar_position_for_pointer(
                gesture,
                view_point(event.position),
                viewport,
            ));
        }
        self.toolbar_drag = None;
        cx.stop_propagation();
        cx.notify();
    }

    /// Builds one inline action from the shared catalog while keeping existing workflow handlers.
    fn inline_secondary_action_button(
        &self,
        action: WorkspaceInlineAction,
        locale: Locale,
        colors: ThemeColors,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let label = locale.text(match action {
            WorkspaceInlineAction::Scroll => UiText::OverlayScrollShot,
            WorkspaceInlineAction::Qr => UiText::OverlayQr,
            WorkspaceInlineAction::Ocr => UiText::OverlayOcr,
            WorkspaceInlineAction::Translate => UiText::OverlayTranslate,
            WorkspaceInlineAction::RecordArea => UiText::OverlayRecordArea,
            WorkspaceInlineAction::RecordWindow => UiText::OverlayRecordWindow,
        });
        let config = WorkspaceButtonConfig::icon(
            colors,
            WorkspaceButtonTone::Neutral,
            false,
            true,
            secondary_action_tooltip(locale, action.tooltip_key()),
        );
        let button = match action {
            WorkspaceInlineAction::Scroll => workspace_icon_button(
                action.id(),
                action.icon(),
                label,
                config,
                cx.listener(|this, _, _, cx| {
                    let app = this.app.clone();
                    cx.defer(move |cx| app.update(cx, |app, cx| app.start_manual_scroll(cx)));
                }),
            ),
            WorkspaceInlineAction::Qr => workspace_icon_button(
                action.id(),
                action.icon(),
                label,
                config,
                cx.listener(|this, _, _, cx| {
                    let app = this.app.clone();
                    cx.defer(move |cx| app.update(cx, |app, cx| app.recognize_qr_selection(cx)));
                }),
            ),
            WorkspaceInlineAction::Ocr => workspace_icon_button(
                action.id(),
                action.icon(),
                label,
                config,
                cx.listener(|this, _, _, cx| {
                    let app = this.app.clone();
                    cx.defer(move |cx| app.update(cx, |app, cx| app.recognize_text_selection(cx)));
                }),
            ),
            WorkspaceInlineAction::Translate => workspace_icon_button(
                action.id(),
                action.icon(),
                label,
                config,
                cx.listener(|this, _, _, cx| {
                    let app = this.app.clone();
                    cx.defer(move |cx| app.update(cx, |app, cx| app.translate_selection(cx)));
                }),
            ),
            WorkspaceInlineAction::RecordArea => workspace_icon_button(
                action.id(),
                action.icon(),
                label,
                config,
                cx.listener(|this, _, _, cx| {
                    let app = this.app.clone();
                    cx.defer(move |cx| app.update(cx, |app, cx| app.start_region_recording(cx)));
                }),
            ),
            WorkspaceInlineAction::RecordWindow => workspace_icon_button(
                action.id(),
                action.icon(),
                label,
                config,
                cx.listener(|this, _, _, cx| {
                    let app = this.app.clone();
                    cx.defer(move |cx| {
                        app.update(cx, |app, cx| app.start_selected_window_recording(cx))
                    });
                }),
            ),
        };
        button.on_key_down(stop_overlay_action_key_propagation)
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
        self.reset_toolbar_position();
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
            tools: WorkspaceAnnotationToolSpec::group_tools(group),
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
            let tools = WorkspaceAnnotationToolSpec::group_tools(group);
            let target = match direction {
                AnnotationToolGroupFocusDirection::Next => tools.iter().flatten().next().copied(),
                AnnotationToolGroupFocusDirection::Previous => {
                    tools.iter().flatten().last().copied()
                }
            };
            if let Some(target) = target
                && let Some(focus_handle) = self.annotation_tool_group_item_focus_handles
                    [group.index()]
                .iter()
                .zip(tools.iter().flatten())
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
        self.last_workspace_scale_factor = window.scale_factor();
        let show_inline_secondary_actions =
            view_rect(viewport).width >= INLINE_SECONDARY_ACTIONS_MIN_VIEWPORT_WIDTH;
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
        // Show only renderer-backed controls for the active tool. This keeps the style row
        // contextual while matching Snow Shot's selected-tool workspace without opening More.
        let show_annotation_style = show_annotation_controls && style_capabilities.has_controls();
        let visible_style_capabilities = if show_annotation_style {
            style_capabilities
        } else {
            AnnotationStyleCapabilities::EMPTY
        };
        let mut layout_snapshot = workspace_layout_snapshot(WorkspaceLayoutInput {
            selection,
            display_bounds,
            transform,
            viewport,
            hover_pixel,
            inspection_target,
            show_annotation_controls,
            annotation_toolbar_items,
            annotation_style_capabilities: visible_style_capabilities,
            annotation_tool_group,
            annotation_tool_width,
            has_recognition_result: recognition_result.is_some(),
            has_recognition_retry: recognition_retry.is_some(),
            recognition_in_flight,
        });
        if self
            .manual_toolbar_position
            .is_some_and(|position| Some(position.selection) != selection)
        {
            self.reset_toolbar_position();
        }
        if let Some(position) = self.manual_toolbar_position {
            apply_manual_toolbar_position(&mut layout_snapshot, position, viewport);
        }
        if show_more_actions && self.manual_toolbar_position.is_none() {
            position_workspace_for_more_menu(&mut layout_snapshot, selection, viewport);
        }
        self.last_workspace_snapshot = Some(layout_snapshot);
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
        let inline_secondary_action_buttons = WorkspaceInlineAction::catalog()
            .iter()
            .copied()
            .map(|action| self.inline_secondary_action_button(action, locale, workspace_colors, cx))
            .collect::<Vec<_>>();

        div()
            .size_full()
            .relative()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .on_mouse_move(cx.listener(|this, event, window, cx| {
                if this.toolbar_drag.is_some() {
                    this.update_toolbar_drag(event, local_viewport(window), cx);
                    cx.stop_propagation();
                }
            }))
            .capture_any_mouse_up(cx.listener(
                |this, event: &MouseUpEvent, window, cx| {
                    if this.toolbar_drag.is_some() {
                        this.finish_toolbar_drag(event, local_viewport(window), cx);
                    }
                },
            ))
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
                        if this.toolbar_drag.is_some() {
                            this.update_toolbar_drag(event, local_viewport(window), cx);
                            return;
                        }
                        let viewport = local_viewport(window);
                        this.update_selection(event, viewport, window, cx)
                    }))
                    .capture_any_mouse_up(cx.listener(
                        move |this, event: &MouseUpEvent, window, cx| {
                            if this.toolbar_drag.is_some() {
                                this.finish_toolbar_drag(event, local_viewport(window), cx);
                                return;
                            }
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
                            let group_tools = WorkspaceAnnotationToolSpec::group_tools(group);
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
                                    .children(group_tools.into_iter().flatten().enumerate().map(
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
                                                    .child(annotation_number_value_label(
                                                        locale, value,
                                                    )),
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
                                        let value = annotation_width_value_label(locale, width);
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
                                        let value = annotation_text_size_value_label(locale, font_size);
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
                    .px(px(OVERLAY_ACTION_BAR_HORIZONTAL_PADDING))
                    .py(px(OVERLAY_ACTION_BAR_PADDING))
                    .rounded(px(ThemeMetrics::default().radius_md))
                    .border_1()
                    .border_color(workspace_colors.toolbar_border)
                    .bg(workspace_colors.toolbar_surface)
                    .shadow_lg()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .when(can_export, |actions| {
                        let toolbar_layout_for_drag = action_layout;
                        let selection_for_drag = selection;
                        actions
                            .child(
                                workspace_drag_handle(workspace_colors).on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, event, _, cx| {
                                        this.begin_toolbar_drag(
                                            event,
                                            toolbar_layout_for_drag,
                                            selection_for_drag,
                                            cx,
                                        );
                                    }),
                                ),
                            )
                            .when(show_annotation_controls, |actions| {
                                actions.child(annotation_toolbar_icon_button(
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
                                    .children(
                                            WorkspaceAnnotationToolSpec::catalog()
                                                .iter()
                                                .copied()
                                                .map(|spec| {
                                                let tool = spec.tool();
                                                let group = spec.group();
                                                let active = selected_tool == Some(tool);
                                                let expanded = group.is_some_and(|group| {
                                                    annotation_tool_group == Some(group)
                                                });
                                                let label = locale.text(spec.label());
                                                let tooltip = annotation_tool_tooltip(locale, spec);
                                                let button = annotation_toolbar_icon_button(
                                                    format!("overlay-tool-{}", spec.id()),
                                                    spec.icon(),
                                                    label,
                                                    tooltip,
                                                    workspace_colors,
                                                    active,
                                                    cx.listener(move |this, _, window, cx| {
                                                        if active && let Some(group) = group {
                                                            this
                                                                .toggle_annotation_tool_group_from_trigger(
                                                                    group, window, cx,
                                                                );
                                                        } else {
                                                            let app = this.app.clone();
                                                            cx.defer(move |cx| {
                                                                app.update(cx, |app, cx| {
                                                                    app.select_annotation_tool(
                                                                        tool, cx,
                                                                    );
                                                                });
                                                            });
                                                        }
                                                    }),
                                                )
                                                .aria_expanded(expanded);
                                                match group {
                                                    Some(group)
                                                        if WorkspaceAnnotationToolSpec::group_tools(
                                                            group,
                                                        )
                                                        .into_iter()
                                                        .flatten()
                                                        .next()
                                                            == Some(tool) =>
                                                    {
                                                        let focus_handle = self
                                                            .annotation_tool_group_trigger_focus_handle(
                                                                group,
                                                            );
                                                        button
                                                            .track_focus(&focus_handle)
                                                            .on_key_down(cx.listener(
                                                                move |this, event, window, cx| {
                                                                    this
                                                                        .handle_annotation_tool_group_trigger_key_down(
                                                                            group, event, window,
                                                                            cx,
                                                                        );
                                                                },
                                                            ))
                                                    }
                                                    Some(group) => button.on_key_down(cx.listener(
                                                        move |this, event, window, cx| {
                                                            this
                                                                .handle_annotation_tool_group_trigger_key_down(
                                                                    group, event, window, cx,
                                                                );
                                                        },
                                                    )),
                                                    None => button,
                                                }
                                            }),
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
                                    )
                            ))
                            .when(show_annotation_controls && !show_inline_secondary_actions, |actions| {
                                actions.child(workspace_separator(
                                    "overlay-action-result-separator",
                                    workspace_colors,
                                ))
                            })
                            .when(show_inline_secondary_actions, |actions| {
                                actions
                                    .child(workspace_separator(
                                        "overlay-inline-secondary-separator",
                                        workspace_colors,
                                    ))
                                    .child(
                                        div()
                                            .id("overlay-inline-secondary-actions")
                                            .flex()
                                            .items_center()
                                            .gap(px(OVERLAY_ACTION_ITEM_GAP))
                                            .children(inline_secondary_action_buttons),
                                    )
                            })
                            .when(show_inline_secondary_actions, |actions| {
                                actions.child(workspace_separator(
                                    "overlay-inline-result-separator",
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
                                            WorkspaceResultAction::Pin.id(),
                                            WorkspaceResultAction::Pin.icon(),
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
                                            WorkspaceResultAction::Save.id(),
                                            WorkspaceResultAction::Save.icon(),
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
                                            WorkspaceResultAction::More.id(),
                                            WorkspaceResultAction::More.icon(),
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
                                            WorkspaceResultAction::Cancel.id(),
                                            WorkspaceResultAction::Cancel.icon(),
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
                                            WorkspaceResultAction::Copy.id(),
                                            WorkspaceResultAction::Copy.icon(),
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
                                            cx.listener(|this, _, window, cx| {
                                                // Copy becomes disabled while its background worker runs.
                                                // Keep keyboard cancellation routed through the overlay
                                                // instead of leaving focus on the disabled toolbar button.
                                                this.focus_handle.focus(window, cx);
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
                                            menu.w(px(layout.width))
                                                .left(px(layout.left))
                                                .top(px(layout.top_offset))
                                        })
                                        .p(px(OVERLAY_ACTION_BAR_PADDING))
                                        .flex()
                                        .flex_wrap()
                                        .justify_end()
                                        .gap(px(OVERLAY_ACTION_ITEM_GAP))
                                        .child(secondary_action_button(
                                            SecondaryAction::SaveAnnotations.id(),
                                            secondary_navigation
                                                .for_action(SecondaryAction::SaveAnnotations),
                                            locale.text(UiText::OverlaySaveAnnotations),
                                            SecondaryAction::SaveAnnotations.width(),
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
                                            SecondaryAction::SaveEditable.id(),
                                            secondary_navigation
                                                .for_action(SecondaryAction::SaveEditable),
                                            locale.text(UiText::OverlaySaveEditable),
                                            SecondaryAction::SaveEditable.width(),
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
                                            SecondaryAction::OpenAnnotations.id(),
                                            secondary_navigation
                                                .for_action(SecondaryAction::OpenAnnotations),
                                            locale.text(UiText::OverlayOpenAnnotations),
                                            SecondaryAction::OpenAnnotations.width(),
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
                                            SecondaryAction::QuickSave.id(),
                                            secondary_navigation
                                                .for_action(SecondaryAction::QuickSave),
                                            locale.text(UiText::OverlayQuickSave),
                                            SecondaryAction::QuickSave.width(),
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
                                            SecondaryAction::ScrollShot.id(),
                                            secondary_navigation
                                                .for_action(SecondaryAction::ScrollShot),
                                            locale.text(UiText::OverlayScrollShot),
                                            SecondaryAction::ScrollShot.width(),
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
                                            SecondaryAction::Qr.id(),
                                            secondary_navigation.for_action(SecondaryAction::Qr),
                                            locale.text(UiText::OverlayQr),
                                            SecondaryAction::Qr.width(),
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
                                            SecondaryAction::Ocr.id(),
                                            secondary_navigation.for_action(SecondaryAction::Ocr),
                                            locale.text(UiText::OverlayOcr),
                                            SecondaryAction::Ocr.width(),
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
                                            SecondaryAction::CopyColor.id(),
                                            secondary_navigation
                                                .for_action(SecondaryAction::CopyColor),
                                            locale.text(UiText::OverlayCopyColor),
                                            SecondaryAction::CopyColor.width(),
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
                                            SecondaryAction::Translate.id(),
                                            secondary_navigation
                                                .for_action(SecondaryAction::Translate),
                                            locale.text(UiText::OverlayTranslate),
                                            SecondaryAction::Translate.width(),
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
                                            SecondaryAction::RecordArea.id(),
                                            secondary_navigation
                                                .for_action(SecondaryAction::RecordArea),
                                            locale.text(UiText::OverlayRecordArea),
                                            SecondaryAction::RecordArea.width(),
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
                                            SecondaryAction::RecordWindow.id(),
                                            secondary_navigation
                                                .for_action(SecondaryAction::RecordWindow),
                                            locale.text(UiText::OverlayRecordWindow),
                                            SecondaryAction::RecordWindow.width(),
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
                                                SecondaryAction::RetryRecognition.id(),
                                                secondary_navigation
                                                    .for_action(SecondaryAction::RetryRecognition),
                                                retry_label,
                                                SecondaryAction::RetryRecognition.width(),
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
                                                    SecondaryAction::CopyRecognition.id(),
                                                    secondary_navigation.for_action(
                                                        SecondaryAction::CopyRecognition,
                                                    ),
                                                    locale.text(UiText::OverlayCopyText),
                                                    SecondaryAction::CopyRecognition.width(),
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
                                                    SecondaryAction::ClearRecognition.id(),
                                                    secondary_navigation.for_action(
                                                        SecondaryAction::ClearRecognition,
                                                    ),
                                                    locale.text(UiText::OverlayClearResult),
                                                    SecondaryAction::ClearRecognition.width(),
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
