//! Shared visual primitives for the screenshot workspace.

use gpui::prelude::*;
use gpui::{App, ClickEvent, Context, Render, SharedString, Stateful, Window, div, px};
use gpui::{Div, Hsla, Role};

use crate::theme::{ThemeColors, ThemeMetrics};

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

impl WorkspaceButtonConfig {
    /// Creates an icon configuration using the workspace's stable toolbar hit height.
    pub(crate) fn icon(
        width: f32,
        colors: ThemeColors,
        tone: WorkspaceButtonTone,
        active: bool,
        enabled: bool,
        tooltip: impl Into<SharedString>,
    ) -> Self {
        Self {
            width: Some(width),
            height: ThemeMetrics::default().workspace_icon_button_hit_area,
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

/// The compact fallback glyphs used by icon-first screenshot actions.
///
/// These Unicode marks avoid a new icon-asset dependency for the native GPUI surface; accessible
/// labels and tooltips carry the complete action names for symbols that need more explanation.
pub(crate) mod icon {
    pub(crate) const MARK: &str = "✎";
    pub(crate) const PIN: &str = "⌖";
    pub(crate) const COPY: &str = "⧉";
    pub(crate) const SAVE: &str = "⇩";
    pub(crate) const MORE: &str = "⋯";
    pub(crate) const CANCEL: &str = "×";
    pub(crate) const UNDO: &str = "↶";
    pub(crate) const REDO: &str = "↷";
}

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

/// Builds a divider without making each toolbar row choose its own geometry.
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

/// Builds one icon-first workspace action with shared focus, hover, busy, and accessibility state.
pub(crate) fn workspace_icon_button(
    id: impl Into<gpui::ElementId>,
    icon: &'static str,
    aria_label: impl Into<SharedString>,
    config: WorkspaceButtonConfig,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    workspace_button(id, icon, aria_label, config, on_click)
}

/// Builds a text action for secondary menus and annotation context controls.
pub(crate) fn workspace_text_button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    config: WorkspaceButtonConfig,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let label = label.into();
    workspace_button(id, label.clone(), label, config, on_click)
}

/// Builds the shared button shell used by icon and text workspace controls.
fn workspace_button(
    id: impl Into<gpui::ElementId>,
    content: impl Into<SharedString>,
    aria_label: impl Into<SharedString>,
    config: WorkspaceButtonConfig,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let metrics = ThemeMetrics::default();
    let content = content.into();
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
    let mut button = div()
        .id(id)
        .role(Role::Button)
        .aria_label(aria_label)
        .when_some(width, |button, width| button.w(px(width)))
        .h(px(height))
        .px_3()
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
        .child(content)
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
    if !enabled {
        button = button.cursor_not_allowed();
    }
    button
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
            colors.toolbar_elevated,
            colors.text_disabled,
            colors.toolbar_border,
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
                colors.toolbar_active
            } else {
                colors.toolbar_elevated
            },
            if active {
                colors.background
            } else {
                colors.text
            },
            colors.toolbar_border,
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
