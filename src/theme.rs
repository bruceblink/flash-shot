//! Small semantic palette for the application shell.

use gpui::*;

#[derive(Clone, Copy, Debug)]
pub struct ThemeColors {
    pub background: Hsla,
    pub panel: Hsla,
    pub border: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub accent: Hsla,
    pub success: Hsla,
}

impl Default for ThemeColors {
    fn default() -> Self {
        Self {
            background: Hsla::from(rgb(0x15171b)),
            panel: Hsla::from(rgb(0x202329)),
            border: Hsla::from(rgb(0x30343b)),
            text: Hsla::from(rgb(0xf4f6f8)),
            muted: Hsla::from(rgb(0xaeb6c2)),
            accent: Hsla::from(rgb(0x4fc3f7)),
            success: Hsla::from(rgb(0x58d68d)),
        }
    }
}
