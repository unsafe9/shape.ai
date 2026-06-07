//! OB5.2 / OB5.3 — object scene-core hardening: golden + round-trip + regression.
//!
//! These are end-to-end checks against the public `shape_scene_core::object`
//! surface (a `tests/` integration binary, so only the crate's re-exported API is
//! in scope — no private internals). Rust is the single source of truth for the
//! golden vector: it is embedded inline below, not derived from a TS oracle.
//!
//! - OB5.2 GOLDEN: a fixed, deterministic op sequence built on an empty scene must
//!   serialize to a stable, byte-exact JSON, and re-running the same sequence must
//!   produce byte-identical output (determinism).
//! - OB5.2 ROUND-TRIP: path-string <-> parsed-geometry on several contour shapes,
//!   and a full ObjectScene JSON serialize -> deserialize -> ensure_parsed equality.
//! - OB5.3 REGRESSION: anchor re-projection on target edit, split<->merge round
//!   trip, deterministic auto-layout spacing, coalesced-drag undo/redo, an identity
//!   3-tier split/merge, and region point-in-polygon hit-test (AA + rotated).

use shape_scene_core::object::{
    apply_object_op, apply_sequence, reproject_object_anchors, solve_layout, Anchor, Fill,
    FillRule, Geometry, HandlePoint, Layout, LayoutAlign, LayoutDirection, LayoutSizing, LocalPoint,
    Object, ObjectOp, ObjectScene, OutlineDeriver, Paint, PathNode, Stroke, StubOutlineDeriver,
    SubPath, Text, TextRun, Transform3x3, UndoStack,
};
use shape_scene_core::object::region::point_in_polygon;

// ---------------------------------------------------------------------------
// Shared geometry builders (object-local quantized i32, 1/8 px units).
// ---------------------------------------------------------------------------

/// A closed axis-aligned rect with corners (x0,y0)-(x1,y1).
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

/// A two-node open polyline (a connector skeleton).
fn connector(ax: i32, ay: i32, bx: i32, by: i32) -> Geometry {
    Geometry::from_subpaths(
        vec![SubPath {
            closed: false,
            nodes: vec![PathNode::corner(ax, ay), PathNode::corner(bx, by)],
        }],
        FillRule::EvenOdd,
    )
}

// ---------------------------------------------------------------------------
// OB5.2 GOLDEN — a fixed op sequence -> stable, byte-exact ObjectScene JSON.
// ---------------------------------------------------------------------------

/// Build the golden scene from an empty `ObjectScene` by applying a fixed,
/// deterministic op sequence: insert a rect + a text object + a connector with
/// two anchors, then set-transform, set-style, edit-geometry, and reorder.
///
/// Every input is integer/`*.0` valued so the serialized form is fully
/// predictable (no float-formatting ambiguity), and all `order` keys are fixed
/// literals (no key generation), so the output is a stable golden.
fn build_golden_scene() -> ObjectScene {
    let mut scene = ObjectScene::default();

    // 1. A rect object at z-order "a0".
    let rect_obj = Object::new("rect-1", "a0", rect(0, 0, 80, 40));

    // 2. A text object at "a1" (its own small box geometry + a single run).
    let mut text_obj = Object::new("text-1", "a1", rect(0, 0, 160, 24));
    text_obj.text = Some(Text {
        runs: vec![TextRun {
            text: "hello".into(),
            color: None,
            size: Some(96),
            bold: false,
            italic: false,
            font: None,
        }],
        align: shape_scene_core::object::TextAlign::Start,
        valign: shape_scene_core::object::TextVAlign::Top,
    });

    // 3. A connector at "a2" whose two endpoint nodes anchor onto the rect+text.
    let mut conn = Object::new("conn-1", "a2", connector(40, 20, 80, 12));
    conn.anchors = vec![
        Anchor { node_index: 0, target: "rect-1".into(), at: LocalPoint { x: 80, y: 40 } },
        Anchor { node_index: 1, target: "text-1".into(), at: LocalPoint { x: 0, y: 0 } },
    ];

    let ops = vec![
        ObjectOp::InsertObject { object: rect_obj },
        ObjectOp::InsertObject { object: text_obj },
        ObjectOp::InsertObject { object: conn },
        // set-transform: translate the rect (integer px so it formats as `*.0`).
        ObjectOp::SetTransform {
            id: "rect-1".into(),
            transform: Transform3x3::translate(100.0, 50.0),
        },
        // set-style: give the rect a solid fill + a solid stroke.
        ObjectOp::SetStyle {
            id: "rect-1".into(),
            fill: Some(shape_scene_core::object::FieldEdit::Set {
                value: Fill { paint: Paint::Solid { color: "#ff0000".into() }, opacity: 1.0 },
            }),
            stroke: Some(shape_scene_core::object::FieldEdit::Set {
                value: Stroke {
                    paint: Paint::Solid { color: "#000000".into() },
                    width: 8,
                    opacity: 1.0,
                    dash: Vec::new(),
                    cap: shape_scene_core::object::LineCap::Butt,
                    join: shape_scene_core::object::LineJoin::Miter,
                },
            }),
        },
        // edit-geometry: grow the rect to (0,0)-(120,40).
        ObjectOp::EditGeometry { id: "rect-1".into(), geometry: rect(0, 0, 120, 40) },
        // reorder: bump the text object's z-order key.
        ObjectOp::Reorder { id: "text-1".into(), order: "a5".into() },
    ];

    apply_sequence(&mut scene, ops).expect("golden op sequence applies");
    scene
}

/// The byte-exact expected golden JSON for `build_golden_scene()`. Rust is the
/// single source of truth — this is hand-locked, not generated from a TS oracle.
const GOLDEN_JSON: &str = concat!(
    "{",
    "\"sceneVersion\":7,",
    "\"objects\":[",
    // rect-1: translated, filled+stroked, geometry grown, order "a0".
    "{",
    "\"id\":\"rect-1\",",
    "\"order\":\"a0\",",
    "\"transform\":[[1.0,0.0,100.0],[0.0,1.0,50.0],[0.0,0.0,1.0]],",
    "\"geometry\":{\"d\":\"M 0 0 L 120 0 L 120 40 L 0 40 Z\",\"fillRule\":\"evenOdd\"},",
    "\"fill\":{\"paint\":{\"kind\":\"solid\",\"color\":\"#ff0000\"},\"opacity\":1.0},",
    "\"stroke\":{\"paint\":{\"kind\":\"solid\",\"color\":\"#000000\"},\"width\":8,\"opacity\":1.0,\"cap\":\"butt\",\"join\":\"miter\"}",
    "},",
    // text-1: reordered to "a5", a single text run.
    "{",
    "\"id\":\"text-1\",",
    "\"order\":\"a5\",",
    "\"transform\":[[1.0,0.0,0.0],[0.0,1.0,0.0],[0.0,0.0,1.0]],",
    "\"geometry\":{\"d\":\"M 0 0 L 160 0 L 160 24 L 0 24 Z\",\"fillRule\":\"evenOdd\"},",
    "\"text\":{\"runs\":[{\"text\":\"hello\",\"size\":96,\"bold\":false,\"italic\":false}],\"align\":\"start\",\"valign\":\"top\"}",
    "},",
    // conn-1: open polyline with two anchors.
    "{",
    "\"id\":\"conn-1\",",
    "\"order\":\"a2\",",
    "\"transform\":[[1.0,0.0,0.0],[0.0,1.0,0.0],[0.0,0.0,1.0]],",
    "\"geometry\":{\"d\":\"M 40 20 L 80 12\",\"fillRule\":\"evenOdd\"},",
    "\"anchors\":[",
    "{\"nodeIndex\":0,\"target\":\"rect-1\",\"at\":{\"x\":80,\"y\":40}},",
    "{\"nodeIndex\":1,\"target\":\"text-1\",\"at\":{\"x\":0,\"y\":0}}",
    "]",
    "}",
    "],",
    "\"tags\":[],",
    "\"selection\":{\"kind\":\"canvas\"},",
    "\"updatedAt\":\"\"",
    "}",
);

#[test]
fn ob52_golden_scene_serializes_to_expected_json() {
    let scene = build_golden_scene();
    let json = serde_json::to_string(&scene).expect("serialize golden scene");
    assert_eq!(json, GOLDEN_JSON, "golden scene JSON drifted from the locked vector");
}

#[test]
fn ob52_golden_sequence_is_deterministic() {
    // Applying the identical fixed sequence twice yields byte-identical JSON.
    let a = serde_json::to_string(&build_golden_scene()).expect("serialize a");
    let b = serde_json::to_string(&build_golden_scene()).expect("serialize b");
    assert_eq!(a, b, "the same op sequence must be byte-deterministic");
}

// ---------------------------------------------------------------------------
// OB5.2 ROUND-TRIP — path-string <-> parsed geometry, and scene JSON round-trip.
// ---------------------------------------------------------------------------

/// Encode contours to a path-string, parse them back, and assert the parsed form
/// equals the original (path-string is the canonical at-rest/wire encoding).
fn assert_path_round_trip(subpaths: Vec<SubPath>, fill_rule: FillRule, expected_d: &str) {
    let g = Geometry::from_subpaths(subpaths.clone(), fill_rule);
    assert_eq!(g.path_string, expected_d, "path-string encoding drifted");

    let mut reparsed = Geometry { path_string: g.path_string.clone(), ..Default::default() };
    reparsed.parse().expect("parse path-string");
    assert_eq!(reparsed.subpaths, subpaths, "parsed contours differ from the source");
}

#[test]
fn ob52_path_string_round_trips_rect() {
    let sub = vec![SubPath {
        closed: true,
        nodes: vec![
            PathNode::corner(0, 0),
            PathNode::corner(80, 0),
            PathNode::corner(80, 40),
            PathNode::corner(0, 40),
        ],
    }];
    assert_path_round_trip(sub, FillRule::EvenOdd, "M 0 0 L 80 0 L 80 40 L 0 40 Z");
}

#[test]
fn ob52_path_string_round_trips_multi_subpath_donut() {
    // A donut: an outer closed ring + an inner closed ring (two subpaths, the
    // even-odd hole). The codec must preserve both contours and their order.
    let outer = SubPath {
        closed: true,
        nodes: vec![
            PathNode::corner(0, 0),
            PathNode::corner(100, 0),
            PathNode::corner(100, 100),
            PathNode::corner(0, 100),
        ],
    };
    let inner = SubPath {
        closed: true,
        nodes: vec![
            PathNode::corner(25, 25),
            PathNode::corner(75, 25),
            PathNode::corner(75, 75),
            PathNode::corner(25, 75),
        ],
    };
    assert_path_round_trip(
        vec![outer, inner],
        FillRule::EvenOdd,
        "M 0 0 L 100 0 L 100 100 L 0 100 Z M 25 25 L 75 25 L 75 75 L 25 75 Z",
    );
}

#[test]
fn ob52_path_string_round_trips_cubic() {
    // An open cubic: node 0 has an out-handle, node 1 an in-handle; the codec
    // converts to/from absolute control points and back to relative offsets.
    let sub = SubPath {
        closed: false,
        nodes: vec![
            PathNode {
                x: 0,
                y: 0,
                in_handle: None,
                out_handle: Some(HandlePoint { dx: 10, dy: 0 }),
                width: None,
            },
            PathNode {
                x: 40,
                y: 40,
                in_handle: Some(HandlePoint { dx: -10, dy: 0 }),
                out_handle: None,
                width: None,
            },
        ],
    };
    assert_path_round_trip(vec![sub], FillRule::NonZero, "M 0 0 C 10 0 30 40 40 40");
}

#[test]
fn ob52_scene_json_serialize_deserialize_ensure_parsed_equals_original() {
    // The golden scene (rect + text + anchored connector) round-trips through
    // JSON: serialize -> deserialize -> ensure_parsed must equal the original
    // (subpaths are #[serde(skip)] and rehydrate from the path-string).
    let original = build_golden_scene();
    let json = serde_json::to_string(&original).expect("serialize");
    let mut back: ObjectScene = serde_json::from_str(&json).expect("deserialize");

    // Before hydration, parsed contours are empty (path-string is the at-rest form).
    assert!(back.objects.iter().all(|o| o.geometry.subpaths.is_empty()));

    back.ensure_parsed().expect("hydrate parsed geometry");
    assert_eq!(back, original, "scene differs after JSON round-trip + ensure_parsed");
}

// ---------------------------------------------------------------------------
// OB5.3 REGRESSION
// ---------------------------------------------------------------------------

/// Two rects + a connector whose two endpoint nodes anchor onto each rect.
fn scene_with_anchored_edge() -> ObjectScene {
    let mut scene = ObjectScene::default();
    apply_sequence(
        &mut scene,
        vec![
            ObjectOp::InsertObject { object: Object::new("rect-a", "a0", rect(0, 0, 80, 40)) },
            ObjectOp::InsertObject { object: Object::new("rect-b", "a1", rect(200, 0, 280, 40)) },
            {
                let mut edge = Object::new("edge", "a2", connector(40, 20, 240, 20));
                edge.anchors = vec![
                    Anchor { node_index: 0, target: "rect-a".into(), at: LocalPoint { x: 78, y: 22 } },
                    Anchor { node_index: 1, target: "rect-b".into(), at: LocalPoint { x: 202, y: 18 } },
                ];
                ObjectOp::InsertObject { object: edge }
            },
        ],
    )
    .expect("seed anchored-edge scene");
    scene
}

#[test]
fn ob53_anchor_reprojects_after_target_geometry_edit() {
    let deriver = StubOutlineDeriver;
    let mut scene = scene_with_anchored_edge();

    // Endpoint of anchor 0 before editing rect-a (stub reprojects to the nearest
    // outline vertex of (78,22) -> (80,40)).
    let before = reproject_object_anchors(&deriver, &scene, "edge");
    assert_eq!(before.len(), 2);
    assert_eq!(before[0], (0, LocalPoint { x: 80, y: 40 }));

    // Edit rect-a's geometry so its outline vertices move; the same anchor `at`
    // must reproject to a different endpoint (re-projection tracks the target).
    apply_object_op(
        &mut scene,
        ObjectOp::EditGeometry { id: "rect-a".into(), geometry: rect(0, 0, 200, 200) },
    )
    .expect("edit rect-a");

    let after = reproject_object_anchors(&deriver, &scene, "edge");
    assert_eq!(after.len(), 2);
    // (78,22) is now nearest the grown rect's (0,0) corner, not (80,40).
    assert_eq!(after[0], (0, LocalPoint { x: 0, y: 0 }));
    assert_ne!(
        after[0].1, before[0].1,
        "anchor endpoint must change when the target geometry edits"
    );
}

#[test]
fn ob53_split_then_merge_restores_original() {
    // A two-contour donut object. Split it into two single-contour objects, then
    // merge them back: the merged geometry must restore the original path-string.
    let donut = Geometry::from_subpaths(
        vec![
            SubPath {
                closed: true,
                nodes: vec![
                    PathNode::corner(0, 0),
                    PathNode::corner(100, 0),
                    PathNode::corner(100, 100),
                    PathNode::corner(0, 100),
                ],
            },
            SubPath {
                closed: true,
                nodes: vec![
                    PathNode::corner(25, 25),
                    PathNode::corner(75, 25),
                    PathNode::corner(75, 75),
                    PathNode::corner(25, 75),
                ],
            },
        ],
        FillRule::EvenOdd,
    );
    let original_d = donut.path_string.clone();

    let mut scene = ObjectScene::default();
    apply_object_op(&mut scene, ObjectOp::InsertObject { object: Object::new("d", "a0", donut) })
        .expect("insert donut");

    // Split every contour into its own object.
    apply_object_op(
        &mut scene,
        ObjectOp::Split {
            id: "d".into(),
            new_ids: vec!["d-outer".into(), "d-inner".into()],
            contours: Vec::new(),
        },
    )
    .expect("split donut");
    // Source consumed (all contours peeled); two new single-contour objects exist.
    assert!(scene.get("d").is_none());
    assert!(scene.get("d-outer").is_some());
    assert!(scene.get("d-inner").is_some());

    // Merge the two halves back into one object.
    apply_object_op(
        &mut scene,
        ObjectOp::Merge {
            ids: vec!["d-outer".into(), "d-inner".into()],
            into: Some("d-outer".into()),
        },
    )
    .expect("merge halves");

    let merged = scene.get("d-outer").expect("survivor exists");
    assert_eq!(
        merged.geometry.path_string, original_d,
        "split -> merge must restore the original multi-subpath geometry"
    );
    assert!(scene.get("d-inner").is_none(), "the merged-away object is gone");
}

#[test]
fn ob53_split_inverse_then_inverse_round_trips_identity_three_tier() {
    // Identity round trip via the apply inverses: an object's split returns a
    // faithful inverse; applying it restores the exact pre-split scene, and
    // re-applying *that* inverse restores the post-split scene (3-tier identity).
    let mut scene = ObjectScene::default();
    apply_object_op(
        &mut scene,
        ObjectOp::InsertObject {
            object: Object::new("g", "a0", rect(0, 0, 100, 100)),
        },
    )
    .expect("insert");
    // Give it a second contour so split produces two objects.
    apply_object_op(
        &mut scene,
        ObjectOp::EditGeometry {
            id: "g".into(),
            geometry: Geometry::from_subpaths(
                vec![
                    SubPath {
                        closed: true,
                        nodes: vec![
                            PathNode::corner(0, 0),
                            PathNode::corner(100, 0),
                            PathNode::corner(100, 100),
                            PathNode::corner(0, 100),
                        ],
                    },
                    SubPath {
                        closed: true,
                        nodes: vec![
                            PathNode::corner(25, 25),
                            PathNode::corner(75, 25),
                            PathNode::corner(75, 75),
                            PathNode::corner(25, 75),
                        ],
                    },
                ],
                FillRule::EvenOdd,
            ),
        },
    )
    .expect("two-contour geometry");

    let mut pre_split = scene.clone();
    pre_split.ensure_parsed().expect("hydrate pre-split");

    // Tier 1: split -> capture the faithful inverse.
    let inv_split = apply_object_op(
        &mut scene,
        ObjectOp::Split {
            id: "g".into(),
            new_ids: vec!["g0".into(), "g1".into()],
            contours: Vec::new(),
        },
    )
    .expect("split");
    let mut post_split = scene.clone();
    post_split.ensure_parsed().expect("hydrate post-split");

    // Tier 2: apply the inverse -> back to the pre-split scene (modulo version).
    let inv_undo = apply_object_op(&mut scene, inv_split).expect("apply split inverse");
    scene.ensure_parsed().expect("hydrate after undo");
    assert!(scene.get("g").is_some(), "pre-split object restored");
    assert!(scene.get("g0").is_none() && scene.get("g1").is_none());
    assert_eq!(
        scene.get("g").unwrap().geometry.path_string,
        pre_split.get("g").unwrap().geometry.path_string,
        "split inverse restored the original geometry",
    );

    // Tier 3: apply the re-inverse -> back to the post-split scene.
    apply_object_op(&mut scene, inv_undo).expect("apply re-inverse");
    scene.ensure_parsed().expect("hydrate after redo");
    assert!(scene.get("g").is_none());
    assert!(scene.get("g0").is_some() && scene.get("g1").is_some());
    assert_eq!(
        scene.get("g0").unwrap().geometry.path_string,
        post_split.get("g0").unwrap().geometry.path_string,
        "re-inverse restored the post-split geometry",
    );
}

#[test]
fn ob53_auto_layout_spaces_children_deterministically() {
    // A row group of three unit-rects (80x40 quantized = 10x5 px), gap 16q = 2px,
    // padding 0. solve_layout spaces them along x by width(10) + gap(2) = 12px,
    // and the result is order-stable + repeatable.
    let mut scene = ObjectScene::default();
    let mut group = Object::new("grp", "g0", Geometry::default());
    group.layout = Some(Layout {
        direction: LayoutDirection::Row,
        gap: 16,
        padding: 0,
        align: LayoutAlign::Start,
        sizing: LayoutSizing::Hug,
    });
    scene.objects.push(group);
    for (id, order) in [("c", "a2"), ("a", "a0"), ("b", "a1")] {
        let mut child = Object::new(id, order, rect(0, 0, 80, 40));
        child.parent = Some("grp".into());
        child.geometry.ensure_parsed().expect("hydrate child");
        scene.objects.push(child);
    }

    let out = solve_layout(&scene, "grp", &StubOutlineDeriver);
    // Children come back in fractional-order (a,b,c) regardless of insertion order.
    let ids: Vec<&str> = out.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, vec!["a", "b", "c"]);

    let xs: Vec<f64> = out.iter().map(|(_, t)| t.m[0][2]).collect();
    assert!((xs[0] - 0.0).abs() < 1e-9, "first x = {}", xs[0]);
    assert!((xs[1] - 12.0).abs() < 1e-9, "second x = {}", xs[1]);
    assert!((xs[2] - 24.0).abs() < 1e-9, "third x = {}", xs[2]);

    // Deterministic: re-solving the same scene yields an identical layout.
    let again = solve_layout(&scene, "grp", &StubOutlineDeriver);
    assert_eq!(out, again, "auto-layout solve must be deterministic");
}

#[test]
fn ob53_coalesced_drag_is_one_undo_step_redo_replays() {
    // A drag is many set-transform ops folded into ONE undo entry: undo lands at
    // the pre-gesture state, redo replays the gesture's final state (D21).
    let mut scene = ObjectScene::default();
    apply_object_op(
        &mut scene,
        ObjectOp::InsertObject { object: Object::new("r", "a0", rect(0, 0, 80, 40)) },
    )
    .expect("insert");

    let mut stack = UndoStack::new("dragger".into());
    stack.begin_coalesce();
    assert!(stack.is_coalescing());
    for step in 1..=5_i32 {
        let forward = ObjectOp::SetTransform {
            id: "r".into(),
            transform: Transform3x3::translate(f64::from(step) * 10.0, 0.0),
        };
        let inverse = apply_object_op(&mut scene, forward.clone()).expect("apply drag step");
        stack.record(forward, inverse);
    }
    stack.end_coalesce();

    // The whole drag is exactly one undo step; the scene is at the final offset.
    assert_eq!(stack.undo_depth(), 1, "a coalesced drag is one undo step");
    let final_transform = scene.get("r").unwrap().transform;
    assert_eq!(final_transform, Transform3x3::translate(50.0, 0.0));

    // Undo lands all the way back at the pre-gesture (identity) state.
    let undo_op = stack.undo().expect("undo available");
    let re_inverse = apply_object_op(&mut scene, undo_op).expect("apply undo");
    stack.note_undo_applied(re_inverse);
    assert_eq!(scene.get("r").unwrap().transform, Transform3x3::IDENTITY);
    assert!(!stack.can_undo());
    assert!(stack.can_redo());

    // Redo replays the gesture's final state.
    let redo_op = stack.redo().expect("redo available");
    let inv = apply_object_op(&mut scene, redo_op).expect("apply redo");
    stack.note_redo_applied(inv);
    assert_eq!(scene.get("r").unwrap().transform, final_transform);
    assert_eq!(stack.undo_depth(), 1);
    assert!(!stack.can_redo());
}

#[test]
fn ob53_per_actor_undo_stacks_are_independent() {
    // Two actors edit the same scene; each actor's UndoStack is client-local and
    // independent — one actor's undo never disturbs the other's stack.
    let mut scene = ObjectScene::default();
    apply_object_op(
        &mut scene,
        ObjectOp::InsertObject { object: Object::new("r", "a0", rect(0, 0, 80, 40)) },
    )
    .expect("insert");

    let mut alice = UndoStack::new("alice".into());
    let mut bob = UndoStack::new("bob".into());

    let a_fwd = ObjectOp::SetTransform { id: "r".into(), transform: Transform3x3::translate(10.0, 0.0) };
    let a_inv = apply_object_op(&mut scene, a_fwd.clone()).expect("alice edit");
    alice.record(a_fwd, a_inv);

    let b_fwd = ObjectOp::SetTransform { id: "r".into(), transform: Transform3x3::translate(10.0, 20.0) };
    let b_inv = apply_object_op(&mut scene, b_fwd.clone()).expect("bob edit");
    bob.record(b_fwd, b_inv);

    assert_eq!(alice.undo_depth(), 1);
    assert_eq!(bob.undo_depth(), 1);

    // Alice undoes; only her stack drains, bob's is untouched.
    let undo_op = alice.undo().expect("alice undo");
    let ri = apply_object_op(&mut scene, undo_op).expect("apply alice undo");
    alice.note_undo_applied(ri);
    assert_eq!(alice.undo_depth(), 0);
    assert!(alice.can_redo());
    assert_eq!(bob.undo_depth(), 1, "bob's stack is unaffected by alice's undo");
    assert!(!bob.can_redo());
}

#[test]
fn ob53_hit_test_point_in_polygon_axis_aligned() {
    // Region derivation + point-in-polygon on an axis-aligned rect outline.
    let deriver = StubOutlineDeriver;
    let g = rect(0, 0, 80, 40);
    let region = deriver.derive_region(&g, 1).expect("derive region");
    assert!(region.closed);

    // Inside hits; outside misses (via the trait default + the raw fn).
    assert!(deriver.contains(&region, LocalPoint { x: 40, y: 20 }));
    assert!(!deriver.contains(&region, LocalPoint { x: 200, y: 200 }));
    assert!(point_in_polygon(&region.outline, LocalPoint { x: 1, y: 1 }));
    assert!(!point_in_polygon(&region.outline, LocalPoint { x: -1, y: 20 }));
}

#[test]
fn ob53_hit_test_point_in_polygon_rotated_outline() {
    // A manually-rotated outline: a diamond (a 45-degree square) whose vertices are
    // not axis-aligned. point_in_polygon must classify center-in / corner-gap-out.
    let diamond = vec![
        LocalPoint { x: 50, y: 0 },
        LocalPoint { x: 100, y: 50 },
        LocalPoint { x: 50, y: 100 },
        LocalPoint { x: 0, y: 50 },
    ];
    // The centroid is inside.
    assert!(point_in_polygon(&diamond, LocalPoint { x: 50, y: 50 }));
    // A point in the AABB but outside the diamond (near a clipped corner) misses.
    assert!(!point_in_polygon(&diamond, LocalPoint { x: 5, y: 5 }));
    assert!(!point_in_polygon(&diamond, LocalPoint { x: 95, y: 95 }));
}
