//! The move-together propagation graph: the single source of the SameDelta +
//! Reproject closure the renderer live preview and any commit-side consumer query.
//!
//! Two relationships make objects move together during a drag:
//! - SameDelta (parent containment): children carry world-absolute transforms,
//!   so moving the parent applies the SAME world delta to every descendant. The
//!   graph stores `parent -> child` edges.
//! - Reproject (anchors): an anchored node is bound to a `target`, so when the
//!   target (or its subtree) moves, the follower's geometry reprojects through
//!   the target's new transform. The graph stores `target -> follower` edges.
//!
//! [`BindingGraph`] is built once per feed and held by the consumer, so a per-move
//! query is O(closure), never O(scene). Pure (no time/rng/IO/GPU).

use std::collections::HashMap;

/// The minimal binding inputs of one scene object: `id`, containment `parent`
/// (SameDelta source), anchor targets (Reproject sources).
pub struct BindingNode {
    pub id: String,
    pub parent: Option<String>,
    pub anchor_targets: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropKind {
    SameDelta,
    Reproject,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PropEdge {
    pub id: String,
    pub kind: PropKind,
}

/// `id -> outgoing edges`, combining SameDelta and Reproject. Built once per feed;
/// queried via [`BindingGraph::propagation_closure`].
#[derive(Clone, Debug, Default)]
pub struct BindingGraph {
    adjacency: HashMap<String, Vec<PropEdge>>,
}

impl BindingGraph {
    /// Invert each node's `parent` (SameDelta) and `anchor_targets` (Reproject)
    /// into the outgoing adjacency. Edges are appended in node order so the closure
    /// BFS is deterministic.
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

    /// BFS the SameDelta edges from `roots` (the same-world-delta set), then the
    /// Reproject followers of every same-delta object (each a `(follower, target)`
    /// pair so the caller knows which moved target to reproject through).
    ///
    /// `same_delta` is deduped by id, roots in input order, each root's whole
    /// subtree expanded parent-before-child before the next root. `reproject` is
    /// deduped by the `(follower, target)` pair: a follower anchored to two moved
    /// targets yields one pair per target; duplicate anchors onto one target
    /// collapse to one pair.
    pub fn propagation_closure(&self, roots: &[String]) -> (Vec<String>, Vec<(String, String)>) {
        let mut same_delta: Vec<String> = Vec::new();
        // One root's full subtree before the next (not a global breadth-first sweep).
        for root in roots {
            // A root already expanded as a descendant is skipped; order is fixed by
            // first insert.
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
                // A follower already in the moved set rides the same delta and
                // needs no separate reproject; each (follower, target) pair once.
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

    // A follower anchored to TWO moved targets yields one pair PER target, so
    // BOTH anchored nodes reproject in the live preview.
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

    // Two anchors onto the SAME target collapse to one `(follower, target)` pair.
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
