//! shape.ai client collaboration session core.
//!
//! The pure Rust collaboration runtime every future shell shares. It ports the
//! semantics of the web shell's TypeScript runtime (`platforms/web/runtime/*`) to
//! Rust so the same session logic runs natively (macOS Metal, iOS) and on wasm32:
//!
//! - [`outbox`] — durable unacked-op bookkeeping: append-before-send,
//!   remove-on-ack, monotonic `local_seq`, deterministic replay order. Persistence
//!   sits behind the [`outbox::OutboxStore`] port (the web shell keeps IndexedDB).
//! - [`peers`] — peer presence registry: latest-wins per `userId`, TTL expiry vs
//!   an injected `now_ms`, stable color assignment.
//! - [`scene_client`] — viewport windowing *decisions*: region math, margin,
//!   whether to re-subscribe. Debounce timing stays shell-side.
//! - [`sync_engine`] — the session core: optimistic local apply over scene-core
//!   [`apply_object_op`](shape_scene_core::object::apply_object_op), per-
//!   `(objectId:field)` ownership gating remote patches, base-revision tracking,
//!   the 33ms coalescing *policy* (decision only — the shell drives the timer),
//!   and welcome-snapshot reconcile with unacked-op replay.
//!
//! Purity: no ambient time, randomness, threads, or IO. The clock (`now_ms` /
//! `now: &str`), persistence (the outbox port), and transport (the
//! [`sync_engine::EngineTransport`] sink) are all injected. The crate computes
//! decisions; the shell drives timers and side effects.

pub mod outbox;
pub mod peers;
pub mod scene_client;
pub mod sync_engine;

/// wasm-bindgen session bridge (the web shell FFI); gated off the `wasm` feature
/// so native builds never touch wasm-bindgen.
#[cfg(feature = "wasm")]
pub mod wasm_session;

/// Re-export scene-core's wasm bridge so its `#[wasm_bindgen]` exports
/// (apply_object_op, the catalogs, the builders, WasmUndoStack, …) land in THIS
/// crate's wasm-pack bundle alongside [`wasm_session`]. The single bundle carries
/// both crates' exports, so `platforms/web/bridge/sceneCoreWasm.ts` keeps working
/// against the same artifact that now also exposes the session.
#[cfg(feature = "wasm")]
pub use shape_scene_core::wasm_api;

pub use outbox::{op_id_key, InMemoryOutboxStore, OpId, OutboxEntry, OutboxError, OutboxStore};
pub use peers::{PeerPresence, PeerRegistry, PresencePayload, DEFAULT_PEER_TTL_MS};
pub use scene_client::{
    bbox_equals, window_from_viewport, Bbox, WindowState, DEFAULT_VIEWPORT_MARGIN,
};
pub use sync_engine::{AuthorResult, EngineTransport, SyncEngine, COALESCE_MS};
