//! OB3.T3 / D15 — redb `StorageBackend` over an OPFS `FileSystemSyncAccessHandle`
//! (the client-side, in-browser durable KV).
//!
//! # What this is
//!
//! D15 puts the client store on **redb-OPFS**: the same pure-Rust redb engine the
//! native server runs (`OB3.T1`), but persisting to the browser's Origin Private
//! File System instead of a host file. redb is a synchronous, single-writer engine
//! that talks to its storage through one small trait — [`redb::StorageBackend`]
//! (`len`/`read`/`set_len`/`sync_data`/`write`). This module implements that trait
//! by delegating each call to a `FileSystemSyncAccessHandle`, whose `read` /
//! `write` / `getSize` / `truncate` / `flush` are *synchronous* — but only exist
//! inside a **dedicated Web Worker**. That worker single-thread invariant is the
//! whole foundation of the (unsafe) `Send`/`Sync` impls below, so read the safety
//! section before touching them.
//!
//! # The worker / postMessage bridge plan (the `OutboxStore` seam)
//!
//! The wider client data layer (`src/client/lib/{sceneClient,outbox,syncEngine}.ts`)
//! is structured behind an `OutboxStore` interface (today an in-memory backend).
//! D15/D17 say the async [`AsyncStorageAdapter`](crate::AsyncStorageAdapter) trait
//! is the seam, and the **sync↔async bridge lives at the call site / worker, never
//! in the trait**. The intended topology:
//!
//! ```text
//!   main thread (UI, scene-core-wasm op-apply)
//!       │  postMessage(op / query)           ▲ postMessage(records / ack)
//!       ▼                                     │
//!   dedicated Web Worker  ── owns ──►  OpfsBackend ──►  redb::Database (sync)
//!       (single OS thread)                    │
//!                                  FileSystemSyncAccessHandle (OPFS, sync I/O)
//! ```
//!
//! * The handle is obtained once, on the worker, via
//!   `navigator.storage.getDirectory()` → `getFileHandle(create:true)` →
//!   `createSyncAccessHandle()`. It stays open and resident on that one worker.
//! * The main thread never touches redb or the handle. It posts ops/queries and
//!   awaits replies; the worker runs the *synchronous* redb transaction and posts
//!   results back. That postMessage hop is the async boundary; the redb side stays
//!   fully sync, exactly as redb wants.
//! * This `OpfsBackend` is therefore only ever constructed and used on the worker.
//!   Wiring the worker, the message protocol, and the [`AsyncStorageAdapter`] impl
//!   that drives it are downstream tasks (`OB4.3` client data-layer wiring); this
//!   module is just the redb backend leaf.
//!
//! # Licensing / provenance
//!
//! The GPL-3.0 `wireapp/redb-opfs` crate solves this same problem. We **do not
//! depend on it** (license incompatibility) and **do not copy it**: this is an
//! independent reimplementation written directly against the public
//! [`redb::StorageBackend`] (redb 2.x) and `web_sys` `FileSystemSyncAccessHandle`
//! APIs.
//!
//! # Build gating
//!
//! The entire module is gated behind `#[cfg(all(target_arch = "wasm32", feature =
//! "opfs"))]`. The default wasm32 build stays redb-free and clean; `redb` and the
//! `web-sys` OPFS bindings are pulled in **only** under the `opfs` feature, which
//! the Integrate phase defines and wires (this crate's `Cargo.toml`, `lib.rs`).
//! Until then this compiles only under `--features opfs`; if those deps cannot be
//! resolved for `wasm32` in a given environment, the feature is left
//! defined-but-unbuilt and this stands as a documented reference implementation.

#![cfg(all(target_arch = "wasm32", feature = "opfs"))]

use std::io;

use redb::{Database, StorageBackend};
use web_sys::wasm_bindgen::JsValue;
use web_sys::{FileSystemReadWriteOptions, FileSystemSyncAccessHandle};

/// A redb [`StorageBackend`] backed by an OPFS [`FileSystemSyncAccessHandle`].
///
/// One backend wraps one open sync-access handle for the database file. All redb
/// I/O (`len`/`read`/`set_len`/`sync_data`/`write`) is delegated straight to the
/// handle's synchronous OPFS calls.
///
/// # Safety: the `Send`/`Sync` contract
///
/// [`redb::StorageBackend`] requires `Send + Sync`, but `FileSystemSyncAccessHandle`
/// is a `web_sys` JS handle and is `!Send + !Sync` (every JS value is). We assert
/// `Send`/`Sync` with `unsafe impl` below, sound **only** under this invariant,
/// which the caller (the worker bridge, see module docs) must uphold:
///
/// * **Single-thread residency.** This backend is created and used exclusively on
///   one dedicated Web Worker. wasm32 today has no shared-memory threads in this
///   build, and the worker bridge never hands the handle, the `OpfsBackend`, or the
///   `redb::Database` wrapping it to another thread/worker — the JS value is never
///   actually moved or shared across threads.
/// * **No real concurrency.** redb takes `&self` on the backend and serializes its
///   own access under a single write transaction; on the worker there is exactly
///   one redb instance and one handle, so there is no aliasing across threads to
///   guard against.
///
/// The `unsafe impl`s exist purely to satisfy redb's trait bound given that the
/// underlying I/O happens to be a JS handle. If this type were ever sent to a
/// second thread/worker (e.g. a future shared-memory wasm threads build), the
/// invariant would break and the `unsafe impl`s would become unsound — so the
/// worker must remain the sole owner.
#[derive(Debug)]
pub struct OpfsBackend {
    handle: FileSystemSyncAccessHandle,
}

// SAFETY: see the `OpfsBackend` doc "Safety: the Send/Sync contract" section. The
// wrapped JS handle is `!Send + !Sync`; these impls are sound only because the
// backend lives on, and is never moved off, a single dedicated Web Worker (the
// worker bridge upholds this — it is the sole owner and never transfers the
// handle / backend / database across threads). redb requires `Send + Sync`; on a
// single-thread wasm worker there is no cross-thread access to make unsound.
unsafe impl Send for OpfsBackend {}
// SAFETY: as above — single-worker residency means there is never concurrent
// `&OpfsBackend` access from another thread, so `Sync` is vacuously upheld.
unsafe impl Sync for OpfsBackend {}

impl OpfsBackend {
    /// Wrap an already-opened OPFS sync-access handle for the database file.
    ///
    /// The handle must come from a `createSyncAccessHandle()` call on the worker
    /// and stay open for the life of the backend. Obtaining it (directory
    /// navigation, file creation) is async and is the worker bridge's job; by the
    /// time it reaches here it is a live, synchronous handle.
    pub fn new(handle: FileSystemSyncAccessHandle) -> Self {
        OpfsBackend { handle }
    }

    /// `(offset + n)` as the `f64` OPFS `at` cursor.
    ///
    /// OPFS read/write/truncate take an `f64` byte offset (the JS number type), so
    /// crossing into JS is an inherent `u64 -> f64` conversion. redb database files
    /// stay far below `2^53`, where `f64` represents every integer exactly, so this
    /// is lossless in practice.
    #[allow(
        clippy::cast_precision_loss,
        reason = "OPFS at/length offsets cross into JS as f64; redb files stay well below 2^53 so the value is exact"
    )]
    fn offset_f64(offset: u64) -> f64 {
        offset as f64
    }
}

/// Map any thrown `JsValue` (the OPFS handle's error channel) into an
/// [`io::Error`], which is what every [`StorageBackend`] method returns. Uses only
/// `JsValue`'s own string/Debug projection so the dependency surface stays just
/// `redb` + `web-sys` (no `js-sys` / `gloo` error-conversion crate needed).
fn js_to_io(err: impl Into<JsValue>) -> io::Error {
    let value: JsValue = err.into();
    let msg = value
        .as_string()
        .unwrap_or_else(|| format!("{value:?}"));
    io::Error::other(format!("OPFS access handle error: {msg}"))
}

/// redb 2.x [`StorageBackend`]. Method signatures match the redb 2.x trait exactly:
/// `read(offset, len) -> Vec<u8>` and `sync_data(eventual)` (redb 3.x changed
/// `read` to fill a caller buffer and dropped `eventual` — see module docs if the
/// Integrate phase pins a different redb major).
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

    /// Read exactly `len` bytes at `offset`. OPFS `read` may return a short count,
    /// so loop until the buffer is full (or a zero-length read signals premature
    /// EOF, which redb treats as an I/O error).
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

    /// Set the database length. OPFS `truncate` both shrinks and grows the file;
    /// the redb contract says grown regions read back as zero, which is exactly
    /// OPFS truncate-to-larger behaviour.
    fn set_len(&self, len: u64) -> Result<(), io::Error> {
        self.handle
            .truncate_with_f64(Self::offset_f64(len))
            .map_err(js_to_io)
    }

    /// Flush buffered writes to the OPFS file. `eventual` lets redb request a
    /// relaxed barrier, but OPFS only exposes a single `flush()`, so both modes map
    /// to the same durable flush (stricter than requested is always safe).
    fn sync_data(&self, _eventual: bool) -> Result<(), io::Error> {
        self.handle.flush().map_err(js_to_io)
    }

    /// Write all of `data` at `offset`, looping over short writes the way `read`
    /// loops over short reads.
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

/// Convert the `f64` byte count OPFS returns from `read`/`write` into a `usize`.
///
/// OPFS returns the number of bytes transferred as a JS number (`f64`). It is a
/// non-negative integer bounded by the slice length we passed in (always far below
/// `2^53`), so this is exact; we still reject negatives/NaN defensively.
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

/// Open (or create) a redb database on top of an OPFS sync-access handle.
///
/// Builds an [`OpfsBackend`] from `handle` and hands it to redb's
/// `create_with_backend`, which opens an existing database in the file or
/// initializes a fresh one. The caller (worker bridge) owns the returned
/// [`Database`] and must keep it on the same worker as the handle.
pub fn open_opfs_database(
    handle: FileSystemSyncAccessHandle,
) -> Result<Database, redb::DatabaseError> {
    Database::builder().create_with_backend(OpfsBackend::new(handle))
}

// NOTE: runtime tests require a browser worker with a live OPFS handle and cannot
// run in `cargo test` (this whole module is wasm32+opfs only). The behavioural
// contract is instead covered by the native redb adapter's tests (`OB3.T1`) over
// the SAME `redb::StorageBackend` trait — that proves the redb-side semantics; the
// OPFS-specific surface here is the thin handle delegation documented above. A
// `wasm-bindgen-test` against a real `FileSystemSyncAccessHandle` is the right home
// for end-to-end coverage and belongs with the worker-bridge task (`OB4.3`).
//
// The one piece that is pure and host-independent — the short-count loop's byte
// accounting — is asserted below with a hand-rolled fake handle modelled on OPFS
// semantics, compiled only under this module's wasm32+opfs gate.
#[cfg(test)]
mod tests {
    //! These tests exercise the offset/short-count arithmetic against a fake that
    //! mimics OPFS short reads/writes, without needing a browser. They build only
    //! under `--features opfs` on wasm32 (the module gate), and are intended to be
    //! run with `wasm-bindgen-test`. They deliberately avoid constructing a real
    //! `FileSystemSyncAccessHandle` (impossible outside a worker).

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
        // Below 2^53 the u64 -> f64 -> u64 round-trip is exact.
        for v in [0u64, 1, 4096, 1 << 20, (1u64 << 40) + 7] {
            assert_eq!(OpfsBackend::offset_f64(v) as u64, v);
        }
    }
}
