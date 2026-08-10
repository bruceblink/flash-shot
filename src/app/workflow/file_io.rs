//! Screenshot export naming and editable-project file I/O.

use super::*;

pub(super) fn copy_annotated_frame_selection(
    frame: &CaptureFrame,
    document: &AnnotationDocument,
    selection: PhysicalRect,
    clipboard: &impl ClipboardService,
) -> std::io::Result<()> {
    clipboard.copy_image(&frame.composite_annotations(document)?.crop(selection)?)
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
        unix_timestamp_ms(),
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
    quick_save_full_screen_frame_in_with_prefix(frame, directory, prefix, unix_timestamp_ms())
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
    timestamp_ms: u128,
) -> std::io::Result<PathBuf> {
    let path = reserve_quick_save_path(directory, prefix, timestamp_ms)?;
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
    timestamp_ms: u128,
) -> std::io::Result<PathBuf> {
    let path = reserve_quick_save_path(directory, prefix, timestamp_ms)?;
    match save_annotated_frame_selection(frame, document, selection, path.clone()) {
        Ok(()) => Ok(path),
        Err(error) => {
            let _ = std::fs::remove_file(&path);
            Err(error)
        }
    }
}

/// Atomically reserves a collision-safe final name before an encoder can start writing.
///
/// A plain existence check is insufficient when two capture workflows finish in the same
/// millisecond. The zero-byte reservation makes the filename claim exclusive; the image writer
/// replaces that reservation only after the encoded bytes are complete.
pub(super) fn reserve_quick_save_path(
    directory: &Path,
    prefix: &str,
    timestamp_ms: u128,
) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(directory)?;
    let mut candidate =
        next_quick_save_path_with_prefix(directory, prefix, timestamp_ms, Path::exists);
    loop {
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
                candidate =
                    next_quick_save_path_with_prefix(directory, prefix, timestamp_ms, Path::exists);
            }
            Err(error) => return Err(error),
        }
    }
}

pub(super) fn next_quick_save_path_with_prefix(
    directory: &Path,
    prefix: &str,
    timestamp_ms: u128,
    exists: impl Fn(&Path) -> bool,
) -> PathBuf {
    let stem = format!("{prefix}-{timestamp_ms}");
    let initial = directory.join(format!("{stem}.png"));
    if !exists(&initial) {
        return initial;
    }
    for index in 2_u32.. {
        let path = directory.join(format!("{stem}-{index}.png"));
        if !exists(&path) {
            return path;
        }
    }
    unreachable!("u32 path suffixes cannot be exhausted")
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
