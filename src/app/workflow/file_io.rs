//! Screenshot export naming and editable-project file I/O.

use super::*;

/// Prepares a selection Copy and commits it only while cancellation has not won the race.
///
/// Pixel compositing is intentionally done before the final atomic check: Escape can discard the
/// prepared image without changing the user's clipboard, but a native clipboard write itself is
/// not safely interruptible once it begins.
pub(super) fn copy_selection_snapshot_cancellable(
    frame: &CaptureFrame,
    document: &AnnotationDocument,
    selection: PhysicalRect,
    clipboard: &(impl ClipboardService + ?Sized),
    cancellation: &crate::app::SelectionCopyCancellation,
) -> std::io::Result<bool> {
    if cancellation.is_cancelled() {
        return Ok(false);
    }
    let copied = frame.composite_annotations(document)?.crop(selection)?;
    clipboard.copy_image_cancellable(&copied, cancellation)
}

pub(super) fn save_annotated_frame_selection(
    frame: &CaptureFrame,
    document: &AnnotationDocument,
    selection: PhysicalRect,
    path: PathBuf,
) -> std::io::Result<()> {
    frame
        .composite_annotations(document)?
        .crop(selection)?
        .save_image(path)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ImageTimestamp {
    pub(super) year: u16,
    pub(super) month: u16,
    pub(super) day: u16,
    pub(super) hour: u16,
    pub(super) minute: u16,
    pub(super) second: u16,
    pub(super) millisecond: u16,
}

/// Builds the native Save dialog's collision-resistant default image name.
///
/// The timestamp is local time with millisecond precision, while the UUIDv7 keeps two captures
/// created in the same millisecond distinct without relying on a filesystem collision suffix.
pub(super) fn default_image_filename(export_format: u8) -> String {
    generated_image_filename(
        crate::settings::DEFAULT_SAVE_PREFIX,
        export_extension(export_format),
    )
}

/// Builds the lossless PNG name shared by quick saves and editable-project exports.
///
/// The default prefix is the product name. A local timestamp makes files easy to scan, while a
/// UUIDv7 prevents same-millisecond captures from sharing a filename.
pub(super) fn default_png_image_filename() -> String {
    generated_image_filename(crate::settings::DEFAULT_SAVE_PREFIX, "png")
}

/// Combines a safe name prefix with the current local timestamp and a UUIDv7.
fn generated_image_filename(software_name: &str, extension: &str) -> String {
    format_default_image_filename(
        software_name,
        local_image_timestamp(),
        uuid::Uuid::now_v7(),
        extension,
    )
}

fn format_default_image_filename(
    software_name: &str,
    timestamp: ImageTimestamp,
    uuid: uuid::Uuid,
    extension: &str,
) -> String {
    format!(
        "{software_name}{:04}{:02}{:02}{:02}{:02}{:02}{:03}{uuid}.{extension}",
        timestamp.year,
        timestamp.month,
        timestamp.day,
        timestamp.hour,
        timestamp.minute,
        timestamp.second,
        timestamp.millisecond,
    )
}

#[cfg(windows)]
fn local_image_timestamp() -> ImageTimestamp {
    use windows_sys::Win32::{Foundation::SYSTEMTIME, System::SystemInformation::GetLocalTime};

    let mut system_time = SYSTEMTIME::default();
    // SAFETY: GetLocalTime writes a SYSTEMTIME into the valid mutable pointer we provide.
    unsafe { GetLocalTime(&mut system_time) };
    ImageTimestamp {
        year: system_time.wYear,
        month: system_time.wMonth,
        day: system_time.wDay,
        hour: system_time.wHour,
        minute: system_time.wMinute,
        second: system_time.wSecond,
        millisecond: system_time.wMilliseconds,
    }
}

#[cfg(not(windows))]
fn local_image_timestamp() -> ImageTimestamp {
    let timestamp_ms = unix_timestamp_ms();
    let days = (timestamp_ms / 86_400_000) as i64;
    let day_ms = timestamp_ms % 86_400_000;
    let (year, month, day) = civil_date_from_days(days);
    ImageTimestamp {
        year,
        month,
        day,
        hour: (day_ms / 3_600_000) as u16,
        minute: ((day_ms / 60_000) % 60) as u16,
        second: ((day_ms / 1_000) % 60) as u16,
        millisecond: (day_ms % 1_000) as u16,
    }
}

#[cfg(not(windows))]
fn civil_date_from_days(days_since_unix_epoch: i64) -> (u16, u16, u16) {
    let z = days_since_unix_epoch + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    let year = year + if month <= 2 { 1 } else { 0 };
    (year as u16, month as u16, day as u16)
}

/// Writes a quick save using the active history root and persisted filename prefix.
pub(super) fn quick_save_annotated_frame_selection_with_prefix(
    frame: &CaptureFrame,
    document: &AnnotationDocument,
    selection: PhysicalRect,
    directory: &Path,
    prefix: &str,
) -> std::io::Result<PathBuf> {
    quick_save_annotated_frame_selection_in_with_prefix(
        frame,
        document,
        selection,
        directory,
        prefix,
        local_image_timestamp(),
        uuid::Uuid::now_v7(),
    )
}

/// Retries a quick save in a managed fallback directory when the selected root is no longer
/// writable. The closure owns the actual image encoding so every save variant shares this rule.
pub(super) fn quick_save_with_fallback(
    directory: &Path,
    fallback_directory: Option<&Path>,
    save: impl Fn(&Path) -> std::io::Result<PathBuf>,
) -> std::io::Result<PathBuf> {
    match save(directory) {
        Ok(path) => Ok(path),
        Err(primary_error) => {
            let Some(fallback_directory) = fallback_directory else {
                return Err(primary_error);
            };
            if fallback_directory == directory {
                return Err(primary_error);
            }
            save(fallback_directory).map_err(|fallback_error| {
                std::io::Error::other(format!(
                    "selected quick-save folder failed: {primary_error}; fallback folder failed: {fallback_error}"
                ))
            })
        }
    }
}

/// Resolves the managed Pictures/Flash Shot root unless it is already the active root.
pub(super) fn managed_history_fallback(preferred: &Path) -> Option<PathBuf> {
    let managed = crate::history::managed_history_directory().ok()?;
    let managed = managed.canonicalize().unwrap_or(managed);
    (managed != preferred).then_some(managed)
}

pub(super) fn quick_save_annotated_frame_selection_with_fallback(
    frame: &CaptureFrame,
    document: &AnnotationDocument,
    selection: PhysicalRect,
    directory: &Path,
    fallback_directory: Option<&Path>,
    prefix: &str,
) -> std::io::Result<PathBuf> {
    quick_save_with_fallback(directory, fallback_directory, |directory| {
        quick_save_annotated_frame_selection_with_prefix(
            frame, document, selection, directory, prefix,
        )
    })
}

/// Writes a full capture into the active history root using the persisted filename prefix.
pub(super) fn quick_save_full_screen_frame_with_prefix(
    frame: &CaptureFrame,
    directory: &Path,
    prefix: &str,
) -> std::io::Result<PathBuf> {
    quick_save_full_screen_frame_in_with_prefix(
        frame,
        directory,
        prefix,
        local_image_timestamp(),
        uuid::Uuid::now_v7(),
    )
}

pub(super) fn quick_save_full_screen_frame_with_fallback(
    frame: &CaptureFrame,
    directory: &Path,
    fallback_directory: Option<&Path>,
    prefix: &str,
) -> std::io::Result<PathBuf> {
    quick_save_with_fallback(directory, fallback_directory, |directory| {
        quick_save_full_screen_frame_with_prefix(frame, directory, prefix)
    })
}

pub(super) fn quick_save_full_screen_frame_in_with_prefix(
    frame: &CaptureFrame,
    directory: &Path,
    prefix: &str,
    timestamp: ImageTimestamp,
    uuid: uuid::Uuid,
) -> std::io::Result<PathBuf> {
    let path = reserve_quick_save_path(directory, prefix, timestamp, uuid)?;
    match frame.save_png(&path) {
        Ok(()) => Ok(path),
        Err(error) => {
            let _ = std::fs::remove_file(&path);
            Err(error)
        }
    }
}

pub(super) fn quick_save_annotated_frame_selection_in_with_prefix(
    frame: &CaptureFrame,
    document: &AnnotationDocument,
    selection: PhysicalRect,
    directory: &Path,
    prefix: &str,
    timestamp: ImageTimestamp,
    uuid: uuid::Uuid,
) -> std::io::Result<PathBuf> {
    let path = reserve_quick_save_path(directory, prefix, timestamp, uuid)?;
    match save_annotated_frame_selection(frame, document, selection, path.clone()) {
        Ok(()) => Ok(path),
        Err(error) => {
            let _ = std::fs::remove_file(&path);
            Err(error)
        }
    }
}

/// Atomically reserves a generated PNG name before an encoder starts writing.
///
/// A UUIDv7 makes collisions exceptionally unlikely. `create_new` remains the final filesystem
/// authority and retries with a new UUIDv7 if a pre-existing file has the same generated name.
pub(super) fn reserve_quick_save_path(
    directory: &Path,
    prefix: &str,
    timestamp: ImageTimestamp,
    mut generated_uuid: uuid::Uuid,
) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(directory)?;
    loop {
        let candidate = directory.join(format_default_image_filename(
            prefix,
            timestamp,
            generated_uuid,
            "png",
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => {
                drop(file);
                return Ok(candidate);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                generated_uuid = uuid::Uuid::now_v7();
            }
            Err(error) => return Err(error),
        }
    }
}

pub(super) fn unix_timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

pub(super) fn export_path(mut path: PathBuf) -> PathBuf {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    if !matches!(extension.as_deref(), Some("png" | "jpg" | "jpeg" | "webp")) {
        path.set_extension("png");
    }
    path
}

pub(super) const fn export_extension(format: u8) -> &'static str {
    match format {
        1 => "jpg",
        2 => "webp",
        _ => "png",
    }
}

/// Keeps editable-project images lossless even when a caller chooses another extension.
pub(super) fn png_path(mut path: PathBuf) -> PathBuf {
    let is_png = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("png"));
    if !is_png {
        path.set_extension("png");
    }
    path
}

pub(super) fn annotation_document_path(mut path: PathBuf) -> PathBuf {
    let is_json = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"));
    if !is_json {
        path.set_extension("annotations.json");
    }
    path
}

pub(super) fn annotation_sidecar_path(image_path: &Path) -> PathBuf {
    image_path.with_extension("annotations.json")
}

pub(super) fn save_annotation_document(
    document: &AnnotationDocument,
    path: PathBuf,
) -> std::io::Result<()> {
    let json = document.to_json().map_err(std::io::Error::other)?;
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    let mut file = std::fs::File::create(&temporary)?;
    use std::io::Write;
    file.write_all(json.as_bytes())?;
    file.sync_all()?;
    drop(file);
    crate::image::replace_file(&temporary, &path)
}

pub(super) fn save_editable_project(
    frame: &CaptureFrame,
    document: &AnnotationDocument,
    image_path: PathBuf,
) -> std::io::Result<()> {
    let local_bounds = PhysicalRect {
        left: 0,
        top: 0,
        right: i32::try_from(frame.width).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "frame width overflow")
        })?,
        bottom: i32::try_from(frame.height).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "frame height overflow")
        })?,
    };
    let local_document = document
        .rebased_to(local_bounds)
        .map_err(std::io::Error::other)?;
    frame.save_png(&image_path)?;
    save_annotation_document(&local_document, annotation_sidecar_path(&image_path))
}

pub(super) fn load_annotation_document(
    path: &Path,
    expected_canvas: PhysicalRect,
) -> std::io::Result<AnnotationDocument> {
    let json = std::fs::read_to_string(path)?;
    let document = AnnotationDocument::from_json(&json).map_err(std::io::Error::other)?;
    if document.canvas_bounds() != expected_canvas {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "annotation document canvas does not match the current screenshot",
        ));
    }
    Ok(document)
}

pub(super) fn open_image_project(
    path: &Path,
) -> std::io::Result<(
    PathBuf,
    CaptureFrame,
    Option<AnnotationDocument>,
    Option<String>,
)> {
    let frame = CaptureFrame::open_png(path)?;
    let sidecar = annotation_sidecar_path(path);
    if !sidecar.exists() {
        return Ok((path.to_owned(), frame, None, None));
    }
    match load_annotation_document(&sidecar, frame.bounds) {
        Ok(document) => Ok((path.to_owned(), frame, Some(document), None)),
        Err(error) => Ok((
            path.to_owned(),
            frame,
            None,
            Some(format!("could not load {}: {error}", sidecar.display())),
        )),
    }
}

pub(super) fn open_annotation_project(
    path: &Path,
) -> std::io::Result<(PathBuf, CaptureFrame, AnnotationDocument)> {
    let image_path = project_image_path(path)?;
    let frame = CaptureFrame::open_png(&image_path)?;
    let document = load_annotation_document(path, frame.bounds)?;
    Ok((image_path, frame, document))
}

pub(super) fn project_image_path(sidecar_path: &Path) -> std::io::Result<PathBuf> {
    let filename = sidecar_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "annotation project has no file name",
            )
        })?;
    let stem = filename.strip_suffix(".annotations.json").ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "annotation project file must end with .annotations.json",
        )
    })?;
    Ok(sidecar_path.with_file_name(format!("{stem}.png")))
}

pub(super) fn next_annotation_counters(document: &AnnotationDocument) -> (u64, u32) {
    let next_id = document
        .annotations()
        .iter()
        .map(|annotation| annotation.id.value())
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let next_sequence = document
        .annotations()
        .iter()
        .filter_map(|annotation| match annotation.kind {
            AnnotationKind::Number { value, .. } => Some(value),
            _ => None,
        })
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    (next_id, next_sequence)
}

#[cfg(test)]
mod tests {
    use super::{ImageTimestamp, format_default_image_filename};

    #[test]
    fn default_image_filename_contains_local_timestamp_and_uuid_v7() {
        let uuid = uuid::Uuid::parse_str("018f2b50-7b2d-7cc0-8000-000000000000").unwrap();
        let name = format_default_image_filename(
            "FlashShot",
            ImageTimestamp {
                year: 2026,
                month: 8,
                day: 14,
                hour: 12,
                minute: 30,
                second: 45,
                millisecond: 987,
            },
            uuid,
            "png",
        );

        assert_eq!(
            name,
            "FlashShot20260814123045987018f2b50-7b2d-7cc0-8000-000000000000.png"
        );
        assert_eq!(uuid.get_version_num(), 7);
    }
}
