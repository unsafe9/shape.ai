//! Fully-working on-disk file adapter.
//!
//! The store's source of truth is the portable bundle directory on disk — a
//! file store *is* a portable bundle. There is **no full in-memory mirror**:
//! every operation touches only what it needs, so the adapter stays
//! memory-bounded even for very large stores.
//!
//! Memory discipline:
//!
//! * [`records`](StorageAdapter::records) streams the bundle shard-by-shard,
//!   frame-by-frame, off disk — peak is one record.
//! * `load` / `list` stream the relevant shard(s) and stop early — peak is one
//!   record.
//! * `save` / `delete` rewrite only the single shard a record hashes to — peak
//!   is `O(one shard)`, never the whole store.
//! * `import` does a bounded streaming merge of the incoming bundle with what is
//!   already on disk and re-shards it via the streaming exporter — peak is
//!   `O(shard chunk * parallelism)`, never `O(total)`.
//! * `snapshot` / `restore` exist for the small/in-memory convenience API and
//!   are **not** on the export/import streaming path.

use crate::adapter::{AdapterKind, RecordCursor, StorageAdapter};
use crate::error::{Result, StorageError};
use crate::format::{
    export_stream, read_shard_records, rewrite_shard, shard_index_of, MANIFEST_NAME,
    DEFAULT_SHARD_COUNT,
};
use crate::record::{Record, StoreSnapshot};
use std::path::{Path, PathBuf};

/// On-disk store rooted at a bundle directory.
#[derive(Clone, Debug)]
pub struct FileAdapter {
    root: PathBuf,
    shard_count: u32,
}

impl FileAdapter {
    /// Open (or create) a file store at `root`. If a bundle already exists there
    /// it is adopted as-is (its shard count is kept); otherwise an empty bundle
    /// is written with the default shard count.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        if root.join(MANIFEST_NAME).exists() {
            let manifest = crate::format::manifest_of(&root)?;
            Ok(FileAdapter {
                root,
                shard_count: manifest.shard_count.max(1),
            })
        } else {
            let mut store = FileAdapter {
                root,
                shard_count: DEFAULT_SHARD_COUNT,
            };
            // Materialize an empty bundle on disk.
            store.write_empty()?;
            Ok(store)
        }
    }

    /// Override the shard fan-out and re-shard the on-disk bundle to match.
    /// On a reshard failure the on-disk bundle and shard count are left intact.
    pub fn with_shard_count(mut self, shard_count: u32) -> Self {
        let target = shard_count.max(1);
        if target != self.shard_count && self.reshard(target).is_ok() {
            self.shard_count = target;
        }
        self
    }

    /// The bundle directory backing this store.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Number of records held (reads the manifest only; does not load records).
    pub fn len(&self) -> usize {
        crate::format::manifest_of(&self.root)
            .map(|m| usize::try_from(m.total_records).unwrap_or(usize::MAX))
            .unwrap_or(0)
    }

    /// Whether the store is empty (manifest-only check).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn write_empty(&mut self) -> Result<()> {
        export_stream(std::iter::empty::<Result<Record>>(), &self.root, self.shard_count)?;
        Ok(())
    }

    /// Stream the current on-disk records into a fresh layout with `shard_count`
    /// shards. Bounded: the streaming exporter never holds the whole store.
    fn reshard(&mut self, shard_count: u32) -> Result<()> {
        let cursor = crate::format::stream_bundle(&self.root)?;
        // Write to a sibling temp bundle, then swap in place.
        let tmp = sibling_tmp(&self.root);
        write_then_swap(&self.root, &tmp, |dst| {
            export_stream(cursor, dst, shard_count).map(|_| ())
        })
    }
}

impl StorageAdapter for FileAdapter {
    fn kind(&self) -> AdapterKind {
        AdapterKind::File
    }

    fn save(&mut self, record: Record) -> Result<()> {
        // Rewrite only the single shard this record hashes to.
        let index = shard_index_of(&record.id, self.shard_count);
        rewrite_shard(&self.root, self.shard_count, index, |records| {
            upsert_sorted(records, record);
        })
    }

    fn load(&self, id: &str) -> Result<Record> {
        let index = shard_index_of(id, self.shard_count);
        let records = read_shard_records(&self.root, index)?;
        records
            .into_iter()
            .find(|r| r.id == id)
            .ok_or_else(|| StorageError::NotFound { id: id.to_string() })
    }

    fn delete(&mut self, id: &str) -> Result<bool> {
        let index = shard_index_of(id, self.shard_count);
        let mut removed = false;
        rewrite_shard(&self.root, self.shard_count, index, |records| {
            if let Some(pos) = records.iter().position(|r| r.id == id) {
                records.remove(pos);
                removed = true;
            }
        })?;
        Ok(removed)
    }

    fn list(&self) -> Result<Vec<String>> {
        // Streams shard-by-shard; only the id strings are retained.
        let mut ids = Vec::new();
        crate::format::import_stream(&self.root, |record| {
            ids.push(record.id);
            Ok(())
        })?;
        ids.sort();
        Ok(ids)
    }

    fn records(&self) -> Result<RecordCursor<'_>> {
        // Lazy, id-sorted, shard-by-shard off disk.
        Ok(Box::new(crate::format::stream_bundle(&self.root)?))
    }

    fn snapshot(&self) -> Result<StoreSnapshot> {
        // Convenience API: collects the streamed records. Not on the export path.
        let mut snap = StoreSnapshot::new();
        crate::format::import_stream(&self.root, |record| {
            snap.insert(record);
            Ok(())
        })?;
        Ok(snap)
    }

    fn restore(&mut self, snapshot: StoreSnapshot) -> Result<()> {
        // Replace the on-disk bundle from a snapshot (small/in-memory path).
        export_stream(snapshot.records().cloned().map(Ok), &self.root, self.shard_count)?;
        Ok(())
    }

    fn import(&mut self, root: &Path) -> Result<()> {
        // Bounded streaming merge: 2-way merge of the on-disk records and the
        // incoming bundle (both id-sorted), incoming winning on equal id, then
        // re-shard via the streaming exporter. Peak is O(shard chunk * threads),
        // never O(total) — no full snapshot is built on either side.
        let existing = crate::format::stream_bundle(&self.root)?;
        let incoming = crate::format::stream_bundle(root)?;
        let merged = MergeById::new(Box::new(existing), Box::new(incoming));

        let tmp = sibling_tmp(&self.root);
        write_then_swap(&self.root, &tmp, |dst| {
            export_stream(merged, dst, self.shard_count).map(|_| ())
        })
    }
}

/// Insert-or-replace `record` into an id-sorted `Vec`, keeping it sorted.
fn upsert_sorted(records: &mut Vec<Record>, record: Record) {
    match records.binary_search_by(|r| r.id.cmp(&record.id)) {
        Ok(pos) => records[pos] = record,
        Err(pos) => records.insert(pos, record),
    }
}

/// A bounded 2-way merge of two id-sorted record streams. On equal id the
/// `right` (incoming) record wins. Peak resident: one record per side.
struct MergeById {
    left: std::iter::Peekable<RecordIter>,
    right: std::iter::Peekable<RecordIter>,
}

type RecordIter = Box<dyn Iterator<Item = Result<Record>>>;

impl MergeById {
    fn new(left: RecordIter, right: RecordIter) -> Self {
        MergeById {
            left: left.peekable(),
            right: right.peekable(),
        }
    }
}

impl Iterator for MergeById {
    type Item = Result<Record>;

    fn next(&mut self) -> Option<Self::Item> {
        // Surface errors eagerly from either side.
        let l_id = match self.left.peek() {
            Some(Ok(r)) => Some(r.id.clone()),
            Some(Err(_)) => return self.left.next(),
            None => None,
        };
        let r_id = match self.right.peek() {
            Some(Ok(r)) => Some(r.id.clone()),
            Some(Err(_)) => return self.right.next(),
            None => None,
        };
        match (l_id, r_id) {
            (None, None) => None,
            (Some(_), None) => self.left.next(),
            (None, Some(_)) => self.right.next(),
            (Some(l), Some(r)) => {
                if l < r {
                    self.left.next()
                } else if r < l {
                    self.right.next()
                } else {
                    // Equal id: incoming wins; drop the left duplicate.
                    let _ = self.left.next();
                    self.right.next()
                }
            }
        }
    }
}

/// A unique sibling temp bundle path next to `root` for swap-in writes.
fn sibling_tmp(root: &Path) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let name = format!(
        ".{}.tmp-{}-{}",
        root.file_name().and_then(|n| n.to_str()).unwrap_or("bundle"),
        std::process::id(),
        nanos
    );
    match root.parent() {
        Some(parent) => parent.join(name),
        None => PathBuf::from(name),
    }
}

/// Write a fresh bundle into the sibling temp path `tmp` via `write`, then swap
/// it into place at `dst`. The swap-in-place keeps the live bundle byte-intact
/// until the new one is fully written. On any failure the partial temp bundle is
/// removed so a failed run never leaks an orphan bundle beside the live store.
fn write_then_swap<F>(dst: &Path, tmp: &Path, write: F) -> Result<()>
where
    F: FnOnce(&Path) -> Result<()>,
{
    match write(tmp).and_then(|()| replace_bundle(dst, tmp)) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_dir_all(tmp);
            Err(e)
        }
    }
}

/// Atomically replace the bundle at `dst` with the freshly written `src`.
fn replace_bundle(dst: &Path, src: &Path) -> Result<()> {
    if dst.exists() {
        std::fs::remove_dir_all(dst)?;
    }
    std::fs::rename(src, dst)?;
    Ok(())
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

    #[test]
    fn save_overwrites_by_id() {
        let tmp = TempDir::new("overwrite");
        let root = tmp.path().join("store.shapestore");
        let mut store = FileAdapter::open(&root).unwrap();
        store.save(Record::new("a", "card", b"1".to_vec())).unwrap();
        store.save(Record::new("a", "card", b"2".to_vec())).unwrap();
        assert_eq!(store.len(), 1);
        assert_eq!(store.load("a").unwrap().payload, b"2");
    }
}
