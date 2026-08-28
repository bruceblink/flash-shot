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
    RecognitionBusy,
    RecognitionSelectAreaQr,
    RecognitionQrInProgress,
    RecognitionQrNone,
    RecognitionQrFound,
    RecognitionQrCode,
    RecognitionQrCodes,
    RecognitionQrFailed,
    RecognitionSelectAreaText,
    RecognitionTextInProgress,
    RecognitionTextNone,
    RecognitionTextCompleted,
    RecognitionTextTitle,
    RecognitionOcrUnavailable,
    RecognitionOcrFailed,
    RecognitionSelectAreaTranslate,
    TranslationDisabled,
    TranslationUnavailable,
    TranslationInProgress,
    TranslationNoText,
    TranslationCompleted,
    TranslationPreparationFailed,
    TranslationOcrFailed,
    TranslationServiceFailed,
    TranslationSupportReady,
    TranslationSupportDisabled,
    TranslationSupportNeedsAttention,
    TranslationServiceReady,
    TranslationServiceNoText,
    OcrSupportReady,
    OcrSupportLanguageMissing,
    OcrSupportUnavailable,
    OcrSupportCheckFailed,
    OcrSupportCheckBusy,
    OcrSupportCheckInProgress,
    OcrSupportCheck,
    OcrLanguageAutomatic,
    OcrLanguageEnglish,
    OcrLanguageSimplifiedChinese,
    OcrLanguageEnglishSimplifiedChinese,
    OcrLanguageChanged,
    OcrLanguageSaveFailed,
    TranslationServiceTestBusy,
    TranslationServiceTestInProgress,
    TranslationServiceTestCancelled,
    TranslationServiceTest,
    TranslationServiceCancelTest,
    SettingsLocalOcr,
    SettingsTranslation,
    RecordingSupportCheckInProgress,
    RecordingStoppingAlready,
    RecordingStopFailed,
    RecordingWaitDirectoryCheck,
    RecordingFinishScreenshotFirst,
    RecordingPreparingDisplay,
    RecordingStartupCancelled,
    RecordingSupportCheckBusy,
    RecordingStopBeforeSupportCheck,
    RecordingSupportCheckCancelled,
    RecordingDirectoryControlled,
    RecordingWaitBeforeDirectoryChange,
    RecordingChooseDirectory,
    RecordingChooseDirectoryPrompt,
    RecordingDirectorySaved,
    RecordingDirectorySaveFailed,
    RecordingDirectoryUseFailed,
    RecordingDirectoryUnchanged,
    RecordingDirectoryDefaultAlready,
    RecordingDirectoryReset,
    RecordingDirectoryResetPath,
    RecordingDirectoryResetFailed,
    RecordingWaitBeforeDirectoryCheck,
    RecordingDirectoryCheckInProgress,
    RecordingDirectoryReady,
    RecordingDirectoryCheckFailed,
    RecordingDirectoryOpened,
    RecordingDirectoryOpenFailed,
    RecordingSelectRegion,
    RecordingSupportCheckBeforeStart,
    RecordingPreparingRegion,
    RecordingSelectWindow,
    RecordingResolvingWindow,
    RecordingDisplayDiscoveryInProgress,
    RecordingDisplayChanged,
    RecordingDisplayDiscoveryFailed,
    RecordingAudioDiscoveryInProgress,
    RecordingAudioChanged,
    RecordingAudioDiscoveryFailed,
    RecordingPausing,
    RecordingResuming,
    RecordingPauseFailed,
    RecordingStarting,
    RecordingActive,
    RecordingPaused,
    RecordingProgress,
    RecordingStopping,
    RecordingSaved,
    RecordingSavedNotification,
    RecordingFailed,
    RecordingTargetScreen,
    RecordingTargetDisplay,
    RecordingTargetWindow,
    RecordingTargetRegion,
    RecordingAudioAutomatic,
    RecordingAudioDisabled,
    RecordingAudioMicrophone,
    RecordingAudioSystem,
    RecordingDisplayPrimary,
    RecordingDisplayLabel,
    RecordingStartFailureMissingFfmpeg,
    RecordingStartFailureUnsupported,
    RecordingStartFailureGeneric,
    RecordingStartConflictStopping,
    RecordingStartConflictActive,
    RecordingStartConflictStarting,
    RecordingDiscoveryConflict,
    RecordingSupportCheckConflict,
    RecordingSupportReady,
    RecordingSupportDesktopUnavailable,
    RecordingSettingsDisplay,
    RecordingSettingsAudio,
    RecordingSettingsVideoFolder,
    RecordingFolderUnavailable,
    RecordingChooseFolderAction,
    RecordingCheckFolderAction,
    RecordingOpenFolderAction,
    RecordingUseDefaultFolderAction,
    RecordingCheckSupportAction,
    RecordingCancelCheckAction,
    RecordingCancelStartAction,
    RecordingStoppingAction,
    RecordingDiscoveringAction,
    RecordingCheckingFolderAction,
    RecordingStopAction,
    RecordingRecordDisplayAction,
    RecordingPauseAction,
    RecordingResumeAction,
    RecordingStatusLabel,
    RecordingProgressPreparing,
    RecordingProgressStopping,
    RecordingProgressIdle,
    RecordingStateActive,
    RecordingStatePaused,
    RecordingProgressSummary,
    RecognitionResultCopied,
    RecognitionResultCopyFailed,
    CapturePageDescription,
    LibraryPageDescription,
    RecordPageDescription,
    AppPageDescription,
    CapturePreferences,
    GlobalShortcut,
    IncludeCursor,
    Shortcut,
    FullScreenKey,
    FocusedWindowKey,
    CaptureDelay,
    CaptureDelayOff,
    ColorCopyFormat,
    RegisteredShortcut,
    DisabledShortcut,
    CaptureDelayDisabled,
    CaptureDelaySet,
    CaptureDelaySaveFailed,
    CaptureCursorSaveFailed,
    CaptureCursorIncluded,
    CaptureCursorOmitted,
    ColorCopyFormatChanged,
    ColorCopyFormatSaveFailed,
    ExportFormatChanged,
    ExportFormatSaveFailed,
    SettingsOpenFailed,
    DelayedCaptureScheduled,
    DelayedCaptureCancelled,
    CaptureStarting,
    CaptureSummary,
    CaptureFocusedWindow,
    CaptureFocusedWindowUnavailable,
    CaptureFailed,
    CaptureAnnotationDocumentCreateFailed,
    CaptureRecordingStoppingConflict,
    CaptureRecordingActiveConflict,
    CaptureRecordingStartingConflict,
    AnnotationResizing,
    AnnotationMoving,
    SelectionMoving,
    SelectionDimensions,
    SelectionDimensionLabel,
    SelectionHoverDetails,
    HoverPixelDetails,
    FrameDimensions,
    SmartTargetDetails,
    SmartTargetLabel,
    InspectionControl,
    InspectionWindow,
    OverlaySmartTargetReady,
    OverlaySelectionReady,
    OverlaySeedSelectionFailed,
    ScrollingScreenshot,
    ScrollingSelectArea,
    ScrollingStartFailed,
    ScrollingAlreadyActive,
    ScrollingWaitForFinish,
    ScrollingReady,
    ScrollingNotActive,
    ScrollingNotCollecting,
    ScrollingFrameCaptureBusy,
    ScrollingCapturingNextFrame,
    ScrollingAssistFailed,
    ScrollingSettling,
    ScrollingFrameCaptured,
    ScrollingFrameCaptureFailed,
    ScrollingWaitForFrame,
    ScrollingNeedAnotherFrame,
    ScrollingFinishFailed,
    ScrollingStitching,
    ScrollingStitched,
    ScrollingOpenFailed,
    ScrollingCancelled,
    ScrollingNoNewContentFinish,
    ScrollingNoNewContentRetry,
    ScrollingOverlapMismatch,
    ScrollingCaptureView,
    ScrollingRetryView,
    ScrollingCaptureInProgress,
    ScrollingScrolling,
    ScrollingScrollDownCapture,
    ScrollingFinish,
    ScrollingNotReady,
    ScrollingNoFrames,
    ScrollingOneFrame,
    ScrollingManyFrames,
    ScrollingReadyToFinish,
    ScrollingCaptureAnother,
    CancelDelay,
    Appearance,
    Language,
    StartWithWindows,
    Updates,
    CheckNow,
    CancelCheck,
    UpdateCheckBusy,
    UpdateChecksDisabled,
    UpdateChecksUnavailable,
    UpdateCheckInProgress,
    UpdateCheckCancelled,
    UpdateAvailable,
    UpdateCurrent,
    UpdateNewerLocal,
    UpdateCheckFailed,
    Dark,
    Light,
    LanguageChanged,
    LanguagePreferenceSaveFailed,
    SaveFailedKeepSelection,
    SaveFailedNeedsNewCapture,
    HistoryRecordFailed,
    HistoryFilesRemovedIndexFailed,
    HistoryRetentionUpdateFailed,
    HistoryRetentionUpdating,
    HistoryRetentionDeleteFailed,
    HistoryRetentionUpdated,
    HistoryRetentionSaveFailed,
    HistoryFallbackUsed,
    HistoryFallbackPreferenceFailed,
    QuickSaveFolderBusy,
    QuickSaveFolderChoosing,
    QuickSaveFolderPrompt,
    QuickSaveFolderSelectionCancelled,
    QuickSaveFolderChanged,
    QuickSaveFolderPreferenceSaveFailed,
    QuickSaveFolderUseFailed,
    QuickSaveFolderChecking,
    QuickSaveFolderReady,
    QuickSaveFolderCheckFailed,
    QuickSavePrefixChanged,
    QuickSavePrefixSaveFailed,
    HistorySelectionRemoved,
    HistorySelectionCount,
    HistoryNoMatches,
    HistoryAlreadySelected,
    HistorySelectionAdded,
    HistorySelectionCleared,
    HistorySelectAtLeastOne,
    HistoryAlreadyEmpty,
    HistoryClearConfirmation,
    HistoryClearScopeAll,
    HistoryClearScopeFiltered,
    HistoryClearScopeSelected,
    HistoryClearCancelled,
    HistoryWaitingForReads,
    HistoryClearing,
    HistoryCleared,
    HistoryDeletedSelected,
    HistoryClearedFiltered,
    HistoryClearedWithFailures,
    HistoryRemoving,
    HistoryRemoveFailed,
    HistoryRemoved,
    HistoryFolderOpened,
    HistoryFolderOpenFailed,
    HistoryUnavailableNoParent,
    HistoryUnavailable,
    SaveStartFailed,
    SaveDialogAboveCaptureFailed,
    SaveSelectionChoosing,
    SaveHistoryBusy,
    SaveSelectionInProgress,
    SaveCompleted,
    SaveTransitionFailed,
    NotificationScreenshotSaved,
    PinnedSaveBusy,
    PinnedSaveWaitingForHistory,
    PinnedSaveInProgress,
    PinnedImageSavedTo,
    PinnedImageSaveFailed,
    NotificationPinnedImageSaved,
    SelectionCopiedToClipboard,
    CopyCancelledBeforeClipboardChanged,
    CopyFailed,
    NotificationScreenshotCopied,
    FullScreenCopyInProgress,
    FullScreenCopiedToClipboard,
    FullScreenCopyFailed,
    NotificationFullScreenCopied,
    FullScreenSaveInProgress,
    FullScreenSavedTo,
    FullScreenSaveFailed,
    NotificationFullScreenSaved,
    FullScreenPinInProgress,
    ClipboardBusy,
    ClipboardActionSelection,
    ClipboardActionRecognizedText,
    ClipboardActionColor,
    ClipboardActionFullScreen,
    ClipboardActionPinnedImage,
    ClipboardActionHistoryImage,
    ClipboardActionClipboardImagePin,
    SelectionCopyAreaRequired,
    SelectionCopyInProgress,
    SelectionCopyCancelling,
    SelectionCopyWaitingForCommit,
    HistoryCopyInProgress,
    HistoryCopiedToClipboard,
    HistoryCopyFailed,
    ColorCopyAreaRequired,
    ColorCopiedToClipboard,
    ColorCopyFailed,
    ClipboardPinInProgress,
    AnnotationDocumentUnavailable,
    AnnotationSaveDialogFailed,
    AnnotationSaveChoosing,
    AnnotationSaved,
    AnnotationSaveCancelled,
    AnnotationSaveFailed,
    EditableProjectSaveDialogFailed,
    EditableProjectSaveChoosing,
    EditableProjectSaved,
    EditableProjectSaveCancelled,
    EditableProjectSaveFailed,
    CaptureFrameUnavailable,
    AnnotationOpenDialogFailed,
    AnnotationOpenChoosing,
    AnnotationOpenPrompt,
    AnnotationLoaded,
    AnnotationOpenCancelled,
    AnnotationOpenFailed,
    AnnotationNumberMarker,
    AnnotationColorSelected,
    AnnotationWidth,
    AnnotationTextSize,
    AnnotationOpacity,
    AnnotationFillUnavailable,
    AnnotationFillEnabled,
    AnnotationFillDisabled,
    AnnotationSelectionToolSelected,
    AnnotationToolSelected,
    AnnotationTextEditing,
    AnnotationWatermarkPlacing,
    AnnotationNumberPlacing,
    AnnotationDrawingBlur,
    AnnotationDrawingMosaic,
    AnnotationDrawingHighlight,
    AnnotationDrawingRectangle,
    AnnotationDrawingEllipse,
    AnnotationDrawingLine,
    AnnotationDrawingArrow,
    AnnotationDrawingFreehand,
    AnnotationToolAdded,
    AnnotationFreehandAdded,
    AnnotationAdded,
    AnnotationToolCancelled,
    AnnotationFreehandCancelled,
    AnnotationCancelled,
    AnnotationTextPrompt,
    AnnotationMoved,
    AnnotationResized,
    AnnotationMoveCancelled,
    AnnotationResizeCancelled,
    AnnotationTextDeleted,
    AnnotationTextCancelled,
    AnnotationTextMissing,
    AnnotationTextUnsupported,
    AnnotationTextUpdated,
    AnnotationTextEditPrompt,
    AnnotationColorSamplerMoveFailed,
    AnnotationEditCancelled,
    AnnotationDeselected,
    AnnotationUndone,
    AnnotationRedone,
    AnnotationDeleted,
    AnnotationDuplicated,
    AnnotationRotationUnsupported,
    AnnotationRotatedClockwise,
    AnnotationSelectedPosition,
    AnnotationBroughtToFront,
    AnnotationSentToBack,
    AnnotationBroughtForward,
    AnnotationSentBackward,
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
    OpenImageChoosing,
    OpenImagePrompt,
    OpenProjectChoosing,
    OpenProjectPrompt,
    OpenHistoryInProgress,
    PinHistoryInProgress,
    OpenImageCancelled,
    OpenImageOpened,
    OpenImageOpenedWithoutAnnotations,
    OpenImageFailed,
    SettingsHideBeforeEditorFailed,
    ImageEditorOpenFailed,
    CaptureOverlayOpenFailed,
    ScrollingControlOpenFailed,
    ImageEditorTitle,
    ScrollingScreenshotTitle,
    PinSelectionAreaRequired,
    PinPreparingImage,
    ShortcutUseFailed,
    CaptureShortcutChanged,
    FullScreenShortcutChanged,
    FocusedWindowShortcutChanged,
    ShortcutsRemainDisabled,
    ShortcutPreferenceSaveFailed,
    GlobalShortcutsRegisterFailed,
    GlobalShortcutDisabled,
    GlobalShortcutEnabled,
    GlobalShortcutDisableFailed,
    GlobalShortcutEnableFailed,
    GlobalShortcutRegisterFailed,
    GlobalShortcutPreferenceSaveFailed,
    ExecutableNotFound,
    AutoStartEnabled,
    AutoStartDisabled,
    AutoStartManagedElsewhere,
    AutoStartUpdateFailed,
    AppearancePreferenceSaveFailed,
    AppearanceChanged,
    ShortcutOff,
    CaptureSessionOperationFailed,
    AnnotationOperationFailed,
    CaptureDelaySeconds,
    ExportSourceUnavailable,
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
            Self::RecognitionBusy => "Recognition is already in progress",
            Self::RecognitionSelectAreaQr => "Select an area before recognizing a QR code",
            Self::RecognitionQrInProgress => "Recognizing QR code locally...",
            Self::RecognitionQrNone => "No QR code found in the selection",
            Self::RecognitionQrFound => "Found {count} QR code(s)",
            Self::RecognitionQrCode => "QR code",
            Self::RecognitionQrCodes => "QR codes",
            Self::RecognitionQrFailed => "QR recognition failed: {error}",
            Self::RecognitionSelectAreaText => "Select an area before recognizing text",
            Self::RecognitionTextInProgress => "Recognizing text locally ({language})...",
            Self::RecognitionTextNone => "No text found in the selection",
            Self::RecognitionTextCompleted => "Text recognized locally",
            Self::RecognitionTextTitle => "Recognized text",
            Self::RecognitionOcrUnavailable => {
                "Local OCR is unavailable. Install Tesseract or set FLASH_SHOT_TESSERACT."
            }
            Self::RecognitionOcrFailed => "OCR failed: {error}",
            Self::RecognitionSelectAreaTranslate => "Select an area before translating text",
            Self::TranslationDisabled => {
                "Translation is disabled. Configure FLASH_SHOT_TRANSLATION_ENDPOINT to opt in."
            }
            Self::TranslationUnavailable => "Translation is unavailable: {error}",
            Self::TranslationInProgress => "Recognizing and translating text...",
            Self::TranslationNoText => "No text found in the selection",
            Self::TranslationCompleted => "Translation completed",
            Self::TranslationPreparationFailed => {
                "Could not prepare the selection for translation: {error}"
            }
            Self::TranslationOcrFailed => "Could not recognize text for translation: {error}",
            Self::TranslationServiceFailed => {
                "Translation service failed: {error}. Check the endpoint and try again."
            }
            Self::TranslationSupportReady => {
                "Translation ready: HTTPS endpoint configured for {language}"
            }
            Self::TranslationSupportDisabled => {
                "Translation is disabled. Set FLASH_SHOT_TRANSLATION_ENDPOINT to opt in."
            }
            Self::TranslationSupportNeedsAttention => {
                "Translation configuration needs attention: {error}"
            }
            Self::TranslationServiceReady => "Translation service ready ({count} characters)",
            Self::TranslationServiceNoText => {
                "Translation service returned no text. Check the endpoint response."
            }
            Self::OcrSupportReady => "Local OCR ready: {version} with {language}",
            Self::OcrSupportLanguageMissing => {
                "Tesseract is installed but the {language} language data is missing. Install that language pack or choose another OCR language."
            }
            Self::OcrSupportUnavailable => {
                "Local OCR is unavailable. Install Tesseract or set FLASH_SHOT_TESSERACT: {error}"
            }
            Self::OcrSupportCheckFailed => "Could not check local OCR support: {error}",
            Self::OcrSupportCheckBusy => "Local OCR support check is already in progress",
            Self::OcrSupportCheckInProgress => "Checking local OCR support...",
            Self::OcrSupportCheck => "Check support",
            Self::OcrLanguageAutomatic => "automatic",
            Self::OcrLanguageEnglish => "English",
            Self::OcrLanguageSimplifiedChinese => "Simplified Chinese",
            Self::OcrLanguageEnglishSimplifiedChinese => "English + Simplified Chinese",
            Self::OcrLanguageChanged => "Local OCR language: {language}",
            Self::OcrLanguageSaveFailed => "Could not save OCR language preference: {error}",
            Self::TranslationServiceTestBusy => "Translation service test is already in progress",
            Self::TranslationServiceTestInProgress => "Testing translation service...",
            Self::TranslationServiceTestCancelled => "Translation service test cancelled",
            Self::TranslationServiceTest => "Test service",
            Self::TranslationServiceCancelTest => "Cancel test",
            Self::SettingsLocalOcr => "Local OCR",
            Self::SettingsTranslation => "Translation",
            Self::RecordingSupportCheckInProgress => "Checking FFmpeg recording support...",
            Self::RecordingStoppingAlready => "Screen recording is already stopping...",
            Self::RecordingStopFailed => "Could not stop screen recording: {error}",
            Self::RecordingWaitDirectoryCheck => "Wait for the recording folder check to finish",
            Self::RecordingFinishScreenshotFirst => {
                "Finish or cancel the current screenshot before recording"
            }
            Self::RecordingPreparingDisplay => {
                "Discovering FFmpeg and preparing display recording..."
            }
            Self::RecordingStartupCancelled => "Screen recording startup cancelled",
            Self::RecordingSupportCheckBusy => {
                "FFmpeg recording support check is already in progress"
            }
            Self::RecordingStopBeforeSupportCheck => {
                "Stop the current recording before checking support"
            }
            Self::RecordingSupportCheckCancelled => "FFmpeg recording support check cancelled",
            Self::RecordingDirectoryControlled => "Recording folder is controlled by {env}: {path}",
            Self::RecordingWaitBeforeDirectoryChange => {
                "Wait for the current recording action before changing its folder"
            }
            Self::RecordingChooseDirectory => "Choose a folder for MP4 recordings...",
            Self::RecordingChooseDirectoryPrompt => "Choose recording folder",
            Self::RecordingDirectorySaved => "MP4 recordings now use {path}",
            Self::RecordingDirectorySaveFailed => {
                "Could not save recording folder preference: {error}"
            }
            Self::RecordingDirectoryUseFailed => "Could not use recording folder: {error}",
            Self::RecordingDirectoryUnchanged => "Recording folder unchanged",
            Self::RecordingDirectoryDefaultAlready => {
                "MP4 recordings already use the default folder"
            }
            Self::RecordingDirectoryReset => "Recording folder returned to the default location",
            Self::RecordingDirectoryResetPath => "Recording folder returned to {path}",
            Self::RecordingDirectoryResetFailed => {
                "Could not reset recording folder preference: {error}"
            }
            Self::RecordingWaitBeforeDirectoryCheck => {
                "Wait for the current recording action before checking its folder"
            }
            Self::RecordingDirectoryCheckInProgress => "Checking recording folder...",
            Self::RecordingDirectoryReady => "Recording folder is ready: {path}",
            Self::RecordingDirectoryCheckFailed => "Recording folder check failed: {error}",
            Self::RecordingDirectoryOpened => "Opened recording folder {path}",
            Self::RecordingDirectoryOpenFailed => "Could not open recording folder: {error}",
            Self::RecordingSelectRegion => "Select a region before starting a recording",
            Self::RecordingSupportCheckBeforeStart => {
                "Cancel or wait for the FFmpeg support check before recording"
            }
            Self::RecordingPreparingRegion => "Preparing region recording...",
            Self::RecordingSelectWindow => "Select a window before starting a recording",
            Self::RecordingResolvingWindow => "Looking up selected window bounds for recording...",
            Self::RecordingDisplayDiscoveryInProgress => "Discovering displays for recording...",
            Self::RecordingDisplayChanged => "Recording display: {display}",
            Self::RecordingDisplayDiscoveryFailed => "Could not discover displays: {error}",
            Self::RecordingAudioDiscoveryInProgress => "Discovering recording audio sources...",
            Self::RecordingAudioChanged => "Recording audio: {audio}",
            Self::RecordingAudioDiscoveryFailed => "Could not discover recording audio: {error}",
            Self::RecordingPausing => "Pausing screen recording...",
            Self::RecordingResuming => "Resuming screen recording...",
            Self::RecordingPauseFailed => "Could not change recording pause state: {error}",
            Self::RecordingStarting => "Starting {target} recording...",
            Self::RecordingActive => "Recording {target}...",
            Self::RecordingPaused => "{target} recording paused",
            Self::RecordingProgress => "Recording {target}: {seconds}s, {frames} frames",
            Self::RecordingStopping => "Stopping {target} recording...",
            Self::RecordingSaved => "Screen recording saved to {path}",
            Self::RecordingSavedNotification => "Screen recording saved",
            Self::RecordingFailed => "Screen recording failed: {error}",
            Self::RecordingTargetScreen => "screen",
            Self::RecordingTargetDisplay => "display",
            Self::RecordingTargetWindow => "window",
            Self::RecordingTargetRegion => "selected area",
            Self::RecordingAudioAutomatic => "auto",
            Self::RecordingAudioDisabled => "off",
            Self::RecordingAudioMicrophone => "mic: {device}",
            Self::RecordingAudioSystem => "system audio",
            Self::RecordingDisplayPrimary => "primary",
            Self::RecordingDisplayLabel => "display {label}",
            Self::RecordingStartFailureMissingFfmpeg => {
                "Recording is unavailable because FFmpeg was not found. Install FFmpeg or set FLASH_SHOT_FFMPEG: {error}"
            }
            Self::RecordingStartFailureUnsupported => {
                "This FFmpeg build cannot record the selected source. Use a build with ddagrab or gdigrab: {error}"
            }
            Self::RecordingStartFailureGeneric => "Could not start screen recording: {error}",
            Self::RecordingStartConflictStopping => "Screen recording is already stopping...",
            Self::RecordingStartConflictActive => {
                "Stop the current recording before starting another"
            }
            Self::RecordingStartConflictStarting => {
                "Screen recording startup is already in progress..."
            }
            Self::RecordingDiscoveryConflict => "Wait for recording source discovery to finish...",
            Self::RecordingSupportCheckConflict => {
                "Cancel or wait for the FFmpeg support check before recording"
            }
            Self::RecordingSupportReady => "FFmpeg {version} ready ({backend})",
            Self::RecordingSupportDesktopUnavailable => {
                "FFmpeg {version}: desktop capture unavailable"
            }
            Self::RecordingSettingsDisplay => "Display",
            Self::RecordingSettingsAudio => "Audio",
            Self::RecordingSettingsVideoFolder => "Video folder",
            Self::RecordingFolderUnavailable => "Recording folder unavailable",
            Self::RecordingChooseFolderAction => "Choose folder",
            Self::RecordingCheckFolderAction => "Check folder",
            Self::RecordingOpenFolderAction => "Open folder",
            Self::RecordingUseDefaultFolderAction => "Use default",
            Self::RecordingCheckSupportAction => "Check support",
            Self::RecordingCancelCheckAction => "Cancel check",
            Self::RecordingCancelStartAction => "Cancel start",
            Self::RecordingStoppingAction => "Stopping...",
            Self::RecordingDiscoveringAction => "Discovering...",
            Self::RecordingCheckingFolderAction => "Checking folder...",
            Self::RecordingStopAction => "Stop recording",
            Self::RecordingRecordDisplayAction => "Record display",
            Self::RecordingPauseAction => "Pause",
            Self::RecordingResumeAction => "Resume",
            Self::RecordingStatusLabel => "Status",
            Self::RecordingProgressPreparing => "Preparing recording...",
            Self::RecordingProgressStopping => "Stopping recording...",
            Self::RecordingProgressIdle => "Recording is idle",
            Self::RecordingStateActive => "Recording",
            Self::RecordingStatePaused => "Paused",
            Self::RecordingProgressSummary => "{state} - {seconds}s, {frames} frames",
            Self::RecognitionResultCopied => "{title} copied to clipboard",
            Self::RecognitionResultCopyFailed => "Could not copy {title}: {error}",
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
            Self::CapturePreferences => "Capture preferences",
            Self::GlobalShortcut => "Global shortcut",
            Self::IncludeCursor => "Include cursor",
            Self::Shortcut => "Shortcut",
            Self::FullScreenKey => "Full screen key",
            Self::FocusedWindowKey => "Focused window key",
            Self::CaptureDelay => "Capture delay",
            Self::CaptureDelayOff => "Off",
            Self::ColorCopyFormat => "Color copy format",
            Self::RegisteredShortcut => "Registered: {shortcut}",
            Self::DisabledShortcut => "Disabled: {shortcut}",
            Self::CaptureDelayDisabled => "Capture delay disabled",
            Self::CaptureDelaySet => "Capture delay set to {seconds} seconds",
            Self::CaptureDelaySaveFailed => "Could not save capture delay: {error}",
            Self::CaptureCursorSaveFailed => "Could not save cursor preference: {error}",
            Self::CaptureCursorIncluded => "Capture will include the system cursor",
            Self::CaptureCursorOmitted => "Capture will omit the system cursor",
            Self::ColorCopyFormatChanged => "Color copy format: {format}",
            Self::ColorCopyFormatSaveFailed => "Could not save color format preference: {error}",
            Self::ExportFormatChanged => "Default export format: {format}",
            Self::ExportFormatSaveFailed => "Could not save export format preference: {error}",
            Self::SettingsOpenFailed => "Could not open settings: {error}",
            Self::DelayedCaptureScheduled => "Capture scheduled in {seconds} seconds",
            Self::DelayedCaptureCancelled => "Delayed capture cancelled",
            Self::CaptureStarting => "Capturing virtual desktop...",
            Self::CaptureSummary => {
                "Captured {width} x {height} physical pixels across {display_count} display(s) in {duration_ms} ms ({cpu_copy_count} CPU copies)"
            }
            Self::CaptureFocusedWindow => "Focused window: {width} x {height} physical pixels",
            Self::CaptureFocusedWindowUnavailable => {
                "Could not find a focused window outside Flash Shot"
            }
            Self::CaptureFailed => "Capture failed: {error}",
            Self::CaptureAnnotationDocumentCreateFailed => {
                "Could not create annotation document: {error}"
            }
            Self::CaptureRecordingStoppingConflict => "Screen recording is already stopping...",
            Self::CaptureRecordingActiveConflict => {
                "Stop the current recording before starting a capture"
            }
            Self::CaptureRecordingStartingConflict => {
                "Wait for screen recording startup to finish before capturing"
            }
            Self::AnnotationResizing => "Resizing annotation...",
            Self::AnnotationMoving => "Moving annotation...",
            Self::SelectionMoving => "Moving selection...",
            Self::SelectionDimensions => "Selection: {width} x {height} physical pixels",
            Self::SelectionDimensionLabel => "{width} x {height} px",
            Self::SelectionHoverDetails => "{width} x {height} px | ({x}, {y}) {color}",
            Self::HoverPixelDetails => "({x}, {y}) {color}",
            Self::FrameDimensions => "{width} x {height} physical pixels",
            Self::SmartTargetDetails => "{kind}: {width} x {height} px | ({x}, {y}) {color}",
            Self::SmartTargetLabel => "{kind} | {width} x {height} px",
            Self::InspectionControl => "Control",
            Self::InspectionWindow => "Window",
            Self::OverlaySmartTargetReady => {
                "Smart target ready: {width} x {height} physical pixels"
            }
            Self::OverlaySelectionReady => "Selection ready: {width} x {height} physical pixels",
            Self::OverlaySeedSelectionFailed => "Could not seed acceptance selection: {error}",
            Self::ScrollingScreenshot => "Scrolling screenshot",
            Self::ScrollingSelectArea => "Select an area before starting a scrolling screenshot",
            Self::ScrollingStartFailed => "Could not start scrolling screenshot: {error}",
            Self::ScrollingAlreadyActive => "A scrolling screenshot is already active",
            Self::ScrollingWaitForFinish => "Wait for the scrolling screenshot to finish",
            Self::ScrollingReady => "Scrolling screenshot ready. One frame captured.",
            Self::ScrollingNotActive => "Scrolling screenshot is not active",
            Self::ScrollingNotCollecting => "Scrolling screenshot is not collecting frames",
            Self::ScrollingFrameCaptureBusy => "Scroll frame capture is already in progress",
            Self::ScrollingCapturingNextFrame => "Capturing next scroll frame...",
            Self::ScrollingAssistFailed => "Could not assist scroll: {error}",
            Self::ScrollingSettling => "Scrolled target content. Capturing when it settles...",
            Self::ScrollingFrameCaptured => "Captured scroll frame {count} ({overlap} px overlap)",
            Self::ScrollingFrameCaptureFailed => "Could not capture scroll frame: {error}",
            Self::ScrollingWaitForFrame => "Wait for the current scroll frame capture to finish",
            Self::ScrollingNeedAnotherFrame => "Capture another scroll frame before finishing",
            Self::ScrollingFinishFailed => "Could not finish scrolling screenshot: {error}",
            Self::ScrollingStitching => "Stitching scrolling screenshot",
            Self::ScrollingStitched => {
                "Scrolling screenshot stitched {frames} frames with {joins} overlap joins"
            }
            Self::ScrollingOpenFailed => "Could not open stitched capture: {error}",
            Self::ScrollingCancelled => "Scrolling screenshot cancelled",
            Self::ScrollingNoNewContentFinish => {
                "No new content was revealed. Finish the scrolling screenshot or adjust the page and capture again."
            }
            Self::ScrollingNoNewContentRetry => {
                "No new content was revealed. Scroll the page, then capture again."
            }
            Self::ScrollingOverlapMismatch => {
                "That frame did not overlap the previous one: {error}. Adjust the scroll position and capture again."
            }
            Self::ScrollingCaptureView => "Capture view",
            Self::ScrollingRetryView => "Retry view",
            Self::ScrollingCaptureInProgress => "Capturing...",
            Self::ScrollingScrolling => "Scrolling...",
            Self::ScrollingScrollDownCapture => "Scroll down + capture",
            Self::ScrollingFinish => "Finish",
            Self::ScrollingNotReady => "Not ready",
            Self::ScrollingNoFrames => "No frames",
            Self::ScrollingOneFrame => "1 frame",
            Self::ScrollingManyFrames => "{count} frames",
            Self::ScrollingReadyToFinish => "{count} - ready to finish",
            Self::ScrollingCaptureAnother => "{count} - capture another",
            Self::CancelDelay => "Cancel delay",
            Self::Appearance => "Appearance",
            Self::Language => "Language",
            Self::StartWithWindows => "Start with Windows",
            Self::Updates => "Updates",
            Self::CheckNow => "Check now",
            Self::CancelCheck => "Cancel check",
            Self::UpdateCheckBusy => "Update check is already in progress",
            Self::UpdateChecksDisabled => {
                "Update checks are disabled: set FLASH_SHOT_UPDATE_ENDPOINT"
            }
            Self::UpdateChecksUnavailable => "Update checks are unavailable: {error}",
            Self::UpdateCheckInProgress => "Checking for updates...",
            Self::UpdateCheckCancelled => "Update check cancelled",
            Self::UpdateAvailable => {
                "Update available: {version} (download from your configured release channel)"
            }
            Self::UpdateCurrent => "Flash Shot {version} is up to date",
            Self::UpdateNewerLocal => "Installed version is newer than release manifest {version}",
            Self::UpdateCheckFailed => "Could not check for updates: {error}",
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
            Self::HistoryRecordFailed => {
                " (history unavailable: {error}; the image was saved and can be used normally)"
            }
            Self::HistoryFilesRemovedIndexFailed => {
                "Capture files were removed, but history could not be updated: {error}. Reopen Library to refresh."
            }
            Self::HistoryRetentionUpdateFailed => {
                "Could not update history retention: {error}. The previous limit remains active; try again."
            }
            Self::HistoryRetentionUpdating => "Updating screenshot history retention to {count}...",
            Self::HistoryRetentionDeleteFailed => {
                "Could not remove {count} capture(s); history retention was unchanged"
            }
            Self::HistoryRetentionUpdated => {
                "Screenshot history retains the latest {count} captures"
            }
            Self::HistoryRetentionSaveFailed => {
                "History retention is {count} captures for this session but could not be saved: {error}"
            }
            Self::HistoryFallbackUsed => "; quick-save folder unavailable; using {path}",
            Self::HistoryFallbackPreferenceFailed => {
                " (could not persist the fallback folder: {error})"
            }
            Self::QuickSaveFolderBusy => {
                "Finish active history work before changing the save folder"
            }
            Self::QuickSaveFolderChoosing => {
                "Choose a folder for quick saves and screenshot history..."
            }
            Self::QuickSaveFolderPrompt => "Choose quick-save folder",
            Self::QuickSaveFolderSelectionCancelled => "Quick-save folder selection cancelled",
            Self::QuickSaveFolderChanged => "Quick saves and history now use {path}",
            Self::QuickSaveFolderPreferenceSaveFailed => {
                "Could not save quick-save folder preference: {error}"
            }
            Self::QuickSaveFolderUseFailed => "Could not use quick-save folder: {error}",
            Self::QuickSaveFolderChecking => "Checking quick-save folder {path}...",
            Self::QuickSaveFolderReady => "Quick-save folder is ready: {path}",
            Self::QuickSaveFolderCheckFailed => "Quick-save folder check failed: {error}",
            Self::QuickSavePrefixChanged => {
                "Quick-save names use {prefix}<yyyyMMddHHmmssSSS><UUIDv7>.png"
            }
            Self::QuickSavePrefixSaveFailed => {
                "Could not save quick-save naming preference: {error}"
            }
            Self::HistorySelectionRemoved => "Capture removed from selection",
            Self::HistorySelectionCount => "{count} capture(s) selected",
            Self::HistoryNoMatches => "No captures match the current filter",
            Self::HistoryAlreadySelected => "{count} capture(s) already selected",
            Self::HistorySelectionAdded => "Selected {count} additional capture(s)",
            Self::HistorySelectionCleared => "History selection cleared",
            Self::HistorySelectAtLeastOne => "Select at least one capture first",
            Self::HistoryAlreadyEmpty => "Screenshot history is already empty",
            Self::HistoryClearConfirmation => {
                "Confirm deletion of {count} {scope} saved capture(s)"
            }
            Self::HistoryClearScopeAll => "all",
            Self::HistoryClearScopeFiltered => "filtered",
            Self::HistoryClearScopeSelected => "selected",
            Self::HistoryClearCancelled => "Screenshot history clear cancelled",
            Self::HistoryWaitingForReads => "Waiting for active history reads before deleting...",
            Self::HistoryClearing => "Clearing {count} saved capture(s)...",
            Self::HistoryCleared => "Screenshot history cleared",
            Self::HistoryDeletedSelected => "Deleted {count} selected capture(s)",
            Self::HistoryClearedFiltered => "Cleared {count} filtered capture(s)",
            Self::HistoryClearedWithFailures => {
                "Cleared {deleted} capture(s); {failed} could not be deleted"
            }
            Self::HistoryRemoving => "Removing {path}...",
            Self::HistoryRemoveFailed => "Could not remove screenshot history item: {error}",
            Self::HistoryRemoved => "Removed {path} from screenshot history",
            Self::HistoryFolderOpened => "Opened screenshot folder {path}",
            Self::HistoryFolderOpenFailed => "Could not open screenshot folder: {error}",
            Self::HistoryUnavailableNoParent => "; history unavailable: saved path has no parent",
            Self::HistoryUnavailable => "; history unavailable: {error}",
            Self::SaveStartFailed => "Could not start saving the selection: {error}",
            Self::SaveDialogAboveCaptureFailed => {
                "Could not show Save dialog above capture: {error}"
            }
            Self::SaveSelectionChoosing => "Choose where to save the selection...",
            Self::SaveHistoryBusy => "Waiting for active history work before saving...",
            Self::SaveSelectionInProgress => "Quick saving selection...",
            Self::SaveCompleted => "{source} saved to {path}",
            Self::SaveTransitionFailed => "Could not finish saving the selection: {error}",
            Self::NotificationScreenshotSaved => "Screenshot saved",
            Self::PinnedSaveBusy => "Another pinned image is already saving. Try again shortly.",
            Self::PinnedSaveWaitingForHistory => {
                "Waiting for active history work before saving the pinned image..."
            }
            Self::PinnedSaveInProgress => "Saving pinned image...",
            Self::PinnedImageSavedTo => "Pinned image saved to {path}",
            Self::PinnedImageSaveFailed => "Could not save pinned image: {error}",
            Self::NotificationPinnedImageSaved => "Pinned image saved",
            Self::SelectionCopiedToClipboard => "Selection copied to clipboard",
            Self::CopyCancelledBeforeClipboardChanged => {
                "Copy cancelled before the clipboard changed"
            }
            Self::CopyFailed => "Copy failed: {error}",
            Self::NotificationScreenshotCopied => "Screenshot copied to clipboard",
            Self::FullScreenCopyInProgress => "Capturing full screen for clipboard...",
            Self::FullScreenCopiedToClipboard => "Full screen copied to clipboard",
            Self::FullScreenCopyFailed => "Could not copy full screen: {error}",
            Self::NotificationFullScreenCopied => "Full screen copied to clipboard",
            Self::FullScreenSaveInProgress => "Capturing full screen to save...",
            Self::FullScreenSavedTo => "Full screen saved to {path}",
            Self::FullScreenSaveFailed => "Could not save full screen: {error}",
            Self::NotificationFullScreenSaved => "Full screen saved",
            Self::FullScreenPinInProgress => "Capturing full screen to pin...",
            Self::ClipboardBusy => "Wait for the current clipboard copy to finish before {action}",
            Self::ClipboardActionSelection => "copying a selection",
            Self::ClipboardActionRecognizedText => "copying recognized text",
            Self::ClipboardActionColor => "copying a color",
            Self::ClipboardActionFullScreen => "copying the full screen",
            Self::ClipboardActionPinnedImage => "copying a pinned image",
            Self::ClipboardActionHistoryImage => "copying a history image",
            Self::ClipboardActionClipboardImagePin => "pinning a clipboard image",
            Self::SelectionCopyAreaRequired => "Select an area before copying",
            Self::SelectionCopyInProgress => "Copying selection in the background...",
            Self::SelectionCopyCancelling => "Cancelling background clipboard copy...",
            Self::SelectionCopyWaitingForCommit => {
                "Clipboard write already started; waiting for copy to finish..."
            }
            Self::HistoryCopyInProgress => "Copying {path}...",
            Self::HistoryCopiedToClipboard => "History image copied to clipboard",
            Self::HistoryCopyFailed => "Could not copy history image: {error}",
            Self::ColorCopyAreaRequired => "Move over the captured image to copy a color",
            Self::ColorCopiedToClipboard => "{color} copied to clipboard",
            Self::ColorCopyFailed => "Could not copy {color}: {error}",
            Self::ClipboardPinInProgress => "Reading clipboard image...",
            Self::AnnotationDocumentUnavailable => "Annotation document is unavailable",
            Self::AnnotationSaveDialogFailed => "Could not show annotation Save dialog: {error}",
            Self::AnnotationSaveChoosing => "Choose where to save annotations...",
            Self::AnnotationSaved => "Annotations saved to {path}",
            Self::AnnotationSaveCancelled => "Annotation save cancelled",
            Self::AnnotationSaveFailed => "Could not save annotations: {error}",
            Self::EditableProjectSaveDialogFailed => {
                "Could not show editable-project Save dialog: {error}"
            }
            Self::EditableProjectSaveChoosing => "Choose where to save the editable image...",
            Self::EditableProjectSaved => "Editable project saved to {image} and {sidecar}",
            Self::EditableProjectSaveCancelled => "Editable-project save cancelled",
            Self::EditableProjectSaveFailed => "Could not save editable project: {error}",
            Self::CaptureFrameUnavailable => "Capture frame is unavailable",
            Self::AnnotationOpenDialogFailed => "Could not show annotation Open dialog: {error}",
            Self::AnnotationOpenChoosing => "Choose annotations to open...",
            Self::AnnotationOpenPrompt => "Open annotation document",
            Self::AnnotationLoaded => "Loaded annotations from {path}",
            Self::AnnotationOpenCancelled => "Open annotations cancelled",
            Self::AnnotationOpenFailed => "Could not open annotations: {error}",
            Self::AnnotationNumberMarker => "Number marker: {value}",
            Self::AnnotationColorSelected => "Annotation color selected",
            Self::AnnotationWidth => "Annotation width: {width} px",
            Self::AnnotationTextSize => "Text size: {size} px",
            Self::AnnotationOpacity => "Annotation opacity: {percent}%",
            Self::AnnotationFillUnavailable => "Fill is available for rectangles and ellipses",
            Self::AnnotationFillEnabled => "Shape fill enabled",
            Self::AnnotationFillDisabled => "Shape fill disabled",
            Self::AnnotationSelectionToolSelected => "Selection tool selected",
            Self::AnnotationToolSelected => "{tool} tool selected",
            Self::AnnotationTextEditing => "Editing text...",
            Self::AnnotationWatermarkPlacing => "Placing watermark...",
            Self::AnnotationNumberPlacing => "Placing number...",
            Self::AnnotationDrawingBlur => "Drawing blur...",
            Self::AnnotationDrawingMosaic => "Drawing mosaic...",
            Self::AnnotationDrawingHighlight => "Drawing highlight...",
            Self::AnnotationDrawingRectangle => "Drawing rectangle...",
            Self::AnnotationDrawingEllipse => "Drawing ellipse...",
            Self::AnnotationDrawingLine => "Drawing line...",
            Self::AnnotationDrawingArrow => "Drawing arrow...",
            Self::AnnotationDrawingFreehand => "Drawing freehand...",
            Self::AnnotationToolAdded => "{tool} added",
            Self::AnnotationFreehandAdded => "Freehand stroke added",
            Self::AnnotationAdded => "Annotation added",
            Self::AnnotationToolCancelled => "{tool} cancelled",
            Self::AnnotationFreehandCancelled => "Freehand stroke cancelled",
            Self::AnnotationCancelled => "Annotation cancelled",
            Self::AnnotationTextPrompt => "Type {kind}, then press Enter",
            Self::AnnotationMoved => "Annotation moved",
            Self::AnnotationResized => "Annotation resized",
            Self::AnnotationMoveCancelled => "Annotation move cancelled",
            Self::AnnotationResizeCancelled => "Annotation resize cancelled",
            Self::AnnotationTextDeleted => "Text annotation deleted",
            Self::AnnotationTextCancelled => "Text cancelled",
            Self::AnnotationTextMissing => "Text annotation no longer exists",
            Self::AnnotationTextUnsupported => "Selected annotation cannot be edited as text",
            Self::AnnotationTextUpdated => "Text annotation updated",
            Self::AnnotationTextEditPrompt => "Edit text, then press Enter",
            Self::AnnotationColorSamplerMoveFailed => "Could not move color sampler: {error}",
            Self::AnnotationEditCancelled => "Annotation edit cancelled",
            Self::AnnotationDeselected => "Annotation deselected",
            Self::AnnotationUndone => "Annotation undone",
            Self::AnnotationRedone => "Annotation redone",
            Self::AnnotationDeleted => "Annotation deleted",
            Self::AnnotationDuplicated => "Annotation duplicated",
            Self::AnnotationRotationUnsupported => {
                "Rotation is not supported for text or number annotations"
            }
            Self::AnnotationRotatedClockwise => "Annotation rotated clockwise",
            Self::AnnotationSelectedPosition => "Selected annotation {position} of {count}",
            Self::AnnotationBroughtToFront => "Annotation brought to front",
            Self::AnnotationSentToBack => "Annotation sent to back",
            Self::AnnotationBroughtForward => "Annotation brought forward",
            Self::AnnotationSentBackward => "Annotation sent backward",
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
            Self::OpenImageChoosing => "Choose a PNG image to annotate...",
            Self::OpenImagePrompt => "Open PNG image",
            Self::OpenProjectChoosing => "Choose an editable annotation project...",
            Self::OpenProjectPrompt => "Open annotation project",
            Self::OpenHistoryInProgress => "Opening {path}...",
            Self::PinHistoryInProgress => "Pinning {path}...",
            Self::OpenImageCancelled => "Open image cancelled",
            Self::OpenImageOpened => "Opened {path} for annotation",
            Self::OpenImageOpenedWithoutAnnotations => {
                "Opened {path} without annotations: {warning}"
            }
            Self::OpenImageFailed => "Could not open image: {error}",
            Self::SettingsHideBeforeEditorFailed => {
                "Could not hide settings before opening the editor: {error}"
            }
            Self::ImageEditorOpenFailed => "Image editor window failed: {error}",
            Self::CaptureOverlayOpenFailed => "Capture overlay failed: {error}",
            Self::ScrollingControlOpenFailed => {
                "Could not open scrolling screenshot controls: {error}"
            }
            Self::ImageEditorTitle => "Flash Shot - Edit image",
            Self::ScrollingScreenshotTitle => "Flash Shot - Scrolling screenshot",
            Self::PinSelectionAreaRequired => "Select an area before pinning",
            Self::PinPreparingImage => "Preparing pinned image...",
            Self::ShortcutUseFailed => "Could not use shortcut: {error}",
            Self::CaptureShortcutChanged => "Capture shortcut changed to {shortcut}",
            Self::FullScreenShortcutChanged => "Full-screen shortcut: {shortcut}",
            Self::FocusedWindowShortcutChanged => "Focused-window shortcut: {shortcut}",
            Self::ShortcutsRemainDisabled => "{status}; global shortcuts remain disabled",
            Self::ShortcutPreferenceSaveFailed => "Could not save shortcut preference: {error}",
            Self::GlobalShortcutsRegisterFailed => "Could not register global shortcuts: {error}",
            Self::GlobalShortcutDisabled => "Global capture shortcut disabled",
            Self::GlobalShortcutEnabled => "Global capture shortcut enabled: {shortcut}",
            Self::GlobalShortcutDisableFailed => "Could not disable global shortcut: {error}",
            Self::GlobalShortcutEnableFailed => "Could not enable global shortcut: {error}",
            Self::GlobalShortcutRegisterFailed => "Could not register {shortcut}: {error}",
            Self::GlobalShortcutPreferenceSaveFailed => {
                "Could not save global shortcut preference: {error}"
            }
            Self::ExecutableNotFound => "Could not find the application executable: {error}",
            Self::AutoStartEnabled => "Launch at sign-in enabled",
            Self::AutoStartDisabled => "Launch at sign-in disabled",
            Self::AutoStartManagedElsewhere => {
                "Launch at sign-in is managed by a different Flash Shot executable"
            }
            Self::AutoStartUpdateFailed => "Could not update launch at sign-in: {error}",
            Self::AppearancePreferenceSaveFailed => "Could not save appearance preference: {error}",
            Self::AppearanceChanged => "Appearance changed to {mode}",
            Self::ShortcutOff => "Off",
            Self::CaptureSessionOperationFailed => "Could not update capture state: {error}",
            Self::AnnotationOperationFailed => "Could not update annotation: {error}",
            Self::CaptureDelaySeconds => "{seconds}s",
            Self::ExportSourceUnavailable => "Capture frame or annotation document is unavailable",
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
            Self::RecognitionBusy => "识别正在进行中",
            Self::RecognitionSelectAreaQr => "请先选择区域再识别二维码",
            Self::RecognitionQrInProgress => "正在本地识别二维码...",
            Self::RecognitionQrNone => "选区中未找到二维码",
            Self::RecognitionQrFound => "找到 {count} 个二维码",
            Self::RecognitionQrCode => "二维码",
            Self::RecognitionQrCodes => "二维码",
            Self::RecognitionQrFailed => "二维码识别失败：{error}",
            Self::RecognitionSelectAreaText => "请先选择区域再识别文字",
            Self::RecognitionTextInProgress => "正在本地识别文字（{language}）...",
            Self::RecognitionTextNone => "选区中未找到文字",
            Self::RecognitionTextCompleted => "文字已在本地识别",
            Self::RecognitionTextTitle => "识别出的文字",
            Self::RecognitionOcrUnavailable => {
                "本地 OCR 不可用。请安装 Tesseract，或设置 FLASH_SHOT_TESSERACT。"
            }
            Self::RecognitionOcrFailed => "OCR 失败：{error}",
            Self::RecognitionSelectAreaTranslate => "请先选择区域再翻译文字",
            Self::TranslationDisabled => {
                "翻译已禁用。如需启用，请配置 FLASH_SHOT_TRANSLATION_ENDPOINT。"
            }
            Self::TranslationUnavailable => "翻译不可用：{error}",
            Self::TranslationInProgress => "正在识别并翻译文字...",
            Self::TranslationNoText => "选区中未找到文字",
            Self::TranslationCompleted => "翻译完成",
            Self::TranslationPreparationFailed => "无法准备翻译选区：{error}",
            Self::TranslationOcrFailed => "无法为翻译识别文字：{error}",
            Self::TranslationServiceFailed => "翻译服务失败：{error}。请检查端点后重试。",
            Self::TranslationSupportReady => "翻译已就绪：已为 {language} 配置 HTTPS 端点",
            Self::TranslationSupportDisabled => {
                "翻译已禁用。如需启用，请设置 FLASH_SHOT_TRANSLATION_ENDPOINT。"
            }
            Self::TranslationSupportNeedsAttention => "翻译配置需要检查：{error}",
            Self::TranslationServiceReady => "翻译服务已就绪（{count} 个字符）",
            Self::TranslationServiceNoText => "翻译服务未返回文字，请检查端点响应。",
            Self::OcrSupportReady => "本地 OCR 已就绪：{version}，使用 {language}",
            Self::OcrSupportLanguageMissing => {
                "Tesseract 已安装，但缺少 {language} 语言数据。请安装语言包或选择其他 OCR 语言。"
            }
            Self::OcrSupportUnavailable => {
                "本地 OCR 不可用。请安装 Tesseract，或设置 FLASH_SHOT_TESSERACT：{error}"
            }
            Self::OcrSupportCheckFailed => "无法检查本地 OCR 支持：{error}",
            Self::OcrSupportCheckBusy => "本地 OCR 支持检查正在进行中",
            Self::OcrSupportCheckInProgress => "正在检查本地 OCR 支持...",
            Self::OcrSupportCheck => "检查支持",
            Self::OcrLanguageAutomatic => "自动",
            Self::OcrLanguageEnglish => "英语",
            Self::OcrLanguageSimplifiedChinese => "简体中文",
            Self::OcrLanguageEnglishSimplifiedChinese => "英语 + 简体中文",
            Self::OcrLanguageChanged => "本地 OCR 语言：{language}",
            Self::OcrLanguageSaveFailed => "无法保存 OCR 语言偏好：{error}",
            Self::TranslationServiceTestBusy => "翻译服务测试正在进行中",
            Self::TranslationServiceTestInProgress => "正在测试翻译服务...",
            Self::TranslationServiceTestCancelled => "翻译服务测试已取消",
            Self::TranslationServiceTest => "测试服务",
            Self::TranslationServiceCancelTest => "取消测试",
            Self::SettingsLocalOcr => "本地 OCR",
            Self::SettingsTranslation => "翻译",
            Self::RecordingSupportCheckInProgress => "正在检查 FFmpeg 录屏支持...",
            Self::RecordingStoppingAlready => "屏幕录制正在停止...",
            Self::RecordingStopFailed => "无法停止屏幕录制：{error}",
            Self::RecordingWaitDirectoryCheck => "请等待录屏目录检查完成",
            Self::RecordingFinishScreenshotFirst => "请先完成或取消当前截图，再开始录屏",
            Self::RecordingPreparingDisplay => "正在检查 FFmpeg 并准备显示器录屏...",
            Self::RecordingStartupCancelled => "已取消屏幕录制启动",
            Self::RecordingSupportCheckBusy => "FFmpeg 录屏支持检查正在进行中",
            Self::RecordingStopBeforeSupportCheck => "请先停止当前录屏，再检查支持情况",
            Self::RecordingSupportCheckCancelled => "已取消 FFmpeg 录屏支持检查",
            Self::RecordingDirectoryControlled => "录屏目录由 {env} 控制：{path}",
            Self::RecordingWaitBeforeDirectoryChange => "请等待当前录屏操作完成，再修改目录",
            Self::RecordingChooseDirectory => "正在选择 MP4 录屏目录...",
            Self::RecordingChooseDirectoryPrompt => "选择录屏目录",
            Self::RecordingDirectorySaved => "MP4 录屏将使用 {path}",
            Self::RecordingDirectorySaveFailed => "无法保存录屏目录偏好：{error}",
            Self::RecordingDirectoryUseFailed => "无法使用录屏目录：{error}",
            Self::RecordingDirectoryUnchanged => "录屏目录未更改",
            Self::RecordingDirectoryDefaultAlready => "MP4 录屏已经使用默认目录",
            Self::RecordingDirectoryReset => "录屏目录已恢复为默认位置",
            Self::RecordingDirectoryResetPath => "录屏目录已恢复为 {path}",
            Self::RecordingDirectoryResetFailed => "无法重置录屏目录偏好：{error}",
            Self::RecordingWaitBeforeDirectoryCheck => "请等待当前录屏操作完成，再检查目录",
            Self::RecordingDirectoryCheckInProgress => "正在检查录屏目录...",
            Self::RecordingDirectoryReady => "录屏目录已就绪：{path}",
            Self::RecordingDirectoryCheckFailed => "录屏目录检查失败：{error}",
            Self::RecordingDirectoryOpened => "已打开录屏目录 {path}",
            Self::RecordingDirectoryOpenFailed => "无法打开录屏目录：{error}",
            Self::RecordingSelectRegion => "请先选择区域，再开始录屏",
            Self::RecordingSupportCheckBeforeStart => {
                "请取消或等待 FFmpeg 支持检查完成，再开始录屏"
            }
            Self::RecordingPreparingRegion => "正在准备区域录屏...",
            Self::RecordingSelectWindow => "请先选择窗口，再开始录屏",
            Self::RecordingResolvingWindow => "正在查找所选窗口的边界以录屏...",
            Self::RecordingDisplayDiscoveryInProgress => "正在发现可录制的显示器...",
            Self::RecordingDisplayChanged => "录屏显示器：{display}",
            Self::RecordingDisplayDiscoveryFailed => "无法发现显示器：{error}",
            Self::RecordingAudioDiscoveryInProgress => "正在发现录屏音频源...",
            Self::RecordingAudioChanged => "录屏音频：{audio}",
            Self::RecordingAudioDiscoveryFailed => "无法发现录屏音频：{error}",
            Self::RecordingPausing => "正在暂停屏幕录制...",
            Self::RecordingResuming => "正在继续屏幕录制...",
            Self::RecordingPauseFailed => "无法更改录屏暂停状态：{error}",
            Self::RecordingStarting => "正在启动{target}录屏...",
            Self::RecordingActive => "正在录制{target}...",
            Self::RecordingPaused => "{target}录屏已暂停",
            Self::RecordingProgress => "正在录制{target}：{seconds} 秒，{frames} 帧",
            Self::RecordingStopping => "正在停止{target}录屏...",
            Self::RecordingSaved => "屏幕录制已保存到 {path}",
            Self::RecordingSavedNotification => "屏幕录制已保存",
            Self::RecordingFailed => "屏幕录制失败：{error}",
            Self::RecordingTargetScreen => "屏幕",
            Self::RecordingTargetDisplay => "显示器",
            Self::RecordingTargetWindow => "窗口",
            Self::RecordingTargetRegion => "所选区域",
            Self::RecordingAudioAutomatic => "自动",
            Self::RecordingAudioDisabled => "关闭",
            Self::RecordingAudioMicrophone => "麦克风：{device}",
            Self::RecordingAudioSystem => "系统声音",
            Self::RecordingDisplayPrimary => "主显示器",
            Self::RecordingDisplayLabel => "显示器 {label}",
            Self::RecordingStartFailureMissingFfmpeg => {
                "录屏不可用，因为未找到 FFmpeg。请安装 FFmpeg，或设置 FLASH_SHOT_FFMPEG：{error}"
            }
            Self::RecordingStartFailureUnsupported => {
                "当前 FFmpeg 版本无法录制所选来源。请使用支持 ddagrab 或 gdigrab 的版本：{error}"
            }
            Self::RecordingStartFailureGeneric => "无法启动屏幕录制：{error}",
            Self::RecordingStartConflictStopping => "屏幕录制正在停止...",
            Self::RecordingStartConflictActive => "请先停止当前录屏，再开始新的录屏",
            Self::RecordingStartConflictStarting => "屏幕录制正在启动...",
            Self::RecordingDiscoveryConflict => "请等待录屏来源发现完成...",
            Self::RecordingSupportCheckConflict => "请取消或等待 FFmpeg 支持检查完成，再开始录屏",
            Self::RecordingSupportReady => "FFmpeg {version} 已就绪（{backend}）",
            Self::RecordingSupportDesktopUnavailable => "FFmpeg {version}：桌面捕获不可用",
            Self::RecordingSettingsDisplay => "显示器",
            Self::RecordingSettingsAudio => "音频",
            Self::RecordingSettingsVideoFolder => "视频目录",
            Self::RecordingFolderUnavailable => "录屏目录不可用",
            Self::RecordingChooseFolderAction => "选择目录",
            Self::RecordingCheckFolderAction => "检查目录",
            Self::RecordingOpenFolderAction => "打开目录",
            Self::RecordingUseDefaultFolderAction => "使用默认目录",
            Self::RecordingCheckSupportAction => "检查支持",
            Self::RecordingCancelCheckAction => "取消检查",
            Self::RecordingCancelStartAction => "取消启动",
            Self::RecordingStoppingAction => "正在停止...",
            Self::RecordingDiscoveringAction => "正在发现...",
            Self::RecordingCheckingFolderAction => "正在检查目录...",
            Self::RecordingStopAction => "停止录屏",
            Self::RecordingRecordDisplayAction => "录制显示器",
            Self::RecordingPauseAction => "暂停",
            Self::RecordingResumeAction => "继续",
            Self::RecordingStatusLabel => "状态",
            Self::RecordingProgressPreparing => "正在准备录屏...",
            Self::RecordingProgressStopping => "正在停止录屏...",
            Self::RecordingProgressIdle => "录屏未运行",
            Self::RecordingStateActive => "录制中",
            Self::RecordingStatePaused => "已暂停",
            Self::RecordingProgressSummary => "{state} - {seconds} 秒，{frames} 帧",
            Self::RecognitionResultCopied => "{title}已复制到剪贴板",
            Self::RecognitionResultCopyFailed => "无法复制{title}：{error}",
            Self::CapturePageDescription => "开始截图或调整截图偏好。",
            Self::LibraryPageDescription => "查找已保存截图、修改输出位置并管理历史记录。",
            Self::RecordPageDescription => "选择录制来源、输出目录和录屏控制。",
            Self::AppPageDescription => "设置外观、语言、启动和更新偏好。",
            Self::CapturePreferences => "截图偏好",
            Self::GlobalShortcut => "全局快捷键",
            Self::IncludeCursor => "包含光标",
            Self::Shortcut => "快捷键",
            Self::FullScreenKey => "全屏快捷键",
            Self::FocusedWindowKey => "焦点窗口快捷键",
            Self::CaptureDelay => "截图延迟",
            Self::CaptureDelayOff => "关闭",
            Self::ColorCopyFormat => "颜色复制格式",
            Self::RegisteredShortcut => "已注册：{shortcut}",
            Self::DisabledShortcut => "已禁用：{shortcut}",
            Self::CaptureDelayDisabled => "截图延迟已关闭",
            Self::CaptureDelaySet => "截图延迟已设置为 {seconds} 秒",
            Self::CaptureDelaySaveFailed => "无法保存截图延迟：{error}",
            Self::CaptureCursorSaveFailed => "无法保存光标偏好：{error}",
            Self::CaptureCursorIncluded => "截图将包含系统光标",
            Self::CaptureCursorOmitted => "截图将不包含系统光标",
            Self::ColorCopyFormatChanged => "颜色复制格式：{format}",
            Self::ColorCopyFormatSaveFailed => "无法保存颜色格式偏好：{error}",
            Self::ExportFormatChanged => "默认导出格式：{format}",
            Self::ExportFormatSaveFailed => "无法保存导出格式偏好：{error}",
            Self::SettingsOpenFailed => "无法打开设置：{error}",
            Self::DelayedCaptureScheduled => "将在 {seconds} 秒后截图",
            Self::DelayedCaptureCancelled => "已取消延迟截图",
            Self::CaptureStarting => "正在捕获虚拟桌面...",
            Self::CaptureSummary => {
                "已捕获 {width} x {height} 个物理像素，涵盖 {display_count} 个显示器，用时 {duration_ms} ms（{cpu_copy_count} 次 CPU 复制）"
            }
            Self::CaptureFocusedWindow => "焦点窗口：{width} x {height} 个物理像素",
            Self::CaptureFocusedWindowUnavailable => "找不到 Flash Shot 之外的焦点窗口",
            Self::CaptureFailed => "截图失败：{error}",
            Self::CaptureAnnotationDocumentCreateFailed => "无法创建标注文档：{error}",
            Self::CaptureRecordingStoppingConflict => "屏幕录制正在停止...",
            Self::CaptureRecordingActiveConflict => "请先停止当前录屏，再开始截图",
            Self::CaptureRecordingStartingConflict => "请等待屏幕录制启动完成，再开始截图",
            Self::AnnotationResizing => "正在调整标注大小...",
            Self::AnnotationMoving => "正在移动标注...",
            Self::SelectionMoving => "正在移动选区...",
            Self::SelectionDimensions => "选区：{width} x {height} 个物理像素",
            Self::SelectionDimensionLabel => "{width} x {height} 像素",
            Self::SelectionHoverDetails => "{width} x {height} 像素 | ({x}, {y}) {color}",
            Self::HoverPixelDetails => "({x}, {y}) {color}",
            Self::FrameDimensions => "{width} x {height} 个物理像素",
            Self::SmartTargetDetails => "{kind}：{width} x {height} 像素 | ({x}, {y}) {color}",
            Self::SmartTargetLabel => "{kind} | {width} x {height} 像素",
            Self::InspectionControl => "控件",
            Self::InspectionWindow => "窗口",
            Self::OverlaySmartTargetReady => "智能目标就绪：{width} x {height} 个物理像素",
            Self::OverlaySelectionReady => "选区就绪：{width} x {height} 个物理像素",
            Self::OverlaySeedSelectionFailed => "无法准备验收选区：{error}",
            Self::ScrollingScreenshot => "长截图",
            Self::ScrollingSelectArea => "请先选择区域再开始长截图",
            Self::ScrollingStartFailed => "无法开始长截图：{error}",
            Self::ScrollingAlreadyActive => "长截图已在进行中",
            Self::ScrollingWaitForFinish => "请等待长截图完成",
            Self::ScrollingReady => "长截图已就绪，已捕获 1 个视口。",
            Self::ScrollingNotActive => "长截图未在进行中",
            Self::ScrollingNotCollecting => "长截图当前未收集视口",
            Self::ScrollingFrameCaptureBusy => "滚动视口捕获已在进行中",
            Self::ScrollingCapturingNextFrame => "正在捕获下一个滚动视口...",
            Self::ScrollingAssistFailed => "无法辅助滚动：{error}",
            Self::ScrollingSettling => "已滚动目标内容，等待稳定后捕获...",
            Self::ScrollingFrameCaptured => "已捕获第 {count} 个滚动视口（重叠 {overlap} 像素）",
            Self::ScrollingFrameCaptureFailed => "无法捕获滚动视口：{error}",
            Self::ScrollingWaitForFrame => "请等待当前滚动视口捕获完成",
            Self::ScrollingNeedAnotherFrame => "完成前请再捕获一个滚动视口",
            Self::ScrollingFinishFailed => "无法完成长截图：{error}",
            Self::ScrollingStitching => "正在拼接长截图",
            Self::ScrollingStitched => "长截图已拼接：{frames} 个视口，{joins} 个重叠连接",
            Self::ScrollingOpenFailed => "无法打开拼接后的截图：{error}",
            Self::ScrollingCancelled => "已取消长截图",
            Self::ScrollingNoNewContentFinish => {
                "未发现新内容。请完成长截图，或调整页面后重新捕获。"
            }
            Self::ScrollingNoNewContentRetry => "未发现新内容。请滚动页面后重新捕获。",
            Self::ScrollingOverlapMismatch => {
                "该视口与上一视口没有重叠：{error}。请调整滚动位置后重新捕获。"
            }
            Self::ScrollingCaptureView => "捕获视口",
            Self::ScrollingRetryView => "重试视口",
            Self::ScrollingCaptureInProgress => "正在捕获...",
            Self::ScrollingScrolling => "滚动中...",
            Self::ScrollingScrollDownCapture => "向下滚动并捕获",
            Self::ScrollingFinish => "完成",
            Self::ScrollingNotReady => "未就绪",
            Self::ScrollingNoFrames => "没有视口",
            Self::ScrollingOneFrame => "1 个视口",
            Self::ScrollingManyFrames => "{count} 个视口",
            Self::ScrollingReadyToFinish => "{count}，可以完成",
            Self::ScrollingCaptureAnother => "{count}，请再捕获一个",
            Self::CancelDelay => "取消延时",
            Self::Appearance => "外观",
            Self::Language => "语言",
            Self::StartWithWindows => "随 Windows 启动",
            Self::Updates => "更新",
            Self::CheckNow => "立即检查",
            Self::CancelCheck => "取消检查",
            Self::UpdateCheckBusy => "更新检查正在进行中",
            Self::UpdateChecksDisabled => "更新检查已禁用：请设置 FLASH_SHOT_UPDATE_ENDPOINT",
            Self::UpdateChecksUnavailable => "更新检查不可用：{error}",
            Self::UpdateCheckInProgress => "正在检查更新...",
            Self::UpdateCheckCancelled => "已取消更新检查",
            Self::UpdateAvailable => "发现更新：{version}（请从已配置的发布渠道下载）",
            Self::UpdateCurrent => "Flash Shot {version} 已是最新版本",
            Self::UpdateNewerLocal => "当前安装版本高于发布清单中的 {version}",
            Self::UpdateCheckFailed => "无法检查更新：{error}",
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
            Self::HistoryRecordFailed => "（历史记录不可用：{error}；图片已保存，仍可正常使用）",
            Self::HistoryFilesRemovedIndexFailed => {
                "截图文件已移除，但历史记录未能更新：{error}。请重新打开图库刷新。"
            }
            Self::HistoryRetentionUpdateFailed => {
                "无法更新历史记录保留数量：{error}。当前仍使用原设置，请重试。"
            }
            Self::HistoryRetentionUpdating => "正在将截图历史保留数量更新为 {count}...",
            Self::HistoryRetentionDeleteFailed => "无法删除 {count} 张截图；截图历史保留设置未更改",
            Self::HistoryRetentionUpdated => "截图历史记录保留最近 {count} 张截图",
            Self::HistoryRetentionSaveFailed => {
                "本次会话的历史保留数量为 {count} 张截图，但无法保存：{error}"
            }
            Self::HistoryFallbackUsed => "；快速保存目录不可用，已改用 {path}",
            Self::HistoryFallbackPreferenceFailed => "（无法保存回退目录设置：{error}）",
            Self::QuickSaveFolderBusy => "请先完成正在进行的历史记录操作，再更改保存目录",
            Self::QuickSaveFolderChoosing => "请选择快速保存和截图历史记录目录...",
            Self::QuickSaveFolderPrompt => "选择快速保存目录",
            Self::QuickSaveFolderSelectionCancelled => "已取消选择快速保存目录",
            Self::QuickSaveFolderChanged => "快速保存和历史记录现在使用 {path}",
            Self::QuickSaveFolderPreferenceSaveFailed => "无法保存快速保存目录设置：{error}",
            Self::QuickSaveFolderUseFailed => "无法使用快速保存目录：{error}",
            Self::QuickSaveFolderChecking => "正在检查快速保存目录 {path}...",
            Self::QuickSaveFolderReady => "快速保存目录已就绪：{path}",
            Self::QuickSaveFolderCheckFailed => "快速保存目录检查失败：{error}",
            Self::QuickSavePrefixChanged => {
                "快速保存文件名使用 {prefix}<yyyyMMddHHmmssSSS><UUIDv7>.png"
            }
            Self::QuickSavePrefixSaveFailed => "无法保存快速保存命名设置：{error}",
            Self::HistorySelectionRemoved => "已从选择中移除截图",
            Self::HistorySelectionCount => "已选择 {count} 张截图",
            Self::HistoryNoMatches => "当前筛选条件没有匹配的截图",
            Self::HistoryAlreadySelected => "已有 {count} 张截图被选择",
            Self::HistorySelectionAdded => "已额外选择 {count} 张截图",
            Self::HistorySelectionCleared => "已清除历史记录选择",
            Self::HistorySelectAtLeastOne => "请先选择至少一张截图",
            Self::HistoryAlreadyEmpty => "截图历史记录已经为空",
            Self::HistoryClearConfirmation => "确认删除 {scope} 的 {count} 张已保存截图",
            Self::HistoryClearScopeAll => "全部",
            Self::HistoryClearScopeFiltered => "筛选结果",
            Self::HistoryClearScopeSelected => "已选择",
            Self::HistoryClearCancelled => "已取消清除截图历史记录",
            Self::HistoryWaitingForReads => "正在等待历史记录读取完成后再删除...",
            Self::HistoryClearing => "正在清除 {count} 张已保存截图...",
            Self::HistoryCleared => "截图历史记录已清除",
            Self::HistoryDeletedSelected => "已删除 {count} 张已选择的截图",
            Self::HistoryClearedFiltered => "已清除 {count} 张筛选出的截图",
            Self::HistoryClearedWithFailures => "已清除 {deleted} 张截图；{failed} 张无法删除",
            Self::HistoryRemoving => "正在移除 {path}...",
            Self::HistoryRemoveFailed => "无法移除截图历史记录项：{error}",
            Self::HistoryRemoved => "已从截图历史记录移除 {path}",
            Self::HistoryFolderOpened => "已打开截图目录 {path}",
            Self::HistoryFolderOpenFailed => "无法打开截图目录：{error}",
            Self::HistoryUnavailableNoParent => "；历史记录不可用：保存路径没有父目录",
            Self::HistoryUnavailable => "；历史记录不可用：{error}",
            Self::SaveStartFailed => "无法开始保存选区：{error}",
            Self::SaveDialogAboveCaptureFailed => "无法在截图上方显示保存对话框：{error}",
            Self::SaveSelectionChoosing => "请选择保存选区的位置...",
            Self::SaveHistoryBusy => "正在等待历史记录操作完成后再保存...",
            Self::SaveSelectionInProgress => "正在快速保存选区...",
            Self::SaveCompleted => "已将{source}保存到 {path}",
            Self::SaveTransitionFailed => "无法完成选区保存：{error}",
            Self::NotificationScreenshotSaved => "截图已保存",
            Self::PinnedSaveBusy => "另一个置顶图片正在保存，请稍后重试",
            Self::PinnedSaveWaitingForHistory => "正在等待历史记录操作完成后再保存置顶图片...",
            Self::PinnedSaveInProgress => "正在保存置顶图片...",
            Self::PinnedImageSavedTo => "置顶图片已保存到 {path}",
            Self::PinnedImageSaveFailed => "无法保存置顶图片：{error}",
            Self::NotificationPinnedImageSaved => "置顶图片已保存",
            Self::SelectionCopiedToClipboard => "选区已复制到剪贴板",
            Self::CopyCancelledBeforeClipboardChanged => "已取消复制，剪贴板内容未更改",
            Self::CopyFailed => "复制失败：{error}",
            Self::NotificationScreenshotCopied => "截图已复制到剪贴板",
            Self::FullScreenCopyInProgress => "正在捕获全屏到剪贴板...",
            Self::FullScreenCopiedToClipboard => "全屏截图已复制到剪贴板",
            Self::FullScreenCopyFailed => "无法复制全屏截图：{error}",
            Self::NotificationFullScreenCopied => "全屏截图已复制到剪贴板",
            Self::FullScreenSaveInProgress => "正在捕获全屏并保存...",
            Self::FullScreenSavedTo => "全屏截图已保存到 {path}",
            Self::FullScreenSaveFailed => "无法保存全屏截图：{error}",
            Self::NotificationFullScreenSaved => "全屏截图已保存",
            Self::FullScreenPinInProgress => "正在捕获全屏并固定...",
            Self::ClipboardBusy => "请等待当前剪贴板复制完成后再{action}",
            Self::ClipboardActionSelection => "复制选区",
            Self::ClipboardActionRecognizedText => "复制识别文字",
            Self::ClipboardActionColor => "复制颜色",
            Self::ClipboardActionFullScreen => "复制全屏截图",
            Self::ClipboardActionPinnedImage => "复制置顶图片",
            Self::ClipboardActionHistoryImage => "复制历史图片",
            Self::ClipboardActionClipboardImagePin => "固定剪贴板图片",
            Self::SelectionCopyAreaRequired => "请先选择区域再复制",
            Self::SelectionCopyInProgress => "正在后台复制选区...",
            Self::SelectionCopyCancelling => "正在取消后台剪贴板复制...",
            Self::SelectionCopyWaitingForCommit => "剪贴板写入已开始，正在等待复制完成...",
            Self::HistoryCopyInProgress => "正在复制 {path}...",
            Self::HistoryCopiedToClipboard => "历史图片已复制到剪贴板",
            Self::HistoryCopyFailed => "无法复制历史图片：{error}",
            Self::ColorCopyAreaRequired => "请将指针移到截图上再复制颜色",
            Self::ColorCopiedToClipboard => "{color}已复制到剪贴板",
            Self::ColorCopyFailed => "无法复制{color}：{error}",
            Self::ClipboardPinInProgress => "正在读取剪贴板图片...",
            Self::AnnotationDocumentUnavailable => "标注文档不可用",
            Self::AnnotationSaveDialogFailed => "无法显示标注保存对话框：{error}",
            Self::AnnotationSaveChoosing => "请选择保存标注的位置...",
            Self::AnnotationSaved => "标注已保存到 {path}",
            Self::AnnotationSaveCancelled => "已取消保存标注",
            Self::AnnotationSaveFailed => "无法保存标注：{error}",
            Self::EditableProjectSaveDialogFailed => "无法显示可编辑项目保存对话框：{error}",
            Self::EditableProjectSaveChoosing => "请选择保存可编辑图片的位置...",
            Self::EditableProjectSaved => "可编辑项目已保存到 {image} 和 {sidecar}",
            Self::EditableProjectSaveCancelled => "已取消保存可编辑项目",
            Self::EditableProjectSaveFailed => "无法保存可编辑项目：{error}",
            Self::CaptureFrameUnavailable => "截图帧不可用",
            Self::AnnotationOpenDialogFailed => "无法显示标注打开对话框：{error}",
            Self::AnnotationOpenChoosing => "请选择要打开的标注...",
            Self::AnnotationOpenPrompt => "打开标注文档",
            Self::AnnotationLoaded => "已从 {path} 加载标注",
            Self::AnnotationOpenCancelled => "已取消打开标注",
            Self::AnnotationOpenFailed => "无法打开标注：{error}",
            Self::AnnotationNumberMarker => "序号：{value}",
            Self::AnnotationColorSelected => "已选择标注颜色",
            Self::AnnotationWidth => "标注线宽：{width} 像素",
            Self::AnnotationTextSize => "文字大小：{size} 像素",
            Self::AnnotationOpacity => "标注不透明度：{percent}%",
            Self::AnnotationFillUnavailable => "填充仅适用于矩形和椭圆",
            Self::AnnotationFillEnabled => "已启用形状填充",
            Self::AnnotationFillDisabled => "已禁用形状填充",
            Self::AnnotationSelectionToolSelected => "选择工具已选中",
            Self::AnnotationToolSelected => "已选择{tool}工具",
            Self::AnnotationTextEditing => "正在编辑文字...",
            Self::AnnotationWatermarkPlacing => "正在放置水印...",
            Self::AnnotationNumberPlacing => "正在放置序号...",
            Self::AnnotationDrawingBlur => "正在绘制模糊...",
            Self::AnnotationDrawingMosaic => "正在绘制马赛克...",
            Self::AnnotationDrawingHighlight => "正在绘制高亮...",
            Self::AnnotationDrawingRectangle => "正在绘制矩形...",
            Self::AnnotationDrawingEllipse => "正在绘制椭圆...",
            Self::AnnotationDrawingLine => "正在绘制直线...",
            Self::AnnotationDrawingArrow => "正在绘制箭头...",
            Self::AnnotationDrawingFreehand => "正在绘制画笔...",
            Self::AnnotationToolAdded => "已添加{tool}",
            Self::AnnotationFreehandAdded => "已添加画笔笔划",
            Self::AnnotationAdded => "已添加标注",
            Self::AnnotationToolCancelled => "已取消{tool}",
            Self::AnnotationFreehandCancelled => "已取消画笔笔划",
            Self::AnnotationCancelled => "已取消标注",
            Self::AnnotationTextPrompt => "请输入{kind}，然后按 Enter",
            Self::AnnotationMoved => "标注已移动",
            Self::AnnotationResized => "标注已调整大小",
            Self::AnnotationMoveCancelled => "已取消移动标注",
            Self::AnnotationResizeCancelled => "已取消调整标注大小",
            Self::AnnotationTextDeleted => "文字标注已删除",
            Self::AnnotationTextCancelled => "已取消文字输入",
            Self::AnnotationTextMissing => "文字标注已不存在",
            Self::AnnotationTextUnsupported => "选中的标注不是可编辑文字",
            Self::AnnotationTextUpdated => "文字标注已更新",
            Self::AnnotationTextEditPrompt => "编辑文字，然后按 Enter",
            Self::AnnotationColorSamplerMoveFailed => "无法移动颜色取样器：{error}",
            Self::AnnotationEditCancelled => "已取消标注编辑",
            Self::AnnotationDeselected => "已取消选择标注",
            Self::AnnotationUndone => "已撤销标注操作",
            Self::AnnotationRedone => "已重做标注操作",
            Self::AnnotationDeleted => "标注已删除",
            Self::AnnotationDuplicated => "标注已复制",
            Self::AnnotationRotationUnsupported => "文字或序号标注不支持旋转",
            Self::AnnotationRotatedClockwise => "标注已顺时针旋转",
            Self::AnnotationSelectedPosition => "已选择第 {position} 个标注，共 {count} 个",
            Self::AnnotationBroughtToFront => "标注已置于顶层",
            Self::AnnotationSentToBack => "标注已置于底层",
            Self::AnnotationBroughtForward => "标注已上移",
            Self::AnnotationSentBackward => "标注已下移",
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
            Self::OpenImageChoosing => "请选择要标注的 PNG 图片...",
            Self::OpenImagePrompt => "打开 PNG 图片",
            Self::OpenProjectChoosing => "请选择可编辑标注项目...",
            Self::OpenProjectPrompt => "打开标注项目",
            Self::OpenHistoryInProgress => "正在打开 {path}...",
            Self::PinHistoryInProgress => "正在固定 {path}...",
            Self::OpenImageCancelled => "已取消打开图片",
            Self::OpenImageOpened => "已打开 {path}，可以开始标注",
            Self::OpenImageOpenedWithoutAnnotations => "已打开 {path}，未加载标注：{warning}",
            Self::OpenImageFailed => "无法打开图片：{error}",
            Self::SettingsHideBeforeEditorFailed => "打开编辑器前无法隐藏设置窗口：{error}",
            Self::ImageEditorOpenFailed => "无法打开图片编辑器窗口：{error}",
            Self::CaptureOverlayOpenFailed => "无法打开截图覆盖层：{error}",
            Self::ScrollingControlOpenFailed => "无法打开长截图控制器：{error}",
            Self::ImageEditorTitle => "Flash Shot - 图片编辑",
            Self::ScrollingScreenshotTitle => "Flash Shot - 长截图",
            Self::PinSelectionAreaRequired => "请先选择区域，再固定图片",
            Self::PinPreparingImage => "正在准备置顶图片...",
            Self::ShortcutUseFailed => "无法使用快捷键：{error}",
            Self::CaptureShortcutChanged => "截图快捷键已改为 {shortcut}",
            Self::FullScreenShortcutChanged => "全屏快捷键：{shortcut}",
            Self::FocusedWindowShortcutChanged => "焦点窗口快捷键：{shortcut}",
            Self::ShortcutsRemainDisabled => "{status}；全局快捷键仍处于禁用状态",
            Self::ShortcutPreferenceSaveFailed => "无法保存快捷键偏好：{error}",
            Self::GlobalShortcutsRegisterFailed => "无法注册全局快捷键：{error}",
            Self::GlobalShortcutDisabled => "全局截图快捷键已禁用",
            Self::GlobalShortcutEnabled => "全局截图快捷键已启用：{shortcut}",
            Self::GlobalShortcutDisableFailed => "无法禁用全局快捷键：{error}",
            Self::GlobalShortcutEnableFailed => "无法启用全局快捷键：{error}",
            Self::GlobalShortcutRegisterFailed => "无法注册 {shortcut}：{error}",
            Self::GlobalShortcutPreferenceSaveFailed => "无法保存全局快捷键偏好：{error}",
            Self::ExecutableNotFound => "找不到应用程序：{error}",
            Self::AutoStartEnabled => "已启用随系统登录启动",
            Self::AutoStartDisabled => "已禁用随系统登录启动",
            Self::AutoStartManagedElsewhere => "随系统登录启动由其他 Flash Shot 程序管理",
            Self::AutoStartUpdateFailed => "无法更新随系统登录启动设置：{error}",
            Self::AppearancePreferenceSaveFailed => "无法保存外观偏好：{error}",
            Self::AppearanceChanged => "外观已切换为 {mode}",
            Self::ShortcutOff => "关闭",
            Self::CaptureSessionOperationFailed => "无法更新截图状态：{error}",
            Self::AnnotationOperationFailed => "无法更新标注：{error}",
            Self::CaptureDelaySeconds => "{seconds} 秒",
            Self::ExportSourceUnavailable => "截图帧或标注文档不可用",
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
            Locale::SimplifiedChinese.text(UiText::RecordingSettingsDisplay),
            "显示器"
        );
        assert_eq!(
            Locale::SimplifiedChinese.format_template(
                UiText::RecordingProgress,
                &[("target", "所选区域"), ("seconds", "3"), ("frames", "117")],
            ),
            "正在录制所选区域：3 秒，117 帧"
        );
        assert_eq!(
            Locale::English.format_template(
                UiText::RecordingStartFailureMissingFfmpeg,
                &[("error", "ffmpeg.exe")],
            ),
            "Recording is unavailable because FFmpeg was not found. Install FFmpeg or set FLASH_SHOT_FFMPEG: ffmpeg.exe"
        );
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
    fn annotation_feedback_localizes_tools_styles_and_text_editing() {
        assert_eq!(
            Locale::English.format_template(
                UiText::AnnotationToolSelected,
                &[("tool", Locale::English.text(UiText::OverlayArrow))],
            ),
            "Arrow tool selected"
        );
        assert_eq!(
            Locale::SimplifiedChinese.format_template(
                UiText::AnnotationToolSelected,
                &[("tool", Locale::SimplifiedChinese.text(UiText::OverlayArrow))],
            ),
            "已选择箭头工具"
        );
        assert_eq!(
            Locale::SimplifiedChinese
                .format_template(UiText::AnnotationNumberMarker, &[("value", "12")],),
            "序号：12"
        );
        assert_eq!(
            Locale::SimplifiedChinese.format_template(UiText::AnnotationWidth, &[("width", "4")],),
            "标注线宽：4 像素"
        );
        assert_eq!(
            Locale::SimplifiedChinese
                .format_template(UiText::AnnotationOpacity, &[("percent", "50")],),
            "标注不透明度：50%"
        );
        assert_eq!(
            Locale::SimplifiedChinese.format_template(
                UiText::AnnotationTextPrompt,
                &[(
                    "kind",
                    Locale::SimplifiedChinese.text(UiText::OverlayWatermark)
                )],
            ),
            "请输入水印，然后按 Enter"
        );
        assert_eq!(
            Locale::SimplifiedChinese.text(UiText::AnnotationTextUnsupported),
            "选中的标注不是可编辑文字"
        );
        assert_eq!(
            Locale::SimplifiedChinese.text(UiText::AnnotationMoveCancelled),
            "已取消移动标注"
        );
        assert_eq!(
            Locale::English.text(UiText::AnnotationUndone),
            "Annotation undone"
        );
        assert_eq!(
            Locale::SimplifiedChinese.text(UiText::AnnotationRotatedClockwise),
            "标注已顺时针旋转"
        );
        assert_eq!(
            Locale::SimplifiedChinese.format_template(
                UiText::AnnotationSelectedPosition,
                &[("position", "2"), ("count", "5")],
            ),
            "已选择第 2 个标注，共 5 个"
        );
        assert_eq!(
            Locale::SimplifiedChinese.format_template(
                UiText::AnnotationColorSamplerMoveFailed,
                &[("error", "拒绝访问")],
            ),
            "无法移动颜色取样器：拒绝访问"
        );
        assert_eq!(
            Locale::SimplifiedChinese.text(UiText::AnnotationBroughtForward),
            "标注已上移"
        );
        assert_eq!(
            Locale::SimplifiedChinese.format_template(
                UiText::AnnotationOperationFailed,
                &[("error", "历史记录不可用")],
            ),
            "无法更新标注：历史记录不可用"
        );
    }

    #[test]
    fn capture_preference_feedback_localizes_labels_and_dynamic_values() {
        assert_eq!(
            Locale::SimplifiedChinese.text(UiText::CapturePreferences),
            "截图偏好"
        );
        assert_eq!(
            Locale::SimplifiedChinese.text(UiText::CaptureDelayOff),
            "关闭"
        );
        assert_eq!(
            Locale::SimplifiedChinese
                .format_template(UiText::RegisteredShortcut, &[("shortcut", "Ctrl+Alt+S")],),
            "已注册：Ctrl+Alt+S"
        );
        assert_eq!(
            Locale::SimplifiedChinese
                .format_template(UiText::CaptureDelaySet, &[("seconds", "5")],),
            "截图延迟已设置为 5 秒"
        );
        assert_eq!(
            Locale::SimplifiedChinese.text(UiText::CaptureCursorIncluded),
            "截图将包含系统光标"
        );
        assert_eq!(
            Locale::SimplifiedChinese
                .format_template(UiText::ColorCopyFormatChanged, &[("format", "RGB")],),
            "颜色复制格式：RGB"
        );
        assert_eq!(
            Locale::SimplifiedChinese
                .format_template(UiText::SettingsOpenFailed, &[("error", "窗口不可用")],),
            "无法打开设置：窗口不可用"
        );
        assert_eq!(
            Locale::English.format_template(
                UiText::CaptureSessionOperationFailed,
                &[("error", "invalid state")],
            ),
            "Could not update capture state: invalid state"
        );
        assert_eq!(
            Locale::SimplifiedChinese
                .format_template(UiText::CaptureDelaySeconds, &[("seconds", "5")],),
            "5 秒"
        );
        assert_eq!(
            Locale::English.text(UiText::ExportSourceUnavailable),
            "Capture frame or annotation document is unavailable"
        );
    }

    #[test]
    fn scrolling_feedback_localizes_actions_and_progress_details() {
        assert_eq!(
            Locale::SimplifiedChinese.text(UiText::ScrollingScreenshot),
            "长截图"
        );
        assert_eq!(
            Locale::SimplifiedChinese.text(UiText::ScrollingScrollDownCapture),
            "向下滚动并捕获"
        );
        assert_eq!(
            Locale::SimplifiedChinese.format_template(
                UiText::ScrollingFrameCaptured,
                &[("count", "3"), ("overlap", "120")],
            ),
            "已捕获第 3 个滚动视口（重叠 120 像素）"
        );
        assert_eq!(
            Locale::SimplifiedChinese.format_template(
                UiText::ScrollingStitched,
                &[("frames", "4"), ("joins", "3")],
            ),
            "长截图已拼接：4 个视口，3 个重叠连接"
        );
        assert_eq!(
            Locale::English
                .format_template(UiText::ScrollingOverlapMismatch, &[("error", "no overlap")],),
            "That frame did not overlap the previous one: no overlap. Adjust the scroll position and capture again."
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
        assert_eq!(
            Locale::SimplifiedChinese.format_template(
                UiText::HistoryFilesRemovedIndexFailed,
                &[("error", "拒绝访问")],
            ),
            "截图文件已移除，但历史记录未能更新：拒绝访问。请重新打开图库刷新。"
        );
        assert_eq!(
            Locale::English.format_template(
                UiText::HistoryRetentionUpdateFailed,
                &[("error", "disk full")],
            ),
            "Could not update history retention: disk full. The previous limit remains active; try again."
        );
    }

    #[test]
    fn clipboard_feedback_localizes_actions_paths_and_errors() {
        assert_eq!(
            Locale::English.format_template(
                UiText::ClipboardBusy,
                &[(
                    "action",
                    Locale::English.text(UiText::ClipboardActionSelection)
                )],
            ),
            "Wait for the current clipboard copy to finish before copying a selection"
        );
        assert_eq!(
            Locale::SimplifiedChinese.format_template(
                UiText::HistoryCopyInProgress,
                &[("path", "D:\\Screenshots\\capture.png")],
            ),
            "正在复制 D:\\Screenshots\\capture.png..."
        );
        assert_eq!(
            Locale::SimplifiedChinese.format_template(
                UiText::ColorCopyFailed,
                &[("color", "#AABBCC"), ("error", "剪贴板被占用")],
            ),
            "无法复制#AABBCC：剪贴板被占用"
        );
        assert_eq!(
            Locale::SimplifiedChinese.text(UiText::SelectionCopyCancelling),
            "正在取消后台剪贴板复制..."
        );
    }

    #[test]
    fn library_and_history_status_templates_keep_dynamic_details_localized() {
        assert_eq!(
            Locale::English.format_template(
                UiText::QuickSaveFolderChanged,
                &[("path", "D:\\Screenshots")],
            ),
            "Quick saves and history now use D:\\Screenshots"
        );
        assert_eq!(
            Locale::SimplifiedChinese
                .format_template(UiText::QuickSaveFolderCheckFailed, &[("error", "拒绝访问")],),
            "快速保存目录检查失败：拒绝访问"
        );
        assert_eq!(
            Locale::SimplifiedChinese.format_template(
                UiText::HistoryRetentionSaveFailed,
                &[("count", "50"), ("error", "磁盘已满")],
            ),
            "本次会话的历史保留数量为 50 张截图，但无法保存：磁盘已满"
        );
    }
}
