//! shape.ai shared scene core.
//!
//! Pure, platform-free logic shared by the client (wasm32) and server (native):
//! the scene document model, the operation enum, op-apply with per-property LWW,
//! the operation envelope, fractional indexing, templates, the wire protocol
//! serde, and the command catalog.
//!
//! Invariants (see CLAUDE.md + canvas-cockpit task breakdown):
//! - No ambient time, randomness, threads, or IO. Every such seam is an injected
//!   parameter (`now: &str`, an explicit operation id, etc.).
//! - The `Scene` model is byte-for-byte equivalent to the legacy TS `schema.ts`
//!   so the op-apply port is verified against golden vectors generated from TS.

pub mod apply;
pub mod canvas;
pub mod command;
pub mod envelope;
pub mod fractional;
pub mod graph;
pub mod lww;
pub mod model;
pub mod op;
pub mod primitive;
pub mod templates;
pub mod tool;
pub mod wire;

// MG0.3/MG0.4: the wasm-bindgen JS bridge. Gated so native builds/tests and a
// bare wasm32 check never pull wasm-bindgen; built on with `--features wasm`.
#[cfg(feature = "wasm")]
pub mod wasm_api;

pub use apply::{
    add_shape_scene_comment, apply_render_patch_to_shape_scene, apply_scene_patch,
    update_shape_scene_comment, update_shape_scene_group_tags, AppliedCommentUpdate,
    AppliedRenderPatch,
};
pub use envelope::{
    append_to_operation_log, create_operation_log, derive_target_ids, entries_by_actor,
    entries_by_target, synthesise_local_envelope, ActorType, OperationEnvelope, OperationLog,
    SourceToolCall,
};
pub use model::{
    primary_selection, primitive_kind, Bounds, DecisionGraph, EdgeType, GraphEdge, GraphNode,
    NodeStatus, NodeType, ObjectMeta, Point, PrimitiveKind, ProposalStatus, Scene, SceneArtifact,
    SceneComment, SceneEdge, SceneGroup, SceneNode, SceneProposal, SceneSelection, ScenePatch,
    Size, Tag, WorldPoint, WorldRect, ExportType,
};
pub use op::{ExtendedOpPatch, ExtendedRenderPatch, RenderCard, RenderGroup, RenderScenePatch};
pub use primitive::{insert_primitive_ops, InsertIds, PrimitiveSpec};
pub use tool::{default_tool, ActiveTool};
pub use canvas::{new_canvas, Canvas, CanvasId, CanvasSummary};
pub use command::{command_catalog, command_catalog_json, Command, CommandCategory};
pub use fractional::{cmp_keys, generate_key_between, generate_n_keys_between};
pub use graph::{
    apply_graph_patch, bounds_intersect, descendant_group_ids, edge_type_labels, expanded_bounds,
    export_type_labels, graph_text_digest, group_tags, make_mermaid, node_bounds, node_type_labels,
    point_in_bounds, scene_graph_for_group, selected_subgraph, selection_target, GraphPatch,
};
pub use lww::{
    lww_merge_property, validate_bounds_positive, validate_group_targets, validate_no_group_cycle,
    LwwEntry, LwwToken, PropertyStore,
};
pub use templates::{
    adr_template, apply_template, catalog, decision_map_template, dependency_diagram_template,
    idea_board_template, investigation_map_template, presentation_outline, presentation_style_tokens,
    presentation_template, recipe_from_selection, registry, server_architecture_template,
    todo_board_template, validate_presentation_meta, wiki_note_template, AppliedTemplate,
    RecipeEdge, RecipeFrame, RecipeLayout, RecipeShape, RecipeSize, SceneStyleToken, SuggestedTag,
    TemplateCategory, TemplateContract, TemplateExports, TemplateMetadata, TemplatePromptHints,
    TemplateRecipe, TemplateTags,
};
pub use wire::{Channel, ClientMessage, OpId, Region, ServerMessage, WireOp};
