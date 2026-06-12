//! Server-authoritative sync primitives the canvas actor composes to be the
//! single authority for an ordered, idempotent op stream. Transport- and
//! storage-agnostic: [`ws`](crate::ws) maps the wire envelope on, and
//! [`canvas_actor`](crate::canvas_actor) drives it against the object core.
//!
//! - opId dedup: a replayed [`OpId`] short-circuits to its stored ack without
//!   re-running apply, so the scene and server seq never move twice for one op.
//! - journal recovery: every applied op is journaled and written through
//!   region-indexed; on spawn the actor replays the journal tail past the last
//!   write-through so ops survive a crash.
//! - per-property LWW: the convergence is `apply_object_op_lww` in scene-core,
//!   keyed by the server's monotonic `seq`; the actor only stamps the seq.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use shape_scene_core::object::ObjectOp;

/// `(clientId, localSeq)` idempotency key. Redefined here (vs
/// [`shape_scene_core::OpId`]) so the WS envelope owns its own serde and never
/// couples to scene-core's `WireOp` shape.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpId {
    pub client_id: String,
    pub local_seq: i64,
}

/// One op on the WS `ops` array: an envelope around a whole [`ObjectOp`] with its
/// idempotency key, authored-against revision, and client clock.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpEnvelope {
    pub op_id: OpId,
    pub base_revision: i64,
    #[serde(default)]
    pub ts: String,
    pub op: ObjectOp,
}

/// Stored per [`OpId`] so a replay returns the original ack without re-applying.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpAck {
    pub seq: i64,
    pub revision: i64,
}

/// The recovery record per applied op: the exact [`ObjectOp`] to replay, its
/// `seq`, `opId` (to rebuild dedup), and authored-against `baseRevision`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalEntry {
    pub seq: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op_id: Option<OpId>,
    pub base_revision: i64,
    pub op: ObjectOp,
}

/// Per-canvas seen-opId table. Bounded by [`MAX_SEEN_OPS`]; on overflow the
/// oldest ids drop (a replay of an evicted op re-applies — acceptable for a
/// session guard, the journal is the durable record).
#[derive(Debug, Default)]
pub struct DedupTable {
    acks: HashMap<OpId, OpAck>,
    /// Insertion order, for bounded FIFO eviction.
    order: std::collections::VecDeque<OpId>,
}

pub const MAX_SEEN_OPS: usize = 4096;

impl DedupTable {
    pub fn new() -> Self {
        DedupTable::default()
    }

    /// The ack a previously-seen op produced, if this op is a duplicate.
    pub fn seen(&self, op_id: &OpId) -> Option<OpAck> {
        self.acks.get(op_id).copied()
    }

    /// Idempotent: re-recording the same id keeps the original ack.
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

    pub fn len(&self) -> usize {
        self.acks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.acks.is_empty()
    }
}

pub const CHECKPOINT_INTERVAL: i64 = 32;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedup_returns_original_ack_for_replay() {
        let mut table = DedupTable::new();
        let id = OpId { client_id: "c1".to_string(), local_seq: 7 };
        assert!(table.seen(&id).is_none());

        table.record(id.clone(), OpAck { seq: 12, revision: 12 });
        let again = table.seen(&id).expect("op is now seen");
        assert_eq!(again, OpAck { seq: 12, revision: 12 });

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
