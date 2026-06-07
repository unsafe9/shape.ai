//! Scene document model — a faithful Rust port of `src/shared/schema.ts`.
//!
//! Serialization is kept byte-equivalent to the TS Zod model (camelCase field
//! names, the same optional/required fields, the same defaults) so the op-apply
//! port can be verified against golden vectors generated from the TS source.

use serde::{Deserialize, Serialize};

/// Free-form metadata bag (`z.record(string, unknown)`), preserved verbatim.
pub type ObjectMeta = serde_json::Map<String, serde_json::Value>;

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// `WorldPoint` / `WorldRect` (renderScene.ts) are structurally identical to
/// `Point` / `Bounds`; op payloads use these aliases.
pub type WorldPoint = Point;
pub type WorldRect = Bounds;

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeType {
    Proposition,
    DecisionPoint,
    Option,
    Evidence,
    Tradeoff,
    Blocker,
    Subdecision,
    Task,
    Artifact,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeType {
    DependsOn,
    Supports,
    Blocks,
    TradesOffWith,
    ChoosesBetween,
    DecomposesTo,
    Produces,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    Draft,
    Viable,
    Conditional,
    Infeasible,
    Unknown,
    Selected,
    Deferred,
    Complete,
}

impl Default for NodeStatus {
    fn default() -> Self {
        NodeStatus::Draft
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportType {
    Madr,
    Yadr,
    ImagePrompt,
    AiPlanMd,
    DesignDocMd,
    ConfluenceHtml,
    Mermaid,
    ArchitectureImage,
}

// ---------------------------------------------------------------------------
// Graph-level (placement-free) entities
// ---------------------------------------------------------------------------

fn default_version() -> i64 {
    1
}

fn default_confidence() -> f64 {
    0.5
}

fn default_size() -> Size {
    Size {
        width: 390.0,
        height: 390.0,
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphNode {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: NodeType,
    pub title: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub status: NodeStatus,
    #[serde(default = "default_confidence")]
    pub confidence: f64,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    #[serde(default)]
    pub child_decision_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdge {
    pub id: String,
    #[serde(rename = "type")]
    pub edge_type: EdgeType,
    pub source: String,
    pub target: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub rationale: String,
    #[serde(default = "default_confidence")]
    pub confidence: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecisionGraph {
    #[serde(default = "default_version")]
    pub version: i64,
    #[serde(default)]
    pub nodes: Vec<GraphNode>,
    #[serde(default)]
    pub edges: Vec<GraphEdge>,
}

// ---------------------------------------------------------------------------
// Scene-level entities
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tag {
    pub id: String,
    pub name: String,
    pub color: String,
    #[serde(default)]
    pub description: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneGroup {
    pub id: String,
    /// Always serialized (even as `null`), mirroring the TS `.nullable().default(null)`.
    #[serde(default)]
    pub parent_group_id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub summary: String,
    pub bounds: Bounds,
    #[serde(default)]
    pub tag_ids: Vec<String>,
    #[serde(default)]
    pub z_index: f64,
    #[serde(default)]
    pub collapsed: bool,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<ObjectMeta>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneNode {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: NodeType,
    pub title: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub status: NodeStatus,
    #[serde(default = "default_confidence")]
    pub confidence: f64,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    #[serde(default)]
    pub child_decision_ids: Vec<String>,
    pub group_id: String,
    pub position: Point,
    #[serde(default = "default_size")]
    pub size: Size,
    #[serde(default)]
    pub z_index: f64,
    #[serde(default)]
    pub tag_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<ObjectMeta>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneEdge {
    pub id: String,
    #[serde(rename = "type")]
    pub edge_type: EdgeType,
    pub source: String,
    pub target: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub rationale: String,
    #[serde(default = "default_confidence")]
    pub confidence: f64,
    pub group_id: String,
    #[serde(default)]
    pub tag_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<ObjectMeta>,
}

/// Discriminated selection union. `multi` is an ephemeral shell-only set; the
/// canonical persisted selection is single-anchor (see `primary_selection`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SceneSelection {
    Canvas,
    Group { id: String },
    Node { id: String },
    Edge { id: String },
    Multi { ids: Vec<String> },
}

impl Default for SceneSelection {
    fn default() -> Self {
        SceneSelection::Canvas
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneComment {
    pub id: String,
    pub target: SceneSelection,
    pub body: String,
    #[serde(default = "default_author")]
    pub author: String,
    #[serde(default)]
    pub resolved: bool,
    pub created_at: String,
    pub updated_at: String,
}

fn default_author() -> String {
    "human".to_string()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneArtifact {
    pub id: String,
    #[serde(rename = "type")]
    pub export_type: ExportType,
    pub title: String,
    pub target: SceneSelection,
    pub path: String,
    pub content_type: String,
    pub created_at: String,
    pub scene_version: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProposalStatus {
    Pending,
    Accepted,
    Rejected,
}

impl Default for ProposalStatus {
    fn default() -> Self {
        ProposalStatus::Pending
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneProposal {
    pub id: String,
    pub actor_id: String,
    #[serde(default)]
    pub status: ProposalStatus,
    #[serde(default)]
    pub operation: serde_json::Value,
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scene {
    #[serde(default = "default_version")]
    pub version: i64,
    #[serde(default)]
    pub scene_version: i64,
    #[serde(default)]
    pub groups: Vec<SceneGroup>,
    #[serde(default)]
    pub nodes: Vec<SceneNode>,
    #[serde(default)]
    pub edges: Vec<SceneEdge>,
    #[serde(default)]
    pub tags: Vec<Tag>,
    #[serde(default)]
    pub comments: Vec<SceneComment>,
    #[serde(default)]
    pub artifacts: Vec<SceneArtifact>,
    /// Additive optional field (no default) — omitted from output when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposals: Option<Vec<SceneProposal>>,
    #[serde(default)]
    pub selection: SceneSelection,
    pub updated_at: String,
}

// ---------------------------------------------------------------------------
// App scene patch (commit diff) — mirrors `scenePatchSchema`.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslateGroup {
    pub group_id: String,
    pub dx: f64,
    pub dy: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenePatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<SceneGroup>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nodes: Option<Vec<SceneNode>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edges: Option<Vec<SceneEdge>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translate_groups: Option<Vec<TranslateGroup>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remove_group_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remove_node_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remove_edge_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<SceneSelection>,
}

// ---------------------------------------------------------------------------
// Primitive-kind discriminator + primary-selection projection.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrimitiveKind {
    Shape,
    Text,
    Edge,
    Frame,
    ImageArtifact,
    CommentMarker,
}

/// Trait mirror of the TS `primitiveKind` overloads: each persisted object knows
/// its primitive kind structurally.
pub trait HasPrimitiveKind {
    fn primitive_kind(&self) -> PrimitiveKind;
}

impl HasPrimitiveKind for SceneGroup {
    fn primitive_kind(&self) -> PrimitiveKind {
        PrimitiveKind::Frame
    }
}
impl HasPrimitiveKind for SceneNode {
    fn primitive_kind(&self) -> PrimitiveKind {
        PrimitiveKind::Shape
    }
}
impl HasPrimitiveKind for SceneEdge {
    fn primitive_kind(&self) -> PrimitiveKind {
        PrimitiveKind::Edge
    }
}
impl HasPrimitiveKind for SceneComment {
    fn primitive_kind(&self) -> PrimitiveKind {
        PrimitiveKind::CommentMarker
    }
}
impl HasPrimitiveKind for SceneArtifact {
    fn primitive_kind(&self) -> PrimitiveKind {
        PrimitiveKind::ImageArtifact
    }
}

/// Convenience free function mirroring the TS call site.
pub fn primitive_kind<T: HasPrimitiveKind>(obj: &T) -> PrimitiveKind {
    obj.primitive_kind()
}

/// Down-project a (possibly `multi`) selection to its single-anchor primary.
pub fn primary_selection(selection: &SceneSelection) -> SceneSelection {
    match selection {
        SceneSelection::Multi { ids } => SceneSelection::Node {
            id: ids[0].clone(),
        },
        other => other.clone(),
    }
}
