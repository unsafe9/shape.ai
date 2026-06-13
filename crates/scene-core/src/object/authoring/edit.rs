//! Move / duplicate / detach authoring — the commit-op generators for the three
//! selection edits the shell used to assemble in TS: duplicate (clone the
//! selection at a canonical offset), detach (clear an open-class member's anchors
//! then translate it whole), and the selection-aware move-roots policy (a Multi
//! drag on a member moves the whole set; otherwise the picked single root
//! cascades its own subtree).
//!
//! Pure (no time/rng/IO/GPU), pointer-width-agnostic. Fresh ids/order keys are
//! minted from injected seams: `id_prefix` + an index, fractional order chained
//! from the scene's current top key, so a repeat call never collides.

use crate::fractional::{generate_key_between, next_order_key};
use crate::object::cascade::{move_ops, MoveRoots};
use crate::object::model::{Object, ObjectScene, ObjectSelection, Transform3x3};
use crate::object::op::ObjectOp;

/// The canonical paste/duplicate offset (logical px). Each clone lands +40/+40
/// down-right of its source so it is visibly distinct yet overlapping. Owned
/// here so the shell never re-invents the constant.
pub const DUPLICATE_OFFSET_PX: f64 = 40.0;

/// World-translate `transform` by `(dx, dy)` — a pre-multiply by a pure
/// translation, which shifts only the translation column and leaves the linear
/// part. Matches the shell's prior `shiftTransform`.
fn shifted(transform: &Transform3x3, dx: f64, dy: f64) -> Transform3x3 {
    Transform3x3::translate(dx, dy).mul(transform)
}

/// `insert-object` ops cloning each live id in `ids`, in input order, with a
/// fresh id (`{id_prefix}-{n}`) and a fresh fractional order key, offset by the
/// canonical duplicate translate. Order keys chain strictly above the scene's
/// current top (and above each prior clone), so the clones stack on top in
/// selection order and a repeat call mints non-colliding keys/ids. Unknown ids
/// are skipped; an empty result when nothing resolves.
pub fn duplicate_ops(
    scene: &ObjectScene,
    ids: &[String],
    id_prefix: &str,
    order_seed: u32,
) -> Vec<ObjectOp> {
    let mut ops = Vec::new();
    // The first clone lands strictly above the current top; each subsequent one
    // strictly above the previous clone, all via fractional indexing.
    let mut prev_order = next_order_key(scene);
    let mut n = order_seed;
    for id in ids {
        let Some(src) = scene.get(id) else { continue };
        let clone = Object {
            id: format!("{id_prefix}-{n}"),
            order: prev_order.clone(),
            transform: shifted(&src.transform, DUPLICATE_OFFSET_PX, DUPLICATE_OFFSET_PX),
            ..src.clone()
        };
        // Chain the next clone's key strictly above this one.
        prev_order = generate_key_between(Some(&prev_order), None)
            .unwrap_or_else(|_| format!("{prev_order}~"));
        n = n.wrapping_add(1);
        ops.push(ObjectOp::InsertObject { object: clone });
    }
    ops
}

/// Alt-detach commit ops for an Alt-held body drag of an ANCHORED open-class
/// object: a whole-vector `set-anchor` clearing its anchors, THEN the
/// [`move_ops`] for a single-root translate computed against the scene with the
/// dragged object's anchors already cleared. Clearing first means endpoint
/// routing sees no pins, so the member keeps the 0-rebake whole-object
/// `SetTransform` translate instead of reprojecting an anchored endpoint
/// (followers anchored TO the dragged object still follow). `[]` when `id` is
/// not in the scene.
pub fn detach_move_ops(scene: &ObjectScene, id: &str, delta: &Transform3x3) -> Vec<ObjectOp> {
    if scene.get(id).is_none() {
        return Vec::new();
    }
    let mut detached = scene.clone();
    if let Some(object) = detached.objects.iter_mut().find(|o| o.id == id) {
        object.anchors.clear();
    }
    let mut ops = vec![ObjectOp::SetAnchor { id: id.to_string(), anchors: Vec::new() }];
    ops.extend(move_ops(&detached, &MoveRoots::Single(id.to_string()), delta));
    ops
}

/// The drag-root policy: a Multi selection whose ids include the picked `id`
/// moves EVERY member (the group drags together); otherwise the picked single
/// root cascades only its own subtree. The single source of truth the shell's
/// `moveRootsFor` mirrored.
pub fn move_roots_for(selection: &ObjectSelection, id: &str) -> MoveRoots {
    match selection {
        ObjectSelection::Multi { ids } if ids.iter().any(|i| i == id) => {
            MoveRoots::Multi(ids.clone())
        }
        _ => MoveRoots::Single(id.to_string()),
    }
}

/// The commit ops for a body drag, deriving the [`MoveRoots`] from the
/// `selection` + picked `id` via [`move_roots_for`] so the shell stops branching
/// the cascade policy in TS. Cascade BEFORE follow, same as [`move_ops`].
pub fn move_ops_for_pick(
    scene: &ObjectScene,
    selection: &ObjectSelection,
    id: &str,
    delta: &Transform3x3,
) -> Vec<ObjectOp> {
    move_ops(scene, &move_roots_for(selection, id), delta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::model::{
        Anchor, FillRule, Geometry, LocalPoint, ObjectScene, ObjectSelection, Transform3x3,
    };

    fn polyline(d: &str) -> Geometry {
        let mut g = Geometry { path_string: d.to_string(), fill_rule: FillRule::EvenOdd, subpaths: Vec::new() };
        g.ensure_parsed().unwrap();
        g
    }

    fn obj(id: &str, order: &str, tx: f64, ty: f64) -> Object {
        let mut o = Object::new(id, order, polyline("M 0 0 L 8 0"));
        o.transform = Transform3x3::translate(tx, ty);
        o
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

    fn inserted(ops: &[ObjectOp]) -> Vec<&Object> {
        ops.iter()
            .filter_map(|op| match op {
                ObjectOp::InsertObject { object } => Some(object),
                _ => None,
            })
            .collect()
    }

    // --- S4 duplicate ---

    #[test]
    fn duplicate_clones_get_fresh_ids_order_and_the_canonical_offset() {
        let scene = scene_of(vec![obj("a", "a0", 100.0, 50.0), obj("b", "a1", 200.0, 60.0)]);
        let ops = duplicate_ops(&scene, &["a".to_string(), "b".to_string()], "dup", 0);
        let clones = inserted(&ops);
        assert_eq!(clones.len(), 2, "one insert per selected id");
        // Fresh ids, none colliding with a live object.
        assert_eq!(clones[0].id, "dup-0");
        assert_eq!(clones[1].id, "dup-1");
        assert!(scene.get("dup-0").is_none() && scene.get("dup-1").is_none());
        // Canonical +40/+40 offset from each source.
        assert_eq!((clones[0].transform.m[0][2], clones[0].transform.m[1][2]), (140.0, 90.0));
        assert_eq!((clones[1].transform.m[0][2], clones[1].transform.m[1][2]), (240.0, 100.0));
        // Fresh order keys sort strictly above the scene top ("a1") and ascend.
        assert!(clones[0].order.as_str() > "a1", "clone above top: {}", clones[0].order);
        assert!(clones[1].order > clones[0].order, "clones ascend");
    }

    // A repeat duplicate at the next index seed mints fresh, non-colliding ids
    // and order keys — pasting twice never authors a duplicate insert id.
    #[test]
    fn duplicate_is_non_colliding_on_repeat() {
        let mut scene = scene_of(vec![obj("a", "a0", 0.0, 0.0)]);
        let first = duplicate_ops(&scene, &["a".to_string()], "dup", 0);
        let ObjectOp::InsertObject { object: clone1 } = &first[0] else { panic!() };
        // Commit the first clone into the scene, then duplicate again.
        scene.objects.push(clone1.clone());
        let second = duplicate_ops(&scene, &["a".to_string()], "dup", 1);
        let ObjectOp::InsertObject { object: clone2 } = &second[0] else { panic!() };
        assert_ne!(clone1.id, clone2.id, "fresh id on repeat");
        assert_ne!(clone1.order, clone2.order, "fresh order on repeat");
        assert!(clone2.order.as_str() > clone1.order.as_str(), "second clone lands on top");
    }

    #[test]
    fn duplicate_skips_unknown_ids() {
        let scene = scene_of(vec![obj("a", "a0", 0.0, 0.0)]);
        let ops = duplicate_ops(&scene, &["ghost".to_string(), "a".to_string()], "dup", 0);
        let clones = inserted(&ops);
        assert_eq!(clones.len(), 1, "only the live id clones");
        assert_eq!(clones[0].id, "dup-0", "the unknown id does not consume an index gap it cannot fill");
    }

    // --- S13 detach ---

    /// An identity-transform open line (0,0)->(100,0)px anchored at node 0 to a
    /// rect, so a plain move would normally pin that endpoint (anchor-follow).
    fn anchored_open_edge() -> Object {
        let mut o = Object::new("edge", "a1", polyline("M 0 0 L 800 0"));
        o.anchors = vec![Anchor { node_index: 0, target: "rect".into(), at: LocalPoint { x: 0, y: 0 } }];
        o
    }

    // Detach emits a whole-vector set-anchor:[] FIRST, then a WHOLE-object
    // translate (set-transform) — never an anchor-follow endpoint reprojection
    // (edit-geometry). The cleared scene makes endpoint routing see no pins.
    #[test]
    fn detach_emits_clear_anchor_then_whole_object_translate() {
        let scene = scene_of(vec![obj("rect", "a0", 0.0, 0.0), anchored_open_edge()]);
        let ops = detach_move_ops(&scene, "edge", &Transform3x3::translate(40.0, 30.0));
        // First op clears the anchors.
        match &ops[0] {
            ObjectOp::SetAnchor { id, anchors } => {
                assert_eq!(id, "edge");
                assert!(anchors.is_empty(), "anchors cleared");
            }
            other => panic!("expected set-anchor first, got {}", other.kind()),
        }
        // The move is a whole-object set-transform, NOT an endpoint edit-geometry.
        let move_op = ops[1..].iter().find(|op| matches!(op, ObjectOp::SetTransform { id, .. } if id == "edge"));
        assert!(move_op.is_some(), "a whole-object translate for the detached edge: {ops:?}");
        assert!(
            !ops.iter().any(|op| matches!(op, ObjectOp::EditGeometry { id, .. } if id == "edge")),
            "no anchor-follow reprojection of the detached edge: {ops:?}"
        );
        // The translate carries the whole delta.
        let ObjectOp::SetTransform { transform, .. } = move_op.unwrap() else { unreachable!() };
        assert_eq!((transform.m[0][2], transform.m[1][2]), (40.0, 30.0));
    }

    // Without the detach, a plain single-root move of the same anchored edge pins
    // the anchored endpoint (an edit-geometry deform), proving detach changed the
    // routing rather than the scene already lacking pins.
    #[test]
    fn detach_differs_from_a_plain_move_of_the_same_anchored_edge() {
        let scene = scene_of(vec![obj("rect", "a0", 0.0, 0.0), anchored_open_edge()]);
        let plain = move_ops(&scene, &MoveRoots::Single("edge".to_string()), &Transform3x3::translate(40.0, 30.0));
        assert!(
            plain.iter().any(|op| matches!(op, ObjectOp::EditGeometry { id, .. } if id == "edge")),
            "a plain move pins the anchored endpoint (deform): {plain:?}"
        );
    }

    #[test]
    fn detach_missing_id_is_empty() {
        let scene = scene_of(vec![obj("a", "a0", 0.0, 0.0)]);
        assert!(detach_move_ops(&scene, "ghost", &Transform3x3::translate(1.0, 1.0)).is_empty());
    }

    // --- moveRoots derivation ---

    // A Multi selection whose ids include the picked id moves EVERY member; the
    // same fixture proves move_ops_for_pick derives the same roots the shell's
    // moveRootsFor did.
    #[test]
    fn multi_pick_on_a_member_moves_the_whole_set() {
        let sel = ObjectSelection::Multi { ids: vec!["a".into(), "b".into()] };
        match move_roots_for(&sel, "a") {
            MoveRoots::Multi(ids) => assert_eq!(ids, vec!["a".to_string(), "b".to_string()]),
            MoveRoots::Single(_) => panic!("multi-on-member must be Multi"),
        }
        let scene = scene_of(vec![obj("a", "a0", 0.0, 0.0), obj("b", "a1", 50.0, 0.0)]);
        let ops = move_ops_for_pick(&scene, &sel, "a", &Transform3x3::translate(10.0, 0.0));
        let ids: Vec<&str> = ops
            .iter()
            .filter_map(|op| match op {
                ObjectOp::SetTransform { id, .. } => Some(id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(ids, vec!["a", "b"], "the whole set moves");
    }

    // A pick OUTSIDE the multi (or a single selection) cascades only the picked
    // single root's subtree — Single roots, matching moveRootsFor's else-branch.
    #[test]
    fn pick_outside_multi_is_single_root() {
        let sel = ObjectSelection::Multi { ids: vec!["a".into(), "b".into()] };
        assert!(matches!(move_roots_for(&sel, "c"), MoveRoots::Single(id) if id == "c"));
        assert!(matches!(
            move_roots_for(&ObjectSelection::Object { id: "a".into() }, "a"),
            MoveRoots::Single(id) if id == "a"
        ));
        assert!(matches!(
            move_roots_for(&ObjectSelection::Canvas, "a"),
            MoveRoots::Single(id) if id == "a"
        ));
    }

    // move_ops_for_pick with a single root matches the bare move_ops on the same
    // Single roots — the derivation adds policy, not a different op stream.
    #[test]
    fn move_ops_for_pick_single_matches_bare_move_ops() {
        let scene = scene_of(vec![obj("a", "a0", 0.0, 0.0), obj("b", "a1", 5.0, 5.0)]);
        let sel = ObjectSelection::Object { id: "a".into() };
        let viapick = move_ops_for_pick(&scene, &sel, "a", &Transform3x3::translate(3.0, 4.0));
        let bare = move_ops(&scene, &MoveRoots::Single("a".to_string()), &Transform3x3::translate(3.0, 4.0));
        assert_eq!(viapick, bare);
    }
}
