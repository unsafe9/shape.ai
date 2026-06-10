//! Tier-1/#14 — commit-time anchor move-together (the canonical SetTransform
//! reproject). The single source of truth for "what-you-drag-is-what-you-commit":
//! when a target object is moved by a `set-transform` op, every object anchored
//! onto that target reprojects its bound geometry node through the target's NEW
//! transform, so the anchored endpoint tracks the target without drift.
//!
//! Ported from the shell `anchorCreate.ts` (`anchorFollowOps` /
//! `reprojectAnchoredGeometry` / `synthesizeCreateAnchors`) so the canvas logic
//! lives in the Rust core; the shell now only batches the returned ops.
//!
//! The TRANSFORM reproject here is the canonical mirror of the renderer-core LIVE
//! preview (`transform_bindings::reproject_node_local_px` + `rewrite_geometry_node`):
//! both de-quantize the target-local anchor point by [`UNITS_PER_PX`], carry it to
//! world through the target's transform, map back into the follower's OWN local
//! pixel space (follower inverse), then quantize with the SAME `round()`. A
//! cross-core equivalence test (`reproject_matches_cross_core_vector`) pins one
//! concrete numeric vector in BOTH cores so either copy drifting fails its own test.
//!
//! Pure (no time/rng/IO/GPU), pointer-width-agnostic. Region-based anchor
//! resolution ([`super::anchors`]) is a SEPARATE axis (geometry-edit follow); this
//! module is the transform-based move-together only and must not be folded into it.

use super::model::{
    Anchor, Geometry, LocalPoint, Object, ObjectScene, Transform3x3, GEOMETRY_QUANTUM_PER_PX,
};
use super::op::ObjectOp;

/// Quantized units per logical pixel. Matches renderer-core
/// `transform_bindings::UNITS_PER_PX` and the shell `GEOMETRY_QUANTUM_PER_PX`
/// (Q=8); the reproject quantization MUST stay byte-equivalent across all three.
const UNITS_PER_PX: f64 = GEOMETRY_QUANTUM_PER_PX as f64;

/// A 2x3 affine (the top two rows of a row-major 3x3 with `g=h=0,i=1`), used for
/// the tiny invert/apply the reproject needs. A scene-core-local copy (no shared
/// crate); equivalent to the shell `applyTransform` / `invertAffine`.
type Affine = [[f64; 3]; 2];

/// The affine rows of a transform (absent => identity), dropping the projective
/// bottom row (anchors are affine-only, matching the shell `Transform3x3` shape).
fn affine_of(t: &Transform3x3) -> Affine {
    [
        [t.m[0][0], t.m[0][1], t.m[0][2]],
        [t.m[1][0], t.m[1][1], t.m[1][2]],
    ]
}

/// Apply a row-major affine to a point. Mirrors the shell `applyTransform`.
fn apply_affine(a: &Affine, x: f64, y: f64) -> (f64, f64) {
    (
        a[0][0] * x + a[0][1] * y + a[0][2],
        a[1][0] * x + a[1][1] * y + a[1][2],
    )
}

/// Invert a row-major affine (`g=h=0,i=1`). Returns identity when singular,
/// matching the shell `invertAffine` (a singular follower then no-ops the node).
fn invert_affine(a: &Affine) -> Affine {
    let (a00, a01, a02) = (a[0][0], a[0][1], a[0][2]);
    let (a10, a11, a12) = (a[1][0], a[1][1], a[1][2]);
    let det = a00 * a11 - a01 * a10;
    if det == 0.0 {
        return [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    }
    let ia = a11 / det;
    let ib = -a01 / det;
    let id = -a10 / det;
    let ie = a00 / det;
    [
        [ia, ib, -(ia * a02 + ib * a12)],
        [id, ie, -(id * a02 + ie * a12)],
    ]
}

/// The signed-decimal number spans of a path-string, matching the shell regex
/// `-?\d+(?:\.\d+)?` and renderer-core `NumberSpans`. Each `(start, end)` is a
/// half-open byte range; non-number characters between spans are skipped.
fn number_spans(d: &str) -> Vec<(usize, usize)> {
    let bytes = d.as_bytes();
    let n = bytes.len();
    let mut spans = Vec::new();
    let mut pos = 0usize;
    while pos < n {
        let b = bytes[pos];
        if b != b'-' && !b.is_ascii_digit() {
            pos += 1;
            continue;
        }
        let start = pos;
        if bytes[pos] == b'-' {
            pos += 1;
        }
        while pos < n && bytes[pos].is_ascii_digit() {
            pos += 1;
        }
        if pos < n && bytes[pos] == b'.' {
            pos += 1;
            while pos < n && bytes[pos].is_ascii_digit() {
                pos += 1;
            }
        }
        spans.push((start, pos));
    }
    spans
}

/// The object-local node points parsed from a path-string's M/L/C coords, in pair
/// order (M/L/C all contribute pairs). Mirrors the shell `localNodes`.
fn local_nodes(d: &str) -> Vec<(f64, f64)> {
    let spans = number_spans(d);
    let mut out = Vec::with_capacity(spans.len() / 2);
    let mut i = 0;
    while i + 1 < spans.len() {
        let x: f64 = d[spans[i].0..spans[i].1].parse().unwrap_or(0.0);
        let y: f64 = d[spans[i + 1].0..spans[i + 1].1].parse().unwrap_or(0.0);
        out.push((x, y));
        i += 2;
    }
    out
}

/// The geometry node index of `object` closest to the world point `(wx, wy)`,
/// after mapping the world point into the object's local quantized space (so the
/// match is transform-invariant). Returns `None` when the geometry has no nodes.
/// Mirrors the shell `nodeIndexNearestWorld`.
fn node_index_nearest_world(object: &Object, wx: f64, wy: f64) -> Option<i32> {
    let nodes = local_nodes(&object.geometry.path_string);
    if nodes.is_empty() {
        return None;
    }
    let inv = invert_affine(&affine_of(&object.transform));
    let (lx, ly) = apply_affine(&inv, wx, wy);
    let lx = lx * UNITS_PER_PX;
    let ly = ly * UNITS_PER_PX;
    let mut best = 0i32;
    let mut best_d = f64::INFINITY;
    for (i, (nx, ny)) in nodes.iter().enumerate() {
        let dx = nx - lx;
        let dy = ny - ly;
        let dist = dx * dx + dy * dy;
        if dist < best_d {
            best_d = dist;
            best = i32::try_from(i).unwrap_or(i32::MAX);
        }
    }
    Some(best)
}

/// Rewrite the `node_index`-th coordinate PAIR of a path-string `d` to the
/// quantized object-local ints `(x, y)`, preserving every command token and every
/// other coordinate. Returns `None` when the pair is unaddressable OR the rewrite
/// is a no-op (the new coords already match). Mirrors the shell `setPathNode`
/// (returning `None` on no-op so the caller skips a pointless edit) and is
/// byte-equivalent to renderer-core `rewrite_geometry_node`.
fn set_path_node(d: &str, node_index: i32, x: i64, y: i64) -> Option<String> {
    let spans = number_spans(d);
    let mut out = String::with_capacity(d.len());
    let mut last = 0usize;
    let mut pair: i32 = 0;
    let mut numbers_seen = 0usize;
    let mut hit = false;
    let mut changed = false;
    for (start, end) in spans {
        out.push_str(&d[last..start]);
        let is_x = numbers_seen % 2 == 0;
        let at_target = pair == node_index;
        if !is_x {
            pair += 1;
        }
        numbers_seen += 1;
        if at_target {
            hit = true;
            let replacement = if is_x { x } else { y };
            let original = &d[start..end];
            let replacement_str = replacement.to_string();
            if original != replacement_str {
                changed = true;
            }
            out.push_str(&replacement_str);
        } else {
            out.push_str(&d[start..end]);
        }
        last = end;
    }
    out.push_str(&d[last..]);
    if hit && changed {
        Some(out)
    } else {
        None
    }
}

/// The canonical TRANSFORM reproject: the follower-local QUANTIZED node position
/// of `anchor`'s bound node when its `target` carries `target_transform` (the NEW
/// transform, post-move).
///
/// Formula (shared with renderer-core `reproject_node_local_px`, then quantized
/// like `rewrite_geometry_node`):
///   1. de-quantize the target-local anchor point: `(at / UNITS_PER_PX)`
///   2. world point: `target_transform * (at / UNITS_PER_PX)`
///   3. follower-local px: `follower_transform^-1 * world`
///   4. quantize: `(px * UNITS_PER_PX).round() as i64`
///
/// A pure target translation therefore shifts the bound node by EXACTLY the same
/// world delta (identity bases), the falsifiable move-together property.
fn reproject_node_local_quantized(
    follower_transform: &Transform3x3,
    target_transform: &Transform3x3,
    at: LocalPoint,
) -> (i64, i64) {
    let lx = f64::from(at.x) / UNITS_PER_PX;
    let ly = f64::from(at.y) / UNITS_PER_PX;
    let (wx, wy) = apply_affine(&affine_of(target_transform), lx, ly);
    let inv = invert_affine(&affine_of(follower_transform));
    let (fx, fy) = apply_affine(&inv, wx, wy);
    let qx = (fx * UNITS_PER_PX).round() as i64;
    let qy = (fy * UNITS_PER_PX).round() as i64;
    (qx, qy)
}

/// Reproject `follower`'s node bound by `anchor` through `target`'s NEW transform,
/// returning the updated geometry — or `None` when the rewrite is a no-op (the
/// node already sits where the target puts it) or the node is unaddressable.
/// Mirrors the shell `reprojectAnchoredGeometry`.
///
/// W3-G13: the commit path ([`anchor_follow_ops`]) now folds EVERY moved-target
/// anchor into one cumulative rewrite via the same two helpers; this single-anchor
/// wrapper survives only as the cross-core pin's entry point
/// (`reproject_matches_cross_core_vector`), hence test-only.
#[cfg(test)]
fn reproject_anchored_geometry(
    follower: &Object,
    anchor: &Anchor,
    target_transform: &Transform3x3,
) -> Option<Geometry> {
    let (qx, qy) = reproject_node_local_quantized(&follower.transform, target_transform, anchor.at);
    let d = set_path_node(&follower.geometry.path_string, anchor.node_index, qx, qy)?;
    Some(Geometry {
        path_string: d,
        fill_rule: follower.geometry.fill_rule,
        subpaths: Vec::new(),
    })
}

/// The shared LIVE reproject: rewrite the `node_index`-th node of `geometry_d` to
/// where `at` lands when `target_transform` is moved by `delta`, returning the new
/// path-string — or `None` on a no-op / unaddressable node / singular follower.
///
/// The renderer holds the target's BASE transform and the drag `delta` separately
/// (the same `(target_base, delta)` shape its instance-matrix preview writes), so
/// this composes `delta * target_transform` to form the target's NEW transform
/// before the reproject, then drives the SAME two helpers the commit path's
/// [`anchor_follow_ops`] uses ([`reproject_node_local_quantized`] +
/// [`set_path_node`]). Sharing those helpers is what makes the renderer preview and
/// the committed move byte-equivalent (pinned by `reproject_matches_cross_core_vector`).
///
/// `at` is the target-local QUANTIZED anchor point. Pure (no time/rng/IO/GPU),
/// pointer-width-agnostic.
pub fn reproject_geometry_node(
    follower_transform: &Transform3x3,
    target_transform: &Transform3x3,
    delta: &Transform3x3,
    at: LocalPoint,
    node_index: i32,
    geometry_d: &str,
) -> Option<String> {
    let target_new = delta.mul(target_transform);
    let (qx, qy) = reproject_node_local_quantized(follower_transform, &target_new, at);
    set_path_node(geometry_d, node_index, qx, qy)
}

/// The `edit-geometry` ops a committed move produces so anchored objects follow
/// their targets. `transform_ops` are the move's `set-transform` ops (each a moved
/// id + its NEW transform); every follower anchor whose target moved reprojects
/// through that target's NEW transform.
///
/// W3-G13: ONE cumulative `edit-geometry` per follower — each follower starts from
/// its base path-string and folds in EVERY moved-target anchor (two anchors onto
/// one moved target, or anchors onto two different moved targets, land in the SAME
/// op), so a later rewrite can never stomp an earlier one when the Batch applies
/// sequentially. Ops come out in scene-object (follower) order.
///
/// A follower that is itself in the moved set is skipped (it rides its own
/// transform), and an object anchored to nothing moved authors nothing — so an
/// unanchored (Alt-created) move stays a no-op. The whole batch is returned in ONE
/// call (the shell batches it). Mirrors the shell `anchorFollowOps`.
pub fn anchor_follow_ops(scene: &ObjectScene, transform_ops: &[ObjectOp]) -> Vec<ObjectOp> {
    // The moved set: each id paired with its NEW transform, last write winning (a
    // Batch applies sequentially). A moved id absent from the scene anchors nothing
    // (the shell's `find` miss is a continue).
    let mut moved: Vec<(&str, &Transform3x3)> = Vec::new();
    for op in transform_ops {
        if let ObjectOp::SetTransform { id, transform } = op {
            if scene.get(id).is_none() {
                continue;
            }
            if let Some(entry) = moved.iter_mut().find(|(mid, _)| *mid == id.as_str()) {
                entry.1 = transform;
            } else {
                moved.push((id.as_str(), transform));
            }
        }
    }
    let mut ops = Vec::new();
    for follower in &scene.objects {
        if moved.iter().any(|(id, _)| *id == follower.id) || follower.anchors.is_empty() {
            continue;
        }
        let base = &follower.geometry.path_string;
        // `Some` once any anchor's rewrite landed; later anchors fold into it.
        let mut rewritten: Option<String> = None;
        for anchor in &follower.anchors {
            let Some((_, target_new)) = moved.iter().find(|(id, _)| *id == anchor.target) else {
                continue;
            };
            let (qx, qy) =
                reproject_node_local_quantized(&follower.transform, target_new, anchor.at);
            let current = rewritten.as_deref().unwrap_or(base);
            if let Some(d) = set_path_node(current, anchor.node_index, qx, qy) {
                rewritten = Some(d);
            }
        }
        // Author only when the cumulative result differs from the base — the
        // no-op-authors-nothing contract.
        let Some(d) = rewritten.filter(|d| d != base) else {
            continue;
        };
        ops.push(ObjectOp::EditGeometry {
            id: follower.id.clone(),
            geometry: Geometry {
                path_string: d,
                fill_rule: follower.geometry.fill_rule,
                subpaths: Vec::new(),
            },
        });
    }
    ops
}

/// Synthesize the persistent anchor(s) for a snapped drag-create, or `None` when
/// no anchor should be authored (the target is the created object, or the created
/// geometry has no node). The anchor binds `created`'s node nearest the snapped
/// world endpoint `(endpoint_x, endpoint_y)` to `target`; `at` is the snapped
/// world point mapped into the target's LOCAL quantized space, so the endpoint
/// reprojects through the target's transform on a later move. Mirrors the shell
/// `synthesizeCreateAnchors`.
pub fn synthesize_create_anchors(
    created: &Object,
    target: &Object,
    endpoint_x: f64,
    endpoint_y: f64,
) -> Option<Vec<Anchor>> {
    if target.id == created.id {
        return None;
    }
    let node_index = node_index_nearest_world(created, endpoint_x, endpoint_y)?;
    let inv = invert_affine(&affine_of(&target.transform));
    let (lx, ly) = apply_affine(&inv, endpoint_x, endpoint_y);
    let at = LocalPoint {
        x: (lx * UNITS_PER_PX).round() as i32,
        y: (ly * UNITS_PER_PX).round() as i32,
    };
    Some(vec![Anchor {
        node_index,
        target: target.id.clone(),
        at,
    }])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::model::{FillRule, Geometry, ObjectSelection, SubPath};

    /// The quantum (Q=8) used by the fixtures, for de/quantizing in assertions.
    const Q: i32 = GEOMETRY_QUANTUM_PER_PX;

    fn polyline(d: &str) -> Geometry {
        let mut g = Geometry { path_string: d.to_string(), fill_rule: FillRule::EvenOdd, subpaths: Vec::new() };
        g.ensure_parsed().unwrap();
        g
    }

    /// A two-node open line at object-local (0,0)-(lx,ly) quantized units, with the
    /// given transform translate.
    fn line(id: &str, lx: i32, ly: i32, tx: f64, ty: f64) -> Object {
        let mut o = Object::new(id, "a0", polyline(&format!("M 0 0 L {lx} {ly}")));
        o.transform = Transform3x3::translate(tx, ty);
        o
    }

    fn translate(tx: f64, ty: f64) -> Transform3x3 {
        Transform3x3::translate(tx, ty)
    }

    fn move_op(id: &str, transform: Transform3x3) -> ObjectOp {
        ObjectOp::SetTransform { id: id.to_string(), transform }
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

    /// The world position of object `obj`'s geometry node `i` under its transform
    /// (de-quantize the local node, apply the affine). Used to assert move-together.
    fn world_node(obj: &Object, i: usize) -> (f64, f64) {
        let nodes = local_nodes(&obj.geometry.path_string);
        let (nx, ny) = nodes[i];
        apply_affine(&affine_of(&obj.transform), nx / f64::from(Q), ny / f64::from(Q))
    }

    /// A line whose node 1 is anchored to `target` at the snapped world point.
    fn anchored_line(target: &Object, endpoint_x: f64, endpoint_y: f64) -> Object {
        // Start at world (40,30); snapped endpoint at (endpoint_x, endpoint_y).
        let start_lx = 40 * Q;
        let start_ly = 30 * Q;
        let end_lx = (endpoint_x as i32) * Q;
        let end_ly = (endpoint_y as i32) * Q;
        let mut o = Object::new(
            "edge-1",
            "a1",
            polyline(&format!("M {start_lx} {start_ly} L {end_lx} {end_ly}")),
        );
        o.transform = Transform3x3::IDENTITY;
        o.anchors = synthesize_create_anchors(&o, target, endpoint_x, endpoint_y).unwrap();
        o
    }

    // (a) a pure-translation SetTransform of the target shifts the follower bound
    //     node by the SAME world delta.
    #[test]
    fn pure_translation_moves_follower_node_by_the_same_world_delta() {
        let target = line("rect-a", 0, 0, 200.0, 0.0);
        let follower = anchored_line(&target, 200.0, 30.0);
        // At rest the bound node sits at the snap world point (200,30).
        let (rx, ry) = world_node(&follower, 1);
        assert!((rx - 200.0).abs() < 1e-9 && (ry - 30.0).abs() < 1e-9, "rest ({rx},{ry})");

        let scene = scene_of(vec![target.clone(), follower.clone()]);
        let ops = anchor_follow_ops(&scene, &[move_op("rect-a", translate(250.0, 20.0))]);
        assert_eq!(ops.len(), 1, "one follow op");
        let ObjectOp::EditGeometry { id, geometry } = &ops[0] else {
            panic!("expected edit-geometry");
        };
        assert_eq!(id, "edge-1");
        let mut followed = follower.clone();
        followed.geometry = geometry.clone();
        let (mx, my) = world_node(&followed, 1);
        // Target moved +50 x / +20 y from its base (200,0) => endpoint (250,50).
        assert!((mx - 250.0).abs() < 1e-9 && (my - 50.0).abs() < 1e-9, "moved ({mx},{my})");
        // The free node 0 is untouched.
        assert_eq!(world_node(&followed, 0), world_node(&follower, 0));
    }

    // FALSIFY (a): if the reproject is removed (op emitted but geometry unchanged),
    // the moved node would NOT reach the target's new world point — covered above by
    // the strict (250,50) assertion failing.

    // (b) an unanchored object authors nothing.
    #[test]
    fn unanchored_object_authors_nothing() {
        let target = line("rect-a", 0, 0, 200.0, 0.0);
        // Same drag, but no anchor synthesized (Alt-create bypassed the snap).
        let alt = line("edge-1", 160 * Q, 0, 0.0, 0.0);
        assert!(alt.anchors.is_empty());
        let scene = scene_of(vec![target, alt]);
        let ops = anchor_follow_ops(&scene, &[move_op("rect-a", translate(250.0, 20.0))]);
        assert!(ops.is_empty(), "an unanchored move-target authors no follow ops");
    }

    // (c) an object anchored only to something-not-moved authors nothing.
    #[test]
    fn anchored_only_to_unmoved_authors_nothing() {
        let target = line("rect-a", 0, 0, 200.0, 0.0);
        let follower = anchored_line(&target, 200.0, 30.0);
        let scene = scene_of(vec![target, follower]);
        // Move a DIFFERENT object the follower is not anchored to.
        let ops = anchor_follow_ops(&scene, &[move_op("rect-z", translate(10.0, 10.0))]);
        assert!(ops.is_empty(), "moving an unrelated object reprojects nothing");
    }

    // (d) a follower that is itself in the moved set is skipped.
    #[test]
    fn follower_in_the_moved_set_is_skipped() {
        let target = line("rect-a", 0, 0, 200.0, 0.0);
        let follower = anchored_line(&target, 200.0, 30.0);
        let scene = scene_of(vec![target, follower]);
        // Move BOTH the target and the follower in one batch: the follower rides its
        // own transform, so it must NOT also get a reproject edit-geometry.
        let ops = anchor_follow_ops(
            &scene,
            &[move_op("rect-a", translate(250.0, 20.0)), move_op("edge-1", translate(5.0, 5.0))],
        );
        assert!(
            ops.iter().all(|op| !matches!(op, ObjectOp::EditGeometry { id, .. } if id == "edge-1")),
            "a follower moved in the same batch is not separately reprojected"
        );
        assert!(ops.is_empty(), "the only anchored follower is itself moved => no ops");
    }

    // W3-G13 (RED before the cumulative rewrite): TWO anchors onto the SAME moved
    // target land in ONE edit-geometry with BOTH nodes rewritten — the old
    // first-anchor `find` dropped the second bound node.
    #[test]
    fn two_anchors_to_one_moved_target_author_one_cumulative_edit() {
        let target = line("rect-a", 0, 0, 200.0, 0.0);
        // Both endpoints anchored to rect-a: node 0 at world (200,0), node 1 at
        // world (200,30) (target-local (0,0) and (0,30) px).
        let mut follower = Object::new("edge-1", "a1", polyline("M 1600 0 L 1600 240"));
        follower.transform = Transform3x3::IDENTITY;
        follower.anchors = vec![
            Anchor { node_index: 0, target: "rect-a".into(), at: LocalPoint { x: 0, y: 0 } },
            Anchor { node_index: 1, target: "rect-a".into(), at: LocalPoint { x: 0, y: 30 * Q } },
        ];
        let scene = scene_of(vec![target, follower]);
        let ops = anchor_follow_ops(&scene, &[move_op("rect-a", translate(250.0, 20.0))]);
        assert_eq!(ops.len(), 1, "one cumulative follow op");
        let ObjectOp::EditGeometry { id, geometry } = &ops[0] else {
            panic!("expected edit-geometry");
        };
        assert_eq!(id, "edge-1");
        // Target new transform translate(250,20): node 0 -> world (250,20) ->
        // quantized (2000,160); node 1 -> world (250,50) -> quantized (2000,400).
        assert_eq!(geometry.path_string, "M 2000 160 L 2000 400");
    }

    // W3-G13 (RED before the cumulative rewrite): a follower anchored to two
    // DIFFERENT targets, both moved in one batch, authors ONE edit-geometry with
    // both nodes rewritten — the old per-target ops each started from the BASE
    // geometry, so the second op stomped the first node rewrite on apply.
    #[test]
    fn anchors_to_two_moved_targets_author_one_cumulative_edit() {
        let a = line("rect-a", 0, 0, 100.0, 0.0);
        let b = line("rect-b", 0, 0, 300.0, 0.0);
        // Node 0 anchored to rect-a at world (100,0); node 1 to rect-b at (300,0).
        let mut follower = Object::new("edge-1", "a1", polyline("M 800 0 L 2400 0"));
        follower.transform = Transform3x3::IDENTITY;
        follower.anchors = vec![
            Anchor { node_index: 0, target: "rect-a".into(), at: LocalPoint { x: 0, y: 0 } },
            Anchor { node_index: 1, target: "rect-b".into(), at: LocalPoint { x: 0, y: 0 } },
        ];
        let scene = scene_of(vec![a, b, follower]);
        let ops = anchor_follow_ops(
            &scene,
            &[move_op("rect-a", translate(110.0, 5.0)), move_op("rect-b", translate(310.0, 7.0))],
        );
        assert_eq!(ops.len(), 1, "one cumulative op, not one per moved target");
        let ObjectOp::EditGeometry { id, geometry } = &ops[0] else {
            panic!("expected edit-geometry");
        };
        assert_eq!(id, "edge-1");
        // Node 0 -> world (110,5) -> quantized (880,40); node 1 -> world (310,7)
        // -> quantized (2480,56). BOTH survive in the one op.
        assert_eq!(geometry.path_string, "M 880 40 L 2480 56");
    }

    // W3-G13: with two different targets but only ONE moved, only its bound node
    // rewrites; the other node stays byte-identical to the base.
    #[test]
    fn only_the_moved_target_node_is_rewritten() {
        let a = line("rect-a", 0, 0, 100.0, 0.0);
        let b = line("rect-b", 0, 0, 300.0, 0.0);
        let mut follower = Object::new("edge-1", "a1", polyline("M 800 0 L 2400 0"));
        follower.transform = Transform3x3::IDENTITY;
        follower.anchors = vec![
            Anchor { node_index: 0, target: "rect-a".into(), at: LocalPoint { x: 0, y: 0 } },
            Anchor { node_index: 1, target: "rect-b".into(), at: LocalPoint { x: 0, y: 0 } },
        ];
        let scene = scene_of(vec![a, b, follower]);
        let ops = anchor_follow_ops(&scene, &[move_op("rect-a", translate(110.0, 5.0))]);
        assert_eq!(ops.len(), 1, "one follow op for the one moved target");
        let ObjectOp::EditGeometry { id, geometry } = &ops[0] else {
            panic!("expected edit-geometry");
        };
        assert_eq!(id, "edge-1");
        // Node 0 follows rect-a; node 1 (anchored to unmoved rect-b) is untouched.
        assert_eq!(geometry.path_string, "M 880 40 L 2400 0");
    }

    // (e) synthesize round-trip: a snapped create yields an Anchor whose `at` maps
    //     back (through the target transform) to the snap world point; no-target
    //     (self) yields None.
    #[test]
    fn synthesize_round_trips_through_target_transform() {
        let target = line("rect-a", 0, 0, 200.0, 0.0); // outline origin at world (200,0)
        let line = line("edge-1", 160 * Q, 0, 0.0, 0.0);
        let anchors = synthesize_create_anchors(&line, &target, 200.0, 30.0).expect("anchor");
        assert_eq!(anchors.len(), 1);
        let a = &anchors[0];
        assert_eq!(a.target, "rect-a");
        assert_eq!(a.node_index, 1, "node 1 is the dragged endpoint");
        // `at` is world (200,30) in the target's local quantized space: local (0,30)
        // => quantized (0, 30*Q).
        assert_eq!(a.at, LocalPoint { x: 0, y: 30 * Q });
        // Round-trip: de-quantize `at`, apply the target transform => snap world pt.
        let (wx, wy) = apply_affine(
            &affine_of(&target.transform),
            f64::from(a.at.x) / f64::from(Q),
            f64::from(a.at.y) / f64::from(Q),
        );
        assert!((wx - 200.0).abs() < 1e-9 && (wy - 30.0).abs() < 1e-9, "round-trip ({wx},{wy})");
    }

    #[test]
    fn synthesize_returns_none_for_self_target() {
        let line = line("edge-1", 100 * Q, 0, 0.0, 0.0);
        assert!(synthesize_create_anchors(&line, &line, 100.0, 0.0).is_none());
    }

    // CROSS-CORE EQUIVALENCE GUARD (the drift killer). ONE concrete numeric vector,
    // hand-computed, asserted HERE and in renderer-core
    // `reproject_matches_cross_core_vector` against the SAME expected, so either
    // copy drifting fails its own test.
    //
    // Shared formula (documented in both cores):
    //   follower_base = translate(100, 0); target_new = translate(15, 27)
    //     (= delta translate(5,7) composed onto target_base translate(10,20));
    //   anchor.at = (16, 8) quantized => (2, 1) px de-quantized (Q=8);
    //   d = "M 0 0 L 64 0"; node_index = 1.
    //   world = target_new * (2, 1) = (17, 28)
    //   follower-local px = follower_base^-1 * world = (17-100, 28) = (-83, 28)
    //   quantized = round((-83, 28) * 8) = (-664, 224)
    //   => set_path_node("M 0 0 L 64 0", 1, -664, 224) = "M 0 0 L -664 224"
    #[test]
    fn reproject_matches_cross_core_vector() {
        let follower = {
            let mut o = Object::new("f", "a0", polyline("M 0 0 L 64 0"));
            o.transform = translate(100.0, 0.0);
            o
        };
        let anchor = Anchor { node_index: 1, target: "t".into(), at: LocalPoint { x: 16, y: 8 } };
        let target_new = translate(15.0, 27.0);

        let (qx, qy) = reproject_node_local_quantized(&follower.transform, &target_new, anchor.at);
        assert_eq!((qx, qy), (-664, 224), "hand-computed quantized follower-local node");

        let geometry = reproject_anchored_geometry(&follower, &anchor, &target_new).expect("rewrite");
        assert_eq!(geometry.path_string, "M 0 0 L -664 224");

        // The shared LIVE wrapper composes `delta * target_base` internally and must
        // reach the SAME pinned vector — so the renderer preview (which calls it) and
        // the commit path stay byte-equivalent.
        let target_base = translate(10.0, 20.0);
        let delta = translate(5.0, 7.0);
        let rewritten = reproject_geometry_node(
            &follower.transform,
            &target_base,
            &delta,
            anchor.at,
            anchor.node_index,
            "M 0 0 L 64 0",
        )
        .expect("addressable, changed");
        assert_eq!(rewritten, "M 0 0 L -664 224");
    }

    /// WIRE-CASING PIN (cross-core). The shell forwards each object's `anchors` to
    /// the renderer VERBATIM (`canvasHost.ts`), so the renderer's `RAnchor` serde
    /// (camelCase `nodeIndex`) only round-trips if scene-core EMITS that casing. A
    /// `rename_all` regression to snake_case would silently drop `anchors` at the
    /// renderer parse and kill live follow with no host-test failure anywhere. This
    /// pins the emitted wire shape; its renderer twin
    /// (`anchored_follower_survives_wire_serde_round_trip`) pins the parse of it.
    #[test]
    fn anchor_serializes_with_camelcase_wire_keys() {
        let mut follower = Object::new("f", "a0", polyline("M 0 0 L 64 0"));
        follower.anchors =
            vec![Anchor { node_index: 1, target: "t".into(), at: LocalPoint { x: 16, y: 8 } }];
        let json = serde_json::to_string(&follower).expect("object serializes to wire JSON");
        assert!(json.contains("\"nodeIndex\":1"), "anchor must wire as camelCase nodeIndex: {json}");
        assert!(json.contains("\"target\":\"t\""), "anchor target must wire: {json}");
        assert!(json.contains("\"at\":{\"x\":16,\"y\":8}"), "anchor `at` must wire {{x,y}}: {json}");
        assert!(!json.contains("node_index"), "snake_case node_index would be dropped: {json}");
    }

    /// A `set-path-node` no-op (the new coords already match) authors nothing, so
    /// the commit path skips a pointless edit-geometry — matching the shell.
    #[test]
    fn reproject_no_op_authors_nothing() {
        // Target at its base so the follower node stays where it already is.
        let target = line("rect-a", 0, 0, 200.0, 0.0);
        let follower = anchored_line(&target, 200.0, 30.0);
        let scene = scene_of(vec![target.clone(), follower]);
        // Move the target to its CURRENT transform (a no-move): the reproject is a
        // no-op, so no follow op is authored.
        let ops = anchor_follow_ops(&scene, &[move_op("rect-a", target.transform)]);
        assert!(ops.is_empty(), "a no-op reproject authors nothing");
    }

    /// `SubPath` import keeps the parse path exercised by `polyline` honest.
    #[test]
    fn polyline_parses_into_subpaths() {
        let g = polyline("M 0 0 L 80 0");
        assert_eq!(g.subpaths.len(), 1);
        assert_eq!(g.subpaths[0], SubPath { closed: false, nodes: g.subpaths[0].nodes.clone() });
    }
}
