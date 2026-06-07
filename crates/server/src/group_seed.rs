//! MG-7 group seeding — a faithful port of `src/server/local.ts::seedGroupScene`
//! and the `nextGroupOffset` grid packing from `src/server/storage.ts`.
//!
//! The Node `POST /api/groups` route seeds a brand-new group with a FIXED
//! 10-node / 9-edge decision graph, then offsets the whole group onto a free grid
//! cell so it does not overlap existing top-level groups. This module reproduces
//! both, emitting a [`ScenePatch`] the canvas actor applies through scene-core so
//! the seed persists + broadcasts exactly like any other write.

use shape_scene_core::{
    Bounds, EdgeType, NodeStatus, NodeType, Point, Scene, SceneEdge, SceneGroup, SceneNode,
    ScenePatch, Size,
};

const NODE_WIDTH: f64 = 270.0;
const NODE_HEIGHT: f64 = 178.0;
const BOUNDS_PADDING: f64 = 120.0;

const GROUP_GAP: f64 = 140.0;
const COLLISION_PADDING: f64 = 60.0;

/// One seeded node spec: the placement-free graph fields plus its grid position.
struct SeedNode {
    key: &'static str,
    node_type: NodeType,
    title: &'static str,
    summary: &'static str,
    status: NodeStatus,
    confidence: f64,
    x: f64,
    y: f64,
}

struct SeedEdge {
    key: &'static str,
    edge_type: EdgeType,
    source: &'static str,
    target: &'static str,
    label: &'static str,
}

/// The fixed seed graph. Node positions are the compact `defaultSeedNodePositions`
/// from `local.ts`; the proposition's title is set to the prompt title in
/// [`seed_group`] (its `title` here is a placeholder).
fn seed_nodes() -> [SeedNode; 10] {
    [
        SeedNode {
            key: "n-proposition",
            node_type: NodeType::Proposition,
            title: "",
            summary: "Problem statement and target outcome.",
            status: NodeStatus::Selected,
            confidence: 0.72,
            x: 0.0,
            y: 300.0,
        },
        SeedNode {
            key: "n-decision-points",
            node_type: NodeType::DecisionPoint,
            title: "Decision points",
            summary: "The choice should be driven by feasibility, evidence strength, reversibility, and agent permissions.",
            status: NodeStatus::Draft,
            confidence: 0.68,
            x: 360.0,
            y: 300.0,
        },
        SeedNode {
            key: "n-option-graph",
            node_type: NodeType::Option,
            title: "Typed decision graph",
            summary: "Use a typed graph as the source of truth for discussion and exports.",
            status: NodeStatus::Viable,
            confidence: 0.82,
            x: 720.0,
            y: 120.0,
        },
        SeedNode {
            key: "n-option-freeform",
            node_type: NodeType::Option,
            title: "Freeform mindmap",
            summary: "Flexible, but weak at enforcing architectural decision quality.",
            status: NodeStatus::Conditional,
            confidence: 0.54,
            x: 720.0,
            y: 480.0,
        },
        SeedNode {
            key: "n-evidence",
            node_type: NodeType::Evidence,
            title: "Evidence ledger",
            summary: "Every recommendation should trace to a concrete assumption, source, or probe.",
            status: NodeStatus::Draft,
            confidence: 0.7,
            x: 1080.0,
            y: 40.0,
        },
        SeedNode {
            key: "n-tradeoff",
            node_type: NodeType::Tradeoff,
            title: "Readable vs complete",
            summary: "Show compact nodes by default and move deep rationale into the inspector.",
            status: NodeStatus::Draft,
            confidence: 0.75,
            x: 1080.0,
            y: 300.0,
        },
        SeedNode {
            key: "n-blocker",
            node_type: NodeType::Blocker,
            title: "Unbounded local permissions",
            summary: "Shell execution and code editing are out of MVP scope unless a future approval boundary is added.",
            status: NodeStatus::Infeasible,
            confidence: 0.88,
            x: 1080.0,
            y: 560.0,
        },
        SeedNode {
            key: "n-subdecision",
            node_type: NodeType::Subdecision,
            title: "Export scope",
            summary: "Exports must work for the whole graph and selected subgraphs.",
            status: NodeStatus::Draft,
            confidence: 0.66,
            x: 1440.0,
            y: 120.0,
        },
        SeedNode {
            key: "n-task",
            node_type: NodeType::Task,
            title: "First vertical slice",
            summary: "Create a group, inspect a node, leave comments, and export Markdown.",
            status: NodeStatus::Draft,
            confidence: 0.61,
            x: 1440.0,
            y: 380.0,
        },
        SeedNode {
            key: "n-artifact",
            node_type: NodeType::Artifact,
            title: "Derived artifacts",
            summary: "MADR, YADR, Mermaid, and image-generation prompts.",
            status: NodeStatus::Draft,
            confidence: 0.64,
            x: 1440.0,
            y: 640.0,
        },
    ]
}

fn seed_edges() -> [SeedEdge; 9] {
    [
        SeedEdge { key: "e1", edge_type: EdgeType::DecomposesTo, source: "n-proposition", target: "n-decision-points", label: "decide by" },
        SeedEdge { key: "e2", edge_type: EdgeType::ChoosesBetween, source: "n-decision-points", target: "n-option-graph", label: "recommended" },
        SeedEdge { key: "e3", edge_type: EdgeType::ChoosesBetween, source: "n-decision-points", target: "n-option-freeform", label: "alternative" },
        SeedEdge { key: "e4", edge_type: EdgeType::Supports, source: "n-evidence", target: "n-option-graph", label: "supports" },
        SeedEdge { key: "e5", edge_type: EdgeType::TradesOffWith, source: "n-option-graph", target: "n-tradeoff", label: "accepts" },
        SeedEdge { key: "e6", edge_type: EdgeType::Blocks, source: "n-blocker", target: "n-option-freeform", label: "weakens" },
        SeedEdge { key: "e7", edge_type: EdgeType::DecomposesTo, source: "n-option-graph", target: "n-subdecision", label: "needs" },
        SeedEdge { key: "e8", edge_type: EdgeType::DependsOn, source: "n-task", target: "n-option-graph", label: "builds on" },
        SeedEdge { key: "e9", edge_type: EdgeType::Produces, source: "n-subdecision", target: "n-artifact", label: "exports" },
    ]
}

/// Port of `titleFromPrompt`: first non-empty line, trimmed, capped at 70 chars
/// (TS uses `…` only when `> 70`, replacing the 67..70 slice with `...`).
pub fn title_from_prompt(prompt: &str) -> String {
    let first = prompt
        .split('\n')
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("Untitled group");
    let chars: Vec<char> = first.chars().collect();
    if chars.len() > 70 {
        let head: String = chars.iter().take(67).collect();
        format!("{head}...")
    } else {
        first.to_string()
    }
}

fn scoped(group_id: &str, key: &str) -> String {
    format!("{group_id}-{key}")
}

/// Port of `boundsForNodes`: padded AABB over the node frames, or a default frame
/// for an empty node set.
fn bounds_for_nodes(nodes: &[SceneNode]) -> Bounds {
    if nodes.is_empty() {
        return Bounds { x: 0.0, y: 0.0, width: 1200.0, height: 800.0 };
    }
    let min_x = nodes.iter().map(|n| n.position.x).fold(f64::INFINITY, f64::min);
    let min_y = nodes.iter().map(|n| n.position.y).fold(f64::INFINITY, f64::min);
    let max_x = nodes
        .iter()
        .map(|n| n.position.x + n.size.width)
        .fold(f64::NEG_INFINITY, f64::max);
    let max_y = nodes
        .iter()
        .map(|n| n.position.y + n.size.height)
        .fold(f64::NEG_INFINITY, f64::max);
    Bounds {
        x: min_x - BOUNDS_PADDING,
        y: min_y - BOUNDS_PADDING,
        width: max_x - min_x + BOUNDS_PADDING * 2.0,
        height: max_y - min_y + BOUNDS_PADDING * 2.0,
    }
}

/// The result of seeding: a [`ScenePatch`] the actor commits, the resulting group
/// (post-offset) for the HTTP response, and the explanation message.
pub struct SeededGroup {
    pub patch: ScenePatch,
    pub group: SceneGroup,
    pub message: String,
}

/// Build the FIXED seed scene for a new group, then offset it onto a free grid
/// cell relative to the existing top-level groups in `scene`. Mirrors the Node
/// `createGroup` (seedGroupScene + nextGroupOffset). `now` and `group_id` are
/// injected so the result is deterministic (no ambient time/randomness).
pub fn seed_group(
    scene: &Scene,
    group_id: &str,
    prompt: &str,
    title_override: Option<&str>,
    parent_group_id: Option<&str>,
    tag_ids: &[String],
    now: &str,
) -> SeededGroup {
    let title = title_from_prompt(prompt);

    // Nodes at their compact seed positions (pre-offset). The proposition node
    // carries the prompt-derived title; all others use their fixed labels.
    let base_nodes: Vec<SceneNode> = seed_nodes()
        .iter()
        .enumerate()
        .map(|(index, spec)| SceneNode {
            id: scoped(group_id, spec.key),
            node_type: spec.node_type,
            title: if spec.key == "n-proposition" {
                title.clone()
            } else {
                spec.title.to_string()
            },
            summary: spec.summary.to_string(),
            detail: spec.summary.to_string(),
            status: spec.status,
            confidence: spec.confidence,
            evidence_refs: vec![],
            child_decision_ids: vec![],
            group_id: group_id.to_string(),
            position: Point { x: spec.x, y: spec.y },
            size: Size { width: NODE_WIDTH, height: NODE_HEIGHT },
            z_index: index as f64,
            tag_ids: vec![],
            updated_at: Some(now.to_string()),
            meta: None,
        })
        .collect();

    let seed_bounds = bounds_for_nodes(&base_nodes);
    let offset = next_group_offset(scene, &seed_bounds);

    let nodes: Vec<SceneNode> = base_nodes
        .into_iter()
        .map(|mut n| {
            n.position = Point { x: n.position.x + offset.x, y: n.position.y + offset.y };
            n
        })
        .collect();

    let edges: Vec<SceneEdge> = seed_edges()
        .iter()
        .map(|spec| SceneEdge {
            id: scoped(group_id, spec.key),
            edge_type: spec.edge_type,
            source: scoped(group_id, spec.source),
            target: scoped(group_id, spec.target),
            label: spec.label.to_string(),
            rationale: spec.label.to_string(),
            confidence: 0.7,
            group_id: group_id.to_string(),
            tag_ids: vec![],
            updated_at: Some(now.to_string()),
            meta: None,
        })
        .collect();

    let group = SceneGroup {
        id: group_id.to_string(),
        parent_group_id: parent_group_id.map(|p| p.to_string()),
        title: title_override
            .filter(|t| !t.trim().is_empty())
            .map(|t| t.to_string())
            .unwrap_or(title),
        summary: prompt.to_string(),
        bounds: bounds_for_nodes(&nodes),
        tag_ids: tag_ids.to_vec(),
        z_index: 0.0,
        collapsed: false,
        created_at: now.to_string(),
        updated_at: now.to_string(),
        meta: None,
    };

    let patch = ScenePatch {
        groups: Some(vec![group.clone()]),
        nodes: Some(nodes),
        edges: Some(edges),
        selection: Some(shape_scene_core::SceneSelection::Group {
            id: group_id.to_string(),
        }),
        ..Default::default()
    };

    SeededGroup {
        patch,
        group,
        message: "Created a group on the infinite scene canvas.".to_string(),
    }
}

/// Port of `nextGroupOffset`: find a free grid cell for a group whose pre-offset
/// frame is `desired`, packing top-level groups in a square-ish grid and avoiding
/// overlap (with `COLLISION_PADDING` slack). Returns the (dx, dy) to translate the
/// seed scene by so the group lands on that cell.
fn next_group_offset(scene: &Scene, desired: &Bounds) -> Point {
    let groups: Vec<&SceneGroup> = scene
        .groups
        .iter()
        .filter(|g| g.parent_group_id.is_none())
        .collect();
    let cell_width = (desired.width + GROUP_GAP).max(1900.0);
    let cell_height = (desired.height + GROUP_GAP).max(1200.0);
    let columns = ((groups.len() + 1) as f64).sqrt().ceil().max(3.0) as usize;

    let max_attempts = (256).max((groups.len() + 1) * 4);
    for index in 0..max_attempts {
        let bounds = Bounds {
            x: (index % columns) as f64 * cell_width,
            y: (index / columns) as f64 * cell_height,
            width: desired.width,
            height: desired.height,
        };
        let free = groups.iter().all(|g| {
            overlap_area(&expand(&bounds, COLLISION_PADDING), &expand(&g.bounds, COLLISION_PADDING)) == 0.0
        });
        if free {
            return Point { x: bounds.x - desired.x, y: bounds.y - desired.y };
        }
    }
    let fallback = groups.len();
    Point {
        x: (fallback % columns) as f64 * cell_width - desired.x,
        y: (fallback / columns) as f64 * cell_height - desired.y,
    }
}

fn expand(b: &Bounds, padding: f64) -> Bounds {
    Bounds {
        x: b.x - padding,
        y: b.y - padding,
        width: b.width + padding * 2.0,
        height: b.height + padding * 2.0,
    }
}

fn overlap_area(a: &Bounds, b: &Bounds) -> f64 {
    let x = (a.x + a.width).min(b.x + b.width) - a.x.max(b.x);
    let y = (a.y + a.height).min(b.y + b.height) - a.y.max(b.y);
    x.max(0.0) * y.max(0.0)
}
