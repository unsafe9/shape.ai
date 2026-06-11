//! wasm-bindgen session bridge — gated behind `cfg(feature = "wasm")`.
//!
//! `WasmSession` is the THIN FFI wrapper the web shell drives: it owns the pure
//! [`SyncEngine`](crate::sync_engine::SyncEngine) (over an in-memory bookkeeping
//! outbox + a buffering transport) and a [`PeerRegistry`](crate::peers), and
//! exposes their decisions as JSON-in / JSON-out methods. The collaboration logic
//! — optimistic apply, transient ownership, the coalescing flush decision,
//! reconnect replay, peer latest-wins/TTL — runs HERE in Rust; the TS shell keeps
//! only the IO seams (the WS socket, IndexedDB durability, the flush/expiry
//! timers).
//!
//! Error policy mirrors `scene-core`'s `wasm_api.rs`: these methods never panic
//! across the FFI boundary. A malformed-input/serialize failure is returned as a
//! JSON `{"error": "<message>"}` string (the TS adapter branches on it); domain
//! failures flow through normally (e.g. a rejected op rides the `errors` array of
//! the author result).
//!
//! Seams the TS shell still drives:
//!   - persistence: [`WasmSession::author`] returns the durable `WireOp` entry the
//!     shell writes to IndexedDB; ack/reject return the removed `OpId`s so the
//!     shell drops the matching rows. On reconnect the shell reads its rows back
//!     and hands them to [`WasmSession::reconcile_snapshot`] for replay.
//!   - transport: buffered coalesced envelopes are drained with
//!     [`WasmSession::take_pending`] and shipped on the socket.
//!   - timers: the shell arms its coalescing timer while
//!     [`WasmSession::flush_armed`] is true and fires [`WasmSession::on_flush_due`];
//!     it stamps each `ts` and each peer-frame `now_ms`.

use serde::Serialize;
use wasm_bindgen::prelude::wasm_bindgen;

use shape_scene_core::object::model::ObjectScene;
use shape_scene_core::object::op::ObjectOp;
use shape_scene_core::wire::{OpId, WireOp};

use crate::outbox::{InMemoryOutboxStore, OutboxEntry};
use crate::peers::PeerRegistry;
use crate::sync_engine::{EngineTransport, SyncEngine};

/// Serialize `value`, or fall back to an `{"error": ...}` JSON if serialization
/// fails. Keeps every success payload infallible across the boundary.
fn ok_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|e| error_json(&format!("serialize failed: {e}")))
}

/// Build an `{"error": "<msg>"}` JSON string (infallible).
fn error_json(message: &str) -> String {
    #[derive(Serialize)]
    struct ErrorPayload<'a> {
        error: &'a str,
    }
    serde_json::to_string(&ErrorPayload { error: message })
        .unwrap_or_else(|_| "{\"error\":\"unserializable error\"}".to_string())
}

/// Parse `json` into `T`, mapping a serde error into an `Err(error_json)`.
fn parse<T: serde::de::DeserializeOwned>(label: &str, json: &str) -> Result<T, String> {
    serde_json::from_str(json).map_err(|e| error_json(&format!("invalid {label} JSON: {e}")))
}

/// Convert a JS-supplied millisecond `f64` (timestamp / duration / revision) to
/// the `i64` the pure cores carry. wasm-bindgen forces JS numbers across the FFI
/// as `f64`; these values are always integral ms, so round to the nearest integer
/// and clamp to the `i64` range.
#[allow(
    clippy::cast_possible_truncation,
    reason = "JS-supplied integral ms rounded + clamped to i64; the only f64->i64 seam"
)]
fn ms(value: f64) -> i64 {
    value.round().clamp(i64::MIN as f64, i64::MAX as f64) as i64
}

/// Buffering transport: the engine pushes coalesced batches here; the shell drains
/// them with [`WasmSession::take_pending`] and ships them on the WS socket. The
/// inner buffer is FLAT (concatenated entries) — the TS adapter sends one `ops`
/// frame per flush, which is exactly one drained batch.
#[derive(Default)]
struct BufferTransport {
    flushed: Vec<OutboxEntry>,
}

impl BufferTransport {
    /// Drain the buffered envelopes the engine has flushed so far.
    fn take(&mut self) -> Vec<OutboxEntry> {
        std::mem::take(&mut self.flushed)
    }
}

impl EngineTransport for BufferTransport {
    fn send_envelopes(&mut self, entries: &[OutboxEntry]) {
        self.flushed.extend_from_slice(entries);
    }
}

/// The author-result wire shape returned to the shell: the rejecting-core errors
/// (empty on success), the minted `op_id`, the captured inverse op (the undo
/// entry, D21), and the durable `WireOp` entry the shell persists to IndexedDB
/// (null when the op was rejected and nothing was enqueued).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthorWire<'a> {
    errors: &'a [String],
    op_id: Option<&'a OpId>,
    inverse: Option<&'a ObjectOp>,
    /// The persisted envelope, or null when the op was rejected.
    entry: Option<&'a WireOp>,
}

#[wasm_bindgen]
pub struct WasmSession {
    engine: SyncEngine<BufferTransport, InMemoryOutboxStore>,
    peers: PeerRegistry,
}

#[wasm_bindgen]
impl WasmSession {
    /// Build a session from the `welcome` scene JSON. `client_id` is stamped into
    /// every `opId.clientId` + `WireOp.actor`; `self_user_id` (empty for none) is
    /// the peer self-skip identity; `coalesce_ms < 0` uses the default window;
    /// `peer_ttl_ms < 0` uses the default stale window.
    #[wasm_bindgen(constructor)]
    pub fn new(
        welcome_scene_json: &str,
        client_id: &str,
        self_user_id: &str,
        coalesce_ms: f64,
        peer_ttl_ms: f64,
    ) -> Result<WasmSession, String> {
        let scene: ObjectScene = parse("welcome scene", welcome_scene_json)?;
        let coalesce = (coalesce_ms >= 0.0).then(|| ms(coalesce_ms));
        let ttl = (peer_ttl_ms >= 0.0).then(|| ms(peer_ttl_ms));
        let self_id = (!self_user_id.is_empty()).then(|| self_user_id.to_string());
        Ok(WasmSession {
            engine: SyncEngine::new(
                scene,
                client_id,
                InMemoryOutboxStore::new(),
                BufferTransport::default(),
                coalesce,
            ),
            peers: PeerRegistry::new(self_id, ttl),
        })
    }

    /// Author a local op (`op_json`) stamped at `ts`: optimistic apply, take
    /// transient ownership, persist + buffer. Returns the [`AuthorWire`] JSON
    /// (errors / opId / inverse / the durable entry to persist), or `{error}` on
    /// malformed input.
    pub fn author(&mut self, op_json: &str, ts: &str) -> String {
        let op: ObjectOp = match parse("op", op_json) {
            Ok(v) => v,
            Err(e) => return e,
        };
        match self.engine.author(op, ts) {
            Ok(res) => {
                let entry = res
                    .op_id
                    .as_ref()
                    .and_then(|id| self.engine.outbox_entry(id));
                ok_json(&AuthorWire {
                    errors: &res.errors,
                    op_id: res.op_id.as_ref(),
                    inverse: res.inverse.as_ref(),
                    entry: entry.as_ref(),
                })
            }
            Err(e) => error_json(&e.to_string()),
        }
    }

    /// Apply a REMOTE op (`op_json`) honoring transient ownership. Returns `"true"`
    /// when applied, `"false"` when dropped (owned key or a domain reject), or
    /// `{error}` on malformed input.
    pub fn apply_remote(&mut self, op_json: &str) -> String {
        let op: ObjectOp = match parse("op", op_json) {
            Ok(v) => v,
            Err(e) => return e,
        };
        ok_json(&self.engine.apply_remote(op))
    }

    /// Reconcile an ack: drop the acked `op_ids` (a JSON `OpId[]`) from the outbox
    /// bookkeeping, release ownership, advance the base revision (`revision < 0` =
    /// no revision). Returns the removed `OpId[]` JSON so the shell drops the
    /// matching IndexedDB rows, or `{error}` on malformed input.
    pub fn on_ack(&mut self, op_ids_json: &str, revision: f64) -> String {
        let op_ids: Vec<OpId> = match parse("opIds", op_ids_json) {
            Ok(v) => v,
            Err(e) => return e,
        };
        let rev = (revision >= 0.0).then(|| ms(revision));
        if let Err(e) = self.engine.on_ack(&op_ids, rev) {
            return error_json(&e.to_string());
        }
        ok_json(&op_ids)
    }

    /// Reconcile a rejected op: drop its `op_ids` (a JSON `OpId[]`) and release
    /// ownership. Returns the removed `OpId[]` JSON (the shell drops the rows), or
    /// `{error}` on malformed input.
    pub fn on_rejected(&mut self, op_ids_json: &str) -> String {
        let op_ids: Vec<OpId> = match parse("opIds", op_ids_json) {
            Ok(v) => v,
            Err(e) => return e,
        };
        if let Err(e) = self.engine.on_rejected(&op_ids) {
            return error_json(&e.to_string());
        }
        ok_json(&op_ids)
    }

    /// Reconnect reconcile: reset the base to the `snapshot_json` scene, seed the
    /// bookkeeping outbox from `persisted_entries_json` (the durable `WireOp[]` the
    /// shell read back from IndexedDB), then replay them on top — re-buffering them
    /// for re-send. Returns `null` on success or `{error}` on malformed input. The
    /// shell drains the replayed batch with [`take_pending`](Self::take_pending).
    pub fn reconcile_snapshot(
        &mut self,
        snapshot_json: &str,
        persisted_entries_json: &str,
    ) -> String {
        let snapshot: ObjectScene = match parse("snapshot", snapshot_json) {
            Ok(v) => v,
            Err(e) => return e,
        };
        let entries: Vec<WireOp> = match parse("persisted entries", persisted_entries_json) {
            Ok(v) => v,
            Err(e) => return e,
        };
        self.engine.reseed_outbox(entries);
        if let Err(e) = self.engine.reconcile_snapshot(snapshot) {
            return error_json(&e.to_string());
        }
        "null".to_string()
    }

    /// Flush the buffered coalesced frame immediately (gesture end / shutdown).
    /// Disarms the flush decision. The shell ships the result of a following
    /// [`take_pending`](Self::take_pending).
    pub fn flush(&mut self) {
        self.engine.flush();
    }

    /// The shell's coalescing timer fired: drain the pending buffer into one frame
    /// (the engine's decision). Disarms the flush decision.
    pub fn on_flush_due(&mut self) {
        self.engine.on_flush_due();
    }

    /// Drain the buffered envelopes the engine flushed: the `WireOp[]` JSON the
    /// shell ships as one `ops` frame. Empty when nothing is pending.
    pub fn take_pending(&mut self) -> String {
        ok_json(&self.engine.transport_mut().take())
    }

    /// The current optimistic object scene (`ObjectScene` JSON).
    pub fn scene(&self) -> String {
        ok_json(self.engine.scene())
    }

    /// The base revision the next authored op is stamped against.
    pub fn base_revision(&self) -> f64 {
        self.engine.base_revision() as f64
    }

    /// True while the shell must keep its coalescing timer armed (a flush is due).
    pub fn flush_armed(&self) -> bool {
        self.engine.flush_armed()
    }

    /// Number of currently-unacked outbox entries (the bookkeeping count).
    pub fn outbox_len(&self) -> u32 {
        u32::try_from(self.engine.outbox_len()).unwrap_or(u32::MAX)
    }

    /// The `(object,field)` keys currently held under transient ownership, as a
    /// sorted `string[]` JSON. Diagnostic parity with the engine's `owned_key_set`.
    pub fn owned_key_set(&self) -> String {
        ok_json(&self.engine.owned_key_set())
    }

    // --- peer presence registry ----------------------------------------------

    /// Ingest one inbound presence `payload_json` frame, stamping `now_ms` as its
    /// `last_seen`. Returns `"true"` when it updated the registry, `"false"`
    /// otherwise (no `userId`, or the local user's own frame), or `{error}` on
    /// malformed input.
    pub fn ingest_presence(&mut self, payload_json: &str, now_ms: f64) -> String {
        let payload: serde_json::Value = match parse("presence payload", payload_json) {
            Ok(v) => v,
            Err(e) => return e,
        };
        ok_json(&self.peers.ingest(&payload, ms(now_ms)))
    }

    /// Drop peers whose last frame is older than the TTL relative to `now_ms`.
    /// Returns `"true"` when any peer was removed (so the shell re-emits).
    pub fn expire_peers(&mut self, now_ms: f64) -> String {
        ok_json(&self.peers.expire(ms(now_ms)))
    }

    /// The live (currently-tracked) peers as a `PeerPresence[]` JSON, stable-
    /// ordered by `userId`.
    pub fn peers(&self) -> String {
        let peers: Vec<PeerWire> = self
            .peers
            .list()
            .into_iter()
            .map(|p| PeerWire {
                user_id: p.user_id,
                cursor: p.cursor,
                viewport: p.viewport,
                color: p.color,
                last_seen: p.last_seen,
            })
            .collect();
        ok_json(&peers)
    }

    /// Drop all tracked peers (canvas switch / disconnect).
    pub fn clear_peers(&mut self) {
        self.peers.clear();
    }
}

/// Wire shape for a tracked peer (camelCase, mirroring the TS `PeerPresence`).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PeerWire {
    user_id: String,
    cursor: Option<shape_scene_core::model::WorldPoint>,
    viewport: Option<shape_scene_core::model::Bounds>,
    color: &'static str,
    last_seen: i64,
}
