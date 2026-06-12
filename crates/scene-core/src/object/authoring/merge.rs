//! Multi-stroke endpoint merge: when a freehand release recognizes OPEN and one
//! of its ends lands within `tolerance_px` of an existing open-class object's
//! endpoint (node 0 / last, world space), the stroke MERGES into that object
//! instead of inserting:
//!   (i)   one end matches      -> existing path + stroke chain into one open path;
//!   (ii)  two ends match two   -> A + stroke + B chain; start-side survives, B
//!         different objects        deleted;
//!   (iii) two ends match the   -> the stroke closes that object into a ring.
//!         SAME object's ends
//!
//! The matched geometry is flattened to world px ([`FLATTEN_STEPS_PER_CURVE`] per
//! curve), oriented around the junction(s), plus the raw stroke samples (matched
//! ends snapped onto the junction), then RE-RECOGNIZED. A closing sequence
//! canonicalizes per pen mode; a still-open chain keeps the Free pipeline (Basic's
//! open force-line would collapse the chain to its chord).
//!
//! Ops (one batch, forward authoring only): ONE `edit-geometry` on the survivor
//! (merged recognition mapped through inv(T_survivor), Q=8), a `set-anchor` that
//! RELEASES anchors on endpoints the junction turned into interior/ring nodes (the
//! far endpoint's anchor survives, remapped; a closed result clears all), and a
//! `delete` of the absorbed case-(ii) object. The absorbed object's OWN far-endpoint
//! anchor TRANSFERS onto the survivor's new far node before the `delete`. The new
//! stroke is never inserted; `None` = no merge, the caller keeps insert + release.
//! Endpoint merge takes priority over release-anchor authoring.
//!
//! Pure (no time/rng/IO); merge runs once at pen-up. Tolerance is WORLD px.

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

/// Fixed-step sample count per cubic segment when flattening a matched object's
/// geometry into the merged world sequence.
const FLATTEN_STEPS_PER_CURVE: usize = 16;

/// One matched open-class endpoint: which object, which end (`node_last`), its
/// world-px junction position, and the squared match distance.
struct EndpointHit {
    index: usize,
    node_last: bool,
    world: (f64, f64),
    dist_sq: f64,
}

/// The ops merging a released freehand stroke into the open-class object(s) its
/// ends landed on, or `None` when nothing merges — fewer than 2 samples, no
/// endpoint within `tolerance_px`, or the stroke recognizes CLOSED by itself.
pub fn merge_open_stroke_ops(
    scene: &ObjectScene,
    stroke_points_world: &[(f64, f64)],
    mode: RecognizeMode,
    tolerance_px: f64,
) -> Option<Vec<ObjectOp>> {
    if stroke_points_world.len() < 2 || !(tolerance_px > 0.0) {
        return None;
    }
    // Each stroke end matches independently; nearest candidate wins. A degenerate
    // double-match of ONE endpoint keeps only the start-side match.
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
    // Only an OPEN recognition chains; a self-closing stroke commits through the
    // normal insert path.
    if recognize_stroke(stroke_points_world, mode).closed {
        return None;
    }

    // Snap matched ends onto their junctions (the existing endpoint is the truth).
    let mut stroke_pts = stroke_points_world.to_vec();
    if let Some(hit) = &start_hit {
        stroke_pts[0] = hit.world;
    }
    if let Some(hit) = &end_hit {
        let last = stroke_pts.len() - 1;
        stroke_pts[last] = hit.world;
    }
    // A matched object flattened to world px, oriented so its junction endpoint
    // sits LAST (`junction_last`) or FIRST in the chain.
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

    // An open chain rides the Free pipeline so Basic's force-line cannot collapse
    // the chained corners.
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

    // The junction endpoint became an interior node — release its anchor. The far
    // endpoint stays an endpoint, so its anchor survives at its new pair index. A
    // closed result clears everything (anchors live on open-class endpoints only).
    // `node_index` addresses PAIR space (`local_nodes`).
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
        // Case (ii): the absorbed object's OWN far-endpoint anchor (pointing at a
        // third object) moves with its geometry onto the survivor's new far node;
        // only `node_index` remaps (`target`/`at` live in the third frame). The
        // absorbed junction-end anchor is consumed and dropped.
        if let Some(index) = absorbed_index {
            let absorbed = &scene.objects[index];
            let b_last = i32::try_from(
                local_nodes(&absorbed.geometry.path_string).len().checked_sub(1)?,
            )
            .ok()?;
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

/// The single open subpath's nodes of an open-class path-string (one subpath, not
/// closed, >= 2 nodes). `None` = not a candidate.
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
/// (world -> inv(T), same Q=8 round-quantization as the chord deform); control
/// points mapped ABSOLUTELY.
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

    /// Case (i): a second stroke releasing on an existing open path's endpoint
    /// chains into ONE open path via edit-geometry; no insert, nothing deleted.
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
        // Survivor-local (world − translate(100,100), Q=8): both corners preserved,
        // still OPEN.
        assert_eq!(geometry.path_string, "M 0 800 L 0 0 L 800 0 L 800 800");
    }

    /// Case (iii): a third stroke landing on BOTH endpoints of the chained ⊓
    /// closes the ring and Basic re-recognition snaps it to the canonical rect.
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
    /// close it; a ring carries no endpoint anchors, so the rewrite clears them.
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

    #[test]
    fn release_outside_the_tolerance_does_not_merge() {
        let scene = scene_of(vec![open_object("left", "M 0 800 L 0 0", 100.0, 100.0)]);
        let mut pts = Vec::new();
        // Start 15.8px from the nearest endpoint: over the 12px bar.
        edge((115.0, 105.0), (215.0, 105.0), 10, &mut pts);
        pts.push((215.0, 105.0));
        assert!(merge_open_stroke_ops(&scene, &pts, RecognizeMode::Basic, TOL).is_none());
    }

    /// Survivor with a non-zero translate: the merged geometry lands in its LOCAL
    /// space (world -> inv(T_survivor), Q=8).
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

    /// The junction endpoint's anchor is released (it became interior); the far
    /// endpoint's anchor survives at its new index.
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

    /// A stroke that closes by itself never merges, even released on an endpoint.
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

    /// Case (ii): a bridging stroke chains A + stroke + B onto the start-side
    /// survivor; the absorbed B is deleted in the same batch.
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

    /// Case (ii) anchor transfer: the absorbed B's far-endpoint anchor (pointing at
    /// a third object Y) follows its geometry onto the survivor's new far node;
    /// `target`/`at` preserved (Y's frame), only `node_index` remapped.
    #[test]
    fn bridging_transfers_the_absorbed_objects_far_anchor_to_the_survivor() {
        // B at (200,0): node 0 = world (200,0) is the JUNCTION; node 1 = world
        // (200,100) is the FAR endpoint, anchored out to Y.
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
        assert_eq!(ops.len(), 3, "edit + anchor transfer + delete: {ops:?}");
        for op in ops {
            apply_object_op(&mut scene, op).expect("merge ops apply");
        }
        assert!(scene.objects.iter().all(|o| o.id != "b"), "absorbed B is deleted");
        let survivor = scene.objects.iter().find(|o| o.id == "a").expect("survivor present");
        // Survivor "M 0 0 L 1600 0 L 1600 800" has 3 nodes; B's far end is the last
        // (index 2), target/at unchanged.
        assert_eq!(
            survivor.anchors,
            vec![Anchor { node_index: 2, target: "y".into(), at: LocalPoint { x: 48, y: 16 } }],
            "B's far anchor transferred to the survivor's last node, target/at preserved",
        );
    }
}
