//! Zoom-bucket LOD for cubic-bezier curve flattening. Quantizes continuous zoom
//! into discrete buckets and flattens with a per-bucket tolerance, so the polyline
//! can be cached by `(object id, zoom bucket)` and reused for every frame whose
//! zoom lands in the same bucket — a re-zoom within a bucket is a cache hit.
//!
//! Coordinate space: flattening operates in object-local px (`f32`); quantized
//! geometry coords (i32 at 8 units/px) convert via `/8.0` before flattening, and
//! the per-bucket flatness is likewise in px.

#![allow(dead_code)]

use std::collections::HashMap;

/// `bucket = clamp(round(log2(zoom)) + BUCKET_ZERO_OFFSET, 0..=u8::MAX)`: `u8` tiers
/// one octave apart. `round` (not `floor`) places each boundary at a `√2` multiple
/// so the octave anchors sit at band centers (hysteresis: jitter near a dwell zoom
/// cannot flip the bucket). The offset keeps `zoom < 1` (negative `log2`) in range;
/// `zoom == 1.0` maps to `BUCKET_ZERO_OFFSET`.
pub const BUCKET_ZERO_OFFSET: i32 = 16;

/// Quantize `zoom` into a `u8` bucket. Non-finite or non-positive zooms (no real
/// `log2`) clamp to bucket `0`, the coarsest tier.
pub fn zoom_bucket(zoom: f64) -> u8 {
    if !zoom.is_finite() || zoom <= 0.0 {
        return 0;
    }
    let tier = zoom.log2().round() + f64::from(BUCKET_ZERO_OFFSET);
    let clamped = tier.clamp(0.0, f64::from(u8::MAX));
    #[allow(
        clippy::cast_possible_truncation,
        reason = "clamped to [0, u8::MAX] above; the value is an exact integer in range"
    )]
    let bucket = clamped as u8;
    bucket
}

/// The zoom at the center of `bucket` — its octave anchor `2^(bucket - BUCKET_ZERO_OFFSET)`.
pub fn bucket_anchor_zoom(bucket: u8) -> f64 {
    let exponent = i32::from(bucket) - BUCKET_ZERO_OFFSET;
    2.0_f64.powi(exponent)
}

/// Flatness tolerance `ε` (object-local px) for a `bucket`. Tied to the anchor
/// zoom (`ε_local ≈ target_screen / zoom`) so on-screen error stays ~constant at
/// [`TARGET_SCREEN_ERR_PX`], clamped to `[MIN_FLATNESS_PX, MAX_FLATNESS_PX]`.
pub const TARGET_SCREEN_ERR_PX: f32 = 0.25;
pub const MIN_FLATNESS_PX: f32 = 0.05;
pub const MAX_FLATNESS_PX: f32 = 64.0;

pub fn flatness_for_bucket(bucket: u8) -> f32 {
    let anchor_zoom = crate::cast::narrow_f32(bucket_anchor_zoom(bucket));
    let eps = TARGET_SCREEN_ERR_PX / anchor_zoom;
    eps.clamp(MIN_FLATNESS_PX, MAX_FLATNESS_PX)
}

type Pt = (f32, f32);

/// Flatten one cubic bezier into a polyline within `flatness` px of the true curve
/// by recursive de Casteljau subdivision. Includes both endpoints. A non-positive/
/// non-finite `flatness` falls back to [`MIN_FLATNESS_PX`] so recursion terminates.
pub fn flatten_cubic(p0: Pt, c1: Pt, c2: Pt, p1: Pt, flatness: f32) -> Vec<Pt> {
    let tol = if flatness.is_finite() && flatness > 0.0 {
        flatness
    } else {
        MIN_FLATNESS_PX
    };
    let mut out = Vec::new();
    out.push(p0);
    subdivide(p0, c1, c2, p1, tol * tol, 0, &mut out);
    out.push(p1);
    out
}

/// Depth cap guards degenerate/cusp inputs that never pass the flatness test.
const MAX_SUBDIVISION_DEPTH: u8 = 24;

/// Subdivide, pushing only interior points. `tol_sq` is squared flatness so the
/// test avoids a `sqrt`.
fn subdivide(p0: Pt, c1: Pt, c2: Pt, p1: Pt, tol_sq: f32, depth: u8, out: &mut Vec<Pt>) {
    if depth >= MAX_SUBDIVISION_DEPTH || is_flat(p0, c1, c2, p1, tol_sq) {
        return;
    }
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

/// Flatness test using squared perpendicular distances (no `sqrt`).
fn is_flat(p0: Pt, c1: Pt, c2: Pt, p1: Pt, tol_sq: f32) -> bool {
    let d1_sq = dist_sq_to_segment(c1, p0, p1);
    let d2_sq = dist_sq_to_segment(c2, p0, p1);
    d1_sq <= tol_sq && d2_sq <= tol_sq
}

/// Squared distance from `p` to segment `a->b`; for a degenerate chord (`a == b`),
/// squared distance to the shared point.
fn dist_sq_to_segment(p: Pt, a: Pt, b: Pt) -> f32 {
    let abx = b.0 - a.0;
    let aby = b.1 - a.1;
    let apx = p.0 - a.0;
    let apy = p.1 - a.1;
    let len_sq = abx * abx + aby * aby;
    if len_sq <= f32::EPSILON {
        return apx * apx + apy * apy;
    }
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

/// A flattened-polyline cache keyed by `(object id, zoom bucket)`. A new bucket
/// (zoom crosses an octave) or `invalidate(id)` (geometry edit) forces a re-flatten.
/// Ephemeral derived geometry; never persisted.
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

    /// Borrow the cached polyline for `(id, bucket)`, invoking `flatten` only on a
    /// miss so a re-request at the same bucket never re-flattens.
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

    pub fn contains(&self, id: &str, bucket: u8) -> bool {
        self.entries.contains_key(&(id.to_string(), bucket))
    }

    /// Drop every cached bucket for `id` (e.g. its geometry changed).
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

    #[test]
    fn zoom_bucket_steps_one_per_octave() {
        assert_eq!(zoom_bucket(1.0), u8::try_from(BUCKET_ZERO_OFFSET).unwrap());
        assert_eq!(zoom_bucket(2.0), u8::try_from(BUCKET_ZERO_OFFSET + 1).unwrap());
        assert_eq!(zoom_bucket(4.0), u8::try_from(BUCKET_ZERO_OFFSET + 2).unwrap());
        assert_eq!(zoom_bucket(0.5), u8::try_from(BUCKET_ZERO_OFFSET - 1).unwrap());
        assert_eq!(zoom_bucket(0.25), u8::try_from(BUCKET_ZERO_OFFSET - 2).unwrap());
    }

    #[test]
    fn zoom_bucket_anchors_are_band_centered() {
        let anchor = zoom_bucket(1.0);
        // ±20% around the anchor stays in-bucket (boundary is at √2 ≈ 1.414).
        assert_eq!(zoom_bucket(1.2), anchor);
        assert_eq!(zoom_bucket(0.85), anchor);
        assert_eq!(zoom_bucket(1.45), anchor + 1);
    }

    #[test]
    fn zoom_bucket_handles_degenerate_zoom() {
        assert_eq!(zoom_bucket(0.0), 0);
        assert_eq!(zoom_bucket(-1.0), 0);
        assert_eq!(zoom_bucket(f64::NAN), 0);
        assert_eq!(zoom_bucket(f64::INFINITY), 0);
    }

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

    #[test]
    fn flatness_stays_within_clamp_band() {
        for bucket in 0..=u8::MAX {
            let eps = flatness_for_bucket(bucket);
            assert!(eps >= MIN_FLATNESS_PX && eps <= MAX_FLATNESS_PX);
        }
        assert_eq!(flatness_for_bucket(0), MAX_FLATNESS_PX);
        assert_eq!(flatness_for_bucket(u8::MAX), MIN_FLATNESS_PX);
    }

    /// Cubic bezier point at `t`, the analytic ground truth the polyline approximates.
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

        // Slack absorbs the discrete ground-truth sampling.
        let slack = flatness + 0.5;
        for &pt in &poly {
            let d = min_dist_to_curve(pt, p0, c1, c2, p1);
            assert!(
                d <= slack,
                "flattened point {pt:?} is {d} px off the curve (slack {slack})"
            );
        }
    }

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
        assert!(fine.len() > coarse.len());
    }

    #[test]
    fn straight_cubic_flattens_to_endpoints_only() {
        let p0 = (0.0, 0.0);
        let c1 = (25.0, 0.0);
        let c2 = (75.0, 0.0);
        let p1 = (100.0, 0.0);
        let poly = flatten_cubic(p0, c1, c2, p1, 0.1);
        assert_eq!(poly, vec![p0, p1]);
    }

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

    #[test]
    fn cache_hits_within_bucket_and_misses_across_buckets() {
        let mut cache = FlattenCache::new();
        let mut flatten_calls = 0;

        let _ = cache.get_or_insert("obj-1", 16, || {
            flatten_calls += 1;
            vec![(0.0, 0.0), (1.0, 1.0)]
        });
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

        let _ = cache.get_or_insert("obj-1", 17, || {
            flatten_calls += 1;
            vec![(2.0, 2.0)]
        });
        assert_eq!(flatten_calls, 2);
        assert_eq!(cache.misses, 2);
        assert!(cache.contains("obj-1", 16));
        assert!(cache.contains("obj-1", 17));
    }

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

    #[test]
    fn cache_clear_drops_everything() {
        let mut cache = FlattenCache::new();
        cache.get_or_insert("a", 16, || vec![(0.0, 0.0)]);
        cache.get_or_insert("b", 17, || vec![(1.0, 1.0)]);
        assert_eq!(cache.len(), 2);
        cache.clear();
        assert!(cache.is_empty());
    }

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
            assert_eq!(bucket, u8::try_from(BUCKET_ZERO_OFFSET).unwrap());
            let eps = flatness_for_bucket(bucket);
            cache.get_or_insert("curve", bucket, || {
                flatten_calls += 1;
                flatten_cubic(p0, c1, c2, p1, eps)
            });
        }
        assert_eq!(flatten_calls, 1, "all in-bucket zooms share one flatten");
    }
}
