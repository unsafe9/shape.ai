//! Native redb-on-file adapter: an embedded, sync, single-writer KV engine with
//! two tables in one `Database` file:
//!
//! * **main** (`id -> value`): the store-neutral [`Record`], with the payload
//!   **zstd-compressed at rest**; `load` decompresses transparently (byte-exact).
//! * **region** (`region_row_key(canvas, morton, object_id) -> value`): the
//!   Morton (Z-order) index, whose value frames `{object_id, bbox}` so
//!   [`query_region`](AsyncStorageAdapter::query_region) range-scans the window
//!   and refilters on bbox without loading the main record — only the survivors.
//!
//! The async surface runs each sync redb op inside an immediately-ready
//! `async { ... }` block; no `.await` holds a non-`Send` value, so the returned
//! futures are `Send`. Pointer-width-agnostic: every framed field is a fixed-
//! width big-endian integer or length-prefixed bytes; no `usize` reaches keys.

use crate::adapter::{AdapterKind, RecordCursor, StorageAdapter};
use crate::adapter_async::{AsyncStorageAdapter, RegionWindow};
use crate::error::{Result, StorageError};
use crate::morton::{morton_of_world, region_row_key, region_scan_end_excl, region_scan_start};
use crate::record::{Record, StoreSnapshot};
use crate::spatial::{RegionKey, SpatialStore};
use redb::{Database, ReadableTable, ReadableTableMetadata, TableDefinition};
use std::path::Path;
use std::sync::Arc;

const MAIN: TableDefinition<&str, &[u8]> = TableDefinition::new("main");
const REGION: TableDefinition<&[u8], &[u8]> = TableDefinition::new("region");

/// redb-on-file store. Cloneable: the [`Database`] is shared behind an [`Arc`]
/// (redb is internally `Send + Sync`), so cheap clones share one file handle —
/// which the async trait's `&self` methods rely on.
#[derive(Clone)]
pub struct RedbAdapter {
    db: Arc<Database>,
}

impl RedbAdapter {
    /// Open (or create) a redb store at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let db = Database::create(path.as_ref()).map_err(map_db)?;
        Self::from_db(db)
    }

    /// Open a redb store on an in-memory backend (for tests), never touching the
    /// filesystem.
    pub fn open_in_memory() -> Result<Self> {
        let db = Database::builder()
            .create_with_backend(redb::backends::InMemoryBackend::new())
            .map_err(map_db)?;
        Self::from_db(db)
    }

    /// Materialize both tables so reads on a fresh store see empty tables rather
    /// than "table not found".
    fn from_db(db: Database) -> Result<Self> {
        let txn = db.begin_write().map_err(map_txn)?;
        {
            let _ = txn.open_table(MAIN).map_err(map_table)?;
            let _ = txn.open_table(REGION).map_err(map_table)?;
        }
        txn.commit().map_err(map_commit)?;
        Ok(RedbAdapter { db: Arc::new(db) })
    }

    /// Number of records held (a redb table `len`, not a full scan into memory).
    pub fn len(&self) -> usize {
        self.try_len().unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn try_len(&self) -> Result<usize> {
        let txn = self.db.begin_read().map_err(map_txn)?;
        let table = txn.open_table(MAIN).map_err(map_table)?;
        let n = table.len().map_err(map_storage)?;
        Ok(usize::try_from(n).unwrap_or(usize::MAX))
    }

    // ---- sync core: every public op (sync and async) funnels through these ----

    fn put_record(&self, record: &Record) -> Result<()> {
        let txn = self.db.begin_write().map_err(map_txn)?;
        {
            let mut table = txn.open_table(MAIN).map_err(map_table)?;
            let value = encode_record_value(record);
            table
                .insert(record.id.as_str(), value.as_slice())
                .map_err(map_storage)?;
        }
        txn.commit().map_err(map_commit)?;
        Ok(())
    }

    fn get_record(&self, id: &str) -> Result<Record> {
        let txn = self.db.begin_read().map_err(map_txn)?;
        let table = txn.open_table(MAIN).map_err(map_table)?;
        let value = table
            .get(id)
            .map_err(map_storage)?
            .ok_or_else(|| StorageError::NotFound { id: id.to_string() })?;
        decode_record_value(id, value.value())
    }

    fn remove_record(&self, id: &str) -> Result<bool> {
        let txn = self.db.begin_write().map_err(map_txn)?;
        let removed;
        {
            let mut main = txn.open_table(MAIN).map_err(map_table)?;
            removed = main.remove(id).map_err(map_storage)?.is_some();
            // Drop any region row(s) for this id too, so the index never
            // outlives its record.
            if removed {
                let mut region = txn.open_table(REGION).map_err(map_table)?;
                let stale: Vec<Vec<u8>> = collect_region_rows_for(&region, id)?;
                for key in stale {
                    region.remove(key.as_slice()).map_err(map_storage)?;
                }
            }
        }
        txn.commit().map_err(map_commit)?;
        Ok(removed)
    }

    fn all_ids(&self) -> Result<Vec<String>> {
        let txn = self.db.begin_read().map_err(map_txn)?;
        let table = txn.open_table(MAIN).map_err(map_table)?;
        let mut ids = Vec::new();
        // redb ranges are key-sorted, so this is already id-sorted.
        for entry in table.iter().map_err(map_storage)? {
            let (k, _) = entry.map_err(map_storage)?;
            ids.push(k.value().to_string());
        }
        Ok(ids)
    }

    fn all_records(&self) -> Result<Vec<Record>> {
        let txn = self.db.begin_read().map_err(map_txn)?;
        let table = txn.open_table(MAIN).map_err(map_table)?;
        let mut out = Vec::new();
        for entry in table.iter().map_err(map_storage)? {
            let (k, v) = entry.map_err(map_storage)?;
            out.push(decode_record_value(k.value(), v.value())?);
        }
        Ok(out)
    }

    fn replace_all(&self, snapshot: &StoreSnapshot) -> Result<()> {
        let txn = self.db.begin_write().map_err(map_txn)?;
        {
            // redb has no "truncate", so drop every existing key.
            let mut main = txn.open_table(MAIN).map_err(map_table)?;
            let keys: Vec<String> = {
                let mut ks = Vec::new();
                for entry in main.iter().map_err(map_storage)? {
                    let (k, _) = entry.map_err(map_storage)?;
                    ks.push(k.value().to_string());
                }
                ks
            };
            for k in keys {
                main.remove(k.as_str()).map_err(map_storage)?;
            }
            for record in snapshot.records() {
                let value = encode_record_value(record);
                main.insert(record.id.as_str(), value.as_slice())
                    .map_err(map_storage)?;
            }
        }
        {
            // region rows aren't part of the portable snapshot, so clear the
            // index too rather than leave it pointing at records that may be gone.
            let mut region = txn.open_table(REGION).map_err(map_table)?;
            let keys: Vec<Vec<u8>> = {
                let mut ks = Vec::new();
                for entry in region.iter().map_err(map_storage)? {
                    let (k, _) = entry.map_err(map_storage)?;
                    ks.push(k.value().to_vec());
                }
                ks
            };
            for k in keys {
                region.remove(k.as_slice()).map_err(map_storage)?;
            }
        }
        txn.commit().map_err(map_commit)?;
        Ok(())
    }

    fn put_indexed(&self, record: &Record, key: Option<&RegionKey>) -> Result<()> {
        let txn = self.db.begin_write().map_err(map_txn)?;
        {
            let mut main = txn.open_table(MAIN).map_err(map_table)?;
            let value = encode_record_value(record);
            main.insert(record.id.as_str(), value.as_slice())
                .map_err(map_storage)?;
        }
        {
            let mut region = txn.open_table(REGION).map_err(map_table)?;
            // Drop any prior region row(s) for this id first so a moved or
            // un-indexed record leaves no stale Z-order entry behind.
            let stale: Vec<Vec<u8>> = collect_region_rows_for(&region, &record.id)?;
            for k in stale {
                region.remove(k.as_slice()).map_err(map_storage)?;
            }
            if let Some(key) = key {
                // Index at the bbox center's Morton code; the window query
                // range-scans by center and refilters on the stored bbox.
                let cx = (key.min_x + key.max_x) / 2.0;
                let cy = (key.min_y + key.max_y) / 2.0;
                let morton = morton_of_world(cx, cy);
                let row_key = region_row_key(&key.canvas_id, morton, &record.id);
                let value = encode_region_value(&record.id, key);
                region
                    .insert(row_key.as_slice(), value.as_slice())
                    .map_err(map_storage)?;
            }
        }
        txn.commit().map_err(map_commit)?;
        Ok(())
    }

    fn region_query(
        &self,
        canvas_id: &str,
        window: Option<RegionWindow>,
    ) -> Result<Vec<Record>> {
        let txn = self.db.begin_read().map_err(map_txn)?;
        let region = txn.open_table(REGION).map_err(map_table)?;

        // Half-open key range: the Morton window, or the whole canvas when
        // `window` is None. The end is the EXCLUSIVE first key past cell `hi`,
        // covering every object_id tail within that cell.
        let (lo, hi) = match window {
            Some(w) => w.morton_range(),
            None => (u64::MIN, u64::MAX),
        };
        let lo_key = region_scan_start(canvas_id, lo);
        let hi_key = region_scan_end_excl(canvas_id, hi);

        // Range-scan, refilter each candidate on its exact bbox (carried in the
        // index value, so this never touches the main table), and collect ids.
        let mut ids: Vec<String> = Vec::new();
        let range = region
            .range(lo_key.as_slice()..hi_key.as_slice())
            .map_err(map_storage)?;
        for entry in range {
            let (_, v) = entry.map_err(map_storage)?;
            let (object_id, bbox) = decode_region_value(v.value())?;
            let candidate = RegionKey {
                canvas_id: canvas_id.to_string(),
                min_x: bbox.0,
                min_y: bbox.1,
                max_x: bbox.2,
                max_y: bbox.3,
            };
            let keep = match window {
                Some(w) => w.overlaps(&candidate),
                None => true,
            };
            if keep {
                ids.push(object_id);
            }
        }
        drop(region);

        // Load the survivors from the main table, id-sorted.
        ids.sort();
        ids.dedup();
        let main = txn.open_table(MAIN).map_err(map_table)?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(value) = main.get(id.as_str()).map_err(map_storage)? {
                out.push(decode_record_value(&id, value.value())?);
            }
        }
        Ok(out)
    }
}

impl StorageAdapter for RedbAdapter {
    fn kind(&self) -> AdapterKind {
        AdapterKind::Redb
    }

    fn save(&mut self, record: Record) -> Result<()> {
        self.put_record(&record)
    }

    fn load(&self, id: &str) -> Result<Record> {
        self.get_record(id)
    }

    fn delete(&mut self, id: &str) -> Result<bool> {
        self.remove_record(id)
    }

    fn list(&self) -> Result<Vec<String>> {
        self.all_ids()
    }

    fn records(&self) -> Result<RecordCursor<'_>> {
        // redb's range iterator borrows the read transaction, so streaming lazily
        // would need a self-referential cursor. We instead materialize the
        // id-sorted records (redb keys are already sorted) and hand back an owning
        // iterator; a keyset-paginated cursor would be needed for strictly
        // bounded export of a very large redb store.
        let records = self.all_records()?;
        Ok(Box::new(records.into_iter().map(Ok)))
    }

    fn snapshot(&self) -> Result<StoreSnapshot> {
        let mut snap = StoreSnapshot::new();
        for record in self.all_records()? {
            snap.insert(record);
        }
        Ok(snap)
    }

    fn restore(&mut self, snapshot: StoreSnapshot) -> Result<()> {
        self.replace_all(&snapshot)
    }
}

impl AsyncStorageAdapter for RedbAdapter {
    fn save(&self, record: Record) -> impl core::future::Future<Output = Result<()>> + Send {
        let this = self.clone();
        async move { this.put_record(&record) }
    }

    fn load(&self, id: &str) -> impl core::future::Future<Output = Result<Record>> + Send {
        let this = self.clone();
        let id = id.to_string();
        async move { this.get_record(&id) }
    }

    fn delete(&self, id: &str) -> impl core::future::Future<Output = Result<bool>> + Send {
        let this = self.clone();
        let id = id.to_string();
        async move { this.remove_record(&id) }
    }

    fn list(&self) -> impl core::future::Future<Output = Result<Vec<String>>> + Send {
        let this = self.clone();
        async move { this.all_ids() }
    }

    fn snapshot(&self) -> impl core::future::Future<Output = Result<StoreSnapshot>> + Send {
        let this = self.clone();
        async move {
            let mut snap = StoreSnapshot::new();
            for record in this.all_records()? {
                snap.insert(record);
            }
            Ok(snap)
        }
    }

    fn restore(
        &self,
        snapshot: StoreSnapshot,
    ) -> impl core::future::Future<Output = Result<()>> + Send {
        let this = self.clone();
        async move { this.replace_all(&snapshot) }
    }

    fn save_indexed(
        &self,
        record: Record,
        key: Option<RegionKey>,
    ) -> impl core::future::Future<Output = Result<()>> + Send {
        let this = self.clone();
        async move { this.put_indexed(&record, key.as_ref()) }
    }

    fn query_region(
        &self,
        canvas_id: &str,
        window: Option<RegionWindow>,
    ) -> impl core::future::Future<Output = Result<Vec<Record>>> + Send {
        let this = self.clone();
        let canvas_id = canvas_id.to_string();
        async move { this.region_query(&canvas_id, window) }
    }
}

impl SpatialStore for RedbAdapter {
    fn save_indexed(&mut self, record: Record, key: Option<RegionKey>) -> Result<()> {
        self.put_indexed(&record, key.as_ref())
    }

    fn query_region(
        &self,
        canvas_id: &str,
        bbox: Option<(f64, f64, f64, f64)>,
    ) -> Result<RecordCursor<'_>> {
        // The sync window is a raw `(min, max)` tuple; the core scans by
        // `RegionWindow`. region_query already returns id-sorted records, so
        // hand back an owning iterator (bounded by the window).
        let window = bbox.map(|(min_x, min_y, max_x, max_y)| RegionWindow {
            min_x,
            min_y,
            max_x,
            max_y,
        });
        let records = self.region_query(canvas_id, window)?;
        Ok(Box::new(records.into_iter().map(Ok)))
    }
}

// ---- value framing ---------------------------------------------------------
//
// All multi-byte integers are big-endian; all variable bytes are u32-BE
// length-prefixed. No `usize` ever reaches the bytes.

/// Frame a [`Record`] for the main table:
///   kind_len(u32) | kind | version(u64) | payload_marker(u8) | raw_len(u32) | payload_bytes
///
/// `payload_marker` is `1` when `payload_bytes` is the zstd stream, `0` when
/// stored raw; `raw_len` is the original length. Tiny/empty payloads stay raw
/// when compression wouldn't shrink them, so `load` round-trips byte for byte.
fn encode_record_value(record: &Record) -> Vec<u8> {
    let mut out = Vec::new();
    put_bytes(&mut out, record.kind.as_bytes());
    out.extend_from_slice(&record.version.to_be_bytes());

    let raw_len = u32::try_from(record.payload.len()).unwrap_or(u32::MAX);
    if record.payload.is_empty() {
        out.push(0); // marker: raw
        out.extend_from_slice(&0u32.to_be_bytes());
    } else {
        let compressed = zstd_compress(&record.payload);
        if compressed.len() < record.payload.len() {
            out.push(1); // marker: zstd
            out.extend_from_slice(&raw_len.to_be_bytes());
            out.extend_from_slice(&compressed);
        } else {
            out.push(0); // marker: raw (compression did not help)
            out.extend_from_slice(&raw_len.to_be_bytes());
            out.extend_from_slice(&record.payload);
        }
    }
    out
}

/// Inverse of [`encode_record_value`]; decompresses when the marker says so.
fn decode_record_value(id: &str, bytes: &[u8]) -> Result<Record> {
    let mut cur = Cursor::new(bytes);
    let kind = cur.take_bytes()?;
    let version = cur.take_u64()?;
    let marker = cur.take_u8()?;
    let raw_len = cur.take_u32()? as usize;
    let body = cur.rest();
    let payload = match marker {
        0 => body.to_vec(),
        1 => zstd_decompress(body, raw_len)?,
        other => {
            return Err(StorageError::Format(format!(
                "redb: unknown payload marker {other} for id {id}"
            )))
        }
    };
    let kind = String::from_utf8(kind)
        .map_err(|e| StorageError::Format(format!("redb: kind not utf-8 for id {id}: {e}")))?;
    Ok(Record {
        id: id.to_string(),
        kind,
        version,
        payload,
    })
}

/// Frame a region index value: object_id_len(u32) | object_id | 4 x f64-BE bbox.
/// Carries the bbox so `query_region` refilters without loading the main record.
fn encode_region_value(object_id: &str, key: &RegionKey) -> Vec<u8> {
    let mut out = Vec::new();
    put_bytes(&mut out, object_id.as_bytes());
    out.extend_from_slice(&key.min_x.to_be_bytes());
    out.extend_from_slice(&key.min_y.to_be_bytes());
    out.extend_from_slice(&key.max_x.to_be_bytes());
    out.extend_from_slice(&key.max_y.to_be_bytes());
    out
}

/// Inverse of [`encode_region_value`]: `(object_id, (min_x,min_y,max_x,max_y))`.
fn decode_region_value(bytes: &[u8]) -> Result<(String, (f64, f64, f64, f64))> {
    let mut cur = Cursor::new(bytes);
    let id_bytes = cur.take_bytes()?;
    let object_id = String::from_utf8(id_bytes)
        .map_err(|e| StorageError::Format(format!("redb: region object_id not utf-8: {e}")))?;
    let min_x = cur.take_f64()?;
    let min_y = cur.take_f64()?;
    let max_x = cur.take_f64()?;
    let max_y = cur.take_f64()?;
    Ok((object_id, (min_x, min_y, max_x, max_y)))
}

/// Collect every region row key whose framed object_id equals `id`, to drop
/// stale rows on re-index and delete (the common case is zero or one row).
fn collect_region_rows_for(
    region: &impl ReadableTable<&'static [u8], &'static [u8]>,
    id: &str,
) -> Result<Vec<Vec<u8>>> {
    let mut keys = Vec::new();
    for entry in region.iter().map_err(map_storage)? {
        let (k, v) = entry.map_err(map_storage)?;
        let (object_id, _) = decode_region_value(v.value())?;
        if object_id == id {
            keys.push(k.value().to_vec());
        }
    }
    Ok(keys)
}

/// Append a u32-BE length-prefixed byte run.
fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    let len = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(bytes);
}

/// A minimal big-endian reader for the framed values.
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Cursor { bytes, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.bytes.len());
        match end {
            Some(end) => {
                let out = &self.bytes[self.pos..end];
                self.pos = end;
                Ok(out)
            }
            None => Err(StorageError::Format(
                "redb: framed value truncated".to_string(),
            )),
        }
    }

    fn take_u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn take_u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn take_u64(&mut self) -> Result<u64> {
        let b = self.take(8)?;
        Ok(u64::from_be_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    fn take_f64(&mut self) -> Result<f64> {
        Ok(f64::from_bits(self.take_u64()?))
    }

    fn take_bytes(&mut self) -> Result<Vec<u8>> {
        let len = self.take_u32()? as usize;
        Ok(self.take(len)?.to_vec())
    }

    fn rest(&self) -> &'a [u8] {
        &self.bytes[self.pos..]
    }
}

// ---- zstd ------------------------------------------------------------------

/// A balanced speed/ratio point for at-rest payloads.
const ZSTD_LEVEL: i32 = 3;

/// Compress with zstd. On the (practically impossible) error path, fall back to
/// a raw copy; the caller adopts the result only when strictly smaller than raw.
fn zstd_compress(raw: &[u8]) -> Vec<u8> {
    zstd::encode_all(raw, ZSTD_LEVEL).unwrap_or_else(|_| raw.to_vec())
}

fn zstd_decompress(compressed: &[u8], _raw_len: usize) -> Result<Vec<u8>> {
    zstd::decode_all(compressed)
        .map_err(|e| StorageError::Format(format!("redb: zstd decode failed: {e}")))
}

// ---- error mapping (redb 2.x error enum split into one StorageError) --------

fn map_db(e: redb::DatabaseError) -> StorageError {
    StorageError::Io(format!("redb open: {e}"))
}
fn map_txn(e: redb::TransactionError) -> StorageError {
    StorageError::Io(format!("redb txn: {e}"))
}
fn map_table(e: redb::TableError) -> StorageError {
    StorageError::Io(format!("redb table: {e}"))
}
fn map_storage(e: redb::StorageError) -> StorageError {
    StorageError::Io(format!("redb storage: {e}"))
}
fn map_commit(e: redb::CommitError) -> StorageError {
    StorageError::Io(format!("redb commit: {e}"))
}

#[cfg(test)]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    reason = "test fixtures intentionally truncate to byte values"
)]
mod tests {
    // RedbAdapter impls both sync and async traits, which share method names;
    // having both in scope makes `store.save(..)` ambiguous, so each surface is
    // tested in its own module (this one brings only the SYNC trait into scope).
    use super::{
        decode_record_value, encode_record_value, Path, Record, RedbAdapter, RegionKey,
        StorageError,
    };
    use crate::adapter::StorageAdapter;
    use std::env;
    use std::path::PathBuf;

    /// `pub(super)` so the sibling `region_tests` module can reuse it.
    pub(super) struct TempDir(PathBuf);
    impl TempDir {
        pub(super) fn new(tag: &str) -> Self {
            let pid = std::process::id();
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = env::temp_dir().join(format!("shape_storage_redb_{tag}_{pid}_{nanos}"));
            std::fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }
        pub(super) fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn open(tmp: &TempDir, name: &str) -> RedbAdapter {
        RedbAdapter::open(tmp.path().join(name)).unwrap()
    }

    /// A record on `canvas` with a square bbox centered at `(cx, cy)`, half `r`.
    pub(super) fn at(id: &str, canvas: &str, cx: f64, cy: f64, r: f64) -> (Record, RegionKey) {
        (
            Record::new(id, "object", id.as_bytes().to_vec()),
            RegionKey {
                canvas_id: canvas.to_string(),
                min_x: cx - r,
                min_y: cy - r,
                max_x: cx + r,
                max_y: cy + r,
            },
        )
    }

    pub(super) fn block_on<F: core::future::Future>(fut: F) -> F::Output {
        // No-waker executor: the redb op is fully sync, so a single poll suffices.
        use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
        fn noop(_: *const ()) {}
        fn clone(_: *const ()) -> RawWaker {
            RawWaker::new(core::ptr::null(), &VTABLE)
        }
        static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
        let waker = unsafe { Waker::from_raw(RawWaker::new(core::ptr::null(), &VTABLE)) };
        let mut cx = Context::from_waker(&waker);
        let mut fut = Box::pin(fut);
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => v,
            Poll::Pending => panic!("redb future unexpectedly pended"),
        }
    }

    #[test]
    fn crud_round_trip() {
        let tmp = TempDir::new("crud");
        let mut store = open(&tmp, "store.redb");
        assert!(store.is_empty());

        store.save(Record::new("a", "card", b"one".to_vec())).unwrap();
        store.save(Record::new("b", "edge", b"two".to_vec())).unwrap();
        assert_eq!(store.len(), 2);
        assert_eq!(store.load("a").unwrap().payload, b"one");

        store
            .save(Record {
                id: "a".into(),
                kind: "card".into(),
                version: 7,
                payload: b"one-v2".to_vec(),
            })
            .unwrap();
        assert_eq!(store.len(), 2);
        let a = store.load("a").unwrap();
        assert_eq!(a.payload, b"one-v2");
        assert_eq!(a.version, 7);
        assert_eq!(a.kind, "card");

        assert_eq!(store.list().unwrap(), vec!["a", "b"]);

        assert!(store.delete("a").unwrap());
        assert!(!store.delete("a").unwrap());
        assert!(matches!(
            store.load("a"),
            Err(StorageError::NotFound { .. })
        ));
        assert_eq!(store.list().unwrap(), vec!["b"]);

        assert!(matches!(
            store.load("missing"),
            Err(StorageError::NotFound { .. })
        ));
    }

    #[test]
    fn list_and_records_are_id_sorted() {
        let tmp = TempDir::new("sorted");
        let mut store = open(&tmp, "store.redb");
        for id in ["c", "a", "b", "aa"] {
            store.save(Record::new(id, "k", id.as_bytes().to_vec())).unwrap();
        }
        assert_eq!(store.list().unwrap(), vec!["a", "aa", "b", "c"]);
        let ids: Vec<String> = store.records().unwrap().map(|r| r.unwrap().id).collect();
        assert_eq!(ids, vec!["a", "aa", "b", "c"]);
    }

    /// Payload survives the zstd at-rest round-trip exactly across empty,
    /// compressible, random-ish, and binary (NUL / 0xFF) payloads.
    #[test]
    fn payload_survives_zstd_round_trip() {
        let tmp = TempDir::new("zstd");
        let mut store = open(&tmp, "store.redb");

        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("empty", Vec::new()),
            ("tiny", b"x".to_vec()),
            ("compressible", vec![0u8; 100_000]),
            ("highbytes", vec![0xffu8; 8192]),
            ("nul-run", vec![0u8; 5]),
            (
                "varied",
                (0..50_000usize).map(|i| (i * 31 % 256) as u8).collect(),
            ),
            ("utf8", "payload-\u{1f600}-end".as_bytes().to_vec()),
        ];

        for (id, payload) in &cases {
            store
                .save(Record::new(*id, "object", payload.clone()))
                .unwrap();
        }
        for (id, payload) in &cases {
            let got = store.load(id).unwrap();
            assert_eq!(&got.payload, payload, "payload mismatch for {id}");
        }
    }

    /// The compressible payload must actually shrink on disk, proving zstd is
    /// applied (not just round-tripping raw).
    #[test]
    fn compressible_payload_actually_shrinks() {
        let raw = vec![7u8; 100_000];
        let record = Record::new("z", "object", raw.clone());
        let encoded = encode_record_value(&record);
        assert!(
            encoded.len() < raw.len() / 4,
            "expected zstd to shrink the value; got {} bytes for {} raw",
            encoded.len(),
            raw.len()
        );
        let back = decode_record_value("z", &encoded).unwrap();
        assert_eq!(back.payload, raw);
    }

    /// Data-size gate: the at-rest OBJECT payload stores geometry as a compact
    /// path-string `d`, never raw float point arrays or a tessellated mesh, and
    /// zstd shrinks the repetitive path-string materially while `load` round-
    /// trips the exact bytes.
    ///
    /// store-neutral here means no scene-core dependency, so the payload is
    /// hand-built JSON matching scene-core's `Object`/`Geometry` serde surface.
    #[test]
    fn object_payload_stores_path_string_not_raw_points_or_mesh() {
        // A many-node freehand stroke as a compact integer path-string (one M +
        // ~400 L linetos), the shape a real RDP-simplified sketch produces.
        let mut d = String::from("M 0 0");
        for i in 1..=400i32 {
            let x = i * 3;
            let y = (i * 7) % 50 - 25;
            d.push_str(&format!(" L {x} {y}"));
        }

        let payload_json = format!(
            r##"{{"sceneVersion":1,"objects":[{{"id":"stroke-1","order":"a0","transform":[[1,0,0],[0,1,0],[0,0,1]],"geometry":{{"d":"{d}","fillRule":"nonZero"}},"stroke":{{"paint":{{"kind":"solid","color":"#1a1a1a"}},"width":2,"cap":"round","join":"round"}}}}],"tags":[],"selection":{{"kind":"canvas"}},"updatedAt":"1970-01-01T00:00:00Z"}}"##
        );
        let raw = payload_json.into_bytes();

        let text = std::str::from_utf8(&raw).unwrap();
        assert!(text.contains(r#""d":"M 0 0 L 3"#), "payload must carry the path-string d");

        // Raw points / tessellated mesh are not persisted (`subpaths` is
        // #[serde(skip)]).
        for forbidden in ["\"mesh\"", "\"vertices\"", "\"indices\"", "\"subpaths\""] {
            assert!(
                !text.contains(forbidden),
                "at-rest object payload must not contain {forbidden} (raw points/mesh are not stored)"
            );
        }

        let record = Record::new("stroke-1", "object", raw.clone());
        let encoded = encode_record_value(&record);
        assert!(
            (encoded.len() as f64) < (raw.len() as f64) * 0.60,
            "expected zstd to store the object payload below 60% of raw; \
             got {} stored bytes for {} raw bytes",
            encoded.len(),
            raw.len()
        );

        // Use the pure codec, not a file-backed redb DB, so this size-gate test
        // stays light and doesn't inflate the allocator-probe peak of the
        // bounded-memory integrity tests under parallel `cargo test`.
        let got = decode_record_value("stroke-1", &encoded).unwrap();
        assert_eq!(got.payload, raw, "codec must round-trip the exact at-rest bytes");
        assert_eq!(got.kind, "object");
    }

    #[test]
    fn snapshot_restore_round_trip() {
        let tmp = TempDir::new("snap");
        let mut store = open(&tmp, "store.redb");
        for id in ["x", "y", "z"] {
            store.save(Record::new(id, "k", id.as_bytes().to_vec())).unwrap();
        }
        let snap = store.snapshot().unwrap();

        let mut other = open(&tmp, "other.redb");
        other.save(Record::new("stale", "k", b"gone".to_vec())).unwrap();
        other.restore(snap.clone()).unwrap();
        assert_eq!(other.snapshot().unwrap(), snap);
        assert!(matches!(
            other.load("stale"),
            Err(StorageError::NotFound { .. })
        ));
    }

    #[test]
    fn persists_across_reopen() {
        let tmp = TempDir::new("reopen");
        let db = tmp.path().join("store.redb");
        {
            let mut store = RedbAdapter::open(&db).unwrap();
            store.save(Record::new("x", "card", b"hi".to_vec())).unwrap();
            store.save(Record::new("y", "edge", b"yo".to_vec())).unwrap();
        }
        let store = RedbAdapter::open(&db).unwrap();
        assert_eq!(store.len(), 2);
        assert_eq!(store.load("x").unwrap().payload, b"hi");
        assert_eq!(store.load("y").unwrap().payload, b"yo");
        assert_eq!(store.list().unwrap(), vec!["x", "y"]);
    }
}

/// Async region-query tests in a separate module so only the ASYNC trait is in
/// method scope, without colliding with the sync names tested above.
#[cfg(test)]
mod region_tests {
    use super::tests::{at, block_on, TempDir};
    use super::{RedbAdapter, Record};
    use crate::adapter_async::{AsyncStorageAdapter, RegionWindow};

    fn open(tmp: &TempDir, name: &str) -> RedbAdapter {
        RedbAdapter::open(tmp.path().join(name)).unwrap()
    }

    /// save_indexed + query_region returns only window-overlapping records
    /// (id-sorted), None clears the row, delete drops the region row, canvases
    /// stay isolated.
    #[test]
    fn region_query_filters_and_excludes_far_away() {
        let tmp = TempDir::new("region");
        let store = open(&tmp, "store.redb");

        // alpha: three boxes along x; beta: two inside alpha's x-range (so a leak
        // would show up). Inserted out of id order.
        let fixture = [
            at("a-30", "alpha", 30.0, 0.0, 5.0),
            at("a-10", "alpha", 10.0, 0.0, 5.0),
            at("a-20", "alpha", 20.0, 0.0, 5.0),
            at("b-05", "beta", 5.0, 5.0, 2.0),
            at("b-15", "beta", 15.0, 15.0, 2.0),
        ];
        for (record, key) in fixture {
            block_on(store.save_indexed(record, Some(key))).unwrap();
        }

        let ids = |recs: Vec<Record>| -> Vec<String> { recs.into_iter().map(|r| r.id).collect() };

        assert_eq!(
            ids(block_on(store.query_region("alpha", None)).unwrap()),
            vec!["a-10", "a-20", "a-30"]
        );
        assert_eq!(
            ids(block_on(store.query_region("beta", None)).unwrap()),
            vec!["b-05", "b-15"]
        );

        // Window over x in [6, 24] selects a-10 ([5,15]) and a-20 ([15,25]).
        let w = Some(RegionWindow {
            min_x: 6.0,
            min_y: -1.0,
            max_x: 24.0,
            max_y: 1.0,
        });
        assert_eq!(
            ids(block_on(store.query_region("alpha", w)).unwrap()),
            vec!["a-10", "a-20"]
        );

        let far = Some(RegionWindow {
            min_x: 1000.0,
            min_y: 1000.0,
            max_x: 2000.0,
            max_y: 2000.0,
        });
        assert!(block_on(store.query_region("alpha", far)).unwrap().is_empty());

        // beta's boxes never leak into an alpha query despite sharing x.
        let alpha_all = ids(block_on(store.query_region("alpha", None)).unwrap());
        assert!(!alpha_all.iter().any(|id| id.starts_with("b-")));

        // save_indexed(None) clears the region row but keeps the record.
        let (rec, _) = at("a-20", "alpha", 20.0, 0.0, 5.0);
        block_on(store.save_indexed(rec, None)).unwrap();
        assert_eq!(
            ids(block_on(store.query_region("alpha", None)).unwrap()),
            vec!["a-10", "a-30"]
        );
        assert!(
            block_on(store.load("a-20")).is_ok(),
            "record survives un-indexing"
        );

        assert!(block_on(store.delete("a-10")).unwrap());
        assert_eq!(
            ids(block_on(store.query_region("alpha", None)).unwrap()),
            vec!["a-30"]
        );
    }

    /// Re-indexing a moved record must not leave a stale Z-order row.
    #[test]
    fn reindex_moves_without_stale_rows() {
        let tmp = TempDir::new("reindex");
        let store = open(&tmp, "store.redb");

        let (rec, key) = at("a-10", "alpha", 10.0, 0.0, 5.0);
        block_on(store.save_indexed(rec, Some(key))).unwrap();

        let (rec, key) = at("a-10", "alpha", 500.0, 500.0, 5.0);
        block_on(store.save_indexed(rec, Some(key))).unwrap();

        let old = Some(RegionWindow {
            min_x: 5.0,
            min_y: -5.0,
            max_x: 15.0,
            max_y: 5.0,
        });
        assert!(
            block_on(store.query_region("alpha", old)).unwrap().is_empty(),
            "stale row at the old position must be gone"
        );

        let new = Some(RegionWindow {
            min_x: 495.0,
            min_y: 495.0,
            max_x: 505.0,
            max_y: 505.0,
        });
        let got: Vec<String> = block_on(store.query_region("alpha", new))
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(got, vec!["a-10"]);

        assert_eq!(
            block_on(store.query_region("alpha", None)).unwrap().len(),
            1
        );
    }

    /// Region index survives reopen on a fresh adapter reading the same file.
    #[test]
    fn region_index_persists_across_reopen() {
        let tmp = TempDir::new("region-reopen");
        let db = tmp.path().join("store.redb");
        {
            let store = RedbAdapter::open(&db).unwrap();
            for (record, key) in [
                at("a-10", "alpha", 10.0, 0.0, 5.0),
                at("a-30", "alpha", 30.0, 0.0, 5.0),
            ] {
                block_on(store.save_indexed(record, Some(key))).unwrap();
            }
        }
        let store = RedbAdapter::open(&db).unwrap();
        let ids: Vec<String> = block_on(store.query_region("alpha", None))
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, vec!["a-10", "a-30"]);
    }
}
