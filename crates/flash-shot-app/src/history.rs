//! Bounded, local-only screenshot history for files managed by Flash Shot.

use std::{
    collections::VecDeque,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

const INDEX_FILE: &str = "history.json";
const DEFAULT_LIMIT: usize = 30;
const PROFILE_DIR_ENV: &str = "FLASH_SHOT_PROFILE_DIR";
static STORAGE_PROBE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Returns the only directory whose screenshot files this feature manages.
pub fn managed_history_directory() -> io::Result<PathBuf> {
    if let Some(root) = std::env::var_os(PROFILE_DIR_ENV).filter(|root| !root.is_empty()) {
        return create_managed_history_directory(PathBuf::from(root).join("history"));
    }
    let user_dirs = directories::UserDirs::new().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "user picture directory is unavailable",
        )
    })?;
    let pictures = user_dirs.picture_dir().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "user picture directory is unavailable",
        )
    })?;
    create_managed_history_directory(pictures.join("Flash Shot"))
}

/// Creates the history root selected by the current profile without exposing files outside it.
fn create_managed_history_directory(directory: PathBuf) -> io::Result<PathBuf> {
    fs::create_dir_all(&directory)?;
    Ok(directory)
}

/// Confirms that a quick-save directory can create, flush, and remove a private probe file.
///
/// The probe uses `create_new` so it never overwrites a user file, and cleanup runs after every
/// write attempt. This lets Settings surface permissions or disconnected-drive problems before a
/// screenshot needs to be saved.
pub fn verify_writable_directory(directory: impl AsRef<Path>) -> io::Result<()> {
    let directory = directory.as_ref();
    if !fs::metadata(directory)?.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "quick-save path is not a directory",
        ));
    }
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let counter = STORAGE_PROBE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = directory.join(format!(
        ".flash-shot-storage-probe-{}-{timestamp}-{counter}.tmp",
        std::process::id()
    ));
    let probe = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    let flush_result = probe.sync_all();
    drop(probe);
    let cleanup_result = fs::remove_file(&path);
    flush_result?;
    cleanup_result
}

/// Names the user workflow that produced a managed screenshot.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HistorySource {
    #[default]
    Unknown,
    Selection,
    Scrolling,
    FullScreen,
    Pinned,
}

impl HistorySource {
    /// Keeps list metadata short and understandable without leaking internal workflow names.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unknown => "Saved capture",
            Self::Selection => "Selection",
            Self::Scrolling => "Scrolling screenshot",
            Self::FullScreen => "Full screen",
            Self::Pinned => "Pinned image",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryEntry {
    pub path: PathBuf,
    pub created_at_ms: u128,
    pub source: HistorySource,
}

#[derive(Clone, Debug)]
pub struct ScreenshotHistory {
    root: PathBuf,
    limit: usize,
    entries: VecDeque<HistoryEntry>,
}

pub(crate) struct HistoryFileDeletion {
    pub(crate) deleted: Vec<PathBuf>,
    pub(crate) failures: Vec<(PathBuf, String)>,
}

impl ScreenshotHistory {
    pub fn open(root: impl Into<PathBuf>) -> io::Result<Self> {
        Self::open_with_limit(root, DEFAULT_LIMIT)
    }

    pub fn open_with_limit(root: impl Into<PathBuf>, limit: usize) -> io::Result<Self> {
        if limit == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "screenshot history limit must be greater than zero",
            ));
        }
        let root = root.into();
        fs::create_dir_all(&root)?;
        let root = root.canonicalize()?;
        let mut history = Self {
            root,
            limit,
            entries: VecDeque::new(),
        };
        history.load()?;
        Ok(history)
    }

    pub fn entries(&self) -> &VecDeque<HistoryEntry> {
        &self.entries
    }

    pub const fn limit(&self) -> usize {
        self.limit
    }

    /// Returns the private root whose files this history instance is allowed to manage.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Applies a new retention limit immediately so the managed directory and
    /// its index cannot temporarily disagree about what history retains.
    pub fn set_limit(&mut self, limit: usize) -> io::Result<()> {
        if limit == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "screenshot history limit must be greater than zero",
            ));
        }
        let mut entries = self.entries.clone();
        let pruned = entries
            .iter()
            .skip(limit)
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>();
        entries.truncate(limit);

        // Commit the index before pruning files so a failed index write leaves the current
        // history and every screenshot available for another attempt.
        self.write_index_entries(&entries)?;
        self.limit = limit;
        self.entries = entries;
        for path in pruned {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    pub(crate) fn retention_candidates(&self, limit: usize) -> Vec<PathBuf> {
        self.entries
            .iter()
            .skip(limit)
            .map(|entry| entry.path.clone())
            .collect()
    }

    pub(crate) fn set_limit_after_prune(&mut self, limit: usize) -> io::Result<()> {
        if limit == 0 || self.entries.len() > limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "history must be pruned before applying its retention limit",
            ));
        }
        let previous = self.limit;
        self.limit = limit;
        if let Err(error) = self.write_index() {
            self.limit = previous;
            return Err(error);
        }
        Ok(())
    }

    pub fn record(&mut self, path: PathBuf) -> io::Result<()> {
        self.record_with_source(path, HistorySource::Unknown)
    }

    /// Adds a file only after its producer has completed, retaining enough context for later reuse.
    pub fn record_with_source(&mut self, path: PathBuf, source: HistorySource) -> io::Result<()> {
        let path = path.canonicalize().unwrap_or(path);
        if !path.starts_with(&self.root) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "history only manages files inside its own directory",
            ));
        }
        if !path.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "cannot record a screenshot file that does not exist",
            ));
        }
        let mut entries = self.entries.clone();
        entries.retain(|entry| entry.path != path);
        entries.push_front(HistoryEntry {
            path,
            created_at_ms: unix_timestamp_ms(),
            source,
        });
        let pruned = entries
            .iter()
            .skip(self.limit)
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>();
        entries.truncate(self.limit);

        // Persist the new index before deleting an older capture. A read-only or conflicted
        // index therefore keeps the in-memory list and all files unchanged for a retry.
        self.write_index_entries(&entries)?;
        self.entries = entries;
        for path in pruned {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    pub fn clear(&mut self) -> io::Result<()> {
        let paths = self
            .entries
            .iter()
            .filter(|entry| entry.path.starts_with(&self.root))
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>();
        self.write_index_entries(&VecDeque::new())?;
        self.entries.clear();
        for path in paths {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    pub(crate) fn delete_managed_paths(
        &self,
        paths: impl IntoIterator<Item = PathBuf>,
    ) -> HistoryFileDeletion {
        let mut deleted = Vec::new();
        let mut failures = Vec::new();
        for path in paths {
            if !path.starts_with(&self.root) {
                failures.push((
                    path,
                    "history only manages files inside its own directory".to_owned(),
                ));
                continue;
            }
            match fs::remove_file(&path) {
                Ok(()) => deleted.push(path),
                Err(error) if error.kind() == io::ErrorKind::NotFound => deleted.push(path),
                Err(error) => failures.push((path, error.to_string())),
            }
        }
        HistoryFileDeletion { deleted, failures }
    }

    /// Removes only a completed snapshot from the current index so captures recorded while the
    /// background deletion was running remain intact.
    pub(crate) fn forget_deleted(&mut self, deleted: &[PathBuf]) -> io::Result<()> {
        let deleted = deleted
            .iter()
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        let mut entries = self.entries.clone();
        entries.retain(|entry| !deleted.contains(&entry.path));
        self.write_index_entries(&entries)?;
        self.entries = entries;
        Ok(())
    }

    /// Removes one managed screenshot and its index entry. Callers cannot use
    /// this history store to delete files outside its private root directory.
    pub fn remove(&mut self, path: impl AsRef<std::path::Path>) -> io::Result<bool> {
        let path = path.as_ref();
        let index = self
            .entries
            .iter()
            .position(|entry| entry.path == path)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "screenshot is not managed by history",
                )
            })?;
        let entry = self.entries[index].clone();
        if !entry.path.starts_with(&self.root) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "history only manages files inside its own directory",
            ));
        }
        let mut entries = self.entries.clone();
        entries.remove(index);
        self.write_index_entries(&entries)?;
        self.entries = entries;
        let removed_file = match fs::remove_file(&entry.path) {
            Ok(()) => true,
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => return Err(error),
        };
        Ok(removed_file)
    }

    fn load(&mut self) -> io::Result<()> {
        let path = self.root.join(INDEX_FILE);
        let contents = match fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        let values: Vec<serde_json::Value> =
            serde_json::from_str(&contents).map_err(io::Error::other)?;
        self.entries = values
            .into_iter()
            .filter_map(|value| {
                Some(HistoryEntry {
                    path: PathBuf::from(value.get("path")?.as_str()?),
                    created_at_ms: value.get("created_at_ms")?.as_u64()? as u128,
                    source: value
                        .get("source")
                        .and_then(|source| serde_json::from_value(source.clone()).ok())
                        .unwrap_or_default(),
                })
            })
            .filter_map(|entry| {
                self.managed_existing_path(&entry.path)
                    .map(|path| HistoryEntry {
                        path,
                        created_at_ms: entry.created_at_ms,
                        source: entry.source,
                    })
            })
            .collect();
        self.prune()?;
        self.write_index()
    }

    /// Resolves an index entry before trusting it so `..` segments and links
    /// cannot make a user-editable history index refer to files outside this store.
    fn managed_existing_path(&self, path: &Path) -> Option<PathBuf> {
        let path = path.canonicalize().ok()?;
        (path.starts_with(&self.root) && path.is_file()).then_some(path)
    }

    fn prune(&mut self) -> io::Result<()> {
        while self.entries.len() > self.limit {
            if let Some(entry) = self.entries.pop_back()
                && entry.path.starts_with(&self.root)
            {
                match fs::remove_file(entry.path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
        }
        Ok(())
    }

    fn write_index(&self) -> io::Result<()> {
        self.write_index_entries(&self.entries)
    }

    /// Writes a durable replacement index and removes the temporary file on every failure.
    ///
    /// The caller decides when to publish the matching in-memory entries. This keeps an index
    /// write failure retryable instead of exposing a partially updated history list or stale
    /// `history.json.tmp` artifact to the next save.
    fn write_index_entries(&self, entries: &VecDeque<HistoryEntry>) -> io::Result<()> {
        let entries: Vec<_> = entries
            .iter()
            .map(|entry| {
                serde_json::json!({
                    "path": entry.path,
                    "created_at_ms": entry.created_at_ms,
                    "source": entry.source,
                })
            })
            .collect();
        let temporary = self.root.join("history.json.tmp");
        let contents = serde_json::to_vec(&entries).map_err(io::Error::other)?;
        let result = (|| {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&temporary)?;
            file.write_all(&contents)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, self.root.join(INDEX_FILE))
        })();
        if result.is_err() {
            // This path is owned by the history writer. Ignore cleanup errors so the original
            // filesystem failure remains the actionable diagnostic shown by the UI.
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

fn unix_timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::{HistorySource, ScreenshotHistory, verify_writable_directory};
    use std::{fs, io};

    fn directory(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "flash-shot-history-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    #[test]
    fn isolated_profile_history_uses_its_private_root() {
        let root = directory("profile");
        let history = super::create_managed_history_directory(root.join("history")).unwrap();

        assert_eq!(history, root.join("history"));
        assert!(history.is_dir());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn storage_probe_leaves_a_writable_directory_empty() {
        let root = directory("storage-probe");
        fs::create_dir_all(&root).unwrap();

        verify_writable_directory(&root).unwrap();

        assert!(fs::read_dir(&root).unwrap().next().is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn storage_probe_rejects_a_file_path() {
        let root = directory("storage-probe-file");
        fs::create_dir_all(&root).unwrap();
        let file = root.join("not-a-directory.txt");
        fs::write(&file, b"not a directory").unwrap();

        let error = verify_writable_directory(&file).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn records_existing_managed_files_and_restores_them_on_restart() {
        let root = directory("reload");
        fs::create_dir_all(&root).unwrap();
        let image = root.join("one.png");
        fs::write(&image, b"png").unwrap();
        let mut history = ScreenshotHistory::open(&root).unwrap();
        history
            .record_with_source(image.clone(), HistorySource::FullScreen)
            .unwrap();

        let restored = ScreenshotHistory::open(&root).unwrap();
        assert_eq!(restored.entries().len(), 1);
        assert_eq!(restored.entries()[0].path, image.canonicalize().unwrap());
        assert_eq!(restored.entries()[0].source, HistorySource::FullScreen);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn record_index_failure_keeps_the_capture_available_for_retry() {
        let root = directory("record-index-failure");
        fs::create_dir_all(&root).unwrap();
        let first = root.join("one.png");
        let second = root.join("two.png");
        fs::write(&first, b"one").unwrap();
        fs::write(&second, b"two").unwrap();
        let mut history = ScreenshotHistory::open(&root).unwrap();
        history.record(first.clone()).unwrap();

        fs::remove_file(root.join("history.json")).unwrap();
        fs::create_dir(root.join("history.json")).unwrap();
        let error = history
            .record_with_source(second.clone(), HistorySource::Selection)
            .unwrap_err();

        assert!(!error.to_string().is_empty());
        assert_eq!(history.entries().len(), 1);
        assert_eq!(history.entries()[0].path, first.canonicalize().unwrap());
        assert!(second.exists());
        assert!(!root.join("history.json.tmp").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn clear_index_failure_keeps_history_files_and_entries_intact() {
        let root = directory("clear-index-failure");
        fs::create_dir_all(&root).unwrap();
        let image = root.join("one.png");
        fs::write(&image, b"png").unwrap();
        let mut history = ScreenshotHistory::open(&root).unwrap();
        history.record(image.clone()).unwrap();

        fs::remove_file(root.join("history.json")).unwrap();
        fs::create_dir(root.join("history.json")).unwrap();
        assert!(history.clear().is_err());

        assert_eq!(history.entries().len(), 1);
        assert!(image.exists());
        assert!(!root.join("history.json.tmp").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn remove_index_failure_keeps_the_capture_for_a_later_attempt() {
        let root = directory("remove-index-failure");
        fs::create_dir_all(&root).unwrap();
        let image = root.join("one.png");
        fs::write(&image, b"png").unwrap();
        let mut history = ScreenshotHistory::open(&root).unwrap();
        history.record(image.clone()).unwrap();
        let image = image.canonicalize().unwrap();

        fs::remove_file(root.join("history.json")).unwrap();
        fs::create_dir(root.join("history.json")).unwrap();
        assert!(history.remove(&image).is_err());

        assert_eq!(history.entries().len(), 1);
        assert!(image.exists());
        assert!(!root.join("history.json.tmp").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn forget_deleted_index_failure_preserves_entries_for_a_retry() {
        let root = directory("forget-index-failure");
        fs::create_dir_all(&root).unwrap();
        let image = root.join("one.png");
        fs::write(&image, b"png").unwrap();
        let mut history = ScreenshotHistory::open(&root).unwrap();
        history.record(image.clone()).unwrap();
        let image = image.canonicalize().unwrap();
        fs::remove_file(&image).unwrap();

        fs::remove_file(root.join("history.json")).unwrap();
        fs::create_dir(root.join("history.json")).unwrap();
        assert!(
            history
                .forget_deleted(std::slice::from_ref(&image))
                .is_err()
        );

        assert_eq!(history.entries().len(), 1);
        assert_eq!(history.entries()[0].path, image);
        assert!(!root.join("history.json.tmp").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retention_index_failure_keeps_the_previous_limit_and_files() {
        let root = directory("retention-index-failure");
        fs::create_dir_all(&root).unwrap();
        let first = root.join("one.png");
        let second = root.join("two.png");
        fs::write(&first, b"one").unwrap();
        fs::write(&second, b"two").unwrap();
        let mut history = ScreenshotHistory::open_with_limit(&root, 2).unwrap();
        history.record(first.clone()).unwrap();
        history.record(second.clone()).unwrap();

        fs::remove_file(root.join("history.json")).unwrap();
        fs::create_dir(root.join("history.json")).unwrap();
        assert!(history.set_limit(1).is_err());

        assert_eq!(history.limit(), 2);
        assert_eq!(history.entries().len(), 2);
        assert!(first.exists());
        assert!(second.exists());
        assert!(!root.join("history.json.tmp").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retention_removes_the_oldest_managed_screenshot() {
        let root = directory("retention");
        fs::create_dir_all(&root).unwrap();
        let first = root.join("one.png");
        let second = root.join("two.png");
        fs::write(&first, b"one").unwrap();
        fs::write(&second, b"two").unwrap();
        let mut history = ScreenshotHistory::open_with_limit(&root, 1).unwrap();
        history.record(first.clone()).unwrap();
        history.record(second.clone()).unwrap();

        assert!(!first.exists());
        assert!(second.exists());
        assert_eq!(history.entries().len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_index_entries_default_to_an_unknown_source() {
        let root = directory("legacy-source");
        fs::create_dir_all(&root).unwrap();
        let image = root.join("legacy.png");
        fs::write(&image, b"png").unwrap();
        fs::write(
            root.join("history.json"),
            serde_json::json!([{
                "path": image,
                "created_at_ms": 1,
            }])
            .to_string(),
        )
        .unwrap();

        let history = ScreenshotHistory::open(&root).unwrap();

        assert_eq!(history.entries()[0].source, HistorySource::Unknown);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lowering_the_retention_limit_prunes_existing_history_immediately() {
        let root = directory("change-limit");
        fs::create_dir_all(&root).unwrap();
        let first = root.join("one.png");
        let second = root.join("two.png");
        fs::write(&first, b"one").unwrap();
        fs::write(&second, b"two").unwrap();
        let mut history = ScreenshotHistory::open_with_limit(&root, 2).unwrap();
        history.record(first.clone()).unwrap();
        history.record(second.clone()).unwrap();

        history.set_limit(1).unwrap();

        assert_eq!(history.limit(), 1);
        assert!(!first.exists());
        assert!(second.exists());
        assert_eq!(history.entries().len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn staged_retention_deletes_oldest_entries_before_applying_the_limit() {
        let root = directory("staged-retention");
        fs::create_dir_all(&root).unwrap();
        let first = root.join("one.png");
        let second = root.join("two.png");
        fs::write(&first, b"one").unwrap();
        fs::write(&second, b"two").unwrap();
        let mut history = ScreenshotHistory::open_with_limit(&root, 2).unwrap();
        history.record(first.clone()).unwrap();
        history.record(second.clone()).unwrap();

        assert!(history.set_limit_after_prune(1).is_err());
        let candidates = history.retention_candidates(1);
        assert_eq!(candidates, vec![first.canonicalize().unwrap()]);
        let deletion = history.delete_managed_paths(candidates);
        assert!(deletion.failures.is_empty());
        history.forget_deleted(&deletion.deleted).unwrap();
        history.set_limit_after_prune(1).unwrap();

        assert_eq!(history.limit(), 1);
        assert_eq!(history.entries().len(), 1);
        assert_eq!(history.entries()[0].path, second.canonicalize().unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn clear_removes_only_managed_history_files() {
        let root = directory("clear");
        fs::create_dir_all(&root).unwrap();
        let image = root.join("one.png");
        fs::write(&image, b"png").unwrap();
        let mut history = ScreenshotHistory::open(&root).unwrap();
        history.record(image.clone()).unwrap();
        history.clear().unwrap();

        assert!(history.entries().is_empty());
        assert!(!image.exists());
        assert!(root.join("history.json").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn background_clear_snapshot_preserves_newer_history_entries() {
        let root = directory("background-clear");
        fs::create_dir_all(&root).unwrap();
        let first = root.join("one.png");
        let second = root.join("two.png");
        fs::write(&first, b"one").unwrap();
        let mut history = ScreenshotHistory::open(&root).unwrap();
        history.record(first.clone()).unwrap();
        let snapshot = history.clone();

        fs::write(&second, b"two").unwrap();
        history
            .record_with_source(second.clone(), HistorySource::Pinned)
            .unwrap();
        let deletion = snapshot
            .delete_managed_paths(snapshot.entries().iter().map(|entry| entry.path.clone()));
        assert!(deletion.failures.is_empty());
        history.forget_deleted(&deletion.deleted).unwrap();

        assert!(!first.exists());
        assert!(second.exists());
        assert_eq!(history.entries().len(), 1);
        assert_eq!(history.entries()[0].path, second.canonicalize().unwrap());
        assert_eq!(history.entries()[0].source, HistorySource::Pinned);
        let restored = ScreenshotHistory::open(&root).unwrap();
        assert_eq!(restored.entries(), history.entries());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn remove_deletes_one_managed_file_and_keeps_the_other_entries() {
        let root = directory("remove");
        fs::create_dir_all(&root).unwrap();
        let first = root.join("one.png");
        let second = root.join("two.png");
        fs::write(&first, b"one").unwrap();
        fs::write(&second, b"two").unwrap();
        let mut history = ScreenshotHistory::open(&root).unwrap();
        history.record(first.clone()).unwrap();
        history.record(second.clone()).unwrap();

        assert!(history.remove(first.canonicalize().unwrap()).unwrap());
        assert!(!first.exists());
        assert!(second.exists());
        assert_eq!(history.entries().len(), 1);
        assert_eq!(history.entries()[0].path, second.canonicalize().unwrap());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn remove_rejects_unmanaged_paths_without_deleting_them() {
        let root = directory("remove-unmanaged");
        let outside = directory("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let managed = root.join("one.png");
        let unmanaged = outside.join("other.png");
        fs::write(&managed, b"managed").unwrap();
        fs::write(&unmanaged, b"unmanaged").unwrap();
        let mut history = ScreenshotHistory::open(&root).unwrap();
        history.record(managed).unwrap();

        let error = history.remove(&unmanaged).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        assert!(unmanaged.exists());

        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn loading_an_escaped_index_entry_never_manages_or_deletes_an_outside_file() {
        let root = directory("escaped-index");
        let outside = directory("escaped-index-outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let outside_image = outside.join("outside.png");
        fs::write(&outside_image, b"outside").unwrap();
        let escaped = root
            .join("..")
            .join(
                outside
                    .file_name()
                    .expect("temporary directory has a final path component"),
            )
            .join("outside.png");
        fs::write(
            root.join("history.json"),
            serde_json::json!([{
                "path": escaped,
                "created_at_ms": 1,
            }])
            .to_string(),
        )
        .unwrap();

        let mut history = ScreenshotHistory::open(&root).unwrap();

        assert!(history.entries().is_empty());
        history.clear().unwrap();
        assert!(outside_image.exists());
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }
}
