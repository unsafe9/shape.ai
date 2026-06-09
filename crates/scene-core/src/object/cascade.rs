//! Tier-2 — the parent-drag + multi-select transform CASCADE (the canonical
//! commit-time mirror of the renderer-core LIVE preview `transform_bindings`).
//!
//! A drag commits a world-space delta matrix for the dragged object(s). Children
//! are reparented under a frame with world-absolute transforms (D3), so moving a
//! parent must apply the SAME world delta to every descendant — otherwise the
//! frame slides out from under its contents. This is the commit-time op-generation
//! half of the move-together graph; the renderer-core
//! `transform_bindings::Bindings::propagation_closure` is the LIVE-preview half,
//! and the orderings MUST match (so the preview the renderer shows is the same set
//! of objects, in the same order, the commit authors).
//!
//! Ported from the shell `transformCascade.ts` (`cascadeTransformOps` /
//! `cascadeMultiTransformOps` / `composeTransform`) so the canvas logic lives in
//! the Rust core; the shell now only batches the returned ops.
//!
//! Ordering contract (the renderer-core SameDelta closure, mirrored):
//!   - single drag: the dragged root FIRST, then its transitive subtree expanded
//!     parent-before-child, BFS in scene order.
//!   - multi drag: per-root cascade — expand one root's WHOLE subtree before the
//!     next root — then a global dedup by id (a node that is both a multi member
//!     and a descendant of another root appears ONCE, at its first position).
//!
//! Pure (no time/rng/IO/GPU), pointer-width-agnostic.

use super::anchor_follow::anchor_follow_ops;
use super::model::{Object, ObjectScene, Transform3x3};
use super::op::ObjectOp;

/// 3x3 row-major pre-multiply `new = delta * base`. The delta is the cumulative
/// world-space gesture matrix; `base` is the object's existing transform. Reuses
/// the model `Transform3x3::mul` (no new matrix copy) — mirrors the shell
/// `composeTransform`.
fn compose_transform(delta: &Transform3x3, base: &Transform3x3) -> Transform3x3 {
    delta.mul(base)
}

/// Push the `set-transform` op for `object` (its base composed under `delta`).
fn push_move(ops: &mut Vec<ObjectOp>, object: &Object, delta: &Transform3x3) {
    ops.push(ObjectOp::SetTransform {
        id: object.id.clone(),
        transform: compose_transform(delta, &object.transform),
    });
}

/// Append `root`'s subtree `set-transform` ops onto `ops`, skipping any id already
/// present (global dedup across the whole batch). The root (when not already seen)
/// goes first, then its transitive descendants expanded parent-before-child, BFS
/// in scene order — matching the renderer-core SameDelta closure for one root.
fn cascade_subtree_into(
    ops: &mut Vec<ObjectOp>,
    seen: &mut Vec<String>,
    scene: &ObjectScene,
    root: &Object,
    delta: &Transform3x3,
) {
    // A root already fully expanded as a descendant of an earlier root keeps its
    // first position; skip re-walking its subtree (order is fixed by first insert).
    if seen.iter().any(|id| id == &root.id) {
        return;
    }
    seen.push(root.id.clone());
    push_move(ops, root, delta);
    let mut frontier = vec![root.id.clone()];
    while let Some(parent) = frontier.first().cloned() {
        frontier.remove(0);
        for child in &scene.objects {
            if child.parent.as_deref() != Some(parent.as_str()) {
                continue;
            }
            if seen.iter().any(|id| id == &child.id) {
                continue;
            }
            seen.push(child.id.clone());
            push_move(ops, child, delta);
            frontier.push(child.id.clone());
        }
    }
}

/// The `set-transform` ops a drag of `id` produces: the dragged object first, then
/// every descendant (transitively, via the `parent` chain), each carrying the same
/// world-space `delta` composed onto its own base. Order is parent-before-child so
/// the batch is deterministic. Returns `[]` when `id` is not in the scene.
///
/// Mirrors the shell `cascadeTransformOps`; the order matches the renderer-core
/// `propagation_closure` SameDelta result for the single-root case.
pub fn cascade_transform_ops(scene: &ObjectScene, id: &str, delta: &Transform3x3) -> Vec<ObjectOp> {
    let Some(root) = scene.get(id) else {
        return Vec::new();
    };
    let mut ops = Vec::new();
    let mut seen = Vec::new();
    cascade_subtree_into(&mut ops, &mut seen, scene, root, delta);
    ops
}

/// A Multi selection drags as one unit (the renderer anchors the gesture on a
/// single picked id, but the same world `delta` applies to EVERY member and each
/// member's subtree). Per-root cascade — each root's whole subtree before the next
/// root — then a global dedup by id so an object that is both a selected member and
/// a descendant of another member is transformed ONCE (at its first position).
/// Order is member-input order, parent-before-child within each subtree. Returns
/// `[]` when no id is live.
///
/// Mirrors the shell `cascadeMultiTransformOps`; the order matches the
/// renderer-core `propagation_closure` SameDelta result for the multi-root case.
pub fn cascade_multi_transform_ops(
    scene: &ObjectScene,
    ids: &[String],
    delta: &Transform3x3,
) -> Vec<ObjectOp> {
    let mut ops = Vec::new();
    let mut seen = Vec::new();
    for id in ids {
        let Some(root) = scene.get(id) else {
            continue;
        };
        cascade_subtree_into(&mut ops, &mut seen, scene, root, delta);
    }
    ops
}

/// Either one dragged root or a multi-select set. The combined [`move_ops`] entry
/// dispatches on this; it is the clean wire shape the wasm bridge decodes from
/// `{kind:"single",id} | {kind:"multi",ids}`.
pub enum MoveRoots {
    Single(String),
    Multi(Vec<String>),
}

/// The COMBINED commit entry: the cascade `set-transform` ops FOLLOWED BY the
/// [`anchor_follow_ops`] `edit-geometry` ops those moves trigger, as ONE
/// batch-ready Vec. Cascade ops come BEFORE follow ops (a contract — the followers
/// reproject through the moved targets' NEW transforms, which the cascade ops
/// carry). This collapses the shell commit to a single core call.
pub fn move_ops(scene: &ObjectScene, roots: &MoveRoots, delta: &Transform3x3) -> Vec<ObjectOp> {
    let mut ops = match roots {
        MoveRoots::Single(id) => cascade_transform_ops(scene, id, delta),
        MoveRoots::Multi(ids) => cascade_multi_transform_ops(scene, ids, delta),
    };
    let follow = anchor_follow_ops(scene, &ops);
    ops.extend(follow);
    ops
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::model::{
        Anchor, FillRule, Geometry, LocalPoint, ObjectScene, ObjectSelection, Transform3x3,
        GEOMETRY_QUANTUM_PER_PX,
    };

    const Q: i32 = GEOMETRY_QUANTUM_PER_PX;

    fn polyline(d: &str) -> Geometry {
        let mut g = Geometry { path_string: d.to_string(), fill_rule: FillRule::EvenOdd, subpaths: Vec::new() };
        g.ensure_parsed().unwrap();
        g
    }

    /// A two-node line object at translate `(tx, ty)`, optionally parented.
    fn obj(id: &str, parent: Option<&str>, tx: f64, ty: f64) -> Object {
        let mut o = Object::new(id, "a0", polyline("M 0 0 L 8 0"));
        o.parent = parent.map(str::to_string);
        o.transform = Transform3x3::translate(tx, ty);
        o
    }

    fn translate(dx: f64, dy: f64) -> Transform3x3 {
        Transform3x3::translate(dx, dy)
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

    /// The ordered `set-transform` ids of an op batch.
    fn moved_ids(ops: &[ObjectOp]) -> Vec<&str> {
        ops.iter()
            .filter_map(|op| match op {
                ObjectOp::SetTransform { id, .. } => Some(id.as_str()),
                _ => None,
            })
            .collect()
    }

    /// The composed translate of the `set-transform` op for `id` in a batch.
    fn moved_translate(ops: &[ObjectOp], id: &str) -> (f64, f64) {
        for op in ops {
            if let ObjectOp::SetTransform { id: oid, transform } = op {
                if oid == id {
                    return (transform.m[0][2], transform.m[1][2]);
                }
            }
        }
        panic!("no set-transform for {id}");
    }

    // (a) a parent drag applies the same world delta to the parent AND its
    //     children, in parent-before-child order.
    #[test]
    fn cascade_applies_delta_to_parent_and_children_parent_first() {
        let scene = scene_of(vec![
            obj("frame", None, 100.0, 100.0),
            obj("c1", Some("frame"), 110.0, 120.0),
            obj("c2", Some("frame"), 130.0, 140.0),
        ]);
        let ops = cascade_transform_ops(&scene, "frame", &translate(40.0, 25.0));
        // Parent FIRST, then each child in scene order.
        assert_eq!(moved_ids(&ops), vec!["frame", "c1", "c2"]);
        assert_eq!(moved_translate(&ops, "frame"), (140.0, 125.0));
        assert_eq!(moved_translate(&ops, "c1"), (150.0, 145.0));
        assert_eq!(moved_translate(&ops, "c2"), (170.0, 165.0));
    }

    // FALSIFY (a): a child-before-parent order, or a child not shifted by the
    // delta, fails the id-order / translate assertions above.

    // (b) the cascade reaches a grandchild transitively (nested frame).
    #[test]
    fn cascade_is_transitive_to_a_grandchild() {
        let scene = scene_of(vec![
            obj("frame", None, 0.0, 0.0),
            obj("inner", Some("frame"), 50.0, 50.0),
            obj("leaf", Some("inner"), 70.0, 80.0),
        ]);
        let ops = cascade_transform_ops(&scene, "frame", &translate(10.0, -5.0));
        assert_eq!(moved_ids(&ops), vec!["frame", "inner", "leaf"]);
        assert_eq!(moved_translate(&ops, "leaf"), (80.0, 75.0));
    }

    // (c) a single-object drag cascades ONLY its subtree (siblings untouched).
    #[test]
    fn single_drag_cascades_only_its_subtree() {
        let scene = scene_of(vec![
            obj("frame", None, 0.0, 0.0),
            obj("c1", Some("frame"), 10.0, 10.0),
            obj("loner", None, 200.0, 200.0),
        ]);
        let ops = cascade_transform_ops(&scene, "frame", &translate(5.0, 5.0));
        let mut ids = moved_ids(&ops);
        ids.sort_unstable();
        assert_eq!(ids, vec!["c1", "frame"]);
    }

    // (d) a missing dragged id authors nothing.
    #[test]
    fn cascade_missing_id_is_empty() {
        let scene = scene_of(vec![obj("a", None, 0.0, 0.0)]);
        assert!(cascade_transform_ops(&scene, "ghost", &translate(1.0, 1.0)).is_empty());
    }

    // (e) a Multi drag moves every member (and each member's subtree) by the same
    //     delta, in member-input order, deduped.
    #[test]
    fn multi_drag_moves_every_member_in_input_order() {
        let scene = scene_of(vec![
            obj("a", None, 100.0, 100.0),
            obj("b", None, 300.0, 50.0),
            obj("c", None, 500.0, 500.0),
        ]);
        let ids = vec!["a".to_string(), "b".to_string()];
        let ops = cascade_multi_transform_ops(&scene, &ids, &translate(40.0, 25.0));
        assert_eq!(moved_ids(&ops), vec!["a", "b"]); // c is untouched
        assert_eq!(moved_translate(&ops, "a"), (140.0, 125.0));
        assert_eq!(moved_translate(&ops, "b"), (340.0, 75.0));
    }

    // (f) a member that is ALSO a descendant of another root appears ONCE (global
    //     dedup), at its first position, transformed once.
    #[test]
    fn member_that_is_also_a_descendant_appears_once() {
        // `frame` contains `child`; the Multi also explicitly selects `child`.
        let scene = scene_of(vec![
            obj("frame", None, 0.0, 0.0),
            obj("child", Some("frame"), 50.0, 50.0),
        ]);
        let ids = vec!["frame".to_string(), "child".to_string()];
        let ops = cascade_multi_transform_ops(&scene, &ids, &translate(10.0, 10.0));
        // child appears once (frame's cascade), not twice.
        assert_eq!(moved_ids(&ops), vec!["frame", "child"]);
        assert_eq!(
            moved_ids(&ops).iter().filter(|id| **id == "child").count(),
            1,
            "child deduped"
        );
        // Applied once: 50 + 10, not 50 + 20.
        assert_eq!(moved_translate(&ops, "child"), (60.0, 60.0));
    }

    // (g) compose is pre-multiply `delta * base`: a 90° rotation delta about the
    //     origin rotates a child position (proves delta applies on the LEFT).
    #[test]
    fn compose_is_pre_multiply_delta_times_base() {
        let rot90 = Transform3x3 { m: [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]] };
        let scene = scene_of(vec![obj("c", None, 1.0, 0.0)]);
        let ops = cascade_transform_ops(&scene, "c", &rot90);
        let (x, y) = moved_translate(&ops, "c");
        assert!((x - 0.0).abs() < 1e-9, "x={x}");
        assert!((y - 1.0).abs() < 1e-9, "y={y}");
    }

    // --- move_ops: cascade BEFORE follow (the ordering contract) ---

    /// A follower line whose node 1 is anchored to `target` at world (200,30); the
    /// target is a rect outline whose origin sits at world (target_tx, 0).
    fn anchored_follower(target: &Object, endpoint_x: f64, endpoint_y: f64) -> Object {
        use crate::object::anchor_follow::synthesize_create_anchors;
        let start_lx = 40 * Q;
        let start_ly = 30 * Q;
        let end_lx = (endpoint_x as i32) * Q;
        let end_ly = (endpoint_y as i32) * Q;
        let mut o = Object::new(
            "edge",
            "a1",
            polyline(&format!("M {start_lx} {start_ly} L {end_lx} {end_ly}")),
        );
        o.transform = Transform3x3::IDENTITY;
        o.anchors = synthesize_create_anchors(&o, target, endpoint_x, endpoint_y).unwrap();
        o
    }

    // move_ops returns the cascade `set-transform` ops FIRST, then the
    // anchor-follow `edit-geometry` ops — the documented batch ordering.
    #[test]
    fn move_ops_returns_cascade_before_follow() {
        // `target` is a line at translate (200,0) (its outline origin at world
        // (200,0)); `edge` anchors its node 1 to the target at world (200,30).
        let target = obj("target", None, 200.0, 0.0);
        let follower = anchored_follower(&target, 200.0, 30.0);
        let scene = scene_of(vec![target, follower]);

        let ops = move_ops(&scene, &MoveRoots::Single("target".to_string()), &translate(50.0, 20.0));
        // The first op is the dragged target's set-transform; an edit-geometry
        // follow op comes AFTER every set-transform.
        assert!(matches!(ops[0], ObjectOp::SetTransform { ref id, .. } if id == "target"));
        let first_follow = ops
            .iter()
            .position(|op| matches!(op, ObjectOp::EditGeometry { .. }))
            .expect("a follow edit-geometry op");
        let last_move = ops
            .iter()
            .rposition(|op| matches!(op, ObjectOp::SetTransform { .. }))
            .expect("a cascade set-transform op");
        assert!(last_move < first_follow, "every cascade op precedes every follow op");
        // The follower's node 1 reprojects to the target's new endpoint.
        let ObjectOp::EditGeometry { id, .. } = &ops[first_follow] else {
            panic!("expected edit-geometry");
        };
        assert_eq!(id, "edge");
    }

    // move_ops with no anchors authors only the cascade (follow is the no-op).
    #[test]
    fn move_ops_without_anchors_is_cascade_only() {
        let scene = scene_of(vec![obj("a", None, 0.0, 0.0), obj("b", Some("a"), 5.0, 5.0)]);
        let ops = move_ops(&scene, &MoveRoots::Single("a".to_string()), &translate(3.0, 4.0));
        assert_eq!(moved_ids(&ops), vec!["a", "b"]);
        assert!(
            ops.iter().all(|op| matches!(op, ObjectOp::SetTransform { .. })),
            "no follow ops when nothing is anchored"
        );
    }

    // move_ops multi dispatch matches the multi cascade plus follow.
    #[test]
    fn move_ops_multi_dispatch_matches_multi_cascade() {
        let scene = scene_of(vec![obj("a", None, 0.0, 0.0), obj("b", None, 10.0, 0.0)]);
        let ids = vec!["a".to_string(), "b".to_string()];
        let direct = cascade_multi_transform_ops(&scene, &ids, &translate(7.0, 0.0));
        let viamove = move_ops(&scene, &MoveRoots::Multi(ids.clone()), &translate(7.0, 0.0));
        // No anchors => move_ops is exactly the multi cascade.
        assert_eq!(direct, viamove);
    }

    // PIN the ordering equals the renderer-core SameDelta closure ordering: a
    // documented expected sequence for two frames a{b}, d{e} multi-selected [a,d].
    // This is the SAME fixture as renderer-core
    // `transform_bindings::tests::multi_select_roots_expand_each_subtree_once`
    // (expected `["a","b","d","e"]`), so the two cores cannot drift on ordering.
    #[test]
    fn multi_cascade_order_matches_renderer_core_same_delta_closure() {
        let scene = scene_of(vec![
            obj("a", None, 0.0, 0.0),
            obj("b", Some("a"), 0.0, 0.0),
            obj("d", None, 0.0, 0.0),
            obj("e", Some("d"), 0.0, 0.0),
        ]);
        let ids = vec!["a".to_string(), "d".to_string()];
        let ops = cascade_multi_transform_ops(&scene, &ids, &translate(1.0, 1.0));
        assert_eq!(moved_ids(&ops), vec!["a", "b", "d", "e"]);
    }

    // A descendant pre-seen as a multi root keeps its first position (matches the
    // renderer-core `member_that_is_also_a_descendant_appears_once`): selecting
    // [a, b] where b is a's child yields ["a","b"], b not re-walked.
    #[test]
    fn descendant_root_keeps_first_position() {
        let scene = scene_of(vec![obj("a", None, 0.0, 0.0), obj("b", Some("a"), 0.0, 0.0)]);
        let ids = vec!["a".to_string(), "b".to_string()];
        let ops = cascade_multi_transform_ops(&scene, &ids, &translate(1.0, 1.0));
        assert_eq!(moved_ids(&ops), vec!["a", "b"]);
    }

    /// Imports kept honest: `Anchor` / `LocalPoint` exercise the anchored fixture.
    #[test]
    fn anchored_fixture_binds_node_one() {
        let target = obj("target", None, 200.0, 0.0);
        let follower = anchored_follower(&target, 200.0, 30.0);
        let a: &Anchor = &follower.anchors[0];
        assert_eq!(a.node_index, 1);
        assert_eq!(a.target, "target");
        assert_eq!(a.at, LocalPoint { x: 0, y: 30 * Q });
    }
}
