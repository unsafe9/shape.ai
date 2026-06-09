//! Tier-4/#9,#13,#18 — group-hierarchy containment ops + forest queries.
//!
//! The shell's group/ungroup/pop-out/drill-in behavior reduces to a few pure
//! decisions over the object forest: who has children, where a popped-out child
//! reparents, whether a single-selected object is ungroupable, and whether a
//! double-click drills into a container or edits a leaf. These hold no time,
//! randomness, or IO and never apply an op — they only AUTHOR the op (or report
//! the decision) the shell then dispatches.
//!
//! Ported from the shell `grouping.ts` (`popOutOp` / `hasChildren` /
//! `ungroupEnabled` / `doubleClickAction`) so the containment logic lives in the
//! Rust core; the shell now only dispatches the returned op/decision (the inline
//! IME edit vs active-container set stays a shell concern). The authored
//! `Reparent` rides the existing [`super::apply`] arm (with its cycle check)
//! unchanged.
//!
//! Pure (no time/rng/IO/GPU), pointer-width-agnostic.

use super::model::{ObjectId, ObjectScene};
use super::op::ObjectOp;

/// The shell's container-vs-leaf decision for a double-click on an object. An
/// object WITH children is a container (the shell drills in, setting
/// active-container state); a childless object is a leaf (the shell enters inline
/// text edit). The ACTION dispatch stays in the shell — this is the decision only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum DoubleClickAction {
    /// The object is a container (has children) — drill in.
    DrillInContainer,
    /// The object is a leaf (no children) — edit its text.
    EditLeaf,
}

/// Whether `id` is a container (has at least one child) in the object forest.
pub fn has_children(scene: &ObjectScene, id: &str) -> bool {
    scene.objects.iter().any(|o| o.parent.as_deref() == Some(id))
}

/// Ungroup is enabled ONLY for a single selected object that is a container (has
/// children). A childless object, no selection, or the canvas is not ungroupable.
pub fn ungroup_enabled(scene: &ObjectScene, selected_id: Option<&str>) -> bool {
    selected_id.is_some_and(|id| has_children(scene, id))
}

/// The container-vs-leaf decision for a double-click on object `id`: a container
/// (has children) drills in, a leaf edits its text. The shell drives this off the
/// renderer's double-click signal `id`; the core recomputes the children query
/// from the forest so the decision lives here, not in the shell.
pub fn double_click_action(scene: &ObjectScene, id: &str) -> DoubleClickAction {
    if has_children(scene, id) {
        DoubleClickAction::DrillInContainer
    } else {
        DoubleClickAction::EditLeaf
    }
}

/// Pop a child out one level: author a [`ObjectOp::Reparent`] re-homing `id` to its
/// parent's parent (the grandparent), or to the canvas root (`parent: None`) when
/// the parent sits at the root. Returns `None` when `id` is unknown or already at
/// the root (nothing to pop out of). The child keeps its order key — pop-out is a
/// containment change, not a reorder.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::model::{FillRule, Geometry, Object, PathNode, SubPath};

    /// A minimal object with the given id, parent, and order.
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

    // root frame -> mid frame -> deep child, plus a root-level `top`.
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
        // `deep`'s parent is `mid`, whose parent is `root` -> pop out to `root`.
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
        // `mid`'s parent is `root`, whose parent is absent -> pop out to canvas root.
        assert_eq!(
            pop_out_op(&scene, "mid"),
            Some(ObjectOp::Reparent { id: "mid".into(), parent: None, order: "a0".into() })
        );
    }

    #[test]
    fn pop_out_is_none_for_root_level_or_unknown() {
        let scene = nested_scene();
        // `top` sits at the root (no parent) -> nothing to pop out of.
        assert_eq!(pop_out_op(&scene, "top"), None);
        // An unknown id -> None.
        assert_eq!(pop_out_op(&scene, "ghost"), None);
    }

    #[test]
    fn pop_out_preserves_order_key() {
        let scene = scene_of(vec![
            obj("root", None, "a0"),
            obj("mid", Some("root"), "a0"),
            obj("deep", Some("mid"), "Zz9"),
        ]);
        // The child's own order key rides through unchanged (not a reorder).
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
    fn double_click_action_container_vs_leaf() {
        let scene = scene_of(vec![
            obj("frame", None, "a0"),
            obj("child", Some("frame"), "a0"),
            obj("leaf", None, "a0"),
        ]);
        assert_eq!(double_click_action(&scene, "frame"), DoubleClickAction::DrillInContainer);
        assert_eq!(double_click_action(&scene, "leaf"), DoubleClickAction::EditLeaf);
        // An unknown id is treated as a leaf (no children) — the shell edit path.
        assert_eq!(double_click_action(&scene, "ghost"), DoubleClickAction::EditLeaf);
    }
}
