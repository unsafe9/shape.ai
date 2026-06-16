//! Coordination seam for shape.ai scale-out: a [`Coordinator`] supplies a
//! single-writer lease (TTL handoff), pub/sub fan-out, and presence (TTL expiry).
//! No canvas/scene logic — pure infrastructure the platform layer wires up.
//!
//! Determinism: the lease token is not random. Each coordinator owns a monotonic
//! counter and stamps tokens as `"<owner>-<counter>"`, so replays are
//! reproducible; the trait contract never depends on `rand`.

mod file;
mod inmem;

pub use file::FileCoordinator;
pub use inmem::{Clock, InMemoryCoordinator, SystemClock};

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

pub type Result<T> = anyhow::Result<T>;

/// A TTL in whole milliseconds as the `u64` epoch-ms arithmetic uses. `as_millis`
/// returns `u128`; a TTL past `u64::MAX` ms (~584M years) is impossible, so the
/// saturating narrowing is exact in practice.
pub(crate) fn millis_u64(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// A held single-writer lease on a canvas. `token` disambiguates same-owner
/// leases across a steal: a stale holder gets a different token, so its
/// renew/release is rejected. `expires_at` is wall-clock ms since the Unix epoch
/// — advisory only; the coordinator re-checks expiry against its own clock.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    pub canvas_id: String,
    pub owner: String,
    pub token: String,
    pub expires_at: u64,
}

#[async_trait]
pub trait Coordinator: Send + Sync {
    /// Acquire the single-writer lease for `canvas_id`. Succeeds if free or
    /// expired (the new owner steals it), and is idempotent for the same owner;
    /// fails if a different, still-live owner holds it.
    async fn acquire_lease(&self, canvas_id: &str, owner: &str, ttl: Duration) -> Result<Lease>;

    /// Extend `lease` by `ttl`. Errors if it expired and was stolen.
    async fn renew(&self, lease: &Lease, ttl: Duration) -> Result<()>;

    /// Release `lease`. A no-op (never an error) if already stolen/expired.
    async fn release(&self, lease: Lease) -> Result<()>;

    /// The current live owner of `canvas_id`, or `None` if free/expired.
    async fn find_owner(&self, canvas_id: &str) -> Result<Option<String>>;

    async fn publish(&self, canvas_id: &str, msg: Vec<u8>) -> Result<()>;

    /// Messages published before subscribing are not replayed.
    fn subscribe(&self, canvas_id: &str) -> broadcast::Receiver<Vec<u8>>;

    /// Put `key -> val` on `canvas_id` for `ttl`, overwriting any existing key.
    async fn presence_put(
        &self,
        canvas_id: &str,
        key: &str,
        val: Vec<u8>,
        ttl: Duration,
    ) -> Result<()>;

    /// All non-expired presence entries for `canvas_id` as `(key, val)`.
    async fn presence_get(&self, canvas_id: &str) -> Result<Vec<(String, Vec<u8>)>>;
}
