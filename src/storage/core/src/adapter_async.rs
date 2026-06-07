//! OB1.4 — async, backend-neutral storage surface (D14/D16/D17).
//!
//! The async trait is the target signature for the redb cutover: redb is a sync,
//! single-writer, transactional engine, and a future external DB (FoundationDB /
//! Postgres) is async — both hide behind this one `await` boundary. The
//! sync<->async bridge lives at the call site / wasm worker, **never** in the
//! trait, so swapping the backend is cheap. The legacy sync [`StorageAdapter`]
//! stays in place until the OB4.2 cutover; this is purely additive.
//!
//! [`Record`] stays `{id, kind, version, payload:bytes}` (P5): domain-neutral KV.
//! Region query is a Morton range scan (see [`crate::morton`]), not SQL (D16).
//! Pointer-width-agnostic: codes are `u64`, never `usize` on the wire.

use crate::error::Result;
use crate::record::{Record, StoreSnapshot};
use crate::spatial::RegionKey;

/// A windowed region query in world space. `None` window = whole canvas;
/// otherwise an inclusive AABB `(min, max)` in `f64` world units (matches
/// [`RegionKey`]/`SpatialStore`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RegionWindow {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

impl RegionWindow {
    /// The inclusive Morton code range bounding this window (see
    /// [`crate::morton::morton_range`]). Callers feed it to an ordered range
    /// scan, then refilter with [`RegionWindow::contains_bbox`].
    pub fn morton_range(&self) -> (u64, u64) {
        crate::morton::morton_range(self.min_x, self.min_y, self.max_x, self.max_y)
    }

    /// Exact inclusive-AABB overlap test for the range-scan refilter step.
    pub fn overlaps(&self, key: &RegionKey) -> bool {
        key.min_x <= self.max_x
            && key.max_x >= self.min_x
            && key.min_y <= self.max_y
            && key.max_y >= self.min_y
    }
}

/// Async, backend-neutral KV + region store (D14/D16/D17). Uses RPITIT
/// (`-> impl Future + Send`); the redb impl is a sync leaf wrapped by the bridge.
/// `query_region` returns a `Vec<Record>` (bounded by the window) rather than a
/// boxed stream — object-safe with RPITIT and memory-bounded for windowed reads.
pub trait AsyncStorageAdapter {
    /// Persist (insert/overwrite by id).
    fn save(&self, record: Record) -> impl core::future::Future<Output = Result<()>> + Send;

    /// Load by id.
    fn load(&self, id: &str) -> impl core::future::Future<Output = Result<Record>> + Send;

    /// Delete by id; returns whether a record was removed.
    fn delete(&self, id: &str) -> impl core::future::Future<Output = Result<bool>> + Send;

    /// All ids in deterministic id-sorted order.
    fn list(&self) -> impl core::future::Future<Output = Result<Vec<String>>> + Send;

    /// Full snapshot (small / bundle use; the portable format is unchanged).
    fn snapshot(&self) -> impl core::future::Future<Output = Result<StoreSnapshot>> + Send;

    /// Replace contents with `snapshot`.
    fn restore(
        &self,
        snapshot: StoreSnapshot,
    ) -> impl core::future::Future<Output = Result<()>> + Send;

    /// Upsert a record together with its Morton region key (`None` clears the
    /// region row, leaving the record un-indexed). The adapter owns the Morton
    /// encoding; callers never see SQL.
    fn save_indexed(
        &self,
        record: Record,
        key: Option<RegionKey>,
    ) -> impl core::future::Future<Output = Result<()>> + Send;

    /// Windowed read: records of `canvas_id` whose bbox overlaps `window`
    /// (`None` = whole canvas), id-sorted. Implementations do a Morton range
    /// scan + exact bbox-overlap refilter (OB0.1 verdict plan).
    fn query_region(
        &self,
        canvas_id: &str,
        window: Option<RegionWindow>,
    ) -> impl core::future::Future<Output = Result<Vec<Record>>> + Send;
}
