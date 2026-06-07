//! shape.ai native server (`shape_server`).
//!
//! The web/native platform seam in front of the pure scene core: this crate
//! orchestrates transport (axum/tokio), persistence (`shape_storage_core`), and
//! fan-out only. Canvas logic — op-apply, layout, hit testing — stays in
//! `shape_scene_core` and is never reimplemented here.
//!
//! It is **native-only** (tokio/axum) and depends on the sibling crates by path
//! without adding deps to them, so the wasm builds of scene-core/storage-core
//! stay intact.
//!
//! Identity is `userId`-only with **no auth** (C13).
//! TODO(auth): real authn/authz attaches here when identity moves past userId.
//!
//! Phasing: this file exposes the building blocks (`Config`, `build_router`,
//! `serve`) so integration tests can drive the router directly and later phases
//! can add the canvas actor + MCP without reshaping the entry points.

pub mod app;
pub mod canvas_actor;
pub mod canvas_index;
pub mod config;
pub mod group_seed;
pub mod local_export;
pub mod mcp;
pub mod mcp_clients;
pub mod registry;
pub mod scene_api;
pub mod scene_store;
pub mod sync;
pub mod template_store;
pub mod ws;

pub use app::{build_router, build_router_with_mcp};
pub use canvas_actor::{
    ActorHandle, ApplyResult, ArtifactResult, CanvasActor, CommentResult, PatchBroadcast,
};
pub use sync::{DedupTable, OpAck, OpEnvelope, OpId, CHECKPOINT_INTERVAL};
pub use config::Config;
pub use mcp::{SceneMcp, DEFAULT_CANVAS_ID};
pub use mcp_clients::ClientRegistry;
pub use registry::{CanvasRegistry, SpawnError};
pub use ws::{ws_handler, WsClientMessage, WsServerMessage};

/// Bind to the configured address and serve until the process is terminated.
///
/// The shared application state — the canvas actor registry and the in-memory
/// MCP companion registry — is constructed here, before the router, and threaded
/// into [`build_router_with_mcp`]. MG-3 hangs the WebSocket transport off the
/// same registry.
pub async fn serve(config: Config) -> anyhow::Result<()> {
    // Persist under SHAPE_AI_DATA_DIR/.local (canvas sqlite); MG-9 adds canvas
    // CRUD and per-canvas db routing.
    let data_dir = std::env::var("SHAPE_AI_DATA_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from(".local"));
    std::fs::create_dir_all(&data_dir)?;
    let canvases = CanvasRegistry::open(data_dir.join("shape.sqlite"))?;
    let clients = ClientRegistry::new();

    let router = build_router_with_mcp(&config, canvases.clone(), clients);
    let addr = config.socket_addr()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "shape_server listening");
    // MG8.3 graceful shutdown: on ctrl_c, stop accepting connections, then drain
    // the registry (flush + checkpoint every actor, release every lease) so a
    // restart/successor recovers with no data loss.
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("shutdown signal received; draining canvases");
        })
        .await?;
    canvases.shutdown().await;
    tracing::info!("canvases drained; exiting");
    Ok(())
}
