//! redb [`redb::StorageBackend`] over an OPFS `FileSystemSyncAccessHandle` — the
//! client-side, in-browser durable KV.
//!
//! redb is sync and talks to storage through one small trait
//! (`len`/`read`/`set_len`/`sync_data`/`write`); this delegates each call to a
//! `FileSystemSyncAccessHandle` whose I/O is synchronous but exists ONLY inside a
//! **dedicated Web Worker**. That single-worker invariant is the whole
//! foundation of the unsafe `Send`/`Sync` impls below — read the safety section
//! before touching them. The main thread posts ops/queries and awaits replies;
//! the worker runs the sync redb transaction. So this backend is only ever
//! constructed and used on the worker.
//!
//! Licensing: this is an independent reimplementation against the public redb 2.x
//! `StorageBackend` + web_sys APIs — it neither depends on nor copies the GPL-3.0
//! `wireapp/redb-opfs` crate (license incompatibility).
//!
//! Build-gated behind `#[cfg(all(target_arch = "wasm32", feature = "opfs"))]`, so
//! the default wasm32 build stays redb-free; `redb`/`web-sys` OPFS bindings come
//! in only under the `opfs` feature.

#![cfg(all(target_arch = "wasm32", feature = "opfs"))]

use std::io;

// Keyed `redb_wasm` in Cargo.toml (renamed via `package`) so the native `redb`
// feature can't pull it into the default wasm build; aliased back to `redb` here.
use redb_wasm as redb;
use redb::{Database, StorageBackend};
use web_sys::wasm_bindgen::JsValue;
use web_sys::{FileSystemReadWriteOptions, FileSystemSyncAccessHandle};

/// A redb [`StorageBackend`] wrapping one open OPFS sync-access handle; all redb
/// I/O delegates to the handle's synchronous OPFS calls.
///
/// # Safety: the `Send`/`Sync` contract
///
/// `FileSystemSyncAccessHandle` is a `!Send + !Sync` JS handle, but
/// [`redb::StorageBackend`] requires `Send + Sync`. The unsafe impls below are
/// sound ONLY because this backend lives on, and is never moved off, a single
/// dedicated Web Worker (the sole owner; it never hands the handle / backend /
/// database to another thread). If ever sent to a second thread (e.g. a future
/// shared-memory wasm-threads build), the invariant breaks and the impls become
/// unsound.
#[derive(Debug)]
pub struct OpfsBackend {
    handle: FileSystemSyncAccessHandle,
}

// SAFETY: sound only under the single-worker residency invariant; see the
// `OpfsBackend` "Safety" doc section.
unsafe impl Send for OpfsBackend {}
// SAFETY: single-worker residency means no concurrent cross-thread `&self`
// access, so `Sync` is vacuously upheld.
unsafe impl Sync for OpfsBackend {}

impl OpfsBackend {
    /// Wrap an already-open OPFS sync-access handle (from a worker-side
    /// `createSyncAccessHandle()`) that stays open for the backend's life.
    pub fn new(handle: FileSystemSyncAccessHandle) -> Self {
        OpfsBackend { handle }
    }

    /// `offset` as the `f64` OPFS `at` cursor. OPFS takes an `f64` byte offset;
    /// redb files stay far below `2^53`, so the `u64 -> f64` cast is exact.
    #[allow(
        clippy::cast_precision_loss,
        reason = "OPFS at/length offsets cross into JS as f64; redb files stay well below 2^53 so the value is exact"
    )]
    fn offset_f64(offset: u64) -> f64 {
        offset as f64
    }
}

/// Map a thrown `JsValue` into the [`io::Error`] every [`StorageBackend`] method
/// returns, using only `JsValue`'s own string/Debug projection (no js-sys/gloo).
fn js_to_io(err: impl Into<JsValue>) -> io::Error {
    let value: JsValue = err.into();
    let msg = value
        .as_string()
        .unwrap_or_else(|| format!("{value:?}"));
    io::Error::other(format!("OPFS access handle error: {msg}"))
}

/// Signatures match the redb 2.x trait exactly (`read -> Vec<u8>`,
/// `sync_data(eventual)`); redb 3.x fills a caller buffer and drops `eventual`.
impl StorageBackend for OpfsBackend {
    /// Current database length = the OPFS file size.
    #[allow(
        clippy::cast_sign_loss,
        clippy::cast_possible_truncation,
        reason = "getSize() returns an f64 byte count that is a non-negative exact integer below 2^53"
    )]
    fn len(&self) -> Result<u64, io::Error> {
        let size = self.handle.get_size().map_err(js_to_io)?;
        Ok(size as u64)
    }

    /// Read exactly `len` bytes at `offset`, looping over OPFS short reads (a
    /// zero-length read is premature EOF, an I/O error to redb).
    fn read(&self, offset: u64, len: usize) -> Result<Vec<u8>, io::Error> {
        let mut buf = vec![0u8; len];
        let mut filled: usize = 0;
        while filled < len {
            let options = FileSystemReadWriteOptions::new();
            options.set_at(Self::offset_f64(offset + filled as u64));
            let n = self
                .handle
                .read_with_u8_array_and_options(&mut buf[filled..], &options)
                .map_err(js_to_io)?;
            let n = read_count(n)?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!("OPFS read returned 0 bytes at offset {} ({filled}/{len})", offset),
                ));
            }
            filled += n;
        }
        Ok(buf)
    }

    /// Set the database length via OPFS `truncate`; grown regions read back as
    /// zero, matching the redb contract.
    fn set_len(&self, len: u64) -> Result<(), io::Error> {
        self.handle
            .truncate_with_f64(Self::offset_f64(len))
            .map_err(js_to_io)
    }

    /// Flush buffered writes. OPFS exposes a single `flush()`, so `eventual`'s
    /// relaxed barrier maps to the same durable flush (stricter is always safe).
    fn sync_data(&self, _eventual: bool) -> Result<(), io::Error> {
        self.handle.flush().map_err(js_to_io)
    }

    /// Write all of `data` at `offset`, looping over OPFS short writes.
    fn write(&self, offset: u64, data: &[u8]) -> Result<(), io::Error> {
        let mut written: usize = 0;
        while written < data.len() {
            let options = FileSystemReadWriteOptions::new();
            options.set_at(Self::offset_f64(offset + written as u64));
            let n = self
                .handle
                .write_with_u8_array_and_options(&data[written..], &options)
                .map_err(js_to_io)?;
            let n = read_count(n)?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    format!(
                        "OPFS write made no progress at offset {} ({written}/{} bytes)",
                        offset,
                        data.len()
                    ),
                ));
            }
            written += n;
        }
        Ok(())
    }
}

/// Convert OPFS's `f64` transferred-byte count into a `usize`. It is a
/// non-negative integer bounded by the slice length (far below `2^53`, so exact);
/// negatives/NaN are still rejected defensively.
#[allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    reason = "OPFS read/write return a non-negative exact integer byte count bounded by the slice length"
)]
fn read_count(n: f64) -> Result<usize, io::Error> {
    if !n.is_finite() || n < 0.0 {
        return Err(io::Error::other(format!(
            "OPFS reported an invalid transferred byte count: {n}"
        )));
    }
    Ok(n as usize)
}

/// Open (or create) a redb database on top of an OPFS sync-access handle. The
/// caller owns the returned [`Database`] and must keep it on the handle's worker.
pub fn open_opfs_database(
    handle: FileSystemSyncAccessHandle,
) -> Result<Database, redb::DatabaseError> {
    Database::builder().create_with_backend(OpfsBackend::new(handle))
}

// Runtime tests need a browser worker with a live OPFS handle and can't run in
// `cargo test`; the redb-side semantics are covered by the native redb adapter
// over the same `StorageBackend` trait. Only the pure short-count byte
// accounting is asserted below.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_count_rejects_invalid_and_accepts_exact() {
        assert!(read_count(-1.0).is_err());
        assert!(read_count(f64::NAN).is_err());
        assert!(read_count(f64::INFINITY).is_err());
        assert_eq!(read_count(0.0).unwrap(), 0);
        assert_eq!(read_count(4096.0).unwrap(), 4096);
    }

    #[test]
    #[allow(
        clippy::cast_sign_loss,
        clippy::cast_possible_truncation,
        reason = "test asserts the f64 -> u64 round-trip is exact for the small offsets used"
    )]
    fn offset_f64_round_trips_small_values() {
        for v in [0u64, 1, 4096, 1 << 20, (1u64 << 40) + 7] {
            assert_eq!(OpfsBackend::offset_f64(v) as u64, v);
        }
    }
}
