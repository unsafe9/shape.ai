//! Group-hierarchy containment ops + forest queries. Pure (no time/rng/IO) and
//! pointer-width-agnostic; these AUTHOR the op (or report the decision) the
//! shell then dispatches, never applying it themselves.

use crate::fractional::{generate_n_keys_between, next_order_key};
use crate::object::model::{
    Align, CrossAlign, FillRule, Geometry, Lanes, Layout, LayoutAxis, MainAlign, Object, ObjectId,
    ObjectScene, ObjectSelection, Transform3x3,
};
use crate::object::op::ObjectOp;
use crate::object::primitives::rect_path;
use crate::object::region::object_world_aabb;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum DoubleClickAction {
    DrillInContainer,
    EditLeaf,
}

pub fn has_children(scene: &ObjectScene, id: &str) -> bool {
    scene.objects.iter().any(|o| o.parent.as_deref() == Some(id))
}

/// Enabled only for a single selected object that is a container.
pub fn ungroup_enabled(scene: &ObjectScene, selected_id: Option<&str>) -> bool {
    selected_id.is_some_and(|id| has_children(scene, id))
}

pub fn double_click_action(scene: &ObjectScene, id: &str) -> DoubleClickAction {
    if has_children(scene, id) {
        DoubleClickAction::DrillInContainer
    } else {
        DoubleClickAction::EditLeaf
    }
}

/// Whether `selection` still belongs to the active drill-in `container` scope: true
/// for the container itself or one of its DIRECT children, false otherwise (canvas,
/// multi, an unknown id, or any object parented elsewhere). The shell mirrors this in
/// lockstep — it retracts the active-container token exactly when this is false. The
/// positive form sidesteps a negation-naming trap.
pub fn object_selection_in_scope(
    scene: &ObjectScene,
    selection: &ObjectSelection,
    container: &str,
) -> bool {
    match selection {
        ObjectSelection::Object { id } => {
            id == container
                || scene.get(id).and_then(|o| o.parent.as_deref()) == Some(container)
        }
        _ => false,
    }
}

/// Re-home `id` to its grandparent, or to the canvas root when the parent sits
/// at the root. `None` when `id` is unknown or already at the root. The child
/// keeps its order key — pop-out is a containment change, not a reorder.
pub fn pop_out_op(scene: &ObjectScene, id: &str) -> Option<ObjectOp> {
    let child = scene.get(id)?;
    let parent_id = child.parent.as_deref()?;
    let grandparent: Option<ObjectId> = scene.get(parent_id).and_then(|p| p.parent.clone());
    Some(ObjectOp::Reparent {
        id: id.to_string(),
        parent: grandparent,
        order: child.order.clone(),
    })
}

/// Author the ops grouping `ids` under a freshly minted frame `frame_id`: an
/// `insert-object` for the frame (a non-clipping rect sized + placed to the children's
/// union WORLD-AABB), then one `reparent` per child re-homing it into the frame.
///
/// The frame's geometry is a `rect_path` in object-local px; the world placement
/// rides a pure-translation transform to the AABB min corner, so a later move is
/// matrix-only. Children get fresh fractional order keys (in their current
/// relative paint order) so they stack inside the frame as they did outside.
///
/// `None` when fewer than two known children resolve, or no member yields a
/// derivable region (no AABB to frame).
pub fn group_ops(scene: &ObjectScene, ids: &[ObjectId], frame_id: &str) -> Option<Vec<ObjectOp>> {
    // Members that exist, in canonical paint order (fractional `order`, ties by
    // id) so the minted child keys preserve their relative stacking.
    let mut members: Vec<&Object> =
        ids.iter().filter_map(|id| scene.get(id)).collect();
    if members.len() < 2 {
        return None;
    }
    members.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.id.cmp(&b.id)));

    // Union world-AABB over every member with a derivable region. Uses the same
    // control-point-inclusive world-AABB notion as `object_world_aabb` (the
    // surviving selection/overlay semantic) so the frame sizes/places to the
    // transformed-node extents, not the anchor-only region bounds.
    let mut union: Option<(f64, f64, f64, f64)> = None;
    for m in &members {
        if let Some(b) = object_world_aabb(m) {
            let (x0, y0, x1, y1) = (b.min_x, b.min_y, b.max_x, b.max_y);
            union = Some(match union {
                None => (x0, y0, x1, y1),
                Some((ux0, uy0, ux1, uy1)) => {
                    (ux0.min(x0), uy0.min(y0), ux1.max(x1), uy1.max(y1))
                }
            });
        }
    }
    let (min_x, min_y, max_x, max_y) = union?;

    // The frame: a non-clipping rect of the union span, translated to its min corner.
    let mut geometry = Geometry {
        path_string: rect_path(max_x - min_x, max_y - min_y),
        fill_rule: FillRule::NonZero,
        subpaths: Vec::new(),
    };
    let _ = geometry.parse();
    let mut frame = Object::new(frame_id.to_string(), next_order_key(scene), geometry);
    frame.transform = Transform3x3::translate(min_x, min_y);
    frame.clip = Some(false);
    // Default a new frame to Flow, axis inferred from the children's spatial
    // spread: a wider horizontal spread reads as a row, a taller one as a column.
    // `spacing: 0` keeps the frame geometry/transform identical to the union AABB.
    let axis = if (max_x - min_x) >= (max_y - min_y) {
        LayoutAxis::Horizontal
    } else {
        LayoutAxis::Vertical
    };
    frame.layout = Some(Layout {
        axis,
        lanes: Lanes::Count { value: 1 },
        spacing: 0,
        align: Align { main: MainAlign::Start, cross: CrossAlign::Start },
    });

    let mut ops = vec![ObjectOp::InsertObject { object: frame }];

    // Fresh order keys under the new parent, one per member in paint order.
    let child_keys = generate_n_keys_between(None, None, members.len()).ok()?;
    for (m, order) in members.iter().zip(child_keys) {
        ops.push(ObjectOp::Reparent {
            id: m.id.clone(),
            parent: Some(frame_id.to_string()),
            order,
        });
    }
    Some(ops)
}

/// Author the ops dissolving the container `frame_id`: re-home every child to the
/// frame's parent (its grandparent, or the canvas root), each keeping its own
/// order key, then `delete` the now-empty frame. The multi-child generalization
/// of [`pop_out_op`] — children are reparented up one level, frame removed,
/// leaving the forest orphan-free.
///
/// `None` when `frame_id` is unknown.
pub fn ungroup_ops(scene: &ObjectScene, frame_id: &str) -> Option<Vec<ObjectOp>> {
    let frame = scene.get(frame_id)?;
    let grandparent = frame.parent.clone();

    let mut children: Vec<&Object> =
        scene.objects.iter().filter(|o| o.parent.as_deref() == Some(frame_id)).collect();
    children.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.id.cmp(&b.id)));

    let mut ops: Vec<ObjectOp> = children
        .iter()
        .map(|c| ObjectOp::Reparent {
            id: c.id.clone(),
            parent: grandparent.clone(),
            order: c.order.clone(),
        })
        .collect();
    ops.push(ObjectOp::Delete { id: frame_id.to_string() });
    Some(ops)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::model::{
        FillRule, Geometry, Object, PathNode, SubPath, GEOMETRY_QUANTUM_PER_PX,
    };

    fn obj(id: &str, parent: Option<&str>, order: &str) -> Object {
        let geometry = Geometry::from_subpaths(
            vec![SubPath {
                closed: false,
                nodes: vec![PathNode::corner(0, 0), PathNode::corner(8, 0)],
            }],
            FillRule::EvenOdd,
        );
        let mut o = Object::new(id, order, geometry);
        o.parent = parent.map(|p| p.to_string());
        o
    }

    fn scene_of(objects: Vec<Object>) -> ObjectScene {
        ObjectScene { objects, ..Default::default() }
    }

    fn nested_scene() -> ObjectScene {
        scene_of(vec![
            obj("root", None, "a0"),
            obj("mid", Some("root"), "a0"),
            obj("deep", Some("mid"), "a0"),
            obj("top", None, "a0"),
        ])
    }

    #[test]
    fn pop_out_reparents_to_grandparent() {
        let scene = nested_scene();
        assert_eq!(
            pop_out_op(&scene, "deep"),
            Some(ObjectOp::Reparent {
                id: "deep".into(),
                parent: Some("root".into()),
                order: "a0".into()
            })
        );
    }

    #[test]
    fn pop_out_reparents_to_root_when_parent_at_root() {
        let scene = nested_scene();
        assert_eq!(
            pop_out_op(&scene, "mid"),
            Some(ObjectOp::Reparent { id: "mid".into(), parent: None, order: "a0".into() })
        );
    }

    #[test]
    fn pop_out_is_none_for_root_level_or_unknown() {
        let scene = nested_scene();
        assert_eq!(pop_out_op(&scene, "top"), None);
        assert_eq!(pop_out_op(&scene, "ghost"), None);
    }

    #[test]
    fn pop_out_preserves_order_key() {
        let scene = scene_of(vec![
            obj("root", None, "a0"),
            obj("mid", Some("root"), "a0"),
            obj("deep", Some("mid"), "Zz9"),
        ]);
        let op = pop_out_op(&scene, "deep").expect("pop out");
        assert_eq!(op, ObjectOp::Reparent { id: "deep".into(), parent: Some("root".into()), order: "Zz9".into() });
    }

    #[test]
    fn has_children_truth_table() {
        let scene = scene_of(vec![
            obj("frame", None, "a0"),
            obj("child", Some("frame"), "a0"),
            obj("leaf", None, "a0"),
        ]);
        assert!(has_children(&scene, "frame"), "a container has children");
        assert!(!has_children(&scene, "leaf"), "a childless leaf does not");
        assert!(!has_children(&scene, "ghost"), "an unknown id has no children");
    }

    #[test]
    fn ungroup_enabled_truth_table() {
        let scene = scene_of(vec![
            obj("frame", None, "a0"),
            obj("child", Some("frame"), "a0"),
            obj("leaf", None, "a0"),
        ]);
        assert!(ungroup_enabled(&scene, Some("frame")), "a container is ungroupable");
        assert!(!ungroup_enabled(&scene, Some("leaf")), "a childless leaf is not");
        assert!(!ungroup_enabled(&scene, None), "no selection is not ungroupable");
    }

    #[test]
    fn object_selection_in_scope_membership_table() {
        let scene = scene_of(vec![
            obj("frame", None, "a0"),
            obj("child", Some("frame"), "a0"),
            obj("leaf", None, "a0"),
        ]);
        let object = |id: &str| ObjectSelection::Object { id: id.into() };
        assert!(
            object_selection_in_scope(&scene, &object("frame"), "frame"),
            "the container itself stays in scope"
        );
        assert!(
            object_selection_in_scope(&scene, &object("child"), "frame"),
            "a direct child stays in scope"
        );
        assert!(
            !object_selection_in_scope(&scene, &object("leaf"), "frame"),
            "an outside object exits scope"
        );
        assert!(
            !object_selection_in_scope(&scene, &object("ghost"), "frame"),
            "an unknown id exits scope"
        );
        assert!(
            !object_selection_in_scope(&scene, &ObjectSelection::Canvas, "frame"),
            "canvas exits scope"
        );
        assert!(
            !object_selection_in_scope(
                &scene,
                &ObjectSelection::Multi { ids: vec!["child".into()] },
                "frame"
            ),
            "multi exits scope even when a member is in scope"
        );
    }

    #[test]
    fn double_click_action_container_vs_leaf() {
        let scene = scene_of(vec![
            obj("frame", None, "a0"),
            obj("child", Some("frame"), "a0"),
            obj("leaf", None, "a0"),
        ]);
        assert_eq!(double_click_action(&scene, "frame"), DoubleClickAction::DrillInContainer);
        assert_eq!(double_click_action(&scene, "leaf"), DoubleClickAction::EditLeaf);
        assert_eq!(double_click_action(&scene, "ghost"), DoubleClickAction::EditLeaf);
    }

    // --- S2 group_ops / S3 ungroup_ops ---

    use crate::fractional::cmp_keys;
    use crate::object::apply::apply_object_op;

    /// A closed `w`×`h` (px) rect at world `(tx, ty)` via a pure-translation
    /// transform — geometry authored from local (0,0), so the world AABB is the
    /// translate plus the local span.
    fn rect(id: &str, order: &str, tx: f64, ty: f64, w: i32, h: i32) -> Object {
        let q = GEOMETRY_QUANTUM_PER_PX;
        let geometry = Geometry::from_subpaths(
            vec![SubPath {
                closed: true,
                nodes: vec![
                    PathNode::corner(0, 0),
                    PathNode::corner(w * q, 0),
                    PathNode::corner(w * q, h * q),
                    PathNode::corner(0, h * q),
                ],
            }],
            FillRule::NonZero,
        );
        let mut o = Object::new(id, order, geometry);
        o.transform = Transform3x3::translate(tx, ty);
        o
    }

    fn frame_op(ops: &[ObjectOp]) -> &Object {
        match &ops[0] {
            ObjectOp::InsertObject { object } => object,
            other => panic!("expected insert-object first, got {}", other.kind()),
        }
    }

    #[test]
    fn group_ops_frame_is_sized_and_placed_to_the_union_world_aabb() {
        // a: 40x20 at (10,10) -> world [10,10]..[50,30]
        // b: 30x30 at (60,50) -> world [60,50]..[90,80]
        // union -> min (10,10), max (90,80): 80 wide, 70 tall.
        let scene = scene_of(vec![rect("a", "a0", 10.0, 10.0, 40, 20), rect("b", "a1", 60.0, 50.0, 30, 30)]);
        let ops = group_ops(&scene, &["a".into(), "b".into()], "frame").expect("group authors ops");
        let frame = frame_op(&ops);
        // Golden d: rect_path(80, 70) -> q(80)=640, q(70)=560.
        assert_eq!(frame.geometry.path_string, "M 0 0 L 640 0 L 640 560 L 0 560 Z");
        // Placed at the union min corner via a pure-translation transform.
        assert_eq!(frame.transform.m[0][2], 10.0);
        assert_eq!(frame.transform.m[1][2], 10.0);
        assert_eq!(frame.clip, Some(false));
        // A wrong AABB would shift the corner or resize the rect; pin both.
    }

    #[test]
    fn group_ops_defaults_to_flow_with_inferred_axis() {
        // Wider horizontal spread (x span 80 > y span 30) => Horizontal.
        let wide = scene_of(vec![
            rect("a", "a0", 0.0, 0.0, 10, 10),
            rect("b", "a1", 80.0, 20.0, 10, 10),
        ]);
        let wide_ops = group_ops(&wide, &["a".into(), "b".into()], "frame").expect("ops");
        let wide_frame = frame_op(&wide_ops);
        assert_eq!(
            wide_frame.layout,
            Some(Layout {
                axis: LayoutAxis::Horizontal,
                lanes: Lanes::Count { value: 1 },
                spacing: 0,
                align: Align { main: MainAlign::Start, cross: CrossAlign::Start },
            }),
            "wide spread defaults to a horizontal flow",
        );

        // Taller spread (y span 80 > x span 30) => Vertical.
        let tall = scene_of(vec![
            rect("a", "a0", 0.0, 0.0, 10, 10),
            rect("b", "a1", 20.0, 80.0, 10, 10),
        ]);
        let tall_ops = group_ops(&tall, &["a".into(), "b".into()], "frame").expect("ops");
        let tall_frame = frame_op(&tall_ops);
        assert_eq!(
            tall_frame.layout.map(|l| l.axis),
            Some(LayoutAxis::Vertical),
            "tall spread defaults to a vertical flow",
        );
    }

    #[test]
    fn group_ops_reparents_every_child_into_the_frame_with_validating_ascending_keys() {
        let scene = scene_of(vec![rect("a", "a0", 0.0, 0.0, 10, 10), rect("b", "a1", 20.0, 0.0, 10, 10)]);
        let ops = group_ops(&scene, &["a".into(), "b".into()], "frame").expect("ops");
        // One insert (frame) + one reparent per child.
        assert_eq!(ops.len(), 3);
        let mut keys: Vec<String> = Vec::new();
        for op in &ops[1..] {
            match op {
                ObjectOp::Reparent { id, parent, order } => {
                    assert_eq!(parent.as_deref(), Some("frame"), "child {id} re-homed into frame");
                    keys.push(order.clone());
                }
                other => panic!("expected reparent, got {}", other.kind()),
            }
        }
        // Keys in member paint order (a before b) must strictly ascend, and they
        // must apply cleanly through the real core (validation lives in apply).
        assert_eq!(cmp_keys(&keys[0], &keys[1]), core::cmp::Ordering::Less, "{keys:?} not ascending");
        let mut applied = scene.clone();
        applied.ensure_parsed().unwrap();
        for op in ops {
            apply_object_op(&mut applied, op).expect("group op applies");
        }
        // After apply: both children parented to the frame, frame present, no orphans.
        assert!(applied.get("frame").is_some());
        assert_eq!(applied.get("a").unwrap().parent.as_deref(), Some("frame"));
        assert_eq!(applied.get("b").unwrap().parent.as_deref(), Some("frame"));
    }

    #[test]
    fn group_ops_is_none_below_two_members() {
        let scene = scene_of(vec![rect("a", "a0", 0.0, 0.0, 10, 10)]);
        assert!(group_ops(&scene, &["a".into()], "frame").is_none(), "single member: nothing to group");
        assert!(group_ops(&scene, &["ghost".into(), "phantom".into()], "frame").is_none(), "unknown members");
    }

    #[test]
    fn ungroup_ops_reparents_all_children_to_grandparent_and_deletes_the_frame() {
        // root > frame > {a, b}; ungrouping frame re-homes a,b to root and deletes frame.
        let mut a = rect("a", "a0", 0.0, 0.0, 10, 10);
        a.parent = Some("frame".into());
        let mut b = rect("b", "a1", 20.0, 0.0, 10, 10);
        b.parent = Some("frame".into());
        let mut frame = rect("frame", "a0", 0.0, 0.0, 40, 10);
        frame.parent = Some("root".into());
        let scene = scene_of(vec![rect("root", "a0", 0.0, 0.0, 80, 80), frame, a, b]);

        let ops = ungroup_ops(&scene, "frame").expect("ungroup authors ops");
        // Two reparents (children, in paint order) then the frame delete.
        assert_eq!(ops.len(), 3);
        assert_eq!(
            ops[0],
            ObjectOp::Reparent { id: "a".into(), parent: Some("root".into()), order: "a0".into() }
        );
        assert_eq!(
            ops[1],
            ObjectOp::Reparent { id: "b".into(), parent: Some("root".into()), order: "a1".into() }
        );
        assert_eq!(ops[2], ObjectOp::Delete { id: "frame".into() });

        // Drive the real core: after apply the forest is orphan-free.
        let mut applied = scene.clone();
        applied.ensure_parsed().unwrap();
        for op in ops {
            apply_object_op(&mut applied, op).expect("ungroup op applies");
        }
        assert!(applied.get("frame").is_none(), "empty frame deleted");
        assert_eq!(applied.get("a").unwrap().parent.as_deref(), Some("root"));
        assert_eq!(applied.get("b").unwrap().parent.as_deref(), Some("root"));
        // No object references the dissolved frame as a parent.
        assert!(
            applied.objects.iter().all(|o| o.parent.as_deref() != Some("frame")),
            "no orphan still points at the frame"
        );
    }

    #[test]
    fn ungroup_ops_reparents_root_frame_children_to_the_canvas_root() {
        // frame at the canvas root: its children pop out to None (the root).
        let mut a = rect("a", "a0", 0.0, 0.0, 10, 10);
        a.parent = Some("frame".into());
        let mut frame = rect("frame", "a0", 0.0, 0.0, 20, 10);
        frame.parent = None;
        let scene = scene_of(vec![frame, a]);
        let ops = ungroup_ops(&scene, "frame").expect("ops");
        assert_eq!(
            ops[0],
            ObjectOp::Reparent { id: "a".into(), parent: None, order: "a0".into() }
        );
    }

    #[test]
    fn ungroup_ops_is_none_for_unknown_frame() {
        let scene = scene_of(vec![rect("a", "a0", 0.0, 0.0, 10, 10)]);
        assert!(ungroup_ops(&scene, "ghost").is_none());
    }
}
