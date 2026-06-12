//! The object-native MCP toolset: the AI-readable re-representation layer for the
//! object model. There is no `kind`/type stored on objects, so the "kind" labels
//! here are descriptive projections derived from geometry/anchors/text.
//!
//! Everything is a pure function over `&ObjectScene` / `&mut ObjectScene`
//! returning JSON-serializable results and/or [`ObjectOp`]s. Mutating tools
//! return ops rather than touching the scene, so the caller funnels them through
//! the single op-apply path where time/ids/seq are injected; reads reply
//! directly. This keeps the toolset free of time/rng/IO.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use shape_scene_core::object::{
    anchors, connection_graph, semantic_preset_style, Comment, CommentAnchor, FillRule, Geometry,
    Object, ObjectId, ObjectScene, ObjectSelection, OutlineDeriver, PathNode, StubOutlineDeriver,
    SubPath, Text, TextAlign, TextRun, TextVAlign, Transform3x3, GEOMETRY_QUANTUM_PER_PX,
};
use shape_scene_core::object::op::{FieldEdit, ObjectOp};

/// Curve-flattening tolerance for the bounds derivation (the stub deriver ignores
/// it; finest bucket so reported bounds match the true outline).
const FLATNESS: i32 = 1;

/// A descriptive label for what an object looks like, derived from
/// geometry/anchors/text and never persisted. Precedence:
/// - >= 2 anchor targets => `connector` (even with text);
/// - only text, no closed contour => `text`;
/// - one closed subpath => `shape`; an open contour => `stroke`.
pub fn kind_label(object: &Object) -> &'static str {
    let distinct_targets = {
        let mut t: Vec<&ObjectId> = Vec::new();
        for a in &object.anchors {
            if a.target != object.id && !t.contains(&&a.target) {
                t.push(&a.target);
            }
        }
        t.len()
    };
    if distinct_targets >= 2 {
        return "connector";
    }

    let subpaths = &object.geometry.subpaths;
    let closed_subpaths = subpaths.iter().filter(|s| s.closed).count();
    let has_open = subpaths.iter().any(|s| !s.closed);
    let has_text = object
        .text
        .as_ref()
        .is_some_and(|t| t.runs.iter().any(|r| !r.text.trim().is_empty()));

    if has_text && closed_subpaths == 0 {
        return "text";
    }
    if closed_subpaths == 1 && !has_open {
        return "shape";
    }
    if has_open {
        // A single anchor target still reads as a connector; otherwise a free stroke.
        if distinct_targets == 1 {
            return "connector";
        }
        return "stroke";
    }
    if closed_subpaths > 1 {
        return "shape";
    }
    if has_text {
        return "text";
    }
    "shape"
}

/// World-axis-aligned bounds in logical px, or `None` for degenerate geometry.
fn object_bounds_px(object: &Object) -> Option<Bounds> {
    let deriver = StubOutlineDeriver;
    let region = deriver.derive_region(&object.geometry, FLATNESS).ok()?;
    let q = f64::from(GEOMETRY_QUANTUM_PER_PX);
    // The transform can rotate/shear, so map the four corners then re-AABB.
    let corners = [
        (region.bounds.min_x, region.bounds.min_y),
        (region.bounds.max_x, region.bounds.min_y),
        (region.bounds.max_x, region.bounds.max_y),
        (region.bounds.min_x, region.bounds.max_y),
    ];
    let mut it = corners.iter().map(|&(x, y)| {
        object.transform.apply_point(f64::from(x) / q, f64::from(y) / q)
    });
    let (mut min_x, mut min_y) = it.next()?;
    let (mut max_x, mut max_y) = (min_x, min_y);
    for (wx, wy) in it {
        min_x = min_x.min(wx);
        min_y = min_y.min(wy);
        max_x = max_x.max(wx);
        max_y = max_y.max(wy);
    }
    Some(Bounds { x: min_x, y: min_y, width: max_x - min_x, height: max_y - min_y })
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// One row of [`list_objects`]. `kind` is descriptive only (see [`kind_label`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectSummary {
    pub id: ObjectId,
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bounds: Option<Bounds>,
    pub tags: Vec<String>,
}

/// In scene order.
pub fn list_objects(scene: &ObjectScene) -> Vec<ObjectSummary> {
    scene
        .objects
        .iter()
        .map(|o| ObjectSummary {
            id: o.id.clone(),
            kind: kind_label(o),
            bounds: object_bounds_px(o),
            tags: o.tags.clone(),
        })
        .collect()
}

/// The whole [`Object`], `None` when absent.
pub fn get_object(scene: &ObjectScene, id: &str) -> Option<Object> {
    scene.get(id).cloned()
}

/// The minimal create spec an agent supplies, expressed at human scale (logical
/// px) and lowered to the object-local quantized substrate.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateObjectSpec {
    /// Caller-allocated (ids never come from rng in the pure layer).
    pub id: ObjectId,
    /// Fractional z-order key; sorts by plain `str` Ord.
    pub order: String,
    /// `rect` or `text`. Connectors are authored via anchors, not this spec.
    pub shape: CreateShape,
    /// World placement in logical px (the transform translation).
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    /// Integer px so geometry coords never come from an f64 narrowing.
    #[serde(default)]
    pub width: Option<i32>,
    #[serde(default)]
    pub height: Option<i32>,
    #[serde(default)]
    pub text: Option<String>,
    /// A named semantic preset (`decision`, `risk`, `task`, ...).
    #[serde(default)]
    pub style: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CreateShape {
    Rect,
    Text,
}

/// Logical px (integer) -> object-local quantized units. Integer multiply stays
/// exact (no f64 narrowing); geometry coords stay quantized i32.
const fn px(p: i32) -> i32 {
    p * GEOMETRY_QUANTUM_PER_PX
}

/// A closed rectangle from (0,0) to (w,h) in object-local quantized units.
fn rect_geometry(w_q: i32, h_q: i32) -> Geometry {
    Geometry::from_subpaths(
        vec![SubPath {
            closed: true,
            nodes: vec![
                PathNode::corner(0, 0),
                PathNode::corner(w_q, 0),
                PathNode::corner(w_q, h_q),
                PathNode::corner(0, h_q),
            ],
        }],
        FillRule::EvenOdd,
    )
}

/// Build the [`ObjectOp::InsertObject`] for a create spec. `Err` on an invalid
/// spec (a `text`/`rect` box needs a positive size).
pub fn create_object(spec: CreateObjectSpec) -> Result<ObjectOp, String> {
    let (fill, stroke, text_color) = match &spec.style {
        Some(preset) => semantic_preset_style(preset),
        None => (None, None, None),
    };

    let (geometry, text) = match spec.shape {
        CreateShape::Rect => {
            let w = spec.width.unwrap_or(160);
            let h = spec.height.unwrap_or(100);
            if w <= 0 || h <= 0 {
                return Err("rect width/height must be positive".to_string());
            }
            let text = spec
                .text
                .as_deref()
                .filter(|t| !t.trim().is_empty())
                .map(|t| label(t, text_color.clone()));
            (rect_geometry(px(w), px(h)), text)
        }
        CreateShape::Text => {
            let body = spec
                .text
                .as_deref()
                .filter(|t| !t.trim().is_empty())
                .ok_or("text object requires a non-empty text")?;
            // A text object still needs a geometry box to lay out / hit-test in.
            let w = spec.width.unwrap_or(200);
            let h = spec.height.unwrap_or(40);
            if w <= 0 || h <= 0 {
                return Err("text box width/height must be positive".to_string());
            }
            (rect_geometry(px(w), px(h)), Some(label(body, text_color.clone())))
        }
    };

    let mut object = Object::new(spec.id, spec.order, geometry);
    object.transform = Transform3x3::translate(spec.x, spec.y);
    object.fill = fill;
    object.stroke = stroke;
    object.text = text;
    object.tags = spec.tags;
    Ok(ObjectOp::InsertObject { object })
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

/// A declarative patch: each present field becomes its own [`ObjectOp`]; absent
/// fields are untouched.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchObjectSpec {
    pub id: ObjectId,
    /// `Some("")` clears text.
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub style: Option<String>,
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    /// Integer logical px; replaces the rect geometry.
    #[serde(default)]
    pub width: Option<i32>,
    #[serde(default)]
    pub height: Option<i32>,
}

/// Lower a patch into ops in deterministic order: geometry, transform, style,
/// text. `Err` on an inconsistent patch (e.g. only one of width/height).
pub fn patch_object(spec: PatchObjectSpec) -> Result<Vec<ObjectOp>, String> {
    let mut ops: Vec<ObjectOp> = Vec::new();

    match (spec.width, spec.height) {
        (Some(w), Some(h)) => {
            if w <= 0 || h <= 0 {
                return Err("width/height must be positive".to_string());
            }
            ops.push(ObjectOp::EditGeometry {
                id: spec.id.clone(),
                geometry: rect_geometry(px(w), px(h)),
            });
        }
        (None, None) => {}
        _ => return Err("width and height must be set together".to_string()),
    }

    if let (Some(x), Some(y)) = (spec.x, spec.y) {
        ops.push(ObjectOp::SetTransform {
            id: spec.id.clone(),
            transform: Transform3x3::translate(x, y),
        });
    } else if spec.x.is_some() || spec.y.is_some() {
        return Err("x and y must be set together".to_string());
    }

    if let Some(preset) = &spec.style {
        let (fill, stroke, _text_color) = semantic_preset_style(preset);
        ops.push(ObjectOp::SetStyle {
            id: spec.id.clone(),
            fill: Some(FieldEdit::from_option(fill)),
            stroke: Some(FieldEdit::from_option(stroke)),
        });
    }

    if let Some(text) = &spec.text {
        let new_text = if text.trim().is_empty() {
            None
        } else {
            Some(label(text, None))
        };
        ops.push(ObjectOp::SetText { id: spec.id.clone(), text: new_text });
    }

    if ops.is_empty() {
        return Err("patch is empty (no fields set)".to_string());
    }
    Ok(ops)
}

/// Full replace of an object's tag id set ([`ObjectOp::SetTags`]).
pub fn tag_object(id: &str, tags: Vec<String>) -> ObjectOp {
    ObjectOp::SetTags { id: id.to_string(), tags }
}

/// Comment id/author are caller-supplied (no rng); `node_index` anchors it to a
/// geometry node when present.
pub fn add_comment(
    id: &str,
    comment_id: &str,
    author: &str,
    body: &str,
    node_index: Option<i32>,
) -> ObjectOp {
    ObjectOp::AddComment {
        id: id.to_string(),
        comment: Comment {
            id: comment_id.to_string(),
            author: author.to_string(),
            body: body.to_string(),
            at: node_index.map(|node_index| CommentAnchor::Node { node_index }),
            resolved: false,
        },
    }
}

/// Selection lives on [`ObjectScene`] itself, not as an [`ObjectOp`].
pub fn set_selection(scene: &mut ObjectScene, selection: ObjectSelection) {
    scene.selection = selection;
}

/// The three predicates are ANDed; an empty filter matches every object.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryFilter {
    /// Match objects carrying every listed tag id.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Match anchor-graph neighbors of this object id (per [`anchors::neighbors`]).
    #[serde(default)]
    pub connected_to: Option<ObjectId>,
    /// Match objects whose world bounds intersect this region (logical px);
    /// underivable bounds never match.
    #[serde(default)]
    pub region: Option<Bounds>,
}

/// Matching ids in scene order; the predicates are ANDed.
pub fn query(scene: &ObjectScene, filter: &QueryFilter) -> Vec<ObjectId> {
    let connected: Option<Vec<ObjectId>> = filter
        .connected_to
        .as_deref()
        .map(|id| anchors::neighbors(scene, id));

    scene
        .objects
        .iter()
        .filter(|o| {
            if !filter.tags.is_empty() && !filter.tags.iter().all(|t| o.tags.contains(t)) {
                return false;
            }
            if let Some(neighbors) = &connected {
                if !neighbors.contains(&o.id) {
                    return false;
                }
            }
            if let Some(region) = &filter.region {
                match object_bounds_px(o) {
                    Some(b) if bounds_intersect(&b, region) => {}
                    _ => return false,
                }
            }
            true
        })
        .map(|o| o.id.clone())
        .collect()
}

/// Axis-aligned overlap test (touching edges count as intersecting).
fn bounds_intersect(a: &Bounds, b: &Bounds) -> bool {
    a.x <= b.x + b.width
        && b.x <= a.x + a.width
        && a.y <= b.y + b.height
        && b.y <= a.y + a.height
}

/// Text digest of (a scope of) the scene. Empty `scope_ids` => the whole scene;
/// otherwise only those objects and the edges entirely within scope. `mermaid` is
/// a flowchart, `digest` a node/edge listing; unknown types fall back to `digest`.
pub fn export(scene: &ObjectScene, scope_ids: &[String], export_type: &str) -> String {
    let in_scope = |id: &str| scope_ids.is_empty() || scope_ids.iter().any(|s| s == id);

    let nodes: Vec<&Object> = scene.objects.iter().filter(|o| in_scope(&o.id)).collect();
    let edges: Vec<(ObjectId, ObjectId)> = connection_graph(scene)
        .into_iter()
        .filter(|(a, b)| in_scope(a) && in_scope(b))
        .collect();

    match export_type {
        "mermaid" => render_mermaid(&nodes, &edges),
        _ => render_digest(&nodes, &edges),
    }
}

/// First non-empty text run, else the descriptive kind + id.
fn display_label(object: &Object) -> String {
    let text = object
        .text
        .as_ref()
        .and_then(|t| t.runs.iter().map(|r| r.text.trim()).find(|t| !t.is_empty()));
    match text {
        Some(t) => t.to_string(),
        None => format!("{} {}", kind_label(object), object.id),
    }
}

fn render_mermaid(nodes: &[&Object], edges: &[(ObjectId, ObjectId)]) -> String {
    let mut out = String::from("flowchart LR\n");
    for n in nodes {
        out.push_str(&format!(
            "  {}[\"{}\"]\n",
            mermaid_id(&n.id),
            escape_mermaid(&display_label(n))
        ));
    }
    for (a, b) in edges {
        out.push_str(&format!("  {} --> {}\n", mermaid_id(a), mermaid_id(b)));
    }
    out
}

fn render_digest(nodes: &[&Object], edges: &[(ObjectId, ObjectId)]) -> String {
    let mut out = String::new();
    out.push_str("Objects:\n");
    for n in nodes {
        out.push_str(&format!("- {} ({}): {}\n", n.id, kind_label(n), display_label(n)));
    }
    out.push_str("Edges:\n");
    if edges.is_empty() {
        out.push_str("- (none)\n");
    } else {
        for (a, b) in edges {
            out.push_str(&format!("- {a} -> {b}\n"));
        }
    }
    out
}

fn mermaid_id(id: &str) -> String {
    let mut out = String::new();
    for ch in id.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        out.push('n');
    }
    out
}

fn escape_mermaid(s: &str) -> String {
    s.replace('"', "'")
}

/// Self-describing metadata for one object-MCP tool.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolMeta {
    pub name: &'static str,
    pub description: &'static str,
    /// A light input schema, not a full JSON-Schema document.
    pub schema: Value,
}

/// The full tool catalog, in advertised order.
pub fn object_mcp_tools() -> Vec<McpToolMeta> {
    vec![
        McpToolMeta {
            name: "list_objects",
            description:
                "List every object as a summary: id, a descriptive kind label (shape/stroke/connector/text — derived, not stored), world bounds, and tags.",
            schema: json!({ "type": "object", "properties": {} }),
        },
        McpToolMeta {
            name: "get_object",
            description: "Read one full object by id (geometry, style, text, anchors, comments, tags).",
            schema: json!({
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"],
            }),
        },
        McpToolMeta {
            name: "create_object",
            description:
                "Create an object from a simple spec (rect/text) with an optional named semantic style; lowers to one insert-object op.",
            schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "order": { "type": "string" },
                    "shape": { "enum": ["rect", "text"] },
                    "x": { "type": "number" },
                    "y": { "type": "number" },
                    "width": { "type": "number" },
                    "height": { "type": "number" },
                    "text": { "type": "string" },
                    "style": { "type": "string" },
                    "tags": { "type": "array", "items": { "type": "string" } },
                },
                "required": ["id", "order", "shape"],
            }),
        },
        McpToolMeta {
            name: "patch_object",
            description:
                "Patch an object from a JSON patch: set text, named style, world position, and/or rect size; lowers to a sequence of object ops.",
            schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "text": { "type": "string" },
                    "style": { "type": "string" },
                    "x": { "type": "number" },
                    "y": { "type": "number" },
                    "width": { "type": "number" },
                    "height": { "type": "number" },
                },
                "required": ["id"],
            }),
        },
        McpToolMeta {
            name: "tag_object",
            description: "Replace the set of tag ids attached to one object (set-tags op).",
            schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "tags": { "type": "array", "items": { "type": "string" } },
                },
                "required": ["id", "tags"],
            }),
        },
        McpToolMeta {
            name: "add_comment",
            description: "Append a comment to an object, optionally anchored to a geometry node (add-comment op).",
            schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "commentId": { "type": "string" },
                    "author": { "type": "string" },
                    "body": { "type": "string" },
                    "nodeIndex": { "type": "integer" },
                },
                "required": ["id", "commentId", "author", "body"],
            }),
        },
        McpToolMeta {
            name: "set_selection",
            description: "Set the persisted scene selection (canvas / a single object / a multi set).",
            schema: json!({
                "type": "object",
                "properties": {
                    "kind": { "enum": ["canvas", "object", "multi"] },
                    "id": { "type": "string" },
                    "ids": { "type": "array", "items": { "type": "string" } },
                },
                "required": ["kind"],
            }),
        },
        McpToolMeta {
            name: "query",
            description:
                "Find object ids by tag, by connection (anchor connection-graph neighbors of an object), and/or by world region. Predicates are ANDed.",
            schema: json!({
                "type": "object",
                "properties": {
                    "tags": { "type": "array", "items": { "type": "string" } },
                    "connectedTo": { "type": "string" },
                    "region": {
                        "type": "object",
                        "properties": {
                            "x": { "type": "number" },
                            "y": { "type": "number" },
                            "width": { "type": "number" },
                            "height": { "type": "number" },
                        },
                    },
                },
            }),
        },
        McpToolMeta {
            name: "export",
            description:
                "Render a scope of the scene as an AI-readable digest of its connection graph (mermaid flowchart or plain-text node/edge listing).",
            schema: json!({
                "type": "object",
                "properties": {
                    "scopeIds": { "type": "array", "items": { "type": "string" } },
                    "exportType": { "enum": ["mermaid", "digest"] },
                },
            }),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_scene_core::object::{apply_object_op, Anchor, LocalPoint};

    fn apply(scene: &mut ObjectScene, op: ObjectOp) {
        apply_object_op(scene, op).expect("op applies");
    }

    /// Open 2-node geometry anchored to two targets.
    fn connector(id: &str, order: &str, a: &str, b: &str) -> Object {
        let mut obj = Object::new(
            id,
            order,
            Geometry::from_subpaths(
                vec![SubPath {
                    closed: false,
                    nodes: vec![PathNode::corner(0, 0), PathNode::corner(320, 0)],
                }],
                FillRule::NonZero,
            ),
        );
        obj.anchors = vec![
            Anchor { node_index: 0, target: a.to_string(), at: LocalPoint { x: 0, y: 0 } },
            Anchor { node_index: 1, target: b.to_string(), at: LocalPoint { x: 0, y: 0 } },
        ];
        obj
    }

    #[test]
    fn create_object_lowers_to_insert_and_applies() {
        let mut scene = ObjectScene::default();
        let op = create_object(CreateObjectSpec {
            id: "rect-1".into(),
            order: "a0".into(),
            shape: CreateShape::Rect,
            x: 100.0,
            y: 50.0,
            width: Some(160),
            height: Some(100),
            text: Some("Hello".into()),
            style: Some("decision".into()),
            tags: vec!["t-blue".into()],
        })
        .expect("spec is valid");

        match &op {
            ObjectOp::InsertObject { object } => {
                assert_eq!(object.id, "rect-1");
                assert!(object.fill.is_some());
                assert!(object.stroke.is_some());
                assert!(object.text.is_some());
                // World placement is the transform translation, not geometry.
                assert_eq!(object.transform.m[0][2], 100.0);
                assert_eq!(object.transform.m[1][2], 50.0);
            }
            other => panic!("expected insert-object, got {}", other.kind()),
        }

        apply(&mut scene, op);
        assert_eq!(scene.objects.len(), 1);
        let summary = &list_objects(&scene)[0];
        assert_eq!(summary.id, "rect-1");
        assert_eq!(summary.tags, vec!["t-blue".to_string()]);
        let b = summary.bounds.expect("rect has bounds");
        assert!((b.x - 100.0).abs() < 1e-6);
        assert!((b.y - 50.0).abs() < 1e-6);
        assert!((b.width - 160.0).abs() < 1e-6);
        assert!((b.height - 100.0).abs() < 1e-6);
    }

    #[test]
    fn list_objects_labels_a_rect_as_shape_and_a_connector_as_connector() {
        let mut scene = ObjectScene::default();
        apply(
            &mut scene,
            create_object(CreateObjectSpec {
                id: "a".into(),
                order: "a0".into(),
                shape: CreateShape::Rect,
                x: 0.0,
                y: 0.0,
                width: Some(80),
                height: Some(40),
                text: None,
                style: None,
                tags: vec![],
            })
            .unwrap(),
        );
        apply(
            &mut scene,
            create_object(CreateObjectSpec {
                id: "b".into(),
                order: "a1".into(),
                shape: CreateShape::Rect,
                x: 300.0,
                y: 0.0,
                width: Some(80),
                height: Some(40),
                text: None,
                style: None,
                tags: vec![],
            })
            .unwrap(),
        );
        apply(&mut scene, ObjectOp::InsertObject { object: connector("edge", "a2", "a", "b") });

        let summaries = list_objects(&scene);
        let kind_of = |id: &str| summaries.iter().find(|s| s.id == id).unwrap().kind;
        assert_eq!(kind_of("a"), "shape");
        assert_eq!(kind_of("b"), "shape");
        assert_eq!(kind_of("edge"), "connector");
    }

    #[test]
    fn list_objects_labels_a_text_node_as_text() {
        let mut scene = ObjectScene::default();
        apply(
            &mut scene,
            create_object(CreateObjectSpec {
                id: "label-1".into(),
                order: "a0".into(),
                shape: CreateShape::Text,
                x: 0.0,
                y: 0.0,
                width: None,
                height: None,
                text: Some("A note".into()),
                style: None,
                tags: vec![],
            })
            .unwrap(),
        );
        // A pure "text" label needs no closed contour, so verify the open-contour
        // path directly (the create_object box is closed and reads as a shape).
        let mut text_only = Object::new(
            "t",
            "a1",
            Geometry::from_subpaths(
                vec![SubPath {
                    closed: false,
                    nodes: vec![PathNode::corner(0, 0), PathNode::corner(10, 0)],
                }],
                FillRule::NonZero,
            ),
        );
        text_only.text = Some(label("hi", None));
        text_only.geometry.ensure_parsed().unwrap();
        assert_eq!(kind_label(&text_only), "text");
    }

    #[test]
    fn query_by_tag_filters_objects() {
        let mut scene = ObjectScene::default();
        apply(&mut scene, tag_then_insert("a", "a0", &["keep"]));
        apply(&mut scene, tag_then_insert("b", "a1", &["other"]));
        apply(&mut scene, tag_then_insert("c", "a2", &["keep", "extra"]));

        let ids = query(&scene, &QueryFilter { tags: vec!["keep".into()], ..Default::default() });
        assert_eq!(ids, vec!["a".to_string(), "c".to_string()]);
    }

    #[test]
    fn query_by_connection_returns_neighbors() {
        let mut scene = ObjectScene::default();
        apply(&mut scene, tag_then_insert("a", "a0", &[]));
        apply(&mut scene, tag_then_insert("b", "a1", &[]));
        apply(&mut scene, ObjectOp::InsertObject { object: connector("edge", "a2", "a", "b") });
        let ids = query(&scene, &QueryFilter { connected_to: Some("a".into()), ..Default::default() });
        assert_eq!(ids, vec!["b".to_string()]);
    }

    #[test]
    fn query_by_region_intersects_bounds() {
        let mut scene = ObjectScene::default();
        apply(
            &mut scene,
            create_object(CreateObjectSpec {
                id: "near".into(),
                order: "a0".into(),
                shape: CreateShape::Rect,
                x: 0.0,
                y: 0.0,
                width: Some(50),
                height: Some(50),
                text: None,
                style: None,
                tags: vec![],
            })
            .unwrap(),
        );
        apply(
            &mut scene,
            create_object(CreateObjectSpec {
                id: "far".into(),
                order: "a1".into(),
                shape: CreateShape::Rect,
                x: 1000.0,
                y: 1000.0,
                width: Some(50),
                height: Some(50),
                text: None,
                style: None,
                tags: vec![],
            })
            .unwrap(),
        );
        let region = Bounds { x: -10.0, y: -10.0, width: 100.0, height: 100.0 };
        let ids = query(&scene, &QueryFilter { region: Some(region), ..Default::default() });
        assert_eq!(ids, vec!["near".to_string()]);
    }

    #[test]
    fn export_of_two_connected_objects_mentions_both() {
        let mut scene = ObjectScene::default();
        apply(
            &mut scene,
            create_object(CreateObjectSpec {
                id: "src".into(),
                order: "a0".into(),
                shape: CreateShape::Rect,
                x: 0.0,
                y: 0.0,
                width: Some(80),
                height: Some(40),
                text: Some("Source".into()),
                style: None,
                tags: vec![],
            })
            .unwrap(),
        );
        apply(
            &mut scene,
            create_object(CreateObjectSpec {
                id: "dst".into(),
                order: "a1".into(),
                shape: CreateShape::Rect,
                x: 300.0,
                y: 0.0,
                width: Some(80),
                height: Some(40),
                text: Some("Dest".into()),
                style: None,
                tags: vec![],
            })
            .unwrap(),
        );
        apply(&mut scene, ObjectOp::InsertObject { object: connector("edge", "a2", "src", "dst") });

        let digest = export(&scene, &[], "digest");
        assert!(digest.contains("Source"), "digest mentions source label: {digest}");
        assert!(digest.contains("Dest"), "digest mentions dest label: {digest}");
        assert!(digest.contains("src -> dst"), "digest has the edge: {digest}");

        let mermaid = export(&scene, &[], "mermaid");
        assert!(mermaid.starts_with("flowchart LR"));
        assert!(mermaid.contains("src --> dst"));
        assert!(mermaid.contains("Source") && mermaid.contains("Dest"));
        assert_eq!(export(&scene, &[], "nope"), digest);
    }

    #[test]
    fn export_scope_limits_to_listed_objects() {
        let mut scene = ObjectScene::default();
        apply(&mut scene, tag_then_insert("a", "a0", &[]));
        apply(&mut scene, tag_then_insert("b", "a1", &[]));
        let digest = export(&scene, &["a".to_string()], "digest");
        assert!(digest.contains("- a "));
        assert!(!digest.contains("- b "));
    }

    #[test]
    fn patch_object_lowers_to_ordered_ops() {
        let ops = patch_object(PatchObjectSpec {
            id: "r".into(),
            text: Some("New".into()),
            style: Some("risk".into()),
            x: Some(5.0),
            y: Some(6.0),
            width: Some(120),
            height: Some(60),
        })
        .expect("valid patch");
        let kinds: Vec<&str> = ops.iter().map(|o| o.kind()).collect();
        assert_eq!(kinds, vec!["edit-geometry", "set-transform", "set-style", "set-text"]);
    }

    #[test]
    fn patch_object_rejects_partial_box_and_empty_patch() {
        assert!(patch_object(PatchObjectSpec {
            id: "r".into(),
            width: Some(10),
            ..Default::default()
        })
        .is_err());
        assert!(patch_object(PatchObjectSpec { id: "r".into(), ..Default::default() }).is_err());
    }

    #[test]
    fn patch_object_clears_text_with_empty_string() {
        let ops = patch_object(PatchObjectSpec {
            id: "r".into(),
            text: Some("   ".into()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(ops.len(), 1);
        match &ops[0] {
            ObjectOp::SetText { text, .. } => assert!(text.is_none(), "blank text clears"),
            other => panic!("expected set-text, got {}", other.kind()),
        }
    }

    #[test]
    fn tag_add_comment_and_selection_produce_expected_ops() {
        match tag_object("x", vec!["a".into(), "b".into()]) {
            ObjectOp::SetTags { id, tags } => {
                assert_eq!(id, "x");
                assert_eq!(tags, vec!["a".to_string(), "b".to_string()]);
            }
            other => panic!("expected set-tags, got {}", other.kind()),
        }

        match add_comment("x", "c1", "agent", "looks good", Some(0)) {
            ObjectOp::AddComment { id, comment } => {
                assert_eq!(id, "x");
                assert_eq!(comment.id, "c1");
                assert_eq!(comment.author, "agent");
                assert!(matches!(comment.at, Some(CommentAnchor::Node { node_index: 0 })));
            }
            other => panic!("expected add-comment, got {}", other.kind()),
        }

        let mut scene = ObjectScene::default();
        set_selection(&mut scene, ObjectSelection::Object { id: "x".into() });
        assert_eq!(scene.selection, ObjectSelection::Object { id: "x".into() });
    }

    #[test]
    fn tool_catalog_is_complete_and_unique() {
        let tools = object_mcp_tools();
        let names: Vec<&str> = tools.iter().map(|t| t.name).collect();
        for want in [
            "list_objects",
            "get_object",
            "create_object",
            "patch_object",
            "tag_object",
            "add_comment",
            "set_selection",
            "query",
            "export",
        ] {
            assert!(names.contains(&want), "missing tool {want}");
        }
        let mut sorted = names.clone();
        sorted.sort_unstable();
        let before = sorted.len();
        sorted.dedup();
        assert_eq!(before, sorted.len(), "tool names are unique");
        for t in &tools {
            assert!(!t.description.is_empty());
            assert_eq!(t.schema["type"], "object");
        }
    }

    fn tag_then_insert(id: &str, order: &str, tags: &[&str]) -> ObjectOp {
        create_object(CreateObjectSpec {
            id: id.into(),
            order: order.into(),
            shape: CreateShape::Rect,
            x: 0.0,
            y: 0.0,
            width: Some(40),
            height: Some(40),
            text: None,
            style: None,
            tags: tags.iter().map(|t| t.to_string()).collect(),
        })
        .unwrap()
    }
}
