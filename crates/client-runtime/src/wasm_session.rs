//! wasm-bindgen session bridge — gated behind `cfg(feature = "wasm")`.
//!
//! `WasmSession` is the thin FFI wrapper the web shell drives: it owns the pure
//! [`SyncEngine`](crate::sync_engine::SyncEngine) and a [`PeerRegistry`](crate::peers)
//! and exposes their decisions as JSON-in / JSON-out methods. The collaboration
//! logic runs HERE; the TS shell keeps only the IO seams (WS socket, IndexedDB
//! durability, the flush/expiry timers).
//!
//! Error policy mirrors scene-core's `wasm_api`: these methods never panic across
//! the FFI. A malformed-input/serialize failure returns a JSON `{"error": ...}`
//! string; domain failures flow through normally (e.g. a rejected op rides the
//! author result's `errors` array).

use serde::Serialize;
use wasm_bindgen::prelude::wasm_bindgen;

use shape_scene_core::object::model::ObjectScene;
use shape_scene_core::object::op::ObjectOp;
use shape_scene_core::wire::{OpId, WireOp};

use crate::outbox::{InMemoryOutboxStore, OutboxEntry};
use crate::peers::PeerRegistry;
use crate::scene_client::{Bbox, WindowState};
use crate::sync_engine::{EngineTransport, SyncEngine};

/// Serialize `value`, or fall back to an `{"error": ...}` JSON, keeping every
/// success payload infallible across the boundary.
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

/// Convert a JS-supplied millisecond `f64` to the `i64` the cores carry.
/// wasm-bindgen forces JS numbers across the FFI as `f64`; these are integral ms,
/// so round and clamp to the `i64` range.
#[allow(
    clippy::cast_possible_truncation,
    reason = "JS-supplied integral ms rounded + clamped to i64; the only f64->i64 seam"
)]
fn ms(value: f64) -> i64 {
    value.round().clamp(i64::MIN as f64, i64::MAX as f64) as i64
}

/// Buffering transport: the engine pushes coalesced batches here; the shell
/// drains them with [`WasmSession::take_pending`] and ships them on the WS
/// socket. The buffer is FLAT — one `ops` frame per flush is one drained batch.
#[derive(Default)]
struct BufferTransport {
    flushed: Vec<OutboxEntry>,
}

impl BufferTransport {
    fn take(&mut self) -> Vec<OutboxEntry> {
        std::mem::take(&mut self.flushed)
    }
}

impl EngineTransport for BufferTransport {
    fn send_envelopes(&mut self, entries: &[OutboxEntry]) {
        self.flushed.extend_from_slice(entries);
    }
}

/// The author-result wire shape (camelCase): rejecting-core errors, the minted
/// `op_id`, the inverse undo op, and the durable `WireOp` entry the shell
/// persists.
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
    /// Build a session from the `welcome` scene JSON. `client_id` stamps every
    /// `opId.clientId` + `WireOp.actor`; `self_user_id` (empty for none) is the
    /// peer self-skip identity; negative `coalesce_ms`/`peer_ttl_ms` use defaults.
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

    /// Author a local op (`op_json`) stamped at `ts`. Returns the [`AuthorWire`]
    /// JSON, or `{error}` on malformed input.
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

    /// Apply a REMOTE op (`op_json`) honoring transient ownership. Returns
    /// `"true"` when applied, `"false"` when dropped, or `{error}` on bad input.
    pub fn apply_remote(&mut self, op_json: &str) -> String {
        let op: ObjectOp = match parse("op", op_json) {
            Ok(v) => v,
            Err(e) => return e,
        };
        ok_json(&self.engine.apply_remote(op))
    }

    /// Reconcile an ack for `op_ids` (a JSON `OpId[]`): release ownership and
    /// advance the base revision (`revision < 0` = none). Returns the removed
    /// `OpId[]` JSON so the shell drops the rows, or `{error}` on bad input.
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

    /// Reconcile a rejected op for `op_ids` (a JSON `OpId[]`): release ownership.
    /// Returns the removed `OpId[]` JSON, or `{error}` on bad input.
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

    /// Reconnect reconcile: reset the base to `snapshot_json`, seed the outbox
    /// from `persisted_entries_json` (the durable `WireOp[]` the shell read back),
    /// then replay them on top. Returns `null` or `{error}`; the shell drains the
    /// replayed batch with [`take_pending`](Self::take_pending).
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

    /// Flush the buffered coalesced frame immediately (gesture end / shutdown);
    /// the shell ships a following [`take_pending`](Self::take_pending).
    pub fn flush(&mut self) {
        self.engine.flush();
    }

    /// The shell's coalescing timer fired: drain the pending buffer into one frame.
    pub fn on_flush_due(&mut self) {
        self.engine.on_flush_due();
    }

    /// Drain the flushed envelopes as `WireOp[]` JSON; empty when nothing pending.
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

    /// True while the shell must keep its coalescing timer armed.
    pub fn flush_armed(&self) -> bool {
        self.engine.flush_armed()
    }

    /// Number of currently-unacked outbox entries.
    pub fn outbox_len(&self) -> u32 {
        u32::try_from(self.engine.outbox_len()).unwrap_or(u32::MAX)
    }

    /// The `(object,field)` keys held under transient ownership, sorted
    /// `string[]` JSON.
    pub fn owned_key_set(&self) -> String {
        ok_json(&self.engine.owned_key_set())
    }

    /// Drain the `(object,field)` keys that SETTLED since the last drain (an
    /// ack/reject released the last unacked write) as `string[]` JSON. The shell
    /// clears the matching optimistic preview off this signal — it is told which
    /// previews settled, instead of value-comparing a committed transform.
    pub fn take_settled_keys(&mut self) -> String {
        ok_json(&self.engine.take_settled_keys())
    }

    // --- peer presence registry ----------------------------------------------

    /// Ingest one presence `payload_json` frame, stamping `now_ms`. Returns
    /// `"true"` when it updated the registry, `"false"` otherwise, or `{error}`.
    pub fn ingest_presence(&mut self, payload_json: &str, now_ms: f64) -> String {
        let payload: serde_json::Value = match parse("presence payload", payload_json) {
            Ok(v) => v,
            Err(e) => return e,
        };
        ok_json(&self.peers.ingest(&payload, ms(now_ms)))
    }

    /// Drop peers older than the TTL relative to `now_ms`. Returns `"true"` when
    /// any peer was removed (so the shell re-emits).
    pub fn expire_peers(&mut self, now_ms: f64) -> String {
        ok_json(&self.peers.expire(ms(now_ms)))
    }

    /// The live peers as `PeerPresence[]` JSON, stable-ordered by `userId`.
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

/// wasm-bindgen wrapper over [`WindowState`](crate::scene_client::WindowState):
/// the viewport-windowing decision layer. Bboxes cross the boundary as
/// `{x,y,width,height}` JSON; an empty string / `"null"` is whole-canvas (`None`).
#[wasm_bindgen]
pub struct WasmWindow {
    state: WindowState,
}

#[wasm_bindgen]
impl WasmWindow {
    /// Seed from the connect region's bbox JSON (`""` = whole canvas) with an
    /// explicit margin (`margin < 0` uses the default).
    #[wasm_bindgen(constructor)]
    pub fn new(seed_bbox_json: &str, margin: f64) -> Result<WasmWindow, String> {
        let seed = parse_window_bbox("seed bbox", seed_bbox_json)?;
        let state = if margin >= 0.0 {
            WindowState::with_margin(seed, margin)
        } else {
            WindowState::new(seed)
        };
        Ok(WasmWindow { state })
    }

    /// Grow `viewport_json` into the window bbox and decide whether to re-aim.
    /// Returns the bbox JSON to `subscribe` to, `"null"` when unchanged, or
    /// `{error}` on bad input.
    pub fn on_viewport(&mut self, viewport_json: &str) -> String {
        let viewport: Bbox = match parse("viewport", viewport_json) {
            Ok(v) => v,
            Err(e) => return e,
        };
        ok_json(&self.state.on_viewport(viewport))
    }

    /// Re-aim the window to `bbox_json` directly (no margin). Returns the bbox
    /// JSON to `subscribe` to, `"null"` when unchanged, or `{error}` on bad input.
    pub fn set_window(&mut self, bbox_json: &str) -> String {
        let bbox: Bbox = match parse("bbox", bbox_json) {
            Ok(v) => v,
            Err(e) => return e,
        };
        ok_json(&self.state.set_window(bbox))
    }

    /// Drop the window: returns `true` when a whole-canvas `subscribe` must be
    /// sent, `false` when already whole-canvas.
    pub fn subscribe_whole_canvas(&mut self) -> bool {
        self.state.subscribe_whole_canvas()
    }

    /// The window bbox currently subscribed as JSON, or `"null"` for whole-canvas.
    pub fn current_window(&self) -> String {
        ok_json(&self.state.current_window())
    }
}

/// Parse an optional window bbox: `""` is whole-canvas (`None`), otherwise the
/// `{x,y,width,height}` JSON.
fn parse_window_bbox(label: &str, json: &str) -> Result<Option<Bbox>, String> {
    if json.is_empty() {
        return Ok(None);
    }
    parse(label, json).map(Some)
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
