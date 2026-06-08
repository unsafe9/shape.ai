//! HTTP router assembly.
//!
//! This is the platform seam: it wires transport (axum routes) and static hosting
//! only. Canvas behaviour lives in `shape_scene_core`; this crate never
//! reimplements op-apply. The object-native cutover (OB4.1) mounts the WS
//! transport + the object MCP endpoint and drops the bespoke REST domain routes
//! (groups/comments/export are now Feature frames over WS).

use std::path::Path;

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::{
    routing::{delete, get},
    Json, Router,
};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use serde::Deserialize;
use serde_json::{json, Value};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

use crate::config::Config;
use crate::mcp::SceneMcp;
use crate::registry::CanvasRegistry;
use crate::ws::ws_handler;

/// Build the application router for the given configuration.
///
/// Routes:
/// - `GET /api/health` — liveness probe with build identity.
/// - `GET /api/ready`  — readiness probe (200 once the process is serving).
/// - static client assets under `client_dir` with SPA fallback, when present.
pub fn build_router(config: &Config) -> Router {
    let api = Router::new()
        .route("/api/health", get(health))
        .route("/api/ready", get(ready));

    let router = match static_service(&config.client_dir) {
        Some(static_router) => api.merge(static_router),
        None => {
            tracing::info!(
                client_dir = %config.client_dir.display(),
                "no client assets found; serving API only"
            );
            api
        }
    };

    router.layer(TraceLayer::new_for_http())
}

/// Build the full router including the WS transport + the object MCP endpoint.
///
/// This is the entry point [`crate::serve`] uses:
/// - `GET /ws` — the WebSocket transport (two logical channels over one socket),
///   bridged to the per-canvas object actor.
/// - `POST/GET/DELETE /mcp` — the streamable-HTTP object MCP transport.
/// - `GET/POST/DELETE /api/canvases` — canvas CRUD until WS-native canvas ops.
/// - `GET /api/templates` — the builtin object-template catalog.
///
/// `canvases` is the shared canvas actor registry the MCP tools and the WS
/// transport drive. The base [`build_router`] is registry-free so the HTTP-only
/// tests can drive it without constructing a registry.
pub fn build_router_with_mcp(config: &Config, canvases: CanvasRegistry) -> Router {
    let mcp_service = mcp_service(canvases.clone());

    let ws = Router::new()
        .route("/ws", get(ws_handler))
        .with_state(canvases.clone());

    // Canvas CRUD: the client switch UI drives these until WS-native canvas ops
    // land. State is the same registry the WS/MCP surfaces use, so a create here
    // is immediately openable over /ws.
    let canvas_api = Router::new()
        .route("/api/canvases", get(list_canvases).post(create_canvas))
        .route("/api/canvases/:id", delete(delete_canvas))
        .with_state(canvases);

    // Template catalog: object templates are code-defined builtin recipes
    // (`object::templates`), not stored documents, so the catalog is read-only
    // and needs no shared state — the picker lists ids/labels/categories and the
    // client lowers a chosen template to ops via the wasm bridge / a feature frame.
    let template_api = Router::new().route("/api/templates", get(list_templates));

    build_router(config)
        .merge(ws)
        .merge(canvas_api)
        .merge(template_api)
        .nest_service("/mcp", mcp_service)
}

/// Construct the streamable-HTTP MCP transport service. Its factory closure runs
/// once per session, minting a fresh [`SceneMcp`] bound to the shared registry.
fn mcp_service(canvases: CanvasRegistry) -> StreamableHttpService<SceneMcp, LocalSessionManager> {
    StreamableHttpService::new(
        move || Ok(SceneMcp::new(canvases.clone())),
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default(),
    )
}

/// `GET /api/canvases` — list every canvas from the durable index.
async fn list_canvases(State(canvases): State<CanvasRegistry>) -> Json<Value> {
    Json(json!({ "canvases": canvases.list_canvases() }))
}

#[derive(Debug, Deserialize)]
struct CreateCanvasBody {
    title: Option<String>,
}

/// `POST /api/canvases` — create a canvas and return its summary.
async fn create_canvas(
    State(canvases): State<CanvasRegistry>,
    body: Option<Json<CreateCanvasBody>>,
) -> Result<Json<Value>, StatusCode> {
    let title = body
        .and_then(|Json(b)| b.title)
        .unwrap_or_else(|| "Untitled".to_string());
    canvases
        .create_canvas(&title)
        .map(|summary| Json(json!({ "canvas": summary })))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// `DELETE /api/canvases/:id` — delete a canvas (evict, de-index, prune scene).
/// Returns 404 when the id was not in the index.
async fn delete_canvas(
    State(canvases): State<CanvasRegistry>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, StatusCode> {
    let canvas_id = shape_scene_core::CanvasId::from(id.as_str());
    match canvases.delete_canvas(&canvas_id).await {
        Ok(true) => Ok(Json(json!({ "deleted": id }))),
        Ok(false) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// `GET /api/templates` — the builtin object-template catalog (id, label,
/// category, description), in picker display order. Object templates are pure
/// builder recipes in `shape_scene_core::object::templates`, so this is a static
/// read-only list with no persistence.
async fn list_templates() -> Json<Value> {
    Json(json!({ "templates": shape_scene_core::object::object_template_catalog() }))
}

/// Liveness: the server is up and identifies itself.
async fn health() -> Json<Value> {
    Json(json!({
        "ok": true,
        "name": "shape_server",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// Readiness: 200 with a tiny body once the process can serve requests.
async fn ready() -> Json<Value> {
    Json(json!({ "ready": true }))
}

/// A router that serves the pre-built SPA from `dir`, falling back to
/// `index.html` for client-side routes. Returns `None` if the directory or its
/// `index.html` is absent, so static hosting is skipped gracefully.
fn static_service(dir: &Path) -> Option<Router> {
    let index = dir.join("index.html");
    if !dir.is_dir() || !index.is_file() {
        return None;
    }
    let serve_dir = ServeDir::new(dir).fallback(ServeFile::new(index));
    Some(Router::new().fallback_service(serve_dir))
}
