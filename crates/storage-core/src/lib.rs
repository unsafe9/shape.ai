//! Store-neutral storage layer for shape.ai: data lives behind one
//! [`StorageAdapter`] trait, with a single portable on-disk bundle format every
//! backend can `export` to and `import` from.

mod adapter;
mod adapter_async;
mod adapters;
mod error;
pub mod morton;
#[cfg(all(target_arch = "wasm32", feature = "opfs"))]
pub mod opfs_backend;
// Native-only: the bundle format depends on std::fs + rayon. wasm32 keeps the
// data model + trait + MemoryAdapter and no on-disk format.
#[cfg(not(target_arch = "wasm32"))]
pub mod format;
mod record;
mod spatial;

pub use adapter::{AdapterKind, RecordCursor, StorageAdapter};
pub use adapter_async::{AsyncStorageAdapter, RegionWindow};
pub use adapters::MemoryAdapter;
#[cfg(not(target_arch = "wasm32"))]
pub use adapters::{FileAdapter, PostgresAdapter, RemoteServerAdapter, S3Adapter};
#[cfg(all(not(target_arch = "wasm32"), feature = "redb"))]
pub use adapters::RedbAdapter;
pub use error::{Result, StorageError};
#[cfg(not(target_arch = "wasm32"))]
pub use format::{Manifest, ShardEntry, DEFAULT_SHARD_COUNT, FORMAT_VERSION};
pub use record::{Record, StoreSnapshot};
pub use spatial::{RegionKey, SpatialStore};

#[cfg(all(test, not(target_arch = "wasm32")))]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    reason = "test fixtures intentionally truncate to byte values"
)]
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

        let dest_root = tmp.path().join("dest.shapestore");
        let mut dest = FileAdapter::open(&dest_root).unwrap();
        dest.import(&bundle).unwrap();

        assert_eq!(dest.snapshot().unwrap(), source.snapshot().unwrap());

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

        assert_eq!(man_a, man_b);

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
        assert_eq!(AdapterKind::Redb.as_str(), "redb");
        assert_eq!(AdapterKind::Postgres.as_str(), "postgres");
        assert_eq!(AdapterKind::S3.as_str(), "s3");
        assert_eq!(AdapterKind::RemoteServer.as_str(), "remote-server");
    }
}

/// Test-only counting global allocator: tracks live + peak heap bytes while
/// "armed", so a test can assert streaming export/import stays far below the
/// dataset's total size (regression guard against snapshot-the-whole-store).
#[cfg(all(test, not(target_arch = "wasm32")))]
mod alloc_probe {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    pub static LIVE: AtomicUsize = AtomicUsize::new(0);
    pub static PEAK: AtomicUsize = AtomicUsize::new(0);
    pub static ARMED: AtomicBool = AtomicBool::new(false);

    pub struct CountingAlloc;

    unsafe impl GlobalAlloc for CountingAlloc {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let ptr = System.alloc(layout);
            if !ptr.is_null() && ARMED.load(Ordering::Relaxed) {
                let now = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
                PEAK.fetch_max(now, Ordering::Relaxed);
            }
            ptr
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            if ARMED.load(Ordering::Relaxed) {
                // Saturating: pre-arm allocations freed while armed (incl. on
                // other test threads) must not underflow the live counter.
                let mut cur = LIVE.load(Ordering::Relaxed);
                loop {
                    let next = cur.saturating_sub(layout.size());
                    match LIVE.compare_exchange_weak(
                        cur,
                        next,
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    ) {
                        Ok(_) => break,
                        Err(observed) => cur = observed,
                    }
                }
            }
            System.dealloc(ptr, layout);
        }
    }

    pub fn arm() {
        LIVE.store(0, Ordering::Relaxed);
        PEAK.store(0, Ordering::Relaxed);
        ARMED.store(true, Ordering::Relaxed);
    }

    /// Disarm and report the peak live bytes observed while armed.
    pub fn disarm_peak() -> usize {
        ARMED.store(false, Ordering::Relaxed);
        PEAK.load(Ordering::Relaxed)
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[global_allocator]
static GLOBAL: alloc_probe::CountingAlloc = alloc_probe::CountingAlloc;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    reason = "test fixtures intentionally truncate to byte values"
)]
mod integrity {
    use super::*;
    use crate::format::{import_stream, shard_index_of};
    use std::cell::Cell;
    use std::env;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;
    use std::sync::Mutex;

    /// Serializes allocator-probing tests so only one is active at a time
    /// (the probe peak is process-global). Poison-tolerant: it guards only a
    /// measurement window, not shared data, so a later holder may reuse it.
    static PROBE_LOCK: Mutex<()> = Mutex::new(());

    fn probe_guard() -> std::sync::MutexGuard<'static, ()> {
        PROBE_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let pid = std::process::id();
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = env::temp_dir().join(format!("shape_storage_intg_{tag}_{pid}_{nanos}"));
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

    /// One fixed, varied dataset returned in *insertion* order (not sorted) so
    /// tests prove the adapters impose deterministic ordering themselves.
    fn fixed_dataset() -> Vec<Record> {
        let kinds = ["card", "edge", "group", "tag", "note"];
        let mut out = Vec::new();
        for i in 0..200usize {
            let id = format!("rec-{:04}-{}", (i * 7919) % 200, i % 3);
            let kind = kinds[i % kinds.len()].to_string();
            let version = (i as u64 * 3 + 1) % 17;
            let payload: Vec<u8> = match i % 5 {
                0 => Vec::new(),
                1 => vec![0u8; i % 11],
                2 => (0..(i % 257)).map(|b| (b * 31 % 256) as u8).collect(),
                3 => vec![0xffu8; i % 7],
                _ => format!("payload-{i}-\u{1f600}").into_bytes(),
            };
            out.push(Record {
                id,
                kind,
                version,
                payload,
            });
        }
        out
    }

    fn loaded<A: StorageAdapter>(mut adapter: A) -> (A, Vec<Record>) {
        let mut by_id: std::collections::BTreeMap<String, Record> = Default::default();
        for r in fixed_dataset() {
            by_id.insert(r.id.clone(), r.clone());
            adapter.save(r).unwrap();
        }
        let expected: Vec<Record> = by_id.into_values().collect();
        (adapter, expected)
    }

    fn open_file(tmp: &TempDir, name: &str) -> FileAdapter {
        FileAdapter::open(tmp.path().join(name)).unwrap()
    }

    // ---- SAME DATA THROUGH ALL METHODS -----------------------------------

    fn all_methods_consistent<A: StorageAdapter>(adapter: A, expected: &[Record]) {
        let mut adapter = adapter;
        let expected_ids: Vec<String> = expected.iter().map(|r| r.id.clone()).collect();

        for r in expected {
            let got = adapter.load(&r.id).unwrap();
            assert_eq!(&got, r, "load mismatch for {}", r.id);
        }

        let ids = adapter.list().unwrap();
        assert_eq!(ids, expected_ids, "list order/completeness");

        let via_cursor: Vec<Record> = adapter.records().unwrap().map(|r| r.unwrap()).collect();
        assert_eq!(&via_cursor, expected, "records() cursor");

        let snap = adapter.snapshot().unwrap();
        assert_eq!(snap.len(), expected.len());
        let snap_records: Vec<Record> = snap.records().cloned().collect();
        assert_eq!(&snap_records, expected, "snapshot contents");

        assert!(matches!(
            adapter.load("does-not-exist"),
            Err(StorageError::NotFound { .. })
        ));

        let victim = &expected[expected.len() / 2].id;
        assert!(adapter.delete(victim).unwrap(), "delete returns true");
        assert!(!adapter.delete(victim).unwrap(), "second delete returns false");
        assert!(matches!(
            adapter.load(victim),
            Err(StorageError::NotFound { .. })
        ));
        assert!(!adapter.list().unwrap().contains(victim), "victim gone from list");
        assert_eq!(adapter.list().unwrap().len(), expected.len() - 1);
    }

    #[test]
    fn memory_all_methods_same_data() {
        let (adapter, expected) = loaded(MemoryAdapter::new());
        all_methods_consistent(adapter, &expected);
    }

    #[test]
    fn file_all_methods_same_data() {
        let tmp = TempDir::new("allmethods");
        let (adapter, expected) = loaded(open_file(&tmp, "store.shapestore"));
        all_methods_consistent(adapter, &expected);
    }

    // ---- STORED == EXPORT EQUAL ------------------------------------------

    fn read_bundle_bytes(root: &Path) -> Vec<(String, Vec<u8>)> {
        let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(root)
            .unwrap()
            .map(|e| {
                let p = e.unwrap().path();
                let name = p.file_name().unwrap().to_string_lossy().into_owned();
                (name, std::fs::read(&p).unwrap())
            })
            .collect();
        files.sort_by(|a, b| a.0.cmp(&b.0));
        files
    }

    #[test]
    fn repeated_export_is_byte_identical() {
        let tmp = TempDir::new("repeat");
        let (mem, _) = loaded(MemoryAdapter::new());
        let a = tmp.path().join("a.shapestore");
        let b = tmp.path().join("b.shapestore");
        let man_a = mem.export(&a).unwrap();
        let man_b = mem.export(&b).unwrap();
        assert_eq!(man_a, man_b);
        assert_eq!(read_bundle_bytes(&a), read_bundle_bytes(&b));
    }

    #[test]
    fn different_adapter_kinds_export_byte_identical() {
        let tmp = TempDir::new("kinds");
        let (mem, _) = loaded(MemoryAdapter::new());
        let (file, _) = loaded(open_file(&tmp, "file.shapestore"));

        let mem_bundle = tmp.path().join("mem.shapestore");
        let file_bundle = tmp.path().join("from-file.shapestore");
        let man_mem = mem.export(&mem_bundle).unwrap();
        let man_file = file.export(&file_bundle).unwrap();

        assert_eq!(man_mem, man_file, "manifests differ across adapter kinds");
        assert_eq!(
            read_bundle_bytes(&mem_bundle),
            read_bundle_bytes(&file_bundle),
            "shard bytes differ across adapter kinds"
        );
    }

    #[test]
    fn export_import_reexport_is_byte_identical() {
        let tmp = TempDir::new("reexport");
        let (mem, _) = loaded(MemoryAdapter::new());

        let original = tmp.path().join("orig.shapestore");
        mem.export(&original).unwrap();
        let original_bytes = read_bundle_bytes(&original);

        // memory -> bundle -> file -> bundle must equal the original bundle.
        let mut file = open_file(&tmp, "rehydrated.shapestore");
        file.import(&original).unwrap();
        let reexport = tmp.path().join("reexport.shapestore");
        file.export(&reexport).unwrap();
        assert_eq!(original_bytes, read_bundle_bytes(&reexport));

        // ...and memory -> bundle -> memory -> bundle too.
        let mut mem2 = MemoryAdapter::new();
        mem2.import(&original).unwrap();
        let reexport2 = tmp.path().join("reexport2.shapestore");
        mem2.export(&reexport2).unwrap();
        assert_eq!(original_bytes, read_bundle_bytes(&reexport2));
    }

    // ---- ALL DATA IMPORTS CORRECTLY (incl. cross-adapter) ----------------

    fn assert_imports_match(expected: &[Record], imported: &dyn StorageAdapter) {
        let got: Vec<Record> = imported.records().unwrap().map(|r| r.unwrap()).collect();
        assert_eq!(got.len(), expected.len(), "imported record count");
        for (e, g) in expected.iter().zip(got.iter()) {
            assert_eq!(e.id, g.id);
            assert_eq!(e.kind, g.kind, "kind for {}", e.id);
            assert_eq!(e.version, g.version, "version for {}", e.id);
            assert_eq!(e.payload, g.payload, "payload for {}", e.id);
        }
    }

    #[test]
    fn import_into_memory_and_file_matches_source() {
        let tmp = TempDir::new("importall");
        let (mem, expected) = loaded(MemoryAdapter::new());
        let bundle = tmp.path().join("src.shapestore");
        mem.export(&bundle).unwrap();

        let mut into_mem = MemoryAdapter::new();
        into_mem.import(&bundle).unwrap();
        assert_imports_match(&expected, &into_mem);

        let mut into_file = open_file(&tmp, "into.shapestore");
        into_file.import(&bundle).unwrap();
        assert_imports_match(&expected, &into_file);
    }

    #[test]
    fn cross_adapter_memory_file_memory_preserves_everything() {
        let tmp = TempDir::new("xadapter");
        let (mem, expected) = loaded(MemoryAdapter::new());
        let b1 = tmp.path().join("b1.shapestore");
        mem.export(&b1).unwrap();

        let mut file = open_file(&tmp, "mid.shapestore");
        file.import(&b1).unwrap();

        let b2 = tmp.path().join("b2.shapestore");
        file.export(&b2).unwrap();

        let mut mem2 = MemoryAdapter::new();
        mem2.import(&b2).unwrap();
        assert_imports_match(&expected, &mem2);
    }

    /// The SAME logical dataset exported from every real adapter (Memory, File,
    /// Redb) must produce byte-identical bundles, and each must re-import into a
    /// fresh MemoryAdapter reproducing the source snapshot exactly.
    #[cfg(feature = "redb")]
    #[test]
    fn all_adapters_export_byte_identical_and_reimport() {
        // redb in-memory backend + zstd buffers allocate several MB; hold the
        // probe lock so this never overlaps an armed bounded-memory probe.
        let _probe = probe_guard();
        let tmp = TempDir::new("alladapters");

        let (mem, expected) = loaded(MemoryAdapter::new());
        let (file, _) = loaded(open_file(&tmp, "store.shapestore"));
        let mut redb = RedbAdapter::open_in_memory().unwrap();
        for r in &expected {
            // RedbAdapter impls both sync and async traits (both in scope);
            // disambiguate to the sync save the export path relies on.
            StorageAdapter::save(&mut redb, r.clone()).unwrap();
        }

        let mem_bundle = tmp.path().join("mem.shapestore");
        let file_bundle = tmp.path().join("file.shapestore");
        let redb_bundle = tmp.path().join("redb.shapestore");
        let man_mem = mem.export(&mem_bundle).unwrap();
        let man_file = file.export(&file_bundle).unwrap();
        let man_redb = redb.export(&redb_bundle).unwrap();

        assert_eq!(man_mem, man_file, "memory vs file manifest");
        assert_eq!(man_mem, man_redb, "memory vs redb manifest");

        let bytes_mem = read_bundle_bytes(&mem_bundle);
        assert_eq!(bytes_mem, read_bundle_bytes(&file_bundle), "memory vs file bytes");
        assert_eq!(bytes_mem, read_bundle_bytes(&redb_bundle), "memory vs redb bytes");

        let source_snapshot = mem.snapshot().unwrap();
        for bundle in [&mem_bundle, &file_bundle, &redb_bundle] {
            let mut into = MemoryAdapter::new();
            into.import(bundle).unwrap();
            assert_eq!(
                into.snapshot().unwrap(),
                source_snapshot,
                "snapshot mismatch reimporting {}",
                bundle.display()
            );
        }
    }

    // ---- BUNDLE ATOMICITY ------------------------------------------------

    /// A FileAdapter import that fails partway (corrupt incoming bundle) must
    /// leave the existing on-disk bundle fully intact — the swap-in-place write
    /// goes through a sibling temp dir and only renames on success.
    #[test]
    fn file_import_failure_leaves_existing_bundle_intact() {
        let tmp = TempDir::new("atomic");

        let (file_seed, expected) = loaded(open_file(&tmp, "live.shapestore"));
        let live_root = file_seed.root().to_path_buf();
        let before = read_bundle_bytes(&live_root);
        assert!(!before.is_empty(), "seed store must have content");

        // Build a valid bundle, then corrupt one shard so import fails midway.
        let incoming = tmp.path().join("incoming.shapestore");
        let (donor, _) = loaded(MemoryAdapter::new());
        let man = donor.export(&incoming).unwrap();
        let victim = man
            .shards
            .iter()
            .find(|s| s.records > 0)
            .expect("a non-empty shard");
        let shard_path = incoming.join(format!("shard-{:05}.bin", victim.index));
        let mut shard_bytes = std::fs::read(&shard_path).unwrap();
        let last = shard_bytes.len() - 1;
        shard_bytes[last] ^= 0xff;
        std::fs::write(&shard_path, &shard_bytes).unwrap();

        let mut live = FileAdapter::open(&live_root).unwrap();
        let err = live.import(&incoming).unwrap_err();
        assert!(matches!(err, StorageError::Format(_)), "got {err:?}");

        assert_eq!(read_bundle_bytes(&live_root), before, "live bundle bytes changed");
        let reopened = FileAdapter::open(&live_root).unwrap();
        assert_imports_match(&expected, &reopened);

        let leaked: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".tmp-"))
            .collect();
        assert!(leaked.is_empty(), "leaked temp bundle(s): {leaked:?}");
    }

    // ---- LARGE-DATA / BOUNDED-MEMORY -------------------------------------

    /// A lazy, O(1)-resident generator of `n` synthetic records: each is built
    /// on demand inside `map`, never storing the whole set.
    fn big_records(n: usize, payload_len: usize) -> impl Iterator<Item = Result<Record>> {
        (0..n).map(move |i| {
            let payload = vec![(i % 251) as u8; payload_len];
            Ok(Record {
                id: format!("big-{i:08}"),
                kind: ["card", "edge", "group"][i % 3].to_string(),
                version: (i as u64) % 9 + 1,
                payload,
            })
        })
    }

    /// A streaming sink that records the PEAK number of records resident at once
    /// via a drop-tracking guard: a true streaming import stays at 1, while
    /// "build the whole snapshot first" would spike to O(total).
    struct ResidentProbe {
        live: Rc<Cell<usize>>,
        peak: Rc<Cell<usize>>,
        seen: usize,
    }

    struct LiveGuard {
        live: Rc<Cell<usize>>,
    }
    impl Drop for LiveGuard {
        fn drop(&mut self) {
            self.live.set(self.live.get() - 1);
        }
    }

    impl ResidentProbe {
        fn new() -> Self {
            ResidentProbe {
                live: Rc::new(Cell::new(0)),
                peak: Rc::new(Cell::new(0)),
                seen: 0,
            }
        }
        fn ingest(&mut self, _record: Record) {
            let now = self.live.get() + 1;
            self.live.set(now);
            if now > self.peak.get() {
                self.peak.set(now);
            }
            let _guard = LiveGuard {
                live: self.live.clone(),
            };
            self.seen += 1;
        }
    }

    #[test]
    fn large_dataset_roundtrips_in_bounded_memory() {
        // Tens of thousands of records plus a few multi-KB payloads. The total
        // logical size is many MB; bounded streaming must keep peak heap far
        // below that.
        const N: usize = 40_000;
        const PAYLOAD: usize = 128;
        let total_payload_bytes = N * PAYLOAD;

        let _probe = probe_guard();
        let tmp = TempDir::new("large");
        let bundle = tmp.path().join("big.shapestore");

        // Export straight from a lazy generator (no full set in RAM), measuring
        // peak heap residency across the streaming, sharded, parallel write.
        let start = std::time::Instant::now();
        super::alloc_probe::arm();
        let manifest =
            crate::format::export_stream(big_records(N, PAYLOAD), &bundle, DEFAULT_SHARD_COUNT)
                .unwrap();
        let export_peak = super::alloc_probe::disarm_peak();
        assert_eq!(manifest.total_records as usize, N);

        let mut probe = ResidentProbe::new();
        super::alloc_probe::arm();
        import_stream(&bundle, |record| {
            probe.ingest(record);
            Ok(())
        })
        .unwrap();
        let import_peak_bytes = super::alloc_probe::disarm_peak();
        let elapsed = start.elapsed();

        assert_eq!(probe.seen, N, "every record was streamed to the sink");
        assert_eq!(
            probe.peak.get(),
            1,
            "import kept at most ONE record simultaneously resident, got {}",
            probe.peak.get()
        );

        eprintln!(
            "bounded-memory: N={N} total_payload={total_payload_bytes}B \
             export_peak={export_peak}B import_peak={import_peak_bytes}B \
             resident_records={} elapsed={elapsed:?}",
            probe.peak.get()
        );

        // Heap peak during export/import must be a small fraction of the total
        // payload, proving neither materialized the whole store.
        let bound = total_payload_bytes / 2;
        assert!(
            export_peak < bound,
            "export peak heap {export_peak} not bounded (>= {bound}, total {total_payload_bytes})"
        );
        assert!(
            import_peak_bytes < bound,
            "import peak heap {import_peak_bytes} not bounded (>= {bound}, total {total_payload_bytes})"
        );

        assert!(
            elapsed.as_secs() < 30,
            "large roundtrip too slow: {elapsed:?}"
        );

        let mut into = MemoryAdapter::new();
        into.import(&bundle).unwrap();
        assert_eq!(into.len(), N);
        for i in [0usize, N / 2, N - 1] {
            let got = into.load(&format!("big-{i:08}")).unwrap();
            assert_eq!(got.payload, vec![(i % 251) as u8; PAYLOAD]);
            assert_eq!(got.version, (i as u64) % 9 + 1);
        }
    }

    #[test]
    fn large_file_adapter_import_export_roundtrip() {
        // Same scale, exercising the FileAdapter's streaming import (2-way
        // merge + re-shard) and streaming export off disk.
        const N: usize = 20_000;
        const PAYLOAD: usize = 48;
        // Avoid overlapping the allocator-probing large test (process-global
        // probe + heavy allocations would pollute its peak measurement).
        let _probe = probe_guard();
        let tmp = TempDir::new("largefile");
        let src = tmp.path().join("src.shapestore");
        crate::format::export_stream(big_records(N, PAYLOAD), &src, DEFAULT_SHARD_COUNT).unwrap();

        let mut file = open_file(&tmp, "file.shapestore");
        file.import(&src).unwrap();
        assert_eq!(file.len(), N);

        let re = tmp.path().join("re.shapestore");
        file.export(&re).unwrap();
        assert_eq!(read_bundle_bytes(&src), read_bundle_bytes(&re));

        let probe_id = format!("big-{:08}", N / 3);
        assert!(shard_index_of(&probe_id, DEFAULT_SHARD_COUNT) < DEFAULT_SHARD_COUNT);
        assert!(file.delete(&probe_id).unwrap());
        assert_eq!(file.len(), N - 1);
        assert!(matches!(
            file.load(&probe_id),
            Err(StorageError::NotFound { .. })
        ));
    }
}
