//! Overlay-specific controls, focus navigation, and keyboard helpers.

use super::*;

pub(super) type SecondaryAction = WorkspaceMoreAction;

/// Lists the commands that are always rendered in the expanded More panel.
pub(super) const ALWAYS_VISIBLE_SECONDARY_ACTIONS: &[SecondaryAction; 11] =
    WorkspaceMoreAction::always_visible_catalog();

/// Owns one rendered More menu's focus order, including optional recognition commands.
#[derive(Clone)]
pub(super) struct SecondaryActionNavigation {
    pub(super) current: SecondaryAction,
    pub(super) visible_actions: Vec<SecondaryAction>,
    pub(super) focus_handles: [FocusHandle; WorkspaceMoreAction::catalog().len()],
}

impl SecondaryActionNavigation {
    /// Rebinds this navigation model to the action that receives a key event.
    pub(super) fn for_action(&self, current: SecondaryAction) -> Self {
        Self {
            current,
            visible_actions: self.visible_actions.clone(),
            focus_handles: self.focus_handles.clone(),
        }
    }

    /// Returns the persistent handle for the currently rendered action.
    pub(super) fn focus_handle(&self) -> FocusHandle {
        self.focus_handles[self.current.index()].clone()
    }

    /// Focuses the first or last visible item when the More trigger receives an arrow key.
    pub(super) fn focus_edge(
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
    pub(super) fn handle_key_down(&self, event: &KeyDownEvent, window: &mut Window, cx: &mut App) {
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

/// Keeps tool-group keyboard traversal local to the currently materialized popover.
#[derive(Clone)]
pub(super) struct AnnotationToolGroupNavigation {
    pub(super) tools: [Option<AnnotationTool>; ANNOTATION_TOOL_GROUP_MAX_ITEMS],
    pub(super) current: AnnotationTool,
    pub(super) focus_handles: [FocusHandle; ANNOTATION_TOOL_GROUP_MAX_ITEMS],
}

impl AnnotationToolGroupNavigation {
    pub(super) fn focus_handle(&self, tool: AnnotationTool) -> Option<FocusHandle> {
        self.tools
            .iter()
            .flatten()
            .position(|candidate| *candidate == tool)
            .and_then(|index| self.focus_handles.get(index).cloned())
    }

    /// Keeps arrow-key traversal inside a group while plain Enter/Space remains a button click.
    pub(super) fn handle_key_down(&self, event: &KeyDownEvent, window: &mut Window, cx: &mut App) {
        if let Some(direction) = annotation_tool_group_focus_direction(&event.keystroke) {
            if let Some(target) =
                annotation_tool_group_focus_target(self.current, &self.tools, direction)
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
pub(super) enum AnnotationToolGroupFocusDirection {
    Next,
    Previous,
}

/// Maps plain arrow keys to local tool-group focus traversal.
pub(super) fn annotation_tool_group_focus_direction(
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
pub(super) fn annotation_tool_group_focus_target(
    current: AnnotationTool,
    tools: &[Option<AnnotationTool>],
    direction: AnnotationToolGroupFocusDirection,
) -> Option<AnnotationTool> {
    let tool_count = tools.iter().flatten().count();
    if tool_count == 0 {
        return None;
    }
    let current_index = tools.iter().flatten().position(|tool| *tool == current)?;
    let target_index = match direction {
        AnnotationToolGroupFocusDirection::Next => (current_index + 1) % tool_count,
        AnnotationToolGroupFocusDirection::Previous => {
            current_index.checked_sub(1).unwrap_or(tool_count - 1)
        }
    };
    tools.iter().flatten().nth(target_index).copied()
}

/// Resolves the catalog label used by one direct action or its related-tool child.
pub(super) fn annotation_tool_ui_text(tool: AnnotationTool) -> UiText {
    WorkspaceAnnotationToolSpec::for_tool(tool).label()
}

/// Builds one fixed-size More action with a stable keyboard position and focus ring.
#[allow(clippy::too_many_arguments)]
pub(super) fn secondary_action_button(
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
pub(super) fn primary_action_tooltip(locale: Locale, action_id: &str) -> &'static str {
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
pub(super) fn more_actions_button_label(locale: Locale, show_more_actions: bool) -> &'static str {
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
pub(super) fn stop_overlay_action_key_propagation(
    event: &KeyDownEvent,
    _window: &mut Window,
    cx: &mut App,
) {
    if should_stop_overlay_action_key_propagation(&event.keystroke) {
        cx.stop_propagation();
    }
}

/// Identifies standard focused-button activation keys that must not reach overlay shortcuts.
pub(super) fn should_stop_overlay_action_key_propagation(keystroke: &Keystroke) -> bool {
    !keystroke.modifiers.modified() && matches!(keystroke.key.as_str(), "enter" | "space")
}

/// Recognizes the explicit More-menu shortcut without shadowing annotation tool shortcuts.
pub(super) fn more_actions_shortcut(keystroke: &Keystroke) -> bool {
    keystroke.key == "m"
        && keystroke.modifiers.alt
        && !keystroke.modifiers.shift
        && !keystroke.modifiers.control
        && !keystroke.modifiers.platform
        && !keystroke.modifiers.function
}

/// Toggles the smart-selection target between the containing window and the deepest control.
///
/// Alt+W stays local to the capture overlay and does not overlap the global capture bindings or
/// the annotation tool shortcuts.
pub(super) fn smart_target_toggle_shortcut(keystroke: &Keystroke) -> bool {
    keystroke.key == "w"
        && keystroke.modifiers.alt
        && !keystroke.modifiers.shift
        && !keystroke.modifiers.control
        && !keystroke.modifiers.platform
        && !keystroke.modifiers.function
}

/// Recognizes Escape while More is open so it closes the menu before cancelling the capture.
pub(super) fn close_more_actions_shortcut(keystroke: &Keystroke) -> bool {
    keystroke.key == "escape" && !keystroke.modifiers.modified()
}

/// Selects the local arrow-key movement allowed while a More action has focus.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SecondaryActionFocusDirection {
    Next,
    Previous,
}

/// Maps plain arrow keys to menu-local focus traversal without taking annotation shortcuts.
pub(super) fn secondary_action_focus_direction(
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
pub(super) fn secondary_action_focus_target(
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
pub(super) fn handle_more_actions_key_down(
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
pub(super) enum SelectionCursor {
    Crosshair,
    Move,
    ResizeNwse,
    ResizeNesw,
}

#[derive(Clone, Copy)]
pub(super) enum AnnotationActionTone {
    Neutral,
    Primary,
    Destructive,
}

/// Builds one annotation action with consistent spacing, focus feedback, semantic color, and a
/// non-interactive disabled state. Disabled controls stay visible so editing history cannot move
/// the drawing tools underneath the pointer.
pub(super) fn annotation_action_button(
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
pub(super) fn annotation_tool_button(
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

/// Builds one direct annotation action with the same icon hitbox and semantic states as workspace
/// result actions. Related tool choices remain available by activating the selected tool again.
pub(super) fn annotation_toolbar_icon_button(
    id: impl Into<gpui::ElementId>,
    icon: super::super::overlay_toolbar::WorkspaceIcon,
    label: impl Into<gpui::SharedString>,
    tooltip: impl Into<gpui::SharedString>,
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

/// Keeps repeat-activation discoverable while each drawing action remains a direct toolbar button.
pub(super) fn annotation_tool_tooltip(locale: Locale, spec: WorkspaceAnnotationToolSpec) -> String {
    let label = locale.text(spec.label());
    match spec.group() {
        Some(_) => format!(
            "{label} · {}",
            locale.text(UiText::OverlayAnnotationToolOptionsHint)
        ),
        None => label.to_owned(),
    }
}

/// Keeps acceptance selectors and accessibility ids stable as group contents evolve.
pub(super) fn annotation_tool_key(tool: AnnotationTool) -> &'static str {
    WorkspaceAnnotationToolSpec::for_tool(tool).id()
}

/// Gives each group trigger a stable semantic id independent of its localized label.
pub(super) fn annotation_tool_group_key(group: AnnotationToolGroup) -> &'static str {
    group.id()
}

/// Builds one compact style choice with the shared toolbar focus, hover, tooltip, and hitbox
/// behavior. The visible value stays short while the tooltip describes the control group.
pub(super) fn annotation_style_button(
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
