//! The unified move-together PROPAGATION graph: the single source of the
//! SameDelta + Reproject closure both the renderer LIVE preview and any
//! commit-side consumer query.
//!
//! Two structural relationships make objects move together during a drag:
//!
//! - **SameDelta** (parent containment): a frame's children carry world-absolute
//!   transforms, so moving the parent applies the SAME world-space delta to every
//!   descendant. The graph stores `parent -> child` edges (the inverse of an
//!   object's `parent` field).
//! - **Reproject** (anchors): an object's anchored node is bound to a `target`,
//!   so when the target (or its subtree) moves, the follower's bound geometry must
//!   reproject through the target's new transform. The graph stores
//!   `target -> follower` edges (the inverse of an object's `anchors[].target`).
//!
//! [`BindingGraph`] is built ONCE from a slice of [`BindingNode`] (a tiny binding
//! projection of the scene the renderer builds cheaply per feed) and held by the
//! consumer, so a per-move query is O(closure) — only the affected subtree — and
//! never O(scene). Pure (no time/rng/IO/GPU), pointer-width-agnostic.
//!
//! The closure ORDERING is pinned cross-core by `cascade.rs`'s
//! `multi_cascade_order_matches_renderer_core_same_delta_closure`, so the commit
//! path's own scene walk and this graph stay equivalent.

use std::collections::HashMap;

/// The minimal binding inputs of one scene object: its `id`, its containment
/// `parent` (SameDelta source), and the targets of its anchors (Reproject
/// sources). The renderer builds this cheaply per feed from its render scene so a
/// full scene model never has to be re-parsed to query the closure.
pub struct BindingNode {
    pub id: String,
    pub parent: Option<String>,
    pub anchor_targets: Vec<String>,
}

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
/// Built once per scene feed; queried per drag via [`BindingGraph::propagation_closure`].
#[derive(Clone, Debug, Default)]
pub struct BindingGraph {
    adjacency: HashMap<String, Vec<PropEdge>>,
}

impl BindingGraph {
    /// Invert each node's `parent` (SameDelta) and `anchor_targets` (Reproject)
    /// into the outgoing adjacency. Edges are appended in node order so the closure
    /// BFS is deterministic and matches the shell's `cascadeMultiTransformOps`
    /// ordering.
    pub fn build(nodes: &[BindingNode]) -> Self {
        let mut adjacency: HashMap<String, Vec<PropEdge>> = HashMap::new();
        for node in nodes {
            if let Some(parent) = &node.parent {
                adjacency.entry(parent.clone()).or_default().push(PropEdge {
                    id: node.id.clone(),
                    kind: PropKind::SameDelta,
                });
            }
            for target in &node.anchor_targets {
                adjacency.entry(target.clone()).or_default().push(PropEdge {
                    id: node.id.clone(),
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
    /// every same-delta object (`reproject`, each a `(follower, target)` pair so the
    /// caller knows which moved target to reproject through).
    ///
    /// `same_delta_ids` is deduped by id (a node that is both a multi member and a
    /// descendant of another root appears ONCE), with the roots in input order and
    /// each root's whole subtree expanded parent-before-child BEFORE the next root —
    /// matching `cascadeMultiTransformOps` (per-root cascade, then union/dedup).
    /// `reproject` is deduped by the `(follower, target)` PAIR (W3-G13): a follower
    /// anchored to two moved targets yields one pair PER moved target (each bound
    /// node reprojects through its own target), while duplicate anchors onto the
    /// same target still collapse to one pair.
    pub fn propagation_closure(&self, roots: &[String]) -> (Vec<String>, Vec<(String, String)>) {
        let mut same_delta: Vec<String> = Vec::new();
        // Expand one root's full subtree before the next root, so the order matches
        // the shell's per-member cascade union (not a global breadth-first sweep).
        for root in roots {
            // A root that is already a descendant of an earlier root is fully
            // expanded; skip re-walking its subtree (order is fixed by first insert).
            if same_delta.iter().any(|seen| seen == root) {
                continue;
            }
            let mut queue: std::collections::VecDeque<String> = std::collections::VecDeque::new();
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
                // needs no separate reproject; and each (follower, target) pair
                // reprojects once (W3-G13).
                if same_delta.iter().any(|seen| seen == &edge.id) {
                    continue;
                }
                if reproject
                    .iter()
                    .any(|(follower, target)| follower == &edge.id && target == moved)
                {
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

    fn node(id: &str, parent: Option<&str>, anchor_targets: Vec<&str>) -> BindingNode {
        BindingNode {
            id: id.to_string(),
            parent: parent.map(str::to_string),
            anchor_targets: anchor_targets.into_iter().map(str::to_string).collect(),
        }
    }

    #[test]
    fn parent_closure_collects_children_deduped_in_stable_order() {
        // A is the frame; B, C are its children.
        let graph = BindingGraph::build(&[
            node("a", None, Vec::new()),
            node("b", Some("a"), Vec::new()),
            node("c", Some("a"), Vec::new()),
        ]);
        let (same_delta, reproject) = graph.propagation_closure(&["a".to_string()]);
        assert_eq!(same_delta, vec!["a", "b", "c"]);
        assert!(reproject.is_empty());
    }

    #[test]
    fn nested_subtree_propagates_transitively() {
        // A -> B -> C: dragging A moves the whole chain.
        let graph = BindingGraph::build(&[
            node("a", None, Vec::new()),
            node("b", Some("a"), Vec::new()),
            node("c", Some("b"), Vec::new()),
        ]);
        let (same_delta, _) = graph.propagation_closure(&["a".to_string()]);
        assert_eq!(same_delta, vec!["a", "b", "c"]);
    }

    #[test]
    fn multi_select_roots_expand_each_subtree_once() {
        // Two frames a{b} and d{e}; multi-select [a, d] expands both, deduped.
        let graph = BindingGraph::build(&[
            node("a", None, Vec::new()),
            node("b", Some("a"), Vec::new()),
            node("d", None, Vec::new()),
            node("e", Some("d"), Vec::new()),
        ]);
        let (same_delta, _) =
            graph.propagation_closure(&["a".to_string(), "d".to_string()]);
        assert_eq!(same_delta, vec!["a", "b", "d", "e"]);
    }

    #[test]
    fn member_that_is_also_a_descendant_appears_once() {
        // b is both a child of a AND an explicit multi member; it dedups.
        let graph = BindingGraph::build(&[
            node("a", None, Vec::new()),
            node("b", Some("a"), Vec::new()),
        ]);
        let (same_delta, _) =
            graph.propagation_closure(&["a".to_string(), "b".to_string()]);
        assert_eq!(same_delta, vec!["a", "b"]);
    }

    #[test]
    fn anchored_object_reprojects_when_its_target_moves() {
        // f anchors to a; dragging a reprojects f (a moves, f is a follower).
        let graph = BindingGraph::build(&[
            node("a", None, Vec::new()),
            node("f", None, vec!["a"]),
        ]);
        let (same_delta, reproject) = graph.propagation_closure(&["a".to_string()]);
        assert_eq!(same_delta, vec!["a"]);
        assert_eq!(reproject, vec![("f".to_string(), "a".to_string())]);
    }

    #[test]
    fn anchored_to_a_subtree_descendant_still_reprojects() {
        // f anchors to b, b is a child of a; dragging a moves b => f reprojects.
        let graph = BindingGraph::build(&[
            node("a", None, Vec::new()),
            node("b", Some("a"), Vec::new()),
            node("f", None, vec!["b"]),
        ]);
        let (same_delta, reproject) = graph.propagation_closure(&["a".to_string()]);
        assert_eq!(same_delta, vec!["a", "b"]);
        assert_eq!(reproject, vec![("f".to_string(), "b".to_string())]);
    }

    // W3-G13 (RED before pair dedup): a follower anchored to TWO moved targets
    // yields one pair PER target, so BOTH anchored nodes reproject in the live
    // preview — the old follower-id dedup dropped the second target's pair.
    #[test]
    fn follower_anchored_to_two_moved_targets_pairs_with_each() {
        let graph = BindingGraph::build(&[
            node("a", None, Vec::new()),
            node("b", None, Vec::new()),
            node("f", None, vec!["a", "b"]),
        ]);
        let (same_delta, reproject) =
            graph.propagation_closure(&["a".to_string(), "b".to_string()]);
        assert_eq!(same_delta, vec!["a", "b"]);
        assert_eq!(
            reproject,
            vec![("f".to_string(), "a".to_string()), ("f".to_string(), "b".to_string())]
        );
    }

    // W3-G13: two anchors onto the SAME target are one `(follower, target)` pair —
    // the pair dedup still collapses duplicates.
    #[test]
    fn duplicate_anchors_onto_one_target_collapse_to_one_pair() {
        let graph = BindingGraph::build(&[
            node("a", None, Vec::new()),
            node("f", None, vec!["a", "a"]),
        ]);
        let (_, reproject) = graph.propagation_closure(&["a".to_string()]);
        assert_eq!(reproject, vec![("f".to_string(), "a".to_string())]);
    }

    #[test]
    fn unrelated_object_is_absent_from_the_closure() {
        let graph = BindingGraph::build(&[
            node("a", None, Vec::new()),
            node("z", None, Vec::new()),
        ]);
        let (same_delta, reproject) = graph.propagation_closure(&["a".to_string()]);
        assert_eq!(same_delta, vec!["a"]);
        assert!(reproject.is_empty());
        assert!(!same_delta.iter().any(|id| id == "z"));
    }
}
