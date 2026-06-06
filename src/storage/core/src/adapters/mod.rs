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

mod file;
mod memory;
mod stubs;

pub use file::FileAdapter;
pub use memory::MemoryAdapter;
pub use stubs::{PostgresAdapter, RemoteServerAdapter, S3Adapter, SqliteAdapter};
