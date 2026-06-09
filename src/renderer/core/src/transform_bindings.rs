//! W3-G9/#5: the unified move-together bindings graph.
//!
//! Two structural relationships make objects move together LIVE during a drag:
//!
//! - **SameDelta** (D3 parent containment): a frame's children carry world-absolute
//!   transforms, so moving the parent must apply the SAME world-space delta to every
//!   descendant. The graph stores `parent -> child` edges (the inverse of the
//!   `RenderObject.parent` field).
//! - **Reproject** (D5 anchors): an object's anchored node is bound to a `target`,
//!   so when the target (or its subtree) moves, the follower's bound geometry must
//!   reproject through the target's new transform. The graph stores
//!   `target -> follower` edges (the inverse of `RenderObject.anchors[].target`).
//!
//! [`Bindings`] is built ONCE from a [`RenderObjectScene`] at load time and held on
//! the renderer, so a per-move query is O(closure) — only the affected subtree — and
//! never O(scene). This module is pure (no time/IO/GPU, pointer-width-agnostic) and
//! falsifiable in-file; the GPU write that consumes the closure lives in the
//! wasm32-gated `set_object_preview_transform`.

use std::collections::HashMap;

use crate::render_object::RenderObjectScene;

/// Which structural relationship an edge encodes (see module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropKind {
    /// Parent -> child: the child takes the SAME world-space delta as the parent.
    SameDelta,
    /// Target -> follower: the follower's anchored node reprojects through the
    /// target's new transform.
    Reproject,
}

/// One outgoing propagation edge from a node: the neighbour `id` and the kind of
/// move-together the edge encodes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PropEdge {
    pub id: String,
    pub kind: PropKind,
}

/// The precomputed propagation adjacency for a scene: `id -> outgoing edges`,
/// combining SameDelta (parent -> child) and Reproject (target -> follower) edges.
/// Built once per scene load; queried per drag via [`Bindings::propagation_closure`].
#[derive(Clone, Debug, Default)]
pub struct Bindings {
    adjacency: HashMap<String, Vec<PropEdge>>,
}

impl Bindings {
    /// Invert `parent` (SameDelta) and `anchors[].target` (Reproject) into the
    /// outgoing adjacency. Edges are appended in scene order so the closure BFS is
    /// deterministic and matches the shell's `cascadeMultiTransformOps` ordering.
    pub fn build(scene: &RenderObjectScene) -> Self {
        let mut adjacency: HashMap<String, Vec<PropEdge>> = HashMap::new();
        for object in &scene.objects {
            if let Some(parent) = &object.parent {
                adjacency.entry(parent.clone()).or_default().push(PropEdge {
                    id: object.id.clone(),
                    kind: PropKind::SameDelta,
                });
            }
            for anchor in &object.anchors {
                adjacency
                    .entry(anchor.target.clone())
                    .or_default()
                    .push(PropEdge {
                        id: object.id.clone(),
                        kind: PropKind::Reproject,
                    });
            }
        }
        Self { adjacency }
    }

    fn edges(&self, id: &str) -> &[PropEdge] {
        self.adjacency.get(id).map_or(&[], Vec::as_slice)
    }

    /// BFS the SameDelta edges from `roots` to collect every object that takes the
    /// same world delta (`same_delta_ids`), then collect the Reproject followers of
    /// every same-delta object (`reproject_followers`, each a `(follower, target)`
    /// pair so the caller knows which moved target to reproject through).
    ///
    /// `same_delta_ids` is deduped by id (a node that is both a multi member and a
    /// descendant of another root appears ONCE), with the roots in input order and
    /// each root's whole subtree expanded parent-before-child BEFORE the next root —
    /// matching `cascadeMultiTransformOps` (per-root cascade, then union/dedup).
    /// `reproject_followers` is deduped by follower id (a follower anchored to two
    /// moved targets reprojects once, against the first-seen moved target).
    pub fn propagation_closure(
        &self,
        roots: &[String],
    ) -> (Vec<String>, Vec<(String, String)>) {
        let mut same_delta: Vec<String> = Vec::new();
        // Expand one root's full subtree before the next root, so the order matches
        // the shell's per-member cascade union (not a global breadth-first sweep).
        for root in roots {
            // A root that is already a descendant of an earlier root is fully
            // expanded; skip re-walking its subtree (order is fixed by first insert).
            if same_delta.iter().any(|seen| seen == root) {
                continue;
            }
            let mut queue: std::collections::VecDeque<String> =
                std::collections::VecDeque::new();
            same_delta.push(root.clone());
            queue.push_back(root.clone());
            while let Some(current) = queue.pop_front() {
                for edge in self.edges(&current) {
                    if edge.kind != PropKind::SameDelta {
                        continue;
                    }
                    if !same_delta.iter().any(|seen| seen == &edge.id) {
                        same_delta.push(edge.id.clone());
                        queue.push_back(edge.id.clone());
                    }
                }
            }
        }

        let mut reproject: Vec<(String, String)> = Vec::new();
        for moved in &same_delta {
            for edge in self.edges(moved) {
                if edge.kind != PropKind::Reproject {
                    continue;
                }
                // A follower that is itself in the moved set rides the same delta and
                // needs no separate reproject; and each follower reprojects once.
                if same_delta.iter().any(|seen| seen == &edge.id) {
                    continue;
                }
                if reproject.iter().any(|(follower, _)| follower == &edge.id) {
                    continue;
                }
                reproject.push((edge.id.clone(), moved.clone()));
            }
        }
        (same_delta, reproject)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CameraState;
    use crate::render_object::{RAnchor, RLocalPoint, RenderObject};

    fn object(id: &str, parent: Option<&str>, anchors: Vec<RAnchor>) -> RenderObject {
        RenderObject {
            id: id.to_string(),
            parent: parent.map(str::to_string),
            order: "a0".to_string(),
            transform: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            geometry_d: "M 0 0 L 8 0 L 8 8 L 0 8 Z".to_string(),
            fill: None,
            stroke: None,
            text: None,
            anchors,
            clip: false,
        }
    }

    fn anchor(target: &str) -> RAnchor {
        RAnchor {
            node_index: 0,
            target: target.to_string(),
            at: RLocalPoint { x: 0.0, y: 0.0 },
        }
    }

    fn scene(objects: Vec<RenderObject>, multi_select: Vec<&str>) -> RenderObjectScene {
        RenderObjectScene {
            scene_id: "g9".to_string(),
            camera: CameraState {
                x: 0.0,
                y: 0.0,
                zoom: 1.0,
            },
            objects,
            selection: None,
            multi_select: multi_select.into_iter().map(str::to_string).collect(),
        }
    }

    #[test]
    fn parent_closure_collects_children_deduped_in_stable_order() {
        // A is the frame; B, C are its children.
        let s = scene(
            vec![
                object("a", None, Vec::new()),
                object("b", Some("a"), Vec::new()),
                object("c", Some("a"), Vec::new()),
            ],
            Vec::new(),
        );
        let bindings = Bindings::build(&s);
        let (same_delta, reproject) = bindings.propagation_closure(&["a".to_string()]);
        assert_eq!(same_delta, vec!["a", "b", "c"]);
        assert!(reproject.is_empty());
    }

    #[test]
    fn nested_subtree_propagates_transitively() {
        // A -> B -> C: dragging A moves the whole chain.
        let s = scene(
            vec![
                object("a", None, Vec::new()),
                object("b", Some("a"), Vec::new()),
                object("c", Some("b"), Vec::new()),
            ],
            Vec::new(),
        );
        let bindings = Bindings::build(&s);
        let (same_delta, _) = bindings.propagation_closure(&["a".to_string()]);
        assert_eq!(same_delta, vec!["a", "b", "c"]);
    }

    #[test]
    fn multi_select_roots_expand_each_subtree_once() {
        // Two frames a{b} and d{e}; multi-select [a, d] expands both, deduped.
        let s = scene(
            vec![
                object("a", None, Vec::new()),
                object("b", Some("a"), Vec::new()),
                object("d", None, Vec::new()),
                object("e", Some("d"), Vec::new()),
            ],
            vec!["a", "d"],
        );
        let bindings = Bindings::build(&s);
        let (same_delta, _) =
            bindings.propagation_closure(&["a".to_string(), "d".to_string()]);
        assert_eq!(same_delta, vec!["a", "b", "d", "e"]);
    }

    #[test]
    fn member_that_is_also_a_descendant_appears_once() {
        // b is both a child of a AND an explicit multi member; it dedups.
        let s = scene(
            vec![
                object("a", None, Vec::new()),
                object("b", Some("a"), Vec::new()),
            ],
            vec!["a", "b"],
        );
        let bindings = Bindings::build(&s);
        let (same_delta, _) =
            bindings.propagation_closure(&["a".to_string(), "b".to_string()]);
        assert_eq!(same_delta, vec!["a", "b"]);
    }

    #[test]
    fn anchored_object_reprojects_when_its_target_moves() {
        // f anchors to a; dragging a reprojects f (a moves, f is a follower).
        let s = scene(
            vec![
                object("a", None, Vec::new()),
                object("f", None, vec![anchor("a")]),
            ],
            Vec::new(),
        );
        let bindings = Bindings::build(&s);
        let (same_delta, reproject) = bindings.propagation_closure(&["a".to_string()]);
        assert_eq!(same_delta, vec!["a"]);
        assert_eq!(reproject, vec![("f".to_string(), "a".to_string())]);
    }

    #[test]
    fn anchored_to_a_subtree_descendant_still_reprojects() {
        // f anchors to b, b is a child of a; dragging a moves b => f reprojects.
        let s = scene(
            vec![
                object("a", None, Vec::new()),
                object("b", Some("a"), Vec::new()),
                object("f", None, vec![anchor("b")]),
            ],
            Vec::new(),
        );
        let bindings = Bindings::build(&s);
        let (same_delta, reproject) = bindings.propagation_closure(&["a".to_string()]);
        assert_eq!(same_delta, vec!["a", "b"]);
        assert_eq!(reproject, vec![("f".to_string(), "b".to_string())]);
    }

    #[test]
    fn unrelated_object_is_absent_from_the_closure() {
        let s = scene(
            vec![
                object("a", None, Vec::new()),
                object("z", None, Vec::new()),
            ],
            Vec::new(),
        );
        let bindings = Bindings::build(&s);
        let (same_delta, reproject) = bindings.propagation_closure(&["a".to_string()]);
        assert_eq!(same_delta, vec!["a"]);
        assert!(reproject.is_empty());
        assert!(!same_delta.iter().any(|id| id == "z"));
    }
}
