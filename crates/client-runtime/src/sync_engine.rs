//! Client object sync engine (port of `runtime/syncEngine.ts`): durable outbox +
//! optimistic apply + transient ownership/unacked discard + coalescing policy +
//! reconnect reconcile.
//!
//! The engine sits between the shell and the wire transport. It owns:
//!
//!   1. the durable outbox — every authored op is persisted (as the [`WireOp`]
//!      envelope) before it is sent, removed on ack, and replayed in `local_seq`
//!      order on (re)connect;
//!   2. the optimistic local `ObjectScene` — ops apply locally the instant they
//!      are authored, so the shell never waits for a round-trip;
//!   3. transient ownership / unacked discard — while a local write on an
//!      `(object, field)` is still unacked, an incoming REMOTE op for that same
//!      `(object, field)` is ignored, so a peer/echo can't clobber the value the
//!      user is still dragging;
//!   4. coalescing POLICY — rapid ops (continuous move/transform) are batched into
//!      one `ops` frame within a ~33ms window. This crate exposes the "flush due"
//!      decision ([`SyncEngine::flush_armed`] / [`SyncEngine::on_flush_due`]); the
//!      shell drives the actual timer;
//!   5. reconnect reconcile — on a fresh `welcome` snapshot the local base is
//!      reset to the snapshot and the outbox is replayed on top.
//!
//! The op-apply is THE scene-core object op-apply
//! ([`apply_object_op`]) — Rust-to-Rust, the same logic the server runs — applied
//! to a clone so a rejected op leaves the scene untouched.

use std::collections::HashMap;

use shape_scene_core::object::apply_object_op;
use shape_scene_core::object::model::ObjectScene;
use shape_scene_core::object::op::ObjectOp;
use shape_scene_core::wire::{OpId, WireOp};

use crate::outbox::{op_id_key, OutboxEntry, OutboxError, OutboxStore};

/// Default coalescing window (ms): rapid ops within this many ms ride one frame.
pub const COALESCE_MS: i64 = 33;

/// The narrow transport sink the engine drives. The shell implements it; the
/// engine never touches the socket directly. It receives a batch of [`WireOp`]
/// envelopes to send on the reliable channel.
pub trait EngineTransport {
    /// Send a batch of `WireOp` envelopes on the reliable channel.
    fn send_envelopes(&mut self, entries: &[OutboxEntry]);
}

/// Result of [`SyncEngine::author`]: errors (empty on success), the minted
/// `op_id`, and the captured inverse op (the undo entry, D21).
#[derive(Clone, Debug, PartialEq)]
pub struct AuthorResult {
    pub errors: Vec<String>,
    pub op_id: Option<OpId>,
    pub inverse: Option<ObjectOp>,
}

/// Granularity of transient ownership: per `(objectId, field)`.
///
/// Ownership protects an in-flight CONTINUOUS field edit (transform/geometry/
/// text/style) so a peer or self-echo can't clobber the value the user is still
/// authoring before our op is acked. A `set-transform` owns `(id, "transform")`;
/// a `set-text` owns `(id, "text")`; `edit-geometry` owns `(id, "geometry")`;
/// `set-style` owns `(id, "style")`.
///
/// Structural ops (insert/delete/reparent/reorder/tags/...) take NO ownership: a
/// later remote field edit on the same object is a legitimate concurrent change,
/// and create/delete conflicts are settled by the server's authoritative seq
/// ordering. A batch contributes the union of its members' field keys.
fn owned_keys(op: &ObjectOp) -> Vec<String> {
    match op {
        ObjectOp::SetTransform { id, .. } => vec![format!("{id}:transform")],
        ObjectOp::EditGeometry { id, .. } => vec![format!("{id}:geometry")],
        ObjectOp::SetText { id, .. } => vec![format!("{id}:text")],
        ObjectOp::SetStyle { id, .. } => vec![format!("{id}:style")],
        ObjectOp::Batch { ops } => ops.iter().flat_map(owned_keys).collect(),
        _ => Vec::new(),
    }
}

/// Does a remote op write any `(object, field)` key the client currently owns?
fn remote_touches_owned_key(op: &ObjectOp, owned: &HashMap<String, i64>) -> bool {
    if owned.is_empty() {
        return false;
    }
    owned_keys(op).iter().any(|key| owned.contains_key(key))
}

/// Build the `WireOp` envelope for an `ObjectOp`. The `prop_delta` carries the
/// full op; `object_id`/`kind` are descriptive (mirroring the server's
/// `op_to_wire`: the first target id, the op's serde tag).
fn wire_op(op: &ObjectOp, client_id: &str, local_seq: i64, base_revision: i64, ts: &str) -> WireOp {
    WireOp {
        op_id: OpId {
            client_id: client_id.to_string(),
            local_seq,
        },
        object_id: op.target_ids().first().cloned().unwrap_or_default(),
        kind: op_kind(op).to_string(),
        prop_delta: serde_json::to_value(op).expect("op serializes"),
        base_revision,
        actor: client_id.to_string(),
        ts: ts.to_string(),
    }
}

/// The kebab-case `kind` tag of an op, read off its serde tag.
fn op_kind(op: &ObjectOp) -> String {
    match serde_json::to_value(op) {
        Ok(serde_json::Value::Object(map)) => map
            .get("kind")
            .and_then(|k| k.as_str())
            .unwrap_or_default()
            .to_string(),
        _ => String::new(),
    }
}

pub struct SyncEngine<T: EngineTransport, S: OutboxStore> {
    client_id: String,
    outbox: S,
    transport: T,
    coalesce_ms: i64,

    /// Local optimistic scene the shell renders.
    scene: ObjectScene,
    /// Revision the next authored op is based on (server revision + local lead).
    base_revision: i64,

    /// Buffer of envelopes waiting on the coalescing flush.
    pending: Vec<OutboxEntry>,
    /// Whether the shell's coalescing timer is currently armed.
    timer_armed: bool,

    /// `(object,field)` keys with an unacked local write, -> count of owning ops.
    ownership: HashMap<String, i64>,
    /// `op_id` key -> the keys that op owns, so we release them on ack/reject.
    op_owned_keys: HashMap<String, Vec<String>>,
}

impl<T: EngineTransport, S: OutboxStore> SyncEngine<T, S> {
    /// Build an engine from an initial `welcome` scene. `coalesce_ms` defaults to
    /// [`COALESCE_MS`] when `None`.
    pub fn new(
        initial_scene: ObjectScene,
        client_id: impl Into<String>,
        outbox: S,
        transport: T,
        coalesce_ms: Option<i64>,
    ) -> Self {
        let base_revision = initial_scene.scene_version;
        Self {
            client_id: client_id.into(),
            outbox,
            transport,
            coalesce_ms: coalesce_ms.unwrap_or(COALESCE_MS),
            scene: initial_scene,
            base_revision,
            pending: Vec::new(),
            timer_armed: false,
            ownership: HashMap::new(),
            op_owned_keys: HashMap::new(),
        }
    }

    /// The coalescing window (ms) the shell should arm its timer for.
    pub fn coalesce_ms(&self) -> i64 {
        self.coalesce_ms
    }

    /// The current optimistic scene.
    pub fn scene(&self) -> &ObjectScene {
        &self.scene
    }

    /// The base revision the next authored op is stamped against.
    pub fn base_revision(&self) -> i64 {
        self.base_revision
    }

    /// The `(object,field)` keys currently held under transient ownership.
    pub fn owned_key_set(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.ownership.keys().cloned().collect();
        keys.sort();
        keys
    }

    /// True while the shell's coalescing timer must stay armed (a flush is due).
    pub fn flush_armed(&self) -> bool {
        self.timer_armed
    }

    /// Borrow the transport sink (e.g. so the shell can inspect what was sent).
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Mutable transport sink (e.g. to reset a test capture between phases).
    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    /// Number of currently-unacked outbox entries.
    pub fn outbox_len(&self) -> usize {
        self.outbox.all().map(|e| e.len()).unwrap_or(0)
    }

    /// Consume the engine and return its outbox store, so a fresh engine can be
    /// built on the SAME durable outbox to model a reconnect (the durable case).
    pub fn into_outbox(self) -> S {
        self.outbox
    }

    /// Author a local op: apply optimistically (to a clone, so a rejected op never
    /// touches the scene), take transient ownership of its keys, persist its
    /// `WireOp` envelope to the outbox, then buffer it for a coalesced send.
    /// Rejected-by-core ops never enter the outbox or the wire. `ts` is the
    /// envelope timestamp the shell stamps (injected clock).
    pub fn author(&mut self, op: ObjectOp, ts: &str) -> Result<AuthorResult, OutboxError> {
        let mut next = self.scene.clone();
        let inverse = match apply_object_op(&mut next, op.clone()) {
            Ok(inverse) => inverse,
            Err(e) => {
                return Ok(AuthorResult {
                    errors: vec![e.to_string()],
                    op_id: None,
                    inverse: None,
                })
            }
        };

        let local_seq = self.outbox.next_local_seq()?;
        let entry = wire_op(&op, &self.client_id, local_seq, self.base_revision, ts);
        let op_id = entry.op_id.clone();

        self.scene = next;
        self.take_ownership(&op_id, owned_keys(&op));
        self.outbox.append(entry.clone())?;
        self.enqueue(entry);
        Ok(AuthorResult {
            errors: Vec::new(),
            op_id: Some(op_id),
            inverse: Some(inverse),
        })
    }

    /// Apply a REMOTE op (peer or self-echo) to the optimistic scene, honoring
    /// transient ownership: a remote write to an `(object,field)` key we still own
    /// is dropped until our local op is acked. Returns true if applied.
    pub fn apply_remote(&mut self, op: ObjectOp) -> bool {
        if remote_touches_owned_key(&op, &self.ownership) {
            return false;
        }
        let mut next = self.scene.clone();
        if apply_object_op(&mut next, op).is_err() {
            return false;
        }
        self.scene = next;
        true
    }

    /// Reconcile an ack: drop the acked entries from the outbox, release their
    /// ownership, and advance the base revision. A duplicate ack is harmless.
    pub fn on_ack(&mut self, op_ids: &[OpId], revision: Option<i64>) -> Result<(), OutboxError> {
        self.outbox.remove(op_ids)?;
        for id in op_ids {
            self.release_ownership(id);
        }
        if let Some(rev) = revision {
            self.base_revision = self.base_revision.max(rev);
        }
        Ok(())
    }

    /// Reconcile a rejected op: drop it from the outbox and release its ownership
    /// so subsequent remote writes for those keys apply. The optimistic write
    /// stays in the local scene until the next snapshot/patch corrects it.
    pub fn on_rejected(&mut self, op_ids: &[OpId]) -> Result<(), OutboxError> {
        self.outbox.remove(op_ids)?;
        for id in op_ids {
            self.release_ownership(id);
        }
        Ok(())
    }

    /// Reconnect reconcile: reset the local base to a fresh `welcome` snapshot,
    /// then REPLAY every outbox entry (re-send unacked ops). Ownership is rebuilt
    /// from the replayed entries so transient ownership survives a reconnect. The
    /// snapshot is authoritative for everything NOT under a surviving unacked
    /// write.
    pub fn reconcile_snapshot(&mut self, snapshot: ObjectScene) -> Result<(), OutboxError> {
        self.base_revision = snapshot.scene_version;
        self.scene = snapshot;
        self.ownership.clear();
        self.op_owned_keys.clear();

        let entries = self.outbox.all()?;
        for entry in &entries {
            if let Ok(op) = serde_json::from_value::<ObjectOp>(entry.prop_delta.clone()) {
                let mut next = self.scene.clone();
                if apply_object_op(&mut next, op.clone()).is_ok() {
                    self.scene = next;
                }
                self.take_ownership(&entry.op_id, owned_keys(&op));
            }
        }
        if !entries.is_empty() {
            self.transport.send_envelopes(&entries);
        }
        Ok(())
    }

    /// Flush any buffered coalesced frame immediately (e.g. on gesture end /
    /// shutdown). Disarms the timer and sends whatever is pending.
    pub fn flush(&mut self) {
        self.timer_armed = false;
        if self.pending.is_empty() {
            return;
        }
        let batch = std::mem::take(&mut self.pending);
        self.transport.send_envelopes(&batch);
    }

    /// The shell calls this when its coalescing timer fires: drain the pending
    /// buffer into one `ops` frame. Disarms the timer.
    pub fn on_flush_due(&mut self) {
        self.timer_armed = false;
        if self.pending.is_empty() {
            return;
        }
        let batch = std::mem::take(&mut self.pending);
        self.transport.send_envelopes(&batch);
    }

    // --- internals -----------------------------------------------------------

    /// Buffer an envelope and arm the coalescing timer if not already armed.
    fn enqueue(&mut self, entry: OutboxEntry) {
        self.pending.push(entry);
        self.timer_armed = true;
    }

    fn take_ownership(&mut self, op_id: &OpId, keys: Vec<String>) {
        self.op_owned_keys.insert(op_id_key(op_id), keys.clone());
        for key in keys {
            *self.ownership.entry(key).or_insert(0) += 1;
        }
    }

    fn release_ownership(&mut self, op_id: &OpId) {
        let key = op_id_key(op_id);
        let Some(keys) = self.op_owned_keys.remove(&key) else {
            return;
        };
        for k in keys {
            let count = self.ownership.get(&k).copied().unwrap_or(0) - 1;
            if count <= 0 {
                self.ownership.remove(&k);
            } else {
                self.ownership.insert(k, count);
            }
        }
    }
}
