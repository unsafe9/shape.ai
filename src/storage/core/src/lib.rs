//! # shape_storage_core
//!
//! Store-neutral storage layer for shape.ai. It abstracts where data lives
//! behind one [`StorageAdapter`] trait and gives every backend a single,
//! portable, parallel-friendly on-disk format to `export` to and `import`
//! from. See [`idea1.md`](../../../docs/idea1.md) for the originating idea.
//!
//! ## Pieces
//!
//! * [`Record`] / [`StoreSnapshot`] — store-neutral data model (opaque,
//!   versioned, byte-payload records).
//! * [`StorageAdapter`] — `save`/`load`/`delete`/`list` per-record I/O plus
//!   `snapshot`/`restore`, with `export`/`import` provided on top.
//! * [`format`] — the one portable bundle format: a sharded directory written
//!   and read in parallel via rayon, with per-shard CRCs for stability.
//! * [`MemoryAdapter`] / [`FileAdapter`] — real, fully-tested adapters.
//! * [`SqliteAdapter`] / [`PostgresAdapter`] / [`S3Adapter`] /
//!   [`RemoteServerAdapter`] — clearly-marked stubs (drivers unavailable
//!   offline) that still keep the portability contract.

mod adapter;
mod error;
pub mod format;
mod file;
mod memory;
mod record;
mod stubs;

pub use adapter::{AdapterKind, StorageAdapter};
pub use error::{Result, StorageError};
pub use file::FileAdapter;
pub use format::{Manifest, ShardEntry, DEFAULT_SHARD_COUNT, FORMAT_VERSION};
pub use memory::MemoryAdapter;
pub use record::{Record, StoreSnapshot};
pub use stubs::{PostgresAdapter, RemoteServerAdapter, S3Adapter, SqliteAdapter};

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::path::{Path, PathBuf};

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let pid = std::process::id();
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = env::temp_dir().join(format!("shape_storage_it_{tag}_{pid}_{nanos}"));
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

    /// Build a store with a spread of records (varied kinds, sizes, binary
    /// payloads) so shard partitioning and framing are exercised.
    fn sample_store(n: usize) -> MemoryAdapter {
        let mut store = MemoryAdapter::new();
        for i in 0..n {
            let kind = ["card", "edge", "group", "tag"][i % 4];
            let payload: Vec<u8> = (0..(i % 37)).map(|b| (b * i % 256) as u8).collect();
            store
                .save(Record {
                    id: format!("node-{i:04}"),
                    kind: kind.to_string(),
                    version: (i as u64) % 5 + 1,
                    payload,
                })
                .unwrap();
        }
        store
    }

    #[test]
    fn in_memory_roundtrip_via_snapshot() {
        let store = sample_store(50);
        let snap = store.snapshot().unwrap();
        let restored = MemoryAdapter::from_snapshot(snap.clone());
        assert_eq!(restored.snapshot().unwrap(), snap);
    }

    #[test]
    fn file_export_import_roundtrip() {
        let tmp = TempDir::new("roundtrip");
        let bundle = tmp.path().join("export.shapestore");

        let source = sample_store(120);
        let exported = source.export(&bundle).unwrap();
        assert_eq!(exported.total_records, 120);
        assert_eq!(exported.shard_count, DEFAULT_SHARD_COUNT);

        // Import into a different adapter kind (file) and verify equality.
        let dest_root = tmp.path().join("dest.shapestore");
        let mut dest = FileAdapter::open(&dest_root).unwrap();
        dest.import(&bundle).unwrap();

        assert_eq!(dest.snapshot().unwrap(), source.snapshot().unwrap());

        // And back into memory from the file store's own bundle.
        let mut mem = MemoryAdapter::new();
        mem.import(&dest_root).unwrap();
        assert_eq!(mem.snapshot().unwrap(), source.snapshot().unwrap());
    }

    #[test]
    fn cross_adapter_memory_to_file_to_memory() {
        let tmp = TempDir::new("cross");
        let bundle = tmp.path().join("bundle.shapestore");

        let mem = sample_store(33);
        mem.export(&bundle).unwrap();

        let file_root = tmp.path().join("file.shapestore");
        let mut file = FileAdapter::open(&file_root).unwrap();
        file.import(&bundle).unwrap();

        let mut mem2 = MemoryAdapter::new();
        mem2.import(&file_root).unwrap();

        assert_eq!(mem.snapshot().unwrap(), mem2.snapshot().unwrap());
    }

    #[test]
    fn format_is_byte_stable_across_exports() {
        let tmp = TempDir::new("stable");
        let a = tmp.path().join("a.shapestore");
        let b = tmp.path().join("b.shapestore");

        let store = sample_store(80);
        let man_a = store.export(&a).unwrap();
        let man_b = store.export(&b).unwrap();

        // Manifests (incl. per-shard CRCs) must be identical.
        assert_eq!(man_a, man_b);

        // Every shard file's bytes must match across the two exports.
        for entry in &man_a.shards {
            let name = format!("shard-{:05}.bin", entry.index);
            let bytes_a = std::fs::read(a.join(&name)).unwrap();
            let bytes_b = std::fs::read(b.join(&name)).unwrap();
            assert_eq!(bytes_a, bytes_b, "shard {} differed", entry.index);
        }
    }

    #[test]
    fn shard_count_is_configurable_and_lossless() {
        let tmp = TempDir::new("shards");
        let store = sample_store(64);
        for shard_count in [1u32, 2, 16, 64] {
            let bundle = tmp.path().join(format!("sc-{shard_count}.shapestore"));
            let manifest = store.export_with_shards(&bundle, shard_count).unwrap();
            assert_eq!(manifest.shard_count, shard_count);
            let mut mem = MemoryAdapter::new();
            mem.import(&bundle).unwrap();
            assert_eq!(mem.snapshot().unwrap(), store.snapshot().unwrap());
        }
    }

    #[test]
    fn import_detects_corrupted_shard() {
        let tmp = TempDir::new("corrupt");
        let bundle = tmp.path().join("c.shapestore");
        let store = sample_store(40);
        let manifest = store.export(&bundle).unwrap();

        // Corrupt the first non-empty shard.
        let target = manifest
            .shards
            .iter()
            .find(|s| s.records > 0)
            .expect("a non-empty shard");
        let path = bundle.join(format!("shard-{:05}.bin", target.index));
        let mut bytes = std::fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        std::fs::write(&path, &bytes).unwrap();

        let mut mem = MemoryAdapter::new();
        let err = mem.import(&bundle).unwrap_err();
        assert!(matches!(err, StorageError::Format(_)), "got {err:?}");
    }

    #[test]
    fn empty_store_roundtrips() {
        let tmp = TempDir::new("empty");
        let bundle = tmp.path().join("e.shapestore");
        let store = MemoryAdapter::new();
        let manifest = store.export(&bundle).unwrap();
        assert_eq!(manifest.total_records, 0);
        let mut mem = MemoryAdapter::new();
        mem.import(&bundle).unwrap();
        assert!(mem.is_empty());
    }

    #[test]
    fn adapter_kind_names_match_idea_vocabulary() {
        assert_eq!(AdapterKind::Memory.as_str(), "memory");
        assert_eq!(AdapterKind::File.as_str(), "file");
        assert_eq!(AdapterKind::Sqlite.as_str(), "sqlite");
        assert_eq!(AdapterKind::Postgres.as_str(), "postgres");
        assert_eq!(AdapterKind::S3.as_str(), "s3");
        assert_eq!(AdapterKind::RemoteServer.as_str(), "remote-server");
    }
}
