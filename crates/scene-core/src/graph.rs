//! Decision-graph helpers — a faithful Rust port of `src/shared/graph.ts`.
//!
//! Patch application (`apply_graph_patch`), AABB geometry, the text digest and
//! Mermaid renderers, subgraph/scene projections, and the display-label maps.
//! Output strings (`graph_text_digest`, `make_mermaid`) are kept byte-identical
//! to the TS source so they can be golden-verified.

use std::collections::{HashMap, HashSet};

use crate::model::{
    Bounds, DecisionGraph, EdgeType, ExportType, GraphEdge, GraphNode, NodeType, Scene, SceneEdge,
    SceneGroup, SceneNode, SceneSelection, Tag,
};

// ---------------------------------------------------------------------------
// Graph patch — mirrors `graphPatchSchema`.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphPatch {
    #[serde(default)]
    pub add_nodes: Vec<GraphNode>,
    #[serde(default)]
    pub update_nodes: Vec<GraphNode>,
    #[serde(default)]
    pub remove_node_ids: Vec<String>,
    #[serde(default)]
    pub add_edges: Vec<GraphEdge>,
    #[serde(default)]
    pub update_edges: Vec<GraphEdge>,
    #[serde(default)]
    pub remove_edge_ids: Vec<String>,
}

// ---------------------------------------------------------------------------
// Insertion-ordered map (mirrors JS `Map`: `set` keeps a key's position, a new
// key appends, `delete` removes; iteration follows insertion order).
// ---------------------------------------------------------------------------

struct OrderedMap<V> {
    order: Vec<String>,
    map: HashMap<String, V>,
}

impl<V: Clone> OrderedMap<V> {
    fn new() -> Self {
        OrderedMap {
            order: Vec::new(),
            map: HashMap::new(),
        }
    }

    fn set(&mut self, key: String, value: V) {
        if !self.map.contains_key(&key) {
            self.order.push(key.clone());
        }
        self.map.insert(key, value);
    }

    fn delete(&mut self, key: &str) {
        if self.map.remove(key).is_some() {
            self.order.retain(|k| k != key);
        }
    }

    fn keys_in_order(&self) -> Vec<String> {
        self.order.clone()
    }

    fn into_values(mut self) -> Vec<V> {
        self.order
            .iter()
            .filter_map(|k| self.map.remove(k))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Patch application
// ---------------------------------------------------------------------------

/// Port of `applyGraphPatch`. Applies, in order: cascading node removal,
/// edge removal, node add/update, edge add/update; then prunes any edge whose
/// endpoints no longer exist. Result `version` is always `1`.
pub fn apply_graph_patch(graph: &DecisionGraph, patch: &GraphPatch) -> DecisionGraph {
    let mut node_map: OrderedMap<GraphNode> = OrderedMap::new();
    for node in &graph.nodes {
        node_map.set(node.id.clone(), node.clone());
    }
    let mut edge_map: OrderedMap<GraphEdge> = OrderedMap::new();
    for edge in &graph.edges {
        edge_map.set(edge.id.clone(), edge.clone());
    }

    for node_id in &patch.remove_node_ids {
        node_map.delete(node_id);
        let to_remove: Vec<String> = edge_map
            .keys_in_order()
            .into_iter()
            .filter(|edge_id| {
                edge_map
                    .map
                    .get(edge_id)
                    .map(|edge| &edge.source == node_id || &edge.target == node_id)
                    .unwrap_or(false)
            })
            .collect();
        for edge_id in to_remove {
            edge_map.delete(&edge_id);
        }
    }
    for edge_id in &patch.remove_edge_ids {
        edge_map.delete(edge_id);
    }
    for node in &patch.add_nodes {
        node_map.set(node.id.clone(), node.clone());
    }
    for node in &patch.update_nodes {
        node_map.set(node.id.clone(), node.clone());
    }
    for edge in &patch.add_edges {
        edge_map.set(edge.id.clone(), edge.clone());
    }
    for edge in &patch.update_edges {
        edge_map.set(edge.id.clone(), edge.clone());
    }

    let node_ids: HashSet<String> = node_map.map.keys().cloned().collect();
    let nodes = node_map.into_values();
    let edges = edge_map
        .into_values()
        .into_iter()
        .filter(|edge| node_ids.contains(&edge.source) && node_ids.contains(&edge.target))
        .collect();

    DecisionGraph {
        version: 1,
        nodes,
        edges,
    }
}

// ---------------------------------------------------------------------------
// Geometry (inclusive AABB)
// ---------------------------------------------------------------------------

pub fn bounds_intersect(a: &Bounds, b: &Bounds) -> bool {
    a.x <= b.x + b.width && a.x + a.width >= b.x && a.y <= b.y + b.height && a.y + a.height >= b.y
}

pub fn point_in_bounds(x: f64, y: f64, bounds: &Bounds) -> bool {
    x >= bounds.x && x <= bounds.x + bounds.width && y >= bounds.y && y <= bounds.y + bounds.height
}

pub fn node_bounds(node: &SceneNode) -> Bounds {
    Bounds {
        x: node.position.x,
        y: node.position.y,
        width: node.size.width,
        height: node.size.height,
    }
}

pub fn expanded_bounds(bounds: &Bounds, padding: f64) -> Bounds {
    Bounds {
        x: bounds.x - padding,
        y: bounds.y - padding,
        width: bounds.width + padding * 2.0,
        height: bounds.height + padding * 2.0,
    }
}

// ---------------------------------------------------------------------------
// Display-label maps
// ---------------------------------------------------------------------------

pub fn node_type_labels(node_type: NodeType) -> &'static str {
    match node_type {
        NodeType::Proposition => "Proposition",
        NodeType::DecisionPoint => "Decision point",
        NodeType::Option => "Option",
        NodeType::Evidence => "Evidence",
        NodeType::Tradeoff => "Tradeoff",
        NodeType::Blocker => "Blocker",
        NodeType::Subdecision => "Subdecision",
        NodeType::Task => "Task",
        NodeType::Artifact => "Artifact",
    }
}

pub fn edge_type_labels(edge_type: EdgeType) -> &'static str {
    match edge_type {
        EdgeType::DependsOn => "depends on",
        EdgeType::Supports => "supports",
        EdgeType::Blocks => "blocks",
        EdgeType::TradesOffWith => "trades off",
        EdgeType::ChoosesBetween => "chooses",
        EdgeType::DecomposesTo => "decomposes",
        EdgeType::Produces => "produces",
    }
}

pub fn export_type_labels(export_type: ExportType) -> &'static str {
    match export_type {
        ExportType::Madr => "MADR Markdown",
        ExportType::Yadr => "YADR YAML",
        ExportType::ImagePrompt => "Image prompt",
        ExportType::AiPlanMd => "AI task plan",
        ExportType::DesignDocMd => "MADR Markdown",
        ExportType::ConfluenceHtml => "Confluence draft",
        ExportType::Mermaid => "Mermaid diagram",
        ExportType::ArchitectureImage => "Image prompt",
    }
}

// ---------------------------------------------------------------------------
// Text digest + Mermaid
// ---------------------------------------------------------------------------

/// `Math.round` rounds half toward +Infinity; for `confidence` in `[0, 1]`,
/// `confidence * 100` is non-negative, so half-away-from-zero matches.
#[allow(clippy::cast_possible_truncation, reason = "rounded percentage in [0,100] fits i64")]
fn round_pct(confidence: f64) -> i64 {
    (confidence * 100.0).round() as i64
}

/// Mirror of TS `value || fallback`: pick `value` unless it is empty.
fn or_else<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.is_empty() {
        fallback
    } else {
        value
    }
}

pub fn graph_text_digest(graph: &DecisionGraph) -> String {
    let nodes = graph
        .nodes
        .iter()
        .map(node_line)
        .collect::<Vec<_>>()
        .join("\n");
    let edges = graph
        .edges
        .iter()
        .map(edge_line)
        .collect::<Vec<_>>()
        .join("\n");

    let nodes_block = if nodes.is_empty() { "- none" } else { nodes.as_str() };
    let edges_block = if edges.is_empty() { "- none" } else { edges.as_str() };

    [
        "Nodes:",
        nodes_block,
        "",
        "Edges:",
        edges_block,
    ]
    .join("\n")
}

fn node_line(node: &GraphNode) -> String {
    format!(
        "- [{}/{}/{}%] {}: {} — {}",
        node_type_value(node.node_type),
        node_status_str(node),
        round_pct(node.confidence),
        node.id,
        node.title,
        node.summary
    )
}

fn edge_line(edge: &GraphEdge) -> String {
    format!(
        "- [{}/{}%] {} -> {}: {}",
        edge_type_value(edge.edge_type),
        round_pct(edge.confidence),
        edge.source,
        edge.target,
        or_else(&edge.label, &edge.rationale)
    )
}

pub fn make_mermaid(graph: &DecisionGraph) -> String {
    let mut lines: Vec<String> = vec!["flowchart LR".to_string()];
    for node in &graph.nodes {
        lines.push(format!(
            "  {}[\"{}\"]",
            safe_mermaid_id(&node.id),
            escape_mermaid(&format!(
                "{}: {}",
                node_type_labels(node.node_type),
                node.title
            ))
        ));
    }
    for edge in &graph.edges {
        let label = or_else(&edge.label, edge_type_labels(edge.edge_type));
        lines.push(format!(
            "  {} -->|\"{}\"| {}",
            safe_mermaid_id(&edge.source),
            escape_mermaid(label),
            safe_mermaid_id(&edge.target)
        ));
    }
    lines.join("\n")
}

// ---------------------------------------------------------------------------
// Subgraph / scene projections
// ---------------------------------------------------------------------------

/// Port of `selectedSubgraph`. For `group`/`selection` scopes (or a missing id)
/// the whole graph passes through; `node` expands to its 1-hop neighbourhood;
/// `edge` narrows to that edge and its two endpoints.
pub fn selected_subgraph(
    graph: &DecisionGraph,
    scope_kind: &str,
    scope_id: Option<&str>,
) -> DecisionGraph {
    let scope_id = match scope_id {
        Some(id) => id,
        None => return graph.clone(),
    };
    if scope_kind == "group" || scope_kind == "selection" {
        return graph.clone();
    }

    if scope_kind == "node" {
        let mut node_ids: HashSet<String> = HashSet::new();
        node_ids.insert(scope_id.to_string());
        for edge in &graph.edges {
            if edge.source == scope_id {
                node_ids.insert(edge.target.clone());
            }
            if edge.target == scope_id {
                node_ids.insert(edge.source.clone());
            }
        }
        return pick_subgraph(graph, &node_ids, None);
    }

    if scope_kind == "edge" {
        let edge = graph.edges.iter().find(|candidate| candidate.id == scope_id);
        match edge {
            None => {
                return DecisionGraph {
                    version: 1,
                    nodes: Vec::new(),
                    edges: Vec::new(),
                };
            }
            Some(edge) => {
                let mut node_ids: HashSet<String> = HashSet::new();
                node_ids.insert(edge.source.clone());
                node_ids.insert(edge.target.clone());
                let mut edge_ids: HashSet<String> = HashSet::new();
                edge_ids.insert(edge.id.clone());
                return pick_subgraph(graph, &node_ids, Some(&edge_ids));
            }
        }
    }

    graph.clone()
}

/// Port of `sceneGraphForGroup`: project the scene-level group subtree rooted at
/// `group_id` down to a placement-free `DecisionGraph`.
pub fn scene_graph_for_group(scene: &Scene, group_id: &str) -> DecisionGraph {
    let group_ids = descendant_group_ids(scene, group_id);
    let nodes: Vec<GraphNode> = scene
        .nodes
        .iter()
        .filter(|node| group_ids.contains(&node.group_id))
        .map(strip_scene_node)
        .collect();
    let node_ids: HashSet<String> = nodes.iter().map(|node| node.id.clone()).collect();
    let edges: Vec<GraphEdge> = scene
        .edges
        .iter()
        .filter(|edge| {
            group_ids.contains(&edge.group_id)
                && node_ids.contains(&edge.source)
                && node_ids.contains(&edge.target)
        })
        .map(strip_scene_edge)
        .collect();
    DecisionGraph {
        version: 1,
        nodes,
        edges,
    }
}

/// Port of `descendantGroupIds`: the transitive closure of groups whose
/// `parentGroupId` chain reaches `group_id` (inclusive of `group_id`).
pub fn descendant_group_ids(scene: &Scene, group_id: &str) -> HashSet<String> {
    let mut result: HashSet<String> = HashSet::new();
    result.insert(group_id.to_string());
    let mut changed = true;
    while changed {
        changed = false;
        for group in &scene.groups {
            if let Some(parent) = &group.parent_group_id {
                if result.contains(parent) && !result.contains(&group.id) {
                    result.insert(group.id.clone());
                    changed = true;
                }
            }
        }
    }
    result
}

/// Port of `groupTags`: resolve a group's `tagIds` against the tag table, in
/// `tagIds` order, dropping ids with no matching tag.
pub fn group_tags(group: &SceneGroup, tags: &[Tag]) -> Vec<Tag> {
    let by_id: HashMap<&str, &Tag> = tags.iter().map(|tag| (tag.id.as_str(), tag)).collect();
    group
        .tag_ids
        .iter()
        .filter_map(|tag_id| by_id.get(tag_id.as_str()).map(|tag| (*tag).clone()))
        .collect()
}

/// Port of `selectionTarget`: stable string key for a selection.
pub fn selection_target(selection: &SceneSelection) -> String {
    match selection {
        SceneSelection::Canvas => "canvas".to_string(),
        SceneSelection::Multi { ids } => format!("multi:{}", ids.join(",")),
        SceneSelection::Group { id } => format!("group:{}", id),
        SceneSelection::Node { id } => format!("node:{}", id),
        SceneSelection::Edge { id } => format!("edge:{}", id),
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn pick_subgraph(
    graph: &DecisionGraph,
    node_ids: &HashSet<String>,
    edge_ids: Option<&HashSet<String>>,
) -> DecisionGraph {
    DecisionGraph {
        version: 1,
        nodes: graph
            .nodes
            .iter()
            .filter(|node| node_ids.contains(&node.id))
            .cloned()
            .collect(),
        edges: graph
            .edges
            .iter()
            .filter(|edge| {
                node_ids.contains(&edge.source)
                    && node_ids.contains(&edge.target)
                    && edge_ids.map(|ids| ids.contains(&edge.id)).unwrap_or(true)
            })
            .cloned()
            .collect(),
    }
}

fn strip_scene_node(node: &SceneNode) -> GraphNode {
    GraphNode {
        id: node.id.clone(),
        node_type: node.node_type,
        title: node.title.clone(),
        summary: node.summary.clone(),
        detail: node.detail.clone(),
        status: node.status,
        confidence: node.confidence,
        evidence_refs: node.evidence_refs.clone(),
        child_decision_ids: node.child_decision_ids.clone(),
    }
}

fn strip_scene_edge(edge: &SceneEdge) -> GraphEdge {
    GraphEdge {
        id: edge.id.clone(),
        edge_type: edge.edge_type,
        source: edge.source.clone(),
        target: edge.target.clone(),
        label: edge.label.clone(),
        rationale: edge.rationale.clone(),
        confidence: edge.confidence,
    }
}

fn safe_mermaid_id(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn escape_mermaid(value: &str) -> String {
    value.replace('"', "'")
}

// ---------------------------------------------------------------------------
// snake_case serde-value lookups for the digest (`node.type`, `node.status`,
// `edge.type` are emitted as their wire string, not their display label).
// ---------------------------------------------------------------------------

fn node_type_value(node_type: NodeType) -> &'static str {
    match node_type {
        NodeType::Proposition => "proposition",
        NodeType::DecisionPoint => "decision_point",
        NodeType::Option => "option",
        NodeType::Evidence => "evidence",
        NodeType::Tradeoff => "tradeoff",
        NodeType::Blocker => "blocker",
        NodeType::Subdecision => "subdecision",
        NodeType::Task => "task",
        NodeType::Artifact => "artifact",
    }
}

fn edge_type_value(edge_type: EdgeType) -> &'static str {
    match edge_type {
        EdgeType::DependsOn => "depends_on",
        EdgeType::Supports => "supports",
        EdgeType::Blocks => "blocks",
        EdgeType::TradesOffWith => "trades_off_with",
        EdgeType::ChoosesBetween => "chooses_between",
        EdgeType::DecomposesTo => "decomposes_to",
        EdgeType::Produces => "produces",
    }
}

fn node_status_str(node: &GraphNode) -> &'static str {
    use crate::model::NodeStatus::*;
    match node.status {
        Draft => "draft",
        Viable => "viable",
        Conditional => "conditional",
        Infeasible => "infeasible",
        Unknown => "unknown",
        Selected => "selected",
        Deferred => "deferred",
        Complete => "complete",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{NodeStatus, Point, Size};

    fn gnode(id: &str, conf: f64) -> GraphNode {
        GraphNode {
            id: id.to_string(),
            node_type: NodeType::Option,
            title: format!("Title {id}"),
            summary: format!("Summary {id}"),
            detail: String::new(),
            status: NodeStatus::Viable,
            confidence: conf,
            evidence_refs: Vec::new(),
            child_decision_ids: Vec::new(),
        }
    }

    fn gedge(id: &str, source: &str, target: &str) -> GraphEdge {
        GraphEdge {
            id: id.to_string(),
            edge_type: EdgeType::Supports,
            source: source.to_string(),
            target: target.to_string(),
            label: String::new(),
            rationale: format!("because {id}"),
            confidence: 0.5,
        }
    }

    fn graph(nodes: Vec<GraphNode>, edges: Vec<GraphEdge>) -> DecisionGraph {
        DecisionGraph {
            version: 1,
            nodes,
            edges,
        }
    }

    #[test]
    fn apply_patch_ordering_and_cascade_prune() {
        let g = graph(
            vec![gnode("a", 0.5), gnode("b", 0.5), gnode("c", 0.5)],
            vec![gedge("e1", "a", "b"), gedge("e2", "b", "c")],
        );
        // Remove node b: cascade deletes e1 and e2. Then add node d and edge e3 a->d.
        let patch = GraphPatch {
            remove_node_ids: vec!["b".to_string()],
            add_nodes: vec![gnode("d", 0.5)],
            add_edges: vec![gedge("e3", "a", "d")],
            ..Default::default()
        };
        let out = apply_graph_patch(&g, &patch);
        let node_ids: Vec<&str> = out.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(node_ids, vec!["a", "c", "d"]);
        let edge_ids: Vec<&str> = out.edges.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(edge_ids, vec!["e3"]);
        assert_eq!(out.version, 1);
    }

    #[test]
    fn apply_patch_referential_integrity_prune() {
        // Add an edge whose target node is never present -> pruned at the end.
        let g = graph(vec![gnode("a", 0.5)], vec![]);
        let patch = GraphPatch {
            add_edges: vec![gedge("e1", "a", "ghost")],
            ..Default::default()
        };
        let out = apply_graph_patch(&g, &patch);
        assert!(out.edges.is_empty());
    }

    #[test]
    fn apply_patch_update_overwrites_in_place() {
        let g = graph(vec![gnode("a", 0.5), gnode("b", 0.5)], vec![]);
        let mut updated = gnode("a", 0.9);
        updated.title = "Updated A".to_string();
        let patch = GraphPatch {
            update_nodes: vec![updated],
            ..Default::default()
        };
        let out = apply_graph_patch(&g, &patch);
        // Order preserved (a stays first), value overwritten.
        assert_eq!(out.nodes[0].id, "a");
        assert_eq!(out.nodes[0].title, "Updated A");
        assert_eq!(out.nodes[0].confidence, 0.9);
    }

    #[test]
    fn digest_format_matches_ts() {
        let g = graph(
            vec![GraphNode {
                id: "n1".to_string(),
                node_type: NodeType::DecisionPoint,
                title: "Pick DB".to_string(),
                summary: "Choose a store".to_string(),
                detail: String::new(),
                status: NodeStatus::Viable,
                confidence: 0.756,
                evidence_refs: Vec::new(),
                child_decision_ids: Vec::new(),
            }],
            vec![GraphEdge {
                id: "e1".to_string(),
                edge_type: EdgeType::DependsOn,
                source: "n1".to_string(),
                target: "n2".to_string(),
                label: String::new(),
                rationale: "needs schema".to_string(),
                confidence: 0.5,
            }],
        );
        let digest = graph_text_digest(&g);
        let expected = concat!(
            "Nodes:\n",
            "- [decision_point/viable/76%] n1: Pick DB — Choose a store\n",
            "\n",
            "Edges:\n",
            "- [depends_on/50%] n1 -> n2: needs schema"
        );
        assert_eq!(digest, expected);
    }

    #[test]
    fn digest_empty_uses_none_placeholders() {
        let g = graph(vec![], vec![]);
        assert_eq!(
            graph_text_digest(&g),
            "Nodes:\n- none\n\nEdges:\n- none"
        );
    }

    #[test]
    fn digest_edge_label_takes_priority_over_rationale() {
        let mut e = gedge("e1", "a", "b");
        e.label = "blocks build".to_string();
        let g = graph(vec![gnode("a", 0.5), gnode("b", 0.5)], vec![e]);
        let digest = graph_text_digest(&g);
        assert!(digest.contains("a -> b: blocks build"));
    }

    #[test]
    fn mermaid_format_matches_ts() {
        let g = graph(
            vec![GraphNode {
                id: "node-1".to_string(),
                node_type: NodeType::Option,
                title: "Use \"Postgres\"".to_string(),
                summary: String::new(),
                detail: String::new(),
                status: NodeStatus::Draft,
                confidence: 0.5,
                evidence_refs: Vec::new(),
                child_decision_ids: Vec::new(),
            }],
            vec![GraphEdge {
                id: "edge-1".to_string(),
                edge_type: EdgeType::Supports,
                source: "node-1".to_string(),
                target: "node-2".to_string(),
                label: String::new(),
                rationale: String::new(),
                confidence: 0.5,
            }],
        );
        let mermaid = make_mermaid(&g);
        let expected = concat!(
            "flowchart LR\n",
            "  node_1[\"Option: Use 'Postgres'\"]\n",
            "  node_1 -->|\"supports\"| node_2"
        );
        assert_eq!(mermaid, expected);
    }

    #[test]
    fn descendant_group_closure() {
        let scene = Scene {
            version: 1,
            scene_version: 0,
            groups: vec![
                group_fixture("root", None),
                group_fixture("child", Some("root")),
                group_fixture("grandchild", Some("child")),
                group_fixture("other", None),
            ],
            nodes: vec![],
            edges: vec![],
            tags: vec![],
            comments: vec![],
            artifacts: vec![],
            proposals: None,
            selection: SceneSelection::Canvas,
            updated_at: "t".to_string(),
        };
        let ids = descendant_group_ids(&scene, "root");
        assert!(ids.contains("root"));
        assert!(ids.contains("child"));
        assert!(ids.contains("grandchild"));
        assert!(!ids.contains("other"));
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn scene_graph_for_group_strips_placement() {
        let scene = Scene {
            version: 1,
            scene_version: 0,
            groups: vec![
                group_fixture("root", None),
                group_fixture("child", Some("root")),
            ],
            nodes: vec![
                scene_node_fixture("n1", "root"),
                scene_node_fixture("n2", "child"),
                scene_node_fixture("n3", "other"),
            ],
            edges: vec![
                scene_edge_fixture("e1", "root", "n1", "n2"),
                // endpoint outside the subtree -> dropped
                scene_edge_fixture("e2", "child", "n2", "n3"),
            ],
            tags: vec![],
            comments: vec![],
            artifacts: vec![],
            proposals: None,
            selection: SceneSelection::Canvas,
            updated_at: "t".to_string(),
        };
        let g = scene_graph_for_group(&scene, "root");
        let node_ids: Vec<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(node_ids, vec!["n1", "n2"]);
        let edge_ids: Vec<&str> = g.edges.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(edge_ids, vec!["e1"]);
    }

    #[test]
    fn selected_subgraph_node_neighbourhood() {
        let g = graph(
            vec![gnode("a", 0.5), gnode("b", 0.5), gnode("c", 0.5), gnode("d", 0.5)],
            vec![gedge("e1", "a", "b"), gedge("e2", "c", "a"), gedge("e3", "c", "d")],
        );
        let sub = selected_subgraph(&g, "node", Some("a"));
        let mut node_ids: Vec<&str> = sub.nodes.iter().map(|n| n.id.as_str()).collect();
        node_ids.sort();
        assert_eq!(node_ids, vec!["a", "b", "c"]);
        // both edges incident to a are retained (both endpoints in set)
        let mut edge_ids: Vec<&str> = sub.edges.iter().map(|e| e.id.as_str()).collect();
        edge_ids.sort();
        assert_eq!(edge_ids, vec!["e1", "e2"]);
    }

    #[test]
    fn selected_subgraph_group_returns_whole() {
        let g = graph(vec![gnode("a", 0.5)], vec![]);
        let sub = selected_subgraph(&g, "group", Some("g1"));
        assert_eq!(sub.nodes.len(), 1);
    }

    #[test]
    fn selected_subgraph_missing_id_returns_whole() {
        let g = graph(vec![gnode("a", 0.5)], vec![]);
        let sub = selected_subgraph(&g, "node", None);
        assert_eq!(sub.nodes.len(), 1);
    }

    #[test]
    fn selected_subgraph_edge_missing_returns_empty() {
        let g = graph(vec![gnode("a", 0.5)], vec![]);
        let sub = selected_subgraph(&g, "edge", Some("nope"));
        assert!(sub.nodes.is_empty());
        assert!(sub.edges.is_empty());
    }

    #[test]
    fn geometry_inclusive_aabb() {
        let a = Bounds { x: 0.0, y: 0.0, width: 10.0, height: 10.0 };
        let b = Bounds { x: 10.0, y: 10.0, width: 5.0, height: 5.0 };
        // Touching edges count as intersecting (inclusive).
        assert!(bounds_intersect(&a, &b));
        let c = Bounds { x: 11.0, y: 0.0, width: 5.0, height: 5.0 };
        assert!(!bounds_intersect(&a, &c));
        assert!(point_in_bounds(0.0, 0.0, &a));
        assert!(point_in_bounds(10.0, 10.0, &a));
        assert!(!point_in_bounds(10.01, 5.0, &a));
        let e = expanded_bounds(&a, 2.0);
        assert_eq!(e, Bounds { x: -2.0, y: -2.0, width: 14.0, height: 14.0 });
    }

    #[test]
    fn node_bounds_from_position_size() {
        let node = scene_node_fixture("n", "g");
        let b = node_bounds(&node);
        assert_eq!(b, Bounds { x: 1.0, y: 2.0, width: 390.0, height: 390.0 });
    }

    #[test]
    fn label_collisions_match_ts() {
        assert_eq!(export_type_labels(ExportType::Madr), "MADR Markdown");
        assert_eq!(export_type_labels(ExportType::DesignDocMd), "MADR Markdown");
        assert_eq!(export_type_labels(ExportType::ImagePrompt), "Image prompt");
        assert_eq!(
            export_type_labels(ExportType::ArchitectureImage),
            "Image prompt"
        );
        assert_eq!(node_type_labels(NodeType::DecisionPoint), "Decision point");
        assert_eq!(edge_type_labels(EdgeType::TradesOffWith), "trades off");
    }

    #[test]
    fn group_tags_resolves_in_order_dropping_unknown() {
        let tags = vec![
            tag_fixture("t1", "alpha"),
            tag_fixture("t2", "beta"),
        ];
        let mut group = group_fixture("g", None);
        group.tag_ids = vec!["t2".to_string(), "missing".to_string(), "t1".to_string()];
        let resolved = group_tags(&group, &tags);
        let names: Vec<&str> = resolved.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["beta", "alpha"]);
    }

    #[test]
    fn selection_target_strings() {
        assert_eq!(selection_target(&SceneSelection::Canvas), "canvas");
        assert_eq!(
            selection_target(&SceneSelection::Node { id: "n1".into() }),
            "node:n1"
        );
        assert_eq!(
            selection_target(&SceneSelection::Group { id: "g1".into() }),
            "group:g1"
        );
        assert_eq!(
            selection_target(&SceneSelection::Edge { id: "e1".into() }),
            "edge:e1"
        );
        assert_eq!(
            selection_target(&SceneSelection::Multi {
                ids: vec!["a".into(), "b".into()]
            }),
            "multi:a,b"
        );
    }

    #[test]
    fn graph_patch_deserializes_with_defaults() {
        let patch: GraphPatch = serde_json::from_str("{}").unwrap();
        assert_eq!(patch, GraphPatch::default());
        let patch: GraphPatch =
            serde_json::from_str(r#"{"removeNodeIds":["x"]}"#).unwrap();
        assert_eq!(patch.remove_node_ids, vec!["x".to_string()]);
        assert!(patch.add_nodes.is_empty());
    }

    // --- fixtures -------------------------------------------------------------

    fn group_fixture(id: &str, parent: Option<&str>) -> SceneGroup {
        SceneGroup {
            id: id.to_string(),
            parent_group_id: parent.map(|p| p.to_string()),
            title: id.to_string(),
            summary: String::new(),
            bounds: Bounds { x: 0.0, y: 0.0, width: 0.0, height: 0.0 },
            tag_ids: Vec::new(),
            z_index: 0.0,
            collapsed: false,
            created_at: "t".to_string(),
            updated_at: "t".to_string(),
            meta: None,
        }
    }

    fn scene_node_fixture(id: &str, group_id: &str) -> SceneNode {
        SceneNode {
            id: id.to_string(),
            node_type: NodeType::Option,
            title: format!("Title {id}"),
            summary: String::new(),
            detail: String::new(),
            status: NodeStatus::Draft,
            confidence: 0.5,
            evidence_refs: Vec::new(),
            child_decision_ids: Vec::new(),
            group_id: group_id.to_string(),
            position: Point { x: 1.0, y: 2.0 },
            size: Size { width: 390.0, height: 390.0 },
            z_index: 0.0,
            tag_ids: Vec::new(),
            updated_at: None,
            meta: None,
        }
    }

    fn scene_edge_fixture(id: &str, group_id: &str, source: &str, target: &str) -> SceneEdge {
        SceneEdge {
            id: id.to_string(),
            edge_type: EdgeType::Supports,
            source: source.to_string(),
            target: target.to_string(),
            label: String::new(),
            rationale: String::new(),
            confidence: 0.5,
            group_id: group_id.to_string(),
            tag_ids: Vec::new(),
            updated_at: None,
            meta: None,
        }
    }

    fn tag_fixture(id: &str, name: &str) -> Tag {
        Tag {
            id: id.to_string(),
            name: name.to_string(),
            color: "#fff".to_string(),
            description: String::new(),
            created_at: "t".to_string(),
            updated_at: "t".to_string(),
        }
    }
}
