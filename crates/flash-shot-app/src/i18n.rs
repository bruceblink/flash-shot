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
    /// Chooses a supported first-run language from the operating system without changing saved UI
    /// preferences. English remains the safe fallback when Windows does not expose a supported tag.
    pub fn system_default() -> Self {
        #[cfg(windows)]
        {
            windows_preferred_ui_language()
                .as_deref()
                .map(Self::from_system_language_tag)
                .unwrap_or(Self::English)
        }
        #[cfg(not(windows))]
        {
            Self::English
        }
    }

    /// Maps a Windows language tag to one of the application's current UI catalogs.
    pub fn from_system_language_tag(language_tag: &str) -> Self {
        let normalized = language_tag.trim().replace('_', "-").to_ascii_lowercase();
        let mut components = normalized.split('-');
        if components.next() == Some("zh")
            && components.any(|component| matches!(component, "cn" | "sg" | "my" | "hans"))
        {
            Self::SimplifiedChinese
        } else {
            Self::English
        }
    }

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

    /// Expands named placeholders in a catalog entry while keeping wording and parameter order
    /// owned by the active language instead of by individual UI views.
    pub fn format_template(self, key: UiText, replacements: &[(&str, &str)]) -> String {
        replacements
            .iter()
            .fold(self.text(key).to_owned(), |text, (name, value)| {
                text.replace(&format!("{{{name}}}"), value)
            })
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

#[cfg(windows)]
/// Reads the first Windows UI-language tag from its NUL-separated preferred-language list.
fn windows_preferred_ui_language() -> Option<String> {
    use windows_sys::Win32::Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME};

    let mut language_count = 0;
    let mut buffer_length = 0;
    // Windows fills the required UTF-16 buffer length even though this probe reports false for a
    // null buffer. A second call below owns the allocated buffer and must report success.
    unsafe {
        GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut language_count,
            std::ptr::null_mut(),
            &mut buffer_length,
        );
    }
    if language_count == 0 || buffer_length == 0 {
        return None;
    }

    let mut buffer = vec![0_u16; buffer_length as usize];
    if unsafe {
        GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut language_count,
            buffer.as_mut_ptr(),
            &mut buffer_length,
        )
    } == 0
    {
        return None;
    }
    let end = buffer.iter().position(|code_unit| *code_unit == 0)?;
    String::from_utf16(&buffer[..end]).ok()
}

/// Stable keys for application-owned interface text.
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
    PinCapture,
    PinSave,
    PinCopy,
    PinMouseThrough,
    PinSolo,
    PinShowAll,
    PinZoomOutTooltip,
    PinZoomInTooltip,
    PinOpacityTooltip,
    PinMouseThroughTooltip,
    PinSoloTooltip,
    PinShowAllTooltip,
    PinCopyTooltip,
    PinSaveTooltip,
    PinCloseTooltip,
    PinCopyingImage,
    PinWaitingClipboard,
    PinCopiedImage,
    PinCopyFailed,
    PinSavingImage,
    PinSaveBusy,
    PinZoomedIn,
    PinZoomedOut,
    PinOpacity100,
    PinOpacity75,
    PinOpacity50,
    PinOpacity25,
    PinOpacityChangeFailed,
    PinOpacityUnavailable,
    PinMouseThroughEnabled,
    PinMouseThroughDisabled,
    PinMouseThroughNotification,
    PinMouseThroughFailed,
    PinMouseThroughUnavailable,
    PinWindowHandleUnavailable,
    PinNoOtherImages,
    PinOtherImagesHidden,
    PinNoImagesToShow,
    PinAllImagesShown,
    PinSavedImage,
    PinSaveFailed,
    PinSelectionOpened,
    PinClipboardOpened,
    PinFullScreenOpened,
    PinHistoryOpened,
    PinAcceptanceOpened,
    PinSavedPreviewOpened,
    PinRenderFailed,
    PinWindowOpenFailed,
    PinClipboardFailed,
    PinFullScreenFailed,
    PinHistoryFailed,
    PinSelectionError,
    PinClipboardError,
    PinFullScreenError,
    PinHistoryError,
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
    SaveFailedKeepSelection,
    SaveFailedNeedsNewCapture,
    NoPinnedWindowsNeededInputRecovery,
    PinnedWindowInputRestored,
    ReadyWithShortcut,
    ReadySystemServicesDisabledForAcceptance,
    ReadyGlobalShortcutUnavailable,
    ReadyGlobalShortcutDisabled,
    OverlayMark,
    OverlayPin,
    OverlayCopy,
    OverlaySave,
    OverlayMore,
    OverlayLess,
    OverlayCancel,
    OverlayMarkTooltip,
    OverlayPinTooltip,
    OverlayCopyTooltip,
    OverlayCopyingTooltip,
    OverlaySaveTooltip,
    OverlayCancelTooltip,
    OverlayShowMoreTooltip,
    OverlayHideMoreTooltip,
    OverlaySaveAnnotations,
    OverlaySaveEditable,
    OverlayOpenAnnotations,
    OverlayQuickSave,
    OverlayQuickSaveTooltip,
    OverlayScrollShot,
    OverlayScrollShotTooltip,
    OverlayQr,
    OverlayQrTooltip,
    OverlayOcr,
    OverlayOcrTooltip,
    OverlayCopyColor,
    OverlayCopyColorTooltip,
    OverlayTranslate,
    OverlayTranslateTooltip,
    OverlayRecordArea,
    OverlayRecordAreaTooltip,
    OverlayRecordWindow,
    OverlayRecordWindowTooltip,
    OverlayRecognizingSelection,
    OverlayRetryOcr,
    OverlayRetryTranslation,
    OverlayRetryRecognitionTooltip,
    OverlayCopyText,
    OverlayCopyTextTooltip,
    OverlayClearResult,
    OverlayClearResultTooltip,
    OverlayLayers,
    OverlayUndo,
    OverlayRedo,
    OverlayWatermark,
    OverlayText,
    OverlayNumber,
    OverlayBlur,
    OverlayMosaic,
    OverlayHighlight,
    OverlaySelect,
    OverlayRectangle,
    OverlayEllipse,
    OverlayLine,
    OverlayArrow,
    OverlayFreehand,
    OverlaySelected,
    OverlayDelete,
    OverlayEditText,
    OverlayDuplicate,
    OverlayArrange,
    OverlayRotate90,
    OverlayBringForward,
    OverlaySendBackward,
    OverlayBringToFront,
    OverlaySendToBack,
    OverlayFill,
    LibraryQuickSave,
    LibraryFolderAccess,
    LibraryChecking,
    LibraryCheckFolder,
    LibrarySaveFolder,
    LibraryChooseFolder,
    LibraryFileName,
    LibraryOpenAndHistory,
    LibraryOpenPng,
    LibraryOpen,
    LibraryOpenProject,
    LibraryOpenFolder,
    LibrarySaveAs,
    LibraryKeepCaptures,
    LibraryUpdatingCaptures,
    LibraryRecentCaptures,
    LibrarySearchCaptures,
    LibraryFilterAll,
    LibraryFilterSelections,
    LibraryFilterScrolling,
    LibraryFilterFullScreen,
    LibraryFilterPinned,
    LibrarySelectedCount,
    LibrarySelectAllFiltered,
    LibraryClearSelection,
    LibraryDeleteSelected,
    LibraryDeleteCaptures,
    LibraryShowRecent,
    LibraryShowMore,
    LibraryDeleteFiltered,
    LibraryWorking,
    LibraryRemoving,
    LibraryRemove,
    LibraryClearing,
    LibraryClearHistory,
    LibraryEmpty,
    LibraryNoMatches,
    LibraryNoFiltered,
    LibraryClearAllConfirmation,
    LibraryClearFilteredConfirmation,
    LibraryClearSelectedConfirmation,
    LibraryShowingAll,
    LibraryShowingPreview,
    LibraryMatches,
    LibraryCaptureCount,
    LibraryFilteredCaptureCount,
    LibraryEntryFallback,
    LibrarySourceSavedCapture,
    LibrarySourceSelection,
    LibrarySourceScrolling,
    LibrarySourceFullScreen,
    LibrarySourcePinned,
    LibraryJustNow,
    LibraryMinutesAgo,
    LibraryHoursAgo,
    LibraryDaysAgo,
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
            Self::PinCapture => "Pinned capture",
            Self::PinSave => "Save",
            Self::PinCopy => "Copy",
            Self::PinMouseThrough => "Pass",
            Self::PinSolo => "Solo",
            Self::PinShowAll => "Show all",
            Self::PinZoomOutTooltip => "Zoom out (Ctrl+-)",
            Self::PinZoomInTooltip => "Zoom in (Ctrl++)",
            Self::PinOpacityTooltip => "Cycle opacity (Ctrl+O)",
            Self::PinMouseThroughTooltip => "Toggle mouse-through (Ctrl+M; restore from Actions)",
            Self::PinSoloTooltip => "Hide other pinned images (Ctrl+H)",
            Self::PinShowAllTooltip => "Show all pinned images (Ctrl+Shift+H)",
            Self::PinCopyTooltip => "Copy image (Ctrl+C)",
            Self::PinSaveTooltip => "Save image (Ctrl+S)",
            Self::PinCloseTooltip => "Close pinned image (Escape)",
            Self::PinCopyingImage => "Copying image...",
            Self::PinWaitingClipboard => "Waiting for clipboard copy",
            Self::PinCopiedImage => "Copied image",
            Self::PinCopyFailed => "Could not copy image",
            Self::PinSavingImage => "Saving image...",
            Self::PinSaveBusy => "Another pin is already saving",
            Self::PinZoomedIn => "Zoomed in",
            Self::PinZoomedOut => "Zoomed out",
            Self::PinOpacity100 => "Opacity 100%",
            Self::PinOpacity75 => "Opacity 75%",
            Self::PinOpacity50 => "Opacity 50%",
            Self::PinOpacity25 => "Opacity 25%",
            Self::PinOpacityChangeFailed => "Could not change opacity",
            Self::PinOpacityUnavailable => "Window opacity is unavailable",
            Self::PinMouseThroughEnabled => "Mouse through enabled",
            Self::PinMouseThroughDisabled => "Mouse through disabled",
            Self::PinMouseThroughNotification => {
                "Mouse-through enabled; restore pin input from Actions"
            }
            Self::PinMouseThroughFailed => "Could not change mouse-through",
            Self::PinMouseThroughUnavailable => "Mouse through is unavailable",
            Self::PinWindowHandleUnavailable => "Pinned window handle is unavailable",
            Self::PinNoOtherImages => "No other pinned images",
            Self::PinOtherImagesHidden => "Other pinned images hidden",
            Self::PinNoImagesToShow => "No pinned images to show",
            Self::PinAllImagesShown => "All pinned images shown",
            Self::PinSavedImage => "Saved image",
            Self::PinSaveFailed => "Could not save image",
            Self::PinSelectionOpened => "Selection pinned in an always-on-top window",
            Self::PinClipboardOpened => "Clipboard image pinned in an always-on-top window",
            Self::PinFullScreenOpened => "Full screen pinned in an always-on-top window",
            Self::PinHistoryOpened => "History image pinned in an always-on-top window",
            Self::PinAcceptanceOpened => "Pin lifecycle acceptance window opened",
            Self::PinSavedPreviewOpened => "Pinned saved-feedback preview opened",
            Self::PinRenderFailed => "Could not render pinned image: {error}",
            Self::PinWindowOpenFailed => "Could not open pinned window: {error}",
            Self::PinClipboardFailed => "Could not pin clipboard image",
            Self::PinFullScreenFailed => "Could not pin full screen",
            Self::PinHistoryFailed => "Could not pin history image",
            Self::PinSelectionError => "Could not pin selection: {error}",
            Self::PinClipboardError => "Could not pin clipboard image: {error}",
            Self::PinFullScreenError => "Could not pin full screen: {error}",
            Self::PinHistoryError => "Could not pin history image: {error}",
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
            Self::SaveFailedKeepSelection => {
                "Save failed: {error}. Selection kept; choose another location and try Save again."
            }
            Self::SaveFailedNeedsNewCapture => {
                "Save failed: {error}; capture is no longer editable. Start a new capture."
            }
            Self::NoPinnedWindowsNeededInputRecovery => "No pinned windows needed input recovery",
            Self::PinnedWindowInputRestored => "Restored mouse input for {count} pinned window(s)",
            Self::ReadyWithShortcut => "Ready - {shortcut}",
            Self::ReadySystemServicesDisabledForAcceptance => {
                "Ready - system services disabled for acceptance"
            }
            Self::ReadyGlobalShortcutUnavailable => "Ready - global shortcut unavailable",
            Self::ReadyGlobalShortcutDisabled => "Ready - global shortcut disabled",
            Self::OverlayMark => "Mark",
            Self::OverlayPin => "Pin",
            Self::OverlayCopy => "Copy",
            Self::OverlaySave => "Save",
            Self::OverlayMore => "More",
            Self::OverlayLess => "Less",
            Self::OverlayCancel => "Cancel",
            Self::OverlayMarkTooltip => "Show marking tools for this selection",
            Self::OverlayPinTooltip => "Pin selection",
            Self::OverlayCopyTooltip => "Copy selection to clipboard (Enter)",
            Self::OverlayCopyingTooltip => "Copying selection in the background",
            Self::OverlaySaveTooltip => "Save selection as a PNG (Ctrl+S)",
            Self::OverlayCancelTooltip => "Cancel capture (Escape)",
            Self::OverlayShowMoreTooltip => "Show more actions (Alt+M)",
            Self::OverlayHideMoreTooltip => "Hide more actions",
            Self::OverlaySaveAnnotations => "Save annotations",
            Self::OverlaySaveEditable => "Save editable",
            Self::OverlayOpenAnnotations => "Open annotations",
            Self::OverlayQuickSave => "Quick save",
            Self::OverlayQuickSaveTooltip => "Save the selection to the configured library",
            Self::OverlayScrollShot => "Scroll shot",
            Self::OverlayScrollShotTooltip => {
                "Capture a long page by scrolling and stitching viewports"
            }
            Self::OverlayQr => "QR",
            Self::OverlayQrTooltip => "Read QR codes from the selection",
            Self::OverlayOcr => "OCR",
            Self::OverlayOcrTooltip => "Recognize text locally with Tesseract",
            Self::OverlayCopyColor => "Copy color",
            Self::OverlayCopyColorTooltip => "Copy the color beneath the pointer",
            Self::OverlayTranslate => "Translate",
            Self::OverlayTranslateTooltip => {
                "Recognize text, then use the configured translation service"
            }
            Self::OverlayRecordArea => "Record area",
            Self::OverlayRecordAreaTooltip => "Start recording the selected area",
            Self::OverlayRecordWindow => "Record window",
            Self::OverlayRecordWindowTooltip => {
                "Record the visible desktop pixels of the top-level window under this selection"
            }
            Self::OverlayRecognizingSelection => "Recognizing selection...",
            Self::OverlayRetryOcr => "Retry OCR",
            Self::OverlayRetryTranslation => "Retry translation",
            Self::OverlayRetryRecognitionTooltip => "Try recognition again",
            Self::OverlayCopyText => "Copy text",
            Self::OverlayCopyTextTooltip => "Copy recognized text to the clipboard",
            Self::OverlayClearResult => "Clear result",
            Self::OverlayClearResultTooltip => "Clear the recognized result",
            Self::OverlayLayers => "Layers",
            Self::OverlayUndo => "Undo",
            Self::OverlayRedo => "Redo",
            Self::OverlayWatermark => "Watermark",
            Self::OverlayText => "Text",
            Self::OverlayNumber => "Number",
            Self::OverlayBlur => "Blur",
            Self::OverlayMosaic => "Mosaic",
            Self::OverlayHighlight => "Highlight",
            Self::OverlaySelect => "Select",
            Self::OverlayRectangle => "Rectangle",
            Self::OverlayEllipse => "Ellipse",
            Self::OverlayLine => "Line",
            Self::OverlayArrow => "Arrow",
            Self::OverlayFreehand => "Freehand",
            Self::OverlaySelected => "Selected",
            Self::OverlayDelete => "Delete",
            Self::OverlayEditText => "Edit text",
            Self::OverlayDuplicate => "Duplicate",
            Self::OverlayArrange => "Arrange",
            Self::OverlayRotate90 => "Rotate 90",
            Self::OverlayBringForward => "Forward",
            Self::OverlaySendBackward => "Backward",
            Self::OverlayBringToFront => "Front",
            Self::OverlaySendToBack => "Back",
            Self::OverlayFill => "Fill",
            Self::LibraryQuickSave => "Quick save",
            Self::LibraryFolderAccess => "Folder access",
            Self::LibraryChecking => "Checking...",
            Self::LibraryCheckFolder => "Check folder",
            Self::LibrarySaveFolder => "Save folder",
            Self::LibraryChooseFolder => "Choose folder",
            Self::LibraryFileName => "File name",
            Self::LibraryOpenAndHistory => "Open and history",
            Self::LibraryOpenPng => "Open PNG",
            Self::LibraryOpen => "Open",
            Self::LibraryOpenProject => "Open project",
            Self::LibraryOpenFolder => "Open folder",
            Self::LibrarySaveAs => "Save as {format}",
            Self::LibraryKeepCaptures => "Keep {count} captures",
            Self::LibraryUpdatingCaptures => "Updating to {count} captures...",
            Self::LibraryRecentCaptures => "Recent captures",
            Self::LibrarySearchCaptures => "Search captures",
            Self::LibraryFilterAll => "All",
            Self::LibraryFilterSelections => "Selections",
            Self::LibraryFilterScrolling => "Scrolling",
            Self::LibraryFilterFullScreen => "Full screen",
            Self::LibraryFilterPinned => "Pinned",
            Self::LibrarySelectedCount => "{count} selected",
            Self::LibrarySelectAllFiltered => "Select all filtered",
            Self::LibraryClearSelection => "Clear selection",
            Self::LibraryDeleteSelected => "Delete selected",
            Self::LibraryDeleteCaptures => "Delete captures",
            Self::LibraryShowRecent => "Show recent",
            Self::LibraryShowMore => "Show {count} more",
            Self::LibraryDeleteFiltered => "Delete {count} filtered",
            Self::LibraryWorking => "Working...",
            Self::LibraryRemoving => "Removing...",
            Self::LibraryRemove => "Remove",
            Self::LibraryClearing => "Clearing...",
            Self::LibraryClearHistory => "Clear history",
            Self::LibraryEmpty => "Saved screenshots will appear here.",
            Self::LibraryNoMatches => "No captures match \"{query}\".",
            Self::LibraryNoFiltered => "No {filter} captures yet.",
            Self::LibraryClearAllConfirmation => "Delete all {count} saved captures?",
            Self::LibraryClearFilteredConfirmation => "Delete {count} filtered saved captures?",
            Self::LibraryClearSelectedConfirmation => "Delete {count} selected saved captures?",
            Self::LibraryShowingAll => "Showing all {count} captures",
            Self::LibraryShowingPreview => "Showing {shown} of {count} captures",
            Self::LibraryMatches => "{count} match(es) for \"{query}\"",
            Self::LibraryCaptureCount => "{count} capture(s)",
            Self::LibraryFilteredCaptureCount => "{count} {filter} capture(s)",
            Self::LibraryEntryFallback => "Capture",
            Self::LibrarySourceSavedCapture => "Saved capture",
            Self::LibrarySourceSelection => "Selection",
            Self::LibrarySourceScrolling => "Scrolling screenshot",
            Self::LibrarySourceFullScreen => "Full screen",
            Self::LibrarySourcePinned => "Pinned image",
            Self::LibraryJustNow => "Just now",
            Self::LibraryMinutesAgo => "{count}m ago",
            Self::LibraryHoursAgo => "{count}h ago",
            Self::LibraryDaysAgo => "{count}d ago",
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
            Self::PinCapture => "已固定截图",
            Self::PinSave => "保存",
            Self::PinCopy => "复制",
            Self::PinMouseThrough => "穿透",
            Self::PinSolo => "单独显示",
            Self::PinShowAll => "显示全部",
            Self::PinZoomOutTooltip => "缩小（Ctrl+-）",
            Self::PinZoomInTooltip => "放大（Ctrl++）",
            Self::PinOpacityTooltip => "循环调整不透明度（Ctrl+O）",
            Self::PinMouseThroughTooltip => "切换鼠标穿透（Ctrl+M；可从操作中恢复）",
            Self::PinSoloTooltip => "隐藏其他贴图（Ctrl+H）",
            Self::PinShowAllTooltip => "显示全部贴图（Ctrl+Shift+H）",
            Self::PinCopyTooltip => "复制图片（Ctrl+C）",
            Self::PinSaveTooltip => "保存图片（Ctrl+S）",
            Self::PinCloseTooltip => "关闭贴图（Escape）",
            Self::PinCopyingImage => "正在复制图片...",
            Self::PinWaitingClipboard => "正在等待剪贴板复制",
            Self::PinCopiedImage => "图片已复制",
            Self::PinCopyFailed => "无法复制图片",
            Self::PinSavingImage => "正在保存图片...",
            Self::PinSaveBusy => "另一个贴图正在保存",
            Self::PinZoomedIn => "已放大",
            Self::PinZoomedOut => "已缩小",
            Self::PinOpacity100 => "不透明度 100%",
            Self::PinOpacity75 => "不透明度 75%",
            Self::PinOpacity50 => "不透明度 50%",
            Self::PinOpacity25 => "不透明度 25%",
            Self::PinOpacityChangeFailed => "无法调整不透明度",
            Self::PinOpacityUnavailable => "当前窗口不支持不透明度",
            Self::PinMouseThroughEnabled => "鼠标穿透已启用",
            Self::PinMouseThroughDisabled => "鼠标穿透已关闭",
            Self::PinMouseThroughNotification => "鼠标穿透已启用；可从操作中恢复贴图输入",
            Self::PinMouseThroughFailed => "无法切换鼠标穿透",
            Self::PinMouseThroughUnavailable => "当前窗口不支持鼠标穿透",
            Self::PinWindowHandleUnavailable => "无法获取贴图窗口句柄",
            Self::PinNoOtherImages => "没有其他贴图",
            Self::PinOtherImagesHidden => "其他贴图已隐藏",
            Self::PinNoImagesToShow => "没有可显示的贴图",
            Self::PinAllImagesShown => "所有贴图已显示",
            Self::PinSavedImage => "图片已保存",
            Self::PinSaveFailed => "无法保存图片",
            Self::PinSelectionOpened => "选区已固定为置顶贴图",
            Self::PinClipboardOpened => "剪贴板图片已固定为置顶贴图",
            Self::PinFullScreenOpened => "全屏截图已固定为置顶贴图",
            Self::PinHistoryOpened => "历史图片已固定为置顶贴图",
            Self::PinAcceptanceOpened => "贴图生命周期验收窗口已打开",
            Self::PinSavedPreviewOpened => "贴图保存反馈预览已打开",
            Self::PinRenderFailed => "无法渲染贴图：{error}",
            Self::PinWindowOpenFailed => "无法打开贴图窗口：{error}",
            Self::PinClipboardFailed => "无法固定剪贴板图片",
            Self::PinFullScreenFailed => "无法固定全屏截图",
            Self::PinHistoryFailed => "无法固定历史图片",
            Self::PinSelectionError => "无法固定选区：{error}",
            Self::PinClipboardError => "无法固定剪贴板图片：{error}",
            Self::PinFullScreenError => "无法固定全屏截图：{error}",
            Self::PinHistoryError => "无法固定历史图片：{error}",
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
            Self::SaveFailedKeepSelection => {
                "保存失败：{error}。已保留选区，请选择其他位置后重试保存。"
            }
            Self::SaveFailedNeedsNewCapture => {
                "保存失败：{error}；当前截图无法继续编辑，请开始新的截图。"
            }
            Self::NoPinnedWindowsNeededInputRecovery => "没有 Pin 窗口需要恢复输入",
            Self::PinnedWindowInputRestored => "已恢复 {count} 个 Pin 窗口的鼠标输入",
            Self::ReadyWithShortcut => "就绪 - {shortcut}",
            Self::ReadySystemServicesDisabledForAcceptance => "就绪 - 验收模式已禁用系统服务",
            Self::ReadyGlobalShortcutUnavailable => "就绪 - 全局快捷键不可用",
            Self::ReadyGlobalShortcutDisabled => "就绪 - 全局快捷键已禁用",
            Self::OverlayMark => "标注",
            Self::OverlayPin => "贴图",
            Self::OverlayCopy => "复制",
            Self::OverlaySave => "保存",
            Self::OverlayMore => "更多",
            Self::OverlayLess => "收起",
            Self::OverlayCancel => "取消",
            Self::OverlayMarkTooltip => "显示此选区的标注工具",
            Self::OverlayPinTooltip => "将选区固定为贴图",
            Self::OverlayCopyTooltip => "复制选区到剪贴板（Enter）",
            Self::OverlayCopyingTooltip => "正在后台复制选区",
            Self::OverlaySaveTooltip => "将选区保存为 PNG（Ctrl+S）",
            Self::OverlayCancelTooltip => "取消截图（Escape）",
            Self::OverlayShowMoreTooltip => "显示更多操作（Alt+M）",
            Self::OverlayHideMoreTooltip => "隐藏更多操作",
            Self::OverlaySaveAnnotations => "保存标注",
            Self::OverlaySaveEditable => "保存可编辑项目",
            Self::OverlayOpenAnnotations => "打开标注",
            Self::OverlayQuickSave => "快速保存",
            Self::OverlayQuickSaveTooltip => "将选区保存到已配置的图库",
            Self::OverlayScrollShot => "长截图",
            Self::OverlayScrollShotTooltip => "滚动并拼接多个视口以截取长页面",
            Self::OverlayQr => "二维码",
            Self::OverlayQrTooltip => "读取选区中的二维码",
            Self::OverlayOcr => "文字识别",
            Self::OverlayOcrTooltip => "使用 Tesseract 在本地识别文字",
            Self::OverlayCopyColor => "复制颜色",
            Self::OverlayCopyColorTooltip => "复制指针下方的颜色",
            Self::OverlayTranslate => "翻译",
            Self::OverlayTranslateTooltip => "识别文字后使用已配置的翻译服务",
            Self::OverlayRecordArea => "区域录屏",
            Self::OverlayRecordAreaTooltip => "开始录制所选区域",
            Self::OverlayRecordWindow => "窗口录屏",
            Self::OverlayRecordWindowTooltip => "录制选区内顶层窗口的可见桌面像素",
            Self::OverlayRecognizingSelection => "正在识别选区...",
            Self::OverlayRetryOcr => "重试文字识别",
            Self::OverlayRetryTranslation => "重试翻译",
            Self::OverlayRetryRecognitionTooltip => "再次尝试识别",
            Self::OverlayCopyText => "复制文字",
            Self::OverlayCopyTextTooltip => "将识别出的文字复制到剪贴板",
            Self::OverlayClearResult => "清除结果",
            Self::OverlayClearResultTooltip => "清除识别结果",
            Self::OverlayLayers => "图层",
            Self::OverlayUndo => "撤销",
            Self::OverlayRedo => "重做",
            Self::OverlayWatermark => "水印",
            Self::OverlayText => "文字",
            Self::OverlayNumber => "序号",
            Self::OverlayBlur => "模糊",
            Self::OverlayMosaic => "马赛克",
            Self::OverlayHighlight => "高亮",
            Self::OverlaySelect => "选择",
            Self::OverlayRectangle => "矩形",
            Self::OverlayEllipse => "椭圆",
            Self::OverlayLine => "直线",
            Self::OverlayArrow => "箭头",
            Self::OverlayFreehand => "画笔",
            Self::OverlaySelected => "已选中",
            Self::OverlayDelete => "删除",
            Self::OverlayEditText => "编辑文字",
            Self::OverlayDuplicate => "复制",
            Self::OverlayArrange => "排列",
            Self::OverlayRotate90 => "旋转 90°",
            Self::OverlayBringForward => "上移",
            Self::OverlaySendBackward => "下移",
            Self::OverlayBringToFront => "置顶",
            Self::OverlaySendToBack => "置底",
            Self::OverlayFill => "填充",
            Self::LibraryQuickSave => "快速保存",
            Self::LibraryFolderAccess => "目录访问",
            Self::LibraryChecking => "正在检查...",
            Self::LibraryCheckFolder => "检查目录",
            Self::LibrarySaveFolder => "保存目录",
            Self::LibraryChooseFolder => "选择目录",
            Self::LibraryFileName => "文件名",
            Self::LibraryOpenAndHistory => "打开与历史记录",
            Self::LibraryOpenPng => "打开 PNG",
            Self::LibraryOpen => "打开",
            Self::LibraryOpenProject => "打开项目",
            Self::LibraryOpenFolder => "打开目录",
            Self::LibrarySaveAs => "另存为 {format}",
            Self::LibraryKeepCaptures => "保留 {count} 张截图",
            Self::LibraryUpdatingCaptures => "正在更新为保留 {count} 张截图...",
            Self::LibraryRecentCaptures => "最近截图",
            Self::LibrarySearchCaptures => "搜索截图",
            Self::LibraryFilterAll => "全部",
            Self::LibraryFilterSelections => "选区截图",
            Self::LibraryFilterScrolling => "长截图",
            Self::LibraryFilterFullScreen => "全屏截图",
            Self::LibraryFilterPinned => "贴图",
            Self::LibrarySelectedCount => "已选择 {count} 项",
            Self::LibrarySelectAllFiltered => "全选筛选结果",
            Self::LibraryClearSelection => "清除选择",
            Self::LibraryDeleteSelected => "删除已选择项",
            Self::LibraryDeleteCaptures => "删除截图",
            Self::LibraryShowRecent => "显示最近项目",
            Self::LibraryShowMore => "再显示 {count} 项",
            Self::LibraryDeleteFiltered => "删除 {count} 项筛选结果",
            Self::LibraryWorking => "正在处理...",
            Self::LibraryRemoving => "正在移除...",
            Self::LibraryRemove => "移除",
            Self::LibraryClearing => "正在清除...",
            Self::LibraryClearHistory => "清除历史记录",
            Self::LibraryEmpty => "保存的截图会显示在这里。",
            Self::LibraryNoMatches => "没有与“{query}”匹配的截图。",
            Self::LibraryNoFiltered => "暂时没有{filter}。",
            Self::LibraryClearAllConfirmation => "删除全部 {count} 张已保存截图？",
            Self::LibraryClearFilteredConfirmation => "删除 {count} 张筛选出的已保存截图？",
            Self::LibraryClearSelectedConfirmation => "删除 {count} 张已选择的已保存截图？",
            Self::LibraryShowingAll => "正在显示全部 {count} 张截图",
            Self::LibraryShowingPreview => "正在显示 {count} 张截图中的 {shown} 张",
            Self::LibraryMatches => "“{query}”匹配到 {count} 项",
            Self::LibraryCaptureCount => "{count} 张截图",
            Self::LibraryFilteredCaptureCount => "{filter} {count} 张",
            Self::LibraryEntryFallback => "截图",
            Self::LibrarySourceSavedCapture => "已保存截图",
            Self::LibrarySourceSelection => "选区截图",
            Self::LibrarySourceScrolling => "长截图",
            Self::LibrarySourceFullScreen => "全屏截图",
            Self::LibrarySourcePinned => "贴图",
            Self::LibraryJustNow => "刚刚",
            Self::LibraryMinutesAgo => "{count} 分钟前",
            Self::LibraryHoursAgo => "{count} 小时前",
            Self::LibraryDaysAgo => "{count} 天前",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Locale, UiText};

    #[test]
    fn locale_cycles_without_losing_the_english_fallback_language() {
        assert_eq!(Locale::default(), Locale::English);
        assert_eq!(Locale::English.next(), Locale::SimplifiedChinese);
        assert_eq!(Locale::SimplifiedChinese.next(), Locale::English);
    }

    #[test]
    fn system_language_tags_select_only_supported_simplified_chinese_variants() {
        for language_tag in ["zh-CN", "zh_CN", "zh-Hans", "zh-Hans-CN", "zh-SG", "zh-MY"] {
            assert_eq!(
                Locale::from_system_language_tag(language_tag),
                Locale::SimplifiedChinese,
                "{language_tag}"
            );
        }
        for language_tag in ["en-US", "zh-TW", "zh-Hant-TW", "ja-JP", "zh"] {
            assert_eq!(
                Locale::from_system_language_tag(language_tag),
                Locale::English,
                "{language_tag}"
            );
        }
    }

    #[test]
    fn catalog_contains_distinct_shell_translations() {
        assert_eq!(Locale::English.text(UiText::Capture), "Capture");
        assert_eq!(Locale::SimplifiedChinese.text(UiText::Capture), "截图");
        assert_eq!(
            Locale::SimplifiedChinese.text(UiText::CapturePageDescription),
            "开始截图或调整截图偏好。"
        );
        assert_eq!(Locale::SimplifiedChinese.text(UiText::OverlayMark), "标注");
        assert_eq!(
            Locale::SimplifiedChinese.text(UiText::OverlayScrollShot),
            "长截图"
        );
        assert_eq!(
            Locale::SimplifiedChinese.text(UiText::OverlayRecordWindow),
            "窗口录屏"
        );
        assert_eq!(
            Locale::English.text(UiText::OverlayCopyTooltip),
            "Copy selection to clipboard (Enter)"
        );
        assert_eq!(
            Locale::SimplifiedChinese.text(UiText::OverlayFreehand),
            "画笔"
        );
        assert_eq!(
            Locale::SimplifiedChinese.text(UiText::OverlayArrange),
            "排列"
        );
        assert_eq!(
            Locale::SimplifiedChinese.text(UiText::PinMouseThrough),
            "穿透"
        );
        assert_eq!(
            Locale::SimplifiedChinese.text(UiText::PinCapture),
            "已固定截图"
        );
        assert_eq!(
            Locale::SimplifiedChinese.text(UiText::PinCopyingImage),
            "正在复制图片..."
        );
        assert_eq!(
            Locale::English.text(UiText::PinCopyTooltip),
            "Copy image (Ctrl+C)"
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
        assert_eq!(
            Locale::SimplifiedChinese
                .format_template(UiText::PinClipboardError, &[("error", "读取失败")],),
            "无法固定剪贴板图片：读取失败"
        );
    }
}
