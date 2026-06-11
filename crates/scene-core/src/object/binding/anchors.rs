//! OB3.S4 — anchor endpoint resolution + connection graph (D5).
//!
//! Anchors absorb edges into the single object model: a connector is just an
//! object whose endpoint nodes carry [`Anchor`]s pointing at other objects'
//! derived regions. An anchor stores `at` — a local point on the *target's*
//! outline — but the rendered endpoint position is **derived**, never stored:
//! it is `at` re-projected onto the target's current region, so it tracks the
//! target without drift when the target edits (the OB3.S4 reproject step).
//!
//! Everything here is pure (no time/rng/IO), operates on object-local quantized
//! i32 coordinates, and is pointer-width-agnostic in its serialized surface (the
//! anchor index returned by [`reproject_object_anchors`] is a runtime `usize`
//! position, not an addressing field that ever crosses the wire).

use crate::object::model::{Anchor, LocalPoint, Object, ObjectId, ObjectScene};
use crate::object::region::OutlineDeriver;

/// Resolve the **derived** endpoint position for one anchor of `anchored`.
///
/// The position is `anchor.at` re-projected onto `target`'s derived region
/// (target geometry, in target-local coords). The endpoint is never stored on
/// the anchored object — it is recomputed from the target so it stays glued to
/// the target's outline across edits (D5). Returns `None` when the target's
/// geometry is degenerate/empty and no region can be derived.
///
/// `anchored` is accepted for call-site symmetry and future endpoint-side
/// reprojection (e.g. honoring the anchored node's transform); the projection
/// itself is defined purely on the target region per D5.
pub fn resolve_endpoint(
    deriver: &impl OutlineDeriver,
    anchored: &Object,
    anchor: &Anchor,
    target: &Object,
) -> Option<LocalPoint> {
    let _ = anchored;
    let region = deriver.derive_region(&target.geometry, FLATNESS).ok()?;
    Some(deriver.reproject(&region, anchor.at))
}

/// Recompute every endpoint of `object_id`'s anchors against the *current*
/// target regions (OB3.S4 — called after a target's geometry edits).
///
/// Returns `(anchor index, derived endpoint)` pairs in anchor order. Anchors
/// whose target is missing, or whose target region cannot be derived, are
/// skipped (no panic, no placeholder) so a dangling anchor never invents a
/// position. The returned index is the position within `object.anchors`, which
/// the caller maps back onto the geometry node via `Anchor::node_index`.
pub fn reproject_object_anchors(
    deriver: &impl OutlineDeriver,
    scene: &ObjectScene,
    object_id: &str,
) -> Vec<(usize, LocalPoint)> {
    let Some(object) = scene.get(object_id) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (idx, anchor) in object.anchors.iter().enumerate() {
        let Some(target) = scene.get(&anchor.target) else {
            continue;
        };
        if let Some(p) = resolve_endpoint(deriver, object, anchor, target) {
            out.push((idx, p));
        }
    }
    out
}

/// Edges of the connection graph (D5/D6).
///
/// Each object's anchors define how it wires objects together:
/// - An object with **>= 2** distinct anchor targets is an absorbed edge; it
///   contributes every unordered pair of the distinct targets it connects
///   (for the canonical 2-endpoint connector that is the single target pair).
/// - An object with exactly **one** anchor target is a single-anchor
///   attachment; it contributes `(object id, target id)` so the attachment is
///   still queryable as an edge.
///
/// Order is stable: objects in scene order, then targets in first-seen anchor
/// order. Duplicate target ids within one object are collapsed before pairing,
/// and self-referential targets (an anchor onto the anchored object itself) are
/// dropped so the graph never carries a self-loop.
pub fn connection_graph(scene: &ObjectScene) -> Vec<(ObjectId, ObjectId)> {
    let mut edges = Vec::new();
    for object in &scene.objects {
        let targets = distinct_targets(object);
        match targets.as_slice() {
            [] => {}
            [single] => edges.push((object.id.clone(), single.clone())),
            many => {
                for i in 0..many.len() {
                    for j in (i + 1)..many.len() {
                        edges.push((many[i].clone(), many[j].clone()));
                    }
                }
            }
        }
    }
    edges
}

/// Neighbors of `target_id` in the connection graph: every object on the other
/// end of an edge incident to `target_id` (deduplicated, first-seen order).
pub fn neighbors(scene: &ObjectScene, target_id: &str) -> Vec<ObjectId> {
    let mut out: Vec<ObjectId> = Vec::new();
    for (a, b) in connection_graph(scene) {
        let other = if a == target_id {
            Some(b)
        } else if b == target_id {
            Some(a)
        } else {
            None
        };
        if let Some(other) = other {
            if other != target_id && !out.contains(&other) {
                out.push(other);
            }
        }
    }
    out
}

/// Curve-flattening tolerance for region derivation. Anchor reprojection runs at
/// the finest bucket so endpoints land on the true outline regardless of zoom
/// LOD; the stub deriver ignores it.
const FLATNESS: i32 = 1;

/// Distinct anchor target ids for `object`, in first-seen order, excluding any
/// anchor pointing back at the object itself.
fn distinct_targets(object: &Object) -> Vec<ObjectId> {
    let mut targets: Vec<ObjectId> = Vec::new();
    for anchor in &object.anchors {
        if anchor.target == object.id {
            continue;
        }
        if !targets.contains(&anchor.target) {
            targets.push(anchor.target.clone());
        }
    }
    targets
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::model::{FillRule, Geometry, PathNode, SubPath};
    use crate::object::region::{OutlineDeriver, StubOutlineDeriver};

    /// Closed rect contour with corners (x0,y0)-(x1,y1) in quantized units.
    fn rect(x0: i32, y0: i32, x1: i32, y1: i32) -> Geometry {
        Geometry::from_subpaths(
            vec![SubPath {
                closed: true,
                nodes: vec![
                    PathNode::corner(x0, y0),
                    PathNode::corner(x1, y0),
                    PathNode::corner(x1, y1),
                    PathNode::corner(x0, y1),
                ],
            }],
            FillRule::EvenOdd,
        )
    }

    /// Two-node open polyline whose endpoints will anchor to the two rects.
    fn connector(ax: i32, ay: i32, bx: i32, by: i32) -> Geometry {
        Geometry::from_subpaths(
            vec![SubPath {
                closed: false,
                nodes: vec![PathNode::corner(ax, ay), PathNode::corner(bx, by)],
            }],
            FillRule::EvenOdd,
        )
    }

    /// Two rects + a connector whose two nodes anchor onto each rect.
    fn scene_with_edge() -> ObjectScene {
        let mut rect_a = Object::new("rect-a", "a0", rect(0, 0, 80, 40));
        rect_a.geometry.ensure_parsed().unwrap();
        let mut rect_b = Object::new("rect-b", "a1", rect(200, 0, 280, 40));
        rect_b.geometry.ensure_parsed().unwrap();

        let mut edge = Object::new("edge", "a2", connector(40, 20, 240, 20));
        edge.geometry.ensure_parsed().unwrap();
        // node 0 attaches to a point near rect-a's outline; node 1 near rect-b's.
        edge.anchors = vec![
            Anchor { node_index: 0, target: "rect-a".into(), at: LocalPoint { x: 78, y: 22 } },
            Anchor { node_index: 1, target: "rect-b".into(), at: LocalPoint { x: 202, y: 18 } },
        ];

        ObjectScene {
            scene_version: 1,
            objects: vec![rect_a, rect_b, edge],
            tags: Vec::new(),
            selection: crate::object::model::ObjectSelection::Canvas,
            updated_at: String::new(),
        }
    }

    /// Returns true if `p` is a vertex of `obj`'s derived outline.
    fn on_outline(deriver: &StubOutlineDeriver, obj: &Object, p: LocalPoint) -> bool {
        let region = deriver.derive_region(&obj.geometry, FLATNESS).unwrap();
        region.outline.contains(&p)
    }

    #[test]
    fn resolve_endpoint_projects_onto_target_outline() {
        let deriver = StubOutlineDeriver;
        let scene = scene_with_edge();
        let edge = scene.get("edge").unwrap();
        let rect_a = scene.get("rect-a").unwrap();

        let ep = resolve_endpoint(&deriver, edge, &edge.anchors[0], rect_a).unwrap();
        // The stub reprojects to the nearest outline vertex: (78,22) -> (80,40).
        assert!(on_outline(&deriver, rect_a, ep));
    }

    #[test]
    fn reproject_object_anchors_returns_endpoints_on_targets() {
        let deriver = StubOutlineDeriver;
        let scene = scene_with_edge();

        let reprojected = reproject_object_anchors(&deriver, &scene, "edge");
        assert_eq!(reprojected.len(), 2, "both anchors reproject");
        assert_eq!(reprojected[0].0, 0);
        assert_eq!(reprojected[1].0, 1);

        let rect_a = scene.get("rect-a").unwrap();
        let rect_b = scene.get("rect-b").unwrap();
        assert!(on_outline(&deriver, rect_a, reprojected[0].1));
        assert!(on_outline(&deriver, rect_b, reprojected[1].1));
    }

    #[test]
    fn reproject_skips_missing_targets() {
        let deriver = StubOutlineDeriver;
        let mut scene = scene_with_edge();
        scene.get_mut("edge").unwrap().anchors[1].target = "ghost".into();

        let reprojected = reproject_object_anchors(&deriver, &scene, "edge");
        assert_eq!(reprojected.len(), 1, "the dangling anchor is dropped");
        assert_eq!(reprojected[0].0, 0);
    }

    #[test]
    fn connection_graph_yields_the_connected_target_pair() {
        let scene = scene_with_edge();
        let edges = connection_graph(&scene);
        // The two-anchor edge connects rect-a and rect-b; the rects themselves
        // have no anchors, so that pair is the only edge.
        assert_eq!(edges, vec![("rect-a".to_string(), "rect-b".to_string())]);
    }

    #[test]
    fn connection_graph_emits_single_anchor_attachment() {
        let mut scene = scene_with_edge();
        // Drop the second anchor: the edge is now a single-anchor attachment.
        scene.get_mut("edge").unwrap().anchors.truncate(1);

        let edges = connection_graph(&scene);
        assert_eq!(edges, vec![("edge".to_string(), "rect-a".to_string())]);
    }

    #[test]
    fn neighbors_query_by_target() {
        let scene = scene_with_edge();
        assert_eq!(neighbors(&scene, "rect-a"), vec!["rect-b".to_string()]);
        assert_eq!(neighbors(&scene, "rect-b"), vec!["rect-a".to_string()]);
        assert!(neighbors(&scene, "edge").is_empty());
    }

    #[test]
    fn duplicate_targets_collapse_to_no_self_pair() {
        let mut scene = scene_with_edge();
        // Both endpoints anchor to the same rect: one distinct target, so it is
        // a single-anchor-style attachment, not a self-pair.
        scene.get_mut("edge").unwrap().anchors[1].target = "rect-a".into();

        let edges = connection_graph(&scene);
        assert_eq!(edges, vec![("edge".to_string(), "rect-a".to_string())]);
    }
}
