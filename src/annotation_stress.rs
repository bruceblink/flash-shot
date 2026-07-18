//! Repeatable 4K CPU annotation-composite benchmark with a stable output hash.

use std::{io, time::Instant};

use crate::{
    domain::{
        annotation::{
            Annotation, AnnotationCommand, AnnotationDocument, AnnotationId, AnnotationKind,
            AnnotationStyle, CommandHistory,
        },
        geometry::{PhysicalPoint, PhysicalRect},
    },
    platform::capture::{CaptureFrame, PixelFormat},
};

pub const WIDTH: u32 = 3_840;
pub const HEIGHT: u32 = 2_160;
const WARMUP_ITERATIONS: usize = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AnnotationStressConfig {
    pub iterations: usize,
    pub max_p95_ms: Option<u64>,
}

impl Default for AnnotationStressConfig {
    fn default() -> Self {
        Self {
            iterations: 30,
            max_p95_ms: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AnnotationStressReport {
    value: serde_json::Value,
    passed: bool,
}

impl AnnotationStressReport {
    pub const fn passed(&self) -> bool {
        self.passed
    }

    pub fn to_pretty_json(&self) -> io::Result<String> {
        serde_json::to_string_pretty(&self.value).map_err(io::Error::other)
    }
}

pub fn run(config: AnnotationStressConfig) -> io::Result<AnnotationStressReport> {
    if config.iterations == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "iterations must be greater than zero",
        ));
    }
    let frame = benchmark_frame();
    let document = benchmark_document(frame.bounds)?;
    for _ in 0..WARMUP_ITERATIONS {
        let _ = frame.composite_annotations(&document)?;
    }
    let mut samples = Vec::with_capacity(config.iterations);
    let mut fingerprint = 0;
    for _ in 0..config.iterations {
        let started = Instant::now();
        let composited = frame.composite_annotations(&document)?;
        samples.push(started.elapsed().as_secs_f64() * 1_000.0);
        fingerprint = fnv1a64(&composited.pixels);
    }
    samples.sort_by(f64::total_cmp);
    let p95 = percentile(&samples, 95);
    let passed = config.max_p95_ms.is_none_or(|limit| p95 <= limit as f64);
    Ok(AnnotationStressReport {
        value: serde_json::json!({
            "schema_version": 1,
            "test": "annotation_composite_4k_stress",
            "passed": passed,
            "iterations": config.iterations,
            "warmup_iterations": WARMUP_ITERATIONS,
            "frame": { "width": WIDTH, "height": HEIGHT, "annotation_count": document.annotations().len() },
            "latency_ms": { "min": samples[0], "p50": percentile(&samples, 50), "p95": p95, "max": samples[samples.len() - 1] },
            "limit_ms": config.max_p95_ms,
            "pixel_fingerprint_fnv1a64": fingerprint,
        }),
        passed,
    })
}

fn benchmark_frame() -> CaptureFrame {
    let pixels = (0..HEIGHT)
        .flat_map(|y| {
            (0..WIDTH)
                .flat_map(move |x| [(x % 251) as u8, (y % 251) as u8, ((x + y) % 251) as u8, 255])
        })
        .collect::<Vec<_>>();
    CaptureFrame {
        bounds: PhysicalRect {
            left: 0,
            top: 0,
            right: WIDTH as i32,
            bottom: HEIGHT as i32,
        },
        width: WIDTH,
        height: HEIGHT,
        stride: WIDTH as usize * 4,
        format: PixelFormat::Bgra8,
        pixels: pixels.into(),
        capture_duration: std::time::Duration::ZERO,
        cpu_copy_count: 1,
    }
}

fn benchmark_document(bounds: PhysicalRect) -> io::Result<AnnotationDocument> {
    let mut document = AnnotationDocument::new(bounds).map_err(io::Error::other)?;
    let mut history = CommandHistory::default();
    let style = AnnotationStyle {
        stroke_rgba: 0xFF3B30FF,
        fill_rgba: Some(0xFF3B3044),
        stroke_width: 4,
    };
    for (id, kind) in [
        AnnotationKind::Rectangle {
            bounds: PhysicalRect {
                left: 300,
                top: 250,
                right: 1700,
                bottom: 900,
            },
        },
        AnnotationKind::Ellipse {
            bounds: PhysicalRect {
                left: 1900,
                top: 400,
                right: 3300,
                bottom: 1500,
            },
        },
        AnnotationKind::Arrow {
            start: PhysicalPoint { x: 250, y: 1900 },
            end: PhysicalPoint { x: 3500, y: 300 },
        },
        AnnotationKind::Freehand {
            points: vec![
                PhysicalPoint { x: 400, y: 1400 },
                PhysicalPoint { x: 900, y: 1200 },
                PhysicalPoint { x: 1400, y: 1600 },
                PhysicalPoint { x: 2000, y: 1300 },
                PhysicalPoint { x: 2800, y: 1700 },
            ],
        },
    ]
    .into_iter()
    .enumerate()
    {
        history
            .apply(
                &mut document,
                AnnotationCommand::Insert(Annotation {
                    id: AnnotationId::new((id + 1) as u64),
                    kind,
                    style,
                }),
            )
            .map_err(io::Error::other)?;
    }
    Ok(document)
}

fn percentile(sorted: &[f64], percentile: usize) -> f64 {
    let index = (sorted.len() * percentile).div_ceil(100).saturating_sub(1);
    sorted[index.min(sorted.len() - 1)]
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

#[cfg(test)]
mod tests {
    use super::{AnnotationStressConfig, HEIGHT, WIDTH, run};

    #[test]
    fn report_has_4k_dimensions_and_a_stable_fingerprint() {
        let report = run(AnnotationStressConfig {
            iterations: 1,
            max_p95_ms: None,
        })
        .unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&report.to_pretty_json().unwrap()).unwrap();
        assert_eq!(value["frame"]["width"], WIDTH);
        assert_eq!(value["frame"]["height"], HEIGHT);
        assert!(value["pixel_fingerprint_fnv1a64"].as_u64().is_some());
        assert!(report.passed());
    }
}
