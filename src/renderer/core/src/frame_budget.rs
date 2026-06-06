//! Per-frame work budget for interaction-under-load (T3.4).
//!
//! T3.1 gives the LOD ladder ([`crate::lod`]) and T3.2 gives the revision-keyed
//! residency cache ([`crate::render_cache::RenderDataCache`]). Together they make
//! the *resident working set* bounded, but they do not by themselves bound the
//! *per-frame derive cost*: on a reveal frame (pan into an unloaded ring, a tier
//! crossing that bumps revisions, a fresh `load_scene`) every newly-visible
//! object is a cache miss and would re-tessellate in one frame, stalling the
//! gesture the user is mid-way through. T3.4's interaction-latency contract is
//! "degrade detail, never stall": frames may drop visual detail under load, but
//! per-frame work must stay bounded so input stays responsive.
//!
//! This module schedules that bounded work. Given the candidate objects, the
//! camera, the culling viewport, and (optionally) the id the in-flight gesture is
//! manipulating, it:
//!
//! 1. **defers offscreen** — objects outside the padded viewport are never
//!    processed (they are not visible, so their derived data is not needed);
//! 2. **prioritizes the gesture target** — the object the user is dragging/editing
//!    is always processed first, so the action the user is performing never waits
//!    behind background reveal work;
//! 3. **orders the rest richest-tier-first** — using [`crate::lod::lod_tier`] on
//!    apparent size, so the few large foreground objects (which dominate the
//!    visible pixels) are derived before tiny far objects that degrade to a dot;
//! 4. **caps processed count** — only [`FrameWorkBudget::max_derives_per_frame`]
//!    cache *misses* (actual re-tessellations) run this frame; cache *hits* are
//!    free and do not count, and any work beyond the cap is **deferred** to a
//!    later frame, not dropped. Deferred objects still reuse whatever stale cached
//!    data exists, which is the graceful-degradation path.
//!
//! Pure data/geometry: host-neutral, no `JsValue`, no business fields. It reads
//! only `bounds` x `camera.zoom` (via [`crate::lod`]) and object ids. It is the
//! scheduling policy the draw path layers on the existing cull + cache seams; the
//! seed budget constant is tunable by benchmark evidence, exactly like the T3.1
//! tier thresholds and the T3.2 cache cap.

#![allow(dead_code)]

use crate::lod::{apparent_px, lod_tier, LodTier};
use crate::model::{CameraState, WorldRect};
use crate::render_cache::RenderDataCache;

/// Default cap on the number of objects whose derived render data may be
/// (re-)derived in a single frame. Seeded so a worst-case reveal frame spreads
/// its tessellation over a few frames instead of stalling one, keeping the
/// in-flight gesture responsive. Tunable by T3.4 against latency evidence, like
/// the T3.1 tier thresholds and the T3.2 [`crate::render_cache::RENDER_DATA_CACHE_LIMIT`].
pub const MAX_DERIVES_PER_FRAME: usize = 256;

/// One schedulable object: just the id and the geometry LOD needs. The scheduler
/// is generic over what the draw path actually derives, so it never sees business
/// fields — only `id` and `bounds`.
#[derive(Clone, Debug)]
pub struct FrameWorkItem<'a> {
    pub id: &'a str,
    pub bounds: &'a WorldRect,
    /// The revision the cache is keyed on (the object's current mutation count).
    /// An unchanged revision that is already cached is a hit and costs nothing.
    pub revision: u64,
}

/// The per-frame work budget configuration: the cull viewport, the camera the
/// interaction ran at, an optional in-flight gesture target, and the derive cap.
#[derive(Clone, Debug)]
pub struct FrameWorkBudget {
    pub viewport: WorldRect,
    pub camera: CameraState,
    /// The id of the object the in-flight gesture (drag/edit) is manipulating, if
    /// any. It is always scheduled first and is exempt from the offscreen defer so
    /// the active action never waits behind background reveal work.
    pub gesture_target: Option<String>,
    pub max_derives_per_frame: usize,
}

impl FrameWorkBudget {
    /// A budget with the default derive cap and no active gesture.
    pub fn new(viewport: WorldRect, camera: CameraState) -> Self {
        FrameWorkBudget {
            viewport,
            camera,
            gesture_target: None,
            max_derives_per_frame: MAX_DERIVES_PER_FRAME,
        }
    }

    /// Mark the in-flight gesture target so it is prioritized and never deferred.
    pub fn with_gesture_target(mut self, id: impl Into<String>) -> Self {
        self.gesture_target = Some(id.into());
        self
    }

    /// Override the per-frame derive cap (e.g. a test asserting the bound).
    pub fn with_max_derives(mut self, cap: usize) -> Self {
        self.max_derives_per_frame = cap.max(1);
        self
    }
}

/// Per-tier counts of the objects the scheduler actually *processed* this frame,
/// mirroring the diagnostic tier buckets the draw path already reports
/// (`WebGpuFrameStats.{full,compact,...}_tier_count`).
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

/// Outcome of scheduling one frame's work: how much was processed, how much was
/// deferred (visible but over budget) or culled (offscreen), and the per-tier
/// distribution of what was processed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameWorkReport {
    /// Objects whose derived data was produced this frame (cache miss/stale +
    /// re-derive). Bounded by `max_derives_per_frame`. This is the per-frame work
    /// the T3.4 assertion bounds.
    pub processed: usize,
    /// Visible objects skipped this frame because the derive cap was reached; they
    /// reuse stale cached data and are picked up on a later frame.
    pub deferred: usize,
    /// Objects outside the viewport: never processed this frame.
    pub culled: usize,
    /// Visible objects served from cache without a re-derive — free, never counted
    /// against the budget.
    pub cache_hits: usize,
    /// Per-tier distribution of the processed objects.
    pub tier_counts: TierCounts,
}

/// Schedule one frame of bounded per-object render-data derivation.
///
/// `items` is the full candidate set (typically all cards/edges/groups). `budget`
/// holds the viewport, camera, optional gesture target, and derive cap. `cache` is
/// the T3.2 residency cache; `derive` produces the per-object render data on a
/// miss/stale. The function returns a [`FrameWorkReport`] whose `processed` count
/// is guaranteed `<= budget.max_derives_per_frame`, with offscreen objects culled
/// and over-budget visible objects deferred to a later frame.
///
/// Ordering: the gesture target first (if visible), then the remaining visible
/// objects richest-tier-first (largest apparent size), so foreground detail is
/// derived before far objects that degrade to a dot. Cache hits are processed for
/// free regardless of position in the order and never consume the budget.
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

    // 1. Cull offscreen (the gesture target is exempt: an active drag may push its
    //    own bounds momentarily past the cull pad, and we must never starve it).
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

    // 2. Order: gesture target first, then richest tier (largest apparent px)
    //    first. A stable sort keeps scene order within a tier, so the schedule is
    //    deterministic for a given camera and gesture.
    let gesture_target = budget.gesture_target.as_deref();
    visible.sort_by(|(a, a_px, _), (b, b_px, _)| {
        let a_gesture = gesture_target == Some(a.id);
        let b_gesture = gesture_target == Some(b.id);
        b_gesture
            .cmp(&a_gesture)
            .then_with(|| b_px.partial_cmp(a_px).unwrap_or(std::cmp::Ordering::Equal))
    });

    // 3. Process under the cap. Cache hits are free; only misses/stales count.
    for (item, _px, tier) in visible {
        let already_cached = cache.cached_revision(item.id) == Some(item.revision);
        if already_cached {
            // Free hit: touch the cache so LRU recency is correct, do not spend
            // budget, and still record the tier (it is being shown this frame).
            cache.get_or_insert(item.id, item.revision, || derive(item.id));
            report.cache_hits += 1;
            report.tier_counts.record(tier);
            continue;
        }
        if report.processed >= budget.max_derives_per_frame {
            // Over budget: defer this visible object's re-derive to a later frame.
            // It keeps whatever stale data it has (graceful degradation).
            report.deferred += 1;
            continue;
        }
        cache.get_or_insert(item.id, item.revision, || derive(item.id));
        report.processed += 1;
        report.tier_counts.record(tier);
    }

    report
}

/// Half-open AABB overlap, matching the renderer's `rects_intersect` cull test so
/// the scheduler and the draw path agree on what "visible" means.
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

    /// Build N on-screen items laid out in a row inside the viewport. Each item is
    /// `220px` wide so at `zoom = 1.0` every one resolves to `Full` tier — the
    /// most expensive case, where bounding per-frame work matters most.
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

    /// THE T3.4 BOUND: with many more visible objects than the cap, the number of
    /// objects processed (re-derived) in a single frame never exceeds the budget;
    /// the rest are deferred, none are silently dropped, and every candidate is
    /// accounted for.
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
        // Everything visible that was not processed is deferred, not dropped.
        assert_eq!(report.processed + report.deferred, owned.len());
        assert_eq!(report.culled, 0, "all 2000 items are on-screen");
        assert_eq!(report.deferred, owned.len() - 256);
    }

    /// Offscreen objects are deferred (culled) and never processed: a 10k scene
    /// with only a handful visible processes only the visible handful.
    #[test]
    fn offscreen_objects_are_culled_not_processed() {
        let mut owned = row_items(8); // 8 on-screen
        // 1000 far-offscreen items well outside the viewport.
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

    /// The in-flight gesture target is always processed first and is exempt from
    /// the offscreen cull, so the action the user is performing never waits.
    #[test]
    fn gesture_target_is_prioritized_and_never_culled() {
        // The dragged card has been pushed just offscreen by the drag; a cap of 1
        // means only ONE object can be processed this frame.
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
        // The two on-screen cards lost the single slot and are deferred.
        assert_eq!(report.deferred, 2);
    }

    /// Cache hits are free: a steady-state frame where everything is already
    /// cached at the current revision processes zero objects, no matter the scale.
    #[test]
    fn steady_state_cache_hits_cost_no_budget() {
        let owned = row_items(1_000);
        let items = work_items(&owned);
        let budget = FrameWorkBudget::new(viewport(), camera(1.0)).with_max_derives(64);
        let previous = HashMap::new();
        let mut cache: RenderDataCache<u32> = RenderDataCache::with_capacity(2_000);

        // Frame 1: cold cache — bounded fill, the rest deferred.
        let first = schedule_frame_work(&items, &budget, &previous, &mut cache, |_id| 1);
        assert_eq!(first.processed, 64);

        // Warm the whole working set over enough frames (deferred work drains).
        for _ in 0..32 {
            schedule_frame_work(&items, &budget, &previous, &mut cache, |_id| 1);
        }

        // Steady state: everything resident at the same revision -> all hits, zero
        // derives, the per-frame work bound holds trivially.
        let steady = schedule_frame_work(&items, &budget, &previous, &mut cache, |_id| 1);
        assert_eq!(steady.processed, 0, "no re-derive when nothing changed");
        assert_eq!(steady.deferred, 0, "hits never defer");
        assert_eq!(steady.cache_hits, owned.len());
        assert!(steady.processed <= budget.max_derives_per_frame);
    }

    /// Deferred work drains across frames: repeatedly scheduling a cold scene
    /// eventually derives every visible object, each frame staying under the cap.
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
        // The full visible set was derived exactly once across the frames.
        assert_eq!(total_processed, owned.len());
        let drained = schedule_frame_work(&items, &budget, &previous, &mut cache, |_id| 1);
        assert_eq!(drained.processed, 0, "once warm, no further derives");
        assert_eq!(drained.cache_hits, owned.len());
    }

    /// Richest tier first: when the cap admits only some objects, the larger
    /// (Full-tier) foreground object is processed before tiny far ones.
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
