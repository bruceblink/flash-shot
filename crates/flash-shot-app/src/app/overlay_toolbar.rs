//! Shared visual primitives for the screenshot workspace.

use gpui::prelude::*;
use gpui::{
    App, Bounds, ClickEvent, Context, PathBuilder, Pixels, Render, SharedString, Stateful, Window,
    canvas, div, point, px,
};
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

/// Identifies the small line icon rendered inside a screenshot-workspace button.
///
/// Keeping the icon geometry in one enum gives every control the same 16px canvas, stroke width,
/// and baseline. The enum is deliberately local to the workspace so it does not become a general
/// application icon dependency or change the accessible labels that already describe each action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkspaceIcon {
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
    pub(crate) const TEXT: WorkspaceIcon = WorkspaceIcon::Text;
    pub(crate) const SHAPE: WorkspaceIcon = WorkspaceIcon::Shape;
    pub(crate) const LINE: WorkspaceIcon = WorkspaceIcon::Line;
    pub(crate) const MARK: WorkspaceIcon = WorkspaceIcon::Highlight;
    pub(crate) const OBSCURE: WorkspaceIcon = WorkspaceIcon::Obscure;
    pub(crate) const PIN: WorkspaceIcon = WorkspaceIcon::Pin;
    pub(crate) const COPY: WorkspaceIcon = WorkspaceIcon::Copy;
    pub(crate) const SAVE: WorkspaceIcon = WorkspaceIcon::Save;
    pub(crate) const MORE: WorkspaceIcon = WorkspaceIcon::More;
    pub(crate) const CANCEL: WorkspaceIcon = WorkspaceIcon::Cancel;
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
                colors.toolbar_hover
            } else {
                colors.toolbar_elevated
            },
            colors.text,
            // Keep resting neutral actions visually quiet; hover, focus, and active states
            // provide the affordance without stacking a border around every button.
            if active {
                colors.toolbar_focus
            } else {
                colors.toolbar_elevated
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
    use super::{WorkspaceButtonTone, button_colors};
    use crate::theme::{ThemeColors, ThemeMode};

    #[test]
    fn resting_neutral_buttons_blend_into_the_toolbar_surface() {
        for mode in [ThemeMode::Dark, ThemeMode::Light] {
            let colors = ThemeColors::for_mode(mode);
            let (background, foreground, border) =
                button_colors(colors, WorkspaceButtonTone::Neutral, false, true);

            assert_eq!(background, colors.toolbar_elevated);
            assert_eq!(foreground, colors.text);
            assert_eq!(border, colors.toolbar_elevated);
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
}
