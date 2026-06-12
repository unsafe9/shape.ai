//! Pure Rust collaboration session core shared by every shell, running natively
//! and on wasm32:
//!
//! - [`outbox`] — durable unacked-op bookkeeping (append-before-send,
//!   remove-on-ack, monotonic `local_seq`, deterministic replay).
//! - [`peers`] — peer presence: latest-wins per `userId`, TTL expiry, stable color.
//! - [`scene_client`] — viewport windowing *decisions* (region math, margin,
//!   whether to re-subscribe); debounce timing stays shell-side.
//! - [`sync_engine`] — optimistic local apply over scene-core, per-`(objectId:field)`
//!   ownership gating, base-revision tracking, coalescing *policy*, reconnect reconcile.
//!
//! Purity: no ambient time, randomness, threads, or IO — the clock, the outbox
//! port, and the transport sink are all injected. The crate decides; the shell
//! drives timers and side effects.

pub mod outbox;
pub mod peers;
pub mod scene_client;
pub mod sync_engine;

/// wasm-bindgen session bridge (web shell FFI), gated off `wasm` so native
/// builds never touch wasm-bindgen.
#[cfg(feature = "wasm")]
pub mod wasm_session;

/// Re-export scene-core's wasm bridge so its `#[wasm_bindgen]` exports land in
/// THIS crate's wasm-pack bundle alongside [`wasm_session`] — one bundle carries
/// both crates' exports.
#[cfg(feature = "wasm")]
pub use shape_scene_core::wasm_api;

pub use outbox::{op_id_key, InMemoryOutboxStore, OpId, OutboxEntry, OutboxError, OutboxStore};
pub use peers::{PeerPresence, PeerRegistry, PresencePayload, DEFAULT_PEER_TTL_MS};
pub use scene_client::{
    bbox_equals, window_from_viewport, Bbox, WindowState, DEFAULT_VIEWPORT_MARGIN,
};
pub use sync_engine::{AuthorResult, EngineTransport, SyncEngine, COALESCE_MS};
