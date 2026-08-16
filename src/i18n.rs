//! Application-owned UI language resources.
//!
//! This module is intentionally separate from `translation.rs`: that module translates captured
//! text through an optional remote service, while this catalog controls the product interface.

use serde::{Deserialize, Serialize};

/// Languages currently supported by the application chrome and settings surface.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Locale {
    #[default]
    English,
    SimplifiedChinese,
}

impl Locale {
    /// Cycles through the supported locales without changing the user's capture data.
    pub const fn next(self) -> Self {
        match self {
            Self::English => Self::SimplifiedChinese,
            Self::SimplifiedChinese => Self::English,
        }
    }

    /// Returns the language's native display name for a compact settings control.
    pub const fn label(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::SimplifiedChinese => "简体中文",
        }
    }

    /// Returns the stable evidence code used by acceptance reports, not a visible UI label.
    pub const fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::SimplifiedChinese => "zh-CN",
        }
    }

    /// Looks up one stable UI key and falls back to English when a translation is missing.
    pub const fn text(self, key: UiText) -> &'static str {
        match self {
            Self::English => key.english(),
            Self::SimplifiedChinese => key.simplified_chinese(),
        }
    }

    /// Formats the confirmation shown after the interface language changes.
    pub fn language_changed(self, language: &str) -> String {
        self.text(UiText::LanguageChanged)
            .replace("{language}", language)
    }

    /// Formats a persistence failure while preserving the operating system's error detail.
    pub fn language_preference_save_failed(self, error: &dyn std::fmt::Display) -> String {
        self.text(UiText::LanguagePreferenceSaveFailed)
            .replace("{error}", &error.to_string())
    }

    /// Formats the result of a recovery command that restores click handling for Pins.
    pub fn pinned_window_input_restored(self, count: usize) -> String {
        self.text(UiText::PinnedWindowInputRestored)
            .replace("{count}", &count.to_string())
    }

    /// Formats the idle capture status while retaining the user's configured shortcut.
    pub fn ready_with_shortcut(self, shortcut: &str) -> String {
        self.text(UiText::ReadyWithShortcut)
            .replace("{shortcut}", shortcut)
    }
}

/// Stable keys for the settings shell; workflow-specific strings will migrate in later slices.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiText {
    AppName,
    WorkspaceSubtitle,
    Workflow,
    Capture,
    CaptureDescription,
    Library,
    LibraryDescription,
    Record,
    RecordDescription,
    App,
    AppDescription,
    Screenshot,
    RegionCapture,
    FullScreen,
    FocusedWindow,
    PinRecovery,
    RestorePinInput,
    CapturePageDescription,
    LibraryPageDescription,
    RecordPageDescription,
    AppPageDescription,
    CancelDelay,
    Appearance,
    Language,
    StartWithWindows,
    Updates,
    CheckNow,
    CancelCheck,
    Dark,
    Light,
    LanguageChanged,
    LanguagePreferenceSaveFailed,
    NoPinnedWindowsNeededInputRecovery,
    PinnedWindowInputRestored,
    ReadyWithShortcut,
    ReadySystemServicesDisabledForAcceptance,
    ReadyGlobalShortcutUnavailable,
    ReadyGlobalShortcutDisabled,
}

impl UiText {
    const fn english(self) -> &'static str {
        match self {
            Self::AppName => "Flash Shot",
            Self::WorkspaceSubtitle => "Capture workspace",
            Self::Workflow => "WORKFLOW",
            Self::Capture => "Capture",
            Self::CaptureDescription => "Screenshot, annotate, export",
            Self::Library => "Library",
            Self::LibraryDescription => "Saved images and history",
            Self::Record => "Record",
            Self::RecordDescription => "Screen and audio",
            Self::App => "App",
            Self::AppDescription => "Theme, language, startup, updates",
            Self::Screenshot => "Screenshot",
            Self::RegionCapture => "Region capture",
            Self::FullScreen => "Full screen",
            Self::FocusedWindow => "Focused window",
            Self::PinRecovery => "Pin recovery",
            Self::RestorePinInput => "Restore pin input",
            Self::CapturePageDescription => "Start a screenshot or adjust capture preferences.",
            Self::LibraryPageDescription => {
                "Find saved captures, change output, and manage history."
            }
            Self::RecordPageDescription => {
                "Choose capture sources, an output folder, and recording controls."
            }
            Self::AppPageDescription => {
                "Set appearance, language, startup, and update preferences."
            }
            Self::CancelDelay => "Cancel delay",
            Self::Appearance => "Appearance",
            Self::Language => "Language",
            Self::StartWithWindows => "Start with Windows",
            Self::Updates => "Updates",
            Self::CheckNow => "Check now",
            Self::CancelCheck => "Cancel check",
            Self::Dark => "Dark",
            Self::Light => "Light",
            Self::LanguageChanged => "Language changed to {language}",
            Self::LanguagePreferenceSaveFailed => "Could not save language preference: {error}",
            Self::NoPinnedWindowsNeededInputRecovery => "No pinned windows needed input recovery",
            Self::PinnedWindowInputRestored => "Restored mouse input for {count} pinned window(s)",
            Self::ReadyWithShortcut => "Ready - {shortcut}",
            Self::ReadySystemServicesDisabledForAcceptance => {
                "Ready - system services disabled for acceptance"
            }
            Self::ReadyGlobalShortcutUnavailable => "Ready - global shortcut unavailable",
            Self::ReadyGlobalShortcutDisabled => "Ready - global shortcut disabled",
        }
    }

    const fn simplified_chinese(self) -> &'static str {
        match self {
            Self::AppName => "Flash Shot",
            Self::WorkspaceSubtitle => "截图工作区",
            Self::Workflow => "工作流",
            Self::Capture => "截图",
            Self::CaptureDescription => "截图、标注、导出",
            Self::Library => "图库",
            Self::LibraryDescription => "已保存图片与历史记录",
            Self::Record => "录屏",
            Self::RecordDescription => "屏幕与声音",
            Self::App => "应用",
            Self::AppDescription => "外观、语言、启动与更新",
            Self::Screenshot => "截图",
            Self::RegionCapture => "区域截图",
            Self::FullScreen => "全屏截图",
            Self::FocusedWindow => "焦点窗口",
            Self::PinRecovery => "Pin 恢复",
            Self::RestorePinInput => "恢复 Pin 输入",
            Self::CapturePageDescription => "开始截图或调整截图偏好。",
            Self::LibraryPageDescription => "查找已保存截图、修改输出位置并管理历史记录。",
            Self::RecordPageDescription => "选择录制来源、输出目录和录屏控制。",
            Self::AppPageDescription => "设置外观、语言、启动和更新偏好。",
            Self::CancelDelay => "取消延时",
            Self::Appearance => "外观",
            Self::Language => "语言",
            Self::StartWithWindows => "随 Windows 启动",
            Self::Updates => "更新",
            Self::CheckNow => "立即检查",
            Self::CancelCheck => "取消检查",
            Self::Dark => "深色",
            Self::Light => "浅色",
            Self::LanguageChanged => "语言已切换为 {language}",
            Self::LanguagePreferenceSaveFailed => "无法保存语言偏好：{error}",
            Self::NoPinnedWindowsNeededInputRecovery => "没有 Pin 窗口需要恢复输入",
            Self::PinnedWindowInputRestored => "已恢复 {count} 个 Pin 窗口的鼠标输入",
            Self::ReadyWithShortcut => "就绪 - {shortcut}",
            Self::ReadySystemServicesDisabledForAcceptance => "就绪 - 验收模式已禁用系统服务",
            Self::ReadyGlobalShortcutUnavailable => "就绪 - 全局快捷键不可用",
            Self::ReadyGlobalShortcutDisabled => "就绪 - 全局快捷键已禁用",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Locale, UiText};

    #[test]
    fn locale_cycles_without_losing_the_default_language() {
        assert_eq!(Locale::default(), Locale::English);
        assert_eq!(Locale::English.next(), Locale::SimplifiedChinese);
        assert_eq!(Locale::SimplifiedChinese.next(), Locale::English);
    }

    #[test]
    fn catalog_contains_distinct_shell_translations() {
        assert_eq!(Locale::English.text(UiText::Capture), "Capture");
        assert_eq!(Locale::SimplifiedChinese.text(UiText::Capture), "截图");
        assert_eq!(
            Locale::SimplifiedChinese.text(UiText::CapturePageDescription),
            "开始截图或调整截图偏好。"
        );
    }

    #[test]
    fn locale_formats_language_feedback_with_the_current_catalog() {
        assert_eq!(Locale::English.code(), "en");
        assert_eq!(Locale::SimplifiedChinese.code(), "zh-CN");
        assert_eq!(
            Locale::SimplifiedChinese.language_changed("简体中文"),
            "语言已切换为 简体中文"
        );
        assert_eq!(
            Locale::SimplifiedChinese.language_preference_save_failed(&"access denied"),
            "无法保存语言偏好：access denied"
        );
        assert_eq!(
            Locale::SimplifiedChinese.pinned_window_input_restored(2),
            "已恢复 2 个 Pin 窗口的鼠标输入"
        );
        assert_eq!(
            Locale::SimplifiedChinese.ready_with_shortcut("Ctrl+Shift+Print Screen"),
            "就绪 - Ctrl+Shift+Print Screen"
        );
    }
}
