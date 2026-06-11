//! Multi-stroke endpoint merge (anchor-semantics v3 §4 follow-up).
//!
//! A user draws a rect in three strokes — left vertical, then an ㄱ (top +
//! right), then the bottom bar. Per-stroke recognition alone leaves three
//! independent open objects. This module makes strokes COMPOSE: when a freehand
//! release recognizes OPEN and one of its ends lands within the endpoint
//! tolerance of an existing open-class object's endpoint (node 0 / last, world
//! space), the stroke MERGES into that object instead of inserting:
//!
//!   (i)  one end matches            → existing path + stroke chain into one
//!        open path;
//!   (ii) the two ends match TWO     → A + stroke + B chain three ways; the
//!        different objects            start-side match survives, B is deleted;
//!   (iii) the two ends match the    → the stroke closes that object into a
//!        SAME object's two ends       ring (closed-class).
//!
//! The merged sample sequence is the matched geometry flattened to world px
//! (beziers at a fixed step, [`FLATTEN_STEPS_PER_CURVE`]) oriented around the
//! junction(s), plus the raw stroke samples (their matched ends snapped onto
//! the junction). It is then RE-RECOGNIZED through [`recognize_stroke`]: a
//! sequence that closes (the trim ladder included) canonicalizes per the pen
//! mode — Basic snaps the 3-stroke rect to THE rect — while a still-open chain
//! keeps the silhouette-preserving Free pipeline (Basic's open force-line
//! would collapse a multi-stroke chain to its chord, destroying the corners
//! the user just drew; a truly straight chain still resolves to the 2-node
//! line through the Free line fit).
//!
//! Ops (one shell batch, forward authoring only — op-apply untouched): ONE
//! `edit-geometry` on the survivor (the merged recognition mapped world →
//! inv(T_survivor), Q=8 — id/style preserved), a `set-anchor` rewrite that
//! RELEASES anchors on endpoints the junction turned into interior/ring nodes
//! (the far endpoint's anchor survives, remapped to its new pair index; a
//! closed result clears all of them, DU7=(b)), and a `delete` of the absorbed
//! case-(ii) object. The absorbed object's geometry moves into the survivor, so
//! its OWN far-endpoint anchor (an outward anchor to a third object) TRANSFERS
//! onto the survivor's new far node before the `delete` (its junction-end anchor
//! is consumed by the merge and dropped). The new stroke is NEVER inserted.
//! `None` = no merge: the caller keeps the existing insert + release anchoring
//! path. Endpoint merge takes PRIORITY over release-anchor authoring.
//!
//! Pure (no time/rng/IO), pointer-width-agnostic; recognition + merge run once
//! at pen-up (no per-frame cost). The tolerance is WORLD px — the shell
//! converts its screen-px constant through the zoom (the existing snap
//! convention).

use crate::object::anchor_follow::{affine_of, apply_affine, invert_affine, local_nodes};
use crate::object::deform::round_unit;
use crate::object::model::{
    path_string, Anchor, Geometry, HandlePoint, ObjectScene, PathNode, SubPath, Transform3x3,
    GEOMETRY_QUANTUM_PER_PX,
};
use crate::object::op::ObjectOp;
use crate::object::recognize::{recognize_stroke, RecognizeMode};

/// Quantized units per logical pixel (Q=8).
const UNITS_PER_PX: f64 = GEOMETRY_QUANTUM_PER_PX as f64;

/// Fixed-step sample count per cubic segment when flattening a matched
/// object's geometry into the merged world sequence (pen-up one-shot cost).
const FLATTEN_STEPS_PER_CURVE: usize = 16;

/// One matched open-class endpoint: which scene object, which end (`node_last`
/// = the last pair vs pair 0), its world-px position (the junction), and the
/// match distance (squared) for nearest-candidate selection.
struct EndpointHit {
    index: usize,
    node_last: bool,
    world: (f64, f64),
    dist_sq: f64,
}

/// The merge entry (module doc): the ops merging a released freehand stroke
/// (raw world-px samples) into the open-class object(s) whose endpoint(s) its
/// ends landed on, or `None` when nothing merges — fewer than 2 samples, no
/// endpoint within `tolerance_px` (world px), or the stroke recognizes CLOSED
/// by itself (a self-closed shape commits through the normal insert path).
pub fn merge_open_stroke_ops(
    scene: &ObjectScene,
    stroke_points_world: &[(f64, f64)],
    mode: RecognizeMode,
    tolerance_px: f64,
) -> Option<Vec<ObjectOp>> {
    if stroke_points_world.len() < 2 || !(tolerance_px > 0.0) {
        return None;
    }
    // Each stroke end matches independently; nearest candidate wins. A
    // degenerate double-match of ONE endpoint (a short hook landing where it
    // started) keeps only the start-side match.
    let start_hit = nearest_open_endpoint(scene, stroke_points_world[0], tolerance_px);
    let end_hit = match (
        &start_hit,
        nearest_open_endpoint(
            scene,
            stroke_points_world[stroke_points_world.len() - 1],
            tolerance_px,
        ),
    ) {
        (Some(a), Some(b)) if a.index == b.index && a.node_last == b.node_last => None,
        (_, hit) => hit,
    };
    if start_hit.is_none() && end_hit.is_none() {
        return None;
    }
    // The merge gate: only an OPEN recognition chains — a stroke that closes
    // by itself commits as its own shape through the normal insert path. (For
    // open results recognition preserves the input endpoints exactly, so the
    // raw-endpoint candidate scan above already matched the right points.)
    if recognize_stroke(stroke_points_world, mode).closed {
        return None;
    }

    // The stroke samples, matched ends snapped onto their junctions (the
    // existing geometry's endpoint is the truth the new stroke landed near).
    let mut stroke_pts = stroke_points_world.to_vec();
    if let Some(hit) = &start_hit {
        stroke_pts[0] = hit.world;
    }
    if let Some(hit) = &end_hit {
        let last = stroke_pts.len() - 1;
        stroke_pts[last] = hit.world;
    }
    // A matched object flattened to world px, oriented so its junction
    // endpoint sits LAST (`junction_last`) or FIRST in the chain.
    let object_chain = |hit: &EndpointHit, junction_last: bool| -> Option<Vec<(f64, f64)>> {
        let object = &scene.objects[hit.index];
        let nodes = open_subpath_nodes(&object.geometry.path_string)?;
        let mut pts = flatten_nodes_world(&nodes, &object.transform);
        if hit.node_last != junction_last {
            pts.reverse();
        }
        Some(pts)
    };
    // Chain assembly (junction points appear exactly once — the snapped
    // stroke ends duplicate the adjacent chain's boundary, so they drop).
    let (merged, survivor_hit, survivor_chain_first, absorbed_index) = match (&start_hit, &end_hit)
    {
        // (iii) both ends on the SAME object's two endpoints: close the ring.
        (Some(a), Some(b)) if a.index == b.index => {
            let mut pts = object_chain(a, true)?;
            pts.extend_from_slice(&stroke_pts[1..]);
            (pts, a, true, None)
        }
        // (ii) two objects: A + stroke + B; A survives, B is absorbed.
        (Some(a), Some(b)) => {
            let mut pts = object_chain(a, true)?;
            pts.extend_from_slice(&stroke_pts[1..]);
            let b_pts = object_chain(b, false)?;
            pts.extend_from_slice(&b_pts[1..]);
            (pts, a, true, Some(b.index))
        }
        // (i) one end matched.
        (Some(a), None) => {
            let mut pts = object_chain(a, true)?;
            pts.extend_from_slice(&stroke_pts[1..]);
            (pts, a, true, None)
        }
        (None, Some(b)) => {
            let mut pts = stroke_pts.clone();
            let b_pts = object_chain(b, false)?;
            pts.extend_from_slice(&b_pts[1..]);
            (pts, b, false, None)
        }
        (None, None) => return None,
    };

    // Re-recognize the merged sequence. A chain that stays open must keep its
    // silhouette: Basic's open force-line would collapse the corners the user
    // just chained, so the open case rides the Free pipeline (a truly straight
    // chain still resolves to the 2-node line through its line fit).
    let mut rec = recognize_stroke(&merged, mode);
    if !rec.closed && mode == RecognizeMode::Basic {
        rec = recognize_stroke(&merged, RecognizeMode::Free);
    }

    let survivor = &scene.objects[survivor_hit.index];
    let local_d = world_d_to_local(&rec.d, &survivor.transform)?;
    let mut ops = vec![ObjectOp::EditGeometry {
        id: survivor.id.clone(),
        geometry: Geometry {
            path_string: local_d.clone(),
            fill_rule: survivor.geometry.fill_rule,
            subpaths: Vec::new(),
        },
    }];

    // Anchor rewrite: the junction endpoint became an interior node — release
    // its anchor. The far endpoint stays an endpoint (open recognition
    // preserves it exactly), so its anchor survives at its new pair index. A
    // closed result clears everything (anchors live on open-class endpoints
    // only, DU7=(b)). Anchor `node_index` addresses PAIR space (`local_nodes`).
    let new_anchors: Vec<Anchor> = if rec.closed {
        Vec::new()
    } else {
        let survivor_last = i32::try_from(local_nodes(&local_d).len().checked_sub(1)?).ok()?;
        let old_last = i32::try_from(
            local_nodes(&survivor.geometry.path_string).len().checked_sub(1)?,
        )
        .ok()?;
        let far_old = if survivor_hit.node_last { 0 } else { old_last };
        let far_new = if survivor_chain_first { 0 } else { survivor_last };
        let mut kept: Vec<Anchor> = survivor
            .anchors
            .iter()
            .filter(|a| a.node_index == far_old)
            .cloned()
            .map(|mut a| {
                a.node_index = far_new;
                a
            })
            .collect();
        // Case (ii): the absorbed object's geometry was appended after the
        // survivor's start-side far end, so its OWN far-endpoint anchors (an
        // anchor ON that endpoint pointing at a third object) must move with the
        // geometry onto the survivor's NEW far node (the chain's last index).
        // `target`/`at` live in the third object's frame, so only `node_index`
        // remaps — the junction-end anchor of the absorbed object is consumed
        // and dropped. The start-side survivor's anchors are handled above.
        if let Some(index) = absorbed_index {
            let absorbed = &scene.objects[index];
            let b_last = i32::try_from(
                local_nodes(&absorbed.geometry.path_string).len().checked_sub(1)?,
            )
            .ok()?;
            // The absorbed object chains junction-first, so its far endpoint
            // becomes the survivor's last node.
            let b_far_old = if end_hit.as_ref().is_some_and(|b| b.node_last) { 0 } else { b_last };
            kept.extend(absorbed.anchors.iter().filter(|a| a.node_index == b_far_old).cloned().map(
                |mut a| {
                    a.node_index = survivor_last;
                    a
                },
            ));
        }
        kept
    };
    if new_anchors != survivor.anchors {
        ops.push(ObjectOp::SetAnchor { id: survivor.id.clone(), anchors: new_anchors });
    }
    if let Some(index) = absorbed_index {
        ops.push(ObjectOp::Delete { id: scene.objects[index].id.clone() });
    }
    Some(ops)
}

/// The single open subpath's nodes of an open-class path-string (§1: exactly
/// one subpath, not closed, at least two nodes). `None` = not a candidate.
fn open_subpath_nodes(d: &str) -> Option<Vec<PathNode>> {
    let subpaths = path_string::parse(d).ok()?;
    if subpaths.len() != 1 || subpaths[0].closed {
        return None;
    }
    let sub = subpaths.into_iter().next()?;
    (sub.nodes.len() >= 2).then_some(sub.nodes)
}

/// Cubic bezier point at `t` (control points absolute).
fn cubic_point(
    p0: (f64, f64),
    c1: (f64, f64),
    c2: (f64, f64),
    p3: (f64, f64),
    t: f64,
) -> (f64, f64) {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    (
        a * p0.0 + b * c1.0 + c * c2.0 + d * p3.0,
        a * p0.1 + b * c1.1 + c * c2.1 + d * p3.1,
    )
}

/// Flatten an open subpath's nodes to world-px samples under `transform`:
/// line segments contribute their endpoints, bezier segments a fixed
/// [`FLATTEN_STEPS_PER_CURVE`] sweep of their absolute control polygon.
fn flatten_nodes_world(nodes: &[PathNode], transform: &Transform3x3) -> Vec<(f64, f64)> {
    let to_world = |x: f64, y: f64| transform.apply_point(x / UNITS_PER_PX, y / UNITS_PER_PX);
    let mut out = Vec::with_capacity(nodes.len());
    out.push(to_world(f64::from(nodes[0].x), f64::from(nodes[0].y)));
    for w in nodes.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if a.out_handle.is_none() && b.in_handle.is_none() {
            out.push(to_world(f64::from(b.x), f64::from(b.y)));
            continue;
        }
        let p0 = (f64::from(a.x), f64::from(a.y));
        let p3 = (f64::from(b.x), f64::from(b.y));
        let c1 = a.out_handle.map_or(p0, |h| (f64::from(a.x + h.dx), f64::from(a.y + h.dy)));
        let c2 = b.in_handle.map_or(p3, |h| (f64::from(b.x + h.dx), f64::from(b.y + h.dy)));
        for i in 1..=FLATTEN_STEPS_PER_CURVE {
            let t = i as f64 / FLATTEN_STEPS_PER_CURVE as f64;
            let (x, y) = cubic_point(p0, c1, c2, p3, t);
            out.push(to_world(x, y));
        }
    }
    out
}

/// The nearest open-class endpoint (across every scene object's node 0 / last)
/// within `tolerance_px` of the world point, or `None`.
fn nearest_open_endpoint(
    scene: &ObjectScene,
    point: (f64, f64),
    tolerance_px: f64,
) -> Option<EndpointHit> {
    let tol_sq = tolerance_px * tolerance_px;
    let mut best: Option<EndpointHit> = None;
    for (index, object) in scene.objects.iter().enumerate() {
        let Some(nodes) = open_subpath_nodes(&object.geometry.path_string) else {
            continue;
        };
        for (node_last, node) in [(false, &nodes[0]), (true, &nodes[nodes.len() - 1])] {
            let world = object
                .transform
                .apply_point(f64::from(node.x) / UNITS_PER_PX, f64::from(node.y) / UNITS_PER_PX);
            let dx = world.0 - point.0;
            let dy = world.1 - point.1;
            let dist_sq = dx * dx + dy * dy;
            if dist_sq <= tol_sq && best.as_ref().is_none_or(|b| dist_sq < b.dist_sq) {
                best = Some(EndpointHit { index, node_last, world, dist_sq });
            }
        }
    }
    best
}

/// Rewrite a recognized world-units path-string into `transform`'s local space
/// (world → inv(T), the same Q=8 round-quantization as the chord deform),
/// bezier control points mapped ABSOLUTELY like `deform_open_path`.
fn world_d_to_local(d: &str, transform: &Transform3x3) -> Option<String> {
    let subpaths = path_string::parse(d).ok()?;
    let sub = subpaths.into_iter().next()?;
    let inv = invert_affine(&affine_of(transform));
    let map = |x: f64, y: f64| -> (f64, f64) {
        let (lx, ly) = apply_affine(&inv, x / UNITS_PER_PX, y / UNITS_PER_PX);
        (lx * UNITS_PER_PX, ly * UNITS_PER_PX)
    };
    let nodes = sub
        .nodes
        .iter()
        .map(|n| {
            let (x, y) = map(f64::from(n.x), f64::from(n.y));
            let (qx, qy) = (round_unit(x), round_unit(y));
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
    Some(path_string::serialize(&[SubPath { closed: sub.closed, nodes }]))
}

// ---------------------------------------------------------------------------
// Tests — the 3-stroke rect acceptance + the merge decision table.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::apply::apply_object_op;
    use crate::object::model::{FillRule, LocalPoint, Object, ObjectSelection};

    /// World-px endpoint tolerance the shell passes at zoom 1 (12 screen px).
    const TOL: f64 = 12.0;

    fn hydrated(d: &str) -> Geometry {
        let mut g = Geometry {
            path_string: d.to_string(),
            fill_rule: FillRule::EvenOdd,
            subpaths: Vec::new(),
        };
        g.ensure_parsed().unwrap();
        g
    }

    fn scene_of(objects: Vec<Object>) -> ObjectScene {
        ObjectScene {
            scene_version: 1,
            objects,
            tags: Vec::new(),
            selection: ObjectSelection::Canvas,
            updated_at: String::new(),
        }
    }

    fn open_object(id: &str, d: &str, tx: f64, ty: f64) -> Object {
        let mut o = Object::new(id, "a0", hydrated(d));
        o.transform = Transform3x3::translate(tx, ty);
        o
    }

    /// A closed rect (never a merge candidate) for anchor-target fixtures.
    fn closed_box(id: &str, tx: f64, ty: f64) -> Object {
        let mut o = Object::new(id, "a9", hydrated("M 0 0 L 640 0 L 640 320 L 0 320 Z"));
        o.transform = Transform3x3::translate(tx, ty);
        o
    }

    /// Samples along the segment `a -> b` (excluding `b`), `n` per edge.
    fn edge(a: (f64, f64), b: (f64, f64), n: usize, out: &mut Vec<(f64, f64)>) {
        for i in 0..n {
            let t = i as f64 / n as f64;
            out.push((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
        }
    }

    /// Stroke 2 of the 3-stroke rect: the left vertical exists, an ㄱ (top +
    /// right) releases on its TOP endpoint — case (i): ONE open path via
    /// edit-geometry on the survivor, no insert, nothing deleted.
    #[test]
    fn second_stroke_chains_into_the_existing_open_path() {
        let scene = scene_of(vec![open_object("left", "M 0 800 L 0 0", 100.0, 100.0)]);
        let mut pts = Vec::new();
        edge((100.0, 100.0), (200.0, 100.0), 10, &mut pts);
        edge((200.0, 100.0), (200.0, 200.0), 10, &mut pts);
        pts.push((200.0, 200.0));
        let ops = merge_open_stroke_ops(&scene, &pts, RecognizeMode::Basic, TOL)
            .expect("an endpoint hit merges");
        assert_eq!(ops.len(), 1, "one edit-geometry, nothing else: {ops:?}");
        let ObjectOp::EditGeometry { id, geometry } = &ops[0] else {
            panic!("expected edit-geometry: {ops:?}");
        };
        assert_eq!(id, "left");
        // Survivor-local (world − translate(100,100), Q=8): the chained ⊓
        // silhouette, both corners preserved, still OPEN.
        assert_eq!(geometry.path_string, "M 0 800 L 0 0 L 800 0 L 800 800");
    }

    /// THE acceptance: stroke 3 (the bottom bar) lands on BOTH endpoints of
    /// the chained ⊓ — case (iii) closes the ring and Basic re-recognition
    /// snaps it to the canonical rect. Three strokes = one rect object.
    #[test]
    fn third_stroke_closes_the_chain_into_a_basic_rect() {
        let mut scene = scene_of(vec![open_object("left", "M 0 800 L 0 0", 100.0, 100.0)]);
        let mut gamma = Vec::new();
        edge((100.0, 100.0), (200.0, 100.0), 10, &mut gamma);
        edge((200.0, 100.0), (200.0, 200.0), 10, &mut gamma);
        gamma.push((200.0, 200.0));
        for op in merge_open_stroke_ops(&scene, &gamma, RecognizeMode::Basic, TOL)
            .expect("stroke 2 merges")
        {
            apply_object_op(&mut scene, op).expect("stroke-2 ops apply");
        }
        let mut bar = Vec::new();
        edge((100.0, 200.0), (200.0, 200.0), 10, &mut bar);
        bar.push((200.0, 200.0));
        let ops = merge_open_stroke_ops(&scene, &bar, RecognizeMode::Basic, TOL)
            .expect("both bar ends match the survivor's endpoints");
        assert_eq!(ops.len(), 1, "{ops:?}");
        let ObjectOp::EditGeometry { id, geometry } = &ops[0] else {
            panic!("expected edit-geometry: {ops:?}");
        };
        assert_eq!(id, "left");
        assert_eq!(geometry.path_string, "M 0 0 L 800 0 L 800 800 L 0 800 Z");
    }

    /// Case (iii) on a U: both stroke ends on the SAME object's two endpoints
    /// close it — and a ring carries no endpoint anchors (DU7=(b)), so the
    /// rewrite clears the survivor's anchor vector.
    #[test]
    fn stroke_across_both_ends_of_a_u_object_closes_it() {
        let mut u = open_object("u", "M 0 0 L 0 800 L 800 800 L 800 0", 0.0, 0.0);
        u.anchors =
            vec![Anchor { node_index: 0, target: "box".into(), at: LocalPoint { x: 0, y: 0 } }];
        let scene = scene_of(vec![u, closed_box("box", 300.0, 300.0)]);
        let mut pts = Vec::new();
        edge((1.0, 1.0), (99.0, 1.0), 10, &mut pts);
        pts.push((99.0, 1.0));
        let ops = merge_open_stroke_ops(&scene, &pts, RecognizeMode::Basic, TOL)
            .expect("both ends hit the same object's endpoints");
        assert_eq!(ops.len(), 2, "edit + anchor clear: {ops:?}");
        let ObjectOp::EditGeometry { id, geometry } = &ops[0] else {
            panic!("expected edit-geometry first: {ops:?}");
        };
        assert_eq!(id, "u");
        assert_eq!(geometry.path_string, "M 0 0 L 800 0 L 800 800 L 0 800 Z");
        let ObjectOp::SetAnchor { id, anchors } = &ops[1] else {
            panic!("expected set-anchor second: {ops:?}");
        };
        assert_eq!(id, "u");
        assert!(anchors.is_empty(), "a closed ring carries no endpoint anchors");
    }

    /// Outside the endpoint tolerance nothing merges — the caller keeps the
    /// existing insert path.
    #[test]
    fn release_outside_the_tolerance_does_not_merge() {
        let scene = scene_of(vec![open_object("left", "M 0 800 L 0 0", 100.0, 100.0)]);
        let mut pts = Vec::new();
        // Start 15.8px from the nearest endpoint (100,100): over the 12px bar.
        edge((115.0, 105.0), (215.0, 105.0), 10, &mut pts);
        pts.push((215.0, 105.0));
        assert!(merge_open_stroke_ops(&scene, &pts, RecognizeMode::Basic, TOL).is_none());
    }

    /// Survivor with a non-zero translate: the merged geometry lands in the
    /// survivor's LOCAL space (world → inv(T_survivor), Q=8) — a collinear
    /// extension re-fits to one straight line through the survivor's frame.
    #[test]
    fn merged_geometry_is_rewritten_into_the_survivors_local_space() {
        let scene = scene_of(vec![open_object("seg", "M 0 0 L 800 0", 50.0, 30.0)]);
        let mut pts = Vec::new();
        edge((151.0, 31.0), (250.0, 30.0), 10, &mut pts);
        pts.push((250.0, 30.0));
        let ops =
            merge_open_stroke_ops(&scene, &pts, RecognizeMode::Basic, TOL).expect("merges");
        assert_eq!(ops.len(), 1, "{ops:?}");
        let ObjectOp::EditGeometry { id, geometry } = &ops[0] else {
            panic!("expected edit-geometry: {ops:?}");
        };
        assert_eq!(id, "seg");
        // World (50,30)→(250,30) mapped through inv(translate(50,30)).
        assert_eq!(geometry.path_string, "M 0 0 L 1600 0");
    }

    /// The junction endpoint's anchor is RELEASED (that endpoint became an
    /// interior node); the far endpoint's anchor survives at its new index.
    #[test]
    fn absorbed_junction_endpoint_anchor_is_released() {
        let far = Anchor { node_index: 0, target: "box".into(), at: LocalPoint { x: 0, y: 0 } };
        let junction =
            Anchor { node_index: 1, target: "box".into(), at: LocalPoint { x: 640, y: 0 } };
        let mut seg = open_object("seg", "M 0 0 L 800 0", 0.0, 0.0);
        seg.anchors = vec![far.clone(), junction];
        let scene = scene_of(vec![seg, closed_box("box", 300.0, 300.0)]);
        let mut pts = Vec::new();
        edge((101.0, 1.0), (200.0, 0.0), 10, &mut pts);
        pts.push((200.0, 0.0));
        let ops =
            merge_open_stroke_ops(&scene, &pts, RecognizeMode::Basic, TOL).expect("merges");
        assert_eq!(ops.len(), 2, "edit + anchor rewrite: {ops:?}");
        let ObjectOp::SetAnchor { id, anchors } = &ops[1] else {
            panic!("expected set-anchor second: {ops:?}");
        };
        assert_eq!(id, "seg");
        // Node 1 (the junction) released; node 0 (still the start) kept.
        assert_eq!(anchors, &vec![far]);
    }

    /// A stroke that CLOSES BY ITSELF never merges, even released on an
    /// endpoint — it commits as its own shape through the normal insert path.
    #[test]
    fn a_self_closed_stroke_never_merges() {
        let scene = scene_of(vec![open_object("seg", "M 0 0 L 800 0", 0.0, 0.0)]);
        // A circle whose pen-down/pen-up sit ~3px from seg's (100,0) endpoint.
        let pts: Vec<(f64, f64)> = (0..36)
            .map(|i| {
                let theta = f64::from(i) * 10.0_f64.to_radians();
                (95.0 + 8.0 * theta.cos(), 8.0 * theta.sin())
            })
            .collect();
        assert!(merge_open_stroke_ops(&scene, &pts, RecognizeMode::Basic, TOL).is_none());
    }

    /// Case (ii): the stroke bridges TWO objects — A + stroke + B chain into
    /// one path on the start-side survivor; the absorbed B is deleted in the
    /// same batch.
    #[test]
    fn bridging_stroke_chains_two_objects_and_deletes_the_absorbed_one() {
        let scene = scene_of(vec![
            open_object("a", "M 0 0 L 800 0", 0.0, 0.0),
            open_object("b", "M 0 0 L 0 800", 200.0, 0.0),
        ]);
        let mut pts = Vec::new();
        edge((101.0, 1.0), (199.0, 1.0), 10, &mut pts);
        pts.push((199.0, 1.0));
        let ops =
            merge_open_stroke_ops(&scene, &pts, RecognizeMode::Basic, TOL).expect("merges");
        assert_eq!(ops.len(), 2, "edit + delete: {ops:?}");
        let ObjectOp::EditGeometry { id, geometry } = &ops[0] else {
            panic!("expected edit-geometry first: {ops:?}");
        };
        assert_eq!(id, "a", "the start-side match survives");
        assert_eq!(geometry.path_string, "M 0 0 L 1600 0 L 1600 800");
        assert_eq!(ops[1], ObjectOp::Delete { id: "b".into() });
    }

    /// Case (ii) anchor transfer: the absorbed object B carries an OUTWARD anchor
    /// on its FAR endpoint (the non-junction end) pointing at a third object Y.
    /// B's geometry moves into the survivor, so that anchor must follow onto the
    /// survivor's new far node — `target`/`at` preserved (they live in Y's frame),
    /// only `node_index` remapped. Driven through real op-apply.
    #[test]
    fn bridging_transfers_the_absorbed_objects_far_anchor_to_the_survivor() {
        // B = "M 0 0 L 0 800" at (200,0): node 0 = world (200,0) is the JUNCTION
        // (the stroke lands there); node 1 = world (200,100) is the FAR endpoint,
        // and it anchors out to Y.
        let far = Anchor { node_index: 1, target: "y".into(), at: LocalPoint { x: 48, y: 16 } };
        let mut b = open_object("b", "M 0 0 L 0 800", 200.0, 0.0);
        b.anchors = vec![far.clone()];
        let mut scene = scene_of(vec![
            open_object("a", "M 0 0 L 800 0", 0.0, 0.0),
            b,
            closed_box("y", 500.0, 500.0),
        ]);
        let mut pts = Vec::new();
        edge((101.0, 1.0), (199.0, 1.0), 10, &mut pts);
        pts.push((199.0, 1.0));
        let ops =
            merge_open_stroke_ops(&scene, &pts, RecognizeMode::Basic, TOL).expect("merges");
        // edit + anchor transfer + delete (the start-side survivor "a" had no
        // anchors, so the only set-anchor authored is the transfer from "b").
        assert_eq!(ops.len(), 3, "edit + anchor transfer + delete: {ops:?}");
        for op in ops {
            apply_object_op(&mut scene, op).expect("merge ops apply");
        }
        // B is gone; the survivor "a" carries B's far anchor at its NEW far node.
        assert!(scene.objects.iter().all(|o| o.id != "b"), "absorbed B is deleted");
        let survivor = scene.objects.iter().find(|o| o.id == "a").expect("survivor present");
        // Survivor geometry "M 0 0 L 1600 0 L 1600 800" has 3 nodes; B's far end
        // is the LAST node (index 2). target/at unchanged from B's original.
        assert_eq!(
            survivor.anchors,
            vec![Anchor { node_index: 2, target: "y".into(), at: LocalPoint { x: 48, y: 16 } }],
            "B's far anchor transferred to the survivor's last node, target/at preserved",
        );
    }
}
