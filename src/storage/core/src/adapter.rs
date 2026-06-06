//! The `StorageAdapter` trait every backend implements.

use crate::error::Result;
use crate::format::{export_bundle, import_bundle, Manifest, DEFAULT_SHARD_COUNT};
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

/// A data store the rest of the system can read and write through, regardless
/// of where the bytes actually live.
///
/// Two concerns live here:
///
/// 1. **In-store I/O** — `save` / `load` / `delete` / `list`, the per-record
///    operations every backend must support against its native format.
/// 2. **Portability** — `snapshot` / `restore`, the bridge to the single
///    portable bundle format. `export` and `import` are provided on top of
///    these and are the same for every adapter, so any store can round-trip
///    through the on-disk bundle into any other store.
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

    /// Produce the full logical contents as a portable snapshot.
    fn snapshot(&self) -> Result<StoreSnapshot>;

    /// Replace the store's contents with `snapshot`.
    fn restore(&mut self, snapshot: StoreSnapshot) -> Result<()>;

    /// Export the entire store to the one portable bundle format at `root`,
    /// using the default shard count. Implemented once for all adapters.
    fn export(&self, root: &Path) -> Result<Manifest> {
        self.export_with_shards(root, DEFAULT_SHARD_COUNT)
    }

    /// Export with an explicit shard count (parallel-write fan-out width).
    fn export_with_shards(&self, root: &Path, shard_count: u32) -> Result<Manifest> {
        let snapshot = self.snapshot()?;
        export_bundle(&snapshot, root, shard_count)
    }

    /// Import a portable bundle at `root`, replacing this store's contents.
    /// Implemented once for all adapters.
    fn import(&mut self, root: &Path) -> Result<()> {
        let snapshot = import_bundle(root)?;
        self.restore(snapshot)
    }
}
