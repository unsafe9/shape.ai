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

use crate::hit_test_object::{invert_3x3, mat3_mul, UNITS_PER_PX};
use crate::render_object::{RAnchor, RenderObjectScene};

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

/// W3-G9/#4: the LIVE follower-LOCAL pixel position of an anchored node when its
/// target moves by `delta`. Mirrors the shell `reprojectAnchoredGeometry`
/// (anchorCreate.ts): the anchor `at` is a point in the TARGET's local QUANTIZED
/// space, so de-quantize it (`/ UNITS_PER_PX`), carry it to world through the
/// target's PREVIEWED transform (`delta * target_base`), then back into the
/// follower's OWN local pixel space (`follower_base^-1`). Returns the follower-
/// local `(x, y)` in pixels, or `None` when `follower_base` is singular.
///
/// Pure affine compose: `follower_base^-1 * (delta * target_base) * (at / Q)`.
/// A pure target translation therefore moves the follower node by the SAME world
/// delta (asserted in tests). The geometry write that consumes this is in the
/// wasm32-gated preview path; this math half is host-testable.
pub fn reproject_node_local_px(
    follower_base: &[[f64; 3]; 3],
    target_base: &[[f64; 3]; 3],
    delta: &[[f64; 3]; 3],
    at: &RAnchor,
) -> Option<(f64, f64)> {
    let inv = invert_3x3(follower_base)?;
    // `delta * target_base` is the target's PREVIEWED world transform (the same
    // composition the SameDelta instance write applies to the moved target).
    let target_world = mat3_mul(delta, target_base);
    // De-quantize the target-local anchor point to pixels before the affine carry.
    let lx = at.at.x / UNITS_PER_PX;
    let ly = at.at.y / UNITS_PER_PX;
    let wx = target_world[0][0] * lx + target_world[0][1] * ly + target_world[0][2];
    let wy = target_world[1][0] * lx + target_world[1][1] * ly + target_world[1][2];
    let fx = inv[0][0] * wx + inv[0][1] * wy + inv[0][2];
    let fy = inv[1][0] * wx + inv[1][1] * wy + inv[1][2];
    if fx.is_finite() && fy.is_finite() {
        Some((fx, fy))
    } else {
        None
    }
}

/// W3-G9/#4: rewrite the `node_index`-th coordinate PAIR of a path-string `d` to
/// the QUANTIZED `(x, y)` (object-local units, rounded from pixels), preserving
/// every command token and every other coordinate. Mirrors the shell `setPathNode`
/// (anchorCreate.ts): coordinate pairs are counted in token order across the whole
/// string (M/L/C all contribute pairs), and only the target pair is replaced.
/// Returns `None` when the string has no such pair (the node is unaddressable) or
/// when the rewrite is a no-op (the new coords already match), so the caller can
/// skip a pointless re-expand + GPU write.
pub fn rewrite_geometry_node(d: &str, node_index: usize, x_px: f64, y_px: f64) -> Option<String> {
    let qx = (x_px * UNITS_PER_PX).round() as i64;
    let qy = (y_px * UNITS_PER_PX).round() as i64;
    let mut pair = 0usize;
    let mut numbers_seen = 0usize;
    let mut hit = false;
    let mut changed = false;
    let mut out = String::with_capacity(d.len());
    let mut last = 0usize;
    for m in NumberSpans::new(d) {
        out.push_str(&d[last..m.start]);
        let is_x = numbers_seen % 2 == 0;
        let at_target = pair == node_index;
        if !is_x {
            pair += 1;
        }
        numbers_seen += 1;
        if at_target {
            hit = true;
            let replacement = if is_x { qx } else { qy };
            let original = &d[m.start..m.end];
            let replacement_str = replacement.to_string();
            if original != replacement_str {
                changed = true;
            }
            out.push_str(&replacement_str);
        } else {
            out.push_str(&d[m.start..m.end]);
        }
        last = m.end;
    }
    out.push_str(&d[last..]);
    if hit && changed {
        Some(out)
    } else {
        None
    }
}

/// A half-open byte span `[start, end)` of one signed-decimal number token in a
/// path string. Used by [`rewrite_geometry_node`] to splice coordinates without a
/// regex (the pure core carries no regex dependency).
struct NumberSpan {
    start: usize,
    end: usize,
}

/// Iterator over the signed-decimal number spans of a path string, matching the
/// shell `setPathNode` regex `-?\d+(?:\.\d+)?` (an optional leading `-`, digits,
/// an optional `.`-fraction). Non-number characters (command letters, spaces,
/// commas) are skipped between spans.
struct NumberSpans<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> NumberSpans<'a> {
    fn new(d: &'a str) -> Self {
        Self {
            bytes: d.as_bytes(),
            pos: 0,
        }
    }
}

impl Iterator for NumberSpans<'_> {
    type Item = NumberSpan;

    fn next(&mut self) -> Option<NumberSpan> {
        let n = self.bytes.len();
        while self.pos < n {
            let b = self.bytes[self.pos];
            let starts = b == b'-' || b.is_ascii_digit();
            if !starts {
                self.pos += 1;
                continue;
            }
            let start = self.pos;
            if self.bytes[self.pos] == b'-' {
                self.pos += 1;
            }
            while self.pos < n && self.bytes[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
            if self.pos < n && self.bytes[self.pos] == b'.' {
                self.pos += 1;
                while self.pos < n && self.bytes[self.pos].is_ascii_digit() {
                    self.pos += 1;
                }
            }
            return Some(NumberSpan {
                start,
                end: self.pos,
            });
        }
        None
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

    const IDENTITY: [[f64; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    fn translate(dx: f64, dy: f64) -> [[f64; 3]; 3] {
        [[1.0, 0.0, dx], [0.0, 1.0, dy], [0.0, 0.0, 1.0]]
    }

    fn anchor_at(node_index: usize, target: &str, qx: f64, qy: f64) -> RAnchor {
        RAnchor {
            node_index,
            target: target.to_string(),
            at: RLocalPoint { x: qx, y: qy },
        }
    }

    #[test]
    fn reproject_matches_hand_computed_affine_compose() {
        // follower_base translates by (100,0); target_base translates by (10,20);
        // delta translates by (5,7). The anchor `at` is the target-local point
        // (16,8) QUANTIZED units => (2,1) px (Q=8).
        //
        // world = (delta * target_base) * (2,1) = (10+5+2, 20+7+1) = (17, 28).
        // local = follower_base^-1 * world = (17-100, 28-0) = (-83, 28).
        let follower_base = translate(100.0, 0.0);
        let target_base = translate(10.0, 20.0);
        let delta = translate(5.0, 7.0);
        let at = anchor_at(0, "t", 16.0, 8.0);
        let (fx, fy) =
            reproject_node_local_px(&follower_base, &target_base, &delta, &at).expect("non-singular");
        assert!((fx - (-83.0)).abs() < 1e-9, "fx={fx}");
        assert!((fy - 28.0).abs() < 1e-9, "fy={fy}");
    }

    #[test]
    fn pure_target_translation_moves_follower_node_by_the_same_world_delta() {
        // With identity bases, the follower-local node position equals the de-
        // quantized anchor point; a pure target translation `delta` must shift that
        // node by EXACTLY the same world delta (no scale/rotation in play).
        let at = anchor_at(0, "t", 24.0, 40.0); // (3, 5) px de-quantized.
        let (bx, by) =
            reproject_node_local_px(&IDENTITY, &IDENTITY, &IDENTITY, &at).expect("non-singular");
        let delta = translate(11.0, -4.0);
        let (mx, my) =
            reproject_node_local_px(&IDENTITY, &IDENTITY, &delta, &at).expect("non-singular");
        assert!((bx - 3.0).abs() < 1e-9 && (by - 5.0).abs() < 1e-9, "base ({bx},{by})");
        assert!((mx - bx - 11.0).abs() < 1e-9, "dx={}", mx - bx);
        assert!((my - by - (-4.0)).abs() < 1e-9, "dy={}", my - by);
    }

    #[test]
    fn reproject_is_none_for_a_singular_follower_base() {
        let singular = [[0.0, 0.0, 0.0], [0.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        let at = anchor_at(0, "t", 8.0, 8.0);
        assert!(reproject_node_local_px(&singular, &IDENTITY, &IDENTITY, &at).is_none());
    }

    #[test]
    fn rewrite_geometry_node_replaces_only_the_addressed_pair_quantized() {
        // Node 1 is the `L 64 0` pair; move it to (10, 4) px => (80, 32) quantized.
        let d = "M 0 0 L 64 0 L 64 64 L 0 64 Z";
        let out = rewrite_geometry_node(d, 1, 10.0, 4.0).expect("node 1 is addressable");
        assert_eq!(out, "M 0 0 L 80 32 L 64 64 L 0 64 Z");
    }

    #[test]
    fn rewrite_geometry_node_is_none_when_unchanged_or_unaddressable() {
        let d = "M 0 0 L 64 0";
        // Rewriting node 1 to its EXISTING value (8px,0px => 64,0) is a no-op.
        assert!(rewrite_geometry_node(d, 1, 8.0, 0.0).is_none());
        // Node 9 does not exist.
        assert!(rewrite_geometry_node(d, 9, 1.0, 1.0).is_none());
    }

    #[test]
    fn rewrite_geometry_node_counts_pairs_across_cubic_control_points() {
        // `C` contributes three pairs (two controls + endpoint). Pair indices:
        // 0:M(0,0) 1:c1(8,0) 2:c2(16,8) 3:end(24,8) 4:L(32,8). Move pair 4.
        let d = "M 0 0 C 8 0 16 8 24 8 L 32 8";
        let out = rewrite_geometry_node(d, 4, 5.0, 1.0).expect("pair 4 is the L endpoint");
        assert_eq!(out, "M 0 0 C 8 0 16 8 24 8 L 40 8");
    }
}
