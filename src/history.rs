//! Bounded, local-only screenshot history for files managed by Flash Shot.

use std::{
    collections::VecDeque,
    fs, io,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

const INDEX_FILE: &str = "history.json";
const DEFAULT_LIMIT: usize = 30;

/// Returns the only directory whose screenshot files this feature manages.
pub fn managed_history_directory() -> io::Result<PathBuf> {
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
    let directory = pictures.join("Flash Shot");
    fs::create_dir_all(&directory)?;
    Ok(directory)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryEntry {
    pub path: PathBuf,
    pub created_at_ms: u128,
}

#[derive(Clone, Debug)]
pub struct ScreenshotHistory {
    root: PathBuf,
    limit: usize,
    entries: VecDeque<HistoryEntry>,
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

    /// Applies a new retention limit immediately so the managed directory and
    /// its index cannot temporarily disagree about what history retains.
    pub fn set_limit(&mut self, limit: usize) -> io::Result<()> {
        if limit == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "screenshot history limit must be greater than zero",
            ));
        }
        self.limit = limit;
        self.prune()?;
        self.write_index()
    }

    pub fn record(&mut self, path: PathBuf) -> io::Result<()> {
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
        self.entries.retain(|entry| entry.path != path);
        self.entries.push_front(HistoryEntry {
            path,
            created_at_ms: unix_timestamp_ms(),
        });
        self.prune()?;
        self.write_index()
    }

    pub fn clear(&mut self) -> io::Result<()> {
        for entry in &self.entries {
            if entry.path.starts_with(&self.root) {
                match fs::remove_file(&entry.path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
        }
        self.entries.clear();
        self.write_index()
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
        let removed_file = match fs::remove_file(&entry.path) {
            Ok(()) => true,
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => return Err(error),
        };
        self.entries.remove(index);
        self.write_index()?;
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
                })
            })
            .filter(|entry| entry.path.starts_with(&self.root) && entry.path.is_file())
            .collect();
        self.prune()?;
        self.write_index()
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
        let entries: Vec<_> = self
            .entries
            .iter()
            .map(|entry| {
                serde_json::json!({
                    "path": entry.path,
                    "created_at_ms": entry.created_at_ms,
                })
            })
            .collect();
        let temporary = self.root.join("history.json.tmp");
        fs::write(
            &temporary,
            serde_json::to_vec(&entries).map_err(io::Error::other)?,
        )?;
        fs::rename(temporary, self.root.join(INDEX_FILE))
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
    use super::ScreenshotHistory;
    use std::fs;

    fn directory(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "flash-shot-history-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    #[test]
    fn records_existing_managed_files_and_restores_them_on_restart() {
        let root = directory("reload");
        fs::create_dir_all(&root).unwrap();
        let image = root.join("one.png");
        fs::write(&image, b"png").unwrap();
        let mut history = ScreenshotHistory::open(&root).unwrap();
        history.record(image.clone()).unwrap();

        let restored = ScreenshotHistory::open(&root).unwrap();
        assert_eq!(restored.entries().len(), 1);
        assert_eq!(restored.entries()[0].path, image.canonicalize().unwrap());
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
}
