//! Coordination seam for shape.ai scale-out (MG8.1, MG8.6).
//!
//! A [`Coordinator`] gives the server three things it needs once more than one
//! process can serve the same canvas: a **single-writer lease** (only one owner
//! may hold a canvas at a time, with TTL-based handoff), **pub/sub** for
//! cross-instance fan-out, and **presence** (who is on a canvas, with TTL
//! expiry). No canvas/scene logic lives here — this is pure infrastructure that
//! the platform layer wires up.
//!
//! Two implementations ship:
//! - [`InMemoryCoordinator`] — the default; single-process dev. State lives in a
//!   tokio mutex; pub/sub uses per-canvas broadcast channels.
//! - [`FileCoordinator`] — single-host multi-process dev (MG8.6). The lease is a
//!   lock file guarded by an OS advisory lock so lease ownership stays correct
//!   across processes; presence and pub/sub are best-effort over the same dir.
//!
//! Determinism: the lease token is **not** random. Each coordinator owns a
//! monotonic counter and stamps tokens as `"<owner>-<counter>"`, so tests and
//! replays are reproducible. The trait contract never depends on `rand`.

mod file;
mod inmem;

pub use file::FileCoordinator;
pub use inmem::{Clock, InMemoryCoordinator, SystemClock};

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// Result alias for coordinator operations.
pub type Result<T> = anyhow::Result<T>;

/// A held single-writer lease on a canvas.
///
/// A lease is granted by [`Coordinator::acquire_lease`] and is the proof of
/// ownership the caller passes back to [`Coordinator::renew`] /
/// [`Coordinator::release`]. `token` disambiguates two leases by the same owner
/// across a steal: a stale holder whose lease expired and was stolen will have a
/// different `token` than the current holder, so its renew/release is rejected.
///
/// `expires_at` is the wall-clock deadline as milliseconds since the Unix epoch,
/// computed at grant/renew time from the caller-supplied TTL. It is advisory
/// metadata for the holder; the coordinator re-checks expiry against its own
/// clock on every operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    pub canvas_id: String,
    pub owner: String,
    pub token: String,
    pub expires_at: u64,
}

/// Coordination primitives for canvas scale-out.
///
/// All methods are async and `Send`/`Sync`-safe so a single coordinator can be
/// shared (e.g. behind an `Arc`) across the server's connection and actor tasks.
#[async_trait]
pub trait Coordinator: Send + Sync {
    /// Try to acquire the single-writer lease for `canvas_id` on behalf of
    /// `owner`, valid for `ttl`.
    ///
    /// Succeeds if the canvas is free or if the current lease has expired (the
    /// new owner *steals* it). Fails if a different, still-live owner holds it.
    /// Re-acquiring as the *same* owner refreshes the lease (idempotent).
    async fn acquire_lease(&self, canvas_id: &str, owner: &str, ttl: Duration) -> Result<Lease>;

    /// Extend `lease` by `ttl` from now. Errors if the lease is no longer held
    /// by this owner+token (i.e. it expired and was stolen).
    async fn renew(&self, lease: &Lease, ttl: Duration) -> Result<()>;

    /// Release `lease`, freeing the canvas. A no-op if the lease was already
    /// stolen/expired; never errors on a stale release.
    async fn release(&self, lease: Lease) -> Result<()>;

    /// The current live owner of `canvas_id`, or `None` if free/expired.
    async fn find_owner(&self, canvas_id: &str) -> Result<Option<String>>;

    /// Publish an opaque message to all subscribers of `canvas_id`.
    async fn publish(&self, canvas_id: &str, msg: Vec<u8>) -> Result<()>;

    /// Subscribe to messages published on `canvas_id`. Returns a broadcast
    /// receiver; messages published before subscribing are not replayed.
    fn subscribe(&self, canvas_id: &str) -> broadcast::Receiver<Vec<u8>>;

    /// Put a presence entry `key -> val` on `canvas_id`, valid for `ttl`.
    /// Overwrites any existing entry for the same key.
    async fn presence_put(
        &self,
        canvas_id: &str,
        key: &str,
        val: Vec<u8>,
        ttl: Duration,
    ) -> Result<()>;

    /// Get all non-expired presence entries for `canvas_id` as `(key, val)`.
    async fn presence_get(&self, canvas_id: &str) -> Result<Vec<(String, Vec<u8>)>>;
}
