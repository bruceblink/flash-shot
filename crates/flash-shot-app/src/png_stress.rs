//! Repeatable long-image PNG encoding benchmark with decoded-pixel verification.

use std::{
    fs,
    io::{self, Cursor},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use crate::{
    domain::geometry::PhysicalRect,
    platform::capture::{CaptureFrame, PixelFormat},
};

pub const DEFAULT_WIDTH: u32 = 1_440;
pub const DEFAULT_HEIGHT: u32 = 6_000;
const DEFAULT_ITERATIONS: usize = 30;
const WARMUP_ITERATIONS: usize = 2;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PngStressConfig {
    pub width: u32,
    pub height: u32,
    pub iterations: usize,
    pub output: Option<PathBuf>,
    pub max_p95_ms: Option<u64>,
}

impl Default for PngStressConfig {
    fn default() -> Self {
        Self {
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            iterations: DEFAULT_ITERATIONS,
            output: None,
            max_p95_ms: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PngStressReport {
    value: serde_json::Value,
    passed: bool,
}

impl PngStressReport {
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

/// Measures only PNG encoding; frame creation and the final decode stay outside timed samples.
pub fn run(config: &PngStressConfig) -> io::Result<PngStressReport> {
    validate_config(config)?;
    let frame = benchmark_frame(config.width, config.height)?;

    for _ in 0..WARMUP_ITERATIONS {
        std::hint::black_box(frame.encode_png()?);
    }

    let mut samples = Vec::with_capacity(config.iterations);
    let mut final_png = None;
    for iteration in 0..config.iterations {
        let started = Instant::now();
        let encoded = frame.encode_png()?;
        samples.push(started.elapsed().as_secs_f64() * 1_000.0);
        if iteration + 1 == config.iterations {
            final_png = Some(encoded);
        } else {
            std::hint::black_box(encoded);
        }
    }
    let encoded = final_png.expect("validated iterations always produce a final PNG");

    samples.sort_by(f64::total_cmp);
    let p95_ms = percentile(&samples, 95);
    let decoded = decode_png(&encoded)?;
    let source_fingerprint = rgba_fingerprint_from_bgra(&frame);
    let decoded_fingerprint = fnv1a64(&decoded.rgba);
    let pixel_identity_passed = decoded.width == frame.width
        && decoded.height == frame.height
        && decoded.rgba.len() == frame.width as usize * frame.height as usize * 4
        && bgra_matches_rgba(&frame, &decoded.rgba);
    let latency_passed = config.max_p95_ms.is_none_or(|limit| p95_ms <= limit as f64);
    let passed =
        latency_passed && pixel_identity_passed && source_fingerprint == decoded_fingerprint;

    Ok(PngStressReport {
        value: serde_json::json!({
            "schema_version": 1,
            "test": "long_image_png_encode_stress",
            "build_profile": if cfg!(debug_assertions) { "debug" } else { "release" },
            "passed": passed,
            "iterations": config.iterations,
            "warmup_iterations": WARMUP_ITERATIONS,
            "frame": {
                "width": frame.width,
                "height": frame.height,
                "source_pixel_bytes": frame.pixels.len(),
                "stride": frame.stride,
            },
            "png": {
                "encoded_bytes": encoded.len(),
                "decoded_width": decoded.width,
                "decoded_height": decoded.height,
                "source_rgba_fingerprint_fnv1a64": source_fingerprint,
                "decoded_rgba_fingerprint_fnv1a64": decoded_fingerprint,
                "pixel_identity_passed": pixel_identity_passed,
            },
            "latency_ms": {
                "min": samples[0],
                "p50": percentile(&samples, 50),
                "p95": p95_ms,
                "max": samples[samples.len() - 1],
            },
            "thresholds": { "max_p95_ms": config.max_p95_ms },
            "gates": {
                "latency_passed": latency_passed,
                "pixel_identity_passed": pixel_identity_passed,
                "fingerprint_passed": source_fingerprint == decoded_fingerprint,
            },
        }),
        passed,
    })
}

fn validate_config(config: &PngStressConfig) -> io::Result<()> {
    if config.iterations == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "PNG stress iterations must be greater than zero",
        ));
    }
    if config.width == 0 || config.height == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "PNG stress dimensions must be greater than zero",
        ));
    }
    if config.width > i32::MAX as u32 || config.height > i32::MAX as u32 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "PNG stress dimensions must fit in physical capture coordinates",
        ));
    }
    checked_pixel_layout(config.width, config.height)?;
    Ok(())
}

/// Builds one deterministic tight-stride BGRA image shared by every encoding sample.
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

fn checked_pixel_layout(width: u32, height: u32) -> io::Result<(usize, usize)> {
    let stride = usize::try_from(width)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "width does not fit in usize"))?
        .checked_mul(4)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "frame stride overflow"))?;
    let height = usize::try_from(height)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "height does not fit in usize"))?;
    let pixel_bytes = stride
        .checked_mul(height)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "frame size overflow"))?;
    Ok((stride, pixel_bytes))
}

struct DecodedPng {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

/// Decodes the final sample once so benchmark success always includes pixel correctness.
fn decode_png(encoded: &[u8]) -> io::Result<DecodedPng> {
    let decoder = png::Decoder::new(Cursor::new(encoded));
    let mut reader = decoder
        .read_info()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let output_size = reader
        .output_buffer_size()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "decoded PNG size overflow"))?;
    let mut rgba = vec![0_u8; output_size];
    let info = reader
        .next_frame(&mut rgba)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "encoded PNG did not decode as 8-bit RGBA",
        ));
    }
    rgba.truncate(info.buffer_size());
    Ok(DecodedPng {
        width: info.width,
        height: info.height,
        rgba,
    })
}

fn bgra_matches_rgba(frame: &CaptureFrame, rgba: &[u8]) -> bool {
    frame
        .pixels
        .chunks_exact(frame.stride)
        .flat_map(|row| row[..frame.width as usize * 4].as_chunks::<4>().0.iter())
        .zip(rgba.as_chunks::<4>().0.iter())
        .all(|(bgra, rgba)| *bgra == [rgba[2], rgba[1], rgba[0], rgba[3]])
}

fn rgba_fingerprint_from_bgra(frame: &CaptureFrame) -> u64 {
    frame
        .pixels
        .chunks_exact(frame.stride)
        .flat_map(|row| row[..frame.width as usize * 4].as_chunks::<4>().0.iter())
        .flat_map(|pixel| [pixel[2], pixel[1], pixel[0], pixel[3]])
        .fold(0xcbf29ce484222325_u64, fnv1a64_byte)
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .copied()
        .fold(0xcbf29ce484222325_u64, fnv1a64_byte)
}

fn fnv1a64_byte(hash: u64, byte: u8) -> u64 {
    (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
}

fn percentile(sorted: &[f64], percentile: usize) -> f64 {
    let index = (sorted.len() * percentile).div_ceil(100).saturating_sub(1);
    sorted[index.min(sorted.len() - 1)]
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::{DEFAULT_HEIGHT, DEFAULT_WIDTH, PngStressConfig, run};

    #[test]
    fn defaults_use_a_long_image_and_enough_samples_for_p95() {
        let config = PngStressConfig::default();
        assert_eq!(config.width, DEFAULT_WIDTH);
        assert_eq!(config.height, DEFAULT_HEIGHT);
        assert_eq!(config.iterations, 30);
        assert_eq!(config.max_p95_ms, None);
    }

    #[test]
    fn small_report_decodes_to_the_exact_source_pixels() {
        let report = run(&PngStressConfig {
            width: 8,
            height: 6,
            iterations: 2,
            ..PngStressConfig::default()
        })
        .unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&report.to_pretty_json().unwrap()).unwrap();

        assert_eq!(value["frame"]["source_pixel_bytes"], 8 * 6 * 4);
        assert_eq!(value["png"]["decoded_width"], 8);
        assert_eq!(value["png"]["decoded_height"], 6);
        assert_eq!(value["png"]["pixel_identity_passed"], true);
        assert_eq!(
            value["png"]["source_rgba_fingerprint_fnv1a64"],
            value["png"]["decoded_rgba_fingerprint_fnv1a64"]
        );
        assert!(report.passed());
    }

    #[test]
    fn zero_iterations_are_rejected_before_allocating_the_frame() {
        let error = run(&PngStressConfig {
            width: 8,
            height: 6,
            iterations: 0,
            ..PngStressConfig::default()
        })
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
}
