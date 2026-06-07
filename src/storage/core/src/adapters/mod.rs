//! Concrete [`StorageAdapter`](crate::StorageAdapter) implementations.
//!
//! This directory holds every real adapter in one place. The trait itself
//! lives in [`crate::adapter`]; the portable bundle format in [`crate::format`].
//!
//! * [`MemoryAdapter`] — in-process map store (the in-RAM backend).
//! * [`FileAdapter`] — on-disk bundle store that streams record-by-record from
//!   its sharded representation, so export/import/save stay memory-bounded.
//! * sqlite / postgres / s3 / remote-server [`stubs`] — real adapter *shapes*
//!   that keep the portability contract until their drivers are wired in.
//!
//! See [`CLAUDE.md`](./CLAUDE.md) for the memory-discipline rules, the integrity
//! test contract, and how to add a new adapter.

mod memory;
// FileAdapter and the driver stubs lean on the std::fs + rayon bundle format,
// so they are native-only. wasm32 keeps just the in-memory adapter.
#[cfg(not(target_arch = "wasm32"))]
mod file;
// The redb adapter is native-only (redb + zstd are native-only optional deps)
// and behind the `redb` feature. The file is named `redb_store` to avoid a
// name collision with the `redb` crate at module-path resolution.
#[cfg(all(not(target_arch = "wasm32"), feature = "redb"))]
mod redb_store;
#[cfg(not(target_arch = "wasm32"))]
mod stubs;

pub use memory::MemoryAdapter;
#[cfg(not(target_arch = "wasm32"))]
pub use file::FileAdapter;
#[cfg(all(not(target_arch = "wasm32"), feature = "redb"))]
pub use redb_store::RedbAdapter;
#[cfg(not(target_arch = "wasm32"))]
pub use stubs::{PostgresAdapter, RemoteServerAdapter, S3Adapter};
