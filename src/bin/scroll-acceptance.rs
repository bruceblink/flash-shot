//! Deterministic acceptance probe for the scrolling screenshot stitcher.

use std::{io, path::PathBuf, sync::Arc, time::Duration};

use flash_shot::{
    domain::geometry::PhysicalRect,
    platform::capture::{CaptureFrame, PixelFormat},
    scroll::{ManualScrollCapture, OverlapOptions},
};

const FRAME_WIDTH: u32 = 96;
const FRAME_HEIGHT: u32 = 180;
const FRAME_OVERLAP: u32 = 90;
const FRAME_COUNT: u32 = 6;

#[derive(serde::Serialize)]
struct AcceptanceReport {
    frame_count: u32,
    overlaps: Vec<u32>,
    stitched_width: u32,
    stitched_height: u32,
    expected_height: u32,
    pixel_checksum: u64,
    passed: bool,
}

fn main() {
    if let Err(error) = execute() {
        eprintln!("scroll acceptance failed: {error}");
        std::process::exit(1);
    }
}

/// Runs a complete deterministic scrolling session and writes the result for CI or manual review.
fn execute() -> io::Result<()> {
    let output = parse_output(std::env::args().skip(1))?;
    let options = OverlapOptions {
        minimum_rows: FRAME_OVERLAP,
        max_mean_abs_difference: 0,
    };
    let mut capture = ManualScrollCapture::default();
    capture.begin(make_frame(0))?;
    for index in 1..FRAME_COUNT {
        let offset = index * (FRAME_HEIGHT - FRAME_OVERLAP);
        capture.append(make_frame(offset), options)?;
    }
    let stitched = capture.finish(options)?;
    let expected_height = FRAME_HEIGHT + (FRAME_COUNT - 1) * (FRAME_HEIGHT - FRAME_OVERLAP);
    let report = AcceptanceReport {
        frame_count: capture.frame_count() as u32,
        overlaps: stitched.overlaps.clone(),
        stitched_width: stitched.frame.width,
        stitched_height: stitched.frame.height,
        expected_height,
        pixel_checksum: pixel_checksum(&stitched.frame),
        passed: capture.frame_count() as u32 == FRAME_COUNT
            && stitched.frame.width == FRAME_WIDTH
            && stitched.frame.height == expected_height
            && stitched.overlaps == vec![FRAME_OVERLAP; (FRAME_COUNT - 1) as usize],
    };
    let encoded = serde_json::to_vec_pretty(&report).map_err(io::Error::other)?;
    println!("{}", String::from_utf8_lossy(&encoded));
    if let Some(output) = output {
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(output, encoded)?;
    }
    if report.passed {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "stitched output did not match the acceptance dimensions",
        ))
    }
}

fn parse_output(mut arguments: impl Iterator<Item = String>) -> io::Result<Option<PathBuf>> {
    let output = match arguments.next().as_deref() {
        None => None,
        Some("--output") => Some(PathBuf::from(arguments.next().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "--output requires a path")
        })?)),
        Some(argument) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unknown argument: {argument}"),
            ));
        }
    };
    if arguments.next().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "scroll acceptance accepts only --output <path>",
        ));
    }
    Ok(output)
}

/// Builds one viewport from a global page pattern so every overlap row is pixel-identical.
fn make_frame(top: u32) -> CaptureFrame {
    let stride = FRAME_WIDTH as usize * 4;
    let mut pixels = Vec::with_capacity(stride * FRAME_HEIGHT as usize);
    for row in 0..FRAME_HEIGHT {
        let page_row = top + row;
        for column in 0..FRAME_WIDTH {
            let value = ((page_row + column) % 251) as u8;
            pixels.extend_from_slice(&[value, value.wrapping_add(3), value.wrapping_add(7), 255]);
        }
    }
    CaptureFrame {
        bounds: PhysicalRect {
            left: -320,
            top: top as i32,
            right: -320 + FRAME_WIDTH as i32,
            bottom: top as i32 + FRAME_HEIGHT as i32,
        },
        width: FRAME_WIDTH,
        height: FRAME_HEIGHT,
        stride,
        format: PixelFormat::Bgra8,
        pixels: Arc::from(pixels),
        capture_duration: Duration::from_millis(2),
        cpu_copy_count: 1,
    }
}

fn pixel_checksum(frame: &CaptureFrame) -> u64 {
    frame.pixels.iter().fold(0_u64, |checksum, byte| {
        checksum
            .wrapping_mul(1_000_003)
            .wrapping_add(u64::from(*byte))
    })
}

#[cfg(test)]
mod tests {
    use super::{FRAME_COUNT, FRAME_HEIGHT, FRAME_OVERLAP, FRAME_WIDTH, make_frame, parse_output};

    #[test]
    fn output_argument_is_optional_and_bounded() {
        assert!(parse_output(std::iter::empty()).unwrap().is_none());
        assert_eq!(
            parse_output(["--output".to_owned(), "report.json".to_owned()].into_iter())
                .unwrap()
                .unwrap(),
            std::path::PathBuf::from("report.json")
        );
        assert!(parse_output(["--output".to_owned()].into_iter()).is_err());
        assert!(parse_output(["--unknown".to_owned()].into_iter()).is_err());
    }

    #[test]
    fn generated_viewports_have_the_expected_overlap_geometry() {
        let first = make_frame(0);
        let second = make_frame(FRAME_HEIGHT - FRAME_OVERLAP);
        assert_eq!(first.width, FRAME_WIDTH);
        assert_eq!(first.height, FRAME_HEIGHT);
        assert_eq!(second.bounds.top, (FRAME_HEIGHT - FRAME_OVERLAP) as i32);
        assert_eq!(FRAME_COUNT, 6);
    }
}
