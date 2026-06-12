//! Concrete [`StorageAdapter`](crate::StorageAdapter) implementations. The trait
//! lives in [`crate::adapter`]; the portable bundle format in [`crate::format`].

mod memory;
// FileAdapter + driver stubs lean on the std::fs + rayon bundle format, so
// they are native-only; wasm32 keeps just the in-memory adapter.
#[cfg(not(target_arch = "wasm32"))]
mod file;
// Named `redb_store` to avoid colliding with the `redb` crate at module-path
// resolution. Native-only + behind the `redb` feature.
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
