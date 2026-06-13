//! Client object sync engine between the shell and the wire transport. It owns:
//!
//!   1. the durable outbox — every authored op is persisted before send, removed
//!      on ack, and replayed in `local_seq` order on (re)connect;
//!   2. the optimistic local `ObjectScene` — ops apply the instant they're
//!      authored, so the shell never waits for a round-trip;
//!   3. transient ownership — while a local write on an `(object, field)` is
//!      unacked, a remote op for that same key is ignored, so a peer/echo can't
//!      clobber the value the user is still dragging;
//!   4. coalescing POLICY — rapid ops batch into one frame within a ~33ms window;
//!      the engine decides ([`flush_armed`]/[`on_flush_due`]), the shell times it;
//!   5. reconnect reconcile — a fresh `welcome` snapshot resets the base and
//!      replays the outbox on top.
//!
//! Op-apply is THE scene-core [`apply_object_op`] the server runs, applied to a
//! clone so a rejected op leaves the scene untouched.
//!
//! [`flush_armed`]: SyncEngine::flush_armed
//! [`on_flush_due`]: SyncEngine::on_flush_due

use std::collections::HashMap;

use shape_scene_core::object::apply_object_op;
use shape_scene_core::object::model::ObjectScene;
use shape_scene_core::object::op::ObjectOp;
use shape_scene_core::wire::{OpId, WireOp};

use crate::outbox::{op_id_key, OutboxEntry, OutboxError, OutboxStore};

/// Default coalescing window (ms): rapid ops within this many ms ride one frame.
pub const COALESCE_MS: i64 = 33;

/// The transport sink the engine drives. The shell implements it; the engine
/// never touches the socket directly.
pub trait EngineTransport {
    /// Send a batch of `WireOp` envelopes on the reliable channel.
    fn send_envelopes(&mut self, entries: &[OutboxEntry]);
}

/// Result of [`SyncEngine::author`]: errors (empty on success), the minted
/// `op_id`, and the captured inverse op (the undo entry).
#[derive(Clone, Debug, PartialEq)]
pub struct AuthorResult {
    pub errors: Vec<String>,
    pub op_id: Option<OpId>,
    pub inverse: Option<ObjectOp>,
}

/// The `(objectId, field)` keys an op takes transient ownership of, protecting
/// an in-flight continuous field edit (transform/geometry/text/style) until ack.
/// Structural ops (insert/delete/reparent/reorder/tags/...) take NO ownership —
/// those conflicts are settled by the server's authoritative seq ordering. A
/// batch contributes the union of its members' keys.
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

/// Build the `WireOp` envelope for an `ObjectOp`. `prop_delta` carries the full
/// op; `object_id`/`kind` are descriptive (first target id, the op's serde tag).
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

/// The op's kebab-case `kind` tag, read off its serde tag.
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

    /// Envelopes waiting on the coalescing flush.
    pending: Vec<OutboxEntry>,
    timer_armed: bool,

    /// `(object,field)` keys with an unacked local write -> count of owning ops.
    ownership: HashMap<String, i64>,
    /// `op_id` key -> the keys that op owns, released on ack/reject.
    op_owned_keys: HashMap<String, Vec<String>>,
    /// `(object,field)` keys whose LAST unacked write just settled (ack/reject
    /// released ownership), buffered until the shell drains them. This is the
    /// authoritative "this preview is now safe to clear" signal — settling is
    /// driven by the ack/reject event, never by comparing transform values.
    settled_keys: Vec<String>,
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
            settled_keys: Vec::new(),
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

    /// Drain the `(object,field)` keys that settled since the last drain — every
    /// key whose last unacked write was released by an ack/reject. The shell
    /// clears the matching optimistic preview off THIS signal, never off a
    /// transform-value compare (a coincidentally-equal peer write does not
    /// settle a key, so it is never reported here until the real ack lands).
    pub fn take_settled_keys(&mut self) -> Vec<String> {
        std::mem::take(&mut self.settled_keys)
    }

    /// True while the shell's coalescing timer must stay armed (a flush is due).
    pub fn flush_armed(&self) -> bool {
        self.timer_armed
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }

    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    /// Number of currently-unacked outbox entries.
    pub fn outbox_len(&self) -> usize {
        self.outbox.all().map(|e| e.len()).unwrap_or(0)
    }

    /// The persisted outbox entry for `op_id`, or `None`.
    pub fn outbox_entry(&self, op_id: &OpId) -> Option<OutboxEntry> {
        let key = op_id_key(op_id);
        self.outbox
            .all()
            .ok()?
            .into_iter()
            .find(|e| op_id_key(&e.op_id) == key)
    }

    /// Reseed the bookkeeping outbox from durable rows the shell read back, then
    /// run [`reconcile_snapshot`](Self::reconcile_snapshot) to replay them.
    pub fn reseed_outbox(&mut self, entries: Vec<OutboxEntry>) {
        let _ = self.outbox.reseed(entries);
    }

    /// Consume the engine, returning its outbox store so a fresh engine can be
    /// built on the SAME durable outbox to model a reconnect.
    pub fn into_outbox(self) -> S {
        self.outbox
    }

    /// Author a local op: apply optimistically (to a clone, so a rejected op
    /// leaves the scene untouched), take ownership of its keys, persist its
    /// `WireOp`, then buffer it for a coalesced send. Rejected-by-core ops never
    /// enter the outbox or wire. `ts` is the shell-stamped envelope timestamp.
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

        // An empty-Batch inverse means the op changed nothing (apply was a
        // no-op) — the same predicate the kernel returns and `UndoStack::record`
        // skips on. Author NOTHING: no `(object,field)` ownership, no outbox row,
        // no wire envelope. The local scene already equals `next` (unchanged), so
        // a no-op text commit can't grab `${id}:text` and gate a peer's edit.
        if matches!(&inverse, ObjectOp::Batch { ops } if ops.is_empty()) {
            return Ok(AuthorResult {
                errors: Vec::new(),
                op_id: None,
                inverse: Some(inverse),
            });
        }

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

    /// Apply a REMOTE op honoring transient ownership: a remote write to a key we
    /// still own is dropped until our local op is acked. Returns true if applied.
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

    /// Reconcile an ack: drop the acked entries, release their ownership, and
    /// advance the base revision. A duplicate ack is harmless.
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

    /// Reconcile a rejected op: drop it and release its ownership so later remote
    /// writes for those keys apply. The optimistic write stays in the local scene
    /// until the next snapshot/patch corrects it.
    pub fn on_rejected(&mut self, op_ids: &[OpId]) -> Result<(), OutboxError> {
        self.outbox.remove(op_ids)?;
        for id in op_ids {
            self.release_ownership(id);
        }
        Ok(())
    }

    /// Reconnect reconcile: reset the base to a fresh `welcome` snapshot, then
    /// REPLAY every outbox entry (re-sending unacked ops) and rebuild ownership
    /// from them. The snapshot is authoritative for everything NOT under a
    /// surviving unacked write.
    pub fn reconcile_snapshot(&mut self, snapshot: ObjectScene) -> Result<(), OutboxError> {
        // A welcome whose `scene_version` predates our last server-confirmed
        // revision is STALE: a windowed subscribe (pan/zoom) can race a just-acked
        // local mutation and return a snapshot generated before it landed. Adopting
        // it would regress acked state — e.g. resurrect a deleted object whose
        // Delete already left the outbox on ack, so the outbox replay below has
        // nothing left to re-remove it. `base_revision` advances only via `on_ack`
        // (server-assigned revisions) and this reconcile, so a legitimate reconnect
        // or window snapshot is always `>= base_revision`; only a racy stale welcome
        // falls below it. Ignore it — the server's current snapshot follows.
        if snapshot.scene_version < self.base_revision {
            return Ok(());
        }
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

    /// Flush any buffered coalesced frame immediately (gesture end / shutdown).
    /// Disarms the timer and sends whatever is pending.
    pub fn flush(&mut self) {
        self.timer_armed = false;
        if self.pending.is_empty() {
            return;
        }
        let batch = std::mem::take(&mut self.pending);
        self.transport.send_envelopes(&batch);
    }

    /// The shell's coalescing timer fired: drain the pending buffer into one
    /// frame. Disarms the timer.
    pub fn on_flush_due(&mut self) {
        self.timer_armed = false;
        if self.pending.is_empty() {
            return;
        }
        let batch = std::mem::take(&mut self.pending);
        self.transport.send_envelopes(&batch);
    }

    // --- internals -----------------------------------------------------------

    /// Buffer an envelope and arm the coalescing timer.
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
                // The last unacked write on this key is gone: the optimistic
                // preview the shell is holding can now safely clear.
                self.settled_keys.push(k);
            } else {
                self.ownership.insert(k, count);
            }
        }
    }
}
