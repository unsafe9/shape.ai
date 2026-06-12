//! Renderer cache + streaming groundwork: a revision-keyed LRU cache of derived
//! per-object render data and a chunked-streaming helper.
//!
//! [`RenderDataCache`] keys derived data by object id and a monotonic revision: an
//! unchanged revision is a hit (skips re-tessellation), bumping it forces a re-derive.
//! Invalidation reuses the patch path's `dirty_*_ids` set. Bounded by an entry cap
//! with LRU eviction so the working set stays the visible + prefetch set.
//!
//! Pure data/geometry: ephemeral derived state, never persisted.

#![allow(dead_code)]

use std::collections::HashMap;

pub const RENDER_DATA_CACHE_LIMIT: usize = 8192;

#[derive(Clone, Debug)]
struct CachedRenderEntry<T> {
    revision: u64,
    last_used: u64,
    data: T,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CacheOutcome {
    Hit,
    Miss,
    Stale,
}

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

    /// Advance the logical frame clock once per frame so `last_used` gives LRU a
    /// meaningful recency order.
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

    /// Look up `id` at `revision`, invoking `derive` only on a miss or stale
    /// revision so an unchanged object never re-tessellates.
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

    /// The revision an entry is currently cached at, or `None` if absent.
    pub fn cached_revision(&self, id: &str) -> Option<u64> {
        self.entries.get(id).map(|entry| entry.revision)
    }

    /// Drop the cached entry for `id` so it re-derives on its next request even at
    /// the same revision (used by the dirty-id invalidation path).
    pub fn invalidate(&mut self, id: &str) {
        self.entries.remove(id);
    }

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

    /// Evict the LRU entry while over capacity, never evicting `protect` (the entry
    /// just inserted this frame).
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

pub const STREAM_CHUNK_SIZE: usize = 512;

/// Split `items` into chunks of at most `chunk_size` (clamped to `1`); the final
/// chunk may be shorter.
pub fn stream_chunks<T>(items: &[T], chunk_size: usize) -> impl Iterator<Item = &[T]> {
    items.chunks(chunk_size.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn invalidate_dirty_ids_forces_re_derive_at_same_revision() {
        let mut cache: RenderDataCache<u32> = RenderDataCache::new();
        let mut derive_calls = 0;

        cache.begin_frame();
        cache.get_or_insert("edge-1", 1, || {
            derive_calls += 1;
            7
        });

        cache.invalidate_all(["edge-1", "edge-absent"]);
        assert_eq!(cache.cached_revision("edge-1"), None);

        cache.begin_frame();
        cache.get_or_insert("edge-1", 1, || {
            derive_calls += 1;
            8
        });

        assert_eq!(derive_calls, 2, "invalidated entry re-derives at same revision");
    }

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

    #[test]
    fn stream_chunks_splits_into_bounded_groups() {
        let items: Vec<u32> = (0..10).collect();
        let chunks: Vec<&[u32]> = stream_chunks(&items, 3).collect();
        assert_eq!(chunks.len(), 4);
        assert_eq!(chunks[0], &[0, 1, 2]);
        assert_eq!(chunks[3], &[9], "final chunk may be shorter");

        let flat: Vec<u32> = chunks.iter().flat_map(|c| c.iter().copied()).collect();
        assert_eq!(flat, items, "every item streamed exactly once, in order");

        let singles: Vec<&[u32]> = stream_chunks(&items, 0).collect();
        assert_eq!(singles.len(), 10);
    }

    #[test]
    fn stream_chunks_of_empty_yields_nothing() {
        let items: Vec<u32> = Vec::new();
        assert_eq!(stream_chunks(&items, 4).count(), 0);
    }
}
