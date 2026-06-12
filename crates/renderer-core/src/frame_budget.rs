//! Per-frame work budget for interaction-under-load. Bounds the per-frame derive
//! cost so a reveal frame degrades detail instead of stalling the in-flight gesture.
//! The scheduler defers offscreen objects, processes the gesture target first, orders
//! the rest richest-tier-first, and caps the count of cache misses re-derived this
//! frame (hits are free; over-budget work is deferred, reusing stale data, not dropped).
//!
//! Pure data/geometry: reads only `bounds` x `camera.zoom` and object ids.

#![allow(dead_code)]

use crate::lod::{apparent_px, lod_tier, LodTier};
use crate::model::{CameraState, WorldRect};
use crate::render_cache::RenderDataCache;

pub const MAX_DERIVES_PER_FRAME: usize = 256;

#[derive(Clone, Debug)]
pub struct FrameWorkItem<'a> {
    pub id: &'a str,
    pub bounds: &'a WorldRect,
    /// The cache key revision; an unchanged cached revision is a free hit.
    pub revision: u64,
}

#[derive(Clone, Debug)]
pub struct FrameWorkBudget {
    pub viewport: WorldRect,
    pub camera: CameraState,
    /// The in-flight gesture target: scheduled first and exempt from the offscreen
    /// defer so the active action never waits behind background reveal work.
    pub gesture_target: Option<String>,
    pub max_derives_per_frame: usize,
}

impl FrameWorkBudget {
    pub fn new(viewport: WorldRect, camera: CameraState) -> Self {
        FrameWorkBudget {
            viewport,
            camera,
            gesture_target: None,
            max_derives_per_frame: MAX_DERIVES_PER_FRAME,
        }
    }

    pub fn with_gesture_target(mut self, id: impl Into<String>) -> Self {
        self.gesture_target = Some(id.into());
        self
    }

    pub fn with_max_derives(mut self, cap: usize) -> Self {
        self.max_derives_per_frame = cap.max(1);
        self
    }
}

/// Per-tier counts of the objects the scheduler processed this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TierCounts {
    pub full: usize,
    pub compact: usize,
    pub shape_only: usize,
    pub density: usize,
    pub minimap: usize,
}

impl TierCounts {
    fn record(&mut self, tier: LodTier) {
        match tier {
            LodTier::Full => self.full += 1,
            LodTier::Compact => self.compact += 1,
            LodTier::ShapeOnly => self.shape_only += 1,
            LodTier::Density => self.density += 1,
            LodTier::Minimap => self.minimap += 1,
        }
    }

    pub fn total(&self) -> usize {
        self.full + self.compact + self.shape_only + self.density + self.minimap
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameWorkReport {
    /// Objects re-derived this frame (cache miss/stale), bounded by `max_derives_per_frame`.
    pub processed: usize,
    /// Visible objects deferred to a later frame because the cap was reached.
    pub deferred: usize,
    /// Offscreen objects, never processed this frame.
    pub culled: usize,
    /// Visible objects served from cache without a re-derive (free).
    pub cache_hits: usize,
    pub tier_counts: TierCounts,
}

/// Schedule one frame of bounded per-object render-data derivation. `processed` is
/// guaranteed `<= budget.max_derives_per_frame`; offscreen objects are culled and
/// over-budget visible objects deferred. Order: gesture target first, then visible
/// objects richest-tier-first; cache hits are free and never consume the budget.
pub fn schedule_frame_work<T, F>(
    items: &[FrameWorkItem<'_>],
    budget: &FrameWorkBudget,
    previous_tiers: &std::collections::HashMap<String, LodTier>,
    cache: &mut RenderDataCache<T>,
    mut derive: F,
) -> FrameWorkReport
where
    T: Clone,
    F: FnMut(&str) -> T,
{
    cache.begin_frame();
    let mut report = FrameWorkReport::default();

    // Cull offscreen; the gesture target is exempt (an active drag may push its
    // bounds momentarily past the cull pad).
    let mut visible: Vec<(&FrameWorkItem<'_>, f64, LodTier)> = Vec::new();
    for item in items {
        let is_gesture = budget
            .gesture_target
            .as_deref()
            .is_some_and(|target| target == item.id);
        if !is_gesture && !rects_intersect(item.bounds, &budget.viewport) {
            report.culled += 1;
            continue;
        }
        let px = apparent_px(item.bounds, &budget.camera);
        let tier = lod_tier(px, previous_tiers.get(item.id).copied());
        visible.push((item, px, tier));
    }

    // Gesture target first, then richest tier first. Stable sort keeps scene order
    // within a tier, so the schedule is deterministic for a given camera and gesture.
    let gesture_target = budget.gesture_target.as_deref();
    visible.sort_by(|(a, a_px, _), (b, b_px, _)| {
        let a_gesture = gesture_target == Some(a.id);
        let b_gesture = gesture_target == Some(b.id);
        b_gesture
            .cmp(&a_gesture)
            .then_with(|| b_px.partial_cmp(a_px).unwrap_or(std::cmp::Ordering::Equal))
    });

    for (item, _px, tier) in visible {
        let already_cached = cache.cached_revision(item.id) == Some(item.revision);
        if already_cached {
            // Free hit: touch the cache for LRU recency without spending budget.
            cache.get_or_insert(item.id, item.revision, || derive(item.id));
            report.cache_hits += 1;
            report.tier_counts.record(tier);
            continue;
        }
        if report.processed >= budget.max_derives_per_frame {
            report.deferred += 1;
            continue;
        }
        cache.get_or_insert(item.id, item.revision, || derive(item.id));
        report.processed += 1;
        report.tier_counts.record(tier);
    }

    report
}

/// Half-open AABB overlap, matching the renderer's cull test so the scheduler and
/// the draw path agree on what "visible" means.
fn rects_intersect(a: &WorldRect, b: &WorldRect) -> bool {
    a.x <= b.x + b.width
        && a.x + a.width >= b.x
        && a.y <= b.y + b.height
        && a.y + a.height >= b.y
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> WorldRect {
        WorldRect {
            x,
            y,
            width: w,
            height: h,
        }
    }

    fn camera(zoom: f64) -> CameraState {
        CameraState {
            x: 0.0,
            y: 0.0,
            zoom,
        }
    }

    fn viewport() -> WorldRect {
        rect(0.0, 0.0, 1280.0, 720.0)
    }

    /// N on-screen items in a row, each 220px wide so every one resolves to `Full`
    /// tier at `zoom = 1.0` (the most expensive case).
    fn row_items(count: usize) -> Vec<(String, WorldRect)> {
        (0..count)
            .map(|i| (format!("card-{i}"), rect((i % 5) as f64 * 10.0, 0.0, 220.0, 220.0)))
            .collect()
    }

    fn work_items<'a>(owned: &'a [(String, WorldRect)]) -> Vec<FrameWorkItem<'a>> {
        owned
            .iter()
            .map(|(id, bounds)| FrameWorkItem {
                id: id.as_str(),
                bounds,
                revision: 1,
            })
            .collect()
    }

    #[test]
    fn per_frame_processed_count_is_bounded_under_load() {
        let owned = row_items(2_000); // far more than any per-frame cap
        let items = work_items(&owned);
        let budget = FrameWorkBudget::new(viewport(), camera(1.0)).with_max_derives(256);
        let previous = HashMap::new();
        let mut cache: RenderDataCache<u32> = RenderDataCache::new();
        let mut derive_calls = 0;

        let report = schedule_frame_work(&items, &budget, &previous, &mut cache, |_id| {
            derive_calls += 1;
            7
        });

        assert!(
            report.processed <= budget.max_derives_per_frame,
            "processed {} must not exceed the per-frame cap {}",
            report.processed,
            budget.max_derives_per_frame
        );
        assert_eq!(report.processed, 256, "a full reveal frame fills the budget");
        assert_eq!(
            derive_calls, report.processed,
            "derive runs exactly once per processed object, never for deferred ones"
        );
        assert_eq!(report.processed + report.deferred, owned.len());
        assert_eq!(report.culled, 0, "all 2000 items are on-screen");
        assert_eq!(report.deferred, owned.len() - 256);
    }

    #[test]
    fn offscreen_objects_are_culled_not_processed() {
        let mut owned = row_items(8); // 8 on-screen
        for i in 0..1_000 {
            owned.push((format!("far-{i}"), rect(100_000.0 + i as f64 * 300.0, 100_000.0, 220.0, 220.0)));
        }
        let items = work_items(&owned);
        let budget = FrameWorkBudget::new(viewport(), camera(1.0));
        let previous = HashMap::new();
        let mut cache: RenderDataCache<u32> = RenderDataCache::new();

        let report = schedule_frame_work(&items, &budget, &previous, &mut cache, |_id| 1);

        assert_eq!(report.processed, 8, "only the visible objects are processed");
        assert_eq!(report.culled, 1_000, "every offscreen object is culled");
        assert_eq!(report.deferred, 0, "8 visible is under the cap, nothing deferred");
    }

    #[test]
    fn gesture_target_is_prioritized_and_never_culled() {
        // Dragged card pushed offscreen by the drag; a cap of 1 admits one object.
        let owned = vec![
            ("dragged".to_string(), rect(-5_000.0, -5_000.0, 220.0, 220.0)), // offscreen
            ("onscreen-a".to_string(), rect(0.0, 0.0, 220.0, 220.0)),
            ("onscreen-b".to_string(), rect(40.0, 0.0, 220.0, 220.0)),
        ];
        let items = work_items(&owned);
        let budget = FrameWorkBudget::new(viewport(), camera(1.0))
            .with_gesture_target("dragged")
            .with_max_derives(1);
        let previous = HashMap::new();
        let mut cache: RenderDataCache<u32> = RenderDataCache::new();
        let mut derived: Vec<String> = Vec::new();

        let report = schedule_frame_work(&items, &budget, &previous, &mut cache, |id| {
            derived.push(id.to_string());
            1
        });

        assert_eq!(report.processed, 1, "cap of 1 means one derive");
        assert_eq!(derived, vec!["dragged"], "the gesture target wins the single slot");
        assert_eq!(report.culled, 0, "gesture target is exempt from offscreen cull");
        assert_eq!(report.deferred, 2);
    }

    #[test]
    fn steady_state_cache_hits_cost_no_budget() {
        let owned = row_items(1_000);
        let items = work_items(&owned);
        let budget = FrameWorkBudget::new(viewport(), camera(1.0)).with_max_derives(64);
        let previous = HashMap::new();
        let mut cache: RenderDataCache<u32> = RenderDataCache::with_capacity(2_000);

        let first = schedule_frame_work(&items, &budget, &previous, &mut cache, |_id| 1);
        assert_eq!(first.processed, 64);

        // Warm the whole working set as deferred work drains over frames.
        for _ in 0..32 {
            schedule_frame_work(&items, &budget, &previous, &mut cache, |_id| 1);
        }

        let steady = schedule_frame_work(&items, &budget, &previous, &mut cache, |_id| 1);
        assert_eq!(steady.processed, 0, "no re-derive when nothing changed");
        assert_eq!(steady.deferred, 0, "hits never defer");
        assert_eq!(steady.cache_hits, owned.len());
        assert!(steady.processed <= budget.max_derives_per_frame);
    }

    #[test]
    fn deferred_work_drains_across_frames_within_budget() {
        let owned = row_items(500);
        let items = work_items(&owned);
        let cap = 64;
        let budget = FrameWorkBudget::new(viewport(), camera(1.0)).with_max_derives(cap);
        let previous = HashMap::new();
        let mut cache: RenderDataCache<u32> = RenderDataCache::with_capacity(1_000);

        let mut total_processed = 0;
        for _ in 0..16 {
            let report = schedule_frame_work(&items, &budget, &previous, &mut cache, |_id| 1);
            assert!(
                report.processed <= cap,
                "every frame stays within the per-frame cap"
            );
            total_processed += report.processed;
            if report.deferred == 0 {
                break;
            }
        }
        assert_eq!(total_processed, owned.len());
        let drained = schedule_frame_work(&items, &budget, &previous, &mut cache, |_id| 1);
        assert_eq!(drained.processed, 0, "once warm, no further derives");
        assert_eq!(drained.cache_hits, owned.len());
    }

    #[test]
    fn processes_richest_tier_first() {
        let owned = vec![
            ("tiny".to_string(), rect(0.0, 0.0, 10.0, 10.0)), // ~Density/Minimap
            ("huge".to_string(), rect(20.0, 0.0, 400.0, 400.0)), // Full
        ];
        let items = work_items(&owned);
        let budget = FrameWorkBudget::new(viewport(), camera(1.0)).with_max_derives(1);
        let previous = HashMap::new();
        let mut cache: RenderDataCache<u32> = RenderDataCache::new();
        let mut derived: Vec<String> = Vec::new();

        let report = schedule_frame_work(&items, &budget, &previous, &mut cache, |id| {
            derived.push(id.to_string());
            1
        });

        assert_eq!(report.processed, 1);
        assert_eq!(derived, vec!["huge"], "the larger object is derived first");
        assert_eq!(report.tier_counts.full, 1);
    }
}
