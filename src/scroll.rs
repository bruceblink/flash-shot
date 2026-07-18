//! Pure physical-pixel overlap detection and vertical screenshot stitching.

use std::{io, sync::Arc};

use crate::{
    domain::geometry::PhysicalRect,
    platform::capture::{CaptureFrame, PixelFormat},
};

/// Matching limits for adjacent frames in a manual vertical scrolling session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OverlapOptions {
    /// Minimum number of shared rows that proves two frames belong together.
    pub minimum_rows: u32,
    /// Allowed average absolute channel difference per pixel byte in shared rows.
    pub max_mean_abs_difference: u8,
}

impl Default for OverlapOptions {
    fn default() -> Self {
        Self {
            minimum_rows: 16,
            max_mean_abs_difference: 6,
        }
    }
}

/// A stitched physical-pixel frame and the overlap removed before each appended frame.
#[derive(Clone, Debug)]
pub struct StitchedCapture {
    pub frame: CaptureFrame,
    pub overlaps: Vec<u32>,
}

/// Explicit lifecycle for user-driven manual scroll capture.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ManualScrollState {
    #[default]
    Idle,
    Collecting,
    Completed,
    Cancelled,
    Failed,
}

/// Collects compatible viewport frames until the user finishes a manual scroll session.
#[derive(Clone, Debug, Default)]
pub struct ManualScrollCapture {
    state: ManualScrollState,
    frames: Vec<CaptureFrame>,
    failure: Option<String>,
}

impl ManualScrollCapture {
    pub const fn state(&self) -> ManualScrollState {
        self.state
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    pub fn begin(&mut self, first: CaptureFrame) -> io::Result<()> {
        self.require(ManualScrollState::Idle, "begin")?;
        first.validate()?;
        self.frames = vec![first];
        self.failure = None;
        self.state = ManualScrollState::Collecting;
        Ok(())
    }

    /// Adds a frame only when it has a reliable overlap with the latest viewport frame.
    pub fn append(&mut self, frame: CaptureFrame, options: OverlapOptions) -> io::Result<u32> {
        self.require(ManualScrollState::Collecting, "append")?;
        let previous = self
            .frames
            .last()
            .expect("collecting sessions have a first frame");
        let overlap = match detect_vertical_overlap(previous, &frame, options) {
            Ok(overlap) => overlap,
            Err(error) => {
                self.state = ManualScrollState::Failed;
                self.failure = Some(error.to_string());
                return Err(error);
            }
        };
        self.frames.push(frame);
        Ok(overlap)
    }

    pub fn finish(&mut self, options: OverlapOptions) -> io::Result<StitchedCapture> {
        self.require(ManualScrollState::Collecting, "finish")?;
        match stitch_vertical(&self.frames, options) {
            Ok(stitched) => {
                self.state = ManualScrollState::Completed;
                Ok(stitched)
            }
            Err(error) => {
                self.state = ManualScrollState::Failed;
                self.failure = Some(error.to_string());
                Err(error)
            }
        }
    }

    pub fn cancel(&mut self) -> io::Result<()> {
        self.require(ManualScrollState::Collecting, "cancel")?;
        self.frames.clear();
        self.state = ManualScrollState::Cancelled;
        Ok(())
    }

    pub fn reset(&mut self) -> io::Result<()> {
        if !matches!(
            self.state,
            ManualScrollState::Completed | ManualScrollState::Cancelled | ManualScrollState::Failed
        ) {
            return Err(invalid_transition(self.state, "reset"));
        }
        *self = Self::default();
        Ok(())
    }

    fn require(&self, expected: ManualScrollState, operation: &'static str) -> io::Result<()> {
        if self.state == expected {
            Ok(())
        } else {
            Err(invalid_transition(self.state, operation))
        }
    }
}

/// Finds the largest suffix/prefix overlap between two vertically adjacent frames.
pub fn detect_vertical_overlap(
    upper: &CaptureFrame,
    lower: &CaptureFrame,
    options: OverlapOptions,
) -> io::Result<u32> {
    validate_pair(upper, lower, options)?;
    let maximum = upper.height.min(lower.height);
    for rows in (options.minimum_rows..=maximum).rev() {
        if mean_difference(upper, lower, rows)? <= options.max_mean_abs_difference {
            return Ok(rows);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "no reliable vertical overlap found",
    ))
}

/// Stitches a sequence of manually captured, vertically scrolling frames.
pub fn stitch_vertical(
    frames: &[CaptureFrame],
    options: OverlapOptions,
) -> io::Result<StitchedCapture> {
    let Some(first) = frames.first() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "at least one scroll frame is required",
        ));
    };
    first.validate()?;
    if first.format != PixelFormat::Bgra8 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "unsupported pixel format",
        ));
    }

    let row_bytes = first.width as usize * 4;
    let mut pixels = frame_rows(first, 0)?;
    let mut height = first.height;
    let mut overlaps = Vec::with_capacity(frames.len().saturating_sub(1));
    let mut capture_duration = first.capture_duration;
    let mut cpu_copy_count = first.cpu_copy_count;

    for pair in frames.windows(2) {
        let upper = &pair[0];
        let lower = &pair[1];
        let overlap = detect_vertical_overlap(upper, lower, options)?;
        let remaining_rows = lower.height.checked_sub(overlap).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "scroll overlap exceeds frame height",
            )
        })?;
        let additional_bytes = remaining_rows as usize * row_bytes;
        pixels.reserve(additional_bytes);
        pixels.extend(frame_rows(lower, overlap)?);
        height = height.checked_add(remaining_rows).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "stitched screenshot height overflow",
            )
        })?;
        overlaps.push(overlap);
        capture_duration = capture_duration.max(lower.capture_duration);
        cpu_copy_count = cpu_copy_count.saturating_add(lower.cpu_copy_count);
    }

    let frame = CaptureFrame {
        bounds: PhysicalRect {
            left: 0,
            top: 0,
            right: first.width as i32,
            bottom: height as i32,
        },
        width: first.width,
        height,
        stride: row_bytes,
        format: PixelFormat::Bgra8,
        pixels: Arc::from(pixels),
        capture_duration,
        cpu_copy_count: cpu_copy_count.saturating_add(1),
    };
    frame.validate()?;
    Ok(StitchedCapture { frame, overlaps })
}

fn validate_pair(
    upper: &CaptureFrame,
    lower: &CaptureFrame,
    options: OverlapOptions,
) -> io::Result<()> {
    upper.validate()?;
    lower.validate()?;
    if upper.format != PixelFormat::Bgra8 || lower.format != PixelFormat::Bgra8 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "unsupported pixel format",
        ));
    }
    if upper.width != lower.width {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "scroll frames must have the same width",
        ));
    }
    if options.minimum_rows == 0 || options.minimum_rows > upper.height.min(lower.height) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "scroll overlap minimum must fit inside both frames",
        ));
    }
    Ok(())
}

fn mean_difference(upper: &CaptureFrame, lower: &CaptureFrame, rows: u32) -> io::Result<u8> {
    let row_bytes = upper.width as usize * 4;
    let upper_start = (upper.height - rows) as usize * upper.stride;
    let lower_end = rows as usize * lower.stride;
    let mut difference = 0_u64;
    let mut bytes = 0_u64;
    for (upper_row, lower_row) in upper.pixels[upper_start..]
        .chunks_exact(upper.stride)
        .take(rows as usize)
        .zip(lower.pixels[..lower_end].chunks_exact(lower.stride))
    {
        for (upper, lower) in upper_row[..row_bytes].iter().zip(&lower_row[..row_bytes]) {
            difference += u64::from(upper.abs_diff(*lower));
            bytes += 1;
        }
    }
    let mean = difference.div_ceil(bytes.max(1));
    u8::try_from(mean)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "difference overflow"))
}

fn frame_rows(frame: &CaptureFrame, start_row: u32) -> io::Result<Vec<u8>> {
    let row_bytes = frame.width as usize * 4;
    let mut output = Vec::with_capacity((frame.height - start_row) as usize * row_bytes);
    for row in frame
        .pixels
        .chunks_exact(frame.stride)
        .skip(start_row as usize)
    {
        output.extend_from_slice(&row[..row_bytes]);
    }
    Ok(output)
}

fn invalid_transition(state: ManualScrollState, operation: &'static str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("cannot {operation} while manual scroll capture is {state:?}"),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        ManualScrollCapture, ManualScrollState, OverlapOptions, detect_vertical_overlap,
        stitch_vertical,
    };
    use crate::{
        domain::geometry::PhysicalRect,
        platform::capture::{CaptureFrame, PixelFormat},
    };
    use std::{sync::Arc, time::Duration};

    fn frame(rows: std::ops::Range<u8>) -> CaptureFrame {
        let height = rows.len() as u32;
        let mut pixels = Vec::new();
        for value in rows {
            pixels.extend_from_slice(&[value, value, value, 255]);
        }
        CaptureFrame {
            bounds: PhysicalRect {
                left: 100,
                top: 200,
                right: 101,
                bottom: 200 + height as i32,
            },
            width: 1,
            height,
            stride: 4,
            format: PixelFormat::Bgra8,
            pixels: Arc::from(pixels),
            capture_duration: Duration::from_millis(2),
            cpu_copy_count: 1,
        }
    }

    fn options() -> OverlapOptions {
        OverlapOptions {
            minimum_rows: 3,
            max_mean_abs_difference: 0,
        }
    }

    #[test]
    fn detects_the_largest_vertical_overlap() {
        assert_eq!(
            detect_vertical_overlap(&frame(0..10), &frame(6..16), options()).unwrap(),
            4
        );
    }

    #[test]
    fn stitches_frames_once_without_duplicate_overlap_rows() {
        let stitched = stitch_vertical(&[frame(0..10), frame(6..16)], options()).unwrap();

        assert_eq!(stitched.overlaps, [4]);
        assert_eq!(stitched.frame.height, 16);
        assert_eq!(
            stitched.frame.bounds,
            PhysicalRect {
                left: 0,
                top: 0,
                right: 1,
                bottom: 16
            }
        );
        let values: Vec<_> = stitched
            .frame
            .pixels
            .chunks_exact(4)
            .map(|pixel| pixel[0])
            .collect();
        assert_eq!(values, (0..16).collect::<Vec<_>>());
    }

    #[test]
    fn rejects_frames_without_a_reliable_overlap() {
        let error = detect_vertical_overlap(&frame(0..10), &frame(20..30), options()).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn rejects_mismatched_frame_widths() {
        let mut lower = frame(6..16);
        lower.width = 2;
        lower.stride = 8;
        lower.pixels = Arc::from(vec![0; 80]);
        lower.bounds.right = 102;

        let error = detect_vertical_overlap(&frame(0..10), &lower, options()).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn manual_capture_tracks_append_finish_and_reset_lifecycle() {
        let mut capture = ManualScrollCapture::default();

        capture.begin(frame(0..10)).unwrap();
        assert_eq!(capture.state(), ManualScrollState::Collecting);
        assert_eq!(capture.append(frame(6..16), options()).unwrap(), 4);
        let stitched = capture.finish(options()).unwrap();

        assert_eq!(stitched.frame.height, 16);
        assert_eq!(capture.state(), ManualScrollState::Completed);
        assert_eq!(capture.frame_count(), 2);
        capture.reset().unwrap();
        assert_eq!(capture.state(), ManualScrollState::Idle);
        assert_eq!(capture.frame_count(), 0);
    }

    #[test]
    fn manual_capture_records_overlap_failures_and_can_reset() {
        let mut capture = ManualScrollCapture::default();
        capture.begin(frame(0..10)).unwrap();

        assert!(capture.append(frame(20..30), options()).is_err());
        assert_eq!(capture.state(), ManualScrollState::Failed);
        assert!(capture.failure().is_some());
        capture.reset().unwrap();
        assert_eq!(capture.state(), ManualScrollState::Idle);
    }

    #[test]
    fn manual_capture_cancellation_discards_collected_frames() {
        let mut capture = ManualScrollCapture::default();
        capture.begin(frame(0..10)).unwrap();

        capture.cancel().unwrap();
        assert_eq!(capture.state(), ManualScrollState::Cancelled);
        assert_eq!(capture.frame_count(), 0);
        assert!(capture.finish(options()).is_err());
    }
}
