//! Freehand drawing capture primitives (pure geometry): RDP simplification,
//! tangent-estimated bezier fitting, the pen [`Brush`], and the partial-erase
//! subpath split. The pen-up commit lives in [`crate::object::recognize`].
//!
//! Pure (no time/rng/IO — commit id/order are caller-supplied). Coords are i32 @
//! [`GEOMETRY_QUANTUM_PER_PX`] units/px; the one f64 -> i32 quantize clamps into
//! i32 range first, making the narrowing provably safe.

use crate::object::model::{
    Geometry, HandlePoint, LineCap, LineJoin, Paint, PathNode, Stroke, SubPath,
    GEOMETRY_QUANTUM_PER_PX,
};

/// World-px -> quantized i32 (round to nearest, clamp into i32 range so the
/// narrowing cannot truncate/wrap). NaN maps to 0.
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

/// Perpendicular distance from `p` to the infinite line through `a`..`b`; the
/// point distance when `a == b`.
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

/// Ramer-Douglas-Peucker simplification. Drops interior points within `epsilon`
/// of the chord; endpoints always preserved. `epsilon <= 0` keeps every point.
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

/// Catmull-Rom tension: 1/6 reproduces the standard Catmull-Rom -> cubic-Bezier
/// conversion (handle = (p[i+1] - p[i-1]) / 6).
const CATMULL_ROM_TENSION: f64 = 1.0 / 6.0;

/// Convert a polyline to [`PathNode`]s with cubic-bezier handles from local
/// tangents. Interior tangent is the Catmull-Rom direction `p[i+1] - p[i-1]`;
/// endpoints get a one-sided tangent. Positions quantized to i32, endpoints
/// preserved exactly. A 0/1/2-point input yields plain corner nodes.
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
            let (nx, ny) = points[1];
            let out = HandlePoint {
                dx: quantize_px((nx - x) * CATMULL_ROM_TENSION),
                dy: quantize_px((ny - y) * CATMULL_ROM_TENSION),
            };
            (Some(out), None)
        } else if i == last {
            let (prx, pry) = points[i - 1];
            let in_h = HandlePoint {
                dx: quantize_px((prx - x) * CATMULL_ROM_TENSION),
                dy: quantize_px((pry - y) * CATMULL_ROM_TENSION),
            };
            (None, Some(in_h))
        } else {
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

/// Cut a stroke's geometry at a touched point: the node nearest `(x, y)` within
/// `radius` (object-local quantized units) is removed, splitting its subpath into
/// two open pieces. A piece with fewer than 2 nodes is dropped, so erasing the
/// final segment can leave the object empty. `None` when no node is within
/// `radius`. A simple node removal, not a full geometric boolean.
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
        // A closed subpath opens once cut (the cut breaks the loop).
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

/// An open subpath from a node slice; the cut ends drop the handle that pointed
/// at the removed node so they render cleanly.
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

/// A pen brush: stroke color, base width and dash in logical px (empty dash =>
/// solid). On commit it lowers to a [`Stroke`] (quantized units).
#[derive(Clone, Debug, PartialEq)]
pub struct Brush {
    pub color: String,
    pub width_px: f64,
    pub dash: Vec<i32>,
}

impl Brush {
    pub fn new(color: impl Into<String>, width_px: f64) -> Self {
        Brush { color: color.into(), width_px, dash: Vec::new() }
    }

    /// Solid paint, quantized width + dash, round cap/join (freehand ink reads
    /// better round).
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

/// Normalized pen pressure (`0.0..=1.0`) to a per-node stroke width in quantized
/// units. Linear: 0 -> 0, 1 -> `base_width_px`. Out-of-range pressure is clamped.
pub fn pressure_to_width(pressure: f64, base_width_px: f64) -> i32 {
    let p = pressure.clamp(0.0, 1.0);
    quantize_px(p * base_width_px)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::model::FillRule;

    #[test]
    fn rdp_drops_collinear_midpoint() {
        let pts = [(0.0, 0.0), (5.0, 0.0), (10.0, 0.0)];
        let out = rdp_simplify(&pts, 0.5);
        assert_eq!(out, vec![(0.0, 0.0), (10.0, 0.0)]);
    }

    #[test]
    fn rdp_keeps_deviating_midpoint() {
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
        let pts = [(0.0, 0.0), (5.0, 10.0), (10.0, 0.0)];
        let nodes = fit_beziers(&pts);
        assert_eq!(nodes.len(), 3);
        assert_eq!((nodes[0].x, nodes[0].y), (0, 0));
        assert_eq!((nodes[2].x, nodes[2].y), (80, 0));
        assert!(nodes[0].in_handle.is_none() && nodes[0].out_handle.is_some());
        assert!(nodes[2].in_handle.is_some() && nodes[2].out_handle.is_none());
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
        assert_eq!(stroke.width, 16);
        assert_eq!(stroke.cap, LineCap::Round);
        assert_eq!(stroke.join, LineJoin::Round);
        assert!(stroke.dash.is_empty());
    }

    #[test]
    fn pressure_to_width_scales_linearly() {
        assert_eq!(pressure_to_width(1.0, 10.0), 80);
        assert_eq!(pressure_to_width(0.5, 10.0), 40);
        assert_eq!(pressure_to_width(0.0, 10.0), 0);
        assert_eq!(pressure_to_width(2.0, 10.0), 80);
        assert_eq!(pressure_to_width(-1.0, 10.0), 0);
    }

    #[test]
    fn split_subpath_cuts_at_nearest_node_into_two_open_pieces() {
        // Cutting the middle node (index 2) leaves two open pieces: [0,1] and [3,4].
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
        // Cutting the middle node of the second subpath leaves the first whole; the
        // second's two single-node flanks drop.
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
        assert_eq!(quantize_px(0.1), 1);
        assert_eq!(quantize_px(-1.0), -8);
        assert_eq!(quantize_px(f64::NAN), 0);
    }
}
