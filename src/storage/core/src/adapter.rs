//! The `StorageAdapter` trait every backend implements.

use crate::error::Result;
use crate::format::{export_stream, import_stream, Manifest, DEFAULT_SHARD_COUNT};
use crate::record::{Record, StoreSnapshot};
use std::path::Path;

/// The named store kinds the adapter layer abstracts over. Mirrors the
/// idea1.md list (sqlite / postgres / file / s3 / remote-server), plus the
/// real in-memory adapter that backs tests and the default store.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdapterKind {
    /// Fully-implemented in-process map store.
    Memory,
    /// Fully-implemented single-file store.
    File,
    /// Stub: needs an external sqlite backend.
    Sqlite,
    /// Stub: needs an external postgres backend.
    Postgres,
    /// Stub: needs an external S3 backend.
    S3,
    /// Stub: needs an external remote storage server.
    RemoteServer,
}

impl AdapterKind {
    /// Stable lowercase name, matching the idea1.md vocabulary.
    pub fn as_str(self) -> &'static str {
        match self {
            AdapterKind::Memory => "memory",
            AdapterKind::File => "file",
            AdapterKind::Sqlite => "sqlite",
            AdapterKind::Postgres => "postgres",
            AdapterKind::S3 => "s3",
            AdapterKind::RemoteServer => "remote-server",
        }
    }
}

/// A lazy, deterministic record cursor: yields a store's records in id-sorted
/// order, one at a time, without materializing them all. Returned by
/// [`StorageAdapter::records`] and consumed by the streaming export.
pub type RecordCursor<'a> = Box<dyn Iterator<Item = Result<Record>> + 'a>;

/// A data store the rest of the system can read and write through, regardless
/// of where the bytes actually live.
///
/// Three concerns live here:
///
/// 1. **In-store I/O** — `save` / `load` / `delete` / `list`, the per-record
///    operations every backend must support against its native format.
/// 2. **Streaming** — `records` (a lazy, id-sorted cursor) and `ingest` (a
///    one-at-a-time sink). These are what make `export`/`import` memory-safe
///    for large stores: neither ever holds the whole store in RAM.
/// 3. **Portability** — `snapshot` / `restore`, the bridge to the single
///    portable bundle format for small/in-memory use. `export` and `import`
///    are provided on top of the streaming pair, so any store can round-trip
///    through the on-disk bundle into any other store in **bounded memory**.
pub trait StorageAdapter {
    /// Which backend this is.
    fn kind(&self) -> AdapterKind;

    /// Persist a record, inserting or overwriting by id.
    fn save(&mut self, record: Record) -> Result<()>;

    /// Load a record by id.
    fn load(&self, id: &str) -> Result<Record>;

    /// Delete a record by id. Returns whether a record was removed.
    fn delete(&mut self, id: &str) -> Result<bool>;

    /// List all record ids currently held, in deterministic order.
    fn list(&self) -> Result<Vec<String>>;

    /// A lazy cursor over all records in **deterministic id-sorted order**.
    ///
    /// This is the memory-safe read path for export: implementations must yield
    /// records one at a time (e.g. shard-by-shard from disk) rather than
    /// building the whole store in memory. Ordering must be id-sorted so the
    /// exported bundle stays byte-stable.
    fn records(&self) -> Result<RecordCursor<'_>>;

    /// Streaming write sink for import: ingest a single record, inserting or
    /// overwriting by id, without requiring the whole store in memory.
    ///
    /// Defaults to [`save`](StorageAdapter::save); override when ingestion can
    /// be made cheaper than a full per-record persist.
    fn ingest(&mut self, record: Record) -> Result<()> {
        self.save(record)
    }

    /// Produce the full logical contents as a portable snapshot. For
    /// small/in-memory use; **not** used by streaming export.
    fn snapshot(&self) -> Result<StoreSnapshot>;

    /// Replace the store's contents with `snapshot`. For small/in-memory use;
    /// **not** used by streaming import.
    fn restore(&mut self, snapshot: StoreSnapshot) -> Result<()>;

    /// Export the entire store to the one portable bundle format at `root`,
    /// using the default shard count. Implemented once for all adapters.
    fn export(&self, root: &Path) -> Result<Manifest> {
        self.export_with_shards(root, DEFAULT_SHARD_COUNT)
    }

    /// Export with an explicit shard count (parallel-write fan-out width).
    ///
    /// Streams the [`records`](StorageAdapter::records) cursor straight to the
    /// bundle in bounded memory — it never builds a full snapshot.
    fn export_with_shards(&self, root: &Path, shard_count: u32) -> Result<Manifest> {
        export_stream(self.records()?, root, shard_count)
    }

    /// Import a portable bundle at `root`, merging its records into this store.
    ///
    /// Streams the bundle shard-by-shard into [`ingest`](StorageAdapter::ingest)
    /// in bounded memory — it never builds a full snapshot. Implementors that
    /// need replace-not-merge semantics should clear first.
    fn import(&mut self, root: &Path) -> Result<()> {
        import_stream(root, |record| self.ingest(record))?;
        Ok(())
    }
}
