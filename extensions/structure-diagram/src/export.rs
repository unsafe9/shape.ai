//! The export seam: lower a [`Diagram`] to scene-core [`ObjectOp`]s so the graph
//! RIDES ON the one core canvas. Emits ONLY ops — no second apply.
//!
//! Each node lowers to a rect [`Object`] at its `(x, y)` transform with
//! `text = label`. Each edge lowers to an open 2-node connector object whose
//! anchors are `[{node_index:0, target:from, at:center}, {node_index:1, target:to,
//! at:center}]` — IDENTICAL to `templates.rs::connector`. The edges then track
//! their endpoints via `resolve_endpoint`/`reproject_object_anchors` with zero
//! stored geometry, and `connection_graph`/`neighbors`/the existing mermaid export
//! work for free over the result. So a non-layout, relational domain maps onto the
//! core scene with no new edge machinery.
//!
//! Pure: ids/order from the injected [`IdOrderAlloc`]; the SAME domain key always
//! yields the SAME object id, so re-export reuses ids (zero-rebake / incremental).

use serde_json::Value;
use shape_extension_contract::{tag_meta, IdOrderAlloc, ObjectMeta, META_MODEL_KEY};
use shape_scene_core::object::{
    semantic_preset_style, Anchor, Fill, FillRule, Geometry, LineCap, LineJoin, LocalPoint, Object,
    ObjectOp, Paint, PathNode, Stroke, SubPath, Text, TextAlign, TextRun, TextVAlign, Transform3x3,
    GEOMETRY_QUANTUM_PER_PX,
};

use crate::model::{Diagram, Edge, Node};
use crate::NAME;

/// Node box size (logical px). The connector `at` points address each node's local
/// center, so they re-project as the node geometry edits.
const NODE_W: i32 = 160;
const NODE_H: i32 = 72;

const fn px(p: i32) -> i32 {
    p * GEOMETRY_QUANTUM_PER_PX
}

fn rect_geometry(w_px: i32, h_px: i32) -> Geometry {
    let (w, h) = (px(w_px), px(h_px));
    Geometry::from_subpaths(
        vec![SubPath {
            closed: true,
            nodes: vec![
                PathNode::corner(0, 0),
                PathNode::corner(w, 0),
                PathNode::corner(w, h),
                PathNode::corner(0, h),
            ],
        }],
        FillRule::EvenOdd,
    )
}

/// An open 2-node connector body. The world endpoints come from the anchors, not
/// this geometry; the short stub keeps the object non-degenerate before resolution.
/// Identical shape to `templates.rs::segment_geometry`.
fn segment_geometry() -> Geometry {
    Geometry::from_subpaths(
        vec![SubPath {
            closed: false,
            nodes: vec![PathNode::corner(0, 0), PathNode::corner(px(40), 0)],
        }],
        FillRule::NonZero,
    )
}

fn label(text: &str, color: Option<String>) -> Text {
    Text {
        runs: vec![TextRun {
            text: text.to_string(),
            color,
            size: Some(px(13)),
            bold: false,
            italic: false,
            font: None,
        }],
        align: TextAlign::Center,
        valign: TextVAlign::Middle,
    }
}

/// Center of a node box in its own object-local quantized coords (the anchor `at`).
fn node_center() -> LocalPoint {
    LocalPoint { x: px(NODE_W) / 2, y: px(NODE_H) / 2 }
}

fn meta_for(domain_key: &str) -> Option<ObjectMeta> {
    let mut meta = ObjectMeta::new();
    tag_meta(&mut meta, NAME, domain_key);
    Some(meta)
}

fn root_key() -> String {
    "root".to_string()
}
fn node_key(node: &Node) -> String {
    format!("node:{}", node.id)
}
fn edge_key(edge: &Edge) -> String {
    format!("edge:{}", edge.id)
}

/// Lower the whole diagram. Emits, NODES BEFORE EDGES (so a connector's anchor
/// targets exist when its insert validates):
/// 1. the root object (carries the model blob + `meta[ext]`),
/// 2. each node rect at its (x,y),
/// 3. each edge as an open 2-node connector anchored to its from/to node centers.
pub fn export_diagram(
    diagram: &Diagram,
    model_json: &Value,
    alloc: &mut IdOrderAlloc,
) -> Result<Vec<ObjectOp>, String> {
    let mut ops = Vec::new();

    // 1. Root: an invisible model carrier (a 1px box keeps it valid), tagged + with
    //    the model blob, so the whole extension state rides one object's meta.
    let root_id = alloc.id(&root_key());
    let mut root = Object::new(root_id, alloc.order(), rect_geometry(1, 1));
    root.transform = Transform3x3::translate(0.0, 0.0);
    root.hidden = true;
    root.name = Some("Structure diagram".to_string());
    let mut root_meta = ObjectMeta::new();
    tag_meta(&mut root_meta, NAME, &root_key());
    root_meta.insert(META_MODEL_KEY.to_string(), model_json.clone());
    root.meta = Some(root_meta);
    ops.push(ObjectOp::InsertObject { object: root });

    // 2. Nodes: rect objects at their (x,y) world placement, text = label.
    let (fill, stroke, text_color): (Option<Fill>, Option<Stroke>, Option<String>) =
        semantic_preset_style("decision");
    for node in &diagram.nodes {
        let id = alloc.id(&node_key(node));
        let mut rect = Object::new(id, alloc.order(), rect_geometry(NODE_W, NODE_H));
        rect.transform = Transform3x3::translate(node.x, node.y);
        rect.fill = fill.clone();
        rect.stroke = stroke.clone();
        rect.text = Some(label(&node.label, text_color.clone()));
        rect.name = Some(node.label.clone());
        rect.meta = meta_for(&node_key(node));
        ops.push(ObjectOp::InsertObject { object: rect });
    }

    // 3. Edges: open 2-node connectors anchored to from/to node centers. The
    //    anchors carry the endpoint association; geometry is just a non-degenerate
    //    stub. Identical to templates.rs::connector, so resolve_endpoint/reproject
    //    track the endpoints and connection_graph sees the edge for free.
    let at = node_center();
    for edge in &diagram.edges {
        let from_id = alloc.id(&format!("node:{}", edge.from));
        let to_id = alloc.id(&format!("node:{}", edge.to));
        let id = alloc.id(&edge_key(edge));
        let mut conn = Object::new(id, alloc.order(), segment_geometry());
        conn.stroke = Some(Stroke {
            paint: Paint::Solid { color: "#7b8794".to_string() },
            width: px(2),
            opacity: 1.0,
            dash: Vec::new(),
            cap: LineCap::Round,
            join: LineJoin::Round,
        });
        conn.anchors = vec![
            Anchor { node_index: 0, target: from_id, at },
            Anchor { node_index: 1, target: to_id, at },
        ];
        if let Some(text) = &edge.label {
            conn.text = Some(label(text, None));
        }
        conn.meta = meta_for(&edge_key(edge));
        ops.push(ObjectOp::InsertObject { object: conn });
    }

    Ok(ops)
}
