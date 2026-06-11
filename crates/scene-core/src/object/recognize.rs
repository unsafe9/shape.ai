//! Anchor-semantics v3 §4 — pen-up stroke recognition (freehand → shape input).
//!
//! Freehand is a SHAPE INPUT device, not ink: at pen-up the raw stroke is
//! converted to its nearest canonical form, and one stroke commits as ONE
//! object (replacing the D13 "session = one multi-subpath object" policy —
//! a recognized rect and a recognized line have no reason to share an object).
//!
//! Pipeline ([`recognize_stroke`], [`RecognizeMode::Free`]):
//!   1. closure test — trim ladder first ([`trim_overshoot`]: a tail that
//!      crosses back over the head closes at the crossing, and a near-miss
//!      T-junction — an endpoint almost touching the far end's segments —
//!      closes at its projection; dangles dropped either way), else
//!      `dist(start, end) < CLOSE_RATIO · bbox diagonal`.
//!   2. canonical fits, adopted when the confidence threshold passes:
//!      open   → straight line (max perpendicular deviation / chord ratio);
//!      closed → circle/ellipse (least-squares radial residual against the
//!               bbox ellipse), rect (rotated min-area + 0°/90° angle snap),
//!               triangle/polygon (coarse-RDP corner detection, 3–8 sides).
//!   3. fallback — silhouette-preserving normalize: coarse
//!      [`rdp_simplify`] + corner detection + per-run [`fit_beziers`]
//!      smoothing (sharp turns stay corners, smooth runs stay curves; this is
//!      also the open "smooth curve" fit when no corner is detected), capped
//!      at [`MAX_FALLBACK_NODES`]; closed per the step-1 test.
//!
//! [`RecognizeMode::Basic`] (the toolbar default) shares step 1 and then
//! FORCE-snaps to a basic primitive — no confidence thresholds, no polygon
//! (5+ sides) and no silhouette fallback: open → the 2-node line between the
//! exact input endpoints; closed → whichever of ellipse / rect / triangle
//! carries the smallest normalized residual ([`basic_closed_fit`]).
//!
//! The recognizer's geometry helpers REUSE `drawing.rs` ([`rdp_simplify`],
//! [`fit_beziers`], `perpendicular_distance`, `quantize_px`) — no duplicates.
//! Endpoints of OPEN results are preserved exactly (the quantization of the
//! input start/end), the premise the freehand anchoring path builds on.
//!
//! Pure (no time/rng/IO), pointer-width-agnostic; inputs are world-px samples,
//! the emitted path-string is world-px-quantized (Q=8) — the commit
//! ([`recognize_stroke_object`]) subtracts the origin so geometry stays
//! object-local with the position riding the transform translate (P4).

use super::drawing::{
    fit_beziers, perpendicular_distance, quantize_px, rdp_simplify, Brush,
};
use super::model::{
    path_string, FillRule, Geometry, HandlePoint, Object, PathNode, SubPath, Transform3x3,
};

/// Closure threshold: the start→end gap as a fraction of the bbox diagonal.
const CLOSE_RATIO: f64 = 0.15;
/// Overshoot trim scans this arc-length fraction at each end of the stroke for
/// a head/tail self-crossing (a closure overshoot lives near the ends).
const OVERSHOOT_WINDOW_RATIO: f64 = 0.25;
/// A crossing closes the stroke only when BOTH dangles (start→X and X→end arc
/// lengths) stay under this fraction of the total arc — a long dangle is real
/// geometry, not an overshot pen-up.
const OVERSHOOT_MAX_DANGLE_RATIO: f64 = 0.25;
/// Segments closer than this many indices never count as a crossing: adjacent
/// segments share an endpoint and would "intersect" there.
const OVERSHOOT_MIN_INDEX_GAP: usize = 2;
/// Near-miss T-junction: an endpoint whose projection onto a window segment
/// falls within this fraction of the bbox diagonal counts as touching the
/// stroke (a final edge ENDING on the first edge without crossing it)…
const NEAR_JUNCTION_RATIO: f64 = 0.06;
/// …floored at this many px so tiny strokes can still near-miss close.
const MIN_NEAR_JUNCTION_PX: f64 = 4.0;
/// Straight-line confidence: max perpendicular deviation / chord length.
const LINE_MAX_DEV_RATIO: f64 = 0.05;
/// Ellipse confidence: RMS of the normalized radial residual (|p−c| in
/// bbox-ellipse units minus 1).
const ELLIPSE_MAX_RMS: f64 = 0.10;
/// Ellipse fit needs real extent on both axes (a flat closed scribble is not
/// an ellipse).
const MIN_ELLIPSE_RADIUS_PX: f64 = 2.0;
/// Coarse RDP epsilon as a fraction of the bbox diagonal (scale-free), with a
/// 1.5px floor so tiny strokes don't keep every sample.
const NORMALIZE_EPSILON_RATIO: f64 = 0.04;
const MIN_EPSILON_PX: f64 = 1.5;
/// Corner sharpness is measured over a path-distance window of this many
/// epsilons on the RAW samples — a sharp corner keeps its full turn at a small
/// window while a smooth curve spreads it out.
const CORNER_WINDOW_FACTOR: f64 = 2.0;
/// Ring vertices turning less than this are merged away (e.g. a closed stroke
/// STARTED mid-edge always keeps its start sample — it is not a corner).
const COLLINEAR_MERGE_DEG: f64 = 15.0;
/// Minimum windowed turn for a vertex to count as a corner (an octagon's
/// exterior angle is 45°, the polygon ceiling).
const CORNER_MIN_DEG: f64 = 40.0;
/// Rect confidence: every ring turn within this of 90°.
const RECT_ANGLE_TOL_DEG: f64 = 20.0;
/// Rect orientation within this of an axis snaps to 0°/90° (axis-aligned).
const AXIS_SNAP_DEG: f64 = 10.0;
/// Polygon side ceiling (3..=8 per the design).
const POLYGON_MAX_SIDES: usize = 8;
/// How densely the bbox-ellipse outline is sampled for the Basic-mode residual
/// comparison (the rect/triangle outlines are their exact corner rings).
const BASIC_ELLIPSE_OUTLINE_SAMPLES: usize = 64;
/// Fallback node ceiling: a complex blob normalizes, it does not balloon.
const MAX_FALLBACK_NODES: usize = 24;
/// Cubic-arc circle constant (matches the primitive ellipse builders).
const KAPPA: f64 = 0.5523;

/// Pen recognition mode. `Free` is the full module pipeline (polygon +
/// silhouette fallbacks allowed); `Basic` (the toolbar default) force-snaps
/// every stroke to a basic primitive — open → 2-node line, closed → the best
/// of ellipse / rect / triangle by normalized residual, threshold-free.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecognizeMode {
    Basic,
    Free,
}

/// A recognized stroke: the canonical SVG-subset path-string (world-px
/// quantized units, exactly one subpath) plus whether it is closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecognizedStroke {
    pub d: String,
    pub closed: bool,
}

/// Recognize one freehand stroke (raw world-px samples) into its canonical
/// form (module pipeline). Fewer than 3 points skip recognition and fit as-is
/// (a 2-point stroke IS already a line; 0/1 points have no extent).
pub fn recognize_stroke(points: &[(f64, f64)], mode: RecognizeMode) -> RecognizedStroke {
    let (nodes, closed) = recognize_nodes(points, mode);
    RecognizedStroke { d: path_string::serialize(&[SubPath { closed, nodes }]), closed }
}

/// Recognize + commit one stroke to an [`Object`]: the recognized geometry is
/// translated to object-local coords relative to the stroke's bbox min (the
/// origin rides the transform translate, P4 zero-rebake) and the brush lowers
/// to the stroke style. `id`/`order` are caller-supplied (purity).
pub fn recognize_stroke_object(
    points: &[(f64, f64)],
    mode: RecognizeMode,
    brush: &Brush,
    id: String,
    order: String,
) -> Object {
    let (nodes, closed) = recognize_nodes(points, mode);
    let origin_x = points.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let origin_y = points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let ox = quantize_px(origin_x);
    let oy = quantize_px(origin_y);
    let local: Vec<PathNode> = nodes
        .into_iter()
        .map(|n| PathNode { x: n.x - ox, y: n.y - oy, ..n })
        .collect();
    let geometry =
        Geometry::from_subpaths(vec![SubPath { closed, nodes: local }], FillRule::NonZero);
    let mut object = Object::new(id, order, geometry);
    object.transform = Transform3x3::translate(origin_x, origin_y);
    object.stroke = Some(brush.to_stroke());
    object
}

/// The recognition core: canonical-fit nodes (world-px quantized) + closure.
/// The closure ladder is mode-independent; the mode picks the fit set.
fn recognize_nodes(points: &[(f64, f64)], mode: RecognizeMode) -> (Vec<PathNode>, bool) {
    if points.len() < 3 {
        return (fit_beziers(points), false);
    }
    let trimmed = trim_overshoot(points);
    let (points, crossed) = match trimmed.as_deref() {
        Some(loop_pts) => (loop_pts, true),
        None => (points, false),
    };
    let (min_x, min_y, max_x, max_y) = bbox(points);
    let diag = (max_x - min_x).hypot(max_y - min_y);
    let first = points[0];
    let last = points[points.len() - 1];
    let gap = (last.0 - first.0).hypot(last.1 - first.1);
    let closed = crossed || (diag > f64::EPSILON && gap < CLOSE_RATIO * diag);
    if closed {
        if mode == RecognizeMode::Basic {
            return (basic_closed_fit(points, (min_x, min_y, max_x, max_y), diag), true);
        }
        if let Some(nodes) = fit_ellipse(points, (min_x, min_y, max_x, max_y)) {
            return (nodes, true);
        }
        if let Some(nodes) = fit_polygon(points, diag) {
            return (nodes, true);
        }
        (normalize(points, diag, true), true)
    } else {
        if mode == RecognizeMode::Basic {
            return (force_line(points), false);
        }
        if let Some(nodes) = fit_line(points) {
            return (nodes, false);
        }
        (normalize(points, diag, false), false)
    }
}

fn bbox(points: &[(f64, f64)]) -> (f64, f64, f64, f64) {
    let mut b = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for &(x, y) in points {
        b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
    }
    b
}

// ---------------------------------------------------------------------------
// Closure ladder (cross-closure + near-miss T-junction).
// ---------------------------------------------------------------------------

/// Pre-closure trim ladder, tried in order; `None` = unchanged (the gap-based
/// closure test decides as before):
///   (a) [`trim_crossing`] — the tail crosses back over the head exactly;
///   (b) [`trim_near_junction`] — the pen-up END lands ON a head segment
///       without crossing it (a near-miss T-junction);
///   (c) the symmetric START-onto-tail case — (b) on the reversed stroke.
fn trim_overshoot(points: &[(f64, f64)]) -> Option<Vec<(f64, f64)>> {
    if let Some(loop_pts) = trim_crossing(points) {
        return Some(loop_pts);
    }
    if let Some(loop_pts) = trim_near_junction(points) {
        return Some(loop_pts);
    }
    let reversed: Vec<(f64, f64)> = points.iter().rev().copied().collect();
    let mut loop_pts = trim_near_junction(&reversed)?;
    loop_pts.reverse();
    Some(loop_pts)
}

/// `cum[k]` = arc length from the start to `points[k]`, plus the total.
fn arc_lengths(points: &[(f64, f64)]) -> (Vec<f64>, f64) {
    let mut cum = Vec::with_capacity(points.len());
    let mut total = 0.0;
    cum.push(0.0);
    for w in points.windows(2) {
        total += (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
        cum.push(total);
    }
    (cum, total)
}

/// Cross-closure overshoot trim: a stroke whose tail crosses back over its
/// head (a hand-drawn triangle overshooting its start) closes at the crossing
/// X, not at the pen-up gap — without this the crossed tail survives as a
/// spur. Tail segments scan end-inward against head segments start-outward,
/// both within [`OVERSHOOT_WINDOW_RATIO`] of the arc; a crossing trims only
/// when both dangles stay under [`OVERSHOOT_MAX_DANGLE_RATIO`], yielding the
/// loop that starts at X (the dangles dropped).
fn trim_crossing(points: &[(f64, f64)]) -> Option<Vec<(f64, f64)>> {
    let n = points.len();
    if n < 4 {
        return None;
    }
    let (cum, total) = arc_lengths(points);
    if total <= f64::EPSILON {
        return None;
    }
    let window = OVERSHOOT_WINDOW_RATIO * total;
    let max_dangle = OVERSHOOT_MAX_DANGLE_RATIO * total;
    for j in (0..n - 1).rev() {
        if total - cum[j + 1] > window {
            break;
        }
        for i in 0..n - 1 {
            if cum[i] > window || j - i < OVERSHOOT_MIN_INDEX_GAP {
                break;
            }
            let Some(x) =
                segment_intersection(points[i], points[i + 1], points[j], points[j + 1])
            else {
                continue;
            };
            let head_dangle = cum[i] + (x.0 - points[i].0).hypot(x.1 - points[i].1);
            let tail_dangle = total - cum[j] - (x.0 - points[j].0).hypot(x.1 - points[j].1);
            if head_dangle >= max_dangle || tail_dangle >= max_dangle {
                continue;
            }
            let mut loop_pts = Vec::with_capacity(j - i + 1);
            loop_pts.push(x);
            loop_pts.extend_from_slice(&points[i + 1..=j]);
            return Some(loop_pts);
        }
    }
    None
}

/// Near-miss T-junction trim: the pen-up END almost touches an early head
/// segment without crossing it (the browser repro — a rect whose start
/// dangles left of where the final edge lands on the top edge: no exact
/// intersection, and the start→end gap fails CLOSE_RATIO because of the
/// dangle). The nearest projection within [`NEAR_JUNCTION_RATIO`]·diag
/// (floored at [`MIN_NEAR_JUNCTION_PX`]) becomes the junction X: the head
/// dangle (start→X) is dropped and END snaps to X — an exactly-closed loop.
/// Window, dangle, and index-gap rules match [`trim_crossing`].
fn trim_near_junction(points: &[(f64, f64)]) -> Option<Vec<(f64, f64)>> {
    let n = points.len();
    if n < 4 {
        return None;
    }
    let (cum, total) = arc_lengths(points);
    if total <= f64::EPSILON {
        return None;
    }
    let window = OVERSHOOT_WINDOW_RATIO * total;
    let max_dangle = OVERSHOOT_MAX_DANGLE_RATIO * total;
    let (min_x, min_y, max_x, max_y) = bbox(points);
    let tol =
        (NEAR_JUNCTION_RATIO * (max_x - min_x).hypot(max_y - min_y)).max(MIN_NEAR_JUNCTION_PX);
    let end = points[n - 1];
    let mut best: Option<(f64, usize, (f64, f64))> = None;
    for i in 0..n - 1 {
        if cum[i] > window || n - 2 - i < OVERSHOOT_MIN_INDEX_GAP {
            break;
        }
        let (x, dist) = project_to_segment(end, points[i], points[i + 1]);
        if dist >= tol || best.is_some_and(|(d, ..)| dist >= d) {
            continue;
        }
        let head_dangle = cum[i] + (x.0 - points[i].0).hypot(x.1 - points[i].1);
        if head_dangle < max_dangle {
            best = Some((dist, i, x));
        }
    }
    let (_, i, x) = best?;
    let mut loop_pts = Vec::with_capacity(n - i);
    loop_pts.push(x);
    loop_pts.extend_from_slice(&points[i + 1..n - 1]);
    loop_pts.push(x); // END snapped onto the junction.
    Some(loop_pts)
}

/// Closest point on segment `a→b` to `p`, with its distance.
fn project_to_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> ((f64, f64), f64) {
    let ab = (b.0 - a.0, b.1 - a.1);
    let len2 = ab.0 * ab.0 + ab.1 * ab.1;
    let t = if len2 <= f64::EPSILON {
        0.0
    } else {
        (((p.0 - a.0) * ab.0 + (p.1 - a.1) * ab.1) / len2).clamp(0.0, 1.0)
    };
    let x = (a.0 + t * ab.0, a.1 + t * ab.1);
    (x, (p.0 - x.0).hypot(p.1 - x.1))
}

/// Intersection point of segments `a0→a1` and `b0→b1` (`None` when parallel
/// or the crossing falls outside either segment).
fn segment_intersection(
    a0: (f64, f64),
    a1: (f64, f64),
    b0: (f64, f64),
    b1: (f64, f64),
) -> Option<(f64, f64)> {
    let r = (a1.0 - a0.0, a1.1 - a0.1);
    let s = (b1.0 - b0.0, b1.1 - b0.1);
    let denom = r.0 * s.1 - r.1 * s.0;
    if denom.abs() <= f64::EPSILON {
        return None;
    }
    let q = (b0.0 - a0.0, b0.1 - a0.1);
    let t = (q.0 * s.1 - q.1 * s.0) / denom;
    let u = (q.0 * r.1 - q.1 * r.0) / denom;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u))
        .then(|| (a0.0 + t * r.0, a0.1 + t * r.1))
}

// ---------------------------------------------------------------------------
// Canonical fits.
// ---------------------------------------------------------------------------

/// The 2-node line between the exact quantized INPUT endpoints — the
/// anchoring premise, and the unconditional Basic-mode open snap.
fn force_line(points: &[(f64, f64)]) -> Vec<PathNode> {
    let a = points[0];
    let b = points[points.len() - 1];
    vec![
        PathNode::corner(quantize_px(a.0), quantize_px(a.1)),
        PathNode::corner(quantize_px(b.0), quantize_px(b.1)),
    ]
}

/// Straight line: every sample within `LINE_MAX_DEV_RATIO · chord` of the
/// start→end chord. The endpoints are the INPUT endpoints, untouched —
/// the anchoring premise.
fn fit_line(points: &[(f64, f64)]) -> Option<Vec<PathNode>> {
    let a = points[0];
    let b = points[points.len() - 1];
    let chord = (b.0 - a.0).hypot(b.1 - a.1);
    if chord <= f64::EPSILON {
        return None;
    }
    let max_dev = points.iter().map(|&p| perpendicular_distance(p, a, b)).fold(0.0, f64::max);
    (max_dev / chord <= LINE_MAX_DEV_RATIO).then(|| force_line(points))
}

/// Circle/ellipse: least-squares radial residual against the axis-aligned
/// bbox ellipse (a circle is the rx≈ry case — no separate fit). Emits the
/// standard four-cubic-arc closed ring.
fn fit_ellipse(
    points: &[(f64, f64)],
    (min_x, min_y, max_x, max_y): (f64, f64, f64, f64),
) -> Option<Vec<PathNode>> {
    let rx = (max_x - min_x) / 2.0;
    let ry = (max_y - min_y) / 2.0;
    if rx < MIN_ELLIPSE_RADIUS_PX || ry < MIN_ELLIPSE_RADIUS_PX {
        return None;
    }
    let cx = (min_x + max_x) / 2.0;
    let cy = (min_y + max_y) / 2.0;
    let mse = points
        .iter()
        .map(|&(x, y)| {
            let r = ((x - cx) / rx).hypot((y - cy) / ry);
            (r - 1.0) * (r - 1.0)
        })
        .sum::<f64>()
        / points.len() as f64;
    if mse.sqrt() > ELLIPSE_MAX_RMS {
        return None;
    }
    Some(ellipse_nodes(cx, cy, rx, ry))
}

/// The standard four-cubic-arc closed ring for the axis-aligned ellipse, in
/// the parse-canonical closed-curve form: the closing arc's landing node is
/// explicit (last == first position), so `subpaths` and a re-parse of the
/// serialized `d` are byte-identical (no runtime-mirror divergence).
fn ellipse_nodes(cx: f64, cy: f64, rx: f64, ry: f64) -> Vec<PathNode> {
    let kx = KAPPA * rx;
    let ky = KAPPA * ry;
    let handle = |dx: f64, dy: f64| Some(HandlePoint { dx: quantize_px(dx), dy: quantize_px(dy) });
    let node = |x: f64, y: f64, in_h: Option<HandlePoint>, out_h: Option<HandlePoint>| PathNode {
        x: quantize_px(x),
        y: quantize_px(y),
        in_handle: in_h,
        out_handle: out_h,
        width: None,
    };
    vec![
        node(cx - rx, cy, None, handle(0.0, -ky)),
        node(cx, cy - ry, handle(-kx, 0.0), handle(kx, 0.0)),
        node(cx + rx, cy, handle(0.0, -ky), handle(0.0, ky)),
        node(cx, cy + ry, handle(kx, 0.0), handle(-kx, 0.0)),
        node(cx - rx, cy, handle(0.0, ky), None),
    ]
}

/// Triangle/rect/polygon: coarse-RDP vertices, near-collinear ones merged
/// away (a mid-edge stroke start is not a corner), every survivor sharp, 3–8
/// sides. Four right-angled corners route to the rect fit first.
fn fit_polygon(points: &[(f64, f64)], diag: f64) -> Option<Vec<PathNode>> {
    let eps = normalize_epsilon(diag);
    let kept = rdp_simplify(points, eps);
    if kept.len() < 4 {
        return None;
    }
    // Drop the pen-up endpoint: on a closed stroke it rides next to the start
    // and the closing segment re-links the ring.
    let ring = &kept[..kept.len() - 1];
    let idx = kept_indices(points, ring);
    let w = CORNER_WINDOW_FACTOR * eps;
    let vertices: Vec<((f64, f64), f64)> = ring
        .iter()
        .zip(&idx)
        .map(|(&p, &i)| (p, window_turn_deg(points, i, w, true)))
        .filter(|&(_, t)| t >= COLLINEAR_MERGE_DEG)
        .collect();
    if !(3..=POLYGON_MAX_SIDES).contains(&vertices.len())
        || vertices.iter().any(|&(_, t)| t < CORNER_MIN_DEG)
    {
        return None;
    }
    let corners: Vec<(f64, f64)> = vertices.into_iter().map(|(p, _)| p).collect();
    if corners.len() == 4 {
        if let Some(nodes) = fit_rect(&corners) {
            return Some(nodes);
        }
    }
    Some(corners.iter().map(|&(x, y)| PathNode::corner(quantize_px(x), quantize_px(y))).collect())
}

/// Rect from 4 ring corners: every turn ~90°, oriented by the longest edge —
/// within `AXIS_SNAP_DEG` of an axis it snaps to the axis-aligned bbox of the
/// corners; otherwise the min-area rect at that orientation (rotate, bbox,
/// rotate back). `None` when the quad is not right-angled (stays a polygon).
fn fit_rect(corners: &[(f64, f64)]) -> Option<Vec<PathNode>> {
    let n = corners.len();
    for i in 0..n {
        let t = turn_deg(corners[(i + n - 1) % n], corners[i], corners[(i + 1) % n]);
        if (t - 90.0).abs() > RECT_ANGLE_TOL_DEG {
            return None;
        }
    }
    let mut best = (0.0_f64, 0.0_f64); // (edge length, edge angle deg)
    for i in 0..n {
        let a = corners[i];
        let b = corners[(i + 1) % n];
        let len = (b.0 - a.0).hypot(b.1 - a.1);
        if len > best.0 {
            best = (len, (b.1 - a.1).atan2(b.0 - a.0).to_degrees());
        }
    }
    Some(
        oriented_rect_corners(corners, best.1)
            .iter()
            .map(|&(x, y)| PathNode::corner(quantize_px(x), quantize_px(y)))
            .collect(),
    )
}

/// Min bbox of `points` at orientation `edge_angle_deg` (folded into
/// [-45°, 45°) — rect symmetry is mod 90° — and snapped to 0° within
/// [`AXIS_SNAP_DEG`]): bbox in the rotated frame, corners mapped back in ring
/// order. `frame_theta == 0` degenerates to the axis-aligned bbox.
fn oriented_rect_corners(points: &[(f64, f64)], edge_angle_deg: f64) -> [(f64, f64); 4] {
    let mut theta = edge_angle_deg.rem_euclid(90.0);
    if theta >= 45.0 {
        theta -= 90.0;
    }
    let frame_theta = if theta.abs() <= AXIS_SNAP_DEG { 0.0 } else { theta.to_radians() };
    let (sin, cos) = frame_theta.sin_cos();
    let frame: Vec<(f64, f64)> =
        points.iter().map(|&(x, y)| (x * cos + y * sin, -x * sin + y * cos)).collect();
    let (fx0, fy0, fx1, fy1) = bbox(&frame);
    let back = |fx: f64, fy: f64| (fx * cos - fy * sin, fx * sin + fy * cos);
    [back(fx0, fy0), back(fx1, fy0), back(fx1, fy1), back(fx0, fy1)]
}

// ---------------------------------------------------------------------------
// Basic-mode forced closed fit (ellipse vs rect vs triangle, threshold-free).
// ---------------------------------------------------------------------------

/// Basic-mode closed snap: fit ALL THREE basic candidates — the bbox ellipse,
/// the oriented min-bbox rect, the max-area triangle — and adopt the smallest
/// normalized residual: RMS sample→outline distance over the bbox diagonal,
/// the SAME scale for every candidate, so the residuals compare directly. No
/// confidence thresholds and no polygon (5+ sides) or silhouette fallback —
/// one of the three always wins (ties resolve ellipse → rect → triangle).
fn basic_closed_fit(
    points: &[(f64, f64)],
    (min_x, min_y, max_x, max_y): (f64, f64, f64, f64),
    diag: f64,
) -> Vec<PathNode> {
    let cx = (min_x + max_x) / 2.0;
    let cy = (min_y + max_y) / 2.0;
    let rx = (max_x - min_x) / 2.0;
    let ry = (max_y - min_y) / 2.0;
    let ellipse_ring: Vec<(f64, f64)> = (0..BASIC_ELLIPSE_OUTLINE_SAMPLES)
        .map(|i| {
            let t = core::f64::consts::TAU * i as f64 / BASIC_ELLIPSE_OUTLINE_SAMPLES as f64;
            (cx + rx * t.cos(), cy + ry * t.sin())
        })
        .collect();
    let rect = basic_rect_corners(points);
    let tri = basic_triangle_corners(points, diag);
    let scale = diag.max(f64::EPSILON);
    let ellipse_res = outline_rms(points, &ellipse_ring) / scale;
    let rect_res = outline_rms(points, &rect) / scale;
    let tri_res = outline_rms(points, &tri) / scale;
    let corner_ring = |ring: &[(f64, f64)]| {
        ring.iter().map(|&(x, y)| PathNode::corner(quantize_px(x), quantize_px(y))).collect()
    };
    if ellipse_res <= rect_res && ellipse_res <= tri_res {
        ellipse_nodes(cx, cy, rx, ry)
    } else if rect_res <= tri_res {
        corner_ring(&rect)
    } else {
        corner_ring(&tri)
    }
}

/// Basic-mode rect candidate: the AXIS-ALIGNED min bbox of all samples. Basic
/// strokes resolve flat (no rotation) — a tilted box is a Free-mode result; the
/// user rotates the flat rect by hand (the rotate handle) if they want it
/// angled. (Free's [`fit_rect`] still orients by the longest edge.)
fn basic_rect_corners(points: &[(f64, f64)]) -> [(f64, f64); 4] {
    oriented_rect_corners(points, 0.0)
}

/// Basic-mode triangle candidate: an axis-aligned ISOSCELES triangle filling the
/// bbox, its apex pointing the same cardinal direction (up/down/left/right) as the
/// drawn triangle's apex. Basic shapes resolve flat like the rect — the apex of
/// the max-area triple over the coarse ring only picks the cardinal; the
/// silhouette is otherwise normalized to a clean isosceles (the user rotates it
/// by hand if they want it angled).
fn basic_triangle_corners(points: &[(f64, f64)], diag: f64) -> [(f64, f64); 3] {
    let (min_x, min_y, max_x, max_y) = bbox(points);
    let cx = (min_x + max_x) / 2.0;
    let cy = (min_y + max_y) / 2.0;
    // Apex direction: the vertex opposite the longest edge of the max-area triple,
    // measured from that edge's midpoint, then snapped to a cardinal.
    let kept = rdp_simplify(points, normalize_epsilon(diag));
    let ring: &[(f64, f64)] = if kept.len() > 3 { &kept[..kept.len() - 1] } else { &kept };
    let (adx, ady) = if ring.len() >= 3 {
        let area2 = |a: (f64, f64), b: (f64, f64), c: (f64, f64)| {
            ((b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)).abs()
        };
        let mut best = (f64::NEG_INFINITY, [ring[0], ring[1], ring[2]]);
        for i in 0..ring.len() {
            for j in i + 1..ring.len() {
                for k in j + 1..ring.len() {
                    let a2 = area2(ring[i], ring[j], ring[k]);
                    if a2 > best.0 {
                        best = (a2, [ring[i], ring[j], ring[k]]);
                    }
                }
            }
        }
        let [a, b, c] = best.1;
        let ab = (a.0 - b.0).hypot(a.1 - b.1);
        let bc = (b.0 - c.0).hypot(b.1 - c.1);
        let ca = (c.0 - a.0).hypot(c.1 - a.1);
        let (mid, apex) = if ab >= bc && ab >= ca {
            (((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0), c)
        } else if bc >= ca {
            (((b.0 + c.0) / 2.0, (b.1 + c.1) / 2.0), a)
        } else {
            (((c.0 + a.0) / 2.0, (c.1 + a.1) / 2.0), b)
        };
        (apex.0 - mid.0, apex.1 - mid.1)
    } else {
        (0.0, -1.0) // degenerate scribble: default apex up
    };
    // Cardinal-snap the apex (y grows downward, so apex-up = ady < 0) and build the
    // bbox-filling isosceles triangle pointing that way.
    if adx.abs() >= ady.abs() {
        if adx < 0.0 {
            [(max_x, min_y), (max_x, max_y), (min_x, cy)] // apex left
        } else {
            [(min_x, min_y), (min_x, max_y), (max_x, cy)] // apex right
        }
    } else if ady < 0.0 {
        [(min_x, max_y), (max_x, max_y), (cx, min_y)] // apex up
    } else {
        [(min_x, min_y), (max_x, min_y), (cx, max_y)] // apex down
    }
}

/// RMS distance (px) from every sample to a closed outline ring — the shared
/// Basic-mode residual metric (normalized by the caller).
fn outline_rms(points: &[(f64, f64)], ring: &[(f64, f64)]) -> f64 {
    let mse = points
        .iter()
        .map(|&p| {
            let mut best = f64::INFINITY;
            for i in 0..ring.len() {
                let (_, dist) = project_to_segment(p, ring[i], ring[(i + 1) % ring.len()]);
                best = best.min(dist);
            }
            best * best
        })
        .sum::<f64>()
        / points.len() as f64;
    mse.sqrt()
}

// ---------------------------------------------------------------------------
// Silhouette-preserving fallback normalize.
// ---------------------------------------------------------------------------

fn normalize_epsilon(diag: f64) -> f64 {
    (NORMALIZE_EPSILON_RATIO * diag).max(MIN_EPSILON_PX)
}

/// Coarse RDP + corner detection + per-run [`fit_beziers`]: sharp turns stay
/// corner nodes (each smooth run is fitted independently, so the tangents on
/// either side of a corner never smooth across it), smooth runs get
/// Catmull-Rom handles. Doubles epsilon until the node count fits the cap.
fn normalize(points: &[(f64, f64)], diag: f64, closed: bool) -> Vec<PathNode> {
    let mut eps = normalize_epsilon(diag);
    loop {
        let mut kept = rdp_simplify(points, eps);
        if closed && kept.len() > 2 {
            kept.pop(); // pen-up endpoint rides next to the start; Z closes.
        }
        if kept.len() > MAX_FALLBACK_NODES && eps < diag {
            eps *= 2.0;
            continue;
        }
        let idx = kept_indices(points, &kept);
        let w = CORNER_WINDOW_FACTOR * eps;
        let mut cuts: Vec<usize> = vec![0];
        for k in 1..kept.len().saturating_sub(1) {
            if window_turn_deg(points, idx[k], w, closed) >= CORNER_MIN_DEG {
                cuts.push(k);
            }
        }
        cuts.push(kept.len() - 1);
        let mut nodes: Vec<PathNode> = Vec::with_capacity(kept.len());
        for pair in cuts.windows(2) {
            let fitted = fit_beziers(&kept[pair[0]..=pair[1]]);
            match nodes.last_mut() {
                None => nodes.extend(fitted),
                Some(corner) => {
                    // The shared corner keeps the previous run's one-sided
                    // in-handle and takes the next run's out-handle — the
                    // tangents stay independent, so the corner stays sharp.
                    corner.out_handle = fitted[0].out_handle;
                    nodes.extend(fitted.into_iter().skip(1));
                }
            }
        }
        return nodes;
    }
}

// ---------------------------------------------------------------------------
// Corner detection over the RAW samples.
// ---------------------------------------------------------------------------

/// Indices into `raw` of each member of `kept` (RDP keeps exact input points,
/// in order, so an advancing bit-equality scan recovers them).
fn kept_indices(raw: &[(f64, f64)], kept: &[(f64, f64)]) -> Vec<usize> {
    let mut out = Vec::with_capacity(kept.len());
    let mut cursor = 0;
    for &k in kept {
        while cursor < raw.len() && raw[cursor] != k {
            cursor += 1;
        }
        out.push(cursor.min(raw.len().saturating_sub(1)));
        cursor += 1;
    }
    out
}

/// Direction change (degrees, 0 = straight) from `a→b` to `b→c`.
fn turn_deg(a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> f64 {
    let (ux, uy) = (b.0 - a.0, b.1 - a.1);
    let (vx, vy) = (c.0 - b.0, c.1 - b.1);
    (ux * vy - uy * vx).atan2(ux * vx + uy * vy).abs().to_degrees()
}

/// The turn at raw sample `idx` measured over a path-distance window `w` on
/// each side (wrapping across the seam when `closed`). A sharp corner keeps
/// its full angle at a small window; a smooth curve's turn over `2w` stays
/// shallow — this separates a hexagon's vertex from a circle's RDP residue.
fn window_turn_deg(raw: &[(f64, f64)], idx: usize, w: f64, closed: bool) -> f64 {
    let n = raw.len();
    if n < 3 {
        return 0.0;
    }
    let walk = |backward: bool| -> Option<(f64, f64)> {
        let mut i = idx;
        let mut dist = 0.0;
        for _ in 0..n {
            let next = if backward {
                if i == 0 {
                    if !closed {
                        break;
                    }
                    n - 1
                } else {
                    i - 1
                }
            } else if i + 1 >= n {
                if !closed {
                    break;
                }
                0
            } else {
                i + 1
            };
            dist += (raw[next].0 - raw[i].0).hypot(raw[next].1 - raw[i].1);
            i = next;
            if dist >= w {
                return Some(raw[i]);
            }
        }
        (i != idx).then(|| raw[i])
    };
    match (walk(true), walk(false)) {
        (Some(a), Some(b)) => turn_deg(a, raw[idx], b),
        _ => 0.0,
    }
}

// ---------------------------------------------------------------------------
// Tests — synthetic-stroke goldens (deterministic, no rng).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::model::GEOMETRY_QUANTUM_PER_PX;

    const Q: i32 = GEOMETRY_QUANTUM_PER_PX;

    fn parse(d: &str) -> Vec<SubPath> {
        path_string::parse(d).expect("recognized d parses")
    }

    /// Samples along the segment `a -> b` (excluding `b`), `n` per edge.
    fn edge(a: (f64, f64), b: (f64, f64), n: usize, out: &mut Vec<(f64, f64)>) {
        for i in 0..n {
            let t = i as f64 / n as f64;
            out.push((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
        }
    }

    #[test]
    fn noisy_straight_stroke_recognizes_as_a_two_node_line() {
        // 21 samples along (0,0)->(200,10) with alternating ±1px wiggle on the
        // interior: deviation/chord ≈ 0.005, well under the line threshold.
        let pts: Vec<(f64, f64)> = (0..=20)
            .map(|i| {
                let t = f64::from(i) / 20.0;
                let wiggle =
                    if i == 0 || i == 20 { 0.0 } else if i % 2 == 0 { 1.0 } else { -1.0 };
                (200.0 * t, 10.0 * t + wiggle)
            })
            .collect();
        let rec = recognize_stroke(&pts, RecognizeMode::Free);
        assert!(!rec.closed);
        // Exactly two nodes at the EXACT quantized input endpoints.
        assert_eq!(rec.d, format!("M 0 0 L {} {}", 200 * Q, 10 * Q));
    }

    #[test]
    fn rough_circle_recognizes_as_a_closed_four_arc_ellipse() {
        // r(θ) = 50 + 2·sin(7θ) around (100,100): radial noise ±2px on r=50.
        let pts: Vec<(f64, f64)> = (0..72)
            .map(|i| {
                let theta = f64::from(i) * 5.0_f64.to_radians();
                let r = 50.0 + 2.0 * (7.0 * theta).sin();
                (100.0 + r * theta.cos(), 100.0 + r * theta.sin())
            })
            .collect();
        let rec = recognize_stroke(&pts, RecognizeMode::Free);
        assert!(rec.closed);
        let subpaths = parse(&rec.d);
        assert_eq!(subpaths.len(), 1);
        let sp = &subpaths[0];
        assert!(sp.closed);
        // Four cubic arcs in the parse-canonical form: the closing arc's
        // landing node is explicit, so 5 nodes with last == first.
        assert_eq!(sp.nodes.len(), 5, "four-arc ellipse: {}", rec.d);
        let (l, t, r, b) = (&sp.nodes[0], &sp.nodes[1], &sp.nodes[2], &sp.nodes[3]);
        let last = &sp.nodes[4];
        assert_eq!((last.x, last.y), (l.x, l.y));
        assert!(sp.nodes[1..4].iter().all(|n| n.in_handle.is_some() && n.out_handle.is_some()));
        // Cardinal layout: left/right share the center y, top/bottom the
        // center x (the bbox-ellipse center).
        assert_eq!(l.y, r.y);
        assert_eq!(t.x, b.x);
        assert!(l.x < t.x && t.x < r.x);
        assert!(t.y < l.y && l.y < b.y);
    }

    #[test]
    fn rough_rect_recognizes_as_an_axis_snapped_rect() {
        // Perimeter walk of (0,0)-(120,80) with ±1.2px edge noise, ending a
        // little short of the start (the pen-up gap). The noise is under the
        // coarse epsilon, the corners are exact, the orientation is 0° — so
        // the result is the EXACT axis-aligned rect.
        let pts = vec![
            (0.0, 0.0),
            (30.0, 1.2),
            (60.0, -1.0),
            (90.0, 1.0),
            (120.0, 0.0),
            (121.0, 26.0),
            (119.0, 53.0),
            (120.0, 80.0),
            (90.0, 81.0),
            (60.0, 79.0),
            (30.0, 80.8),
            (0.0, 80.0),
            (-1.0, 55.0),
            (1.0, 30.0),
            (0.0, 10.0),
        ];
        let rec = recognize_stroke(&pts, RecognizeMode::Free);
        assert!(rec.closed);
        assert_eq!(rec.d, format!("M 0 0 L {} 0 L {} {} L 0 {} Z", 120 * Q, 120 * Q, 80 * Q, 80 * Q));
    }

    #[test]
    fn triangle_stroke_recognizes_as_a_three_corner_polygon() {
        let mut pts = Vec::new();
        edge((0.0, 0.0), (100.0, 0.0), 10, &mut pts);
        edge((100.0, 0.0), (50.0, 80.0), 10, &mut pts);
        edge((50.0, 80.0), (0.0, 0.0), 9, &mut pts); // stops short of the start
        let rec = recognize_stroke(&pts, RecognizeMode::Free);
        assert!(rec.closed);
        assert_eq!(rec.d, format!("M 0 0 L {} 0 L {} {} Z", 100 * Q, 50 * Q, 80 * Q));
    }

    #[test]
    fn triangle_with_crossing_overshoot_tail_trims_to_a_three_corner_polygon() {
        // Triangle whose last edge overshoots PAST the start, crossing the
        // first edge at (70/3, 0) and leaving a short tail to (15,-25). The
        // pen-up gap (29.2px) exceeds CLOSE_RATIO·diag (21.8px), so without
        // the overshoot trim this reads as an OPEN stroke with the tail kept.
        let mut pts = Vec::new();
        edge((0.0, 0.0), (100.0, 0.0), 10, &mut pts);
        edge((100.0, 0.0), (50.0, 80.0), 10, &mut pts);
        edge((50.0, 80.0), (15.0, -25.0), 10, &mut pts);
        pts.push((15.0, -25.0));
        let rec = recognize_stroke(&pts, RecognizeMode::Free);
        assert!(rec.closed, "cross-closure detected: {}", rec.d);
        // The loop closes at the crossing: corners X=(70/3,0) -> (100,0) ->
        // (50,80), the dangles on both sides trimmed away. 187 = round(8·70/3).
        assert_eq!(rec.d, format!("M 187 0 L {} 0 L {} {} Z", 100 * Q, 50 * Q, 80 * Q));
    }

    #[test]
    fn rect_with_crossing_overshoot_tail_trims_to_the_axis_rect() {
        // Square whose closing edge overshoots through the bottom edge at
        // (60/7, 0) and dangles to (12,-40): the gap (41.8px) fails the gap
        // closure test, but the crossing closes it and the rect fit's axis
        // bbox absorbs the mid-edge crossing corner.
        let mut pts = Vec::new();
        edge((0.0, 0.0), (100.0, 0.0), 10, &mut pts);
        edge((100.0, 0.0), (100.0, 100.0), 10, &mut pts);
        edge((100.0, 100.0), (0.0, 100.0), 10, &mut pts);
        edge((0.0, 100.0), (12.0, -40.0), 10, &mut pts);
        pts.push((12.0, -40.0));
        let rec = recognize_stroke(&pts, RecognizeMode::Free);
        assert!(rec.closed, "cross-closure detected: {}", rec.d);
        assert_eq!(
            rec.d,
            format!("M 0 0 L {} 0 L {} {} L 0 {} Z", 100 * Q, 100 * Q, 100 * Q, 100 * Q)
        );
    }

    #[test]
    fn crossing_with_a_long_dangle_does_not_trim() {
        // Same triangle trajectory, but the tail runs on to (-10,-100): the
        // tail dangle is ~27% of the arc (over OVERSHOOT_MAX_DANGLE_RATIO), so
        // the crossing is real geometry, not overshoot — no trim, and the
        // 100.5px gap keeps the stroke open.
        let mut pts = Vec::new();
        edge((0.0, 0.0), (100.0, 0.0), 10, &mut pts);
        edge((100.0, 0.0), (50.0, 80.0), 10, &mut pts);
        edge((50.0, 80.0), (-10.0, -100.0), 10, &mut pts);
        pts.push((-10.0, -100.0));
        let rec = recognize_stroke(&pts, RecognizeMode::Free);
        assert!(!rec.closed, "long dangle stays untrimmed: {}", rec.d);
    }

    #[test]
    fn adjacent_segments_sharing_a_point_are_not_a_crossing() {
        // A sharp staircase whose first and last segments both sit inside the
        // scan windows: consecutive segments touch at their shared point, which
        // must never read as a self-crossing (index-gap guard) — open stroke.
        let pts = vec![(0.0, 0.0), (8.0, 0.0), (8.0, 4.0), (120.0, 4.0)];
        let rec = recognize_stroke(&pts, RecognizeMode::Free);
        assert!(!rec.closed, "corner touch is not a closure: {}", rec.d);
    }

    #[test]
    fn rect_with_start_dangle_and_t_touching_end_closes_at_the_junction() {
        // The browser repro: the stroke starts on a dangle LEFT of the square
        // ((-30,0)→(0,0)), walks the perimeter, and the final edge ENDS 3px off
        // the top edge at (8,3) — a T-junction near-miss with no exact
        // self-intersection anywhere. The pen-up gap (38.1px) exceeds
        // CLOSE_RATIO·diag (24.6px) BECAUSE of the dangle, so without the
        // projection rung this survives as an open path. The near-junction rung
        // snaps END onto (8,0), drops the dangle, and the rect fit's axis bbox
        // absorbs the mid-edge junction corner.
        let mut pts = Vec::new();
        edge((-30.0, 0.0), (0.0, 0.0), 3, &mut pts);
        edge((0.0, 0.0), (100.0, 0.0), 10, &mut pts);
        edge((100.0, 0.0), (100.0, 100.0), 10, &mut pts);
        edge((100.0, 100.0), (0.0, 100.0), 10, &mut pts);
        edge((0.0, 100.0), (8.0, 3.0), 10, &mut pts);
        pts.push((8.0, 3.0));
        let rec = recognize_stroke(&pts, RecognizeMode::Free);
        assert!(rec.closed, "T-junction near-miss closes: {}", rec.d);
        assert_eq!(
            rec.d,
            format!("M 0 0 L {} 0 L {} {} L 0 {} Z", 100 * Q, 100 * Q, 100 * Q, 100 * Q)
        );
    }

    #[test]
    fn start_t_touch_on_a_tail_segment_closes_symmetrically() {
        // The mirror case (the same square drawn in reverse): the stroke STARTS
        // 3px off the top edge, which gets drawn LAST, and the pen-up tail
        // dangles off past the square. START projects onto a tail-window
        // segment with no crossing anywhere — the symmetric rung trims the tail
        // dangle and snaps START onto the junction.
        let mut pts = Vec::new();
        edge((8.0, 3.0), (0.0, 100.0), 10, &mut pts);
        edge((0.0, 100.0), (100.0, 100.0), 10, &mut pts);
        edge((100.0, 100.0), (100.0, 0.0), 10, &mut pts);
        edge((100.0, 0.0), (0.0, 0.0), 10, &mut pts);
        edge((0.0, 0.0), (-30.0, 0.0), 3, &mut pts);
        pts.push((-30.0, 0.0));
        let rec = recognize_stroke(&pts, RecognizeMode::Free);
        assert!(rec.closed, "tail-side T-junction closes: {}", rec.d);
        assert_eq!(
            rec.d,
            format!("M 0 0 L {} 0 L {} {} L 0 {} Z", 100 * Q, 100 * Q, 100 * Q, 100 * Q)
        );
    }

    #[test]
    fn near_touch_outside_the_projection_tolerance_stays_open() {
        // Same dangling square, but the final edge stops 12px short of the top
        // edge — outside NEAR_JUNCTION_RATIO·diag (9.84px). Not a touch, and
        // the 39.9px gap fails the gap test too: the stroke stays open.
        let mut pts = Vec::new();
        edge((-30.0, 0.0), (0.0, 0.0), 3, &mut pts);
        edge((0.0, 0.0), (100.0, 0.0), 10, &mut pts);
        edge((100.0, 0.0), (100.0, 100.0), 10, &mut pts);
        edge((100.0, 100.0), (0.0, 100.0), 10, &mut pts);
        edge((0.0, 100.0), (8.0, 12.0), 10, &mut pts);
        pts.push((8.0, 12.0));
        let rec = recognize_stroke(&pts, RecognizeMode::Free);
        assert!(!rec.closed, "out-of-tolerance near-miss stays open: {}", rec.d);
    }

    #[test]
    fn t_touch_with_an_oversized_dangle_does_not_trim() {
        // A 140px plumb line INTO the square's top edge (26% of the arc), then
        // the perimeter, ending with a T-touch at (46,3) mid-top-edge: the
        // would-be head dangle exceeds OVERSHOOT_MAX_DANGLE_RATIO, so it is
        // real geometry (a balloon on a string), not an overshot pen-up — no
        // trim, and the 143px gap keeps the stroke open.
        let mut pts = Vec::new();
        edge((40.0, -140.0), (40.0, 0.0), 7, &mut pts);
        edge((40.0, 0.0), (100.0, 0.0), 6, &mut pts);
        edge((100.0, 0.0), (100.0, 100.0), 10, &mut pts);
        edge((100.0, 100.0), (0.0, 100.0), 10, &mut pts);
        edge((0.0, 100.0), (0.0, 0.0), 10, &mut pts);
        edge((0.0, 0.0), (46.0, 3.0), 10, &mut pts);
        pts.push((46.0, 3.0));
        let rec = recognize_stroke(&pts, RecognizeMode::Free);
        assert!(!rec.closed, "oversized dangle stays untrimmed: {}", rec.d);
    }

    #[test]
    fn l_bend_open_stroke_falls_back_with_the_corner_preserved() {
        // (100,0) -> (0,0) -> (0,100): not a line (deviation ratio 0.5), open.
        let mut pts: Vec<(f64, f64)> =
            (0..=10).map(|i| (100.0 - 10.0 * f64::from(i), 0.0)).collect();
        pts.extend((1..=10).map(|i| (0.0, 10.0 * f64::from(i))));
        let rec = recognize_stroke(&pts, RecognizeMode::Free);
        assert!(!rec.closed);
        let sp = &parse(&rec.d)[0];
        assert!(!sp.closed);
        // The elbow survives as an interior corner node at exactly (0,0).
        assert!(
            sp.nodes[1..sp.nodes.len() - 1].iter().any(|n| n.x == 0 && n.y == 0),
            "corner preserved: {}",
            rec.d
        );
        // Endpoints preserved exactly (anchoring premise).
        let first = sp.nodes.first().unwrap();
        let last = sp.nodes.last().unwrap();
        assert_eq!((first.x, first.y), (100 * Q, 0));
        assert_eq!((last.x, last.y), (0, 100 * Q));
    }

    #[test]
    fn complex_closed_blob_normalizes_closed_under_the_node_cap() {
        // A five-petal flower r(θ) = 60 + 18·sin(5θ): too lumpy for the
        // ellipse fit, too many extrema for a polygon — the closed fallback.
        let pts: Vec<(f64, f64)> = (0..72)
            .map(|i| {
                let theta = f64::from(i) * 5.0_f64.to_radians();
                let r = 60.0 + 18.0 * (5.0 * theta).sin();
                (200.0 + r * theta.cos(), 200.0 + r * theta.sin())
            })
            .collect();
        let rec = recognize_stroke(&pts, RecognizeMode::Free);
        assert!(rec.closed);
        let sp = &parse(&rec.d)[0];
        assert!(sp.closed);
        assert!(sp.nodes.len() <= MAX_FALLBACK_NODES, "capped: {} nodes", sp.nodes.len());
        assert!(sp.nodes.len() > 4, "still a blob silhouette, not a primitive");
    }

    #[test]
    fn smooth_open_curve_preserves_its_input_endpoints_exactly() {
        // A sine bump from (0,0) to (100,0): smooth (no corner), not a line.
        let pts: Vec<(f64, f64)> = (0..=20)
            .map(|i| {
                let t = f64::from(i) / 20.0;
                (100.0 * t, 40.0 * (core::f64::consts::PI * t).sin())
            })
            .collect();
        let rec = recognize_stroke(&pts, RecognizeMode::Free);
        assert!(!rec.closed);
        let sp = &parse(&rec.d)[0];
        assert!(sp.nodes.len() > 2, "a curve, not a collapsed line: {}", rec.d);
        let first = sp.nodes.first().unwrap();
        let last = sp.nodes.last().unwrap();
        assert_eq!((first.x, first.y), (0, 0));
        assert_eq!((last.x, last.y), (100 * Q, 0));
        // Interior nodes carry smoothing handles (fit_beziers ran).
        assert!(sp.nodes[1..sp.nodes.len() - 1].iter().all(|n| n.in_handle.is_some()));
    }

    #[test]
    fn recognize_stroke_object_commits_object_local_geometry_with_origin_translate() {
        let pts: Vec<(f64, f64)> = (0..=10)
            .map(|i| (100.0 + 10.0 * f64::from(i), 110.0 + f64::from(i)))
            .collect();
        let object = recognize_stroke_object(
            &pts,
            RecognizeMode::Free,
            &Brush::new("#112233", 2.0),
            "draw-1".into(),
            "a0".into(),
        );
        assert_eq!(object.id, "draw-1");
        // A straight stroke at (100,110)->(200,120): line, object-local from
        // the bbox min with the origin riding the transform.
        assert_eq!(object.geometry.path_string, format!("M 0 0 L {} {}", 100 * Q, 10 * Q));
        assert_eq!(object.transform, Transform3x3::translate(100.0, 110.0));
        assert!(object.stroke.is_some());
        assert!(object.fill.is_none());
    }

    #[test]
    fn degenerate_inputs_fit_as_is_without_recognition() {
        assert_eq!(
            recognize_stroke(&[], RecognizeMode::Free),
            RecognizedStroke { d: String::new(), closed: false }
        );
        let two = recognize_stroke(&[(0.0, 0.0), (10.0, 0.0)], RecognizeMode::Basic);
        assert_eq!(two.d, format!("M 0 0 L {} 0", 10 * Q));
        assert!(!two.closed);
    }

    // -- RecognizeMode::Basic — forced snap to the basic primitives. ---------

    /// An irregular wobbly pentagon: 5 sharp corners (turns 60°–87°), interior
    /// edge samples carrying alternating ±1px noise, pen-up short of the start.
    fn wobbly_pentagon() -> Vec<(f64, f64)> {
        let v = [(0.0, 0.0), (100.0, 10.0), (130.0, 90.0), (40.0, 130.0), (-40.0, 70.0)];
        let mut pts = Vec::new();
        edge(v[0], v[1], 10, &mut pts);
        edge(v[1], v[2], 10, &mut pts);
        edge(v[2], v[3], 10, &mut pts);
        edge(v[3], v[4], 10, &mut pts);
        edge(v[4], v[0], 9, &mut pts); // stops short of the start
        for (i, p) in pts.iter_mut().enumerate() {
            if i % 10 != 0 {
                p.1 += if i % 2 == 0 { 1.0 } else { -1.0 };
            }
        }
        pts
    }

    #[test]
    fn basic_mode_snaps_a_wobbly_pentagon_to_a_canonical_primitive() {
        let pts = wobbly_pentagon();
        // Free keeps the 5-corner polygon (the silhouette).
        let free = recognize_stroke(&pts, RecognizeMode::Free);
        assert!(free.closed);
        let free_sp = &parse(&free.d)[0];
        assert_eq!(free_sp.nodes.len(), 5, "Free keeps the pentagon: {}", free.d);
        assert!(free_sp.nodes.iter().all(|n| n.in_handle.is_none() && n.out_handle.is_none()));
        // Basic forbids the polygon: the same stroke snaps to the closest of
        // ellipse/rect/triangle — here the bbox ellipse (node-shape golden:
        // the parse-canonical four-arc ring, no corner ring).
        let basic = recognize_stroke(&pts, RecognizeMode::Basic);
        assert!(basic.closed);
        let sp = &parse(&basic.d)[0];
        assert_eq!(sp.nodes.len(), 5, "four-arc ellipse: {}", basic.d);
        assert_eq!((sp.nodes[4].x, sp.nodes[4].y), (sp.nodes[0].x, sp.nodes[0].y));
        assert!(sp.nodes[1..4].iter().all(|n| n.in_handle.is_some() && n.out_handle.is_some()));
        assert_ne!(basic.d, free.d, "Basic re-resolved the polygon");
    }

    #[test]
    fn basic_mode_forces_a_wobbly_open_s_curve_to_a_two_node_line() {
        // An S-bend from (0,0) to (150,0) with ±0.8px wiggle on the interior:
        // far over the Free line threshold (dev/chord ≈ 0.13), smooth curve.
        let pts: Vec<(f64, f64)> = (0..=30)
            .map(|i| {
                let t = f64::from(i) / 30.0;
                let wiggle =
                    if i == 0 || i == 30 { 0.0 } else if i % 2 == 0 { 0.8 } else { -0.8 };
                (150.0 * t, 20.0 * (core::f64::consts::TAU * t).sin() + wiggle)
            })
            .collect();
        let free = recognize_stroke(&pts, RecognizeMode::Free);
        assert!(!free.closed);
        let free_sp = &parse(&free.d)[0];
        assert!(free_sp.nodes.len() > 2, "Free keeps the curve: {}", free.d);
        assert!(free.d.contains('C'), "Free smooths with bezier handles: {}", free.d);
        // Basic: unconditional 2-node line between the EXACT quantized input
        // endpoints (fit_line's endpoint preservation, threshold ignored).
        let basic = recognize_stroke(&pts, RecognizeMode::Basic);
        assert!(!basic.closed);
        assert_eq!(basic.d, format!("M 0 0 L {} 0", 150 * Q));
    }

    #[test]
    fn basic_residual_comparison_picks_the_ellipse_for_a_circular_stroke() {
        let pts: Vec<(f64, f64)> = (0..72)
            .map(|i| {
                let theta = f64::from(i) * 5.0_f64.to_radians();
                (100.0 + 50.0 * theta.cos(), 100.0 + 50.0 * theta.sin())
            })
            .collect();
        let rec = recognize_stroke(&pts, RecognizeMode::Basic);
        assert!(rec.closed);
        let sp = &parse(&rec.d)[0];
        // The four-arc ellipse won over rect/triangle: curved ring on the
        // exact circle bbox, cardinals at (50,100)/(100,50)/(150,100)/(100,150).
        assert_eq!(sp.nodes.len(), 5, "four-arc ellipse: {}", rec.d);
        assert!(sp.nodes[1..4].iter().all(|n| n.in_handle.is_some()));
        assert_eq!((sp.nodes[0].x, sp.nodes[0].y), (50 * Q, 100 * Q));
        assert_eq!((sp.nodes[1].x, sp.nodes[1].y), (100 * Q, 50 * Q));
        assert_eq!((sp.nodes[2].x, sp.nodes[2].y), (150 * Q, 100 * Q));
        assert_eq!((sp.nodes[3].x, sp.nodes[3].y), (100 * Q, 150 * Q));
    }

    #[test]
    fn basic_residual_comparison_picks_the_rect_for_a_square_stroke() {
        // Perimeter walk of (0,0)-(100,100), exact corners, inward-only edge
        // noise (so the sample bbox IS the square), pen-up short of the start.
        let pts = vec![
            (0.0, 0.0),
            (30.0, 1.2),
            (60.0, 0.8),
            (90.0, 1.0),
            (100.0, 0.0),
            (99.0, 26.0),
            (98.8, 53.0),
            (100.0, 100.0),
            (70.0, 99.0),
            (40.0, 98.8),
            (0.0, 100.0),
            (1.0, 70.0),
            (0.9, 40.0),
            (0.0, 12.0),
        ];
        let rec = recognize_stroke(&pts, RecognizeMode::Basic);
        assert!(rec.closed);
        assert_eq!(
            rec.d,
            format!("M 0 0 L {} 0 L {} {} L 0 {} Z", 100 * Q, 100 * Q, 100 * Q, 100 * Q)
        );
    }

    #[test]
    fn basic_residual_comparison_picks_the_triangle_for_a_triangular_stroke() {
        let mut pts = Vec::new();
        edge((0.0, 0.0), (100.0, 0.0), 10, &mut pts);
        edge((100.0, 0.0), (50.0, 80.0), 10, &mut pts);
        edge((50.0, 80.0), (0.0, 0.0), 9, &mut pts); // stops short of the start
        let rec = recognize_stroke(&pts, RecognizeMode::Basic);
        assert!(rec.closed);
        assert_eq!(rec.d, format!("M 0 0 L {} 0 L {} {} Z", 100 * Q, 50 * Q, 80 * Q));
    }

    #[test]
    fn basic_rect_candidate_is_always_axis_aligned() {
        // A clean square rotated 30° about its center. Free's fit_rect would
        // orient the box to the stroke; the Basic candidate must resolve it FLAT
        // — the axis-aligned bbox of the samples (every edge horizontal or
        // vertical), leaving any rotation to the user's rotate handle.
        let c = (100.0, 100.0);
        let (s, co) = 30.0_f64.to_radians().sin_cos();
        let rot = |x: f64, y: f64| (c.0 + (x - c.0) * co - (y - c.0) * s, c.1 + (x - c.0) * s + (y - c.0) * co);
        let corners = [(40.0, 40.0), (160.0, 40.0), (160.0, 160.0), (40.0, 160.0)];
        let mut pts = Vec::new();
        for i in 0..4 {
            let a = corners[i];
            let b = corners[(i + 1) % 4];
            edge(rot(a.0, a.1), rot(b.0, b.1), 12, &mut pts);
        }
        let r = basic_rect_corners(&pts);
        // Every edge is axis-aligned (consecutive corners share an x or a y).
        for i in 0..4 {
            let a = r[i];
            let b = r[(i + 1) % 4];
            assert!(
                (a.0 - b.0).abs() < 1e-6 || (a.1 - b.1).abs() < 1e-6,
                "edge {i} not axis-aligned: {a:?} -> {b:?}"
            );
        }
        // And it IS the sample bbox.
        let (bx0, by0, bx1, by1) = bbox(&pts);
        let xs = [r[0].0, r[1].0, r[2].0, r[3].0];
        let ys = [r[0].1, r[1].1, r[2].1, r[3].1];
        let x0 = xs.iter().cloned().fold(f64::INFINITY, f64::min);
        let x1 = xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let y0 = ys.iter().cloned().fold(f64::INFINITY, f64::min);
        let y1 = ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        assert!(
            (x0 - bx0).abs() < 1e-6 && (y0 - by0).abs() < 1e-6 && (x1 - bx1).abs() < 1e-6 && (y1 - by1).abs() < 1e-6,
            "axis rect must equal the sample bbox"
        );
    }

    #[test]
    fn basic_triangle_candidate_is_axis_aligned_isosceles() {
        // A wide, low, slightly tilted triangle pointing UP (base clearly the
        // longest edge). The Basic candidate resolves it to a FLAT isosceles: a
        // horizontal base on the bbox bottom, the apex centered on top — keeping
        // only the cardinal (up), not the tilt.
        let mut pts = Vec::new();
        edge((0.0, 102.0), (120.0, 98.0), 14, &mut pts); // wide base, tilted
        edge((120.0, 98.0), (62.0, 20.0), 12, &mut pts); // up to the apex
        edge((62.0, 20.0), (0.0, 102.0), 11, &mut pts); // back, short of start
        let (min_x, min_y, max_x, max_y) = bbox(&pts);
        let diag = (max_x - min_x).hypot(max_y - min_y);
        let t = basic_triangle_corners(&pts, diag);
        // Apex up: exactly two corners on the bbox bottom (horizontal base), the
        // third (apex) on the bbox top, centered.
        let base: Vec<&(f64, f64)> = t.iter().filter(|p| (p.1 - max_y).abs() < 1e-6).collect();
        assert_eq!(base.len(), 2, "horizontal base on the bbox bottom: {t:?}");
        let apex = t.iter().find(|p| (p.1 - min_y).abs() < 1e-6).expect("apex on the bbox top");
        assert!((apex.0 - (min_x + max_x) / 2.0).abs() < 1e-6, "apex centered (isosceles): {t:?}");
        let mut bx: Vec<f64> = base.iter().map(|p| p.0).collect();
        bx.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert!(
            (bx[0] - min_x).abs() < 1e-6 && (bx[1] - max_x).abs() < 1e-6,
            "base spans the full bbox width: {t:?}"
        );
    }
}
