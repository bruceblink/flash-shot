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
                muted: Hsla::from(rgb(0x64748b)),
                accent: Hsla::from(rgb(0x1689c7)),
                success: Hsla::from(rgb(0x168558)),
                danger: Hsla::from(rgb(0xc53b3b)),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ThemeColors, ThemeMode};

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
}
