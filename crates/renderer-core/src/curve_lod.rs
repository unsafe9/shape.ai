//! OB3.R6 — zoom-bucket LOD for cubic-bezier curve flattening.
//!
//! Object geometry (D2) is an SVG-subset path string whose curves are cubic
//! beziers. Drawing them on the GPU means flattening each cubic into a polyline.
//! Re-flattening every curve every frame is wasteful: the polyline a curve needs
//! depends only on how large it appears on screen, which changes coarsely with
//! zoom, not continuously. This module quantizes the continuous camera zoom into
//! discrete **buckets** and flattens with a per-bucket tolerance (flatness `ε`),
//! so the result can be cached by `(object id, zoom bucket)` and reused for every
//! frame whose zoom lands in the same bucket — a re-zoom within a bucket is a
//! cache hit, not a re-tessellation.
//!
//! This module is additive and host-neutral: pure CPU geometry, no `JsValue`, no
//! GPU device, no business fields. It does not touch the legacy
//! `RenderGroup`/`RenderCard`/`RenderEdge` draw path; the object pipeline wires it
//! in at the OB-4 cutover.
//!
//! Coordinate space: flattening operates in **object-local pixels** (`f32`).
//! Quantized geometry coords (i32 at 8 units/px, D2) convert to px by `/8.0`
//! before flattening; the per-bucket flatness is likewise expressed in px.

#![allow(dead_code)]

use std::collections::HashMap;

/// Zoom-bucket scheme — `log2` tiers.
///
/// Buckets quantize a continuous, strictly-positive `zoom` (world→screen scale,
/// `1.0` == 1 world px per screen px) into `u8` tiers spaced one octave apart:
/// each `+1` in bucket is a doubling of zoom. The mapping is
///
/// ```text
/// bucket = clamp( round(log2(zoom)) + BUCKET_ZERO_OFFSET , 0 ..= u8::MAX )
/// ```
///
/// `round` (not `floor`) is deliberate: it places each bucket *boundary* at a
/// `√2` multiple of zoom — halfway, in log space, between the two integer-octave
/// zoom values that anchor the adjacent buckets. A bucket therefore spans
/// `[2^(k-0.5), 2^(k+0.5))` and its anchor zoom `2^k` sits at the band's center,
/// as far from either boundary as possible. That is the hysteresis-friendly part:
/// realistic dwell zooms (the octave anchors `0.5`, `1`, `2`, `4`, …) sit at band
/// centers, so a small continuous pan/zoom jitter near a dwell point cannot
/// straddle a boundary and flip the bucket. (A consumer that pins exactly on a
/// `√2` boundary can still add its own dead-band, but the centering removes the
/// common flicker case without extra state.)
///
/// `BUCKET_ZERO_OFFSET` shifts the `log2` tier into the non-negative `u8` range so
/// far-zoomed-out views (`zoom < 1`, negative `log2`) still map to valid buckets.
/// With the offset, `zoom == 1.0` is [`BUCKET_ZERO_OFFSET`]; each octave out
/// subtracts one, each octave in adds one.
pub const BUCKET_ZERO_OFFSET: i32 = 16;

/// Quantize a continuous `zoom` into a discrete `u8` bucket. See module/constant
/// docs for the `log2`-octave scheme and why boundaries land at `√2` multiples.
///
/// Non-finite or non-positive zooms (which have no real `log2`) clamp to bucket
/// `0`, the coarsest tier, rather than panicking.
pub fn zoom_bucket(zoom: f64) -> u8 {
    if !zoom.is_finite() || zoom <= 0.0 {
        return 0;
    }
    let tier = zoom.log2().round() + f64::from(BUCKET_ZERO_OFFSET);
    // `tier` is already integer-valued (`round`); clamp into u8 before narrowing
    // so the `as u8` cast cannot wrap or saturate-surprise.
    let clamped = tier.clamp(0.0, f64::from(u8::MAX));
    clamped as u8
}

/// The zoom at the exact center of `bucket` — its octave anchor `2^(bucket -
/// BUCKET_ZERO_OFFSET)`. Inverse of [`zoom_bucket`] at the band center; handy for
/// tests and for picking a representative zoom when pre-warming a bucket.
pub fn bucket_anchor_zoom(bucket: u8) -> f64 {
    let exponent = i32::from(bucket) - BUCKET_ZERO_OFFSET;
    2.0_f64.powi(exponent)
}

/// Flatness tolerance `ε` (in object-local px) for a zoom `bucket`.
///
/// `ε` is the maximum allowed deviation between the flattened polyline and the
/// true curve. It must be **coarser (larger) when zoomed out** — a far curve
/// covers few screen px, so a loose polyline is visually exact — and **finer
/// (smaller) when zoomed in**, where deviations are magnified on screen.
///
/// We tie `ε` to the bucket's anchor zoom so that the *on-screen* error stays
/// roughly constant at a target of [`TARGET_SCREEN_ERR_PX`] screen px across
/// tiers: `ε_local ≈ target_screen / zoom`. Larger bucket (more zoomed in) ⇒
/// larger anchor zoom ⇒ smaller local `ε`. The result is clamped to
/// `[MIN_FLATNESS_PX, MAX_FLATNESS_PX]` so extreme tiers neither explode the
/// vertex count (too fine) nor visibly facet (too coarse).
pub const TARGET_SCREEN_ERR_PX: f32 = 0.25;
/// Floor on `ε`: finer than this wastes vertices below sub-pixel benefit.
pub const MIN_FLATNESS_PX: f32 = 0.05;
/// Ceiling on `ε`: coarser than this can visibly facet even a far curve.
pub const MAX_FLATNESS_PX: f32 = 64.0;

pub fn flatness_for_bucket(bucket: u8) -> f32 {
    // anchor zoom is always finite & positive; the cast to f32 is lossy only in
    // the far tails, which the clamp below absorbs.
    let anchor_zoom = bucket_anchor_zoom(bucket) as f32;
    let eps = TARGET_SCREEN_ERR_PX / anchor_zoom;
    eps.clamp(MIN_FLATNESS_PX, MAX_FLATNESS_PX)
}

type Pt = (f32, f32);

/// Flatten one cubic bezier `p0 -> c1 -> c2 -> p1` into a polyline whose every
/// segment is within `flatness` px of the true curve, by recursive de Casteljau
/// subdivision.
///
/// The returned polyline **includes both endpoints**: `p0` first, `p1` last, with
/// interior points only where the curve bends enough to need them. A segment is
/// accepted (no further split) once both control points `c1`, `c2` lie within
/// `flatness` of the chord `p0->p1` — the standard flatness test. A
/// non-positive/non-finite `flatness` is treated as [`MIN_FLATNESS_PX`] so the
/// recursion always terminates.
pub fn flatten_cubic(p0: Pt, c1: Pt, c2: Pt, p1: Pt, flatness: f32) -> Vec<Pt> {
    let tol = if flatness.is_finite() && flatness > 0.0 {
        flatness
    } else {
        MIN_FLATNESS_PX
    };
    let mut out = Vec::new();
    out.push(p0);
    // Recursion depth cap guards against degenerate/cusp inputs that never quite
    // pass the flatness test; at this depth the segments are far below any sane ε.
    subdivide(p0, c1, c2, p1, tol * tol, 0, &mut out);
    out.push(p1);
    out
}

/// Maximum recursion depth for [`flatten_cubic`]'s subdivision.
const MAX_SUBDIVISION_DEPTH: u8 = 24;

/// Recursively subdivide, pushing only *interior* points (callers supply the
/// endpoints). `tol_sq` is the squared flatness so the test avoids a `sqrt`.
fn subdivide(p0: Pt, c1: Pt, c2: Pt, p1: Pt, tol_sq: f32, depth: u8, out: &mut Vec<Pt>) {
    if depth >= MAX_SUBDIVISION_DEPTH || is_flat(p0, c1, c2, p1, tol_sq) {
        return;
    }
    // de Casteljau split at t = 0.5.
    let p01 = midpoint(p0, c1);
    let p12 = midpoint(c1, c2);
    let p23 = midpoint(c2, p1);
    let p012 = midpoint(p01, p12);
    let p123 = midpoint(p12, p23);
    let mid = midpoint(p012, p123);

    subdivide(p0, p01, p012, mid, tol_sq, depth + 1, out);
    out.push(mid);
    subdivide(mid, p123, p23, p1, tol_sq, depth + 1, out);
}

/// Flatness test: both control points within `sqrt(tol_sq)` of the `p0->p1`
/// chord. Uses squared perpendicular distances so no `sqrt` is needed.
fn is_flat(p0: Pt, c1: Pt, c2: Pt, p1: Pt, tol_sq: f32) -> bool {
    let d1_sq = dist_sq_to_segment(c1, p0, p1);
    let d2_sq = dist_sq_to_segment(c2, p0, p1);
    d1_sq <= tol_sq && d2_sq <= tol_sq
}

/// Squared distance from point `p` to the segment `a->b`. When `a == b`
/// (degenerate chord) this is the squared distance to the shared point.
fn dist_sq_to_segment(p: Pt, a: Pt, b: Pt) -> f32 {
    let abx = b.0 - a.0;
    let aby = b.1 - a.1;
    let apx = p.0 - a.0;
    let apy = p.1 - a.1;
    let len_sq = abx * abx + aby * aby;
    if len_sq <= f32::EPSILON {
        // Degenerate chord: distance to the point `a`.
        return apx * apx + apy * apy;
    }
    // Project `ap` onto `ab`, clamp to the segment, measure the residual.
    let t = ((apx * abx + apy * aby) / len_sq).clamp(0.0, 1.0);
    let cx = a.0 + t * abx;
    let cy = a.1 + t * aby;
    let dx = p.0 - cx;
    let dy = p.1 - cy;
    dx * dx + dy * dy
}

fn midpoint(a: Pt, b: Pt) -> Pt {
    ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5)
}

/// A flattened-polyline cache keyed by `(object id, zoom bucket)`.
///
/// The draw path asks the cache for an object's flattened curve at the current
/// zoom bucket; a hit (same id + bucket) returns the cached polyline without
/// re-flattening, so re-zoom within a bucket — and every steady-state frame — is
/// free. Bumping to a new bucket (zoom crossing an octave boundary) or editing
/// the object (invalidate by id) forces a re-flatten.
///
/// Like [`crate::render_cache::RenderDataCache`], this is ephemeral derived state:
/// it caches geometry, holds no business fields, and is never persisted.
#[derive(Clone, Debug, Default)]
pub struct FlattenCache {
    entries: HashMap<(String, u8), Vec<Pt>>,
    pub hits: usize,
    pub misses: usize,
}

impl FlattenCache {
    pub fn new() -> Self {
        FlattenCache::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Borrow the cached polyline for `(id, bucket)`, flattening on a miss via
    /// `flatten` and caching the result. `flatten` runs **only** on a miss, so a
    /// re-request at the same bucket never re-flattens.
    pub fn get_or_insert<F>(&mut self, id: &str, bucket: u8, flatten: F) -> &[Pt]
    where
        F: FnOnce() -> Vec<Pt>,
    {
        let key = (id.to_string(), bucket);
        if self.entries.contains_key(&key) {
            self.hits += 1;
        } else {
            self.misses += 1;
            let data = flatten();
            self.entries.insert(key.clone(), data);
        }
        self.entries
            .get(&key)
            .expect("entry present after insert")
            .as_slice()
    }

    /// Whether a polyline is cached for exactly `(id, bucket)`.
    pub fn contains(&self, id: &str, bucket: u8) -> bool {
        self.entries.contains_key(&(id.to_string(), bucket))
    }

    /// Drop every cached bucket for `id` (e.g. the object's geometry changed). All
    /// buckets must be re-flattened on their next request.
    pub fn invalidate(&mut self, id: &str) {
        self.entries.retain(|(entry_id, _), _| entry_id != id);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- zoom_bucket -------------------------------------------------------

    /// `zoom_bucket` is monotonic non-decreasing in zoom: zooming in never lowers
    /// the bucket. Sweeps a wide log range to catch any non-monotonic boundary.
    #[test]
    fn zoom_bucket_is_monotonic_in_zoom() {
        let mut prev = zoom_bucket(1e-6);
        let mut z = 1e-6_f64;
        while z < 1e6 {
            let b = zoom_bucket(z);
            assert!(
                b >= prev,
                "bucket dropped as zoom increased: zoom={z} bucket={b} prev={prev}"
            );
            prev = b;
            z *= 1.05; // fine multiplicative sweep across many octave boundaries
        }
    }

    /// Octave anchors map exactly to their tier, and one octave of zoom is exactly
    /// one bucket step — confirming the log2 spacing.
    #[test]
    fn zoom_bucket_steps_one_per_octave() {
        assert_eq!(zoom_bucket(1.0), BUCKET_ZERO_OFFSET as u8);
        assert_eq!(zoom_bucket(2.0), (BUCKET_ZERO_OFFSET + 1) as u8);
        assert_eq!(zoom_bucket(4.0), (BUCKET_ZERO_OFFSET + 2) as u8);
        assert_eq!(zoom_bucket(0.5), (BUCKET_ZERO_OFFSET - 1) as u8);
        assert_eq!(zoom_bucket(0.25), (BUCKET_ZERO_OFFSET - 2) as u8);
    }

    /// Anchor zooms sit at band centers: a small jitter around an anchor does not
    /// cross a boundary (the hysteresis-friendly property), but a √2 step does.
    #[test]
    fn zoom_bucket_anchors_are_band_centered() {
        let anchor = zoom_bucket(1.0);
        // ±20% around the anchor stays in-bucket (boundary is at √2 ≈ 1.414).
        assert_eq!(zoom_bucket(1.2), anchor);
        assert_eq!(zoom_bucket(0.85), anchor);
        // Just past the √2 boundary lands in the next octave up.
        assert_eq!(zoom_bucket(1.45), anchor + 1);
    }

    /// Degenerate zooms clamp to the coarsest bucket instead of panicking.
    #[test]
    fn zoom_bucket_handles_degenerate_zoom() {
        assert_eq!(zoom_bucket(0.0), 0);
        assert_eq!(zoom_bucket(-1.0), 0);
        assert_eq!(zoom_bucket(f64::NAN), 0);
        // Non-finite zoom has no real log2, so it clamps to the coarsest bucket
        // rather than the finest — a far/unknown view is the safe default.
        assert_eq!(zoom_bucket(f64::INFINITY), 0);
    }

    // ---- flatness_for_bucket ----------------------------------------------

    /// Flatness is monotonic non-increasing in bucket: zooming in (higher bucket)
    /// never makes the tolerance coarser. Coarser out, finer in.
    #[test]
    fn flatness_is_finer_when_zoomed_in() {
        let mut prev = flatness_for_bucket(0);
        for bucket in 1..=u8::MAX {
            let eps = flatness_for_bucket(bucket);
            assert!(
                eps <= prev,
                "flatness grew while zooming in: bucket={bucket} eps={eps} prev={prev}"
            );
            prev = eps;
        }
    }

    /// Tolerance stays inside the documented clamp band at every tier.
    #[test]
    fn flatness_stays_within_clamp_band() {
        for bucket in 0..=u8::MAX {
            let eps = flatness_for_bucket(bucket);
            assert!(eps >= MIN_FLATNESS_PX && eps <= MAX_FLATNESS_PX);
        }
        // Far-out tier saturates at the coarse ceiling; far-in at the fine floor.
        assert_eq!(flatness_for_bucket(0), MAX_FLATNESS_PX);
        assert_eq!(flatness_for_bucket(u8::MAX), MIN_FLATNESS_PX);
    }

    // ---- flatten_cubic -----------------------------------------------------

    /// Cubic bezier point at parameter `t`, the analytic ground truth the polyline
    /// must approximate. Used to verify points lie on the curve.
    fn bezier_at(p0: Pt, c1: Pt, c2: Pt, p1: Pt, t: f32) -> Pt {
        let u = 1.0 - t;
        let b0 = u * u * u;
        let b1 = 3.0 * u * u * t;
        let b2 = 3.0 * u * t * t;
        let b3 = t * t * t;
        (
            b0 * p0.0 + b1 * c1.0 + b2 * c2.0 + b3 * p1.0,
            b0 * p0.1 + b1 * c1.1 + b2 * c2.1 + b3 * p1.1,
        )
    }

    /// Min distance from `p` to the densely-sampled true curve — an approximate
    /// "is this point on the curve" check.
    fn min_dist_to_curve(p: Pt, p0: Pt, c1: Pt, c2: Pt, p1: Pt) -> f32 {
        let mut best = f32::MAX;
        for i in 0..=2000 {
            let t = i as f32 / 2000.0;
            let q = bezier_at(p0, c1, c2, p1, t);
            let d = ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt();
            if d < best {
                best = d;
            }
        }
        best
    }

    /// Endpoints are preserved exactly, and every flattened point lies within the
    /// flatness tolerance of the true curve.
    #[test]
    fn flatten_preserves_endpoints_and_stays_near_curve() {
        let p0 = (0.0, 0.0);
        let c1 = (30.0, 90.0);
        let c2 = (70.0, -30.0);
        let p1 = (100.0, 40.0);
        let flatness = 0.5;

        let poly = flatten_cubic(p0, c1, c2, p1, flatness);

        assert!(poly.len() >= 2);
        assert_eq!(poly[0], p0, "first point is the start endpoint");
        assert_eq!(*poly.last().unwrap(), p1, "last point is the end endpoint");

        // Every produced vertex sits on (within tolerance of) the true curve. The
        // flatness bounds chord deviation, so allow a small slack for the discrete
        // ground-truth sampling.
        let slack = flatness + 0.5;
        for &pt in &poly {
            let d = min_dist_to_curve(pt, p0, c1, c2, p1);
            assert!(
                d <= slack,
                "flattened point {pt:?} is {d} px off the curve (slack {slack})"
            );
        }
    }

    /// Chord-deviation check: every flattened *segment* approximates the curve
    /// arc between its parameters to within the tolerance. Samples the curve and
    /// confirms each true point is within `flatness + slack` of the polyline.
    #[test]
    fn flatten_polyline_tracks_curve_within_tolerance() {
        let p0 = (0.0, 0.0);
        let c1 = (10.0, 120.0);
        let c2 = (90.0, 120.0);
        let p1 = (100.0, 0.0);
        let flatness = 0.4;
        let poly = flatten_cubic(p0, c1, c2, p1, flatness);

        let slack = flatness + 0.5;
        for i in 0..=500 {
            let t = i as f32 / 500.0;
            let q = bezier_at(p0, c1, c2, p1, t);
            // Nearest distance from the true point to the polyline.
            let mut best = f32::MAX;
            for seg in poly.windows(2) {
                let d2 = dist_sq_to_segment(q, seg[0], seg[1]);
                if d2 < best {
                    best = d2;
                }
            }
            let d = best.sqrt();
            assert!(
                d <= slack,
                "curve point {q:?} is {d} px from the polyline (slack {slack})"
            );
        }
    }

    /// A finer flatness yields at least as many points as a coarser one for the
    /// same curve — more subdivision when the tolerance tightens.
    #[test]
    fn finer_flatness_yields_at_least_as_many_points() {
        let p0 = (0.0, 0.0);
        let c1 = (20.0, 100.0);
        let c2 = (80.0, -40.0);
        let p1 = (100.0, 50.0);

        let coarse = flatten_cubic(p0, c1, c2, p1, 8.0);
        let medium = flatten_cubic(p0, c1, c2, p1, 1.0);
        let fine = flatten_cubic(p0, c1, c2, p1, 0.1);

        assert!(medium.len() >= coarse.len());
        assert!(fine.len() >= medium.len());
        // A finer tolerance must actually do more work on a curved input.
        assert!(fine.len() > coarse.len());
    }

    /// A straight "curve" (collinear control points) needs no interior points
    /// regardless of tolerance — just the two endpoints.
    #[test]
    fn straight_cubic_flattens_to_endpoints_only() {
        let p0 = (0.0, 0.0);
        let c1 = (25.0, 0.0);
        let c2 = (75.0, 0.0);
        let p1 = (100.0, 0.0);
        let poly = flatten_cubic(p0, c1, c2, p1, 0.1);
        assert_eq!(poly, vec![p0, p1]);
    }

    /// Non-finite / non-positive flatness falls back to the fine floor and still
    /// terminates with valid endpoints.
    #[test]
    fn flatten_handles_degenerate_flatness() {
        let p0 = (0.0, 0.0);
        let c1 = (30.0, 90.0);
        let c2 = (70.0, -30.0);
        let p1 = (100.0, 40.0);
        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let poly = flatten_cubic(p0, c1, c2, p1, bad);
            assert!(poly.len() >= 2);
            assert_eq!(poly[0], p0);
            assert_eq!(*poly.last().unwrap(), p1);
        }
    }

    // ---- FlattenCache ------------------------------------------------------

    /// Re-request at the same bucket is a hit and never re-flattens; a new bucket
    /// is a miss that re-flattens.
    #[test]
    fn cache_hits_within_bucket_and_misses_across_buckets() {
        let mut cache = FlattenCache::new();
        let mut flatten_calls = 0;

        let _ = cache.get_or_insert("obj-1", 16, || {
            flatten_calls += 1;
            vec![(0.0, 0.0), (1.0, 1.0)]
        });
        // Same id + bucket: hit, flatten not called again.
        let again = cache
            .get_or_insert("obj-1", 16, || {
                flatten_calls += 1;
                vec![(9.0, 9.0)] // would only appear on a wrong re-flatten
            })
            .to_vec();
        assert_eq!(again, vec![(0.0, 0.0), (1.0, 1.0)]);
        assert_eq!(flatten_calls, 1, "hit must not re-flatten");
        assert_eq!(cache.hits, 1);
        assert_eq!(cache.misses, 1);

        // Different bucket for the same object: miss, re-flatten.
        let _ = cache.get_or_insert("obj-1", 17, || {
            flatten_calls += 1;
            vec![(2.0, 2.0)]
        });
        assert_eq!(flatten_calls, 2);
        assert_eq!(cache.misses, 2);
        assert!(cache.contains("obj-1", 16));
        assert!(cache.contains("obj-1", 17));
    }

    /// Invalidating an id drops all its buckets, forcing a re-flatten next time.
    #[test]
    fn cache_invalidate_drops_all_buckets_for_id() {
        let mut cache = FlattenCache::new();
        cache.get_or_insert("a", 16, || vec![(0.0, 0.0)]);
        cache.get_or_insert("a", 17, || vec![(1.0, 1.0)]);
        cache.get_or_insert("b", 16, || vec![(2.0, 2.0)]);
        assert_eq!(cache.len(), 3);

        cache.invalidate("a");
        assert!(!cache.contains("a", 16));
        assert!(!cache.contains("a", 17));
        assert!(cache.contains("b", 16), "other objects untouched");

        let mut reflattened = false;
        cache.get_or_insert("a", 16, || {
            reflattened = true;
            vec![(0.0, 0.0)]
        });
        assert!(reflattened, "invalidated id re-flattens");
    }

    /// `clear` drops all residency (e.g. on a full scene reload).
    #[test]
    fn cache_clear_drops_everything() {
        let mut cache = FlattenCache::new();
        cache.get_or_insert("a", 16, || vec![(0.0, 0.0)]);
        cache.get_or_insert("b", 17, || vec![(1.0, 1.0)]);
        assert_eq!(cache.len(), 2);
        cache.clear();
        assert!(cache.is_empty());
    }

    /// End-to-end: a steady zoom that wiggles inside one bucket re-flattens once;
    /// the bucket + per-bucket flatness drive the cache key as designed.
    #[test]
    fn rezoom_within_bucket_is_a_single_flatten() {
        let mut cache = FlattenCache::new();
        let p0 = (0.0, 0.0);
        let c1 = (20.0, 60.0);
        let c2 = (80.0, 60.0);
        let p1 = (100.0, 0.0);
        let mut flatten_calls = 0;

        // Several zooms that all land in the same bucket (anchor 1.0, band centered).
        for zoom in [0.9_f64, 1.0, 1.1, 1.25] {
            let bucket = zoom_bucket(zoom);
            assert_eq!(bucket, BUCKET_ZERO_OFFSET as u8);
            let eps = flatness_for_bucket(bucket);
            cache.get_or_insert("curve", bucket, || {
                flatten_calls += 1;
                flatten_cubic(p0, c1, c2, p1, eps)
            });
        }
        assert_eq!(flatten_calls, 1, "all in-bucket zooms share one flatten");
    }
}
