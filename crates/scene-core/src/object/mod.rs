//! Object-primitive substrate (OB-1 contract keystone).
//!
//! A new submodule authored alongside the legacy `model.rs`/`op.rs`/`apply.rs`
//! during the OB-1..OB-3 parallel run, so nothing existing breaks. The model
//! cutover (OB4.1) re-points consumers here and deletes the legacy types.
//!
//! - [`model`] — the single [`model::Object`] + sub-types + SVG-subset
//!   path-string codec (D1/D2/D4/D5/D7/D18/D20).
//! - [`op`] — the [`op::ObjectOp`] union + wire feature channels (D21/OB1.2).
//! - [`region`] — derived outline/region contract (D6/OB1.3).
//! - [`apply`] — op-apply + inverse-op capture (OB3.S1 slice).
//! - [`anchors`] — anchor endpoint resolution + connection graph (OB3.S4/D5).
//! - [`undo`] — per-actor undo/redo engine (OB3.S8/D21).
//! - [`commands`] — object command catalog (OB3.S9).
//! - [`layout_solve`] — thin auto-layout solve (OB3.A1/D3).
//! - [`validate`] — pure structural validators (OB3.S2).

pub mod anchors;
pub mod apply;
pub mod commands;
pub mod layout_solve;
pub mod model;
pub mod op;
pub mod region;
pub mod undo;
pub mod validate;

pub use anchors::{connection_graph, neighbors, reproject_object_anchors, resolve_endpoint};
pub use apply::{apply_object_op, apply_object_op_lww, apply_sequence, ApplyError};
pub use commands::{
    object_command_catalog, object_command_catalog_json, ObjectCommand, ObjectCommandCategory,
};
pub use layout_solve::solve_layout;
pub use undo::{UndoEntry, UndoStack};
pub use validate::{
    validate_anchor_targets, validate_geometry, validate_no_parent_cycle, validate_object,
    validate_scene, ValidationError,
};
pub use model::{
    Anchor, Comment, CommentAnchor, ContentEmbed, Fill, FillRule, Geometry, GradientStop,
    HandlePoint, Layout, LayoutAlign, LayoutDirection, LayoutSizing, LineCap, LineJoin, LocalPoint,
    Object, ObjectId, ObjectMeta, ObjectScene, ObjectSelection, Paint, PathNode, Stroke, SubPath,
    TagDef, Text, TextAlign, TextRun, TextVAlign, Transform3x3, Warp, GEOMETRY_QUANTUM_PER_PX,
};
pub use op::{FeatureRequest, FeatureResponse, FieldEdit, ObjectOp};
pub use region::{LocalBounds, OutlineDeriver, Region, RegionError, StubOutlineDeriver};

#[cfg(test)]
mod tests {
    use super::model::{FillRule, Geometry, Object, PathNode, SubPath, Transform3x3};
    use super::op::ObjectOp;
    use super::region::{OutlineDeriver, RegionError, StubOutlineDeriver};
    use super::*;

    /// A closed unit rect at (0,0)-(80,40) in quantized units.
    fn rect_geometry() -> Geometry {
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

    #[test]
    fn path_string_round_trips_rect() {
        let g = rect_geometry();
        assert_eq!(g.path_string, "M 0 0 L 80 0 L 80 40 L 0 40 Z");
        let mut reparsed = Geometry { path_string: g.path_string.clone(), ..Default::default() };
        reparsed.parse().expect("parse");
        assert_eq!(reparsed.subpaths, g.subpaths);
    }

    #[test]
    fn path_string_round_trips_cubic() {
        // A node with handles encodes as an absolute cubic and parses back to
        // the same relative handle offsets.
        let sub = SubPath {
            closed: false,
            nodes: vec![
                PathNode { x: 0, y: 0, in_handle: None, out_handle: Some(super::model::HandlePoint { dx: 10, dy: 0 }), width: None },
                PathNode { x: 40, y: 40, in_handle: Some(super::model::HandlePoint { dx: -10, dy: 0 }), out_handle: None, width: None },
            ],
        };
        let g = Geometry::from_subpaths(vec![sub.clone()], FillRule::NonZero);
        assert_eq!(g.path_string, "M 0 0 C 10 0 30 40 40 40");
        let mut reparsed = Geometry { path_string: g.path_string.clone(), ..Default::default() };
        reparsed.parse().expect("parse");
        assert_eq!(reparsed.subpaths, vec![sub]);
    }

    #[test]
    fn object_json_round_trips_via_path_string() {
        let obj = Object::new("rect-1", "a0", rect_geometry());
        let json = serde_json::to_string(&obj).expect("serialize");
        // Geometry serializes as { d, fillRule } — no `subpaths` array.
        assert!(json.contains("\"d\":\"M 0 0 L 80 0 L 80 40 L 0 40 Z\""));
        assert!(!json.contains("subpaths"));
        let mut back: Object = serde_json::from_str(&json).expect("deserialize");
        assert!(back.geometry.subpaths.is_empty());
        back.ensure_parsed().expect("hydrate");
        assert_eq!(back.geometry.subpaths, obj.geometry.subpaths);
    }

    #[test]
    fn vertical_slice_insert_transform_zero_rebake() {
        // OB2.1: insert a rect object -> scene; move it via 3x3 transform only.
        let mut scene = ObjectScene::default();
        let obj = Object::new("rect-1", "a0", rect_geometry());
        let inv_insert = apply_object_op(&mut scene, ObjectOp::InsertObject { object: obj.clone() })
            .expect("insert");
        assert_eq!(scene.objects.len(), 1);
        assert_eq!(inv_insert, ObjectOp::Delete { id: "rect-1".into() });

        let geom_before = scene.get("rect-1").unwrap().geometry.clone();
        let inv_move = apply_object_op(
            &mut scene,
            ObjectOp::SetTransform { id: "rect-1".into(), transform: Transform3x3::translate(100.0, 50.0) },
        )
        .expect("move");
        // Zero-rebake: a transform edit must not touch the geometry (P4).
        assert_eq!(scene.get("rect-1").unwrap().geometry, geom_before);
        assert_eq!(scene.get("rect-1").unwrap().transform, Transform3x3::translate(100.0, 50.0));
        // The inverse restores the identity transform.
        assert_eq!(inv_move, ObjectOp::SetTransform { id: "rect-1".into(), transform: Transform3x3::IDENTITY });
    }

    #[test]
    fn undo_restores_prior_state() {
        let mut scene = ObjectScene::default();
        apply_object_op(&mut scene, ObjectOp::InsertObject { object: Object::new("r", "a0", rect_geometry()) }).unwrap();
        let before = scene.clone();
        let inv = apply_object_op(
            &mut scene,
            ObjectOp::SetTransform { id: "r".into(), transform: Transform3x3::translate(5.0, 5.0) },
        )
        .unwrap();
        // Authoring the inverse through the same path restores prior state (D21).
        apply_object_op(&mut scene, inv).unwrap();
        assert_eq!(scene.get("r").unwrap().transform, before.get("r").unwrap().transform);
    }

    #[test]
    fn delete_inverse_restores_object_and_peer_anchors() {
        let mut scene = ObjectScene::default();
        apply_object_op(&mut scene, ObjectOp::InsertObject { object: Object::new("a", "a0", rect_geometry()) }).unwrap();
        let mut edge = Object::new("e", "a1", rect_geometry());
        edge.anchors = vec![super::model::Anchor { node_index: 0, target: "a".into(), at: super::model::LocalPoint { x: 0, y: 0 } }];
        apply_object_op(&mut scene, ObjectOp::InsertObject { object: edge }).unwrap();

        let inv = apply_object_op(&mut scene, ObjectOp::Delete { id: "a".into() }).unwrap();
        // Deleting `a` prunes the peer anchor on `e`.
        assert!(scene.get("a").is_none());
        assert!(scene.get("e").unwrap().anchors.is_empty());
        // The inverse is a batch that re-inserts `a` AND restores `e`'s anchor.
        apply_object_op(&mut scene, inv).unwrap();
        assert!(scene.get("a").is_some());
        assert_eq!(scene.get("e").unwrap().anchors.len(), 1);
    }

    #[test]
    fn region_derives_rect_bounds_and_hit_test() {
        let g = rect_geometry();
        let deriver = StubOutlineDeriver;
        let region = deriver.derive_region(&g, 1).expect("region");
        assert!(region.closed);
        assert_eq!(region.bounds, super::region::LocalBounds { min_x: 0, min_y: 0, max_x: 80, max_y: 40 });
        // A point inside the rect hits; one outside misses.
        assert!(deriver.contains(&region, super::model::LocalPoint { x: 40, y: 20 }));
        assert!(!deriver.contains(&region, super::model::LocalPoint { x: 200, y: 200 }));
    }

    #[test]
    fn empty_geometry_rejected_cleanly() {
        let deriver = StubOutlineDeriver;
        let g = Geometry::default();
        assert_eq!(deriver.derive_region(&g, 1), Err(RegionError::Empty));
    }

    #[test]
    fn set_style_field_edit_three_states() {
        use super::model::{Fill, Paint};
        use super::op::FieldEdit;
        let mut scene = ObjectScene::default();
        apply_object_op(&mut scene, ObjectOp::InsertObject { object: Object::new("r", "a0", rect_geometry()) }).unwrap();
        let fill = Fill { paint: Paint::Solid { color: "#ff0000".into() }, opacity: 1.0 };
        // Set fill, leave stroke untouched (None).
        let inv = apply_object_op(
            &mut scene,
            ObjectOp::SetStyle { id: "r".into(), fill: Some(FieldEdit::Set { value: fill.clone() }), stroke: None },
        )
        .unwrap();
        assert_eq!(scene.get("r").unwrap().fill, Some(fill));
        // Inverse clears it back to None.
        apply_object_op(&mut scene, inv).unwrap();
        assert_eq!(scene.get("r").unwrap().fill, None);
    }
}
