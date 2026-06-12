//! Group-hierarchy containment ops + forest queries. Pure (no time/rng/IO) and
//! pointer-width-agnostic; these AUTHOR the op (or report the decision) the
//! shell then dispatches, never applying it themselves.

use crate::object::model::{ObjectId, ObjectScene};
use crate::object::op::ObjectOp;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::model::{FillRule, Geometry, Object, PathNode, SubPath};

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
}
