//! Fully-working single-file adapter.
//!
//! Persists the whole store as one bundle directory on disk and keeps an
//! in-memory mirror for fast per-record I/O. Every mutating operation flushes
//! the mirror back to disk, so the on-disk state always reflects the latest
//! writes. Its native persistence reuses the same portable bundle format used
//! by `export`/`import`, so a file store *is* a portable bundle.

use crate::adapter::{AdapterKind, StorageAdapter};
use crate::error::{Result, StorageError};
use crate::format::{export_bundle, import_bundle, DEFAULT_SHARD_COUNT};
use crate::record::{Record, StoreSnapshot};
use std::path::{Path, PathBuf};

/// On-disk store rooted at a bundle directory.
#[derive(Clone, Debug)]
pub struct FileAdapter {
    root: PathBuf,
    mirror: StoreSnapshot,
    shard_count: u32,
}

impl FileAdapter {
    /// Open (or create) a file store at `root`. If a bundle already exists
    /// there it is loaded; otherwise an empty store is created and flushed.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let mirror = if root.join(crate::format::MANIFEST_NAME).exists() {
            import_bundle(&root)?
        } else {
            StoreSnapshot::new()
        };
        let mut store = FileAdapter {
            root,
            mirror,
            shard_count: DEFAULT_SHARD_COUNT,
        };
        // Ensure a bundle exists on disk even for a fresh store.
        store.flush()?;
        Ok(store)
    }

    /// Override the shard fan-out used when flushing this store.
    pub fn with_shard_count(mut self, shard_count: u32) -> Self {
        self.shard_count = shard_count.max(1);
        self
    }

    /// The bundle directory backing this store.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Number of records held.
    pub fn len(&self) -> usize {
        self.mirror.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.mirror.is_empty()
    }

    /// Write the in-memory mirror out to the bundle directory.
    fn flush(&mut self) -> Result<()> {
        export_bundle(&self.mirror, &self.root, self.shard_count)?;
        Ok(())
    }
}

impl StorageAdapter for FileAdapter {
    fn kind(&self) -> AdapterKind {
        AdapterKind::File
    }

    fn save(&mut self, record: Record) -> Result<()> {
        self.mirror.insert(record);
        self.flush()
    }

    fn load(&self, id: &str) -> Result<Record> {
        self.mirror
            .get(id)
            .cloned()
            .ok_or_else(|| StorageError::NotFound { id: id.to_string() })
    }

    fn delete(&mut self, id: &str) -> Result<bool> {
        let removed = self.mirror.remove(id).is_some();
        if removed {
            self.flush()?;
        }
        Ok(removed)
    }

    fn list(&self) -> Result<Vec<String>> {
        Ok(self.mirror.ids().cloned().collect())
    }

    fn snapshot(&self) -> Result<StoreSnapshot> {
        Ok(self.mirror.clone())
    }

    fn restore(&mut self, snapshot: StoreSnapshot) -> Result<()> {
        self.mirror = snapshot;
        self.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    /// A unique temp dir under the OS temp root, cleaned up on drop.
    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let pid = std::process::id();
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = env::temp_dir().join(format!("shape_storage_{tag}_{pid}_{nanos}"));
            std::fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn persists_across_reopen() {
        let tmp = TempDir::new("reopen");
        let root = tmp.path().join("store.shapestore");
        {
            let mut store = FileAdapter::open(&root).unwrap();
            store.save(Record::new("x", "card", b"hi".to_vec())).unwrap();
            store
                .save(Record::new("y", "edge", b"yo".to_vec()))
                .unwrap();
        }
        // Reopen from disk in a fresh adapter instance.
        let store = FileAdapter::open(&root).unwrap();
        assert_eq!(store.len(), 2);
        assert_eq!(store.load("x").unwrap().payload, b"hi");
        assert_eq!(store.list().unwrap(), vec!["x", "y"]);
    }

    #[test]
    fn delete_persists() {
        let tmp = TempDir::new("delete");
        let root = tmp.path().join("store.shapestore");
        let mut store = FileAdapter::open(&root).unwrap();
        store.save(Record::new("a", "card", b"1".to_vec())).unwrap();
        assert!(store.delete("a").unwrap());
        let reopened = FileAdapter::open(&root).unwrap();
        assert!(reopened.is_empty());
    }
}
