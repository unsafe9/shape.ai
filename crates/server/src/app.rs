//! HTTP router assembly.
//!
//! This is the platform seam: it wires transport (axum routes) and static
//! hosting only. Canvas behaviour lives in `shape_scene_core`; this crate never
//! reimplements op-apply. Later phases hang the canvas actor and the MCP
//! endpoint off the same router via shared state.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::extract::{Path as AxumPath, Query, State};
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
use crate::mcp_clients::ClientRegistry;
use crate::registry::CanvasRegistry;
use crate::scene_api::{scene_api_router, SceneApiState};
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

/// Build the full router including the MCP transport + companion-dock HTTP API.
///
/// This is the entry point [`crate::serve`] uses: it layers the MG2.3 + MG-3
/// surface onto the base [`build_router`] router:
/// - `GET /ws` — the MG-3 WebSocket transport (two logical channels over one
///   socket), bridged to the per-canvas actor via the shared `canvases` state.
/// - `POST/GET/DELETE /mcp` — the streamable-HTTP MCP transport (tools/list,
///   tools/call, …), served by rmcp's [`StreamableHttpService`].
/// - `GET /api/mcp/clients` — the live companion identity list (dock).
/// - `GET /api/mcp/trace?clientId=..&limit=..` — one companion's recent trace.
///
/// `canvases` is the shared canvas actor registry the MCP tools and the WS
/// transport drive; `clients` is the in-memory companion/trace registry the dock
/// reads. The `/ws` route carries `canvases` as its own typed state (the base
/// [`build_router`] is registry-free so the HTTP-only tests can drive it without
/// constructing a registry).
pub fn build_router_with_mcp(
    config: &Config,
    canvases: CanvasRegistry,
    clients: ClientRegistry,
) -> Router {
    let mcp_service = mcp_service(canvases.clone(), clients.clone());
    let canvases_for_scene_api = canvases.clone();

    let ws = Router::new()
        .route("/ws", get(ws_handler))
        .with_state(canvases.clone());

    // MG9.1 canvas CRUD: the client switch UI drives these until WS-native
    // canvas ops land. State is the same registry the WS/MCP surfaces use, so a
    // create here is immediately openable over /ws.
    let canvas_api = Router::new()
        .route("/api/canvases", get(list_canvases).post(create_canvas))
        .route("/api/canvases/:id", delete(delete_canvas))
        .with_state(canvases.clone());

    // CC3.2 template library: builtins are seeded once into the shared store on
    // router build; the cockpit reads/writes the catalog through these routes.
    if let Err(error) = canvases.seed_templates() {
        tracing::warn!(%error, "failed to seed builtin templates");
    }
    let template_api = Router::new()
        .route("/api/templates", get(list_templates).post(create_template))
        .route("/api/templates/:id", delete(delete_template))
        .with_state(canvases);

    let dock = Router::new()
        .route("/api/mcp/clients", get(mcp_clients))
        .route("/api/mcp/trace", get(mcp_trace))
        .with_state(clients);

    // MG-7: the legacy Node scene routes, ported. Shares the same canvas registry
    // (single default canvas) and writes exports under `{data_dir}/exports`.
    let scene_api = scene_api_router(SceneApiState {
        canvases: canvases_for_scene_api,
        exports_dir: exports_dir(),
    });

    build_router(config)
        .merge(ws)
        .merge(canvas_api)
        .merge(template_api)
        .merge(dock)
        .merge(scene_api)
        .nest_service("/mcp", mcp_service)
}

/// The exports directory the scene API writes generated artifacts into:
/// `{SHAPE_AI_DATA_DIR or .local}/exports`, matching the Node `EXPORTS_DIR`.
fn exports_dir() -> std::path::PathBuf {
    let data_dir = std::env::var("SHAPE_AI_DATA_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from(".local"));
    data_dir.join("exports")
}

/// Per-session MCP client ids are unique within the process so the dock can tell
/// concurrent companions apart even before they advertise an identity.
static SESSION_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Construct the streamable-HTTP MCP transport service. Its factory closure runs
/// once per session, minting a fresh [`SceneMcp`] bound to the shared registries
/// plus a stable per-session client id.
fn mcp_service(
    canvases: CanvasRegistry,
    clients: ClientRegistry,
) -> StreamableHttpService<SceneMcp, LocalSessionManager> {
    StreamableHttpService::new(
        move || {
            let n = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
            let client_id = format!("http-session-{n}");
            Ok(SceneMcp::new(canvases.clone(), clients.clone(), client_id))
        },
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default(),
    )
}

/// Dock read path: the live companion identity list.
async fn mcp_clients(State(clients): State<ClientRegistry>) -> Json<Value> {
    Json(json!({ "clients": clients.list() }))
}

#[derive(Debug, Deserialize)]
struct TraceQuery {
    #[serde(rename = "clientId")]
    client_id: Option<String>,
    limit: Option<usize>,
}

/// Dock read path: one companion's recent trace (read/write/comment/export ring).
async fn mcp_trace(
    State(clients): State<ClientRegistry>,
    Query(query): Query<TraceQuery>,
) -> Json<Value> {
    let client_id = query.client_id.unwrap_or_default();
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let trace = clients.trace(&client_id, limit);
    let total = trace.len();
    Json(json!({ "clientId": client_id, "trace": trace, "total": total }))
}

/// `GET /api/canvases` — list every canvas from the durable index (MG9.1).
async fn list_canvases(State(canvases): State<CanvasRegistry>) -> Json<Value> {
    Json(json!({ "canvases": canvases.list_canvases() }))
}

#[derive(Debug, Deserialize)]
struct CreateCanvasBody {
    title: Option<String>,
}

/// `POST /api/canvases` — create a canvas and return its summary (MG9.1).
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
/// Returns 404 when the id was not in the index (MG9.1).
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

/// `GET /api/templates` — list every stored template (seeded builtins minus
/// tombstoned + user templates) (CC3.2).
async fn list_templates(State(canvases): State<CanvasRegistry>) -> Json<Value> {
    Json(json!({ "templates": canvases.list_templates() }))
}

/// `POST /api/templates` — create (or overwrite) a user template from a
/// `TemplateContract` body (CC3.2). 400 on a malformed contract.
async fn create_template(
    State(canvases): State<CanvasRegistry>,
    body: Json<shape_scene_core::TemplateContract>,
) -> Result<Json<Value>, StatusCode> {
    let Json(contract) = body;
    canvases
        .create_template(&contract)
        .map(|()| Json(json!({ "template": contract })))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// `DELETE /api/templates/:id` — delete a template, tombstoning it so a deleted
/// builtin is not re-seeded (CC3.2). 404 when no template Record existed.
async fn delete_template(
    State(canvases): State<CanvasRegistry>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, StatusCode> {
    match canvases.delete_template(&id) {
        Ok(true) => Ok(Json(json!({ "deleted": id }))),
        Ok(false) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
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
