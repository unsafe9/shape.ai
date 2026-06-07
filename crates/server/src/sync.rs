//! Server-authoritative sync primitives (MG4.1, MG4.2, MG4.4, MG4.6).
//!
//! This module holds the pieces the canvas actor composes to become the single
//! authority for an ordered, idempotent, convergent op stream. It is transport-
//! and storage-agnostic on purpose: [`ws`](crate::ws) maps the wire envelope onto
//! it and [`canvas_actor`](crate::canvas_actor) drives it against scene-core.
//!
//! The four primitives, and how they compose with scene-core's pure apply:
//!
//! 1. **opId dedup (MG4.2).** Each client op carries an [`OpId`] (`clientId` +
//!    `localSeq`). The actor keeps a [`DedupTable`] of seen ids mapped to the
//!    ack the op originally produced. A replay short-circuits to that stored ack
//!    and does NOT re-run apply, so the scene and server seq never move twice for
//!    one logical op (idempotent).
//!
//! 2. **journal/checkpoint recovery (MG4.1).** The actor journals every applied
//!    op as an [`OpEnvelope`] (which records `opId` + `baseRevision`, the
//!    "authored against" revision) and checkpoints the whole scene every
//!    [`CHECKPOINT_INTERVAL`] ops. On spawn it loads the newest checkpoint and
//!    REPLAYS the journal tail (`seq > checkpoint.seq`) through scene-core apply,
//!    so ops journaled after the last checkpoint survive a crash.
//!
//! 3. **per-property LWW (MG4.4 remote side).** scene-core's
//!    [`PropertyStore`](shape_scene_core::PropertyStore) is the conflict-
//!    resolution primitive: the actor records each property write keyed by the
//!    server's monotonic `seq`, so a later op with a *lower* seq cannot clobber a
//!    property a higher-seq op already won. Apply + LWW compose by *layering*:
//!    scene-core's pure apply produces the next whole scene (last-writer-by-
//!    arrival within that single op), while the [`PropertyStore`] is the durable,
//!    per-property authority consulted when concurrent writers race — the actual
//!    multi-writer convergence is exercised in MG-6; here we build and unit-test
//!    the seq-ordered resolution path.
//!
//! 4. **fractional ordering (MG4.6, additive).** When the server applies a
//!    create op it assigns a deterministic fractional order key (via scene-core
//!    [`generate_key_between`](shape_scene_core::generate_key_between)) into the
//!    created object's `meta["orderKey"]`. This is purely additive: the numeric
//!    `zIndex` field is untouched and the scene-core model is unchanged, so the
//!    golden vectors and renderer keep working. Replacing `zIndex` with the
//!    fractional key end-to-end (renderer + golden re-baseline) is a deferred
//!    follow-up.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use shape_scene_core::{generate_key_between, RenderScenePatch, Scene};

/// `(clientId, localSeq)` idempotency key assigned at the transport boundary.
///
/// Structurally identical to [`shape_scene_core::OpId`]; redefined here so the
/// server-crate WS envelope owns its own serde and never couples to scene-core's
/// granular `WireOp` shape (which is a separate, MG-6 wire form).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpId {
    pub client_id: String,
    pub local_seq: i64,
}

/// One op as it travels over the WS `ops` array: an ENVELOPE around a whole
/// [`RenderScenePatch`], stamped with its idempotency key, the revision it was
/// authored against, and a client clock.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpEnvelope {
    pub op_id: OpId,
    pub base_revision: i64,
    #[serde(default)]
    pub ts: String,
    pub patch: RenderScenePatch,
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
/// Unlike scene-core's [`OperationEnvelope`](shape_scene_core::OperationEnvelope)
/// (audit/undo metadata), this is the *recovery* record: it carries the exact
/// [`RenderScenePatch`] to REPLAY, the server `seq` it was assigned, its `opId`
/// (so dedup state can be rebuilt), and the `baseRevision` it was authored
/// against. Comment ops have no render patch and are not replayed from here.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalEntry {
    pub seq: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op_id: Option<OpId>,
    pub base_revision: i64,
    /// The render patch to replay on recovery. `None` for non-render ops
    /// (e.g. add-comment), which are reconstructed from the checkpoint only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch: Option<RenderScenePatch>,
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

/// Checkpoint cadence: the actor rewrites the whole-scene checkpoint every this
/// many APPLIED ops. Between checkpoints, recovery replays the journal tail.
pub const CHECKPOINT_INTERVAL: i64 = 32;

/// The existing `meta["orderKey"]` of every object that participates in the same
/// ordering bucket as a new object of `patch`'s kind, sorted by key.
///
/// Buckets are intentionally coarse for MG4.6: groups order among groups, cards
/// among cards, edges among edges. The order key is read from `meta["orderKey"]`
/// (absent objects predate fractional ordering and are skipped — they sort by the
/// legacy `zIndex` until a follow-up backfills them).
fn sibling_order_keys(scene: &Scene, patch: &RenderScenePatch) -> Vec<String> {
    let mut keys: Vec<String> = match patch {
        RenderScenePatch::CreateGroup { .. } => scene
            .groups
            .iter()
            .filter_map(|g| order_key_of(g.meta.as_ref()))
            .collect(),
        RenderScenePatch::CreateCard { .. } => scene
            .nodes
            .iter()
            .filter_map(|n| order_key_of(n.meta.as_ref()))
            .collect(),
        RenderScenePatch::CreateEdge { .. } => scene
            .edges
            .iter()
            .filter_map(|e| order_key_of(e.meta.as_ref()))
            .collect(),
        _ => vec![],
    };
    keys.sort();
    keys
}

/// Read `meta["orderKey"]` as a string, if present.
fn order_key_of(meta: Option<&shape_scene_core::ObjectMeta>) -> Option<String> {
    meta?.get("orderKey")?.as_str().map(|s| s.to_string())
}

/// Compute the fractional order key for a NEW object of `patch`'s kind, placing
/// it AFTER all current siblings (append semantics — newest on top).
///
/// Returns `None` for non-create ops (nothing to order). The key is deterministic
/// given the current siblings, so two servers replaying the same op stream derive
/// identical keys.
pub fn next_order_key(scene: &Scene, patch: &RenderScenePatch) -> Option<String> {
    match patch {
        RenderScenePatch::CreateGroup { .. }
        | RenderScenePatch::CreateCard { .. }
        | RenderScenePatch::CreateEdge { .. } => {
            let siblings = sibling_order_keys(scene, patch);
            let last = siblings.last().map(|s| s.as_str());
            // Append after the current last key (or first key if none exist).
            generate_key_between(last, None).ok()
        }
        _ => None,
    }
}

/// The id of the object a create op produces, so the actor can find it in the
/// post-apply scene and stamp its order key.
pub fn created_object_id(patch: &RenderScenePatch) -> Option<String> {
    match patch {
        RenderScenePatch::CreateGroup { group } => Some(group.id.clone()),
        RenderScenePatch::CreateCard { card } => Some(card.id.clone()),
        RenderScenePatch::CreateEdge { edge_id, .. } => Some(edge_id.clone()),
        _ => None,
    }
}

/// Stamp `order_key` into the `meta["orderKey"]` of object `object_id` in
/// `scene`, creating the meta map if absent. Searches groups, then nodes, then
/// edges (ids are globally unique within a scene). No-op if the id is absent.
pub fn stamp_order_key(scene: &mut Scene, object_id: &str, order_key: &str) {
    let value = serde_json::Value::String(order_key.to_string());
    if let Some(g) = scene.groups.iter_mut().find(|g| g.id == object_id) {
        g.meta.get_or_insert_with(Default::default).insert("orderKey".to_string(), value);
        return;
    }
    if let Some(n) = scene.nodes.iter_mut().find(|n| n.id == object_id) {
        n.meta.get_or_insert_with(Default::default).insert("orderKey".to_string(), value);
        return;
    }
    if let Some(e) = scene.edges.iter_mut().find(|e| e.id == object_id) {
        e.meta.get_or_insert_with(Default::default).insert("orderKey".to_string(), value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_scene_core::{PropertyStore, RenderCard, RenderGroup, SceneSelection, WorldRect};
    use serde_json::json;

    fn empty_scene() -> Scene {
        Scene {
            version: 1,
            scene_version: 0,
            groups: vec![],
            nodes: vec![],
            edges: vec![],
            tags: vec![],
            comments: vec![],
            artifacts: vec![],
            proposals: None,
            selection: SceneSelection::Canvas,
            updated_at: "t0".to_string(),
        }
    }

    fn create_group(id: &str) -> RenderScenePatch {
        RenderScenePatch::CreateGroup {
            group: RenderGroup {
                id: id.to_string(),
                title: "G".to_string(),
                summary: String::new(),
                bounds: WorldRect { x: 0.0, y: 0.0, width: 10.0, height: 10.0 },
                tag_ids: vec![],
                z_index: 0.0,
                style_key: String::new(),
            },
        }
    }

    fn create_card(id: &str, group_id: &str) -> RenderScenePatch {
        RenderScenePatch::CreateCard {
            card: RenderCard {
                id: id.to_string(),
                group_id: group_id.to_string(),
                title: "C".to_string(),
                summary: String::new(),
                detail: String::new(),
                status: String::new(),
                node_type: String::new(),
                bounds: WorldRect { x: 0.0, y: 0.0, width: 10.0, height: 10.0 },
                z_index: 0.0,
                style_key: String::new(),
                accessibility_label: String::new(),
            },
        }
    }

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

    // ---- fractional ordering --------------------------------------------------

    #[test]
    fn two_inserts_get_distinct_ordered_keys() {
        let mut scene = empty_scene();

        let p1 = create_group("g1");
        let k1 = next_order_key(&scene, &p1).expect("create gets a key");
        stamp_order_key_create(&mut scene, &p1, &k1);

        let p2 = create_group("g2");
        let k2 = next_order_key(&scene, &p2).expect("create gets a key");
        stamp_order_key_create(&mut scene, &p2, &k2);

        assert_ne!(k1, k2, "two inserts get distinct keys");
        assert!(k1 < k2, "second insert sorts after the first (append): {k1} < {k2}");
    }

    #[test]
    fn order_keys_bucket_by_kind() {
        // A card and a group both at the "first" slot get the same first key,
        // because each kind is an independent ordering bucket.
        let mut scene = empty_scene();
        let g = create_group("g1");
        let kg = next_order_key(&scene, &g).unwrap();
        stamp_order_key_create(&mut scene, &g, &kg);

        let c = create_card("n1", "g1");
        let kc = next_order_key(&scene, &c).unwrap();
        // Cards are empty so the card bucket's first key equals the group's.
        assert_eq!(kg, kc, "independent buckets both start at the first key");
    }

    #[test]
    fn non_create_ops_have_no_order_key() {
        let scene = empty_scene();
        let move_op = RenderScenePatch::MoveCard {
            id: "n1".to_string(),
            position: shape_scene_core::WorldPoint { x: 1.0, y: 2.0 },
        };
        assert!(next_order_key(&scene, &move_op).is_none());
        assert!(created_object_id(&move_op).is_none());
    }

    /// Helper: apply the create's order key by stamping a placeholder object into
    /// the scene (these unit tests don't run scene-core apply). Mirrors what the
    /// actor does post-apply, but inserts the sibling so the next call sees it.
    fn stamp_order_key_create(scene: &mut Scene, patch: &RenderScenePatch, key: &str) {
        use shape_scene_core::{Bounds, NodeStatus, NodeType, Point, SceneGroup, SceneNode, Size};
        let mut meta = serde_json::Map::new();
        meta.insert("orderKey".to_string(), json!(key));
        match patch {
            RenderScenePatch::CreateGroup { group } => scene.groups.push(SceneGroup {
                id: group.id.clone(),
                parent_group_id: None,
                title: group.title.clone(),
                summary: String::new(),
                bounds: Bounds { x: 0.0, y: 0.0, width: 10.0, height: 10.0 },
                tag_ids: vec![],
                z_index: 0.0,
                collapsed: false,
                created_at: "t0".to_string(),
                updated_at: "t0".to_string(),
                meta: Some(meta),
            }),
            RenderScenePatch::CreateCard { card } => scene.nodes.push(SceneNode {
                id: card.id.clone(),
                node_type: NodeType::Task,
                title: card.title.clone(),
                summary: String::new(),
                detail: String::new(),
                status: NodeStatus::Draft,
                confidence: 0.5,
                evidence_refs: vec![],
                child_decision_ids: vec![],
                group_id: card.group_id.clone(),
                position: Point { x: 0.0, y: 0.0 },
                size: Size { width: 10.0, height: 10.0 },
                z_index: 0.0,
                tag_ids: vec![],
                updated_at: None,
                meta: Some(meta),
            }),
            _ => {}
        }
    }

    // ---- LWW path (seq-ordered property resolution) ---------------------------

    #[test]
    fn property_store_resolves_by_server_seq() {
        // The actor consults a PropertyStore keyed by server seq: a later op with
        // a LOWER seq must not clobber a property a higher-seq op already won.
        let mut props = PropertyStore::new();

        // seq 5 writes title="from-5"; seq 8 writes title="from-8".
        assert!(props.apply("n1", "title", json!("from-5"), 5));
        assert!(props.apply("n1", "title", json!("from-8"), 8));
        assert_eq!(props.get_value("n1", "title"), Some(&json!("from-8")));

        // A straggler op carrying seq 6 (lower than the winning 8) loses.
        assert!(!props.apply("n1", "title", json!("straggler-6"), 6));
        assert_eq!(
            props.get_value("n1", "title"),
            Some(&json!("from-8")),
            "lower-seq write does not clobber the higher-seq winner"
        );
    }
}
