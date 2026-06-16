//! Pure structural validators over the object model — reject malformed scenes
//! before they are journaled / applied (degenerate geometry, looping parent
//! chain, anchor/comment addressing a missing node or object). apply.rs gates
//! each op on the relevant validator. Returns a structured [`ValidationError`]
//! matching `ApplyError`'s shape, since the apply path matches on it.
//!
//! Pure: no IO/time/rng. Pointer-width-agnostic — node indices are i32, the
//! flattened count is compared via i64 so no `usize`/`as` narrowing leaks in.
//!
//! Node addressing: an [`Anchor::node_index`] / [`CommentAnchor::Node`] index
//! addresses a node in the object's *own* geometry, counted flat across every
//! subpath in declaration order. Valid range is `0 <= node_index < total_nodes`.

use std::collections::HashMap;

use crate::object::model::{AxisSizing, CommentAnchor, Geometry, Lanes, Layout, Object, ObjectScene, Sizing};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidationError {
    EmptyGeometry,
    /// Open needs >= 2 nodes, closed >= 3.
    DegenerateSubpath { subpath: i32, closed: bool, nodes: i32 },
    ParentCycle { id: String },
    MissingAnchorTarget { id: String, target: String },
    AnchorNodeOutOfRange { id: String, node_index: i32, node_count: i32 },
    CommentNodeOutOfRange { id: String, comment_id: String, node_index: i32, node_count: i32 },
    /// A `Fixed` sizing extent must be a non-negative quantized length.
    NegativeSizing { value: i32 },
    /// `Lanes::Count { value }` must be >= 1 (1 = list, N = grid).
    ZeroLanes,
}

impl core::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ValidationError::EmptyGeometry => write!(f, "geometry has no drawable contour"),
            ValidationError::DegenerateSubpath { subpath, closed, nodes } => write!(
                f,
                "subpath {subpath} is degenerate ({} with {nodes} node(s))",
                if *closed { "closed" } else { "open" }
            ),
            ValidationError::ParentCycle { id } => write!(f, "parent cycle detected at: {id}"),
            ValidationError::MissingAnchorTarget { id, target } => {
                write!(f, "object {id} anchors a missing target: {target}")
            }
            ValidationError::AnchorNodeOutOfRange { id, node_index, node_count } => write!(
                f,
                "object {id} anchor node_index {node_index} out of range (0..{node_count})"
            ),
            ValidationError::CommentNodeOutOfRange {
                id,
                comment_id,
                node_index,
                node_count,
            } => write!(
                f,
                "object {id} comment {comment_id} node_index {node_index} out of range (0..{node_count})"
            ),
            ValidationError::NegativeSizing { value } => {
                write!(f, "fixed sizing extent must be non-negative, got {value}")
            }
            ValidationError::ZeroLanes => write!(f, "layout lane count must be >= 1"),
        }
    }
}

/// A `Fixed` sizing extent on either axis must be a non-negative quantized length.
/// `Hug`/`Fill` carry no value to validate.
pub fn validate_sizing(sizing: &Sizing) -> Result<(), ValidationError> {
    for axis in [sizing.w, sizing.h] {
        if let AxisSizing::Fixed { value } = axis {
            if value < 0 {
                return Err(ValidationError::NegativeSizing { value });
            }
        }
    }
    Ok(())
}

/// A `Lanes::Count` must carry at least one track (1 = single list, N = grid).
pub fn validate_layout(layout: &Layout) -> Result<(), ValidationError> {
    if let Lanes::Count { value } = layout.lanes {
        if value < 1 {
            return Err(ValidationError::ZeroLanes);
        }
    }
    Ok(())
}

/// Total node count across every subpath. Accumulated in i64 and narrowed once
/// with a checked `try_from` so an absurd geometry saturates rather than wrapping.
fn flattened_node_count(geometry: &Geometry) -> i32 {
    let total: i64 = geometry
        .subpaths
        .iter()
        .map(|sp| i64::try_from(sp.nodes.len()).unwrap_or(i64::MAX))
        .sum();
    i32::try_from(total).unwrap_or(i32::MAX)
}

/// Reject an empty path or a subpath too short for its topology. Operates on the
/// parsed `subpaths`; the caller hydrates via `Geometry::ensure_parsed`.
pub fn validate_geometry(geometry: &Geometry) -> Result<(), ValidationError> {
    let has_drawable = geometry.subpaths.iter().any(|sp| !sp.nodes.is_empty());
    if !has_drawable {
        return Err(ValidationError::EmptyGeometry);
    }
    for (i, sp) in geometry.subpaths.iter().enumerate() {
        if sp.nodes.is_empty() {
            continue;
        }
        let min = if sp.closed { 3 } else { 2 };
        if sp.nodes.len() < min {
            let nodes = i32::try_from(sp.nodes.len()).unwrap_or(i32::MAX);
            let subpath = i32::try_from(i).unwrap_or(i32::MAX);
            return Err(ValidationError::DegenerateSubpath {
                subpath,
                closed: sp.closed,
                nodes,
            });
        }
    }
    Ok(())
}

/// No object may be its own ancestor through the `parent` chain. A parent id that
/// resolves to nothing ends the walk (dangling parents are out of scope here).
/// Returns the first offending object.
pub fn validate_no_parent_cycle(scene: &ObjectScene) -> Result<(), ValidationError> {
    let parent_of: HashMap<&str, Option<&str>> = scene
        .objects
        .iter()
        .map(|o| (o.id.as_str(), o.parent.as_deref()))
        .collect();

    for object in &scene.objects {
        let start = object.id.as_str();
        let mut seen: Vec<&str> = vec![start];
        let mut cursor = parent_of.get(start).copied().flatten();
        while let Some(parent) = cursor {
            if seen.contains(&parent) {
                return Err(ValidationError::ParentCycle { id: start.to_string() });
            }
            seen.push(parent);
            cursor = parent_of.get(parent).copied().flatten();
        }
    }
    Ok(())
}

/// Every anchor must resolve: `target` is a live object and `node_index` is in
/// range for the *owning* object's geometry. Returns the first defect.
pub fn validate_anchor_targets(scene: &ObjectScene) -> Result<(), ValidationError> {
    let ids: std::collections::HashSet<&str> =
        scene.objects.iter().map(|o| o.id.as_str()).collect();

    for object in &scene.objects {
        if object.anchors.is_empty() {
            continue;
        }
        let node_count = flattened_node_count(&object.geometry);
        for anchor in &object.anchors {
            if !ids.contains(anchor.target.as_str()) {
                return Err(ValidationError::MissingAnchorTarget {
                    id: object.id.clone(),
                    target: anchor.target.clone(),
                });
            }
            if anchor.node_index < 0 || anchor.node_index >= node_count {
                return Err(ValidationError::AnchorNodeOutOfRange {
                    id: object.id.clone(),
                    node_index: anchor.node_index,
                    node_count,
                });
            }
        }
    }
    Ok(())
}

/// One object in isolation: non-degenerate geometry + in-range `Node`-anchored
/// comments. Anchor targets are cross-object, checked by [`validate_anchor_targets`].
pub fn validate_object(object: &Object) -> Result<(), ValidationError> {
    validate_geometry(&object.geometry)?;
    let node_count = flattened_node_count(&object.geometry);
    for comment in &object.comments {
        if let Some(CommentAnchor::Node { node_index }) = comment.at {
            if node_index < 0 || node_index >= node_count {
                return Err(ValidationError::CommentNodeOutOfRange {
                    id: object.id.clone(),
                    comment_id: comment.id.clone(),
                    node_index,
                    node_count,
                });
            }
        }
    }
    Ok(())
}

/// Run every validator, accumulating all defects (unlike the single-error
/// helpers) so a caller can report every problem at once.
pub fn validate_scene(scene: &ObjectScene) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    for object in &scene.objects {
        if let Err(e) = validate_object(object) {
            errors.push(e);
        }
    }
    if let Err(e) = validate_no_parent_cycle(scene) {
        errors.push(e);
    }
    if let Err(e) = validate_anchor_targets(scene) {
        errors.push(e);
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::model::{
        Anchor, Comment, CommentAnchor, FillRule, Geometry, LocalPoint, Object, ObjectScene,
        PathNode, SubPath,
    };

    fn rect() -> Geometry {
        Geometry::from_subpaths(
            vec![SubPath {
                closed: true,
                nodes: vec![
                    PathNode::corner(0, 0),
                    PathNode::corner(80, 0),
                    PathNode::corner(80, 40),
                    PathNode::corner(0, 40),
                ],
            }],
            FillRule::EvenOdd,
        )
    }

    fn obj(id: &str, parent: Option<&str>) -> Object {
        let mut o = Object::new(id, "a0", rect());
        o.parent = parent.map(|p| p.to_string());
        o
    }

    fn scene(objects: Vec<Object>) -> ObjectScene {
        ObjectScene { objects, ..Default::default() }
    }

    // ---- validate_geometry ---------------------------------------------------

    #[test]
    fn geometry_happy_path() {
        assert_eq!(validate_geometry(&rect()), Ok(()));
        let open = Geometry::from_subpaths(
            vec![SubPath { closed: false, nodes: vec![PathNode::corner(0, 0), PathNode::corner(10, 0)] }],
            FillRule::NonZero,
        );
        assert_eq!(validate_geometry(&open), Ok(()));
    }

    #[test]
    fn geometry_empty_rejected() {
        assert_eq!(validate_geometry(&Geometry::default()), Err(ValidationError::EmptyGeometry));
        let all_empty = Geometry {
            subpaths: vec![SubPath { closed: false, nodes: vec![] }],
            ..Default::default()
        };
        assert_eq!(validate_geometry(&all_empty), Err(ValidationError::EmptyGeometry));
    }

    #[test]
    fn geometry_open_single_node_rejected() {
        let g = Geometry::from_subpaths(
            vec![SubPath { closed: false, nodes: vec![PathNode::corner(5, 5)] }],
            FillRule::NonZero,
        );
        assert_eq!(
            validate_geometry(&g),
            Err(ValidationError::DegenerateSubpath { subpath: 0, closed: false, nodes: 1 })
        );
    }

    #[test]
    fn geometry_closed_two_nodes_rejected() {
        let g = Geometry::from_subpaths(
            vec![SubPath { closed: true, nodes: vec![PathNode::corner(0, 0), PathNode::corner(10, 0)] }],
            FillRule::EvenOdd,
        );
        assert_eq!(
            validate_geometry(&g),
            Err(ValidationError::DegenerateSubpath { subpath: 0, closed: true, nodes: 2 })
        );
    }

    // ---- validate_no_parent_cycle --------------------------------------------

    #[test]
    fn parent_chain_acyclic_passes() {
        let s = scene(vec![
            obj("root", None),
            obj("child", Some("root")),
            obj("grandchild", Some("child")),
        ]);
        assert_eq!(validate_no_parent_cycle(&s), Ok(()));
    }

    #[test]
    fn parent_self_cycle_rejected() {
        let s = scene(vec![obj("solo", Some("solo"))]);
        assert_eq!(validate_no_parent_cycle(&s), Err(ValidationError::ParentCycle { id: "solo".into() }));
    }

    #[test]
    fn parent_two_cycle_rejected() {
        let s = scene(vec![obj("a", Some("b")), obj("b", Some("a"))]);
        assert!(matches!(validate_no_parent_cycle(&s), Err(ValidationError::ParentCycle { .. })));
    }

    #[test]
    fn dangling_parent_is_not_a_cycle() {
        let s = scene(vec![obj("child", Some("ghost"))]);
        assert_eq!(validate_no_parent_cycle(&s), Ok(()));
    }

    // ---- validate_anchor_targets ---------------------------------------------

    #[test]
    fn anchor_targets_happy_path() {
        let mut edge = obj("e", None);
        edge.anchors = vec![Anchor { node_index: 0, target: "a".into(), at: LocalPoint { x: 0, y: 0 } }];
        let s = scene(vec![obj("a", None), edge]);
        assert_eq!(validate_anchor_targets(&s), Ok(()));
    }

    #[test]
    fn anchor_missing_target_rejected() {
        let mut edge = obj("e", None);
        edge.anchors = vec![Anchor { node_index: 0, target: "ghost".into(), at: LocalPoint { x: 0, y: 0 } }];
        let s = scene(vec![edge]);
        assert_eq!(
            validate_anchor_targets(&s),
            Err(ValidationError::MissingAnchorTarget { id: "e".into(), target: "ghost".into() })
        );
    }

    #[test]
    fn anchor_node_index_out_of_range_rejected() {
        let mut edge = obj("e", None);
        edge.anchors = vec![Anchor { node_index: 4, target: "a".into(), at: LocalPoint { x: 0, y: 0 } }];
        let s = scene(vec![obj("a", None), edge]);
        assert_eq!(
            validate_anchor_targets(&s),
            Err(ValidationError::AnchorNodeOutOfRange { id: "e".into(), node_index: 4, node_count: 4 })
        );
    }

    #[test]
    fn anchor_negative_node_index_rejected() {
        let mut edge = obj("e", None);
        edge.anchors = vec![Anchor { node_index: -1, target: "a".into(), at: LocalPoint { x: 0, y: 0 } }];
        let s = scene(vec![obj("a", None), edge]);
        assert_eq!(
            validate_anchor_targets(&s),
            Err(ValidationError::AnchorNodeOutOfRange { id: "e".into(), node_index: -1, node_count: 4 })
        );
    }

    // ---- validate_object -----------------------------------------------------

    #[test]
    fn object_happy_path() {
        let mut o = obj("o", None);
        o.comments = vec![Comment {
            id: "c1".into(),
            author: "me".into(),
            body: "hi".into(),
            at: Some(CommentAnchor::Node { node_index: 0 }),
            resolved: false,
        }];
        assert_eq!(validate_object(&o), Ok(()));
    }

    #[test]
    fn object_propagates_degenerate_geometry() {
        let mut o = obj("o", None);
        o.geometry = Geometry::from_subpaths(
            vec![SubPath { closed: false, nodes: vec![PathNode::corner(0, 0)] }],
            FillRule::NonZero,
        );
        assert_eq!(
            validate_object(&o),
            Err(ValidationError::DegenerateSubpath { subpath: 0, closed: false, nodes: 1 })
        );
    }

    #[test]
    fn object_comment_node_out_of_range_rejected() {
        let mut o = obj("o", None);
        o.comments = vec![Comment {
            id: "c1".into(),
            author: "me".into(),
            body: "hi".into(),
            at: Some(CommentAnchor::Node { node_index: 9 }),
            resolved: false,
        }];
        assert_eq!(
            validate_object(&o),
            Err(ValidationError::CommentNodeOutOfRange {
                id: "o".into(),
                comment_id: "c1".into(),
                node_index: 9,
                node_count: 4,
            })
        );
    }

    #[test]
    fn object_point_anchored_comment_skips_node_check() {
        let mut o = obj("o", None);
        o.comments = vec![Comment {
            id: "c1".into(),
            author: "me".into(),
            body: "hi".into(),
            at: Some(CommentAnchor::Point { at: LocalPoint { x: 999, y: 999 } }),
            resolved: false,
        }];
        assert_eq!(validate_object(&o), Ok(()));
    }

    // ---- validate_scene ------------------------------------------------------

    #[test]
    fn scene_happy_path_collects_nothing() {
        let mut edge = obj("e", Some("a"));
        edge.anchors = vec![Anchor { node_index: 0, target: "a".into(), at: LocalPoint { x: 0, y: 0 } }];
        let s = scene(vec![obj("a", None), edge]);
        assert_eq!(validate_scene(&s), Vec::new());
    }

    #[test]
    fn scene_collects_all_defects() {
        let mut bad_geo = obj("bad-geo", None);
        bad_geo.geometry = Geometry::from_subpaths(
            vec![SubPath { closed: true, nodes: vec![PathNode::corner(0, 0), PathNode::corner(1, 0)] }],
            FillRule::EvenOdd,
        );
        let loops = obj("loops", Some("loops"));
        let mut anchored = obj("anchored", None);
        anchored.anchors =
            vec![Anchor { node_index: 0, target: "ghost".into(), at: LocalPoint { x: 0, y: 0 } }];

        let s = scene(vec![bad_geo, loops, anchored]);
        let errors = validate_scene(&s);
        assert!(errors.iter().any(|e| matches!(e, ValidationError::DegenerateSubpath { .. })));
        assert!(errors.iter().any(|e| matches!(e, ValidationError::ParentCycle { id } if id == "loops")));
        assert!(errors
            .iter()
            .any(|e| matches!(e, ValidationError::MissingAnchorTarget { target, .. } if target == "ghost")));
    }
}
