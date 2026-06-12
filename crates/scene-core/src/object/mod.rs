//! Object-primitive substrate. Internal tiers run kernel <- {authoring, binding,
//! catalog}; the facade below re-surfaces every file-module at the `object::`
//! level so every `object::model`/`object::op`/... path keeps resolving.

pub mod authoring;
pub mod binding;
pub mod catalog;
pub mod kernel;

pub use kernel::{apply, model, op, undo, validate};

pub use authoring::{deform, drawing, merge, primitives, recognize, templates};

pub use binding::{
    anchor_follow, anchors, cascade, grouping, layout_solve, move_together, region,
};

pub use catalog::{commands, gestures, theme};

pub use anchor_follow::{
    anchor_follow_ops, geometry_follow_ops, local_nodes, reproject_geometry_node,
    synthesize_create_anchors,
};
pub use move_together::{BindingGraph, BindingNode, PropEdge, PropKind};
pub use anchors::{connection_graph, neighbors, reproject_object_anchors, resolve_endpoint};
pub use cascade::{cascade_multi_transform_ops, cascade_transform_ops, move_ops, MoveRoots};
pub use apply::{apply_object_op, apply_object_op_lww, apply_sequence, ApplyError};
pub use commands::{
    object_command_catalog, object_command_catalog_json, ObjectCommand, ObjectCommandCategory,
};
pub use deform::{
    deform_open_path, endpoint_release_ops, is_open_class, is_open_class_d, is_pure_translate,
    open_endpoint_pins, route_open_endpoints, EndpointRoute,
};
pub use drawing::{fit_beziers, pressure_to_width, rdp_simplify, Brush};
pub use merge::merge_open_stroke_ops;
pub use recognize::{recognize_stroke, recognize_stroke_object, RecognizeMode, RecognizedStroke};
pub use gestures::{
    object_gesture_catalog, object_gesture_catalog_json, HoldInput, HoldTrigger, ObjectGesture,
    ObjectGestureCategory,
};
pub use grouping::{
    double_click_action, has_children, pop_out_op, ungroup_enabled, DoubleClickAction,
};
pub use primitives::{
    build_primitive, build_primitive_from_drag, build_set_style_op, paint_for_color, DragSpan,
    PrimitiveKind, THEME_DEFAULT_COLOR,
};
pub use layout_solve::solve_layout;
pub use templates::{
    build_template, object_template_catalog, semantic_preset_style, semantic_presets,
    template_to_ops, ObjectTemplate, ObjectTemplateMeta, TemplateCategory,
};
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
pub use theme::{resolve_token, Token, ALL_TOKENS};
pub use region::{LocalBounds, OutlineDeriver, Region, RegionError, StubOutlineDeriver};

#[cfg(test)]
mod tests {
    use super::model::{FillRule, Geometry, Object, PathNode, SubPath, Transform3x3};
    use super::op::ObjectOp;
    use super::region::{OutlineDeriver, RegionError, StubOutlineDeriver};
    use super::*;

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
        assert!(json.contains("\"d\":\"M 0 0 L 80 0 L 80 40 L 0 40 Z\""));
        assert!(!json.contains("subpaths"));
        let mut back: Object = serde_json::from_str(&json).expect("deserialize");
        assert!(back.geometry.subpaths.is_empty());
        back.ensure_parsed().expect("hydrate");
        assert_eq!(back.geometry.subpaths, obj.geometry.subpaths);
    }

    #[test]
    fn vertical_slice_insert_transform_zero_rebake() {
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
        // Zero-rebake: a transform edit must not touch the geometry.
        assert_eq!(scene.get("rect-1").unwrap().geometry, geom_before);
        assert_eq!(scene.get("rect-1").unwrap().transform, Transform3x3::translate(100.0, 50.0));
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
        assert!(scene.get("a").is_none());
        assert!(scene.get("e").unwrap().anchors.is_empty());
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
    fn paint_token_round_trips_through_json() {
        use super::model::Paint;
        let paint = Paint::Token { name: "selection-ring".into() };
        let json = serde_json::to_string(&paint).expect("serialize");
        assert_eq!(json, r#"{"kind":"token","name":"selection-ring"}"#);
        let back: Paint = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, paint);
    }

    #[test]
    fn set_style_field_edit_three_states() {
        use super::model::{Fill, Paint};
        use super::op::FieldEdit;
        let mut scene = ObjectScene::default();
        apply_object_op(&mut scene, ObjectOp::InsertObject { object: Object::new("r", "a0", rect_geometry()) }).unwrap();
        let fill = Fill { paint: Paint::Solid { color: "#ff0000".into() }, opacity: 1.0 };
        let inv = apply_object_op(
            &mut scene,
            ObjectOp::SetStyle { id: "r".into(), fill: Some(FieldEdit::Set { value: fill.clone() }), stroke: None },
        )
        .unwrap();
        assert_eq!(scene.get("r").unwrap().fill, Some(fill));
        apply_object_op(&mut scene, inv).unwrap();
        assert_eq!(scene.get("r").unwrap().fill, None);
    }
}
