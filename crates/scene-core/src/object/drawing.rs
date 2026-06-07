//! OB3.D1/D2 — freehand drawing capture -> object (pure geometry).
//!
//! Pen-and-ink capture in the Rust core. A *drawing session* (D12/D13) is the
//! span from the first pen-down until the caller commits it: every stroke drawn
//! during that span (pen-down -> pen-up) becomes one open [`SubPath`], and the
//! whole session commits to a single [`Object`] carrying those subpaths and one
//! stroke style. Granularity guessing is 0 here — a session is always one object;
//! splitting a sketch into multiple objects is a later, separate concern.
//!
//! Pipeline per stroke:
//!   raw points (transient, world px) --pen-up--> [`rdp_simplify`] (drop
//!   near-collinear samples) --> [`fit_beziers`] (tangent-estimated cubic
//!   handles) --> one open [`SubPath`] appended to the session.
//! On [`DrawingSession::commit`] the accumulated subpaths become a
//! [`Geometry`], the brush becomes a [`Stroke`], and the session origin becomes
//! the object's transform translate (P4 zero-rebake: geometry is object-local,
//! position lives in the transform).
//!
//! Conventions (CLAUDE.md): pure (no time/rng/IO — all inputs passed in, the
//! commit id/order are caller-supplied); pointer-width-agnostic (coords are i32
//! @ [`GEOMETRY_QUANTUM_PER_PX`] units/px, no `usize` in data); no lossy `as`
//! width casts (the one f64 -> i32 quantize step clamps into i32 range first,
//! making the narrowing provably safe under a scoped `#[allow]`).

use super::model::{
    FillRule, Geometry, HandlePoint, LineCap, LineJoin, Object, Paint, PathNode, Stroke, SubPath,
    Transform3x3, GEOMETRY_QUANTUM_PER_PX,
};

/// World-px -> quantized i32 (round to nearest, clamp into i32 range). The clamp
/// makes the final narrowing safe: `value` is bounded to `[i32::MIN, i32::MAX]`
/// as f64 before the cast, so no truncation/wrap can occur. NaN maps to 0.
fn quantize_px(px: f64) -> i32 {
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
fn perpendicular_distance(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
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
}

/// Map a normalized pen pressure (`0.0..=1.0`) to a per-node stroke width in
/// quantized units (D2 `PathNode.width` slot). Linear in `[0,1]`: pressure 0 ->
/// 0 width, pressure 1 -> `base_width_px`. Out-of-range pressure is clamped.
pub fn pressure_to_width(pressure: f64, base_width_px: f64) -> i32 {
    let p = pressure.clamp(0.0, 1.0);
    quantize_px(p * base_width_px)
}

// ---------------------------------------------------------------------------
// Drawing session (D12/D13).
// ---------------------------------------------------------------------------

/// Accumulates the strokes of one freehand drawing session into a set of open
/// subpaths, ready to commit as a single [`Object`]. Raw per-stroke points are
/// captured in **world px** (transient — held only until `end_stroke`); on
/// commit they are translated to object-local quantized coords relative to the
/// session origin (the object's transform carries the origin translate).
#[derive(Clone, Debug)]
pub struct DrawingSession {
    brush: Brush,
    /// Completed strokes, each an open subpath of fitted nodes (world-px coords,
    /// not yet origin-relative — the origin offset is applied at `commit`).
    subpaths: Vec<SubPath>,
    /// In-flight raw points for the stroke currently being drawn (world px).
    current: Option<Vec<(f64, f64)>>,
}

impl DrawingSession {
    /// Start a session with the given brush. No strokes yet.
    pub fn new(brush: Brush) -> Self {
        DrawingSession { brush, subpaths: Vec::new(), current: None }
    }

    /// Pen-down: open a new transient stroke buffer. A stroke already in
    /// progress is discarded (a fresh pen-down supersedes it).
    pub fn begin_stroke(&mut self) {
        self.current = Some(Vec::new());
    }

    /// Push a raw sample (world px) into the in-flight stroke. No-op if no
    /// stroke is open (no preceding `begin_stroke`).
    pub fn push_point(&mut self, x: f64, y: f64) {
        if let Some(buf) = self.current.as_mut() {
            buf.push((x, y));
        }
    }

    /// Pen-up: simplify (RDP at `epsilon`) and bezier-fit the in-flight raw
    /// points into one open [`SubPath`], appended to the session. A stroke with
    /// fewer than 2 points produces nothing (a single tap has no extent). No-op
    /// if no stroke is open.
    pub fn end_stroke(&mut self, epsilon: f64) {
        let Some(raw) = self.current.take() else {
            return;
        };
        if raw.len() < 2 {
            return;
        }
        let simplified = rdp_simplify(&raw, epsilon);
        let nodes = fit_beziers(&simplified);
        self.subpaths.push(SubPath { closed: false, nodes });
    }

    /// True when no committed strokes have accumulated (an in-flight stroke does
    /// not count until `end_stroke`).
    pub fn is_empty(&self) -> bool {
        self.subpaths.is_empty()
    }

    /// Commit the session to one [`Object`]: geometry from the accumulated open
    /// subpaths, translated so coords are object-local relative to
    /// `(origin_x, origin_y)` (world px); the brush lowers to a [`Stroke`]; the
    /// origin becomes the object's transform translate. `id`/`order` are
    /// caller-supplied (purity: no id/time generation here). The session's
    /// subpath coords are world-px-quantized, so the origin is subtracted in
    /// quantized units.
    pub fn commit(
        &self,
        id: String,
        order: String,
        origin_x: f64,
        origin_y: f64,
    ) -> Object {
        let ox = quantize_px(origin_x);
        let oy = quantize_px(origin_y);
        let local_subpaths: Vec<SubPath> = self
            .subpaths
            .iter()
            .map(|sp| SubPath {
                closed: sp.closed,
                nodes: sp
                    .nodes
                    .iter()
                    .map(|n| PathNode {
                        x: n.x - ox,
                        y: n.y - oy,
                        in_handle: n.in_handle,
                        out_handle: n.out_handle,
                        width: n.width,
                    })
                    .collect(),
            })
            .collect();

        let geometry = Geometry::from_subpaths(local_subpaths, FillRule::NonZero);
        let mut object = Object::new(id, order, geometry);
        object.transform = Transform3x3::translate(origin_x, origin_y);
        object.stroke = Some(self.brush_to_stroke());
        object
    }

    /// Lower the brush to a [`Stroke`] (D2): solid paint, width + dash in
    /// quantized units, round cap/join (freehand ink reads better round). The
    /// per-node `width` slot stays open for pressure data (D2/D13).
    fn brush_to_stroke(&self) -> Stroke {
        Stroke {
            paint: Paint::Solid { color: self.brush.color.clone() },
            width: quantize_px(self.brush.width_px),
            opacity: 1.0,
            dash: self.brush.dash.iter().map(|&px| px * GEOMETRY_QUANTUM_PER_PX).collect(),
            cap: LineCap::Round,
            join: LineJoin::Round,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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
    fn session_two_strokes_commit_to_object_with_two_open_subpaths_and_stroke() {
        let mut session = DrawingSession::new(Brush::new("#112233", 2.0));
        assert!(session.is_empty());

        // Stroke 1.
        session.begin_stroke();
        session.push_point(0.0, 0.0);
        session.push_point(5.0, 0.0);
        session.push_point(10.0, 0.0);
        session.end_stroke(0.5);

        // Stroke 2.
        session.begin_stroke();
        session.push_point(0.0, 20.0);
        session.push_point(5.0, 25.0);
        session.push_point(10.0, 20.0);
        session.end_stroke(0.5);

        assert!(!session.is_empty());

        let obj = session.commit("draw-1".into(), "a0".into(), 0.0, 0.0);
        assert_eq!(obj.id, "draw-1");
        assert_eq!(obj.geometry.subpaths.len(), 2);
        assert!(obj.geometry.subpaths.iter().all(|sp| !sp.closed));
        // The committed geometry has a stroke style derived from the brush.
        let stroke = obj.stroke.expect("commit sets a stroke");
        assert_eq!(stroke.paint, Paint::Solid { color: "#112233".into() });
        assert_eq!(stroke.width, 16); // 2px * 8 units/px
        assert_eq!(stroke.cap, LineCap::Round);
        // Identity origin => identity-ish translate transform.
        assert_eq!(obj.transform, Transform3x3::translate(0.0, 0.0));
    }

    #[test]
    fn commit_translates_coords_relative_to_origin() {
        // A stroke at world (100,100)->(110,100), committed at origin (100,100),
        // becomes object-local (0,0)->(80,0) with the origin in the transform.
        let mut session = DrawingSession::new(Brush::new("#000000", 1.0));
        session.begin_stroke();
        session.push_point(100.0, 100.0);
        session.push_point(105.0, 100.0);
        session.push_point(110.0, 100.0);
        session.end_stroke(0.5);

        let obj = session.commit("d".into(), "a0".into(), 100.0, 100.0);
        let sp = &obj.geometry.subpaths[0];
        assert_eq!((sp.nodes[0].x, sp.nodes[0].y), (0, 0));
        let last = sp.nodes.last().unwrap();
        assert_eq!((last.x, last.y), (80, 0));
        assert_eq!(obj.transform, Transform3x3::translate(100.0, 100.0));
    }

    #[test]
    fn end_stroke_ignores_degenerate_taps() {
        let mut session = DrawingSession::new(Brush::new("#000000", 1.0));
        // Pen-up with no begin: no-op.
        session.end_stroke(0.5);
        assert!(session.is_empty());
        // A single-point tap has no extent: no subpath.
        session.begin_stroke();
        session.push_point(3.0, 3.0);
        session.end_stroke(0.5);
        assert!(session.is_empty());
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
    fn quantize_rounds_to_nearest() {
        assert_eq!(quantize_px(1.0), 8);
        assert_eq!(quantize_px(0.5), 4);
        // 0.1px * 8 = 0.8 -> rounds to 1.
        assert_eq!(quantize_px(0.1), 1);
        assert_eq!(quantize_px(-1.0), -8);
        assert_eq!(quantize_px(f64::NAN), 0);
    }
}
