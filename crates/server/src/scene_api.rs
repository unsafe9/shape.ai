//! MG-7 Phase A — the legacy Node `/api/*` scene routes, ported onto the Rust
//! server so the web client's relative `/api` calls (src/client/lib/api.ts) are
//! served entirely by `shape_server`.
//!
//! Every mutation goes through the per-canvas [`ActorHandle`] (scene-core apply +
//! persistence + broadcast); reads go through `get_scene`. Response shapes match
//! the Node routes byte-for-byte (camelCase JSON) so the Svelte shell needs no
//! change. Identity is `userId`-only / no-auth (C13); writes are attributed to the
//! actor `"http"`.

use std::path::PathBuf;

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::{Json, Router};
use axum::routing::{get, patch, post};
use serde::Deserialize;
use serde_json::{json, Value};

use shape_scene_core::{
    scene_graph_for_group, CanvasId, ExportType, RenderScenePatch, Scene, SceneArtifact,
    ScenePatch, SceneSelection, Tag,
};

use crate::canvas_actor::{ActorHandle, ApplyResult, ArtifactResult, CommentResult};
use crate::local_export::{content_type_for, generate_local_export, ExportOutput, ExportScope};
use crate::mcp::DEFAULT_CANVAS_ID;
use crate::registry::CanvasRegistry;

/// Deterministic timestamp injected into scene-core ops authored by the HTTP
/// layer. The actor stamps its own `now` for applied ops; this value is only used
/// where a route builds a scene-core artifact directly. Kept fixed (scene-core is
/// time-free) like the actor's own `now`.
const HTTP_NOW: &str = "1970-01-01T00:00:00Z";

/// Shared state for the scene API: the canvas registry (single default canvas for
/// MG-7) plus the resolved exports directory for written artifacts.
#[derive(Clone)]
pub struct SceneApiState {
    pub canvases: CanvasRegistry,
    pub exports_dir: PathBuf,
}

/// Build the scene API sub-router. Mounted into the main router by
/// [`crate::build_router_with_mcp`].
pub fn scene_api_router(state: SceneApiState) -> Router {
    Router::new()
        .route("/api/scene", get(get_scene).patch(patch_scene))
        .route("/api/groups", post(create_group))
        .route("/api/groups/:id/tags", patch(update_group_tags))
        .route("/api/groups/:id/export", post(export_group))
        .route(
            "/api/groups/:groupId/artifacts/:artifactId",
            get(download_artifact),
        )
        .route("/api/tags", post(create_tag))
        .route("/api/tags/:id", patch(update_tag).delete(delete_tag))
        .route("/api/comments", post(create_comment))
        .route("/api/comments/:id", patch(update_comment))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// shared helpers
// ---------------------------------------------------------------------------

fn default_canvas() -> CanvasId {
    CanvasId::from(DEFAULT_CANVAS_ID)
}

async fn open_default(state: &SceneApiState) -> Result<ActorHandle, ApiError> {
    state
        .canvases
        .get_or_spawn(&default_canvas())
        .await
        .map_err(|e| ApiError::internal(format!("cannot open canvas: {e}")))
}

/// Apply the AND-tag filter the Node `readScene` applies: keep groups carrying
/// every listed tag, then prune nodes/edges to the surviving groups.
fn filter_scene_by_tags(mut scene: Scene, tag_ids: &[String]) -> Scene {
    if tag_ids.is_empty() {
        return scene;
    }
    scene
        .groups
        .retain(|g| tag_ids.iter().all(|t| g.tag_ids.contains(t)));
    let group_ids: std::collections::HashSet<&String> =
        scene.groups.iter().map(|g| &g.id).collect();
    scene.nodes.retain(|n| group_ids.contains(&n.group_id));
    let node_ids: std::collections::HashSet<&String> = scene.nodes.iter().map(|n| &n.id).collect();
    scene.edges.retain(|e| {
        group_ids.contains(&e.group_id)
            && node_ids.contains(&e.source)
            && node_ids.contains(&e.target)
    });
    scene
}

/// A JSON error envelope matching the Node error shape (`{ error, message }`).
struct ApiError {
    status: StatusCode,
    error: &'static str,
    message: String,
}

impl ApiError {
    fn internal(message: impl Into<String>) -> Self {
        ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            error: "internal_error",
            message: message.into(),
        }
    }
    fn not_found(message: impl Into<String>) -> Self {
        ApiError {
            status: StatusCode::NOT_FOUND,
            error: "not_found",
            message: message.into(),
        }
    }
    fn forbidden(message: impl Into<String>) -> Self {
        ApiError {
            status: StatusCode::FORBIDDEN,
            error: "forbidden",
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (
            self.status,
            Json(json!({ "error": self.error, "message": self.message })),
        )
            .into_response()
    }
}

// ---------------------------------------------------------------------------
// GET /api/scene
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct SceneQuery {
    tags: Option<String>,
}

/// `GET /api/scene?tags=a,b` -> `{ scene }` with server-side AND-tag filtering.
async fn get_scene(
    State(state): State<SceneApiState>,
    Query(query): Query<SceneQuery>,
) -> Result<Json<Value>, ApiError> {
    let handle = open_default(&state).await?;
    let scene = handle.get_scene().await;
    let tag_ids: Vec<String> = query
        .tags
        .as_deref()
        .map(|raw| {
            raw.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let scene = filter_scene_by_tags(scene, &tag_ids);
    Ok(Json(json!({ "scene": scene })))
}

// ---------------------------------------------------------------------------
// PATCH /api/scene
// ---------------------------------------------------------------------------

/// `PATCH /api/scene` body `ScenePatch` -> `{ scene }`. Applies a bulk patch
/// through the actor (scene-core `apply_scene_patch`).
async fn patch_scene(
    State(state): State<SceneApiState>,
    Json(patch): Json<ScenePatch>,
) -> Result<Json<Value>, ApiError> {
    let handle = open_default(&state).await?;
    match handle.apply_scene_patch(patch, "http").await {
        ApplyResult::Applied { .. } => {
            let scene = handle.get_scene().await;
            Ok(Json(json!({ "scene": scene })))
        }
        ApplyResult::Rejected { errors } => Err(ApiError::internal(errors.join("; "))),
    }
}

// ---------------------------------------------------------------------------
// POST /api/groups
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateGroupBody {
    prompt: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    parent_group_id: Option<String>,
    #[serde(default)]
    tag_ids: Option<Vec<String>>,
}

/// `POST /api/groups` -> `{ group, scene, message }`. Seeds the FIXED 10-node /
/// 9-edge decision graph (group_seed) onto a free grid cell, applies it through
/// the actor, then returns the persisted group.
async fn create_group(
    State(state): State<SceneApiState>,
    Json(body): Json<CreateGroupBody>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let handle = open_default(&state).await?;
    let scene = handle.get_scene().await;

    // Validate referenced tags exist (mirrors the Node `assertTagsExist`).
    let tag_ids = body.tag_ids.clone().unwrap_or_default();
    let known: std::collections::HashSet<&String> = scene.tags.iter().map(|t| &t.id).collect();
    if let Some(unknown) = tag_ids.iter().find(|t| !known.contains(t)) {
        return Err(ApiError::internal(format!("Tag not found: {unknown}")));
    }

    let group_id = format!("group-{}", group_suffix(&scene));
    let seeded = crate::group_seed::seed_group(
        &scene,
        &group_id,
        &body.prompt,
        body.title.as_deref(),
        body.parent_group_id.as_deref(),
        &tag_ids,
        HTTP_NOW,
    );

    match handle.apply_scene_patch(seeded.patch, "http").await {
        ApplyResult::Applied { .. } => {
            let scene = handle.get_scene().await;
            let group = scene
                .groups
                .iter()
                .find(|g| g.id == group_id)
                .cloned()
                .unwrap_or(seeded.group);
            Ok((
                StatusCode::CREATED,
                Json(json!({ "group": group, "scene": scene, "message": seeded.message })),
            ))
        }
        ApplyResult::Rejected { errors } => Err(ApiError::internal(errors.join("; "))),
    }
}

/// A process-unique-enough id suffix for a new group, derived from the scene size
/// + a monotonic counter (scene-core stays randomness-free).
fn group_suffix(scene: &Scene) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{:x}{:x}", scene.groups.len(), n)
}

// ---------------------------------------------------------------------------
// Tags
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CreateTagBody {
    name: String,
    color: String,
    #[serde(default)]
    description: Option<String>,
}

/// `POST /api/tags` -> `{ tag, scene }`.
async fn create_tag(
    State(state): State<SceneApiState>,
    Json(body): Json<CreateTagBody>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let handle = open_default(&state).await?;
    let scene = handle.get_scene().await;
    let tag_id = format!("tag-{}-{}", slug(&body.name), tag_suffix(&scene));
    let tag = Tag {
        id: tag_id.clone(),
        name: body.name.trim().to_string(),
        color: body.color.clone(),
        description: body.description.clone().unwrap_or_default(),
        created_at: HTTP_NOW.to_string(),
        updated_at: HTTP_NOW.to_string(),
    };
    match handle.apply_patch(RenderScenePatch::CreateTag { tag }, "http").await {
        ApplyResult::Applied { .. } => {
            let scene = handle.get_scene().await;
            let created = scene.tags.iter().find(|t| t.id == tag_id).cloned();
            Ok((StatusCode::CREATED, Json(json!({ "tag": created, "scene": scene }))))
        }
        ApplyResult::Rejected { errors } => Err(ApiError::internal(errors.join("; "))),
    }
}

#[derive(Debug, Deserialize)]
struct UpdateTagBody {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

/// `PATCH /api/tags/:id` -> `{ tag, scene }`. scene-core has no update-tag op, so
/// this re-creates the tag in place (read-modify-rewrite) through a bulk patch is
/// not available for tags; instead it builds the next scene via a delete+create
/// equivalent. To stay within scene-core's op set we read the tag, mutate it, and
/// write it back via a direct scene replace on the actor.
async fn update_tag(
    State(state): State<SceneApiState>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<UpdateTagBody>,
) -> Result<Json<Value>, ApiError> {
    let handle = open_default(&state).await?;
    let scene = handle.get_scene().await;
    let Some(current) = scene.tags.iter().find(|t| t.id == id).cloned() else {
        return Err(ApiError::internal(format!("Tag not found: {id}")));
    };
    let updated = Tag {
        id: current.id.clone(),
        name: body.name.unwrap_or(current.name),
        color: body.color.unwrap_or(current.color),
        description: body.description.unwrap_or(current.description),
        created_at: current.created_at,
        updated_at: HTTP_NOW.to_string(),
    };
    match handle.update_tag(updated).await {
        ApplyResult::Applied { .. } => {
            let scene = handle.get_scene().await;
            let tag = scene.tags.iter().find(|t| t.id == id).cloned();
            Ok(Json(json!({ "tag": tag, "scene": scene })))
        }
        ApplyResult::Rejected { errors } => Err(ApiError::internal(errors.join("; "))),
    }
}

/// `DELETE /api/tags/:id` -> `{ scene }`. Refuses a tag still attached to a group
/// (mirrors `deleteUnusedTag`).
async fn delete_tag(
    State(state): State<SceneApiState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, ApiError> {
    let handle = open_default(&state).await?;
    let scene = handle.get_scene().await;
    let in_use = scene.groups.iter().any(|g| g.tag_ids.contains(&id));
    if in_use {
        return Err(ApiError::internal(
            "Cannot delete a tag that is still attached to groups".to_string(),
        ));
    }
    match handle.delete_tag(&id).await {
        ApplyResult::Applied { .. } => {
            let scene = handle.get_scene().await;
            Ok(Json(json!({ "scene": scene })))
        }
        ApplyResult::Rejected { errors } => Err(ApiError::internal(errors.join("; "))),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateGroupTagsBody {
    tag_ids: Vec<String>,
}

/// `PATCH /api/groups/:id/tags` -> `{ group, scene }`.
async fn update_group_tags(
    State(state): State<SceneApiState>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<UpdateGroupTagsBody>,
) -> Result<Json<Value>, ApiError> {
    let handle = open_default(&state).await?;
    let patch = RenderScenePatch::SetObjectTags {
        target_kind: shape_scene_core::op::TargetKind::Frame,
        id: id.clone(),
        tag_ids: body.tag_ids.clone(),
    };
    match handle.apply_patch(patch, "http").await {
        ApplyResult::Applied { .. } => {
            let scene = handle.get_scene().await;
            let group = scene.groups.iter().find(|g| g.id == id).cloned();
            Ok(Json(json!({ "group": group, "scene": scene })))
        }
        ApplyResult::Rejected { errors } => Err(ApiError::internal(errors.join("; "))),
    }
}

// ---------------------------------------------------------------------------
// Comments
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CreateCommentBody {
    target: SceneSelection,
    body: String,
    #[serde(default)]
    author: Option<String>,
}

/// `POST /api/comments` -> `{ comment, scene }`.
async fn create_comment(
    State(state): State<SceneApiState>,
    Json(body): Json<CreateCommentBody>,
) -> Result<Json<Value>, ApiError> {
    // author is accepted by the wire shape but scene-core stamps "human" today.
    let _ = &body.author;
    let handle = open_default(&state).await?;
    match handle.add_comment(body.target, &body.body, "http").await {
        CommentResult::Added { comment } => {
            let scene = handle.get_scene().await;
            Ok(Json(json!({ "comment": comment, "scene": scene })))
        }
        CommentResult::Rejected { errors } => Err(ApiError::internal(errors.join("; "))),
    }
}

#[derive(Debug, Deserialize)]
struct UpdateCommentBody {
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    resolved: Option<bool>,
}

/// `PATCH /api/comments/:id` -> `{ comment, scene }`.
async fn update_comment(
    State(state): State<SceneApiState>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<UpdateCommentBody>,
) -> Result<Json<Value>, ApiError> {
    let handle = open_default(&state).await?;
    match handle.update_comment(&id, body.body, body.resolved, "http").await {
        CommentResult::Added { comment } => {
            let scene = handle.get_scene().await;
            Ok(Json(json!({ "comment": comment, "scene": scene })))
        }
        CommentResult::Rejected { errors } => Err(ApiError::internal(errors.join("; "))),
    }
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ExportScopeBody {
    kind: String,
    #[serde(default)]
    id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ExportBody {
    #[serde(rename = "type")]
    export_type: ExportType,
    #[serde(default)]
    scope: Option<ExportScopeBody>,
}

/// `POST /api/groups/:id/export` -> `{ scene, group, artifact, preview }`.
/// Generates the export via `local_export` (reusing scene-core graph helpers),
/// writes the content under the exports dir, persists an artifact Record on the
/// scene, and returns the preview the ExportDrawer renders.
async fn export_group(
    State(state): State<SceneApiState>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<ExportBody>,
) -> Result<Json<Value>, ApiError> {
    let handle = open_default(&state).await?;
    let scene = handle.get_scene().await;
    let Some(group) = scene.groups.iter().find(|g| g.id == id).cloned() else {
        return Err(ApiError::not_found("Group not found"));
    };

    let scope = body
        .scope
        .map(|s| ExportScope { kind: s.kind, id: s.id })
        .unwrap_or(ExportScope { kind: "group".to_string(), id: Some(id.clone()) });
    let graph = scene_graph_for_group(&scene, &id);
    let generated: ExportOutput =
        generate_local_export(&graph, body.export_type, &scope, &group.title);

    let content_type = content_type_for(body.export_type);
    let path = write_artifact_content(
        &state.exports_dir,
        &id,
        body.export_type,
        &generated.title,
        &generated.content,
    )
    .map_err(|e| ApiError::internal(format!("failed to write export: {e}")))?;

    let artifact = SceneArtifact {
        id: String::new(),
        export_type: body.export_type,
        title: generated.title.clone(),
        target: SceneSelection::Group { id: id.clone() },
        path,
        content_type: content_type.to_string(),
        created_at: String::new(),
        scene_version: 0,
    };
    let persisted = match handle.add_artifact(&id, artifact).await {
        ArtifactResult::Added { artifact } => artifact,
        ArtifactResult::Rejected { errors } => {
            return Err(ApiError::internal(errors.join("; ")))
        }
    };

    let scene = handle.get_scene().await;
    let mut preview = json!({
        "type": body.export_type,
        "title": generated.title,
        "content": generated.content,
        "contentType": content_type,
    });
    if let Some(image_prompt) = generated.image_prompt {
        preview["imagePrompt"] = Value::String(image_prompt);
    }
    Ok(Json(json!({
        "scene": scene,
        "group": group,
        "artifact": persisted,
        "preview": preview,
    })))
}

/// `GET /api/groups/:groupId/artifacts/:artifactId` -> the artifact file content
/// as an attachment download. 404 if the artifact is missing / not on this group,
/// 403 if its stored path escapes the exports dir (mirrors `isExportPath`).
async fn download_artifact(
    State(state): State<SceneApiState>,
    AxumPath((group_id, artifact_id)): AxumPath<(String, String)>,
) -> Result<axum::response::Response, ApiError> {
    let handle = open_default(&state).await?;
    let scene = handle.get_scene().await;
    let Some(artifact) = scene.artifacts.iter().find(|a| a.id == artifact_id).cloned() else {
        return Err(ApiError::not_found("Artifact not found"));
    };
    let targets_group = matches!(&artifact.target, SceneSelection::Group { id } if id == &group_id);
    if !targets_group {
        return Err(ApiError::not_found("Artifact not found"));
    }
    if !is_export_path(&state.exports_dir, &artifact.path) {
        return Err(ApiError::forbidden(
            "Artifact path is outside export directory",
        ));
    }
    let bytes = std::fs::read(&artifact.path)
        .map_err(|e| ApiError::internal(format!("failed to read artifact: {e}")))?;
    let filename = sanitize_filename(&artifact.title);
    Ok((
        [
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
            (header::CONTENT_TYPE, artifact.content_type.clone()),
        ],
        bytes,
    )
        .into_response())
}

// ---------------------------------------------------------------------------
// file IO + naming helpers (port of storage.ts helpers)
// ---------------------------------------------------------------------------

/// Port of `writeArtifactContent`: write under `{exports}/{groupId}/` with a
/// `{seq}-{type}-{slug-title}.{ext}` filename and return the absolute path.
fn write_artifact_content(
    exports_dir: &std::path::Path,
    group_id: &str,
    export_type: ExportType,
    title: &str,
    content: &str,
) -> std::io::Result<String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static FILE_SEQ: AtomicU64 = AtomicU64::new(0);
    let dir = exports_dir.join(group_id);
    std::fs::create_dir_all(&dir)?;
    let seq = FILE_SEQ.fetch_add(1, Ordering::Relaxed);
    let title_slug: String = sanitize_filename(title).chars().take(80).collect();
    let filename = format!(
        "{seq}-{}-{title_slug}.{}",
        export_type_token(export_type),
        extension_for(export_type)
    );
    let path = dir.join(filename);
    std::fs::write(&path, content)?;
    Ok(path.to_string_lossy().to_string())
}

/// Port of `isExportPath`: the path is the exports dir or sits under it.
fn is_export_path(exports_dir: &std::path::Path, path: &str) -> bool {
    let candidate = std::path::Path::new(path);
    candidate == exports_dir || candidate.starts_with(exports_dir)
}

/// Replace any non `[a-z0-9.-]` run with `-` (mirrors the Node regex
/// `/[^a-z0-9.-]+/gi`).
fn sanitize_filename(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut prev_dash = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' {
            out.push(ch);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    out
}

/// Port of `extensionFor`.
fn extension_for(export_type: ExportType) -> &'static str {
    match export_type {
        ExportType::Yadr => "yaml",
        ExportType::ConfluenceHtml => "html",
        ExportType::Mermaid => "mmd",
        _ => "md",
    }
}

fn export_type_token(export_type: ExportType) -> &'static str {
    match export_type {
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

/// Slugify a tag name like the Node `slug` (`[^a-z0-9]+` -> `-`, trimmed).
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

/// A process-unique-enough id suffix for a new tag.
fn tag_suffix(scene: &Scene) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{:x}{:x}", scene.tags.len(), n)
}
