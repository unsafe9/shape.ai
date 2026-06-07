//! Operation envelope + target-id derivation + in-memory op log.
//! Port of `src/shared/operation.ts`.

use serde::{Deserialize, Serialize};

use crate::model::SceneSelection;
use crate::op::{ExtendedOpPatch, ExtendedRenderPatch, RenderScenePatch};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActorType {
    Human,
    Mcp,
    System,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceToolCall {
    pub tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
}

/// Every canonical edit is wrapped in an envelope before commit. Carries
/// undo / MCP-trace / audit metadata without CRDT or transport behavior.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationEnvelope {
    pub operation_id: String,
    pub actor_id: String,
    pub actor_type: ActorType,
    pub client_id: String,
    pub target_ids: Vec<String>,
    pub timestamp: String,
    /// sceneVersion read BEFORE apply — the "authored against" revision.
    pub base_revision: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_tool_call: Option<SourceToolCall>,
    pub patch: ExtendedRenderPatch,
}

fn selection_target_ids(selection: &SceneSelection) -> Vec<String> {
    match selection {
        SceneSelection::Canvas => vec![],
        SceneSelection::Multi { ids } => ids.clone(),
        SceneSelection::Group { id }
        | SceneSelection::Node { id }
        | SceneSelection::Edge { id } => vec![id.clone()],
    }
}

fn derive_render_target_ids(patch: &RenderScenePatch) -> Vec<String> {
    match patch {
        RenderScenePatch::CreateCard { card } => vec![card.id.clone()],
        RenderScenePatch::CreateGroup { group } => vec![group.id.clone()],
        RenderScenePatch::CreateEdge {
            edge_id,
            source,
            target,
            ..
        } => vec![edge_id.clone(), source.clone(), target.clone()],
        RenderScenePatch::MoveCard { id, .. }
        | RenderScenePatch::SetCardZIndex { id, .. }
        | RenderScenePatch::EditCardText { id, .. }
        | RenderScenePatch::DeleteCard { id }
        | RenderScenePatch::MoveGroup { id, .. }
        | RenderScenePatch::DeleteGroup { id }
        | RenderScenePatch::DeleteEdge { id }
        | RenderScenePatch::ResizeCard { id, .. }
        | RenderScenePatch::ResizeGroup { id, .. }
        | RenderScenePatch::Ungroup { id }
        | RenderScenePatch::SetObjectTags { id, .. } => vec![id.clone()],
        RenderScenePatch::Select { selection } => selection_target_ids(selection),
        RenderScenePatch::AlignCards { ids, .. }
        | RenderScenePatch::DistributeCards { ids, .. }
        | RenderScenePatch::DuplicateObjects { ids, .. } => ids.clone(),
        RenderScenePatch::Batch { ops } => {
            ops.iter().flat_map(derive_render_target_ids).collect()
        }
        RenderScenePatch::GroupObjects { frame_id, ids, .. }
        | RenderScenePatch::SetObjectGroup { frame_id, ids } => {
            let mut out = vec![frame_id.clone()];
            out.extend(ids.iter().cloned());
            out
        }
        RenderScenePatch::CreateTag { tag } => vec![tag.id.clone()],
    }
}

fn derive_extended_target_ids(patch: &ExtendedOpPatch) -> Vec<String> {
    match patch {
        ExtendedOpPatch::AddComment { target, .. } => selection_target_ids(target),
        ExtendedOpPatch::Export { scope_ids, .. } => scope_ids.clone(),
        ExtendedOpPatch::AcceptProposal { proposal_id }
        | ExtendedOpPatch::RejectProposal { proposal_id } => vec![proposal_id.clone()],
    }
}

/// Derive the semantic target ids from a patch (T2.5 §2 derivation table).
pub fn derive_target_ids(patch: &ExtendedRenderPatch) -> Vec<String> {
    match patch {
        ExtendedRenderPatch::Render(p) => derive_render_target_ids(p),
        ExtendedRenderPatch::Extended(e) => derive_extended_target_ids(e),
    }
}

/// Synthesise a local-human envelope (the default in `apply_render_patch`).
///
/// `operation_id` is deterministic here (`op-{now}-local`). The TS source uses
/// `Math.random`; the real per-op id is `(clientId, localSeq)` assigned at the
/// transport boundary (MG4.2), so scene-core stays rng-free.
pub fn synthesise_local_envelope(
    patch: ExtendedRenderPatch,
    base_revision: i64,
    now: &str,
) -> OperationEnvelope {
    let target_ids = derive_target_ids(&patch);
    OperationEnvelope {
        operation_id: format!("op-{now}-local"),
        actor_id: "human".to_string(),
        actor_type: ActorType::Human,
        client_id: "local-shell".to_string(),
        target_ids,
        timestamp: now.to_string(),
        base_revision,
        source_tool_call: None,
        patch,
    }
}

// ---------------------------------------------------------------------------
// In-memory append-only operation log.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OperationLog {
    pub entries: Vec<OperationEnvelope>,
}

pub fn create_operation_log() -> OperationLog {
    OperationLog { entries: vec![] }
}

/// Immutable append — returns a new log, leaving the original untouched.
pub fn append_to_operation_log(log: &OperationLog, envelope: OperationEnvelope) -> OperationLog {
    let mut entries = log.entries.clone();
    entries.push(envelope);
    OperationLog { entries }
}

pub fn entries_by_actor<'a>(log: &'a OperationLog, actor_id: &str) -> Vec<&'a OperationEnvelope> {
    log.entries
        .iter()
        .filter(|e| e.actor_id == actor_id)
        .collect()
}

pub fn entries_by_target<'a>(log: &'a OperationLog, target_id: &str) -> Vec<&'a OperationEnvelope> {
    log.entries
        .iter()
        .filter(|e| e.target_ids.iter().any(|t| t == target_id))
        .collect()
}
