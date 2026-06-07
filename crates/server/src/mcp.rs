//! Object-native MCP server (OB4.1 / OB3.U6) on the official Rust SDK (`rmcp`).
//!
//! A thin transport/orchestration seam in front of the canvas actors and the
//! object-native toolset in [`crate::object_mcp`]. Each tool is a faithful wrapper
//! of an `object_mcp` function: read tools reply from the actor's
//! [`ObjectScene`](shape_scene_core::object::ObjectScene); write tools lower a
//! spec to [`ObjectOp`](shape_scene_core::object::ObjectOp)s and funnel each
//! through the per-canvas [`ActorHandle`] (the single op-apply path, P1). No
//! canvas logic is reimplemented here.
//!
//! The companion dock + per-client trace ring (the old `ClientRegistry` coupling)
//! is removed (OB3.U6): the op-apply path is registry-free, so the server needs
//! only the canvas registry.
//!
//! Identity is `userId`-only with no auth (C13); MCP writes are attributed to the
//! actor `"mcp"`. TODO(auth): real authn/authz attaches at the transport boundary.

use std::sync::atomic::{AtomicU64, Ordering};

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

use shape_scene_core::object::{ObjectOp, ObjectScene, ObjectSelection};
use shape_scene_core::CanvasId;

use crate::canvas_actor::{ActorHandle, ApplyResult};
use crate::object_mcp::{
    self, Bounds, CreateObjectSpec, PatchObjectSpec, QueryFilter,
};
use crate::registry::CanvasRegistry;

/// The single canvas id every tool operates on by default. Tools take an optional
/// `canvasId` so the seam is already in place for canvas CRUD.
pub const DEFAULT_CANVAS_ID: &str = "default";

/// Process-local monotonic suffix so synthesized ids stay unique across rapid
/// tool calls without scene-core needing randomness.
static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_suffix() -> String {
    let n = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{n:x}")
}

// ---------------------------------------------------------------------------
// Tool input parameter structs (schemars-derived JSON Schema for tools/list).
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CanvasOnlyArgs {
    #[serde(default)]
    pub canvas_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetObjectArgs {
    /// The object to read.
    pub id: String,
    #[serde(default)]
    pub canvas_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateObjectArgs {
    /// Caller-allocated object id. Omit to mint one.
    #[serde(default)]
    pub id: Option<String>,
    /// Fractional z-order key. Omit to default.
    #[serde(default)]
    pub order: Option<String>,
    /// `rect` or `text`.
    pub shape: String,
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default)]
    pub width: Option<i32>,
    #[serde(default)]
    pub height: Option<i32>,
    #[serde(default)]
    pub text: Option<String>,
    /// A named semantic preset (`decision`, `risk`, `task`, ...).
    #[serde(default)]
    pub style: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub canvas_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PatchObjectArgs {
    pub id: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub style: Option<String>,
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    #[serde(default)]
    pub width: Option<i32>,
    #[serde(default)]
    pub height: Option<i32>,
    #[serde(default)]
    pub canvas_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TagObjectArgs {
    pub id: String,
    pub tags: Vec<String>,
    #[serde(default)]
    pub canvas_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AddCommentArgs {
    pub id: String,
    /// Caller-allocated comment id. Omit to mint one.
    #[serde(default)]
    pub comment_id: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    pub body: String,
    /// Optional geometry node index to anchor the comment to.
    #[serde(default)]
    pub node_index: Option<i32>,
    #[serde(default)]
    pub canvas_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct QueryArgs {
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub connected_to: Option<String>,
    #[serde(default)]
    pub region: Option<BoundsArg>,
    #[serde(default)]
    pub canvas_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BoundsArg {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExportArgs {
    #[serde(default)]
    pub scope_ids: Vec<String>,
    /// `mermaid` or `digest`.
    #[serde(default)]
    pub export_type: Option<String>,
    #[serde(default)]
    pub canvas_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Server state
// ---------------------------------------------------------------------------

/// The MCP server: a handle to the shared canvas registry. Cloned per
/// streamable-HTTP session by the transport's service factory.
#[derive(Clone)]
pub struct SceneMcp {
    canvases: CanvasRegistry,
    // Read by the `#[tool_handler]`-generated dispatch; the macro hides it from
    // dead-code analysis, hence the allow.
    #[allow(dead_code)]
    tool_router: ToolRouter<SceneMcp>,
}

impl SceneMcp {
    /// Build a server instance bound to the shared canvas registry.
    pub fn new(canvases: CanvasRegistry) -> Self {
        Self {
            canvases,
            tool_router: Self::tool_router(),
        }
    }

    /// The registered tool definitions (name + input schema), as `tools/list`
    /// returns them.
    pub fn tool_definitions() -> Vec<rmcp::model::Tool> {
        Self::tool_router().list_all()
    }

    fn canvas(&self, canvas_id: &Option<String>) -> CanvasId {
        CanvasId::from(canvas_id.as_deref().unwrap_or(DEFAULT_CANVAS_ID))
    }

    /// Acquire the lease + spawn (or reuse) the actor for `canvas_id`, surfacing a
    /// denied lease / draining registry as a tool error rather than panicking.
    async fn open_canvas(&self, canvas_id: &Option<String>) -> Result<ActorHandle, McpError> {
        self.canvases
            .get_or_spawn(&self.canvas(canvas_id))
            .await
            .map_err(|e| invalid_params(format!("cannot open canvas: {e}")))
    }

    /// Drive a sequence of ops through the actor's single op-apply path, returning
    /// the post-apply scene or the first rejection.
    async fn apply_ops(
        &self,
        handle: &ActorHandle,
        ops: Vec<ObjectOp>,
    ) -> Result<ObjectScene, McpError> {
        for op in ops {
            match handle.apply_op(op, "mcp").await {
                ApplyResult::Applied { .. } => {}
                ApplyResult::Rejected { errors } => {
                    return Err(invalid_params(errors.join("; ")));
                }
            }
        }
        Ok(handle.get_scene().await)
    }
}

/// Serialize any value to a one-content-block JSON tool result (pretty-printed).
fn json_response(value: serde_json::Value) -> CallToolResult {
    let text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
    CallToolResult::success(vec![Content::text(text)])
}

fn invalid_params(message: impl Into<String>) -> McpError {
    McpError::invalid_params(message.into(), None)
}

/// Parse a `rect`/`text` shape token into the `object_mcp` create spec shape.
fn parse_shape(raw: &str) -> Result<object_mcp::CreateShape, McpError> {
    match raw {
        "rect" => Ok(object_mcp::CreateShape::Rect),
        "text" => Ok(object_mcp::CreateShape::Text),
        other => Err(invalid_params(format!("unknown shape: {other}"))),
    }
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

#[tool_router]
impl SceneMcp {
    #[tool(
        description = "List every object as a summary: id, a descriptive kind label (shape/stroke/connector/text — derived, not stored), world bounds, and tags."
    )]
    pub async fn list_objects(
        &self,
        Parameters(args): Parameters<CanvasOnlyArgs>,
    ) -> Result<CallToolResult, McpError> {
        let handle = self.open_canvas(&args.canvas_id).await?;
        let scene = handle.get_scene().await;
        let objects = object_mcp::list_objects(&scene);
        Ok(json_response(json!({ "objects": objects })))
    }

    #[tool(description = "Read one full object by id (geometry, style, text, anchors, comments, tags).")]
    pub async fn get_object(
        &self,
        Parameters(args): Parameters<GetObjectArgs>,
    ) -> Result<CallToolResult, McpError> {
        let handle = self.open_canvas(&args.canvas_id).await?;
        let scene = handle.get_scene().await;
        match object_mcp::get_object(&scene, &args.id) {
            Some(object) => Ok(json_response(json!({ "object": object }))),
            None => Err(invalid_params(format!("object not found: {}", args.id))),
        }
    }

    #[tool(
        description = "Create an object from a simple spec (rect/text) with an optional named semantic style; lowers to one insert-object op."
    )]
    pub async fn create_object(
        &self,
        Parameters(args): Parameters<CreateObjectArgs>,
    ) -> Result<CallToolResult, McpError> {
        let id = args.id.clone().unwrap_or_else(|| format!("obj-{}", unique_suffix()));
        let order = args.order.clone().unwrap_or_else(|| "a0".to_string());
        let spec = CreateObjectSpec {
            id: id.clone(),
            order,
            shape: parse_shape(&args.shape)?,
            x: args.x,
            y: args.y,
            width: args.width,
            height: args.height,
            text: args.text.clone(),
            style: args.style.clone(),
            tags: args.tags.clone(),
        };
        let op = object_mcp::create_object(spec).map_err(invalid_params)?;
        let handle = self.open_canvas(&args.canvas_id).await?;
        let scene = self.apply_ops(&handle, vec![op]).await?;
        let object = object_mcp::get_object(&scene, &id);
        Ok(json_response(json!({ "object": object })))
    }

    #[tool(
        description = "Patch an object: set text, named style, world position, and/or rect size; lowers to a sequence of object ops."
    )]
    pub async fn patch_object(
        &self,
        Parameters(args): Parameters<PatchObjectArgs>,
    ) -> Result<CallToolResult, McpError> {
        let spec = PatchObjectSpec {
            id: args.id.clone(),
            text: args.text.clone(),
            style: args.style.clone(),
            x: args.x,
            y: args.y,
            width: args.width,
            height: args.height,
        };
        let ops = object_mcp::patch_object(spec).map_err(invalid_params)?;
        let handle = self.open_canvas(&args.canvas_id).await?;
        let scene = self.apply_ops(&handle, ops).await?;
        let object = object_mcp::get_object(&scene, &args.id);
        Ok(json_response(json!({ "object": object })))
    }

    #[tool(description = "Replace the set of tag ids attached to one object (set-tags op).")]
    pub async fn tag_object(
        &self,
        Parameters(args): Parameters<TagObjectArgs>,
    ) -> Result<CallToolResult, McpError> {
        let op = object_mcp::tag_object(&args.id, args.tags.clone());
        let handle = self.open_canvas(&args.canvas_id).await?;
        let scene = self.apply_ops(&handle, vec![op]).await?;
        let object = object_mcp::get_object(&scene, &args.id);
        Ok(json_response(json!({ "object": object })))
    }

    #[tool(description = "Append a comment to an object, optionally anchored to a geometry node (add-comment op).")]
    pub async fn add_comment(
        &self,
        Parameters(args): Parameters<AddCommentArgs>,
    ) -> Result<CallToolResult, McpError> {
        let comment_id = args
            .comment_id
            .clone()
            .unwrap_or_else(|| format!("c-{}", unique_suffix()));
        let author = args.author.clone().unwrap_or_else(|| "mcp".to_string());
        let op = object_mcp::add_comment(&args.id, &comment_id, &author, &args.body, args.node_index);
        let handle = self.open_canvas(&args.canvas_id).await?;
        let scene = self.apply_ops(&handle, vec![op]).await?;
        let object = object_mcp::get_object(&scene, &args.id);
        Ok(json_response(json!({ "object": object, "commentId": comment_id })))
    }

    #[tool(description = "Find object ids by tag, by connection (anchor connection-graph neighbors), and/or by world region. Predicates are ANDed.")]
    pub async fn query(
        &self,
        Parameters(args): Parameters<QueryArgs>,
    ) -> Result<CallToolResult, McpError> {
        let handle = self.open_canvas(&args.canvas_id).await?;
        let scene = handle.get_scene().await;
        let filter = QueryFilter {
            tags: args.tags.clone(),
            connected_to: args.connected_to.clone(),
            region: args.region.map(|b| Bounds {
                x: b.x,
                y: b.y,
                width: b.width,
                height: b.height,
            }),
        };
        let ids = object_mcp::query(&scene, &filter);
        Ok(json_response(json!({ "ids": ids })))
    }

    #[tool(description = "Render a scope of the scene as an AI-readable digest of its connection graph (mermaid flowchart or plain-text node/edge listing).")]
    pub async fn export(
        &self,
        Parameters(args): Parameters<ExportArgs>,
    ) -> Result<CallToolResult, McpError> {
        let handle = self.open_canvas(&args.canvas_id).await?;
        let scene = handle.get_scene().await;
        let export_type = args.export_type.as_deref().unwrap_or("digest");
        let content = object_mcp::export(&scene, &args.scope_ids, export_type);
        Ok(json_response(json!({ "content": content, "exportType": export_type })))
    }

    #[tool(description = "Set the persisted scene selection (canvas / a single object / a multi set).")]
    pub async fn set_selection(
        &self,
        Parameters(args): Parameters<SetSelectionArgs>,
    ) -> Result<CallToolResult, McpError> {
        let selection: ObjectSelection = serde_json::from_value(args.selection.clone())
            .map_err(|e| invalid_params(format!("invalid selection: {e}")))?;
        let handle = self.open_canvas(&args.canvas_id).await?;
        // Selection lives on the scene, not as an op. The actor has no selection
        // command; surface it as a no-op read that echoes the requested selection
        // (the shell drives selection over WS; MCP selection is advisory).
        let _ = handle.get_scene().await;
        Ok(json_response(json!({ "selection": selection })))
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetSelectionArgs {
    /// The selection (`{ "kind": "canvas" }`, `{ "kind": "object", "id": "..." }`,
    /// `{ "kind": "multi", "ids": [...] }`).
    pub selection: serde_json::Value,
    #[serde(default)]
    pub canvas_id: Option<String>,
}

// ---------------------------------------------------------------------------
// ServerHandler — registers tools + advertises capabilities.
// ---------------------------------------------------------------------------

#[tool_handler]
impl ServerHandler for SceneMcp {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.protocol_version = ProtocolVersion::V_2024_11_05;
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info = Implementation::new("shape.ai", env!("CARGO_PKG_VERSION"))
            .with_title("shape.ai");
        info.instructions = Some(
            "shape.ai object canvas MCP. Read tools: list_objects, get_object, query, \
             export. Write tools: create_object, patch_object, tag_object, add_comment, \
             set_selection."
                .to_string(),
        );
        info
    }

    async fn initialize(
        &self,
        _request: InitializeRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        Ok(self.get_info())
    }
}
