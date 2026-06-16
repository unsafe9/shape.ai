//! shape.ai native server (`shape_server`): the platform seam in front of the
//! pure scene core, orchestrating transport, persistence, and fan-out only.
//! Canvas logic (op-apply, layout, hit testing) stays in `shape_scene_core` and
//! is never reimplemented here.
//!
//! Native-only (tokio/axum); siblings are depended on by path without adding deps
//! to them, so the wasm builds of scene-core/storage-core stay intact.
//!
//! Identity is `userId`-only with no auth.
//! TODO(auth): real authn/authz attaches here when identity moves past userId.

pub mod app;
pub mod canvas_actor;
pub mod canvas_index;
pub mod config;
pub mod extension_registry;
pub mod mcp;
pub mod object_feature;
pub mod object_mcp;
pub mod object_store;
pub mod registry;
pub mod sync;
pub mod ws;

pub use app::{build_router, build_router_with_mcp};
pub use canvas_actor::{ActorHandle, ApplyResult, CanvasActor, PatchBroadcast, SharedStore};
pub use config::Config;
pub use extension_registry::{extension_tools_json, ExtToolDef, ExtensionRegistry};
pub use mcp::{SceneMcp, DEFAULT_CANVAS_ID};
pub use object_feature::{decode_feature, encode_feature_response, handle_feature, FeatureCtx};
pub use object_mcp::{
    object_mcp_tools, CreateObjectSpec, McpToolMeta, ObjectSummary, PatchObjectSpec, QueryFilter,
};
pub use object_store::{ObjectStore, ObjectStoreError, KIND_CANVAS, KIND_OBJECT};
pub use registry::{CanvasRegistry, SpawnError};
pub use sync::{DedupTable, OpAck, OpEnvelope, OpId, CHECKPOINT_INTERVAL};
pub use ws::{ws_handler, WsClientMessage, WsServerMessage};

/// Bind to the configured address and serve until the process is terminated.
pub async fn serve(config: Config) -> anyhow::Result<()> {
    let data_dir = std::env::var("SHAPE_AI_DATA_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from(".local"));
    std::fs::create_dir_all(&data_dir)?;
    let canvases = CanvasRegistry::open(data_dir.join("shape.redb"))?;

    let router = build_router_with_mcp(&config, canvases.clone());
    let addr = config.socket_addr()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "shape_server listening");
    // On ctrl_c, stop accepting, then drain the registry (flush + checkpoint
    // every actor, release every lease).
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
