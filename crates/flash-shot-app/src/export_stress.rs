//! Repeatable full-frame long-image export-preparation benchmark.

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{
    domain::{annotation::AnnotationDocument, geometry::PhysicalRect},
    platform::capture::{CaptureFrame, PixelFormat},
};

pub const DEFAULT_WIDTH: u32 = 1_440;
pub const DEFAULT_HEIGHT: u32 = 6_000;
const DEFAULT_ITERATIONS: usize = 30;
const WARMUP_ITERATIONS: usize = 2;
const DEFAULT_MAX_ADDITIONAL_COPIES: u32 = 0;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportStressConfig {
    pub width: u32,
    pub height: u32,
    pub iterations: usize,
    pub output: Option<PathBuf>,
    pub max_p95_ms: Option<u64>,
    pub max_additional_copies: u32,
    pub require_pixel_reuse: bool,
}

impl Default for ExportStressConfig {
    fn default() -> Self {
        Self {
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            iterations: DEFAULT_ITERATIONS,
            output: None,
            max_p95_ms: None,
            max_additional_copies: DEFAULT_MAX_ADDITIONAL_COPIES,
            require_pixel_reuse: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ExportStressReport {
    value: serde_json::Value,
    passed: bool,
}

impl ExportStressReport {
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

/// Measures the real full-frame export-preparation path and returns its gateable JSON report.
///
/// Frame construction and the single final fingerprint scan stay outside the timed samples, so
/// each sample measures only annotation compositing followed by the full-bounds crop.
pub fn run(config: &ExportStressConfig) -> io::Result<ExportStressReport> {
    validate_config(config)?;
    let frame = benchmark_frame(config.width, config.height)?;
    let document = AnnotationDocument::new(frame.bounds).map_err(io::Error::other)?;

    for _ in 0..WARMUP_ITERATIONS {
        prepare_full_frame_export(&frame, &document)?;
    }

    let mut samples = Vec::with_capacity(config.iterations);
    let mut final_prepared = None;
    for iteration in 0..config.iterations {
        let started = Instant::now();
        let prepared = prepare_full_frame_export(&frame, &document)?;
        samples.push(started.elapsed().as_secs_f64() * 1_000.0);

        // Only the last output is retained for metadata and hashing. Earlier outputs are dropped
        // before the next iteration, matching the peak memory shape of one export preparation.
        if iteration + 1 == config.iterations {
            final_prepared = Some(prepared);
        }
    }
    let prepared = final_prepared.expect("validated iterations always produce a final frame");

    samples.sort_by(f64::total_cmp);
    let p95_ms = percentile(&samples, 95);
    let source_pixel_bytes = u64::try_from(frame.pixels.len())
        .map_err(|_| io::Error::other("source pixel byte count does not fit in u64"))?;
    let additional_cpu_copies = prepared
        .cpu_copy_count
        .checked_sub(frame.cpu_copy_count)
        .ok_or_else(|| io::Error::other("prepared frame has fewer CPU copies than its source"))?;
    // Both operations cover the full bounds, so every reported extra CPU copy materializes one
    // source-sized pixel buffer. This estimates copy traffic, not allocator overhead or PNG I/O.
    let estimated_intermediate_pixel_bytes = source_pixel_bytes
        .checked_mul(u64::from(additional_cpu_copies))
        .ok_or_else(|| io::Error::other("intermediate pixel byte estimate overflow"))?;
    let arc_pixel_reused = Arc::ptr_eq(&frame.pixels, &prepared.pixels);
    // An empty document and a full-frame crop must preserve every BGRA byte, even when the Arc is
    // not reused. Keep this correctness scan outside the latency samples.
    let pixel_identity_passed = frame.pixels == prepared.pixels;

    let latency_passed = config.max_p95_ms.is_none_or(|limit| p95_ms <= limit as f64);
    let copy_count_passed = additional_cpu_copies <= config.max_additional_copies;
    let pixel_reuse_passed = !config.require_pixel_reuse || arc_pixel_reused;
    let passed = latency_passed && copy_count_passed && pixel_reuse_passed && pixel_identity_passed;

    // Hash exactly once after all timed iterations; scanning each full output would distort the
    // benchmark and turn the fingerprint into part of the measured workload.
    let fingerprint = fnv1a64(&prepared.pixels);
    Ok(ExportStressReport {
        value: serde_json::json!({
            "schema_version": 1,
            "test": "full_frame_long_image_export_preparation_stress",
            "passed": passed,
            "iterations": config.iterations,
            "warmup_iterations": WARMUP_ITERATIONS,
            "frame": {
                "width": frame.width,
                "height": frame.height,
                "source_pixel_bytes": source_pixel_bytes,
                "annotation_count": document.annotations().len(),
            },
            "latency_ms": {
                "min": samples[0],
                "p50": percentile(&samples, 50),
                "p95": p95_ms,
                "max": samples[samples.len() - 1],
            },
            "export_preparation": {
                "source_cpu_copy_count": frame.cpu_copy_count,
                "prepared_cpu_copy_count": prepared.cpu_copy_count,
                "additional_cpu_copies": additional_cpu_copies,
                "estimated_intermediate_pixel_bytes": estimated_intermediate_pixel_bytes,
                "arc_pixel_reused": arc_pixel_reused,
                "pixel_identity_passed": pixel_identity_passed,
            },
            "pixel_fingerprint_fnv1a64": fingerprint,
            "thresholds": {
                "max_p95_ms": config.max_p95_ms,
                "max_additional_copies": config.max_additional_copies,
                "require_pixel_reuse": config.require_pixel_reuse,
            },
            "gates": {
                "latency_passed": latency_passed,
                "copy_count_passed": copy_count_passed,
                "pixel_reuse_passed": pixel_reuse_passed,
                "pixel_identity_passed": pixel_identity_passed,
            },
        }),
        passed,
    })
}

/// Rejects dimensions that cannot be represented by capture geometry or a BGRA pixel buffer.
fn validate_config(config: &ExportStressConfig) -> io::Result<()> {
    if config.iterations == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "export stress iterations must be greater than zero",
        ));
    }
    if config.width == 0 || config.height == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "export stress dimensions must be greater than zero",
        ));
    }
    if config.width > i32::MAX as u32 || config.height > i32::MAX as u32 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "export stress dimensions must fit in physical capture coordinates",
        ));
    }
    checked_pixel_layout(config.width, config.height)?;
    Ok(())
}

/// Builds one deterministic BGRA source frame shared by every warmup and measured iteration.
fn benchmark_frame(width: u32, height: u32) -> io::Result<CaptureFrame> {
    let (stride, pixel_bytes) = checked_pixel_layout(width, height)?;
    let mut pixels = vec![0_u8; pixel_bytes];
    for (y, row) in pixels.chunks_exact_mut(stride).enumerate() {
        for (x, pixel) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            pixel.copy_from_slice(&[
                x as u8,
                y as u8,
                x.wrapping_mul(31).wrapping_add(y.wrapping_mul(17)) as u8,
                255,
            ]);
        }
    }

    Ok(CaptureFrame {
        bounds: PhysicalRect {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        },
        width,
        height,
        stride,
        format: PixelFormat::Bgra8,
        pixels: pixels.into(),
        capture_duration: Duration::ZERO,
        cpu_copy_count: 1,
    })
}

/// Runs the same empty-document composite and full-bounds crop used to prepare an export frame.
fn prepare_full_frame_export(
    frame: &CaptureFrame,
    document: &AnnotationDocument,
) -> io::Result<CaptureFrame> {
    frame.composite_annotations(document)?.crop(frame.bounds)
}

/// Calculates stride and total allocation size without allowing integer wraparound.
fn checked_pixel_layout(width: u32, height: u32) -> io::Result<(usize, usize)> {
    let width = usize::try_from(width)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "width does not fit in usize"))?;
    let height = usize::try_from(height)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "height does not fit in usize"))?;
    let stride = width
        .checked_mul(4)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "frame stride overflow"))?;
    let pixel_bytes = stride
        .checked_mul(height)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "frame size overflow"))?;
    Ok((stride, pixel_bytes))
}

/// Uses the nearest-rank percentile, matching the repository's other stress reports.
fn percentile(sorted: &[f64], percentile: usize) -> f64 {
    let index = (sorted.len() * percentile).div_ceil(100).saturating_sub(1);
    sorted[index.min(sorted.len() - 1)]
}

/// Produces a stable pixel identity without introducing another dependency or timed scan.
fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_HEIGHT, DEFAULT_WIDTH, ExportStressConfig, percentile, run};

    #[test]
    fn defaults_target_a_long_full_frame_without_a_machine_specific_latency_gate() {
        let config = ExportStressConfig::default();
        assert_eq!(config.width, DEFAULT_WIDTH);
        assert_eq!(config.height, DEFAULT_HEIGHT);
        assert_eq!(config.iterations, 30);
        assert_eq!(config.max_p95_ms, None);
        assert_eq!(config.max_additional_copies, 0);
        assert!(config.require_pixel_reuse);
    }

    #[test]
    fn small_report_measures_the_full_export_preparation_path() {
        let report = run(&ExportStressConfig {
            width: 8,
            height: 6,
            iterations: 2,
            ..ExportStressConfig::default()
        })
        .unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&report.to_pretty_json().unwrap()).unwrap();

        assert_eq!(value["frame"]["width"], 8);
        assert_eq!(value["frame"]["height"], 6);
        assert_eq!(value["frame"]["source_pixel_bytes"], 8 * 6 * 4);
        assert_eq!(value["export_preparation"]["additional_cpu_copies"], 0);
        assert_eq!(
            value["export_preparation"]["estimated_intermediate_pixel_bytes"],
            0
        );
        assert_eq!(value["export_preparation"]["arc_pixel_reused"], true);
        assert_eq!(value["export_preparation"]["pixel_identity_passed"], true);
        assert!(value["pixel_fingerprint_fnv1a64"].as_u64().is_some());
        assert!(report.passed());
    }

    #[test]
    fn percentile_uses_nearest_rank() {
        let samples: Vec<f64> = (1..=100).map(|value| value as f64).collect();
        assert_eq!(percentile(&samples, 50), 50.0);
        assert_eq!(percentile(&samples, 95), 95.0);
    }
}
