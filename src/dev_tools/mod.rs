//! Opt-in development and acceptance tools dispatched through the sole application binary.

use std::ffi::OsStr;

#[path = "annotation-stress.rs"]
mod annotation_stress;
#[path = "capture-stress.rs"]
mod capture_stress;
#[path = "copy-performance.rs"]
mod copy_performance;
#[path = "export-stress.rs"]
mod export_stress;
#[path = "history-resource-acceptance.rs"]
mod history_resource_acceptance;
#[path = "overlay-copy-batch.rs"]
mod overlay_copy_batch;
#[path = "overlay-interaction-acceptance.rs"]
mod overlay_interaction_acceptance;
#[path = "performance-report.rs"]
mod performance_report;
#[path = "pin-lifecycle-acceptance.rs"]
mod pin_lifecycle_acceptance;
#[path = "png-stress.rs"]
mod png_stress;
#[path = "recognition-acceptance.rs"]
mod recognition_acceptance;
#[path = "recording-acceptance.rs"]
mod recording_acceptance;
#[path = "scroll-acceptance.rs"]
mod scroll_acceptance;
#[path = "settings-ui-acceptance.rs"]
mod settings_ui_acceptance;
mod support;
#[path = "windows-acceptance-probe.rs"]
mod windows_acceptance_probe;

/// Process-local selector used by `scripts/run-dev-tool.ps1`.
pub const DEV_TOOL_ENV: &str = "FLASH_SHOT_DEV_TOOL";

type Entrypoint = fn();

/// Runs the selected development module before normal application startup.
///
/// Returning `false` means no selector was provided, so the caller should continue into the
/// desktop application. Tool arguments remain untouched for the selected module to parse.
pub fn run_from_environment() -> Result<bool, String> {
    let Some(name) = std::env::var_os(DEV_TOOL_ENV) else {
        return Ok(false);
    };
    let entrypoint = resolve(&name).ok_or_else(|| {
        format!(
            "unknown {DEV_TOOL_ENV} value {:?}; expected one of: {}",
            name,
            TOOL_NAMES.join(", ")
        )
    })?;
    entrypoint();
    Ok(true)
}

const TOOL_NAMES: &[&str] = &[
    "annotation-stress",
    "capture-stress",
    "copy-performance",
    "export-stress",
    "history-resource-acceptance",
    "overlay-copy-batch",
    "overlay-interaction-acceptance",
    "performance-report",
    "pin-lifecycle-acceptance",
    "png-stress",
    "recognition-acceptance",
    "recording-acceptance",
    "scroll-acceptance",
    "settings-ui-acceptance",
    "windows-acceptance-probe",
];

fn resolve(name: &OsStr) -> Option<Entrypoint> {
    match name.to_str()? {
        "annotation-stress" => Some(annotation_stress::entrypoint),
        "capture-stress" => Some(capture_stress::entrypoint),
        "copy-performance" => Some(copy_performance::entrypoint),
        "export-stress" => Some(export_stress::entrypoint),
        "history-resource-acceptance" => Some(history_resource_acceptance::entrypoint),
        "overlay-copy-batch" => Some(overlay_copy_batch::entrypoint),
        "overlay-interaction-acceptance" => Some(overlay_interaction_acceptance::entrypoint),
        "performance-report" => Some(performance_report::entrypoint),
        "pin-lifecycle-acceptance" => Some(pin_lifecycle_acceptance::entrypoint),
        "png-stress" => Some(png_stress::entrypoint),
        "recognition-acceptance" => Some(recognition_acceptance::entrypoint),
        "recording-acceptance" => Some(recording_acceptance::entrypoint),
        "scroll-acceptance" => Some(scroll_acceptance::entrypoint),
        "settings-ui-acceptance" => Some(settings_ui_acceptance::entrypoint),
        "windows-acceptance-probe" => Some(windows_acceptance_probe::entrypoint),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{TOOL_NAMES, resolve};
    use std::ffi::OsStr;

    #[test]
    fn every_documented_tool_name_resolves() {
        for name in TOOL_NAMES {
            assert!(resolve(OsStr::new(name)).is_some(), "missing {name}");
        }
    }

    #[test]
    fn unknown_tool_name_is_rejected() {
        assert!(resolve(OsStr::new("not-a-tool")).is_none());
    }
}
