//! Async, backend-neutral storage surface.
//!
//! Both a sync engine (redb) and a future async DB hide behind this one `await`
//! boundary. The sync↔async bridge lives at the call site / wasm worker,
//! **never** in the trait, so swapping the backend stays cheap. Region query is
//! a Morton range scan (see [`crate::morton`]), not SQL; codes are `u64`, never
//! `usize`, on the wire.

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
    /// The inclusive Morton code range bounding this window. Callers feed it to
    /// an ordered range scan, then refilter with [`RegionWindow::overlaps`].
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

/// Async, backend-neutral KV + region store. `query_region` returns a window-
/// bounded `Vec<Record>` (object-safe with RPITIT, memory-bounded).
pub trait AsyncStorageAdapter {
    /// Persist (insert/overwrite by id).
    fn save(&self, record: Record) -> impl core::future::Future<Output = Result<()>> + Send;

    fn load(&self, id: &str) -> impl core::future::Future<Output = Result<Record>> + Send;

    /// Delete by id; returns whether a record was removed.
    fn delete(&self, id: &str) -> impl core::future::Future<Output = Result<bool>> + Send;

    /// All ids in deterministic id-sorted order.
    fn list(&self) -> impl core::future::Future<Output = Result<Vec<String>>> + Send;

    fn snapshot(&self) -> impl core::future::Future<Output = Result<StoreSnapshot>> + Send;

    fn restore(
        &self,
        snapshot: StoreSnapshot,
    ) -> impl core::future::Future<Output = Result<()>> + Send;

    /// Upsert a record with its Morton region key (`None` clears the region row,
    /// leaving the record un-indexed). The adapter owns the Morton encoding.
    fn save_indexed(
        &self,
        record: Record,
        key: Option<RegionKey>,
    ) -> impl core::future::Future<Output = Result<()>> + Send;

    /// Windowed read: records of `canvas_id` whose bbox overlaps `window`
    /// (`None` = whole canvas), id-sorted.
    fn query_region(
        &self,
        canvas_id: &str,
        window: Option<RegionWindow>,
    ) -> impl core::future::Future<Output = Result<Vec<Record>>> + Send;
}
