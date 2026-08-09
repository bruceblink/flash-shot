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
    pub background: Hsla,
    pub panel: Hsla,
    pub border: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub accent: Hsla,
    pub success: Hsla,
    pub danger: Hsla,
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
                background: Hsla::from(rgb(0x15171b)),
                panel: Hsla::from(rgb(0x202329)),
                border: Hsla::from(rgb(0x30343b)),
                text: Hsla::from(rgb(0xf4f6f8)),
                muted: Hsla::from(rgb(0xaeb6c2)),
                accent: Hsla::from(rgb(0x4fc3f7)),
                success: Hsla::from(rgb(0x58d68d)),
                danger: Hsla::from(rgb(0xef6461)),
            },
            ThemeMode::Light => Self {
                background: Hsla::from(rgb(0xf7f8fa)),
                panel: Hsla::from(rgb(0xffffff)),
                border: Hsla::from(rgb(0xd9dee5)),
                text: Hsla::from(rgb(0x1b2430)),
                muted: Hsla::from(rgb(0x5f6f84)),
                accent: Hsla::from(rgb(0x0f75b5)),
                success: Hsla::from(rgb(0x117a4f)),
                danger: Hsla::from(rgb(0xc53b3b)),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Hsla, Rgba};

    use super::{ThemeColors, ThemeMode};

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
                ("danger", colors.danger),
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
}
