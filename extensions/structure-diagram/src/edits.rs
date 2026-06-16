//! Domain edits: the ONE authoring vocabulary both the MCP `author` seam and the
//! UI `resolve` seam emit. Each pure function mutates the [`Diagram`] only — never
//! the scene. Ids are caller-supplied (no rng in the pure layer).

use serde_json::Value;

use crate::model::{Diagram, Edge, Node};

pub const ADD_NODE: &str = "add_node";
pub const CONNECT: &str = "connect";
pub const RELABEL: &str = "relabel";
pub const MOVE_NODE: &str = "move_node";
pub const LIST: &str = "list";

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing string arg: {key}"))
}

fn opt_str_arg<'a>(args: &'a Value, key: &str) -> &'a str {
    args.get(key).and_then(Value::as_str).unwrap_or("")
}

fn f64_arg(args: &Value, key: &str) -> Result<f64, String> {
    args.get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("missing number arg: {key}"))
}

/// Apply a named domain edit. `Err` on an unknown tool or a bad arg.
pub fn apply_edit(diagram: &mut Diagram, tool: &str, args: &Value) -> Result<(), String> {
    match tool {
        ADD_NODE => add_node(diagram, args),
        CONNECT => connect(diagram, args),
        RELABEL => relabel(diagram, args),
        MOVE_NODE => move_node(diagram, args),
        other => Err(format!("unknown diagram author tool: {other}")),
    }
}

/// Add a node. `args: { id, label?, x?, y? }`.
fn add_node(diagram: &mut Diagram, args: &Value) -> Result<(), String> {
    let id = str_arg(args, "id")?.to_string();
    if diagram.node(&id).is_some() {
        return Err(format!("node id already exists: {id}"));
    }
    let label = opt_str_arg(args, "label").to_string();
    let x = args.get("x").and_then(Value::as_f64).unwrap_or(0.0);
    let y = args.get("y").and_then(Value::as_f64).unwrap_or(0.0);
    diagram.nodes.push(Node { id, label, x, y });
    Ok(())
}

/// Connect two existing nodes. `args: { id, from, to, label? }`.
fn connect(diagram: &mut Diagram, args: &Value) -> Result<(), String> {
    let id = str_arg(args, "id")?.to_string();
    if diagram.edge(&id).is_some() {
        return Err(format!("edge id already exists: {id}"));
    }
    let from = str_arg(args, "from")?.to_string();
    let to = str_arg(args, "to")?.to_string();
    if diagram.node(&from).is_none() {
        return Err(format!("no such node: {from}"));
    }
    if diagram.node(&to).is_none() {
        return Err(format!("no such node: {to}"));
    }
    let label = args.get("label").and_then(Value::as_str).map(str::to_string);
    diagram.edges.push(Edge { id, from, to, label });
    Ok(())
}

/// Relabel a node. `args: { node, label }`.
fn relabel(diagram: &mut Diagram, args: &Value) -> Result<(), String> {
    let node_id = str_arg(args, "node")?;
    let label = str_arg(args, "label")?.to_string();
    let node = diagram
        .node_mut(node_id)
        .ok_or_else(|| format!("no such node: {node_id}"))?;
    node.label = label;
    Ok(())
}

/// Move a node. `args: { node, x, y }`. (A node drag on the canvas is a normal
/// object move; this is the authoring-side equivalent for the AI / UI.)
fn move_node(diagram: &mut Diagram, args: &Value) -> Result<(), String> {
    let node_id = str_arg(args, "node")?;
    let x = f64_arg(args, "x")?;
    let y = f64_arg(args, "y")?;
    let node = diagram
        .node_mut(node_id)
        .ok_or_else(|| format!("no such node: {node_id}"))?;
    node.x = x;
    node.y = y;
    Ok(())
}
