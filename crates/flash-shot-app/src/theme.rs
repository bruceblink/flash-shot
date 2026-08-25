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
}

/// Fixed geometry tokens keep compact windows stable while allowing localized text to wrap safely.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemeMetrics {
    pub space_1: f32,
    pub space_2: f32,
    pub space_3: f32,
    pub space_4: f32,
    pub control_height: f32,
    pub toolbar_height: f32,
    pub radius_sm: f32,
    pub radius_md: f32,
}

impl Default for ThemeMetrics {
    fn default() -> Self {
        Self {
            space_1: 4.0,
            space_2: 8.0,
            space_3: 12.0,
            space_4: 16.0,
            control_height: 36.0,
            toolbar_height: 44.0,
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
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Hsla, Rgba};

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
    fn geometry_tokens_keep_controls_on_a_four_pixel_grid() {
        let metrics = ThemeMetrics::default();
        assert_eq!(metrics.space_1, 4.0);
        assert_eq!(metrics.space_2, 8.0);
        assert_eq!(metrics.control_height, 36.0);
        assert_eq!(metrics.toolbar_height, 44.0);
        assert!(metrics.radius_md <= 8.0);
    }
}
