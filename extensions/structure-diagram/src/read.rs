//! The domain read projection backing the MCP `list` tool: a plain node/edge
//! listing over the DOMAIN model (the same shape `object_mcp::export` digest gives
//! over the scene, but in domain vocabulary).

use serde_json::{json, Value};

use crate::model::Diagram;

/// A read projection of the diagram for the MCP `list` tool.
pub fn list_diagram(diagram: &Diagram) -> Value {
    json!({
        "nodes": diagram.nodes.iter().map(|n| json!({
            "id": n.id, "label": n.label, "x": n.x, "y": n.y,
        })).collect::<Vec<_>>(),
        "edges": diagram.edges.iter().map(|e| json!({
            "id": e.id, "from": e.from, "to": e.to, "label": e.label,
        })).collect::<Vec<_>>(),
    })
}
