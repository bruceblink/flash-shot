//! Capture-frame data and image transformations independent from GPUI and Windows capture APIs.

mod frame;
mod image;

pub use frame::{CaptureFrame, PixelColor, PixelFormat};
pub use image::replace_file;
