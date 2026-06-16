//! Pure 2D affine decomposition / composition + the geometry-orientation
//! extraction backing the `Canonicalize` op. The single source of truth the
//! inspector reads rotation/scale/skew through, so the shell does no matrix math.
//!
//! Pure (no time/rng/IO). Matrix convention matches [`Transform3x3`]: row-major
//! `m[0] = [a, c, e]`, `m[1] = [b, d, f]`, `m[2] = [0, 0, 1]`. The linear part
//! `L = [[a, c], [b, d]]` maps local -> world; column 0 `(a, b)` is the image of
//! the local x-axis, column 1 `(c, d)` the local y-axis; translation is `(e, f)`.
//! Angles are radians, CCW-positive in the math convention; the renderer's y-down
//! is irrelevant here (the decomposition returns whatever the matrix encodes).

use serde::{Deserialize, Serialize};

use crate::object::anchor_follow::{affine_of, apply_affine, invert_affine};
use crate::object::model::{Geometry, LocalPoint, SubPath, Transform3x3, GEOMETRY_QUANTUM_PER_PX};

/// Quantized units per logical pixel (Q=8); the geometry de/requantize uses the
/// SAME factor + `round()` discipline as `world_to_local_quantized`.
const UNITS_PER_PX: f64 = GEOMETRY_QUANTUM_PER_PX as f64;

/// An edge shorter than this (object-local px) does not count as carrying a
/// dominant orientation, so a blob of tiny similar edges canonicalizes to a no-op.
const MIN_DOMINANT_EDGE_PX: f64 = 1.0;

/// Below this (radians) a folded angle reads as already axis-aligned, so
/// `canonicalize_orientation` no-ops rather than churning sub-quantum noise.
const ANGLE_EPSILON_RAD: f64 = 1e-6;

/// Affine decomposition result (logical px / radians). Round-trips with
/// [`compose_affine`] for the common translate+rotate+scale case (skew 0).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-gen", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-gen", ts(export, export_to = "object-wire.ts"))]
#[serde(rename_all = "camelCase")]
pub struct AffineDecomposition {
    pub translate: (f64, f64),
    pub rotation_rad: f64,
    pub scale: (f64, f64),
    pub skew_rad: f64,
}

/// QR / Gram-Schmidt decomposition of the 2x2 linear part into
/// translate · rotate · skew · scale.
pub fn decompose_affine(t: &Transform3x3) -> AffineDecomposition {
    let m = &t.m;
    let (a, c, e) = (m[0][0], m[0][1], m[0][2]);
    let (b, d, f) = (m[1][0], m[1][1], m[1][2]);

    let scale_x = a.hypot(b);
    if scale_x == 0.0 {
        // Degenerate x-column: no rotation to recover; report the y-extent only.
        return AffineDecomposition {
            translate: (e, f),
            rotation_rad: 0.0,
            scale: (0.0, c.hypot(d)),
            skew_rad: 0.0,
        };
    }

    let rotation_rad = b.atan2(a);
    // Unit x-column.
    let (ax, bx) = (a / scale_x, b / scale_x);
    // Shear = projection of the y-column onto the unit x-column.
    let shear = ax * c + bx * d;
    // Remove the x-component from the y-column; its length is the y-scale.
    let (cy, dy) = (c - ax * shear, d - bx * shear);
    let mut scale_y = cy.hypot(dy);

    // A reflection (negative determinant) lands in the y-scale sign so rotation
    // stays the proper-rotation branch.
    let det = a * d - b * c;
    if det < 0.0 {
        scale_y = -scale_y;
    }

    let skew_rad = if scale_y != 0.0 { shear.atan2(scale_y) } else { 0.0 };

    AffineDecomposition {
        translate: (e, f),
        rotation_rad,
        scale: (scale_x, scale_y),
        skew_rad,
    }
}

/// Patch one decomposed transform field and recompose, so the shell never owns the
/// deg<->rad conversion nor the translate/rotation decomposition seam. `field` is
/// the inspector control's id: `x`/`y` set the translate axis (px, verbatim),
/// `rotation`/`rotation-flow` set the rotation from the DISPLAY unit (degrees).
/// An unrecognized field leaves the transform unchanged. width/height resize
/// through [`resize_axis`] instead — they are not single-field decompositions.
pub fn set_transform_field(transform: &Transform3x3, field: &str, value: f64) -> Transform3x3 {
    let mut d = decompose_affine(transform);
    match field {
        "x" => d.translate.0 = value,
        "y" => d.translate.1 = value,
        "rotation" | "rotation-flow" => d.rotation_rad = value.to_radians(),
        _ => return *transform,
    }
    compose_affine(d.translate, d.rotation_rad, d.scale, d.skew_rad)
}

/// Build `T(translate) · R(rotation) · Sk(skew) · S(scale)` back into the
/// row-major matrix. Inverse of [`decompose_affine`] for translate+rotate+scale.
pub fn compose_affine(
    translate: (f64, f64),
    rotation_rad: f64,
    scale: (f64, f64),
    skew_rad: f64,
) -> Transform3x3 {
    let (cos, sin) = (rotation_rad.cos(), rotation_rad.sin());
    let tan = skew_rad.tan();
    let (sx, sy) = scale;

    // R · Sk · S, with Sk = [[1, tan],[0, 1]] shearing the y-column.
    // R·Sk = [[cos, cos*tan - sin], [sin, sin*tan + cos]]; then scale the columns.
    let a = cos * sx;
    let b = sin * sx;
    let c = (cos * tan - sin) * sy;
    let d = (sin * tan + cos) * sy;

    Transform3x3 {
        m: [[a, c, translate.0], [b, d, translate.1], [0.0, 0.0, 1.0]],
    }
}

/// Returns `Some((new_transform, new_geometry))`: the geometry's dominant edge
/// angle extracted into `transform` (composed about the geometry centroid so the
/// world appearance is unchanged) and the geometry rewritten axis-aligned. `None`
/// is the no-op case (no dominant angle — already axis-aligned, or freeform with no
/// edge at least [`MIN_DOMINANT_EDGE_PX`] long — a sub-epsilon angle, or no
/// centroid), so the caller skips it without an unused geometry clone.
///
/// `geometry.subpaths` must already be hydrated (`ensure_parsed`).
pub fn canonicalize_orientation(
    transform: &Transform3x3,
    geometry: &Geometry,
) -> Option<(Transform3x3, Geometry)> {
    let angle = dominant_angle(geometry)?;
    if angle.abs() < ANGLE_EPSILON_RAD {
        return None;
    }

    // Centroid (object-local px) of every node — the pivot both the geometry
    // rotation and the transform compensation turn about.
    let (cx, cy) = centroid_px(geometry)?;

    // Rotate every node by -angle about the centroid, in px, then requantize.
    let (cos, sin) = ((-angle).cos(), (-angle).sin());
    let new_subpaths: Vec<SubPath> = geometry
        .subpaths
        .iter()
        .map(|sp| SubPath {
            closed: sp.closed,
            nodes: sp
                .nodes
                .iter()
                .map(|node| {
                    let (nx, ny) = rotate_point_q(node.x, node.y, cx, cy, cos, sin);
                    let mut out = *node;
                    out.x = nx;
                    out.y = ny;
                    // Bezier handles are node-relative offsets — rotate the offset
                    // vector itself (no centroid term), keeping curvature intact.
                    out.in_handle = node.in_handle.map(|h| rotate_handle(h, cos, sin));
                    out.out_handle = node.out_handle.map(|h| rotate_handle(h, cos, sin));
                    out
                })
                .collect(),
        })
        .collect();
    let new_geometry = Geometry::from_subpaths(new_subpaths, geometry.fill_rule);

    // Compose R(angle) about the centroid onto the existing transform on the RIGHT
    // (local space), so each rotated node maps back to its original world point.
    let pivot_rotation = centroid_rotation(cx, cy, angle);
    let new_transform = transform.mul(&pivot_rotation);

    Some((new_transform, new_geometry))
}

/// Re-home a peer anchor's target-local `at` so its WORLD position is preserved
/// when the target's transform changes from `old` to `new` (e.g. `Canonicalize`,
/// which rewrites the target's geometry + transform but holds every world point
/// fixed). The stored `at` is in the target's OLD local quantized space; this maps
/// it to the NEW local quantized space via `new^-1 · old`, requantized with the
/// canonical `round()` discipline. A singular `new` (degenerate) leaves `at`
/// unchanged (the inverse falls back to identity, same as `invert_affine`).
pub fn rehome_anchor_local(old: &Transform3x3, new: &Transform3x3, at: LocalPoint) -> LocalPoint {
    let lx = f64::from(at.x) / UNITS_PER_PX;
    let ly = f64::from(at.y) / UNITS_PER_PX;
    let (wx, wy) = apply_affine(&affine_of(old), lx, ly);
    let inv = invert_affine(&affine_of(new));
    let (nx, ny) = apply_affine(&inv, wx, wy);
    LocalPoint {
        x: quantize(nx),
        y: quantize(ny),
    }
}

/// `translate(cx,cy) · R(angle) · translate(-cx,-cy)` — a CCW rotation about the
/// object-local pivot `(cx, cy)` (logical px).
fn centroid_rotation(cx: f64, cy: f64, angle: f64) -> Transform3x3 {
    let (cos, sin) = (angle.cos(), angle.sin());
    // Linear part R = [[cos, -sin],[sin, cos]]; translation = c - R·c.
    let e = cx - (cos * cx - sin * cy);
    let f = cy - (sin * cx + cos * cy);
    Transform3x3 {
        m: [[cos, -sin, e], [sin, cos, f], [0.0, 0.0, 1.0]],
    }
}

/// The angle (radians) of the geometry's longest edge, folded into `[0, π/2)`
/// (a rectangle's dominant axis is ambiguous by quadrant). `None` when no edge is
/// >= [`MIN_DOMINANT_EDGE_PX`] long (freeform => reads rotation 0).
fn dominant_angle(geometry: &Geometry) -> Option<f64> {
    let mut best_len_sq = 0.0_f64;
    let mut best_angle = 0.0_f64;
    for sp in &geometry.subpaths {
        let n = sp.nodes.len();
        if n < 2 {
            continue;
        }
        let edge_count = if sp.closed { n } else { n - 1 };
        for i in 0..edge_count {
            let from = &sp.nodes[i];
            let to = &sp.nodes[(i + 1) % n];
            let dx = f64::from(to.x - from.x) / UNITS_PER_PX;
            let dy = f64::from(to.y - from.y) / UNITS_PER_PX;
            let len_sq = dx * dx + dy * dy;
            if len_sq > best_len_sq {
                best_len_sq = len_sq;
                best_angle = dy.atan2(dx);
            }
        }
    }
    if best_len_sq.sqrt() < MIN_DOMINANT_EDGE_PX {
        return None;
    }
    Some(fold_to_quadrant(best_angle))
}

/// Fold an angle into `[0, π/2)`: a rect edge is the same dominant axis whether it
/// reads 0, 90, 180, or 270 degrees, so canonicalize to the smallest CCW rotation.
fn fold_to_quadrant(angle: f64) -> f64 {
    let quarter = std::f64::consts::FRAC_PI_2;
    let mut a = angle % quarter;
    if a < 0.0 {
        a += quarter;
    }
    a
}

/// Centroid (object-local px) of every node across every subpath. `None` when the
/// geometry has no nodes. Accumulated in i64 to stay width-agnostic.
fn centroid_px(geometry: &Geometry) -> Option<(f64, f64)> {
    let mut sum_x: i64 = 0;
    let mut sum_y: i64 = 0;
    let mut count: i64 = 0;
    for sp in &geometry.subpaths {
        for node in &sp.nodes {
            sum_x += i64::from(node.x);
            sum_y += i64::from(node.y);
            count += 1;
        }
    }
    if count == 0 {
        return None;
    }
    let cx = (sum_x as f64 / count as f64) / UNITS_PER_PX;
    let cy = (sum_y as f64 / count as f64) / UNITS_PER_PX;
    Some((cx, cy))
}

/// Rotate a quantized node `(x, y)` about the px pivot `(cx, cy)` by the rotation
/// `(cos, sin)`, returning requantized integer units.
fn rotate_point_q(x: i32, y: i32, cx: f64, cy: f64, cos: f64, sin: f64) -> (i32, i32) {
    let px = f64::from(x) / UNITS_PER_PX - cx;
    let py = f64::from(y) / UNITS_PER_PX - cy;
    let rx = cos * px - sin * py + cx;
    let ry = sin * px + cos * py + cy;
    (quantize(rx), quantize(ry))
}

/// Rotate a node-relative bezier handle offset (quantized) by `(cos, sin)`.
fn rotate_handle(
    h: crate::object::model::HandlePoint,
    cos: f64,
    sin: f64,
) -> crate::object::model::HandlePoint {
    let dx = f64::from(h.dx) / UNITS_PER_PX;
    let dy = f64::from(h.dy) / UNITS_PER_PX;
    crate::object::model::HandlePoint {
        dx: quantize(cos * dx - sin * dy),
        dy: quantize(sin * dx + cos * dy),
    }
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "quantize to integer object-local units: .round() then narrow, the canonical de/quantize semantic shared with world_to_local_quantized"
)]
fn quantize(px: f64) -> i32 {
    quantize_units(px, UNITS_PER_PX)
}

/// The single `round(px * scale)` quantization discipline the cores own, exposed
/// so an inspector px edit (spacing/stroke-width/font-size) re-quantizes to the
/// stored i32 through the core rather than a shell-side `Math.round`, keeping the
/// rounding rule single-sourced. `scale` is the control's `unit_scale`.
#[allow(
    clippy::cast_possible_truncation,
    reason = "round() then narrow: the canonical quantization discipline shared with world_to_local_quantized"
)]
pub fn quantize_units(px: f64, scale: f64) -> i32 {
    (px * scale).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::model::{FillRule, PathNode};
    use std::f64::consts::PI;

    fn approx(a: f64, b: f64, eps: f64) -> bool {
        (a - b).abs() < eps
    }

    fn matrices_close(x: &Transform3x3, y: &Transform3x3, eps: f64) -> bool {
        for r in 0..3 {
            for c in 0..3 {
                if !approx(x.m[r][c], y.m[r][c], eps) {
                    return false;
                }
            }
        }
        true
    }

    #[test]
    fn decompose_identity() {
        let d = decompose_affine(&Transform3x3::IDENTITY);
        assert!(approx(d.rotation_rad, 0.0, 1e-12));
        assert!(approx(d.scale.0, 1.0, 1e-12));
        assert!(approx(d.scale.1, 1.0, 1e-12));
        assert!(approx(d.skew_rad, 0.0, 1e-12));
        assert_eq!(d.translate, (0.0, 0.0));
    }

    #[test]
    fn decompose_pure_translate() {
        let d = decompose_affine(&Transform3x3::translate(12.0, -7.5));
        assert_eq!(d.translate, (12.0, -7.5));
        assert!(approx(d.rotation_rad, 0.0, 1e-12));
        assert!(approx(d.scale.0, 1.0, 1e-12));
    }

    #[test]
    fn compose_decompose_round_trips_translate_rotate_scale() {
        // Several translate+rotate+scale matrices (skew 0) must round-trip within
        // 1e-9, including a non-uniform scale.
        let cases = [
            ((10.0, 20.0), 0.4, (2.0, 3.0)),
            ((-5.0, 8.0), -1.1, (1.5, 0.5)),
            ((0.0, 0.0), PI / 3.0, (4.0, 4.0)),
            ((100.0, -100.0), 2.5, (0.25, 2.0)),
        ];
        for (translate, rotation, scale) in cases {
            let m = compose_affine(translate, rotation, scale, 0.0);
            let d = decompose_affine(&m);
            let m2 = compose_affine(d.translate, d.rotation_rad, d.scale, d.skew_rad);
            assert!(
                matrices_close(&m, &m2, 1e-9),
                "round-trip drifted for {translate:?} {rotation} {scale:?}: {m:?} vs {m2:?}"
            );
            assert!(approx(d.translate.0, translate.0, 1e-9));
            assert!(approx(d.translate.1, translate.1, 1e-9));
            assert!(approx(d.scale.0, scale.0, 1e-9));
            assert!(approx(d.scale.1, scale.1, 1e-9));
        }
    }

    #[test]
    fn decompose_recovers_rotation_angle() {
        // A pure +30deg rotation reads back +30deg, unit scale, no skew.
        let m = compose_affine((0.0, 0.0), PI / 6.0, (1.0, 1.0), 0.0);
        let d = decompose_affine(&m);
        assert!(approx(d.rotation_rad, PI / 6.0, 1e-9), "rotation {}", d.rotation_rad);
        assert!(approx(d.scale.0, 1.0, 1e-9));
        assert!(approx(d.scale.1, 1.0, 1e-9));
        assert!(approx(d.skew_rad, 0.0, 1e-9));
    }

    fn rect_q(x0: i32, y0: i32, x1: i32, y1: i32) -> Geometry {
        Geometry::from_subpaths(
            vec![SubPath {
                closed: true,
                nodes: vec![
                    PathNode::corner(x0, y0),
                    PathNode::corner(x1, y0),
                    PathNode::corner(x1, y1),
                    PathNode::corner(x0, y1),
                ],
            }],
            FillRule::EvenOdd,
        )
    }

    #[test]
    fn set_transform_field_rotation_takes_degrees_in_core() {
        // The inspector displays degrees; the matrix stores radians. The deg->rad
        // conversion lives HERE, not in the shell: 90 (deg) reads back PI/2 rad.
        let t = set_transform_field(&Transform3x3::IDENTITY, "rotation", 90.0);
        let d = decompose_affine(&t);
        assert!(approx(d.rotation_rad, PI / 2.0, 1e-9), "rotation {}", d.rotation_rad);
        // rotation-flow shares the degrees contract.
        let tf = set_transform_field(&Transform3x3::IDENTITY, "rotation-flow", 90.0);
        assert!(approx(decompose_affine(&tf).rotation_rad, PI / 2.0, 1e-9));
    }

    #[test]
    fn set_transform_field_translate_is_verbatim_px() {
        let t = set_transform_field(&Transform3x3::IDENTITY, "x", 120.0);
        assert_eq!(decompose_affine(&t).translate.0, 120.0);
        let t = set_transform_field(&t, "y", -7.5);
        assert_eq!(decompose_affine(&t).translate, (120.0, -7.5));
        // An unrecognized field leaves the transform unchanged.
        assert_eq!(set_transform_field(&t, "width", 50.0), t);
    }

    #[test]
    fn quantize_units_rounds_px_times_scale() {
        // The single round(px * scale) discipline: 6 px at Q=8 => 48 units; .5 rounds.
        assert_eq!(quantize_units(6.0, 8.0), 48);
        assert_eq!(quantize_units(2.4, 1.0), 2);
        assert_eq!(quantize_units(2.5, 1.0), 3);
    }

    #[test]
    fn axis_aligned_rect_canonicalizes_to_noop() {
        let g = rect_q(0, 0, 80, 40);
        assert!(
            canonicalize_orientation(&Transform3x3::IDENTITY, &g).is_none(),
            "axis-aligned geometry is a no-op"
        );
    }

    #[test]
    fn rotated_geometry_extracts_angle_and_axis_aligns() {
        // Build a rect's nodes pre-rotated ~20deg in object-local space, identity
        // transform. canonicalize must pull ~20deg into the transform and leave an
        // axis-aligned geometry, while preserving each node's world point.
        let angle = 20.0_f64.to_radians();
        let (cos, sin) = (angle.cos(), angle.sin());
        let base = [(0.0, 0.0), (60.0, 0.0), (60.0, 30.0), (0.0, 30.0)];
        let nodes: Vec<PathNode> = base
            .iter()
            .map(|(x, y)| {
                let rx = cos * x - sin * y;
                let ry = sin * x + cos * y;
                PathNode::corner(quantize(rx), quantize(ry))
            })
            .collect();
        let g = Geometry::from_subpaths(vec![SubPath { closed: true, nodes }], FillRule::EvenOdd);

        let (t, ng) =
            canonicalize_orientation(&Transform3x3::IDENTITY, &g).expect("rotated geometry changes");
        let d = decompose_affine(&t);
        assert!(
            approx(d.rotation_rad.abs(), angle, 2e-2),
            "extracted rotation {} expected ~{angle}",
            d.rotation_rad
        );

        // The rewritten geometry is axis-aligned: every edge is horizontal or
        // vertical within a quantization tolerance.
        let sp = &ng.subpaths[0];
        for i in 0..sp.nodes.len() {
            let a = &sp.nodes[i];
            let b = &sp.nodes[(i + 1) % sp.nodes.len()];
            let dx = (a.x - b.x).abs();
            let dy = (a.y - b.y).abs();
            assert!(dx <= 1 || dy <= 1, "edge {i} not axis-aligned: d=({dx},{dy})");
        }

        // World preservation: each rotated geometry node, carried through the new
        // transform, lands back near the original node's world point.
        let mut gg = ng.clone();
        gg.ensure_parsed().expect("parse");
        for (orig, can) in g.subpaths[0].nodes.iter().zip(&gg.subpaths[0].nodes) {
            let (ox, oy) = (f64::from(orig.x) / UNITS_PER_PX, f64::from(orig.y) / UNITS_PER_PX);
            let (cxp, cyp) = (f64::from(can.x) / UNITS_PER_PX, f64::from(can.y) / UNITS_PER_PX);
            let (wx, wy) = t.apply_point(cxp, cyp);
            assert!(approx(wx, ox, 0.5), "world x {wx} != {ox}");
            assert!(approx(wy, oy, 0.5), "world y {wy} != {oy}");
        }
    }
}
