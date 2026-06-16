//! `structure-diagram` (구조도): a reference data-rep extension proving a
//! RELATIONAL-SHAPED domain (node+edge graph) maps onto the core scene via
//! connectors/anchors — with zero new edge machinery.
//!
//! Domain model `Diagram { nodes, edges }` is the source of truth; nodes lower to
//! rect objects, edges to open 2-node connector objects with anchors (see
//! `export`), so `connection_graph`/`reproject_object_anchors`/`neighbors`/the
//! object-MCP mermaid digest all work for free over the result. The seams: MCP
//! (`edits` + the `read` projection + the `Extension` impl), export (`export`).
//! All pure.

mod edits;
mod export;
mod model;
mod read;

use serde_json::Value;
use shape_extension_contract::{Extension, IdOrderAlloc, McpToolMeta};
use shape_scene_core::object::ObjectOp;

use crate::edits::{ADD_NODE, CONNECT, LIST, MOVE_NODE, RELABEL};
use crate::model::Diagram;

/// The extension name: `ext_diagram_<tool>` MCP tools, `meta[ext] = "diagram"`.
pub const NAME: &str = "diagram";

#[derive(Clone, Copy, Debug, Default)]
pub struct DiagramExtension;

fn diagram_of(model: &Value) -> Result<Diagram, String> {
    if model.is_null() {
        return Ok(Diagram::default());
    }
    serde_json::from_value(model.clone()).map_err(|e| format!("invalid diagram model: {e}"))
}

fn diagram_to_json(diagram: &Diagram) -> Value {
    serde_json::to_value(diagram).expect("Diagram serializes")
}

impl Extension for DiagramExtension {
    fn name(&self) -> &'static str {
        NAME
    }

    fn mcp_tools(&self) -> Vec<McpToolMeta> {
        use serde_json::json;
        vec![
            McpToolMeta {
                name: LIST,
                description: "List the diagram as nodes and edges (domain vocabulary).",
                write: false,
                schema: json!({ "type": "object", "properties": {} }),
            },
            McpToolMeta {
                name: ADD_NODE,
                description: "Add a node at an optional (x, y) world position.",
                write: true,
                schema: json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "label": { "type": "string" },
                        "x": { "type": "number" },
                        "y": { "type": "number" },
                    },
                    "required": ["id"],
                }),
            },
            McpToolMeta {
                name: CONNECT,
                description: "Connect two existing nodes with a directed edge.",
                write: true,
                schema: json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "from": { "type": "string" },
                        "to": { "type": "string" },
                        "label": { "type": "string" },
                    },
                    "required": ["id", "from", "to"],
                }),
            },
            McpToolMeta {
                name: RELABEL,
                description: "Set a node's label.",
                write: true,
                schema: json!({
                    "type": "object",
                    "properties": { "node": { "type": "string" }, "label": { "type": "string" } },
                    "required": ["node", "label"],
                }),
            },
            McpToolMeta {
                name: MOVE_NODE,
                description: "Move a node to a new (x, y) world position.",
                write: true,
                schema: json!({
                    "type": "object",
                    "properties": {
                        "node": { "type": "string" },
                        "x": { "type": "number" },
                        "y": { "type": "number" },
                    },
                    "required": ["node", "x", "y"],
                }),
            },
        ]
    }

    fn read(&self, model: &Value, tool: &str, _args: &Value) -> Result<Value, String> {
        let diagram = diagram_of(model)?;
        match tool {
            LIST => Ok(read::list_diagram(&diagram)),
            other => Err(format!("unknown diagram read tool: {other}")),
        }
    }

    fn author(&self, model: &Value, tool: &str, args: &Value) -> Result<Value, String> {
        let mut diagram = diagram_of(model)?;
        edits::apply_edit(&mut diagram, tool, args)?;
        Ok(diagram_to_json(&diagram))
    }

    fn export(&self, model: &Value, alloc: &mut IdOrderAlloc) -> Result<Vec<ObjectOp>, String> {
        let diagram = diagram_of(model)?;
        export::export_diagram(&diagram, model, alloc)
    }

    fn empty_model(&self) -> Value {
        diagram_to_json(&Diagram::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use shape_extension_contract::{META_EXT_KEY, META_MODEL_KEY};
    use shape_scene_core::object::{
        apply_object_op, connection_graph, neighbors, reproject_object_anchors, ObjectScene,
        StubOutlineDeriver,
    };

    /// Drive the REAL core op-apply (no second apply) over the export ops.
    fn apply_all(ops: Vec<ObjectOp>) -> ObjectScene {
        let mut scene = ObjectScene::default();
        for op in ops {
            apply_object_op(&mut scene, op).expect("op applies through the real core");
        }
        scene
    }

    /// A diagram: A -> B, B -> C, authored through the MCP author seam.
    fn authored_diagram() -> Value {
        let ext = DiagramExtension;
        let mut m = ext.empty_model();
        m = ext.author(&m, ADD_NODE, &json!({ "id": "a", "label": "A", "x": 0.0, "y": 0.0 })).unwrap();
        m = ext.author(&m, ADD_NODE, &json!({ "id": "b", "label": "B", "x": 300.0, "y": 0.0 })).unwrap();
        m = ext.author(&m, ADD_NODE, &json!({ "id": "c", "label": "C", "x": 600.0, "y": 0.0 })).unwrap();
        m = ext.author(&m, CONNECT, &json!({ "id": "e1", "from": "a", "to": "b" })).unwrap();
        m = ext.author(&m, CONNECT, &json!({ "id": "e2", "from": "b", "to": "c" })).unwrap();
        m
    }

    // (a) export() round-trip: the core connection_graph over the exported scene
    // EQUALS the domain edge set — THE proof the relational domain rides anchors.
    #[test]
    fn connection_graph_equals_the_domain_edge_set() {
        let ext = DiagramExtension;
        let model = authored_diagram();
        let mut alloc = IdOrderAlloc::new(NAME);
        let ops = ext.export(&model, &mut alloc).unwrap();
        let scene = apply_all(ops);

        let a = alloc.id("node:a");
        let b = alloc.id("node:b");
        let c = alloc.id("node:c");
        let mut graph = connection_graph(&scene);
        graph.sort();
        let mut want = vec![(a.clone(), b.clone()), (b.clone(), c.clone())];
        want.sort();
        assert_eq!(graph, want, "core connection graph == domain edges (a-b, b-c)");

        // neighbors() over the scene matches the domain adjacency for free.
        assert_eq!(neighbors(&scene, &b).len(), 2, "B neighbors A and C");
    }

    // (a, cont.) reproject_object_anchors returns BOTH endpoints, each ON its
    // target node's outline.
    #[test]
    fn edge_anchors_reproject_onto_both_target_nodes() {
        let ext = DiagramExtension;
        let model = authored_diagram();
        let mut alloc = IdOrderAlloc::new(NAME);
        let ops = ext.export(&model, &mut alloc).unwrap();
        let scene = apply_all(ops);

        let deriver = StubOutlineDeriver;
        let edge_id = alloc.id("edge:e1");
        let endpoints = reproject_object_anchors(&deriver, &scene, &edge_id);
        assert_eq!(endpoints.len(), 2, "both edge endpoints reproject");
        assert_eq!(endpoints[0].0, 0);
        assert_eq!(endpoints[1].0, 1);

        // Each derived endpoint lies on its target node's outline.
        use shape_scene_core::object::OutlineDeriver;
        let a = scene.get(&alloc.id("node:a")).unwrap();
        let b = scene.get(&alloc.id("node:b")).unwrap();
        let on_outline = |obj: &shape_scene_core::object::Object, p| {
            let region = deriver.derive_region(&obj.geometry, 1).unwrap();
            region.outline.contains(&p)
        };
        assert!(on_outline(a, endpoints[0].1), "endpoint 0 on node A");
        assert!(on_outline(b, endpoints[1].1), "endpoint 1 on node B");
    }

    /// Each node lowers to a rect at its (x, y) world placement (the transform
    /// translation), with text = label.
    #[test]
    fn nodes_export_to_rects_at_their_world_position() {
        let ext = DiagramExtension;
        let model = authored_diagram();
        let mut alloc = IdOrderAlloc::new(NAME);
        let ops = ext.export(&model, &mut alloc).unwrap();
        let scene = apply_all(ops);

        let b = scene.get(&alloc.id("node:b")).expect("node B exists");
        assert_eq!(b.transform.m[0][2], 300.0, "B placed at x=300");
        assert_eq!(b.transform.m[1][2], 0.0);
        let text = b.text.as_ref().unwrap().runs[0].text.clone();
        assert_eq!(text, "B");
    }

    /// Every exported object carries `meta[ext] = "diagram"`; the root carries the
    /// model blob (a closed round-trip).
    #[test]
    fn exported_objects_tagged_and_root_carries_model() {
        let ext = DiagramExtension;
        let model = authored_diagram();
        let mut alloc = IdOrderAlloc::new(NAME);
        let ops = ext.export(&model, &mut alloc).unwrap();
        let scene = apply_all(ops);
        for o in &scene.objects {
            let meta = o.meta.as_ref().expect("exported object has meta");
            assert_eq!(meta[META_EXT_KEY], json!("diagram"), "object {} tagged", o.id);
        }
        let root = scene.get(&alloc.id("root")).unwrap();
        let blob = &root.meta.as_ref().unwrap()[META_MODEL_KEY];
        let restored: Diagram = serde_json::from_value(blob.clone()).unwrap();
        assert_eq!(restored.nodes.len(), 3);
        assert_eq!(restored.edges.len(), 2);
    }

    // (b) export determinism with a fixed allocator: byte-identical ops.
    #[test]
    fn export_is_deterministic_with_a_fixed_allocator() {
        let ext = DiagramExtension;
        let model = authored_diagram();
        let a = ext.export(&model, &mut IdOrderAlloc::new(NAME)).unwrap();
        let b = ext.export(&model, &mut IdOrderAlloc::new(NAME)).unwrap();
        assert_eq!(a, b, "same model + fixed allocator => byte-identical ops");
    }

    /// A move edit only changes a node's transform on re-export — its object id is
    /// reused (zero-rebake / incremental), and the edge still tracks it.
    #[test]
    fn move_node_reexports_with_a_reused_id_and_new_transform() {
        let ext = DiagramExtension;
        let model = authored_diagram();
        let moved = ext.author(&model, MOVE_NODE, &json!({ "node": "a", "x": 50.0, "y": 80.0 })).unwrap();
        let mut alloc = IdOrderAlloc::new(NAME);
        let ops = ext.export(&moved, &mut alloc).unwrap();
        let scene = apply_all(ops);
        let a = scene.get(&alloc.id("node:a")).expect("node A id reused after move");
        assert_eq!(a.transform.m[0][2], 50.0);
        assert_eq!(a.transform.m[1][2], 80.0);
        // The edge a->b still resolves over the moved node.
        let deriver = StubOutlineDeriver;
        let eps = reproject_object_anchors(&deriver, &scene, &alloc.id("edge:e1"));
        assert_eq!(eps.len(), 2, "edge still tracks the moved node");
    }

    #[test]
    fn connect_rejects_missing_nodes() {
        let ext = DiagramExtension;
        let model = ext.author(&ext.empty_model(), ADD_NODE, &json!({ "id": "a" })).unwrap();
        assert!(
            ext.author(&model, CONNECT, &json!({ "id": "e", "from": "a", "to": "ghost" })).is_err(),
            "connect to a missing node is rejected"
        );
    }
}
