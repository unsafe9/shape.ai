//! MG2.3 (Phase 3, C12): the shape.ai MCP server on the official Rust SDK
//! (`rmcp`).
//!
//! This is a thin transport/orchestration seam in front of the canvas actors and
//! scene-core graph helpers — exactly like the rest of `shape_server`. Each tool
//! is a faithful behavioural port of the Node `src/server/mcp.ts` tool of the
//! same name, but every scene mutation is funnelled through the per-canvas
//! [`crate::ActorHandle`] (which calls scene-core's `apply_render_patch_to_shape_scene`),
//! and every digest/export is produced by scene-core's `graph` helpers
//! (`scene_graph_for_group` / `graph_text_digest` / `make_mermaid`). No canvas
//! logic is reimplemented here.
//!
//! The tools are registered with rmcp's `#[tool_router]` / `#[tool]` macros and
//! served over the streamable-HTTP transport mounted at `/mcp` (see
//! [`crate::app`]). Tool handler bodies are plain `async fn`s on [`SceneMcp`], so
//! tests can call them directly without standing up the transport.
//!
//! Identity is `userId`-only with no auth (C13); MCP writes are attributed to the
//! actor `"mcp"`.
//! TODO(auth): real authn/authz attaches at the transport boundary.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeRequestParams, InitializeResult,
    ProtocolVersion, ServerCapabilities, ServerInfo,
};
use rmcp::service::RequestContext;
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, RoleServer, ServerHandler};
use serde::Deserialize;
use serde_json::json;

use shape_scene_core::op::TargetKind;
use shape_scene_core::{
    graph_text_digest, make_mermaid, scene_graph_for_group, CanvasId, ExportType, RenderGroup,
    RenderScenePatch, Scene, SceneSelection, Tag,
};

use crate::canvas_actor::{ActorHandle, ApplyResult, CommentResult};
use crate::mcp_clients::{ClientRegistry, TraceKind};
use crate::registry::CanvasRegistry;

/// The single canvas id every tool operates on for MG2.3 (canvas CRUD UI lands
/// in MG-9). Tools take an optional `canvasId` so the seam is already in place.
pub const DEFAULT_CANVAS_ID: &str = "default";

/// Process-local monotonic suffix so synthesized ids (groups, tags) stay unique
/// across rapid tool calls without scene-core needing randomness.
static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_suffix() -> String {
    let n = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{ms:x}{n:x}")
}

// ---------------------------------------------------------------------------
// Tool input parameter structs (schemars-derived JSON Schema for tools/list).
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct QuerySceneArgs {
    /// Optional group tag filter: only groups carrying every listed tag id.
    #[serde(default)]
    pub tag_ids: Option<Vec<String>>,
    /// Canvas to read (defaults to the single MG2.3 canvas).
    #[serde(default)]
    pub canvas_id: Option<String>,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CanvasOnlyArgs {
    #[serde(default)]
    pub canvas_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetGroupArgs {
    /// The group to read.
    pub group_id: String,
    #[serde(default)]
    pub canvas_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateGroupArgs {
    /// Prompt that seeds the new group's title.
    pub prompt: String,
    /// Optional explicit title (overrides the prompt-derived one).
    #[serde(default)]
    pub title: Option<String>,
    /// Optional parent group id for nesting.
    #[serde(default)]
    pub parent_group_id: Option<String>,
    /// Optional registered tag ids to attach.
    #[serde(default)]
    pub tag_ids: Option<Vec<String>>,
    #[serde(default)]
    pub canvas_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PatchSceneArgs {
    /// A canonical render patch (the same `RenderScenePatch` wire shape the shell
    /// emits): `{ "kind": "create-group", "group": { ... } }`, etc.
    pub patch: serde_json::Value,
    #[serde(default)]
    pub canvas_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateTagArgs {
    /// Display name for the tag.
    pub name: String,
    /// Tag colour (hex).
    pub color: String,
    /// Optional description.
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub canvas_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateGroupTagsArgs {
    /// The group whose tags to replace.
    pub group_id: String,
    /// The complete new set of registered tag ids.
    pub tag_ids: Vec<String>,
    #[serde(default)]
    pub canvas_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetSelectionArgs {
    /// The selection to set (`{ "kind": "group", "id": "..." }`, `{ "kind":
    /// "canvas" }`, etc.).
    pub selection: serde_json::Value,
    #[serde(default)]
    pub canvas_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AddCommentArgs {
    /// The comment target selection.
    pub target: serde_json::Value,
    /// Comment body (non-empty).
    pub body: String,
    /// Optional author label.
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub canvas_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExportGroupArgs {
    /// The group to export.
    pub group_id: String,
    /// One export format (`madr`, `markdown`, `yadr`, `mermaid`, …).
    #[serde(default)]
    pub r#type: Option<String>,
    /// Multiple export formats.
    #[serde(default)]
    pub types: Option<Vec<String>>,
    #[serde(default)]
    pub canvas_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetClientTraceArgs {
    /// The MCP client id whose trace to read.
    pub client_id: String,
    /// Max entries (default 50).
    #[serde(default)]
    pub limit: Option<usize>,
}

// ---------------------------------------------------------------------------
// Server state
// ---------------------------------------------------------------------------

/// The MCP server: a handle to the shared canvas registry + the dock client
/// registry, plus the per-session client id. Cloned per streamable-HTTP session
/// by the transport's service factory.
#[derive(Clone)]
pub struct SceneMcp {
    canvases: CanvasRegistry,
    clients: ClientRegistry,
    /// This session's stable client id (used for trace + colour). For tests this
    /// is just a fixed string.
    client_id: Arc<str>,
    // Read by the `#[tool_handler]`-generated dispatch (call_tool / list_tools);
    // dead-code analysis can't see through the macro, hence the allow.
    #[allow(dead_code)]
    tool_router: ToolRouter<SceneMcp>,
}

impl SceneMcp {
    /// Build a server instance bound to the shared registries with the given
    /// per-session client id.
    pub fn new(canvases: CanvasRegistry, clients: ClientRegistry, client_id: impl Into<Arc<str>>) -> Self {
        Self {
            canvases,
            clients,
            client_id: client_id.into(),
            tool_router: Self::tool_router(),
        }
    }

    /// This session's client id.
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    /// The registered tool definitions (name + input schema), as `tools/list`
    /// returns them. Exposed so tests and tooling can enumerate the tool set
    /// without standing up the transport.
    pub fn tool_definitions() -> Vec<rmcp::model::Tool> {
        Self::tool_router().list_all()
    }

    fn canvas(&self, canvas_id: &Option<String>) -> CanvasId {
        CanvasId::from(canvas_id.as_deref().unwrap_or(DEFAULT_CANVAS_ID))
    }

    /// Acquire the lease + spawn (or reuse) the actor for `canvas_id` (MG8.2a),
    /// surfacing a denied lease / draining registry as a tool error rather than
    /// panicking. Every tool that touches a canvas goes through here.
    async fn open_canvas(&self, canvas_id: &Option<String>) -> Result<ActorHandle, McpError> {
        self.canvases
            .get_or_spawn(&self.canvas(canvas_id))
            .await
            .map_err(|e| invalid_params(format!("cannot open canvas: {e}")))
    }

    fn trace(&self, kind: TraceKind, verb: &str, summary: String) {
        self.clients
            .push_trace(&self.client_id, kind, verb, summary, None);
    }

    fn trace_err(&self, verb: &str, message: &str) {
        self.clients.push_trace(
            &self.client_id,
            TraceKind::Error,
            verb,
            format!("error: {message}"),
            Some(message.to_string()),
        );
    }
}

/// Serialize any value to a one-content-block JSON tool result, matching the
/// Node `jsonResponse` shape (pretty-printed JSON text block).
fn json_response(value: serde_json::Value) -> CallToolResult {
    let text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
    CallToolResult::success(vec![Content::text(text)])
}

fn invalid_params(message: impl Into<String>) -> McpError {
    McpError::invalid_params(message.into(), None)
}

/// Apply the tag filter the Node `readScene` applies: keep groups carrying every
/// listed tag, then prune nodes/edges to the surviving groups.
fn filter_scene_by_tags(mut scene: Scene, tag_ids: &[String]) -> Scene {
    if tag_ids.is_empty() {
        return scene;
    }
    scene.groups.retain(|g| tag_ids.iter().all(|t| g.tag_ids.contains(t)));
    let group_ids: std::collections::HashSet<&String> = scene.groups.iter().map(|g| &g.id).collect();
    scene.nodes.retain(|n| group_ids.contains(&n.group_id));
    let node_ids: std::collections::HashSet<&String> = scene.nodes.iter().map(|n| &n.id).collect();
    scene
        .edges
        .retain(|e| group_ids.contains(&e.group_id) && node_ids.contains(&e.source) && node_ids.contains(&e.target));
    scene
}

/// Parse one MCP export-type token to a scene-core [`ExportType`], mirroring the
/// Node `normalizeExportType` (`markdown` → `madr`).
fn parse_export_type(raw: &str) -> Result<ExportType, McpError> {
    let v = match raw {
        "madr" | "markdown" => ExportType::Madr,
        "yadr" => ExportType::Yadr,
        "image_prompt" => ExportType::ImagePrompt,
        "ai_plan_md" => ExportType::AiPlanMd,
        "design_doc_md" => ExportType::DesignDocMd,
        "confluence_html" => ExportType::ConfluenceHtml,
        "mermaid" => ExportType::Mermaid,
        "architecture_image" => ExportType::ArchitectureImage,
        other => return Err(invalid_params(format!("unknown export type: {other}"))),
    };
    Ok(v)
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

#[tool_router]
impl SceneMcp {
    #[tool(
        description = "Query canonical scene data with optional group tag filters. Renderer-side culling owns viewport and zoom behavior."
    )]
    pub async fn query_scene(
        &self,
        Parameters(args): Parameters<QuerySceneArgs>,
    ) -> Result<CallToolResult, McpError> {
        let handle = self.open_canvas(&args.canvas_id).await?;
        let scene = handle.get_scene().await;
        let scene = filter_scene_by_tags(scene, args.tag_ids.as_deref().unwrap_or(&[]));
        self.trace(TraceKind::Read, "query_scene", "read scene".into());
        Ok(json_response(json!({ "scene": scene })))
    }

    #[tool(description = "List top-level and nested groups on the scene canvas.")]
    pub async fn list_groups(
        &self,
        Parameters(args): Parameters<CanvasOnlyArgs>,
    ) -> Result<CallToolResult, McpError> {
        let handle = self.open_canvas(&args.canvas_id).await?;
        let scene = handle.get_scene().await;
        let groups: Vec<serde_json::Value> = scene
            .groups
            .iter()
            .map(|group| {
                let nodes = scene.nodes.iter().filter(|n| n.group_id == group.id).count();
                let edges = scene.edges.iter().filter(|e| e.group_id == group.id).count();
                let artifacts = scene
                    .artifacts
                    .iter()
                    .filter(|a| matches!(&a.target, SceneSelection::Group { id } if id == &group.id))
                    .count();
                json!({
                    "id": group.id,
                    "parentGroupId": group.parent_group_id,
                    "title": group.title,
                    "summary": group.summary,
                    "bounds": group.bounds,
                    "tagIds": group.tag_ids,
                    "nodes": nodes,
                    "edges": edges,
                    "artifacts": artifacts,
                })
            })
            .collect();
        self.trace(TraceKind::Read, "list_groups", "listed groups".into());
        Ok(json_response(json!({ "groups": groups, "tags": scene.tags })))
    }

    #[tool(description = "Read one group with its nodes, edges, tags, comments, artifacts, and graph digest.")]
    pub async fn get_group(
        &self,
        Parameters(args): Parameters<GetGroupArgs>,
    ) -> Result<CallToolResult, McpError> {
        let handle = self.open_canvas(&args.canvas_id).await?;
        let scene = handle.get_scene().await;
        let Some(group) = scene.groups.iter().find(|g| g.id == args.group_id) else {
            self.trace_err("get_group", &format!("Group not found: {}", args.group_id));
            return Err(invalid_params(format!("Group not found: {}", args.group_id)));
        };
        let node_ids: std::collections::HashSet<&String> = scene
            .nodes
            .iter()
            .filter(|n| n.group_id == args.group_id)
            .map(|n| &n.id)
            .collect();
        let nodes: Vec<_> = scene.nodes.iter().filter(|n| n.group_id == args.group_id).cloned().collect();
        let edges: Vec<_> = scene
            .edges
            .iter()
            .filter(|e| e.group_id == args.group_id && node_ids.contains(&e.source) && node_ids.contains(&e.target))
            .cloned()
            .collect();
        let tags: Vec<_> = scene.tags.iter().filter(|t| group.tag_ids.contains(&t.id)).cloned().collect();
        let comments: Vec<_> = scene
            .comments
            .iter()
            .filter(|c| comment_targets_group(&c.target, &args.group_id, &node_ids))
            .cloned()
            .collect();
        let artifacts: Vec<_> = scene
            .artifacts
            .iter()
            .filter(|a| matches!(&a.target, SceneSelection::Group { id } if id == &args.group_id))
            .cloned()
            .collect();
        let graph = scene_graph_for_group(&scene, &args.group_id);
        let digest = graph_text_digest(&graph);
        self.trace(TraceKind::Read, "get_group", format!("read group {}", args.group_id));
        Ok(json_response(json!({
            "group": group,
            "nodes": nodes,
            "edges": edges,
            "tags": tags,
            "comments": comments,
            "artifacts": artifacts,
            "digest": digest,
        })))
    }

    #[tool(description = "Create a new group on the infinite scene canvas from a prompt.")]
    pub async fn create_group(
        &self,
        Parameters(args): Parameters<CreateGroupArgs>,
    ) -> Result<CallToolResult, McpError> {
        let handle = self.open_canvas(&args.canvas_id).await?;
        let group_id = format!("group-{}", unique_suffix());
        let title = args
            .title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| title_from_prompt(&args.prompt));
        let patch = RenderScenePatch::CreateGroup {
            group: RenderGroup {
                id: group_id.clone(),
                title,
                summary: args.prompt.clone(),
                // A sensible default frame; the shell repacks/relayouts on render.
                bounds: shape_scene_core::WorldRect {
                    x: 0.0,
                    y: 0.0,
                    width: 1900.0,
                    height: 1100.0,
                },
                tag_ids: args.tag_ids.clone().unwrap_or_default(),
                z_index: 0.0,
                style_key: String::new(),
            },
        };
        match handle.apply_patch(patch, "mcp").await {
            ApplyResult::Applied { .. } => {
                let scene = handle.get_scene().await;
                let group = scene.groups.iter().find(|g| g.id == group_id).cloned();
                self.trace(TraceKind::Write, "create_group", format!("created group {group_id}"));
                Ok(json_response(json!({ "group": group, "scene": scene })))
            }
            ApplyResult::Rejected { errors } => {
                let msg = errors.join("; ");
                self.trace_err("create_group", &msg);
                Err(invalid_params(msg))
            }
        }
    }

    #[tool(description = "Patch groups, nodes, edges, removals, or selection on the scene canvas.")]
    pub async fn patch_scene(
        &self,
        Parameters(args): Parameters<PatchSceneArgs>,
    ) -> Result<CallToolResult, McpError> {
        let patch: RenderScenePatch = serde_json::from_value(args.patch.clone())
            .map_err(|e| invalid_params(format!("invalid patch: {e}")))?;
        let verb = patch.kind();
        let handle = self.open_canvas(&args.canvas_id).await?;
        match handle.apply_patch(patch, "mcp").await {
            ApplyResult::Applied { .. } => {
                let scene = handle.get_scene().await;
                self.trace(TraceKind::Write, verb, format!("{verb} applied"));
                Ok(json_response(json!({ "scene": scene })))
            }
            ApplyResult::Rejected { errors } => {
                let msg = errors.join("; ");
                self.trace_err(verb, &msg);
                Err(invalid_params(msg))
            }
        }
    }

    #[tool(description = "Create a registered group tag with color and description.")]
    pub async fn create_tag(
        &self,
        Parameters(args): Parameters<CreateTagArgs>,
    ) -> Result<CallToolResult, McpError> {
        let tag_id = format!("tag-{}-{}", slug(&args.name), unique_suffix());
        let tag = Tag {
            id: tag_id.clone(),
            name: args.name.trim().to_string(),
            color: args.color.clone(),
            description: args.description.clone().unwrap_or_default(),
            // scene-core stamps updated_at on apply; created_at carries through.
            created_at: String::new(),
            updated_at: String::new(),
        };
        let handle = self.open_canvas(&args.canvas_id).await?;
        match handle.apply_patch(RenderScenePatch::CreateTag { tag }, "mcp").await {
            ApplyResult::Applied { .. } => {
                let scene = handle.get_scene().await;
                let created = scene.tags.iter().find(|t| t.id == tag_id).cloned();
                self.trace(TraceKind::Write, "create_tag", format!("created tag {tag_id}"));
                Ok(json_response(json!({ "tag": created, "scene": scene })))
            }
            ApplyResult::Rejected { errors } => {
                let msg = errors.join("; ");
                self.trace_err("create_tag", &msg);
                Err(invalid_params(msg))
            }
        }
    }

    #[tool(description = "Replace the registered tag ids attached to one group.")]
    pub async fn update_group_tags(
        &self,
        Parameters(args): Parameters<UpdateGroupTagsArgs>,
    ) -> Result<CallToolResult, McpError> {
        let handle = self.open_canvas(&args.canvas_id).await?;
        let patch = RenderScenePatch::SetObjectTags {
            target_kind: TargetKind::Frame,
            id: args.group_id.clone(),
            tag_ids: args.tag_ids.clone(),
        };
        match handle.apply_patch(patch, "mcp").await {
            ApplyResult::Applied { .. } => {
                let scene = handle.get_scene().await;
                let group = scene.groups.iter().find(|g| g.id == args.group_id).cloned();
                self.trace(TraceKind::Write, "update_group_tags", format!("retagged {}", args.group_id));
                Ok(json_response(json!({ "group": group, "scene": scene })))
            }
            ApplyResult::Rejected { errors } => {
                let msg = errors.join("; ");
                self.trace_err("update_group_tags", &msg);
                Err(invalid_params(msg))
            }
        }
    }

    #[tool(description = "Set the current Web UI scene selection.")]
    pub async fn set_selection(
        &self,
        Parameters(args): Parameters<SetSelectionArgs>,
    ) -> Result<CallToolResult, McpError> {
        let selection: SceneSelection = serde_json::from_value(args.selection.clone())
            .map_err(|e| invalid_params(format!("invalid selection: {e}")))?;
        let handle = self.open_canvas(&args.canvas_id).await?;
        match handle.apply_patch(RenderScenePatch::Select { selection }, "mcp").await {
            ApplyResult::Applied { .. } => {
                let scene = handle.get_scene().await;
                self.trace(TraceKind::Write, "set_selection", "set selection".into());
                Ok(json_response(json!({ "scene": scene })))
            }
            ApplyResult::Rejected { errors } => {
                let msg = errors.join("; ");
                self.trace_err("set_selection", &msg);
                Err(invalid_params(msg))
            }
        }
    }

    #[tool(description = "Add a comment to the canvas, a group, a node, or an edge.")]
    pub async fn add_comment(
        &self,
        Parameters(args): Parameters<AddCommentArgs>,
    ) -> Result<CallToolResult, McpError> {
        let target: SceneSelection = serde_json::from_value(args.target.clone())
            .map_err(|e| invalid_params(format!("invalid target: {e}")))?;
        let handle = self.open_canvas(&args.canvas_id).await?;
        match handle.add_comment(target, &args.body, "mcp").await {
            CommentResult::Added { comment } => {
                let scene = handle.get_scene().await;
                self.trace(TraceKind::Comment, "add_comment", "commented".into());
                Ok(json_response(json!({ "comment": comment, "scene": scene })))
            }
            CommentResult::Rejected { errors } => {
                let msg = errors.join("; ");
                self.trace_err("add_comment", &msg);
                Err(invalid_params(msg))
            }
        }
    }

    #[tool(
        description = "Generate text exports from a group's decision graph (madr/markdown/yadr/mermaid/ai_plan_md/design_doc_md/confluence_html/image_prompt). Content is returned inline."
    )]
    pub async fn export_group(
        &self,
        Parameters(args): Parameters<ExportGroupArgs>,
    ) -> Result<CallToolResult, McpError> {
        let mut types: Vec<ExportType> = Vec::new();
        if let Some(t) = &args.r#type {
            types.push(parse_export_type(t)?);
        }
        for t in args.types.clone().unwrap_or_default() {
            types.push(parse_export_type(&t)?);
        }
        types.dedup();
        if types.is_empty() {
            return Err(invalid_params("type or types is required"));
        }

        let handle = self.open_canvas(&args.canvas_id).await?;
        let scene = handle.get_scene().await;
        let Some(group) = scene.groups.iter().find(|g| g.id == args.group_id).cloned() else {
            self.trace_err("export_group", &format!("Group not found: {}", args.group_id));
            return Err(invalid_params(format!("Group not found: {}", args.group_id)));
        };
        let graph = scene_graph_for_group(&scene, &args.group_id);

        let exports: Vec<serde_json::Value> = types
            .iter()
            .map(|ty| {
                // Canvas logic stays in scene-core: digest/mermaid are the
                // ported renderers; the rest reuse the digest body for MG2.3
                // (full local-export formatting is an MG-7 cutover follow-up).
                let content = match ty {
                    ExportType::Mermaid | ExportType::ArchitectureImage => make_mermaid(&graph),
                    _ => graph_text_digest(&graph),
                };
                json!({
                    "type": export_type_token(*ty),
                    "title": format!("{} export", group.title),
                    "content": content,
                    "contentType": content_type_for(*ty),
                })
            })
            .collect();

        self.trace(TraceKind::Export, "export_group", format!("exported {}", args.group_id));
        let first = exports.first().cloned();
        Ok(json_response(json!({
            "group": group,
            "preview": first,
            "exports": exports,
        })))
    }

    #[tool(
        description = "Return the recent operation trace for a registered MCP client: read/write/comment/export events from the in-memory trace ring."
    )]
    pub async fn get_client_trace(
        &self,
        Parameters(args): Parameters<GetClientTraceArgs>,
    ) -> Result<CallToolResult, McpError> {
        let limit = args.limit.unwrap_or(50).clamp(1, 200);
        let trace = self.clients.trace(&args.client_id, limit);
        let total = trace.len();
        Ok(json_response(json!({
            "clientId": args.client_id,
            "trace": trace,
            "total": total,
        })))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn comment_targets_group(
    target: &SceneSelection,
    group_id: &str,
    node_ids: &std::collections::HashSet<&String>,
) -> bool {
    match target {
        SceneSelection::Group { id } => id == group_id,
        SceneSelection::Node { id } => node_ids.contains(id),
        _ => false,
    }
}

/// Derive a title from a prompt: the first non-empty line, trimmed and capped,
/// mirroring the spirit of the Node `titleFromPrompt`.
fn title_from_prompt(prompt: &str) -> String {
    let line = prompt.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    if line.is_empty() {
        return "Untitled group".to_string();
    }
    let mut t: String = line.chars().take(80).collect();
    if line.chars().count() > 80 {
        t.push('…');
    }
    t
}

/// Slugify a tag name like the Node `slug` (`[^a-z0-9]+` → `-`, trimmed).
fn slug(value: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in value.to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "tag".to_string()
    } else {
        trimmed
    }
}

fn export_type_token(ty: ExportType) -> &'static str {
    match ty {
        ExportType::Madr => "madr",
        ExportType::Yadr => "yadr",
        ExportType::ImagePrompt => "image_prompt",
        ExportType::AiPlanMd => "ai_plan_md",
        ExportType::DesignDocMd => "design_doc_md",
        ExportType::ConfluenceHtml => "confluence_html",
        ExportType::Mermaid => "mermaid",
        ExportType::ArchitectureImage => "architecture_image",
    }
}

fn content_type_for(ty: ExportType) -> &'static str {
    match ty {
        ExportType::Yadr => "application/yaml; charset=utf-8",
        ExportType::ConfluenceHtml => "text/html; charset=utf-8",
        ExportType::Mermaid => "text/plain; charset=utf-8",
        _ => "text/markdown; charset=utf-8",
    }
}

// ---------------------------------------------------------------------------
// ServerHandler — registers tools + advertises capabilities.
// ---------------------------------------------------------------------------

#[tool_handler]
impl ServerHandler for SceneMcp {
    fn get_info(&self) -> ServerInfo {
        // ServerInfo / Implementation are #[non_exhaustive]; construct via the
        // crate's Default + builder and mutate public fields rather than a literal.
        let mut info = ServerInfo::default();
        info.protocol_version = ProtocolVersion::V_2024_11_05;
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info = Implementation::new("shape.ai", env!("CARGO_PKG_VERSION"))
            .with_title("shape.ai");
        info.instructions = Some(
            "shape.ai canvas MCP. Read tools: query_scene, list_groups, get_group, \
             get_client_trace. Write tools: create_group, patch_scene, create_tag, \
             update_group_tags, set_selection, add_comment, export_group."
                .to_string(),
        );
        info
    }

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        // Mirror the Node `oninitialized` hook: register this session's companion
        // identity from the client's advertised Implementation so the dock can
        // track who is doing what.
        // TODO(auth): real authn/authz attaches the verified actor here.
        let info = &request.client_info;
        self.clients
            .register(&self.client_id, &info.name, &info.version, "http");
        Ok(self.get_info())
    }
}
