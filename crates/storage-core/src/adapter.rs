use crate::error::Result;
#[cfg(not(target_arch = "wasm32"))]
use crate::format::{export_stream, import_stream, Manifest, DEFAULT_SHARD_COUNT};
use crate::record::{Record, StoreSnapshot};
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

/// The named store kinds the adapter layer abstracts over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdapterKind {
    Memory,
    File,
    /// Embedded redb-on-file store (native-only).
    Redb,
    /// Stub: needs an external postgres backend.
    Postgres,
    /// Stub: needs an external S3 backend.
    S3,
    /// Stub: needs an external remote storage server.
    RemoteServer,
}

impl AdapterKind {
    /// Stable lowercase name.
    pub fn as_str(self) -> &'static str {
        match self {
            AdapterKind::Memory => "memory",
            AdapterKind::File => "file",
            AdapterKind::Redb => "redb",
            AdapterKind::Postgres => "postgres",
            AdapterKind::S3 => "s3",
            AdapterKind::RemoteServer => "remote-server",
        }
    }
}

/// A lazy cursor yielding a store's records in **id-sorted order**, one at a
/// time without materializing them all. Returned by [`StorageAdapter::records`]
/// and consumed by the streaming export (its id order keeps bundles byte-stable).
pub type RecordCursor<'a> = Box<dyn Iterator<Item = Result<Record>> + 'a>;

/// A data store readable/writable regardless of where the bytes live.
///
/// `records`/`ingest` are the streaming pair that keeps `export`/`import`
/// memory-safe for large stores (neither holds the whole store in RAM);
/// `snapshot`/`restore` are the small/in-memory portability bridge.
pub trait StorageAdapter {
    fn kind(&self) -> AdapterKind;

    /// Persist a record, inserting or overwriting by id.
    fn save(&mut self, record: Record) -> Result<()>;

    fn load(&self, id: &str) -> Result<Record>;

    /// Delete a record by id. Returns whether a record was removed.
    fn delete(&mut self, id: &str) -> Result<bool>;

    /// List all record ids currently held, in deterministic order.
    fn list(&self) -> Result<Vec<String>>;

    /// A lazy cursor over all records in **id-sorted order** (the memory-safe
    /// read path for export; implementations yield one record at a time).
    fn records(&self) -> Result<RecordCursor<'_>>;

    /// Streaming write sink for import: ingest one record (insert/overwrite by
    /// id) without holding the whole store. Override when ingestion can be made
    /// cheaper than a full per-record persist.
    fn ingest(&mut self, record: Record) -> Result<()> {
        self.save(record)
    }

    /// Full logical contents as a portable snapshot (small/in-memory use;
    /// **not** on the streaming export path).
    fn snapshot(&self) -> Result<StoreSnapshot>;

    /// Replace the store's contents with `snapshot` (small/in-memory use;
    /// **not** on the streaming import path).
    fn restore(&mut self, snapshot: StoreSnapshot) -> Result<()>;

    /// Export the store to the portable bundle format at `root`, default shards.
    /// Native-only: the bundle format depends on `std::fs` + rayon.
    #[cfg(not(target_arch = "wasm32"))]
    fn export(&self, root: &Path) -> Result<Manifest> {
        self.export_with_shards(root, DEFAULT_SHARD_COUNT)
    }

    /// Export with an explicit shard count, streaming the [`records`] cursor in
    /// bounded memory (never a full snapshot).
    ///
    /// [`records`]: StorageAdapter::records
    #[cfg(not(target_arch = "wasm32"))]
    fn export_with_shards(&self, root: &Path, shard_count: u32) -> Result<Manifest> {
        export_stream(self.records()?, root, shard_count)
    }

    /// Import a portable bundle at `root`, merging its records into this store
    /// shard-by-shard via [`ingest`] in bounded memory. Implementors needing
    /// replace-not-merge semantics should clear first.
    ///
    /// [`ingest`]: StorageAdapter::ingest
    #[cfg(not(target_arch = "wasm32"))]
    fn import(&mut self, root: &Path) -> Result<()> {
        import_stream(root, |record| self.ingest(record))?;
        Ok(())
    }
}
