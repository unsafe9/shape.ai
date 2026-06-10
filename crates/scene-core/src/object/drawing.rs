//! OB3.D1/D2 — freehand drawing capture primitives (pure geometry).
//!
//! Pen-and-ink capture math in the Rust core: RDP simplification, tangent-
//! estimated bezier fitting, the pen [`Brush`], and the partial-erase subpath
//! split. The pen-up COMMIT lives in [`super::recognize`] (anchor-semantics v3
//! §4): one stroke = one recognized object, which replaced the D12/D13 drawing
//! session (a multi-stroke span committing to a single multi-subpath object).
//!
//! Pipeline per stroke:
//!   raw points (transient, world px) --pen-up--> recognition
//!   ([`super::recognize::recognize_stroke`], which reuses [`rdp_simplify`] +
//!   [`fit_beziers`] for its silhouette-preserving fallback) --> one committed
//!   [`super::model::Object`].
//!
//! Conventions (CLAUDE.md): pure (no time/rng/IO — all inputs passed in, the
//! commit id/order are caller-supplied); pointer-width-agnostic (coords are i32
//! @ [`GEOMETRY_QUANTUM_PER_PX`] units/px, no `usize` in data); no lossy `as`
//! width casts (the one f64 -> i32 quantize step clamps into i32 range first,
//! making the narrowing provably safe under a scoped `#[allow]`).

use super::model::{
    Geometry, HandlePoint, LineCap, LineJoin, Paint, PathNode, Stroke, SubPath,
    GEOMETRY_QUANTUM_PER_PX,
};

/// World-px -> quantized i32 (round to nearest, clamp into i32 range). The clamp
/// makes the final narrowing safe: `value` is bounded to `[i32::MIN, i32::MAX]`
/// as f64 before the cast, so no truncation/wrap can occur. NaN maps to 0.
pub(crate) fn quantize_px(px: f64) -> i32 {
    if px.is_nan() {
        return 0;
    }
    let units = (px * f64::from(GEOMETRY_QUANTUM_PER_PX)).round();
    let clamped = units.clamp(f64::from(i32::MIN), f64::from(i32::MAX));
    #[allow(
        clippy::cast_possible_truncation,
        reason = "clamped to [i32::MIN, i32::MAX] above; the rounded f64 is an exact integer in range"
    )]
    let q = clamped as i32;
    q
}

// ---------------------------------------------------------------------------
// RDP polyline simplification (D1).
// ---------------------------------------------------------------------------

/// Perpendicular distance from `p` to the infinite line through `a`..`b`. If
/// `a == b` the "line" degenerates to a point, so this is the point distance.
pub(crate) fn perpendicular_distance(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (px, py) = p;
    let (ax, ay) = a;
    let (bx, by) = b;
    let dx = bx - ax;
    let dy = by - ay;
    let seg_len = dx.hypot(dy);
    if seg_len <= f64::EPSILON {
        return (px - ax).hypot(py - ay);
    }
    // |cross((b-a),(p-a))| / |b-a|.
    let cross = (dx * (py - ay) - dy * (px - ax)).abs();
    cross / seg_len
}

/// Ramer-Douglas-Peucker polyline simplification (recursive, perpendicular
/// distance). Drops interior points that lie within `epsilon` of the chord
/// between the kept endpoints; endpoints are always preserved. `epsilon <= 0`
/// keeps every point. Inputs/outputs in the same units (world px).
pub fn rdp_simplify(points: &[(f64, f64)], epsilon: f64) -> Vec<(f64, f64)> {
    if points.len() <= 2 {
        return points.to_vec();
    }
    let mut keep = vec![false; points.len()];
    let last = points.len() - 1;
    keep[0] = true;
    keep[last] = true;
    rdp_mark(points, 0, last, epsilon, &mut keep);
    points
        .iter()
        .zip(keep.iter())
        .filter_map(|(p, &k)| k.then_some(*p))
        .collect()
}

/// Recursively mark the point of maximum deviation on `points[start..=end]`
/// when it exceeds `epsilon`, then recurse into the two halves.
fn rdp_mark(points: &[(f64, f64)], start: usize, end: usize, epsilon: f64, keep: &mut [bool]) {
    if end <= start + 1 {
        return;
    }
    let a = points[start];
    let b = points[end];
    let mut max_dist = 0.0_f64;
    let mut max_idx = start;
    for (offset, &p) in points[start + 1..end].iter().enumerate() {
        let d = perpendicular_distance(p, a, b);
        if d > max_dist {
            max_dist = d;
            max_idx = start + 1 + offset;
        }
    }
    if max_dist > epsilon {
        keep[max_idx] = true;
        rdp_mark(points, start, max_idx, epsilon, keep);
        rdp_mark(points, max_idx, end, epsilon, keep);
    }
}

// ---------------------------------------------------------------------------
// Bezier fitting via tangent estimation (D1/D2).
// ---------------------------------------------------------------------------

/// Catmull-Rom tension: the fraction of the neighbor span used as the handle
/// length. 1/6 reproduces the standard Catmull-Rom -> cubic-Bezier conversion
/// (handle = (p[i+1] - p[i-1]) / 6).
const CATMULL_ROM_TENSION: f64 = 1.0 / 6.0;

/// Convert a (simplified) polyline to [`PathNode`]s with cubic-bezier handles
/// estimated from local tangents. For interior node `i` the tangent is the
/// Catmull-Rom direction `p[i+1] - p[i-1]`: `out_handle` reaches toward the next
/// node and `in_handle` is its mirror toward the previous node, each scaled by
/// [`CATMULL_ROM_TENSION`]. Endpoints get a one-sided tangent toward their only
/// neighbor. Node positions are quantized to i32 (D2); **endpoints are preserved
/// exactly** (their quantized position is the quantization of the input point,
/// untouched by fitting). A 0/1/2-point input yields plain corner nodes (no
/// handles) since there is no interior to curve.
pub fn fit_beziers(points: &[(f64, f64)]) -> Vec<PathNode> {
    if points.len() <= 2 {
        return points.iter().map(|&(x, y)| PathNode::corner(quantize_px(x), quantize_px(y))).collect();
    }
    let last = points.len() - 1;
    let mut nodes = Vec::with_capacity(points.len());
    for (i, &(x, y)) in points.iter().enumerate() {
        let qx = quantize_px(x);
        let qy = quantize_px(y);
        let (out_handle, in_handle) = if i == 0 {
            // First endpoint: one-sided tangent toward the next node.
            let (nx, ny) = points[1];
            let out = HandlePoint {
                dx: quantize_px((nx - x) * CATMULL_ROM_TENSION),
                dy: quantize_px((ny - y) * CATMULL_ROM_TENSION),
            };
            (Some(out), None)
        } else if i == last {
            // Last endpoint: one-sided tangent toward the previous node.
            let (prx, pry) = points[i - 1];
            let in_h = HandlePoint {
                dx: quantize_px((prx - x) * CATMULL_ROM_TENSION),
                dy: quantize_px((pry - y) * CATMULL_ROM_TENSION),
            };
            (None, Some(in_h))
        } else {
            // Interior: tangent direction is p[i+1] - p[i-1].
            let (prx, pry) = points[i - 1];
            let (nx, ny) = points[i + 1];
            let tx = (nx - prx) * CATMULL_ROM_TENSION;
            let ty = (ny - pry) * CATMULL_ROM_TENSION;
            let out = HandlePoint { dx: quantize_px(tx), dy: quantize_px(ty) };
            let in_h = HandlePoint { dx: quantize_px(-tx), dy: quantize_px(-ty) };
            (Some(out), Some(in_h))
        };
        nodes.push(PathNode { x: qx, y: qy, in_handle, out_handle, width: None });
    }
    nodes
}

// ---------------------------------------------------------------------------
// Partial erase — subpath split/cut at a touched point (D4, W2-08).
// ---------------------------------------------------------------------------

/// Cut a stroke's geometry at a touched point: the node nearest `(x, y)` within
/// `radius` (all in object-local quantized units) is removed, splitting its
/// subpath into two open subpaths (`[0..i]` and `[i+1..]`). A produced piece
/// with fewer than 2 nodes has no extent and is dropped, so erasing the only/
/// final segment can leave the object empty (the caller deletes it then). Other
/// subpaths pass through untouched.
///
/// This is a SIMPLE split — it removes the closest node, not a full geometric
/// boolean. Returns `None` when no node lies within `radius` (the touch missed
/// every node, so there is nothing to cut). Pure: no IO/time/rng.
pub fn split_subpath_at(geometry: &Geometry, x: i32, y: i32, radius: i32) -> Option<Geometry> {
    let radius_sq = i64::from(radius) * i64::from(radius);
    let mut best: Option<(usize, usize, i64)> = None; // (subpath, node, dist_sq)
    for (si, sp) in geometry.subpaths.iter().enumerate() {
        for (ni, node) in sp.nodes.iter().enumerate() {
            let dx = i64::from(node.x) - i64::from(x);
            let dy = i64::from(node.y) - i64::from(y);
            let dist_sq = dx * dx + dy * dy;
            if dist_sq <= radius_sq && best.is_none_or(|(_, _, b)| dist_sq < b) {
                best = Some((si, ni, dist_sq));
            }
        }
    }
    let (target_si, target_ni, _) = best?;

    let mut out: Vec<SubPath> = Vec::with_capacity(geometry.subpaths.len() + 1);
    for (si, sp) in geometry.subpaths.iter().enumerate() {
        if si != target_si {
            out.push(sp.clone());
            continue;
        }
        // Split this subpath around the removed node, dropping degenerate pieces.
        // A closed subpath opens once it is cut (the cut breaks the loop).
        let left = &sp.nodes[..target_ni];
        let right = &sp.nodes[target_ni + 1..];
        if left.len() >= 2 {
            out.push(open_subpath(left));
        }
        if right.len() >= 2 {
            out.push(open_subpath(right));
        }
    }
    Some(Geometry::from_subpaths(out, geometry.fill_rule))
}

/// An open subpath from a node slice. The cut endpoints lose the dangling handle
/// that pointed at the removed node so the open ends render cleanly (the first
/// node drops its in-handle, the last drops its out-handle).
fn open_subpath(nodes: &[PathNode]) -> SubPath {
    let mut nodes = nodes.to_vec();
    if let Some(first) = nodes.first_mut() {
        first.in_handle = None;
    }
    if let Some(last) = nodes.last_mut() {
        last.out_handle = None;
    }
    SubPath { closed: false, nodes }
}

// ---------------------------------------------------------------------------
// Brush + pressure (D2 per-node width slot).
// ---------------------------------------------------------------------------

/// A pen brush: stroke color, base width in logical px, and a dash pattern in
/// logical px (empty => solid). The brush is the session's stroke style; on
/// commit it lowers to a [`Stroke`] (width + dash converted to quantized units).
#[derive(Clone, Debug, PartialEq)]
pub struct Brush {
    pub color: String,
    pub width_px: f64,
    pub dash: Vec<i32>,
}

impl Brush {
    /// A solid brush with no dash.
    pub fn new(color: impl Into<String>, width_px: f64) -> Self {
        Brush { color: color.into(), width_px, dash: Vec::new() }
    }

    /// Lower the brush to a [`Stroke`] (D2): solid paint, width + dash in
    /// quantized units, round cap/join (freehand ink reads better round). The
    /// per-node `width` slot stays open for pressure data (D2/D13).
    pub(crate) fn to_stroke(&self) -> Stroke {
        Stroke {
            paint: Paint::Solid { color: self.color.clone() },
            width: quantize_px(self.width_px),
            opacity: 1.0,
            dash: self.dash.iter().map(|&px| px * GEOMETRY_QUANTUM_PER_PX).collect(),
            cap: LineCap::Round,
            join: LineJoin::Round,
        }
    }
}

/// Map a normalized pen pressure (`0.0..=1.0`) to a per-node stroke width in
/// quantized units (D2 `PathNode.width` slot). Linear in `[0,1]`: pressure 0 ->
/// 0 width, pressure 1 -> `base_width_px`. Out-of-range pressure is clamped.
pub fn pressure_to_width(pressure: f64, base_width_px: f64) -> i32 {
    let p = pressure.clamp(0.0, 1.0);
    quantize_px(p * base_width_px)
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::model::FillRule;

    #[test]
    fn rdp_drops_collinear_midpoint() {
        // Three exactly-collinear points: the midpoint deviates by 0, so RDP
        // drops it, leaving only the two endpoints.
        let pts = [(0.0, 0.0), (5.0, 0.0), (10.0, 0.0)];
        let out = rdp_simplify(&pts, 0.5);
        assert_eq!(out, vec![(0.0, 0.0), (10.0, 0.0)]);
    }

    #[test]
    fn rdp_keeps_deviating_midpoint() {
        // A midpoint well off the chord is kept.
        let pts = [(0.0, 0.0), (5.0, 5.0), (10.0, 0.0)];
        let out = rdp_simplify(&pts, 0.5);
        assert_eq!(out, vec![(0.0, 0.0), (5.0, 5.0), (10.0, 0.0)]);
    }

    #[test]
    fn rdp_passthrough_two_points() {
        let pts = [(0.0, 0.0), (10.0, 10.0)];
        assert_eq!(rdp_simplify(&pts, 1.0), pts.to_vec());
    }

    #[test]
    fn fit_beziers_preserves_endpoints() {
        // Endpoints quantize to the exact quantization of the input points and
        // are not displaced by fitting. (×8: 0->0, 10->80.)
        let pts = [(0.0, 0.0), (5.0, 10.0), (10.0, 0.0)];
        let nodes = fit_beziers(&pts);
        assert_eq!(nodes.len(), 3);
        assert_eq!((nodes[0].x, nodes[0].y), (0, 0));
        assert_eq!((nodes[2].x, nodes[2].y), (80, 0));
        // First node has only an out-handle, last only an in-handle (one-sided).
        assert!(nodes[0].in_handle.is_none() && nodes[0].out_handle.is_some());
        assert!(nodes[2].in_handle.is_some() && nodes[2].out_handle.is_none());
        // Interior node carries mirrored handles.
        let interior = nodes[1];
        let out = interior.out_handle.unwrap();
        let in_h = interior.in_handle.unwrap();
        assert_eq!(in_h.dx, -out.dx);
        assert_eq!(in_h.dy, -out.dy);
    }

    #[test]
    fn fit_beziers_short_inputs_are_corners() {
        assert!(fit_beziers(&[]).is_empty());
        let one = fit_beziers(&[(1.0, 2.0)]);
        assert_eq!(one, vec![PathNode::corner(8, 16)]);
        let two = fit_beziers(&[(0.0, 0.0), (1.0, 1.0)]);
        assert_eq!(two, vec![PathNode::corner(0, 0), PathNode::corner(8, 8)]);
    }

    #[test]
    fn brush_lowers_to_a_round_quantized_stroke() {
        let stroke = Brush::new("#112233", 2.0).to_stroke();
        assert_eq!(stroke.paint, Paint::Solid { color: "#112233".into() });
        assert_eq!(stroke.width, 16); // 2px * 8 units/px
        assert_eq!(stroke.cap, LineCap::Round);
        assert_eq!(stroke.join, LineJoin::Round);
        assert!(stroke.dash.is_empty());
    }

    #[test]
    fn pressure_to_width_scales_linearly() {
        // base 10px -> 80 quantized units at full pressure; half at 0.5.
        assert_eq!(pressure_to_width(1.0, 10.0), 80);
        assert_eq!(pressure_to_width(0.5, 10.0), 40);
        assert_eq!(pressure_to_width(0.0, 10.0), 0);
        // Out-of-range pressure clamps.
        assert_eq!(pressure_to_width(2.0, 10.0), 80);
        assert_eq!(pressure_to_width(-1.0, 10.0), 0);
    }

    #[test]
    fn split_subpath_cuts_at_nearest_node_into_two_open_pieces() {
        // A 5-node polyline; cutting at the middle node (index 2) drops it and
        // leaves two open pieces: nodes [0,1] and [3,4].
        let geometry = Geometry::from_subpaths(
            vec![SubPath {
                closed: false,
                nodes: vec![
                    PathNode::corner(0, 0),
                    PathNode::corner(10, 0),
                    PathNode::corner(20, 0),
                    PathNode::corner(30, 0),
                    PathNode::corner(40, 0),
                ],
            }],
            FillRule::NonZero,
        );
        let cut = split_subpath_at(&geometry, 21, 1, 8).expect("a node is within radius");
        assert_eq!(cut.subpaths.len(), 2);
        assert!(cut.subpaths.iter().all(|sp| !sp.closed), "pieces are open");
        assert_eq!(cut.subpaths[0].nodes.len(), 2);
        assert_eq!(cut.subpaths[1].nodes.len(), 2);
        assert_eq!((cut.subpaths[0].nodes[0].x, cut.subpaths[0].nodes[1].x), (0, 10));
        assert_eq!((cut.subpaths[1].nodes[0].x, cut.subpaths[1].nodes[1].x), (30, 40));
    }

    #[test]
    fn split_subpath_drops_degenerate_endpoint_pieces() {
        // Cutting the first node leaves no left piece and a 2-node right piece.
        let geometry = Geometry::from_subpaths(
            vec![SubPath {
                closed: false,
                nodes: vec![PathNode::corner(0, 0), PathNode::corner(10, 0), PathNode::corner(20, 0)],
            }],
            FillRule::NonZero,
        );
        let cut = split_subpath_at(&geometry, 0, 0, 8).expect("the first node is within radius");
        assert_eq!(cut.subpaths.len(), 1);
        assert_eq!(cut.subpaths[0].nodes.len(), 2);
        assert_eq!(cut.subpaths[0].nodes[0].x, 10);
    }

    #[test]
    fn split_subpath_returns_none_when_touch_misses_all_nodes() {
        let geometry = Geometry::from_subpaths(
            vec![SubPath {
                closed: false,
                nodes: vec![PathNode::corner(0, 0), PathNode::corner(10, 0)],
            }],
            FillRule::NonZero,
        );
        assert!(split_subpath_at(&geometry, 500, 500, 8).is_none());
    }

    #[test]
    fn split_subpath_leaves_other_subpaths_untouched() {
        // A two-subpath object; cutting the middle node of the second subpath
        // leaves the first whole. The second's two flanks are each a single node
        // (degenerate), so they drop — only the untouched first subpath remains.
        let geometry = Geometry::from_subpaths(
            vec![
                SubPath {
                    closed: false,
                    nodes: vec![PathNode::corner(0, 0), PathNode::corner(10, 0)],
                },
                SubPath {
                    closed: false,
                    nodes: vec![
                        PathNode::corner(0, 50),
                        PathNode::corner(10, 50),
                        PathNode::corner(20, 50),
                    ],
                },
            ],
            FillRule::NonZero,
        );
        let cut = split_subpath_at(&geometry, 10, 50, 8).expect("middle node within radius");
        assert_eq!(cut.subpaths.len(), 1);
        assert_eq!(cut.subpaths[0].nodes.len(), 2);
        assert_eq!((cut.subpaths[0].nodes[0].y, cut.subpaths[0].nodes[1].y), (0, 0));
    }

    #[test]
    fn quantize_rounds_to_nearest() {
        assert_eq!(quantize_px(1.0), 8);
        assert_eq!(quantize_px(0.5), 4);
        // 0.1px * 8 = 0.8 -> rounds to 1.
        assert_eq!(quantize_px(0.1), 1);
        assert_eq!(quantize_px(-1.0), -8);
        assert_eq!(quantize_px(f64::NAN), 0);
    }
}
