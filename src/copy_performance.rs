//! Explicit system-clipboard latency measurements for the ordinary selection export path.

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{
    domain::{annotation::AnnotationDocument, geometry::PhysicalRect},
    performance::{PerformanceRecorder, build_profile},
    platform::{
        capture::{CaptureFrame, PixelFormat},
        clipboard::{ClipboardService, SystemClipboard},
    },
};

pub const COPY_CLICK_TO_CLIPBOARD_READABLE: &str = "copy_click_to_clipboard_readable";
const WARMUP_ITERATIONS: usize = 2;
const DEFAULT_ITERATIONS: usize = 30;
const DEFAULT_MAX_P95_MS: u64 = 250;
const DEFAULT_WIDTH: u32 = 1280;
const DEFAULT_HEIGHT: u32 = 720;
const SOURCE_PADDING: u32 = 16;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CopyPerformanceConfig {
    pub allow_system_clipboard: bool,
    pub iterations: usize,
    pub width: u32,
    pub height: u32,
    pub max_p95_ms: Option<u64>,
    pub output: Option<PathBuf>,
    pub metrics_directory: Option<PathBuf>,
}

impl Default for CopyPerformanceConfig {
    fn default() -> Self {
        Self {
            allow_system_clipboard: false,
            iterations: DEFAULT_ITERATIONS,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            max_p95_ms: Some(DEFAULT_MAX_P95_MS),
            output: None,
            metrics_directory: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CopyPerformanceReport {
    value: serde_json::Value,
    passed: bool,
}

impl CopyPerformanceReport {
    pub const fn passed(&self) -> bool {
        self.passed
    }

    pub fn to_pretty_json(&self) -> io::Result<String> {
        serde_json::to_string_pretty(&self.value).map_err(io::Error::other)
    }

    pub fn write(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, self.to_pretty_json()?)
    }
}

/// Measures the selection composite/crop, system write, and confirmed system read as one action.
///
/// The command deliberately mutates the user's clipboard, so callers must opt in explicitly.
pub fn run(config: &CopyPerformanceConfig) -> io::Result<CopyPerformanceReport> {
    validate_config(config)?;
    if !config.allow_system_clipboard {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "copy performance changes the system clipboard; rerun with --allow-system-clipboard",
        ));
    }

    let frame = benchmark_frame(config.width, config.height)?;
    let selection = benchmark_selection(config.width, config.height)?;
    let document = AnnotationDocument::new(frame.bounds).map_err(io::Error::other)?;
    let clipboard = SystemClipboard;
    let expected = frame.crop(selection)?;
    let recorder = config
        .metrics_directory
        .as_ref()
        .map(PerformanceRecorder::new)
        .transpose()?;
    for _ in 0..WARMUP_ITERATIONS {
        let read_back = copy_selection_and_read(&frame, &document, selection, &clipboard)?;
        validate_read_back(&expected, &read_back)?;
    }

    let mut samples_ms = Vec::with_capacity(config.iterations);
    for _ in 0..config.iterations {
        let started_at = Instant::now();
        let read_back = copy_selection_and_read(&frame, &document, selection, &clipboard)?;
        let elapsed = started_at.elapsed();
        validate_read_back(&expected, &read_back)?;
        if let Some(recorder) = &recorder {
            recorder.record_duration(COPY_CLICK_TO_CLIPBOARD_READABLE, elapsed);
        }
        samples_ms.push(elapsed.as_secs_f64() * 1_000.0);
    }

    build_report(config, samples_ms)
}

/// Calculates deterministic percentile summaries and applies the requested p95 gate.
fn build_report(
    config: &CopyPerformanceConfig,
    mut samples_ms: Vec<f64>,
) -> io::Result<CopyPerformanceReport> {
    if samples_ms.is_empty()
        || samples_ms
            .iter()
            .any(|sample| !sample.is_finite() || *sample < 0.0)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "copy performance samples must be finite non-negative values",
        ));
    }
    samples_ms.sort_by(f64::total_cmp);
    let p95_ms = percentile(&samples_ms, 95);
    let passed = config.max_p95_ms.is_none_or(|limit| p95_ms <= limit as f64);
    Ok(CopyPerformanceReport {
        value: serde_json::json!({
            "schema_version": 1,
            "test": "selection_copy_system_clipboard_performance",
            "metric": COPY_CLICK_TO_CLIPBOARD_READABLE,
            "unit": "ms",
            "build_profile": build_profile(),
            "passed": passed,
            "system_clipboard_mutated": true,
            "iterations": samples_ms.len(),
            "warmup_iterations": WARMUP_ITERATIONS,
            "metrics_directory": config.metrics_directory,
            "selection": {
                "width": config.width,
                "height": config.height,
                "pixel_bytes": u64::from(config.width) * u64::from(config.height) * 4,
            },
            "latency_ms": {
                "min": samples_ms[0],
                "p50": percentile(&samples_ms, 50),
                "p95": p95_ms,
                "max": samples_ms[samples_ms.len() - 1],
            },
            "thresholds": {
                "max_p95_ms": config.max_p95_ms,
            },
            "gates": {
                "p95_passed": passed,
            },
        }),
        passed,
    })
}

/// Uses production ClipboardService writes and a production read to prove data is consumer-readable.
fn copy_selection_and_read(
    frame: &CaptureFrame,
    document: &AnnotationDocument,
    selection: PhysicalRect,
    clipboard: &SystemClipboard,
) -> io::Result<CaptureFrame> {
    let selected = frame.composite_annotations(document)?.crop(selection)?;
    clipboard.copy_image(&selected)?;
    wait_for_readable_selection(clipboard, selected.width, selected.height)
}

/// Polls through brief clipboard contention while rejecting stale images with another geometry.
fn wait_for_readable_selection(
    clipboard: &SystemClipboard,
    expected_width: u32,
    expected_height: u32,
) -> io::Result<CaptureFrame> {
    const READ_ATTEMPTS: usize = 4;
    let mut last_error = None;
    for attempt in 0..READ_ATTEMPTS {
        match clipboard.read_image() {
            Ok(frame) if frame.width == expected_width && frame.height == expected_height => {
                return Ok(frame);
            }
            Ok(frame) => {
                last_error = Some(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "clipboard image was {}x{}, expected {expected_width}x{expected_height}",
                        frame.width, frame.height
                    ),
                ));
            }
            Err(error) => last_error = Some(error),
        }
        if attempt + 1 < READ_ATTEMPTS {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    Err(last_error.expect("clipboard read was attempted"))
}

/// Rejects stale or altered clipboard content without including the comparison in timed samples.
fn validate_read_back(expected: &CaptureFrame, actual: &CaptureFrame) -> io::Result<()> {
    actual.validate()?;
    if actual.width != expected.width
        || actual.height != expected.height
        || actual.format != expected.format
        || actual.pixels != expected.pixels
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "system clipboard image did not match the copied selection",
        ));
    }
    Ok(())
}

/// Rejects dimensions that cannot form a valid BGRA selection before touching the clipboard.
fn validate_config(config: &CopyPerformanceConfig) -> io::Result<()> {
    if config.iterations == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "copy performance iterations must be greater than zero",
        ));
    }
    checked_pixel_layout(config.width, config.height)?;
    Ok(())
}

/// Builds a stable ordinary-sized selection so repeated runs measure the same pixel workload.
fn benchmark_frame(width: u32, height: u32) -> io::Result<CaptureFrame> {
    let source_width = width
        .checked_add(SOURCE_PADDING * 2)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "source width overflow"))?;
    let source_height = height
        .checked_add(SOURCE_PADDING * 2)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "source height overflow"))?;
    let (stride, pixel_bytes) = checked_pixel_layout(source_width, source_height)?;
    let right = i32::try_from(source_width)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "selection width is too large"))?;
    let bottom = i32::try_from(source_height).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidInput, "selection height is too large")
    })?;
    let mut pixels = vec![0_u8; pixel_bytes];
    for (y, row) in pixels.chunks_exact_mut(stride).enumerate() {
        for (x, pixel) in row.chunks_exact_mut(4).enumerate() {
            pixel.copy_from_slice(&[
                x as u8,
                y as u8,
                x.wrapping_mul(19).wrapping_add(y.wrapping_mul(29)) as u8,
                255,
            ]);
        }
    }
    Ok(CaptureFrame {
        bounds: PhysicalRect {
            left: 0,
            top: 0,
            right,
            bottom,
        },
        width: source_width,
        height: source_height,
        stride,
        format: PixelFormat::Bgra8,
        pixels: Arc::from(pixels),
        capture_duration: Duration::ZERO,
        cpu_copy_count: 1,
    })
}

/// Places the benchmark selection inside its source so the timed path performs a real crop.
fn benchmark_selection(width: u32, height: u32) -> io::Result<PhysicalRect> {
    let left = i32::try_from(SOURCE_PADDING).expect("fixed padding fits in i32");
    let top = left;
    let right = i32::try_from(width)
        .ok()
        .and_then(|width| left.checked_add(width))
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "selection width is too large")
        })?;
    let bottom = i32::try_from(height)
        .ok()
        .and_then(|height| top.checked_add(height))
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "selection height is too large")
        })?;
    Ok(PhysicalRect {
        left,
        top,
        right,
        bottom,
    })
}

fn checked_pixel_layout(width: u32, height: u32) -> io::Result<(usize, usize)> {
    if width == 0 || height == 0 || width > i32::MAX as u32 || height > i32::MAX as u32 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "copy performance dimensions must be positive physical coordinates",
        ));
    }
    let stride = usize::try_from(width)
        .ok()
        .and_then(|width| width.checked_mul(4))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "selection stride overflow"))?;
    let pixel_bytes = stride
        .checked_mul(height as usize)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "selection size overflow"))?;
    Ok((stride, pixel_bytes))
}

/// Uses nearest-rank percentiles, matching the other release performance reports.
fn percentile(sorted: &[f64], percentile: usize) -> f64 {
    let index = (sorted.len() * percentile).div_ceil(100).saturating_sub(1);
    sorted[index.min(sorted.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::{CopyPerformanceConfig, build_report, percentile, validate_config};

    #[test]
    fn defaults_require_explicit_clipboard_permission_and_match_the_requirement() {
        let config = CopyPerformanceConfig::default();
        assert!(!config.allow_system_clipboard);
        assert_eq!(config.iterations, 30);
        assert_eq!((config.width, config.height), (1280, 720));
        assert_eq!(config.max_p95_ms, Some(250));
        assert_eq!(config.metrics_directory, None);
    }

    #[test]
    fn report_uses_nearest_rank_p95_and_passes_at_the_limit() {
        let config = CopyPerformanceConfig {
            allow_system_clipboard: true,
            max_p95_ms: Some(95),
            ..CopyPerformanceConfig::default()
        };
        let samples: Vec<f64> = (1..=100).map(|value| value as f64).collect();

        let report = build_report(&config, samples).unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&report.to_pretty_json().unwrap()).unwrap();

        assert_eq!(value["metric"], "copy_click_to_clipboard_readable");
        assert_eq!(value["latency_ms"]["p95"], 95.0);
        assert_eq!(value["thresholds"]["max_p95_ms"], 95);
        assert_eq!(value["system_clipboard_mutated"], true);
        assert!(report.passed());
    }

    #[test]
    fn report_fails_when_p95_exceeds_the_limit() {
        let config = CopyPerformanceConfig {
            max_p95_ms: Some(94),
            ..CopyPerformanceConfig::default()
        };
        let samples: Vec<f64> = (1..=100).map(|value| value as f64).collect();

        let report = build_report(&config, samples).unwrap();

        assert_eq!(
            percentile(&(1..=100).map(|value| value as f64).collect::<Vec<_>>(), 95,),
            95.0
        );
        assert!(!report.passed());
    }

    #[test]
    fn invalid_configs_are_rejected_before_clipboard_access() {
        let error = validate_config(&CopyPerformanceConfig {
            iterations: 0,
            ..CopyPerformanceConfig::default()
        })
        .unwrap_err();
        assert!(error.to_string().contains("iterations"));

        let error = validate_config(&CopyPerformanceConfig {
            width: 0,
            ..CopyPerformanceConfig::default()
        })
        .unwrap_err();
        assert!(error.to_string().contains("dimensions"));
    }

    #[test]
    fn run_refuses_system_clipboard_access_without_explicit_permission() {
        let error = super::run(&CopyPerformanceConfig {
            iterations: 1,
            width: 2,
            height: 2,
            ..CopyPerformanceConfig::default()
        })
        .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("--allow-system-clipboard"));
    }
}
