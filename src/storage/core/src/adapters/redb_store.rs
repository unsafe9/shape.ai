//! Native redb-on-file adapter (OB3.T1 + T2 Morton region index + T4 zstd).
//!
//! redb is an embedded, sync, single-writer transactional KV engine. This
//! adapter backs a store with two redb tables in one `Database` file:
//!
//! * **main** (`id -> value`): the store-neutral [`Record`]. The value frames
//!   `{kind, version, payload}` with the payload **zstd-compressed at rest**
//!   (T4); `load` decompresses transparently, so the payload round-trips byte
//!   for byte.
//! * **region** (`region_row_key(canvas, morton, object_id) -> value`): the Morton (Z-order)
//!   region index (T2). The value frames `{object_id, bbox}`, so
//!   [`query_region`](AsyncStorageAdapter::query_region) can range-scan the
//!   Z-order window and refilter on the exact bbox **without loading the main
//!   record** — it only loads the survivors. See [`crate::morton`].
//!
//! Native-only: redb, zstd, and the redb file all assume `std::fs`, so the whole
//! module is gated `#[cfg(not(target_arch = "wasm32"))]` at the module site
//! (see `adapters/mod.rs`), exactly like the file/sqlite adapters. It is also
//! behind a `redb` cargo feature the Integrate phase adds.
//!
//! The async surface ([`AsyncStorageAdapter`]) is the redb cutover target
//! (OB1.4): redb is sync, so each async method runs the sync redb op inside an
//! immediately-ready `async { ... }` block. No `.await` is held across any
//! non-`Send` value, so the returned futures are `Send` as the trait requires.
//!
//! Pointer-width-agnostic: every framed field is a fixed-width big-endian
//! integer or length-prefixed bytes; no `usize` ever reaches the wire/keys.

use crate::adapter::{AdapterKind, RecordCursor, StorageAdapter};
use crate::adapter_async::{AsyncStorageAdapter, RegionWindow};
use crate::error::{Result, StorageError};
use crate::morton::{morton_of_world, region_row_key, region_scan_end_excl, region_scan_start};
use crate::record::{Record, StoreSnapshot};
use crate::spatial::{RegionKey, SpatialStore};
use redb::{Database, ReadableTable, ReadableTableMetadata, TableDefinition};
use std::path::Path;
use std::sync::Arc;

/// The main record table: `id -> framed record value`.
const MAIN: TableDefinition<&str, &[u8]> = TableDefinition::new("main");

/// The region index table: `region_row_key(canvas, morton, object_id) -> framed region value`.
const REGION: TableDefinition<&[u8], &[u8]> = TableDefinition::new("region");

/// redb-on-file store. Cloneable: the underlying [`Database`] is shared behind an
/// [`Arc`] (redb is internally `Send + Sync`), so cheap clones share one file
/// handle — which the async trait's `&self` methods rely on.
#[derive(Clone)]
pub struct RedbAdapter {
    db: Arc<Database>,
}

impl RedbAdapter {
    /// Open (or create) a redb store at `path`. The two tables are created lazily
    /// on first write; opening only needs the database file.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let db = Database::create(path.as_ref()).map_err(map_db)?;
        Self::from_db(db)
    }

    /// Open a redb store backed by an in-memory backend (for tests). Shares the
    /// same surface as [`open`](Self::open) but never touches the filesystem, so
    /// the server's in-memory registry/actor tests run without a temp file.
    pub fn open_in_memory() -> Result<Self> {
        let db = Database::builder()
            .create_with_backend(redb::backends::InMemoryBackend::new())
            .map_err(map_db)?;
        Self::from_db(db)
    }

    /// Materialize both tables so reads on a fresh store see empty tables rather
    /// than "table not found", then wrap the database. Shared by `open` and
    /// `open_in_memory`.
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

    /// Whether the store is empty.
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
            // Delete removes the record AND any stale region row(s) for it, so the
            // index never outlives its record. The region key is canvas-prefixed,
            // so we scan and drop every region row whose framed object_id matches.
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
            // Clear both tables by reopening them empty: redb has no "truncate",
            // so drop every existing key. `retain` would also work; explicit
            // removal keeps the single-writer txn obvious.
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
            // restore replaces the logical store; region rows are not part of the
            // portable snapshot, so clear the index too rather than leaving it
            // pointing at records that may no longer exist.
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
            // Always drop any prior region row(s) for this id first so a moved or
            // un-indexed record never leaves a stale Z-order entry behind.
            let stale: Vec<Vec<u8>> = collect_region_rows_for(&region, &record.id)?;
            for k in stale {
                region.remove(k.as_slice()).map_err(map_storage)?;
            }
            if let Some(key) = key {
                // Index at the bbox center's Morton code (the usual choice): the
                // window query range-scans by center and refilters on the stored
                // bbox, so the exact bbox is what decides membership.
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

        // Decide the half-open key range to scan: a Morton window for a bounded
        // query, or the whole canvas's Morton space when `window` is None. The
        // row keys carry an `object_id` tail (so co-located objects don't collide),
        // so the end is the EXCLUSIVE first key of the cell past `hi` — covering
        // every object id within the `hi` cell.
        let (lo, hi) = match window {
            Some(w) => w.morton_range(),
            None => (u64::MIN, u64::MAX),
        };
        let lo_key = region_scan_start(canvas_id, lo);
        let hi_key = region_scan_end_excl(canvas_id, hi);

        // Range-scan the Z-order window, refilter each candidate on its exact
        // bbox, and collect surviving object ids. Only the survivors' main
        // records are loaded — the index value carries the bbox so the refilter
        // never touches the main table.
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

        // Load the survivors from the main table and return them id-sorted.
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
        // INTEGRATE: add `AdapterKind::Redb` (+ `as_str` => "redb") in adapter.rs.
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
        // redb's range iterator borrows the read transaction, which would need a
        // self-referential cursor to stream lazily. The portable-format contract
        // only needs an id-sorted cursor, so we materialize the id-sorted records
        // up front (redb keys are already sorted) and hand back an owning
        // iterator. INTEGRATE NOTE: if a very large redb store must export in
        // strictly bounded memory, replace this with a keyset-paginated cursor
        // like sqlite's `KeysetCursor` (page by id ranges).
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
        // The sync `SpatialStore` window is a raw `(min, max)` AABB tuple; the
        // redb core scans by `RegionWindow`. region_query already returns the
        // window's records id-sorted, so hand back an owning iterator (bounded by
        // the window, exactly like the async surface).
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
// Manual, pointer-width-agnostic framing keeps the value bytes dependency-light
// and lets the main value carry the T4 zstd marker inline. All multi-byte
// integers are big-endian; all variable bytes are u32-BE length-prefixed.

/// Frame a [`Record`] for the main table:
///   kind_len(u32) | kind | version(u64) | payload_marker(u8) | raw_len(u32) | payload_bytes
///
/// The payload is zstd-compressed at rest (T4). `payload_marker` records whether
/// `payload_bytes` is the compressed stream (`1`) or stored raw (`0`); `raw_len`
/// is the original uncompressed length, used both to size the decode buffer and
/// to round-trip empty payloads exactly. Tiny/empty payloads are stored raw when
/// compression would not shrink them, so `load` round-trips byte for byte either
/// way.
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

/// Inverse of [`encode_record_value`]; decompresses the payload (T4) when the
/// marker says so, restoring the original bytes exactly.
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

/// Collect every region row key whose framed object_id equals `id`. Used to drop
/// stale index rows on re-index and on delete. Bounded by the index size; the
/// common case is zero or one row.
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

/// A minimal big-endian byte reader for the framed values.
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

// ---- zstd (T4) -------------------------------------------------------------

/// Default zstd level: a balanced speed/ratio point for at-rest payloads.
const ZSTD_LEVEL: i32 = 3;

/// Compress with zstd at [`ZSTD_LEVEL`]. On the (practically impossible) error
/// path, fall back to a copy of the raw bytes; the caller only adopts the result
/// when it is strictly smaller than raw, so a copy is harmless.
fn zstd_compress(raw: &[u8]) -> Vec<u8> {
    zstd::encode_all(raw, ZSTD_LEVEL).unwrap_or_else(|_| raw.to_vec())
}

/// Decompress a zstd stream into a `raw_len`-sized payload.
fn zstd_decompress(compressed: &[u8], _raw_len: usize) -> Result<Vec<u8>> {
    zstd::decode_all(compressed)
        .map_err(|e| StorageError::Format(format!("redb: zstd decode failed: {e}")))
}

// ---- error mapping ---------------------------------------------------------
//
// INTEGRATE NOTE: these map redb's distinct error types into `StorageError`.
// They are written against redb 2.x's error enum split (DatabaseError /
// TransactionError / TableError / StorageError / CommitError). If the resolved
// redb major version merges or renames these, collapse these helpers to match —
// the call sites only need `Fn(E) -> StorageError`. A dedicated
// `StorageError::Backend(String)` variant could replace the `Io(...)` reuse if
// the Integrate phase prefers; `Io` is used here to avoid editing error.rs.

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
    // `RedbAdapter` implements BOTH the sync `StorageAdapter` and the async
    // `AsyncStorageAdapter`, which share method names (save/load/delete/list/
    // snapshot/restore). Having both traits in method scope makes every
    // `store.save(..)` ambiguous, so the two surfaces are tested in separate
    // modules: this one brings only the SYNC trait into scope; the nested
    // `region` module (below) brings only the ASYNC trait. Shared helpers live
    // here and are imported by the nested module via `use super::*`.
    use super::{
        decode_record_value, encode_record_value, Path, Record, RedbAdapter, RegionKey,
        StorageError,
    };
    use crate::adapter::StorageAdapter;
    use std::env;
    use std::path::PathBuf;

    /// A unique temp dir under the OS temp root, cleaned up on drop. `pub(super)`
    /// so the sibling `region_tests` module can reuse it.
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
    /// `pub(super)` so the sibling `region_tests` module can reuse it.
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
        // Minimal no-waker executor: every future here completes without ever
        // yielding (the redb op is fully synchronous), so a single poll suffices.
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

        // Upsert by id, preserving version + kind.
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

        // list is id-sorted.
        assert_eq!(store.list().unwrap(), vec!["a", "b"]);

        // delete returns bool and is idempotent.
        assert!(store.delete("a").unwrap());
        assert!(!store.delete("a").unwrap());
        assert!(matches!(
            store.load("a"),
            Err(StorageError::NotFound { .. })
        ));
        assert_eq!(store.list().unwrap(), vec!["b"]);

        // missing id errors.
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

    /// T4: payload survives the zstd at-rest round-trip exactly, for empty,
    /// highly-compressible, random-ish, and binary (NUL / 0xFF) payloads.
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

    /// The compressible payload must actually shrink on disk: the value bytes
    /// stored for a 100k zero-run must be far smaller than the raw payload,
    /// proving zstd is applied (not just round-tripping raw).
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
        // And it still decodes back to the exact bytes.
        let back = decode_record_value("z", &encoded).unwrap();
        assert_eq!(back.payload, raw);
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

    /// Persistence: reopen the Database from the same path and confirm the data
    /// (and exact payloads) survive a fresh adapter instance.
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

/// Async region-query tests. Kept in a separate module so the ASYNC trait — not
/// the sync one — is the only `StorageAdapter`-shaped trait in method scope here,
/// making `block_on(store.save_indexed/query_region/load/delete(..))` resolve to
/// the async surface without colliding with the sync names tested above.
#[cfg(test)]
mod region_tests {
    use super::tests::{at, block_on, TempDir};
    use super::{RedbAdapter, Record};
    use crate::adapter_async::{AsyncStorageAdapter, RegionWindow};

    fn open(tmp: &TempDir, name: &str) -> RedbAdapter {
        RedbAdapter::open(tmp.path().join(name)).unwrap()
    }

    /// T2 + async: save_indexed + query_region returns only records overlapping
    /// the window and excludes far-away ones, id-sorted; None clears the row;
    /// delete drops the region row; canvases are isolated.
    #[test]
    fn region_query_filters_and_excludes_far_away() {
        let tmp = TempDir::new("region");
        let store = open(&tmp, "store.redb");

        // alpha: three boxes along x; beta: two boxes sitting inside alpha's
        // x-range (so a leak would show up). Inserted out of id order.
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

        // Whole-canvas (None window), id-sorted, per-canvas isolation.
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

        // A window far from everything returns nothing.
        let far = Some(RegionWindow {
            min_x: 1000.0,
            min_y: 1000.0,
            max_x: 2000.0,
            max_y: 2000.0,
        });
        assert!(block_on(store.query_region("alpha", far)).unwrap().is_empty());

        // beta's boxes never leak into an alpha query even though they share x.
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

        // delete removes the record AND its region row.
        assert!(block_on(store.delete("a-10")).unwrap());
        assert_eq!(
            ids(block_on(store.query_region("alpha", None)).unwrap()),
            vec!["a-30"]
        );
    }

    /// Re-indexing a moved record must not leave a stale Z-order row: after
    /// moving a-10 far away, a window over its old position no longer returns it,
    /// while a window over its new position does.
    #[test]
    fn reindex_moves_without_stale_rows() {
        let tmp = TempDir::new("reindex");
        let store = open(&tmp, "store.redb");

        let (rec, key) = at("a-10", "alpha", 10.0, 0.0, 5.0);
        block_on(store.save_indexed(rec, Some(key))).unwrap();

        // Move it to (500, 500).
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

        // Whole-canvas still has exactly one row.
        assert_eq!(
            block_on(store.query_region("alpha", None)).unwrap().len(),
            1
        );
    }

    /// Region index survives reopen: query_region works on a fresh adapter
    /// instance reading the same file.
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
