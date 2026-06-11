//! Renderer cache + streaming groundwork for the scene core.
//!
//! T3.2: an effectively unbounded scene cannot re-tessellate every object every
//! frame, nor load every object at once. This module adds two additive, core-owned
//! structures that the draw path layers on the existing seams (`VertexRanges`
//! slots, the `dirty_*_ids` invalidation set, `apply_patch_batch` upserts):
//!
//! 1. [`RenderDataCache`] — caches *derived* per-object render data (tessellated
//!    vertices, route geometry, etc.) keyed by object id **and** a monotonic
//!    revision. An object whose revision is unchanged is a cache hit and skips
//!    re-tessellation; bumping its revision (or evicting it) forces a re-derive.
//!    Invalidation reuses the same `dirty_*_ids` set the patch path already
//!    collects, so no new dirtiness tracking is introduced. The cache is bounded
//!    by an entry cap with LRU eviction, mirroring [`crate::text::TextLayoutCache`]
//!    so the working set stays the visible + prefetch set, never the whole scene.
//!
//! 2. [`stream_chunks`] — splits an object set into fixed-size chunks so the
//!    snapshot/merge path processes objects incrementally rather than all-at-once.
//!
//! Pure data/geometry: host-neutral, no `JsValue`, no business fields. It is
//! ephemeral derived state, never persisted (Locked Decision: caches/index/budgets
//! are core-owned and ephemeral).
//!
//! These structures are groundwork: the draw path (`build_draw_list`) wires them
//! in downstream, so the module is `allow(dead_code)` until then.

#![allow(dead_code)]

use std::collections::HashMap;

/// Default cap on cached render-data entries. Seeded so the resident working set
/// is bounded by visible + prefetch objects for realistic viewports rather than
/// the whole (unbounded) scene; tunable by T3.4 against memory/latency evidence.
pub const RENDER_DATA_CACHE_LIMIT: usize = 8192;

/// One object's cached derived render data, tagged with the revision it was
/// derived at and the frame it was last touched (for LRU eviction).
#[derive(Clone, Debug)]
struct CachedRenderEntry<T> {
    revision: u64,
    last_used: u64,
    data: T,
}

/// Outcome of a [`RenderDataCache::get_or_insert`] lookup, so callers and tests
/// can distinguish a hit (revision matched, re-tessellation skipped) from a miss
/// (absent or stale revision, data re-derived).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CacheOutcome {
    /// Entry was present at the requested revision; cached data reused.
    Hit,
    /// Entry was absent; data derived and inserted.
    Miss,
    /// Entry was present but at a stale revision; data re-derived and replaced.
    Stale,
}

/// A revision-keyed LRU cache of derived per-object render data.
///
/// `T` is whatever the draw path derives per object — for the renderer that is
/// tessellated vertex data / cached route geometry. The cache is generic so the
/// same residency + invalidation policy serves cards, edges, and density tiles
/// without duplicating the bookkeeping.
#[derive(Clone, Debug)]
pub struct RenderDataCache<T> {
    entries: HashMap<String, CachedRenderEntry<T>>,
    capacity: usize,
    frame: u64,
    pub hits: usize,
    pub misses: usize,
    pub evictions: usize,
}

impl<T: Clone> RenderDataCache<T> {
    pub fn new() -> Self {
        Self::with_capacity(RENDER_DATA_CACHE_LIMIT)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        RenderDataCache {
            entries: HashMap::new(),
            capacity: capacity.max(1),
            frame: 0,
            hits: 0,
            misses: 0,
            evictions: 0,
        }
    }

    /// Advance the logical frame clock. The draw path calls this once per frame so
    /// `last_used` reflects the frame an entry was actually requested, giving LRU
    /// eviction a meaningful recency order. Returns the new frame number.
    pub fn begin_frame(&mut self) -> u64 {
        self.frame += 1;
        self.frame
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Look up `id` at `revision`, deriving and caching the data on a miss or a
    /// stale revision. `derive` is invoked only when re-tessellation is actually
    /// needed, so an unchanged object never re-tessellates.
    pub fn get_or_insert<F>(&mut self, id: &str, revision: u64, derive: F) -> &T
    where
        F: FnOnce() -> T,
    {
        let frame = self.frame;
        let outcome = match self.entries.get(id) {
            Some(entry) if entry.revision == revision => CacheOutcome::Hit,
            Some(_) => CacheOutcome::Stale,
            None => CacheOutcome::Miss,
        };
        match outcome {
            CacheOutcome::Hit => {
                self.hits += 1;
                let entry = self.entries.get_mut(id).expect("entry present on hit");
                entry.last_used = frame;
            }
            CacheOutcome::Miss | CacheOutcome::Stale => {
                self.misses += 1;
                let data = derive();
                self.entries.insert(
                    id.to_string(),
                    CachedRenderEntry {
                        revision,
                        last_used: frame,
                        data,
                    },
                );
                self.evict_if_needed(id);
            }
        }
        &self.entries.get(id).expect("entry present after insert").data
    }

    /// The revision an entry is currently cached at, or `None` if absent. Lets the
    /// draw path cheaply test whether a re-derive is needed before borrowing.
    pub fn cached_revision(&self, id: &str) -> Option<u64> {
        self.entries.get(id).map(|entry| entry.revision)
    }

    /// Drop the cached entry for `id`. Used by the dirty-id invalidation path so a
    /// moved/edited object is re-derived on its next request even at the same
    /// revision number.
    pub fn invalidate(&mut self, id: &str) {
        self.entries.remove(id);
    }

    /// Invalidate every id in `dirty_ids` — the exact set the patch path already
    /// collects (`dirty_card_ids` / `dirty_edge_ids` / `dirty_group_ids`), so cache
    /// invalidation reuses existing dirtiness tracking instead of inventing its own.
    pub fn invalidate_all<I, S>(&mut self, dirty_ids: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for id in dirty_ids {
            self.entries.remove(id.as_ref());
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Evict the least-recently-used entry while over capacity, never evicting
    /// `protect` (the entry just inserted this frame). Counts each eviction so
    /// budget pressure is observable in frame stats.
    fn evict_if_needed(&mut self, protect: &str) {
        while self.entries.len() > self.capacity {
            let victim = self
                .entries
                .iter()
                .filter(|(id, _)| id.as_str() != protect)
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(id, _)| id.clone());
            match victim {
                Some(id) => {
                    self.entries.remove(&id);
                    self.evictions += 1;
                }
                None => break,
            }
        }
    }
}

impl<T: Clone> Default for RenderDataCache<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// Default streaming chunk size: how many objects the snapshot/merge path
/// processes per step instead of all-at-once. Tunable by T3.4.
pub const STREAM_CHUNK_SIZE: usize = 512;

/// Split `items` into fixed-size chunks of at most `chunk_size`, so an unbounded
/// object set is streamed/processed incrementally rather than loaded in one pass.
/// A `chunk_size` of `0` is clamped to `1`. The final chunk may be shorter.
pub fn stream_chunks<T>(items: &[T], chunk_size: usize) -> impl Iterator<Item = &[T]> {
    items.chunks(chunk_size.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A second request at the same revision is a hit and never re-derives.
    #[test]
    fn unchanged_revision_is_cache_hit_and_skips_derive() {
        let mut cache: RenderDataCache<u32> = RenderDataCache::new();
        let mut derive_calls = 0;

        cache.begin_frame();
        let outcome = cache.get_or_insert("card-1", 1, || {
            derive_calls += 1;
            10
        });
        assert_eq!(*outcome, 10);

        cache.begin_frame();
        let outcome = cache.get_or_insert("card-1", 1, || {
            derive_calls += 1;
            99 // would be returned only if a re-derive happened
        });
        assert_eq!(*outcome, 10, "hit must return cached data, not re-derive");

        assert_eq!(derive_calls, 1, "derive runs once for a stable revision");
        assert_eq!(cache.hits, 1);
        assert_eq!(cache.misses, 1);
    }

    /// Bumping the revision re-derives (stale), and an absent id is a miss.
    #[test]
    fn bumped_revision_invalidates_and_re_derives() {
        let mut cache: RenderDataCache<u32> = RenderDataCache::new();
        let mut derive_calls = 0;

        cache.begin_frame();
        cache.get_or_insert("card-1", 1, || {
            derive_calls += 1;
            10
        });

        cache.begin_frame();
        let outcome = cache.get_or_insert("card-1", 2, || {
            derive_calls += 1;
            20
        });
        assert_eq!(*outcome, 20, "stale revision must re-derive");

        assert_eq!(derive_calls, 2);
        assert_eq!(cache.hits, 0);
        assert_eq!(cache.misses, 2);
        assert_eq!(cache.cached_revision("card-1"), Some(2));
        assert_eq!(cache.cached_revision("card-missing"), None);
    }

    /// Explicit dirty-id invalidation forces a re-derive even at the same revision.
    #[test]
    fn invalidate_dirty_ids_forces_re_derive_at_same_revision() {
        let mut cache: RenderDataCache<u32> = RenderDataCache::new();
        let mut derive_calls = 0;

        cache.begin_frame();
        cache.get_or_insert("edge-1", 1, || {
            derive_calls += 1;
            7
        });

        // The patch path collected this edge as dirty (e.g. an endpoint moved)
        // without changing its revision counter.
        cache.invalidate_all(["edge-1", "edge-absent"]);
        assert_eq!(cache.cached_revision("edge-1"), None);

        cache.begin_frame();
        cache.get_or_insert("edge-1", 1, || {
            derive_calls += 1;
            8
        });

        assert_eq!(derive_calls, 2, "invalidated entry re-derives at same revision");
    }

    /// Over capacity, the least-recently-used entry is evicted and counted; the
    /// entry just inserted this frame is never the victim.
    #[test]
    fn lru_eviction_drops_least_recently_used_over_capacity() {
        let mut cache: RenderDataCache<u32> = RenderDataCache::with_capacity(2);

        cache.begin_frame();
        cache.get_or_insert("a", 1, || 1);

        cache.begin_frame();
        cache.get_or_insert("b", 1, || 2);

        // Touch "a" so "b" becomes the least-recently-used entry.
        cache.begin_frame();
        cache.get_or_insert("a", 1, || 1);

        // Insert "c": over capacity, "b" (LRU) is evicted, "a" and "c" survive.
        cache.begin_frame();
        cache.get_or_insert("c", 1, || 3);

        assert_eq!(cache.len(), 2);
        assert_eq!(cache.evictions, 1);
        assert_eq!(cache.cached_revision("b"), None, "LRU victim evicted");
        assert_eq!(cache.cached_revision("a"), Some(1));
        assert_eq!(cache.cached_revision("c"), Some(1));
    }

    /// Clearing drops all residency (e.g. on a full `load_scene` replace).
    #[test]
    fn clear_drops_all_entries() {
        let mut cache: RenderDataCache<u32> = RenderDataCache::new();
        cache.begin_frame();
        cache.get_or_insert("a", 1, || 1);
        cache.get_or_insert("b", 1, || 2);
        assert_eq!(cache.len(), 2);
        cache.clear();
        assert!(cache.is_empty());
    }

    /// Chunked streaming covers every item exactly once with bounded chunk size.
    #[test]
    fn stream_chunks_splits_into_bounded_groups() {
        let items: Vec<u32> = (0..10).collect();
        let chunks: Vec<&[u32]> = stream_chunks(&items, 3).collect();
        assert_eq!(chunks.len(), 4);
        assert_eq!(chunks[0], &[0, 1, 2]);
        assert_eq!(chunks[3], &[9], "final chunk may be shorter");

        let flat: Vec<u32> = chunks.iter().flat_map(|c| c.iter().copied()).collect();
        assert_eq!(flat, items, "every item streamed exactly once, in order");

        // A zero chunk size is clamped to 1 rather than panicking.
        let singles: Vec<&[u32]> = stream_chunks(&items, 0).collect();
        assert_eq!(singles.len(), 10);
    }

    /// The streaming helper handles an empty set without yielding chunks.
    #[test]
    fn stream_chunks_of_empty_yields_nothing() {
        let items: Vec<u32> = Vec::new();
        assert_eq!(stream_chunks(&items, 4).count(), 0);
    }
}
