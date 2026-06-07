//! Operation enum — a faithful port of the `RenderScenePatch` / `ExtendedOpPatch`
//! unions from `src/shared/renderPatch.ts`.
//!
//! Every variant tag and field name is kept identical to the TS wire shape so a
//! patch JSON serialized by the shell deserializes here unchanged.

use serde::{Deserialize, Serialize};

use crate::model::{SceneSelection, Tag, WorldPoint, WorldRect};

// ---------------------------------------------------------------------------
// Render-projection payloads (renderScene.ts) carried by create ops.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderGroup {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub summary: String,
    pub bounds: WorldRect,
    #[serde(default)]
    pub tag_ids: Vec<String>,
    #[serde(default)]
    pub z_index: f64,
    #[serde(default)]
    pub style_key: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderCard {
    pub id: String,
    pub group_id: String,
    pub title: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub status: String,
    #[serde(rename = "type", default)]
    pub node_type: String,
    pub bounds: WorldRect,
    #[serde(default)]
    pub z_index: f64,
    #[serde(default)]
    pub style_key: String,
    #[serde(default)]
    pub accessibility_label: String,
}

// ---------------------------------------------------------------------------
// Small field enums.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EditField {
    Title,
    Summary,
    Detail,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Axis {
    X,
    Y,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlignMode {
    Start,
    Center,
    End,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetKind {
    Card,
    Edge,
    Frame,
}

// ---------------------------------------------------------------------------
// Core op union (the 22 kinds the renderer/scene-core apply path handles).
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum RenderScenePatch {
    #[serde(rename_all = "camelCase")]
    CreateGroup { group: RenderGroup },
    #[serde(rename_all = "camelCase")]
    DeleteGroup { id: String },
    #[serde(rename_all = "camelCase")]
    MoveGroup { id: String, delta: WorldPoint },
    #[serde(rename_all = "camelCase")]
    MoveCard { id: String, position: WorldPoint },
    #[serde(rename_all = "camelCase")]
    SetCardZIndex { id: String, z_index: f64 },
    #[serde(rename_all = "camelCase")]
    EditCardText {
        id: String,
        field: EditField,
        value: String,
    },
    #[serde(rename_all = "camelCase")]
    CreateCard { card: RenderCard },
    #[serde(rename_all = "camelCase")]
    DeleteCard { id: String },
    #[serde(rename_all = "camelCase")]
    CreateEdge {
        group_id: String,
        source: String,
        target: String,
        edge_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    DeleteEdge { id: String },
    #[serde(rename_all = "camelCase")]
    Select { selection: SceneSelection },
    #[serde(rename_all = "camelCase")]
    ResizeCard { id: String, bounds: WorldRect },
    #[serde(rename_all = "camelCase")]
    ResizeGroup { id: String, bounds: WorldRect },
    #[serde(rename_all = "camelCase")]
    AlignCards {
        ids: Vec<String>,
        axis: Axis,
        mode: AlignMode,
    },
    #[serde(rename_all = "camelCase")]
    DistributeCards { ids: Vec<String>, axis: Axis },
    #[serde(rename_all = "camelCase")]
    DuplicateObjects { ids: Vec<String>, delta: WorldPoint },
    #[serde(rename_all = "camelCase")]
    Batch { ops: Vec<RenderScenePatch> },
    #[serde(rename_all = "camelCase")]
    GroupObjects {
        ids: Vec<String>,
        frame_id: String,
        #[serde(default)]
        parent_group_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bounds: Option<WorldRect>,
    },
    #[serde(rename_all = "camelCase")]
    Ungroup { id: String },
    #[serde(rename_all = "camelCase")]
    SetObjectGroup { ids: Vec<String>, frame_id: String },
    #[serde(rename_all = "camelCase")]
    SetObjectTags {
        target_kind: TargetKind,
        id: String,
        tag_ids: Vec<String>,
    },
    #[serde(rename_all = "camelCase")]
    CreateTag { tag: Tag },
}

impl RenderScenePatch {
    /// The discriminant string, mirroring `patch.kind`.
    pub fn kind(&self) -> &'static str {
        match self {
            RenderScenePatch::CreateGroup { .. } => "create-group",
            RenderScenePatch::DeleteGroup { .. } => "delete-group",
            RenderScenePatch::MoveGroup { .. } => "move-group",
            RenderScenePatch::MoveCard { .. } => "move-card",
            RenderScenePatch::SetCardZIndex { .. } => "set-card-z-index",
            RenderScenePatch::EditCardText { .. } => "edit-card-text",
            RenderScenePatch::CreateCard { .. } => "create-card",
            RenderScenePatch::DeleteCard { .. } => "delete-card",
            RenderScenePatch::CreateEdge { .. } => "create-edge",
            RenderScenePatch::DeleteEdge { .. } => "delete-edge",
            RenderScenePatch::Select { .. } => "select",
            RenderScenePatch::ResizeCard { .. } => "resize-card",
            RenderScenePatch::ResizeGroup { .. } => "resize-group",
            RenderScenePatch::AlignCards { .. } => "align-cards",
            RenderScenePatch::DistributeCards { .. } => "distribute-cards",
            RenderScenePatch::DuplicateObjects { .. } => "duplicate-objects",
            RenderScenePatch::Batch { .. } => "batch",
            RenderScenePatch::GroupObjects { .. } => "group-objects",
            RenderScenePatch::Ungroup { .. } => "ungroup",
            RenderScenePatch::SetObjectGroup { .. } => "set-object-group",
            RenderScenePatch::SetObjectTags { .. } => "set-object-tags",
            RenderScenePatch::CreateTag { .. } => "create-tag",
        }
    }
}

// ---------------------------------------------------------------------------
// Extended op union (envelope-only; no apply path in scene-core today).
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ExtendedOpPatch {
    #[serde(rename_all = "camelCase")]
    AddComment {
        target: SceneSelection,
        body: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        author: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Export {
        scope_ids: Vec<String>,
        export_type: String,
    },
    #[serde(rename_all = "camelCase")]
    AcceptProposal { proposal_id: String },
    #[serde(rename_all = "camelCase")]
    RejectProposal { proposal_id: String },
}

/// Union of all patch kinds that can appear inside an `OperationEnvelope`.
/// Untagged: the inner enums are internally tagged on disjoint `kind` values,
/// so serde resolves the right side without an extra wrapper key.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ExtendedRenderPatch {
    Render(RenderScenePatch),
    Extended(ExtendedOpPatch),
}

impl From<RenderScenePatch> for ExtendedRenderPatch {
    fn from(p: RenderScenePatch) -> Self {
        ExtendedRenderPatch::Render(p)
    }
}

impl From<ExtendedOpPatch> for ExtendedRenderPatch {
    fn from(p: ExtendedOpPatch) -> Self {
        ExtendedRenderPatch::Extended(p)
    }
}
