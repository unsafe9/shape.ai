//! Server-authoritative sync primitives (MG4.1, MG4.2) — object-native (OB4.1).
//!
//! This module holds the pieces the canvas actor composes to become the single
//! authority for an ordered, idempotent op stream. It is transport- and
//! storage-agnostic on purpose: [`ws`](crate::ws) maps the wire envelope onto it
//! and [`canvas_actor`](crate::canvas_actor) drives it against the object core.
//!
//! The primitives, and how they compose with the object op-apply path:
//!
//! 1. **opId dedup (MG4.2).** Each client op carries an [`OpId`] (`clientId` +
//!    `localSeq`). The actor keeps a [`DedupTable`] of seen ids mapped to the ack
//!    the op originally produced. A replay short-circuits to that stored ack and
//!    does NOT re-run apply, so the scene and server seq never move twice for one
//!    logical op (idempotent).
//!
//! 2. **journal/checkpoint recovery (MG4.1).** The actor journals every applied op
//!    as an [`OpEnvelope`] (which records `opId` + `baseRevision`, the "authored
//!    against" revision) and the [`ObjectStore`](crate::ObjectStore) writes the
//!    touched objects through region-indexed on every op. On spawn the actor loads
//!    the object scene from the store and REPLAYS the journal tail
//!    (`seq > checkpoint.seq`) through the object op-apply path, so ops journaled
//!    after the last write-through survive a crash.
//!
//! 3. **per-property LWW.** The object store's
//!    [`apply`](crate::ObjectStore::apply) consults a per-canvas
//!    [`PropertyStore`](shape_scene_core::PropertyStore) keyed by the server's
//!    monotonic `seq`, so a later op with a *lower* seq cannot clobber a property a
//!    higher-seq op already won. The actor only stamps the seq; the convergence is
//!    `apply_object_op_lww` in scene-core (the single op-apply path, P1).
//!
//! Fractional ordering is no longer a server concern: the object model carries an
//! explicit fractional `order` field minted by the caller (shell / object MCP),
//! so the server never stamps an order key.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use shape_scene_core::object::ObjectOp;

/// `(clientId, localSeq)` idempotency key assigned at the transport boundary.
///
/// Structurally identical to [`shape_scene_core::OpId`]; redefined here so the
/// server-crate WS envelope owns its own serde and never couples to scene-core's
/// granular `WireOp` shape.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpId {
    pub client_id: String,
    pub local_seq: i64,
}

/// One op as it travels over the WS `ops` array: an ENVELOPE around a whole
/// [`ObjectOp`], stamped with its idempotency key, the revision it was authored
/// against, and a client clock.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpEnvelope {
    pub op_id: OpId,
    pub base_revision: i64,
    #[serde(default)]
    pub ts: String,
    pub op: ObjectOp,
}

/// The ack an applied op produced: the server seq + scene revision after apply.
///
/// Stored per [`OpId`] so a replay returns the ORIGINAL ack without re-applying.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpAck {
    pub seq: i64,
    pub revision: i64,
}

/// The durable journal record the actor writes per applied op (MG4.1/MG4.2).
///
/// This is the *recovery* record: it carries the exact [`ObjectOp`] to REPLAY,
/// the server `seq` it was assigned, its `opId` (so dedup state can be rebuilt),
/// and the `baseRevision` it was authored against.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalEntry {
    pub seq: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op_id: Option<OpId>,
    pub base_revision: i64,
    /// The object op to replay on recovery.
    pub op: ObjectOp,
}

/// Per-canvas seen-opId table for idempotent dedup (MG4.2).
///
/// Maps each applied [`OpId`] to the ack it produced. Bounded for a session by
/// [`MAX_SEEN_OPS`]; on overflow the oldest-inserted ids are dropped (a replay of
/// a long-evicted op would re-apply, which is acceptable for a session-scoped
/// guard — the journal remains the durable record).
#[derive(Debug, Default)]
pub struct DedupTable {
    acks: HashMap<OpId, OpAck>,
    /// Insertion order, for bounded FIFO eviction.
    order: std::collections::VecDeque<OpId>,
}

/// How many distinct opIds the in-memory dedup table retains per canvas session.
pub const MAX_SEEN_OPS: usize = 4096;

impl DedupTable {
    pub fn new() -> Self {
        DedupTable::default()
    }

    /// The ack a previously-seen op produced, if this op is a duplicate.
    pub fn seen(&self, op_id: &OpId) -> Option<OpAck> {
        self.acks.get(op_id).copied()
    }

    /// Record that `op_id` applied and produced `ack`. Idempotent: re-recording
    /// the same id keeps the original ack and does not reorder eviction.
    pub fn record(&mut self, op_id: OpId, ack: OpAck) {
        if self.acks.contains_key(&op_id) {
            return;
        }
        self.acks.insert(op_id.clone(), ack);
        self.order.push_back(op_id);
        while self.order.len() > MAX_SEEN_OPS {
            if let Some(evicted) = self.order.pop_front() {
                self.acks.remove(&evicted);
            }
        }
    }

    /// Number of retained opIds (for tests/observability).
    pub fn len(&self) -> usize {
        self.acks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.acks.is_empty()
    }
}

/// Checkpoint cadence retained for journal-tail recovery framing. With the object
/// store writing every op through region-indexed, the per-object Records are
/// always exact-to-the-last-op; the journal still records every op so the dedup
/// table can be rebuilt from its suffix on recovery.
pub const CHECKPOINT_INTERVAL: i64 = 32;

#[cfg(test)]
mod tests {
    use super::*;

    // ---- dedup ----------------------------------------------------------------

    #[test]
    fn dedup_returns_original_ack_for_replay() {
        let mut table = DedupTable::new();
        let id = OpId { client_id: "c1".to_string(), local_seq: 7 };
        assert!(table.seen(&id).is_none());

        table.record(id.clone(), OpAck { seq: 12, revision: 12 });
        let again = table.seen(&id).expect("op is now seen");
        assert_eq!(again, OpAck { seq: 12, revision: 12 });

        // Re-recording keeps the original ack (idempotent).
        table.record(id.clone(), OpAck { seq: 99, revision: 99 });
        assert_eq!(table.seen(&id).unwrap(), OpAck { seq: 12, revision: 12 });
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn dedup_distinguishes_clients_and_local_seqs() {
        let mut table = DedupTable::new();
        let a = OpId { client_id: "c1".to_string(), local_seq: 1 };
        let b = OpId { client_id: "c2".to_string(), local_seq: 1 };
        let c = OpId { client_id: "c1".to_string(), local_seq: 2 };
        table.record(a.clone(), OpAck { seq: 1, revision: 1 });
        assert!(table.seen(&b).is_none(), "different client is a different op");
        assert!(table.seen(&c).is_none(), "different localSeq is a different op");
    }
}
