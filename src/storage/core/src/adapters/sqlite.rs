//! Native sqlite adapter (rusqlite).
//!
//! Backs a store with a single `records` table in a sqlite database (a file on
//! disk, or in-memory for tests). It is native-only and behind the default
//! `sqlite` feature, because rusqlite is a native-only optional dependency.
//!
//! Memory discipline: every read path is bounded.
//!
//! * [`load`](StorageAdapter::load) / [`delete`](StorageAdapter::delete) /
//!   [`save`](StorageAdapter::save) touch a single row.
//! * [`list`](StorageAdapter::list) selects only id strings, `ORDER BY id`.
//! * [`records`](StorageAdapter::records) is a **keyset-paginated** cursor: it
//!   pulls rows in id-sorted chunks of [`CURSOR_CHUNK`] and yields them one at a
//!   time, fetching the next chunk only when the current one drains. Peak
//!   resident is one chunk, never the whole table — so the trait's streaming
//!   `export`/`import` stay memory-bounded.
//!
//! ACID: bulk writes (`restore`, and the merging `import` driven by `ingest`)
//! run inside a single transaction.

use crate::adapter::{AdapterKind, RecordCursor, StorageAdapter};
use crate::error::{Result, StorageError};
use crate::record::{Record, StoreSnapshot};
use crate::spatial::{RegionKey, SpatialStore};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

/// Rows fetched per keyset page by the streaming [`records`](StorageAdapter::records)
/// cursor. Bounds the cursor's resident set to one page regardless of table size.
const CURSOR_CHUNK: usize = 256;

/// On-disk (or in-memory) sqlite-backed store.
pub struct SqliteAdapter {
    conn: Connection,
}

impl SqliteAdapter {
    /// Open (or create) a sqlite store at `path`, creating the schema if needed.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path.as_ref()).map_err(map_err)?;
        Self::with_connection(conn)
    }

    /// Open an in-memory sqlite store (for tests), creating the schema.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(map_err)?;
        Self::with_connection(conn)
    }

    fn with_connection(conn: Connection) -> Result<Self> {
        // Enforce the region_index -> records foreign key so deleting a record
        // cascades to its region row (sqlite leaves FKs off by default).
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(map_err)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS records (
                 id      TEXT PRIMARY KEY,
                 kind    TEXT NOT NULL,
                 version INTEGER NOT NULL,
                 payload BLOB NOT NULL
             );
             CREATE TABLE IF NOT EXISTS region_index (
                 id        TEXT PRIMARY KEY REFERENCES records(id) ON DELETE CASCADE,
                 canvas_id TEXT NOT NULL,
                 min_x     REAL NOT NULL,
                 min_y     REAL NOT NULL,
                 max_x     REAL NOT NULL,
                 max_y     REAL NOT NULL
             );
             CREATE INDEX IF NOT EXISTS region_index_canvas ON region_index(canvas_id);
             CREATE INDEX IF NOT EXISTS region_index_bbox
                 ON region_index(canvas_id, min_x, max_x, min_y, max_y);",
        )
        .map_err(map_err)?;
        Ok(SqliteAdapter { conn })
    }

    /// Number of records held (a `COUNT(*)`, not a full scan into memory).
    pub fn len(&self) -> usize {
        self.conn
            .query_row("SELECT COUNT(*) FROM records", [], |row| row.get::<_, i64>(0))
            .map(|n| usize::try_from(n).unwrap_or(usize::MAX))
            .unwrap_or(0)
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Upsert one record using the supplied connection-like handle.
    fn upsert_on(conn: &Connection, record: &Record) -> Result<()> {
        conn.execute(
            "INSERT INTO records (id, kind, version, payload)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
                 kind = excluded.kind,
                 version = excluded.version,
                 payload = excluded.payload",
            params![record.id, record.kind, version_to_sql(record.version), record.payload],
        )
        .map(|_| ())
        .map_err(map_err)
    }
}

impl StorageAdapter for SqliteAdapter {
    fn kind(&self) -> AdapterKind {
        AdapterKind::Sqlite
    }

    fn save(&mut self, record: Record) -> Result<()> {
        Self::upsert_on(&self.conn, &record)
    }

    fn load(&self, id: &str) -> Result<Record> {
        self.conn
            .query_row(
                "SELECT id, kind, version, payload FROM records WHERE id = ?1",
                params![id],
                row_to_record,
            )
            .optional()
            .map_err(map_err)?
            .ok_or_else(|| StorageError::NotFound { id: id.to_string() })
    }

    fn delete(&mut self, id: &str) -> Result<bool> {
        let affected = self
            .conn
            .execute("DELETE FROM records WHERE id = ?1", params![id])
            .map_err(map_err)?;
        Ok(affected > 0)
    }

    fn list(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM records ORDER BY id")
            .map_err(map_err)?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(map_err)?;
        let mut ids = Vec::new();
        for id in rows {
            ids.push(id.map_err(map_err)?);
        }
        Ok(ids)
    }

    fn records(&self) -> Result<RecordCursor<'_>> {
        Ok(Box::new(KeysetCursor::new(&self.conn)))
    }

    fn ingest(&mut self, record: Record) -> Result<()> {
        Self::upsert_on(&self.conn, &record)
    }

    fn snapshot(&self) -> Result<StoreSnapshot> {
        let mut snap = StoreSnapshot::new();
        for record in self.records()? {
            snap.insert(record?);
        }
        Ok(snap)
    }

    fn restore(&mut self, snapshot: StoreSnapshot) -> Result<()> {
        let tx = self.conn.transaction().map_err(map_err)?;
        tx.execute("DELETE FROM records", []).map_err(map_err)?;
        for record in snapshot.records() {
            Self::upsert_on(&tx, record)?;
        }
        tx.commit().map_err(map_err)?;
        Ok(())
    }

    fn import(&mut self, root: &Path) -> Result<()> {
        // Bulk upsert under one transaction. Records still stream one-at-a-time
        // from the bundle into the prepared statement — peak resident is one
        // record plus the bundle's per-shard buffers, never O(total).
        let tx = self.conn.transaction().map_err(map_err)?;
        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO records (id, kind, version, payload)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(id) DO UPDATE SET
                         kind = excluded.kind,
                         version = excluded.version,
                         payload = excluded.payload",
                )
                .map_err(map_err)?;
            crate::format::import_stream(root, |record| {
                stmt.execute(params![
                    record.id,
                    record.kind,
                    version_to_sql(record.version),
                    record.payload
                ])
                .map(|_| ())
                .map_err(map_err)
            })?;
        }
        tx.commit().map_err(map_err)?;
        Ok(())
    }
}

impl SpatialStore for SqliteAdapter {
    fn save_indexed(&mut self, record: Record, key: Option<RegionKey>) -> Result<()> {
        // Upsert the record and its region row (or clear the region row) under one
        // transaction so the index never lags the record write.
        let tx = self.conn.transaction().map_err(map_err)?;
        Self::upsert_on(&tx, &record)?;
        match key {
            Some(key) => {
                tx.execute(
                    "INSERT INTO region_index (id, canvas_id, min_x, min_y, max_x, max_y)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(id) DO UPDATE SET
                         canvas_id = excluded.canvas_id,
                         min_x = excluded.min_x,
                         min_y = excluded.min_y,
                         max_x = excluded.max_x,
                         max_y = excluded.max_y",
                    params![record.id, key.canvas_id, key.min_x, key.min_y, key.max_x, key.max_y],
                )
                .map_err(map_err)?;
            }
            None => {
                tx.execute("DELETE FROM region_index WHERE id = ?1", params![record.id])
                    .map_err(map_err)?;
            }
        }
        tx.commit().map_err(map_err)?;
        Ok(())
    }

    fn query_region(
        &self,
        canvas_id: &str,
        bbox: Option<(f64, f64, f64, f64)>,
    ) -> Result<RecordCursor<'_>> {
        Ok(Box::new(RegionCursor::new(&self.conn, canvas_id, bbox)))
    }
}

/// A bounded, keyset-paginated cursor over the `records` table in id order.
///
/// It never materializes the whole table: it fetches up to [`CURSOR_CHUNK`] rows
/// at a time (ids strictly greater than the last id yielded) and yields them one
/// at a time, fetching the next page only once the current one is exhausted. The
/// `id` primary key gives a deterministic, gap-free ascending walk.
struct KeysetCursor<'c> {
    conn: &'c Connection,
    /// Buffered page, drained front-to-back via `pos`.
    page: Vec<Record>,
    pos: usize,
    /// Last id yielded; the next page selects ids strictly greater than this.
    last_id: Option<String>,
    /// Set once a short (final) page has been seen, so we stop querying.
    done: bool,
    /// Carries a fatal query error to the next `next()` call.
    error: Option<StorageError>,
}

impl<'c> KeysetCursor<'c> {
    fn new(conn: &'c Connection) -> Self {
        KeysetCursor {
            conn,
            page: Vec::new(),
            pos: 0,
            last_id: None,
            done: false,
            error: None,
        }
    }

    /// Fetch the next page (ids > `last_id`) into `page`, resetting `pos`.
    fn fetch_page(&mut self) -> Result<()> {
        self.page.clear();
        self.pos = 0;
        let sql = "SELECT id, kind, version, payload FROM records
                   WHERE (?1 IS NULL OR id > ?1)
                   ORDER BY id LIMIT ?2";
        let mut stmt = self.conn.prepare(sql).map_err(map_err)?;
        let limit = i64::try_from(CURSOR_CHUNK).expect("CURSOR_CHUNK fits i64");
        let rows = stmt
            .query_map(
                params![self.last_id, limit],
                row_to_record,
            )
            .map_err(map_err)?;
        for row in rows {
            self.page.push(row.map_err(map_err)?);
        }
        // A short page means the table is exhausted.
        if self.page.len() < CURSOR_CHUNK {
            self.done = true;
        }
        if let Some(last) = self.page.last() {
            self.last_id = Some(last.id.clone());
        }
        Ok(())
    }
}

impl Iterator for KeysetCursor<'_> {
    type Item = Result<Record>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(err) = self.error.take() {
            return Some(Err(err));
        }
        if self.pos >= self.page.len() {
            if self.done {
                return None;
            }
            if let Err(e) = self.fetch_page() {
                return Some(Err(e));
            }
            if self.page.is_empty() {
                return None;
            }
        }
        let record = std::mem::replace(
            &mut self.page[self.pos],
            Record {
                id: String::new(),
                kind: String::new(),
                version: 0,
                payload: Vec::new(),
            },
        );
        self.pos += 1;
        Some(Ok(record))
    }
}

/// A bounded, keyset-paginated cursor for [`SpatialStore::query_region`].
///
/// Joins `records` to `region_index` filtered by `canvas_id` and (optionally) a
/// bbox-overlap window, walking `region_index.id` strictly ascending in pages of
/// [`CURSOR_CHUNK`]. Like [`KeysetCursor`], peak resident is one page, never the
/// whole result set. Ordering is `id`, matching the trait contract.
struct RegionCursor<'c> {
    conn: &'c Connection,
    canvas_id: String,
    bbox: Option<(f64, f64, f64, f64)>,
    page: Vec<Record>,
    pos: usize,
    last_id: Option<String>,
    done: bool,
}

impl<'c> RegionCursor<'c> {
    fn new(conn: &'c Connection, canvas_id: &str, bbox: Option<(f64, f64, f64, f64)>) -> Self {
        RegionCursor {
            conn,
            canvas_id: canvas_id.to_string(),
            bbox,
            page: Vec::new(),
            pos: 0,
            last_id: None,
            done: false,
        }
    }

    fn fetch_page(&mut self) -> Result<()> {
        self.page.clear();
        self.pos = 0;
        // bbox-overlap is inclusive AABB intersect; `NULL`-guarded params let one
        // statement serve both the windowed and whole-canvas (NULL bbox) cases.
        let sql = "SELECT r.id, r.kind, r.version, r.payload
                   FROM region_index AS ri
                   JOIN records AS r ON r.id = ri.id
                   WHERE ri.canvas_id = ?1
                     AND (?2 IS NULL OR ri.id > ?2)
                     AND (?3 IS NULL OR (
                            ri.min_x <= ?4 AND ri.max_x >= ?3
                        AND ri.min_y <= ?6 AND ri.max_y >= ?5))
                   ORDER BY ri.id LIMIT ?7";
        let mut stmt = self.conn.prepare(sql).map_err(map_err)?;
        let (qminx, qminy, qmaxx, qmaxy) = match self.bbox {
            Some(b) => (Some(b.0), Some(b.1), Some(b.2), Some(b.3)),
            None => (None, None, None, None),
        };
        let limit = i64::try_from(CURSOR_CHUNK).expect("CURSOR_CHUNK fits i64");
        let rows = stmt
            .query_map(
                params![
                    self.canvas_id,
                    self.last_id,
                    qminx,
                    qmaxx,
                    qminy,
                    qmaxy,
                    limit
                ],
                row_to_record,
            )
            .map_err(map_err)?;
        for row in rows {
            self.page.push(row.map_err(map_err)?);
        }
        if self.page.len() < CURSOR_CHUNK {
            self.done = true;
        }
        if let Some(last) = self.page.last() {
            self.last_id = Some(last.id.clone());
        }
        Ok(())
    }
}

impl Iterator for RegionCursor<'_> {
    type Item = Result<Record>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.page.len() {
            if self.done {
                return None;
            }
            if let Err(e) = self.fetch_page() {
                return Some(Err(e));
            }
            if self.page.is_empty() {
                return None;
            }
        }
        let record = std::mem::replace(
            &mut self.page[self.pos],
            Record {
                id: String::new(),
                kind: String::new(),
                version: 0,
                payload: Vec::new(),
            },
        );
        self.pos += 1;
        Some(Ok(record))
    }
}

/// Map a `records` row tuple into a [`Record`].
fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<Record> {
    Ok(Record {
        id: row.get(0)?,
        kind: row.get(1)?,
        version: version_from_sql(row.get::<_, i64>(2)?),
        payload: row.get(3)?,
    })
}

/// sqlite INTEGER columns are i64; `Record::version` is u64. Persist via a
/// lossless bit reinterpretation — round-trips for every value, and queries
/// order by id (never version), so the sign reinterpretation is inert.
fn version_to_sql(v: u64) -> i64 {
    i64::from_ne_bytes(v.to_ne_bytes())
}
fn version_from_sql(v: i64) -> u64 {
    u64::from_ne_bytes(v.to_ne_bytes())
}

/// Map a rusqlite error into a [`StorageError`] without panicking.
fn map_err(e: rusqlite::Error) -> StorageError {
    StorageError::Io(format!("sqlite: {e}"))
}

#[cfg(test)]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    reason = "test fixtures intentionally truncate to byte values"
)]
mod tests {
    use super::*;
    use crate::adapters::MemoryAdapter;
    use std::env;
    use std::path::PathBuf;

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let pid = std::process::id();
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = env::temp_dir().join(format!("shape_storage_sqlite_{tag}_{pid}_{nanos}"));
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
    fn crud_round_trip() {
        let mut store = SqliteAdapter::open_in_memory().unwrap();
        assert!(store.is_empty());

        store.save(Record::new("a", "card", b"one".to_vec())).unwrap();
        store.save(Record::new("b", "edge", b"two".to_vec())).unwrap();
        assert_eq!(store.len(), 2);
        assert_eq!(store.load("a").unwrap().payload, b"one");

        // Upsert by id.
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

        // Missing id.
        assert!(matches!(
            store.load("missing"),
            Err(StorageError::NotFound { .. })
        ));
    }

    #[test]
    fn list_is_id_sorted() {
        let mut store = SqliteAdapter::open_in_memory().unwrap();
        for id in ["c", "a", "b", "aa"] {
            store
                .save(Record::new(id, "k", id.as_bytes().to_vec()))
                .unwrap();
        }
        assert_eq!(store.list().unwrap(), vec!["a", "aa", "b", "c"]);
    }

    #[test]
    fn delete_returns_bool() {
        let mut store = SqliteAdapter::open_in_memory().unwrap();
        store.save(Record::new("a", "k", b"x".to_vec())).unwrap();
        assert!(store.delete("a").unwrap());
        assert!(!store.delete("a").unwrap());
        assert!(matches!(
            store.load("a"),
            Err(StorageError::NotFound { .. })
        ));
    }

    #[test]
    fn records_cursor_is_id_sorted_and_complete() {
        let mut store = SqliteAdapter::open_in_memory().unwrap();
        // Span more than one keyset page to exercise pagination.
        let n = CURSOR_CHUNK * 2 + 17;
        for i in 0..n {
            store
                .save(Record::new(
                    format!("id-{i:06}"),
                    "k",
                    format!("p-{i}").into_bytes(),
                ))
                .unwrap();
        }
        let ids: Vec<String> = store.records().unwrap().map(|r| r.unwrap().id).collect();
        assert_eq!(ids.len(), n);
        let mut expected: Vec<String> = (0..n).map(|i| format!("id-{i:06}")).collect();
        expected.sort();
        assert_eq!(ids, expected);
    }

    #[test]
    fn empty_records_cursor_yields_nothing() {
        let store = SqliteAdapter::open_in_memory().unwrap();
        assert_eq!(store.records().unwrap().count(), 0);
    }

    #[test]
    fn snapshot_restore_round_trip() {
        let mut store = SqliteAdapter::open_in_memory().unwrap();
        for id in ["x", "y", "z"] {
            store
                .save(Record::new(id, "k", id.as_bytes().to_vec()))
                .unwrap();
        }
        let snap = store.snapshot().unwrap();

        let mut other = SqliteAdapter::open_in_memory().unwrap();
        // Pre-existing row that restore must clear.
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
        let db = tmp.path().join("store.sqlite");
        {
            let mut store = SqliteAdapter::open(&db).unwrap();
            store.save(Record::new("a", "card", b"hi".to_vec())).unwrap();
            store.save(Record::new("b", "edge", b"yo".to_vec())).unwrap();
        }
        let store = SqliteAdapter::open(&db).unwrap();
        assert_eq!(store.len(), 2);
        assert_eq!(store.load("a").unwrap().payload, b"hi");
        assert_eq!(store.list().unwrap(), vec!["a", "b"]);
    }

    /// Build a varied dataset, save it into sqlite, export to a bundle, import
    /// into a MemoryAdapter, and assert the snapshots match byte-for-byte — the
    /// portable bundle interops across adapter kinds.
    #[test]
    fn cross_adapter_sqlite_export_to_memory_import() {
        let tmp = TempDir::new("cross");
        let bundle = tmp.path().join("bundle.shapestore");

        let mut sqlite = SqliteAdapter::open_in_memory().unwrap();
        for i in 0..300usize {
            let kind = ["card", "edge", "group", "tag"][i % 4];
            let payload: Vec<u8> = (0..(i % 37)).map(|b| (b * i % 256) as u8).collect();
            sqlite
                .save(Record {
                    id: format!("node-{i:04}"),
                    kind: kind.to_string(),
                    version: (i as u64) % 5 + 1,
                    payload,
                })
                .unwrap();
        }

        let manifest = sqlite.export(&bundle).unwrap();
        assert_eq!(manifest.total_records, 300);

        let mut mem = MemoryAdapter::new();
        mem.import(&bundle).unwrap();

        assert_eq!(mem.snapshot().unwrap(), sqlite.snapshot().unwrap());
    }

    /// Bundle bytes from a sqlite store must be byte-identical to those from a
    /// memory store holding the same logical contents.
    #[test]
    fn sqlite_and_memory_export_byte_identical() {
        let tmp = TempDir::new("bytes");
        let records: Vec<Record> = (0..150usize)
            .map(|i| Record {
                id: format!("rec-{:04}", (i * 7919) % 150),
                kind: ["a", "b", "c"][i % 3].to_string(),
                version: (i as u64) % 9,
                payload: vec![(i % 251) as u8; i % 13],
            })
            .collect();

        let mut sqlite = SqliteAdapter::open_in_memory().unwrap();
        let mut mem = MemoryAdapter::new();
        for r in &records {
            sqlite.save(r.clone()).unwrap();
            mem.save(r.clone()).unwrap();
        }

        let sqlite_bundle = tmp.path().join("sqlite.shapestore");
        let mem_bundle = tmp.path().join("mem.shapestore");
        let man_sqlite = sqlite.export(&sqlite_bundle).unwrap();
        let man_mem = mem.export(&mem_bundle).unwrap();
        assert_eq!(man_sqlite, man_mem);

        for entry in &man_sqlite.shards {
            let name = format!("shard-{:05}.bin", entry.index);
            let a = std::fs::read(sqlite_bundle.join(&name)).unwrap();
            let b = std::fs::read(mem_bundle.join(&name)).unwrap();
            assert_eq!(a, b, "shard {} differed", entry.index);
        }
    }

    /// Import several thousand records through the streaming bundle path and
    /// assert full correctness. The records()/import path is the streaming one,
    /// so memory stays bounded; here we assert the data, not the heap.
    #[test]
    fn bounded_bulk_import_is_correct() {
        const N: usize = 5_000;
        const PAYLOAD: usize = 32;
        let tmp = TempDir::new("bulk");
        let bundle = tmp.path().join("big.shapestore");

        // Build the bundle from a lazy generator (no full set in RAM).
        crate::format::export_stream(
            (0..N).map(|i| {
                Ok(Record {
                    id: format!("big-{i:08}"),
                    kind: ["card", "edge", "group"][i % 3].to_string(),
                    version: (i as u64) % 9 + 1,
                    payload: vec![(i % 251) as u8; PAYLOAD],
                })
            }),
            &bundle,
            crate::format::DEFAULT_SHARD_COUNT,
        )
        .unwrap();

        let mut sqlite = SqliteAdapter::open_in_memory().unwrap();
        sqlite.import(&bundle).unwrap();
        assert_eq!(sqlite.len(), N);

        // The streaming cursor reproduces every record in id order.
        let via_cursor: Vec<Record> = sqlite.records().unwrap().map(|r| r.unwrap()).collect();
        assert_eq!(via_cursor.len(), N);
        for i in [0usize, N / 2, N - 1] {
            let got = sqlite.load(&format!("big-{i:08}")).unwrap();
            assert_eq!(got.payload, vec![(i % 251) as u8; PAYLOAD]);
            assert_eq!(got.version, (i as u64) % 9 + 1);
        }
    }
}
