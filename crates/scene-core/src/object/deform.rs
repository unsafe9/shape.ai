//! Chord-similarity deform for open-class paths (anchor-semantics design v3
//! §1/§2a, `docs/object-redesign-anchor-semantics-design.md`).
//!
//! open-class ⇔ exactly one subpath and that subpath is `closed:false` — the
//! data-level dichotomy of §1, not a UI classifier. An open path's pose IS its
//! endpoint pair: when the endpoints move (s,e) → (s′,e′), the unique
//! similarity `S = T(s′)·R(Δθ)·σI·T(−s)` taking the old chord to the new one is
//! applied to EVERY coordinate pair — bezier control points included — so the
//! drawn silhouette rotates/stretches as one piece (the "rope/rubber-band"
//! answer; absorbs v2 DU3 handle-follow with no separate mechanism).
//!
//! [`deform_open_path`] is the single source both the commit path
//! (EditGeometry) and the renderer live preview (G14 reexpand+patch, scene-core
//! consumed as an rlib in-process — no per-frame FFI) call, so committed and
//! previewed bytes cannot drift.
//!
//! Degenerate guard (§2a): an old chord under 1px, or a scale ratio σ outside
//! `[1/SIGMA_MAX, SIGMA_MAX]`, falls back to a pure translation by (s′−s) — no
//! rotation/scale — protecting spiral-like inputs whose chord ≪ arc length.
//!
//! Pure (no time/rng/IO), pointer-width-agnostic. Coordinates are object-local
//! quantized units (Q=8), the same space `reproject_node_local_quantized` emits.

use super::model::{path_string, Geometry, HandlePoint, PathNode, SubPath, GEOMETRY_QUANTUM_PER_PX};

/// Quantized units per logical pixel (Q=8).
const UNITS_PER_PX: f64 = GEOMETRY_QUANTUM_PER_PX as f64;

/// Old-chord length (quantized units) below which σ blows up: 1px. Falls back
/// to translation.
const MIN_CHORD_UNITS: f64 = UNITS_PER_PX;

/// Uniform-scale clamp bound: σ outside `[1/SIGMA_MAX, SIGMA_MAX]` is treated
/// as degenerate and falls back to translation.
const SIGMA_MAX: f64 = 64.0;

/// True iff `geometry` is open-class (§1): exactly one subpath, not closed.
/// Reads the hydrated `subpaths`; an unhydrated geometry (fresh off the wire)
/// is classified from its path-string instead of silently reporting false.
pub fn is_open_class(geometry: &Geometry) -> bool {
    if geometry.subpaths.is_empty() {
        return is_open_class_d(&geometry.path_string);
    }
    geometry.subpaths.len() == 1 && !geometry.subpaths[0].closed
}

/// Path-string form of [`is_open_class`] (the renderer consumes this via the
/// rlib). Malformed input is not open-class.
pub fn is_open_class_d(d: &str) -> bool {
    match path_string::parse(d) {
        Ok(subpaths) => subpaths.len() == 1 && !subpaths[0].closed,
        Err(_) => false,
    }
}

/// Round a deformed coordinate back to a quantized i32 unit (clamp into i32
/// range; NaN maps to 0) — the same provably-safe narrowing as
/// `drawing::quantize_px`, minus the px→unit scale (inputs are already units).
fn round_unit(v: f64) -> i32 {
    if v.is_nan() {
        return 0;
    }
    let r = v.round().clamp(f64::from(i32::MIN), f64::from(i32::MAX));
    #[allow(
        clippy::cast_possible_truncation,
        reason = "clamped to [i32::MIN, i32::MAX] above; the rounded f64 is an exact integer in range"
    )]
    let q = r as i32;
    q
}

/// Apply the chord similarity to an open-class path-string: node 0 is the old
/// start, the last node the old end; the similarity taking that chord to
/// `new_start`→`new_end` rewrites every coordinate pair (bezier control points
/// included), round-quantized back to integers. Coordinates — inputs and the
/// path-string alike — are quantized units (Q=8).
///
/// Returns `None` when `d` is not open-class (multi-subpath / closed /
/// malformed); a degenerate chord or σ falls back to translation (module doc).
pub fn deform_open_path(d: &str, new_start: (f64, f64), new_end: (f64, f64)) -> Option<String> {
    let subpaths = path_string::parse(d).ok()?;
    if subpaths.len() != 1 || subpaths[0].closed {
        return None;
    }
    let sub = &subpaths[0];
    let first = sub.nodes.first()?;
    let last = sub.nodes.last()?;
    let old_start = (f64::from(first.x), f64::from(first.y));
    let old_end = (f64::from(last.x), f64::from(last.y));
    let (ux, uy) = (old_end.0 - old_start.0, old_end.1 - old_start.1);
    let (vx, vy) = (new_end.0 - new_start.0, new_end.1 - new_start.1);
    let chord_sq = ux * ux + uy * uy;

    // The similarity as a complex ratio z = (e′−s′)/(e−s): one (zr, zi) pair
    // encodes R(Δθ)·σ, with σ = |z|. `None` = degenerate ⇒ translation only.
    let mut z = None;
    if chord_sq.sqrt() >= MIN_CHORD_UNITS {
        let zr = (vx * ux + vy * uy) / chord_sq;
        let zi = (vy * ux - vx * uy) / chord_sq;
        let sigma = zr.hypot(zi);
        if (1.0 / SIGMA_MAX..=SIGMA_MAX).contains(&sigma) {
            z = Some((zr, zi));
        }
    }
    let map = |x: f64, y: f64| -> (f64, f64) {
        match z {
            Some((zr, zi)) => {
                let dx = x - old_start.0;
                let dy = y - old_start.1;
                (new_start.0 + zr * dx - zi * dy, new_start.1 + zi * dx + zr * dy)
            }
            None => (x + new_start.0 - old_start.0, y + new_start.1 - old_start.1),
        }
    };

    let nodes = sub
        .nodes
        .iter()
        .map(|n| {
            let (x, y) = map(f64::from(n.x), f64::from(n.y));
            let (qx, qy) = (round_unit(x), round_unit(y));
            // Handles are node-relative, but the similarity applies to their
            // ABSOLUTE control points, so every emitted C coordinate is exactly
            // round(S(absolute)) — the handle-follow contract.
            let follow = |h: Option<HandlePoint>| {
                h.map(|h| {
                    let (ax, ay) = map(f64::from(n.x + h.dx), f64::from(n.y + h.dy));
                    HandlePoint { dx: round_unit(ax) - qx, dy: round_unit(ay) - qy }
                })
            };
            PathNode {
                x: qx,
                y: qy,
                in_handle: follow(n.in_handle),
                out_handle: follow(n.out_handle),
                width: n.width,
            }
        })
        .collect();
    Some(path_string::serialize(&[SubPath { closed: false, nodes }]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::model::FillRule;

    /// A canonical 3-node open path with bezier handles: n0 (0,0) with
    /// out-handle (8,−8), n1 (32,0) with in-handle (−8,−8), n2 (64,0).
    /// Chord: (0,0) → (64,0). All coords quantized units (8 = 1px).
    const CURVE: &str = "M 0 0 C 8 -8 24 -8 32 0 L 64 0";

    fn hydrated(d: &str) -> Geometry {
        let mut g = Geometry {
            path_string: d.to_string(),
            fill_rule: FillRule::EvenOdd,
            subpaths: Vec::new(),
        };
        g.ensure_parsed().unwrap();
        g
    }

    // -- is_open_class_d: positive / negative --

    #[test]
    fn open_class_d_accepts_a_single_open_subpath() {
        assert!(is_open_class_d("M 0 0 L 64 0"));
        assert!(is_open_class_d(CURVE));
    }

    #[test]
    fn open_class_d_rejects_closed_multi_and_malformed() {
        assert!(!is_open_class_d("M 0 0 L 80 0 L 80 40 L 0 40 Z"), "closed");
        assert!(!is_open_class_d("M 0 0 L 8 0 M 16 0 L 24 0"), "multi-subpath");
        assert!(!is_open_class_d(""), "empty");
        assert!(!is_open_class_d("Q 1 2"), "malformed");
    }

    // -- is_open_class: hydrated + unhydrated --

    #[test]
    fn open_class_reads_hydrated_subpaths() {
        assert!(is_open_class(&hydrated("M 0 0 L 64 0")));
        assert!(!is_open_class(&hydrated("M 0 0 L 80 0 L 80 40 L 0 40 Z")));
    }

    #[test]
    fn open_class_classifies_unhydrated_geometry_from_the_path_string() {
        let g = Geometry {
            path_string: "M 0 0 L 64 0".to_string(),
            fill_rule: FillRule::EvenOdd,
            subpaths: Vec::new(),
        };
        assert!(is_open_class(&g));
    }

    // -- deform: identity (same endpoints -> byte-identical d) --

    #[test]
    fn identity_endpoints_return_the_same_d() {
        assert_eq!(deform_open_path(CURVE, (0.0, 0.0), (64.0, 0.0)), Some(CURVE.to_string()));
    }

    // -- deform: pure translation (both endpoints share one delta) --

    #[test]
    fn pure_translation_shifts_every_coordinate_pair() {
        // Both endpoints +(16,24): z = 1 (no rotation/scale), so every node AND
        // every absolute control point shifts by exactly that delta.
        assert_eq!(
            deform_open_path(CURVE, (16.0, 24.0), (80.0, 24.0)),
            Some("M 16 24 C 24 16 40 16 48 24 L 80 24".to_string())
        );
    }

    // -- deform: hand-computed rotation + scale, handles following --

    #[test]
    fn rotate_and_scale_carry_the_bezier_handles() {
        // Chord (64,0) → (0,128): z = 2i, i.e. rotate +90° and scale ×2, so
        // S(x,y) = (−2y, 2x). Every absolute pair, control points included:
        //   n0 (0,0)   → (0,0);   c1 (8,−8)  → (16,16)
        //   c2 (24,−8) → (16,48); n1 (32,0)  → (0,64)
        //   n2 (64,0)  → (0,128)
        let out = deform_open_path(CURVE, (0.0, 0.0), (0.0, 128.0)).expect("open-class");
        assert_eq!(out, "M 0 0 C 16 16 16 48 0 64 L 0 128");
    }

    // -- degenerate guards: translation fallback --

    #[test]
    fn sub_pixel_chord_falls_back_to_translation() {
        // Old chord is 4 units (< 8 = 1px): the requested rotation/scale is
        // ignored; the whole path translates by (new_start − old_start) and the
        // free end does NOT land on new_end.
        assert_eq!(
            deform_open_path("M 0 0 L 4 0", (8.0, 8.0), (104.0, 208.0)),
            Some("M 8 8 L 12 8".to_string())
        );
    }

    #[test]
    fn sigma_outside_the_clamp_falls_back_to_translation() {
        // σ = 65 > 64: fallback. new_start == old_start, so the d is unchanged.
        assert_eq!(
            deform_open_path("M 0 0 L 8 0", (0.0, 0.0), (520.0, 0.0)),
            Some("M 0 0 L 8 0".to_string())
        );
        // σ = 5/640 < 1/64: same fallback on the shrink side.
        assert_eq!(
            deform_open_path("M 0 0 L 640 0", (0.0, 0.0), (5.0, 0.0)),
            Some("M 0 0 L 640 0".to_string())
        );
        // σ = 64 exactly stays a similarity (the clamp is inclusive).
        assert_eq!(
            deform_open_path("M 0 0 L 8 0", (0.0, 0.0), (512.0, 0.0)),
            Some("M 0 0 L 512 0".to_string())
        );
    }

    // -- deform: non-open-class input --

    #[test]
    fn closed_or_multi_subpath_deforms_to_none() {
        assert!(deform_open_path("M 0 0 L 80 0 L 80 40 L 0 40 Z", (0.0, 0.0), (1.0, 1.0)).is_none());
        assert!(deform_open_path("M 0 0 L 8 0 M 16 0 L 24 0", (0.0, 0.0), (1.0, 1.0)).is_none());
        assert!(deform_open_path("", (0.0, 0.0), (1.0, 1.0)).is_none());
    }
}
