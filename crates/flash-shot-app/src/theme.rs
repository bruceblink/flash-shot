//! Small semantic palette for the application shell.

use gpui::*;

/// Persisted appearance choices shared by every native Flash Shot surface.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
}

impl ThemeMode {
    pub const fn toggled(self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::Dark,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ThemeColors {
    /// The three surfaces separate the desktop canvas from active controls without card stacking.
    pub canvas: Hsla,
    pub surface: Hsla,
    pub surface_elevated: Hsla,
    pub surface_hover: Hsla,
    /// Compatibility aliases kept while feature surfaces migrate to semantic token names.
    pub background: Hsla,
    pub panel: Hsla,
    pub border: Hsla,
    pub text: Hsla,
    pub text_muted: Hsla,
    pub muted: Hsla,
    pub text_disabled: Hsla,
    pub accent: Hsla,
    pub accent_hover: Hsla,
    pub accent_pressed: Hsla,
    pub focus: Hsla,
    pub success: Hsla,
    pub warning: Hsla,
    pub danger: Hsla,
    pub info: Hsla,
    /// Foregrounds for the dark capture scrim, which stays dark even in the light app theme.
    pub overlay_text: Hsla,
    pub overlay_muted: Hsla,
}

/// Fixed geometry tokens keep compact windows stable while allowing localized text to wrap safely.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemeMetrics {
    pub space_1: f32,
    pub space_2: f32,
    pub space_3: f32,
    pub space_4: f32,
    pub header_height: f32,
    pub status_height: f32,
    pub navigation_width: f32,
    pub row_min_height: f32,
    pub toggle_width: f32,
    pub toggle_height: f32,
    pub control_height: f32,
    pub toolbar_height: f32,
    pub radius_sm: f32,
    pub radius_md: f32,
}

impl ThemeMetrics {
    // Shared overlay and Pin geometry keeps the same interaction surface stable across views.
    pub const SPACE_1: f32 = 4.0;
    pub const SPACE_2: f32 = 8.0;
    pub const SPACE_3: f32 = 12.0;
    pub const SPACE_4: f32 = 16.0;
    pub const HEADER_HEIGHT: f32 = 76.0;
    pub const STATUS_HEIGHT: f32 = 48.0;
    pub const NAVIGATION_WIDTH: f32 = 164.0;
    pub const ROW_MIN_HEIGHT: f32 = 40.0;
    pub const TOGGLE_WIDTH: f32 = 36.0;
    pub const TOGGLE_HEIGHT: f32 = 20.0;
    pub const CONTROL_HEIGHT: f32 = 36.0;
    pub const TOOLBAR_HEIGHT: f32 = 44.0;
    pub const OVERLAY_EDGE_INSET: f32 = 18.0;
    pub const OVERLAY_BOTTOM_SAFE_INSET: f32 = 96.0;
    pub const OVERLAY_ACTION_BAR_WIDTH: f32 = 620.0;
    pub const OVERLAY_ACTION_BAR_GAP: f32 = 12.0;
    pub const OVERLAY_ACTION_ITEM_GAP: f32 = 6.0;
    pub const OVERLAY_ACTION_ITEM_HEIGHT: f32 = 36.0;
    pub const OVERLAY_ACTION_BAR_PADDING: f32 = 6.0;
    pub const OVERLAY_ACTION_BAR_BORDER: f32 = 1.0;
    pub const OVERLAY_SECONDARY_MENU_GAP: f32 = 8.0;
    pub const ANNOTATION_ACTION_HEIGHT: f32 = 32.0;
    pub const ANNOTATION_TOOL_ROW_HEIGHT: f32 = 34.0;
    pub const ANNOTATION_TOOL_GAP: f32 = 8.0;
    pub const ANNOTATION_TOOLBAR_PADDING: f32 = 4.0;
    pub const PIN_CONTROL_HEIGHT: f32 = 30.0;
    pub const PIN_TOOLBAR_PADDING: f32 = 8.0;
    pub const PIN_TOOLBAR_GAP: f32 = 8.0;
    pub const PIN_CONTROL_GAP: f32 = 4.0;
    pub const PIN_TOOLBAR_CLOSE_INSET: f32 = 48.0;
    pub const PIN_CLOSE_SIZE: f32 = 32.0;
    pub const PIN_TOP_CONTROLS_HEIGHT: f32 = 62.0;
}

impl Default for ThemeMetrics {
    fn default() -> Self {
        Self {
            space_1: Self::SPACE_1,
            space_2: Self::SPACE_2,
            space_3: Self::SPACE_3,
            space_4: Self::SPACE_4,
            header_height: Self::HEADER_HEIGHT,
            status_height: Self::STATUS_HEIGHT,
            navigation_width: Self::NAVIGATION_WIDTH,
            row_min_height: Self::ROW_MIN_HEIGHT,
            toggle_width: Self::TOGGLE_WIDTH,
            toggle_height: Self::TOGGLE_HEIGHT,
            control_height: Self::CONTROL_HEIGHT,
            toolbar_height: Self::TOOLBAR_HEIGHT,
            radius_sm: 4.0,
            radius_md: 8.0,
        }
    }
}

impl Default for ThemeColors {
    fn default() -> Self {
        Self::for_mode(ThemeMode::Dark)
    }
}

impl ThemeColors {
    pub fn for_mode(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Dark => Self {
                canvas: Hsla::from(rgb(0x0b1015)),
                surface: Hsla::from(rgb(0x121a22)),
                surface_elevated: Hsla::from(rgb(0x1a2631)),
                surface_hover: Hsla::from(rgb(0x20323f)),
                background: Hsla::from(rgb(0x0b1015)),
                panel: Hsla::from(rgb(0x121a22)),
                border: Hsla::from(rgb(0x2c3b47)),
                text: Hsla::from(rgb(0xf4f7fa)),
                text_muted: Hsla::from(rgb(0xa7b5c1)),
                muted: Hsla::from(rgb(0xa7b5c1)),
                text_disabled: Hsla::from(rgb(0x6c7a86)),
                accent: Hsla::from(rgb(0x54d6ff)),
                accent_hover: Hsla::from(rgb(0x82e2ff)),
                accent_pressed: Hsla::from(rgb(0x20b8e4)),
                focus: Hsla::from(rgb(0x9beaff)),
                success: Hsla::from(rgb(0x70e0a5)),
                warning: Hsla::from(rgb(0xffd166)),
                danger: Hsla::from(rgb(0xff7078)),
                info: Hsla::from(rgb(0x7ad8ff)),
                overlay_text: Hsla::from(rgb(0xf4f7fa)),
                overlay_muted: Hsla::from(rgb(0xa7b5c1)),
            },
            ThemeMode::Light => Self {
                canvas: Hsla::from(rgb(0xf3f7fa)),
                surface: Hsla::from(rgb(0xffffff)),
                surface_elevated: Hsla::from(rgb(0xf9fcfe)),
                surface_hover: Hsla::from(rgb(0xe8f3f7)),
                background: Hsla::from(rgb(0xf3f7fa)),
                panel: Hsla::from(rgb(0xffffff)),
                border: Hsla::from(rgb(0xd2dfe6)),
                text: Hsla::from(rgb(0x17232d)),
                text_muted: Hsla::from(rgb(0x536979)),
                muted: Hsla::from(rgb(0x536979)),
                text_disabled: Hsla::from(rgb(0x92a2ad)),
                accent: Hsla::from(rgb(0x006b8f)),
                accent_hover: Hsla::from(rgb(0x005a78)),
                accent_pressed: Hsla::from(rgb(0x004962)),
                focus: Hsla::from(rgb(0x004962)),
                success: Hsla::from(rgb(0x0b744b)),
                warning: Hsla::from(rgb(0x8a5b00)),
                danger: Hsla::from(rgb(0xb0263d)),
                info: Hsla::from(rgb(0x126b9a)),
                overlay_text: Hsla::from(rgb(0xffffff)),
                overlay_muted: Hsla::from(rgb(0xd2dfe6)),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Hsla, Rgba, rgb};

    use super::{ThemeColors, ThemeMetrics, ThemeMode};

    /// Converts an sRGB channel to linear light before WCAG luminance is calculated.
    fn linear_channel(channel: f32) -> f32 {
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    }

    /// Returns the WCAG contrast ratio for two opaque GPUI colors.
    fn contrast_ratio(left: Hsla, right: Hsla) -> f32 {
        fn luminance(color: Hsla) -> f32 {
            let color = Rgba::from(color);
            0.2126 * linear_channel(color.r)
                + 0.7152 * linear_channel(color.g)
                + 0.0722 * linear_channel(color.b)
        }

        let left = luminance(left);
        let right = luminance(right);
        (left.max(right) + 0.05) / (left.min(right) + 0.05)
    }

    #[test]
    fn theme_mode_cycles_between_dark_and_light() {
        assert_eq!(ThemeMode::Dark.toggled(), ThemeMode::Light);
        assert_eq!(ThemeMode::Light.toggled(), ThemeMode::Dark);
    }

    #[test]
    fn light_theme_uses_a_distinct_surface_palette() {
        let dark = ThemeColors::for_mode(ThemeMode::Dark);
        let light = ThemeColors::for_mode(ThemeMode::Light);
        assert_ne!(dark.background, light.background);
        assert_ne!(dark.text, light.text);
        assert_ne!(light.surface, light.surface_hover);
        assert_ne!(dark.surface, dark.surface_elevated);
    }

    #[test]
    fn destructive_actions_have_a_distinct_semantic_color() {
        for mode in [ThemeMode::Dark, ThemeMode::Light] {
            let colors = ThemeColors::for_mode(mode);
            assert_ne!(colors.danger, colors.accent);
            assert_ne!(colors.danger, colors.success);
        }
    }

    #[test]
    fn semantic_foregrounds_meet_normal_text_contrast_on_every_surface() {
        const MINIMUM_CONTRAST: f32 = 4.5;

        for mode in [ThemeMode::Dark, ThemeMode::Light] {
            let colors = ThemeColors::for_mode(mode);
            let foregrounds = [
                ("text", colors.text),
                ("muted", colors.muted),
                ("accent", colors.accent),
                ("success", colors.success),
                ("warning", colors.warning),
                ("danger", colors.danger),
                ("info", colors.info),
            ];
            let surfaces = [("background", colors.background), ("panel", colors.panel)];

            for (foreground_name, foreground) in foregrounds {
                for (surface_name, surface) in surfaces {
                    let contrast = contrast_ratio(foreground, surface);
                    assert!(
                        contrast >= MINIMUM_CONTRAST,
                        "{mode:?} {foreground_name} on {surface_name} contrast {contrast:.2} is below {MINIMUM_CONTRAST:.1}:1"
                    );
                }
            }
        }
    }

    #[test]
    fn overlay_foregrounds_remain_readable_on_the_capture_scrim() {
        let scrim = Hsla::from(rgb(0x0b0d10));
        for mode in [ThemeMode::Dark, ThemeMode::Light] {
            let colors = ThemeColors::for_mode(mode);
            assert!(contrast_ratio(colors.overlay_text, scrim) >= 4.5);
            assert!(contrast_ratio(colors.overlay_muted, scrim) >= 4.5);
        }
    }

    #[test]
    fn geometry_tokens_keep_controls_on_a_four_pixel_grid() {
        let metrics = ThemeMetrics::default();
        assert_eq!(metrics.space_1, 4.0);
        assert_eq!(metrics.space_2, 8.0);
        assert_eq!(metrics.header_height, 76.0);
        assert_eq!(metrics.status_height, 48.0);
        assert_eq!(metrics.navigation_width, 164.0);
        assert_eq!(metrics.row_min_height, 40.0);
        assert_eq!(metrics.toggle_width, 36.0);
        assert_eq!(metrics.toggle_height, 20.0);
        assert_eq!(metrics.control_height, 36.0);
        assert_eq!(metrics.toolbar_height, 44.0);
        assert!(metrics.radius_md <= 8.0);
    }

    #[test]
    fn overlay_and_pin_controls_share_stable_geometry_tokens() {
        let metrics = ThemeMetrics::default();
        assert_eq!(
            ThemeMetrics::OVERLAY_ACTION_ITEM_HEIGHT,
            metrics.control_height
        );
        assert_eq!(ThemeMetrics::PIN_CLOSE_SIZE, 32.0);
        assert_eq!(ThemeMetrics::PIN_CONTROL_GAP, metrics.space_1);
        assert_eq!(ThemeMetrics::PIN_TOOLBAR_GAP, metrics.space_2);
        assert_eq!(ThemeMetrics::PIN_CONTROL_HEIGHT, 30.0);
        assert_eq!(ThemeMetrics::PIN_TOP_CONTROLS_HEIGHT, 62.0);
    }
}
