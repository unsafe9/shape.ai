//! Per-canvas actor (OB4.1): a tokio task that owns one canvas's object scene.
//!
//! Each [`CanvasActor`] serializes every edit to one canvas through an mpsc
//! command channel, so its [`ObjectScene`] is never touched concurrently. Canvas
//! logic itself is *not* reimplemented here: the actor drives the object store
//! ([`ObjectStore`]) which calls scene-core's single op-apply path
//! (`apply_object_op_lww`); the actor only orchestrates the server sequence,
//! durable journaling, idempotent dedup, and fan-out to subscribers.
//!
//! Storage is shared across canvases via a single [`RedbAdapter`] behind a
//! `Mutex`; the actor namespaces every `Record` id by `canvasId`. The object
//! store is given a thin [`SharedRedb`] adapter that locks the shared store on
//! each operation, so the store's per-canvas working set, per-property LWW gate,
//! region index, and per-op write-through are all reused unchanged.
//!
//! ## Server-authoritative convergence
//!
//! The actor serializes every op and stamps a strictly-increasing `seq`, so a
//! later-arriving op always carries a higher `seq` than an earlier one. The seq
//! is the authority — last arrival wins per property. The object store's
//! [`apply`](ObjectStore::apply) consults a per-canvas
//! [`PropertyStore`](shape_scene_core::PropertyStore) at that `seq`: an op that
//! lost the LWW race for a property applies as a no-op `Batch` and persists
//! nothing, leaving the higher-seq winner in place.
//!
//! ## Bounded working set
//!
//! `self.store` is never the whole canvas in memory: the [`ObjectStore`] holds a
//! per-canvas resident working set and the complete canvas always lives durably
//! in the region-indexed [`RedbAdapter`]. Every applied op writes through
//! immediately (region-indexed), so any object reloaded from the store is
//! exact-to-the-last-op. The journal records every op for crash recovery of the
//! in-flight tail.
//!
//! ## Feature channel
//!
//! Request/response Feature frames (comment upsert, template apply, canvas
//! switch, export) are lowered to [`ObjectOp`]s by
//! [`handle_feature`](crate::handle_feature) and pushed through the same op-apply
//! path, so the Feature channel never grows a second way to mutate the scene.

use std::sync::{Arc, Mutex};

use shape_scene_core::object::{FeatureRequest, FeatureResponse, ObjectOp, ObjectScene};
use shape_scene_core::CanvasId;
use shape_storage_core::{
    Record, RecordCursor, RedbAdapter, RegionKey, RegionWindow, SpatialStore, StorageAdapter,
    StorageError,
};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::object_feature::{handle_feature, FeatureCtx};
use crate::object_store::ObjectStore;
use crate::sync::{DedupTable, JournalEntry, OpAck, OpEnvelope, OpId, MAX_SEEN_OPS};

/// A single shared storage adapter, guarded so concurrent canvas actors can
/// persist into the same backing store without racing.
pub type SharedStore = Arc<Mutex<RedbAdapter>>;

/// A thin adapter that delegates [`StorageAdapter`] + [`SpatialStore`] to the
/// shared [`RedbAdapter`] behind the actor's `Mutex`. Cursor methods collect
/// under the lock and hand back an owned iterator, so the lock is never held
/// across the cursor's lifetime.
pub struct SharedRedb(SharedStore);

impl SharedRedb {
    fn lock(&self) -> std::sync::MutexGuard<'_, RedbAdapter> {
        self.0.lock().expect("storage mutex poisoned")
    }
}

impl StorageAdapter for SharedRedb {
    fn kind(&self) -> shape_storage_core::AdapterKind {
        self.lock().kind()
    }
    fn save(&mut self, record: Record) -> Result<(), StorageError> {
        self.lock().save(record)
    }
    fn load(&self, id: &str) -> Result<Record, StorageError> {
        self.lock().load(id)
    }
    fn delete(&mut self, id: &str) -> Result<bool, StorageError> {
        self.lock().delete(id)
    }
    fn list(&self) -> Result<Vec<String>, StorageError> {
        self.lock().list()
    }
    fn records(&self) -> Result<RecordCursor<'_>, StorageError> {
        let collected: Vec<Result<Record, StorageError>> = self.lock().records()?.collect();
        Ok(Box::new(collected.into_iter()))
    }
    fn snapshot(&self) -> Result<shape_storage_core::StoreSnapshot, StorageError> {
        self.lock().snapshot()
    }
    fn restore(&mut self, snapshot: shape_storage_core::StoreSnapshot) -> Result<(), StorageError> {
        self.lock().restore(snapshot)
    }
}

impl SpatialStore for SharedRedb {
    fn save_indexed(&mut self, record: Record, key: Option<RegionKey>) -> Result<(), StorageError> {
        self.lock().save_indexed(record, key)
    }
    fn query_region(
        &self,
        canvas_id: &str,
        bbox: Option<(f64, f64, f64, f64)>,
    ) -> Result<RecordCursor<'_>, StorageError> {
        let collected: Vec<Result<Record, StorageError>> =
            self.lock().query_region(canvas_id, bbox)?.collect();
        Ok(Box::new(collected.into_iter()))
    }
}

/// Outcome of an [`CanvasCommand::ApplyEnvelope`].
#[derive(Clone, Debug, PartialEq)]
pub enum ApplyResult {
    /// The op applied and was persisted.
    Applied {
        /// The actor's monotonic server sequence after this apply.
        seq: i64,
        /// The new scene revision (`scene.scene_version`) after this apply.
        revision: i64,
    },
    /// scene-core rejected the op; nothing was persisted or broadcast.
    Rejected { errors: Vec<String> },
}

/// What gets fanned out to subscribers when an op is applied.
#[derive(Clone, Debug)]
pub struct PatchBroadcast {
    /// The actor's server sequence assigned to this apply.
    pub seq: i64,
    /// The object op that was applied.
    pub op: ObjectOp,
    /// The full object scene after the apply. Whole-scene fan-out is fine here;
    /// a granular per-op delta is a later refinement.
    pub scene: ObjectScene,
    /// Who authored this op (the `userId` / `clientId` passed on the write path).
    /// The WS fan-out skips echoing a broadcast back to its originating
    /// connection — the originator already applied it optimistically. `None` for
    /// server-internal ops with no client author.
    pub author: Option<String>,
}

/// Commands the actor task accepts over its mpsc channel.
pub enum CanvasCommand {
    ApplyEnvelope {
        envelope: OpEnvelope,
        actor_user_id: String,
        reply: oneshot::Sender<ApplyResult>,
    },
    /// Apply a bare op (no envelope/dedup): the object MCP write path and tests.
    ApplyOp {
        op: ObjectOp,
        actor_user_id: String,
        reply: oneshot::Sender<ApplyResult>,
    },
    /// Lower + apply a Feature request through `handle_feature`.
    Feature {
        request: FeatureRequest,
        actor_user_id: String,
        reply: oneshot::Sender<FeatureResponse>,
    },
    GetScene {
        reply: oneshot::Sender<ObjectScene>,
    },
    GetSceneRegion {
        window: Option<RegionWindow>,
        reply: oneshot::Sender<ObjectScene>,
    },
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

/// A clone-able handle to a running [`CanvasActor`].
#[derive(Clone)]
pub struct ActorHandle {
    tx: mpsc::Sender<CanvasCommand>,
    broadcast_tx: broadcast::Sender<PatchBroadcast>,
}

impl ActorHandle {
    /// Apply an op ENVELOPE (MG4.2): carries an `opId` for idempotent dedup, the
    /// `baseRevision` it was authored against, and the op. A duplicate `opId`
    /// returns the ORIGINAL ack without re-applying.
    pub async fn apply_envelope(&self, envelope: OpEnvelope, user_id: &str) -> ApplyResult {
        let (reply, rx) = oneshot::channel();
        if self
            .tx
            .send(CanvasCommand::ApplyEnvelope {
                envelope,
                actor_user_id: user_id.to_string(),
                reply,
            })
            .await
            .is_err()
        {
            return ApplyResult::Rejected {
                errors: vec!["canvas actor unavailable".to_string()],
            };
        }
        rx.await.unwrap_or(ApplyResult::Rejected {
            errors: vec!["canvas actor dropped reply".to_string()],
        })
    }

    /// Apply a bare [`ObjectOp`] authored by `user_id` (no dedup envelope). Used
    /// by the object MCP tools and tests.
    pub async fn apply_op(&self, op: ObjectOp, user_id: &str) -> ApplyResult {
        let (reply, rx) = oneshot::channel();
        if self
            .tx
            .send(CanvasCommand::ApplyOp {
                op,
                actor_user_id: user_id.to_string(),
                reply,
            })
            .await
            .is_err()
        {
            return ApplyResult::Rejected {
                errors: vec!["canvas actor unavailable".to_string()],
            };
        }
        rx.await.unwrap_or(ApplyResult::Rejected {
            errors: vec!["canvas actor dropped reply".to_string()],
        })
    }

    /// Lower + apply a Feature request through the single op-apply path, returning
    /// the Feature response.
    pub async fn feature(&self, request: FeatureRequest, user_id: &str) -> FeatureResponse {
        let (reply, rx) = oneshot::channel();
        if self
            .tx
            .send(CanvasCommand::Feature {
                request,
                actor_user_id: user_id.to_string(),
                reply,
            })
            .await
            .is_err()
        {
            return FeatureResponse::FeatureError {
                request_id: None,
                message: "canvas actor unavailable".to_string(),
            };
        }
        rx.await.unwrap_or(FeatureResponse::FeatureError {
            request_id: None,
            message: "canvas actor dropped reply".to_string(),
        })
    }

    /// Snapshot the current object scene.
    pub async fn get_scene(&self) -> ObjectScene {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(CanvasCommand::GetScene { reply })
            .await
            .expect("canvas actor task dropped");
        rx.await.expect("canvas actor dropped reply")
    }

    /// Snapshot the scene filtered to a region window (`None` = whole canvas).
    pub async fn get_scene_region(&self, window: Option<RegionWindow>) -> ObjectScene {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(CanvasCommand::GetSceneRegion { window, reply })
            .await
            .expect("canvas actor task dropped");
        rx.await.expect("canvas actor dropped reply")
    }

    /// Subscribe to the fan-out of applied ops.
    pub fn subscribe(&self) -> broadcast::Receiver<PatchBroadcast> {
        self.broadcast_tx.subscribe()
    }

    /// Flush + stop the actor task.
    pub async fn shutdown(&self) {
        let (reply, rx) = oneshot::channel();
        if self
            .tx
            .send(CanvasCommand::Shutdown { reply })
            .await
            .is_ok()
        {
            let _ = rx.await;
        }
    }
}

/// The actor task body: owns one canvas's object store for `canvas_id`.
pub struct CanvasActor {
    canvas_id: CanvasId,
    store: ObjectStore<SharedRedb>,
    shared: SharedStore,
    /// Monotonic server sequence; bumped once per accepted op.
    seq: i64,
    /// Seen-opId table for idempotent dedup (MG4.2). Session-scoped + bounded;
    /// rebuilt from the journal suffix on recovery.
    dedup: DedupTable,
    rx: mpsc::Receiver<CanvasCommand>,
    broadcast_tx: broadcast::Sender<PatchBroadcast>,
}

/// A deterministic RFC3339-ish timestamp seam. scene-core stays ambient-time
/// free, so the actor injects `now`.
fn now() -> String {
    "1970-01-01T00:00:00Z".to_string()
}

/// The storage id of one journal entry (the durable op log).
fn journal_record_id(canvas_id: &CanvasId, seq: i64) -> String {
    format!("{canvas_id}:journal:{seq}")
}

impl CanvasActor {
    /// Spawn the actor task for `canvas_id`, loading durable state from `store`,
    /// and return a clone-able handle to it.
    pub fn spawn(canvas_id: CanvasId, store: SharedStore) -> ActorHandle {
        let (tx, rx) = mpsc::channel(64);
        let (broadcast_tx, _) = broadcast::channel(256);

        let mut object_store = ObjectStore::new(SharedRedb(Arc::clone(&store)));
        let (seq, dedup) = recover_durable_state(&canvas_id, &store, &mut object_store);

        let actor = CanvasActor {
            canvas_id,
            store: object_store,
            shared: store,
            seq,
            dedup,
            rx,
            broadcast_tx: broadcast_tx.clone(),
        };
        tokio::spawn(actor.run());

        ActorHandle { tx, broadcast_tx }
    }

    async fn run(mut self) {
        while let Some(cmd) = self.rx.recv().await {
            match cmd {
                CanvasCommand::ApplyEnvelope {
                    envelope,
                    actor_user_id,
                    reply,
                } => {
                    let result = self.handle_apply_envelope(envelope, &actor_user_id);
                    let _ = reply.send(result);
                }
                CanvasCommand::ApplyOp {
                    op,
                    actor_user_id,
                    reply,
                } => {
                    let base_revision = self.revision();
                    let result = self.handle_apply(op, base_revision, None, &actor_user_id);
                    let _ = reply.send(result);
                }
                CanvasCommand::Feature {
                    request,
                    actor_user_id,
                    reply,
                } => {
                    let response = self.handle_feature(request, &actor_user_id);
                    let _ = reply.send(response);
                }
                CanvasCommand::GetScene { reply } => {
                    let scene = self
                        .store
                        .scene(&self.canvas_id)
                        .expect("scene loads")
                        .clone();
                    let _ = reply.send(scene);
                }
                CanvasCommand::GetSceneRegion { window, reply } => {
                    // A windowed read goes straight to the region index, so it
                    // covers cold objects without pulling the whole canvas
                    // resident. The scene-level meta rides the working set.
                    let objects = self
                        .store
                        .query_region(&self.canvas_id, window)
                        .expect("region query");
                    let mut scene = self
                        .store
                        .scene(&self.canvas_id)
                        .expect("scene loads")
                        .clone();
                    scene.objects = objects;
                    let _ = reply.send(scene);
                }
                CanvasCommand::Shutdown { reply } => {
                    // Every op already writes through region-indexed, so a clean
                    // shutdown is durable without a final checkpoint.
                    let _ = reply.send(());
                    break;
                }
            }
        }
    }

    /// The current scene revision (`scene_version`).
    fn revision(&mut self) -> i64 {
        self.store
            .scene(&self.canvas_id)
            .expect("scene loads")
            .scene_version
    }

    /// Apply an op envelope (MG4.2): dedup by `opId` first, then apply.
    ///
    /// A duplicate `opId` is idempotent — it returns the ORIGINAL ack without
    /// re-running apply, so the scene and server seq do not move twice.
    fn handle_apply_envelope(&mut self, envelope: OpEnvelope, actor_user_id: &str) -> ApplyResult {
        if let Some(ack) = self.dedup.seen(&envelope.op_id) {
            return ApplyResult::Applied {
                seq: ack.seq,
                revision: ack.revision,
            };
        }
        self.handle_apply(
            envelope.op,
            envelope.base_revision,
            Some(envelope.op_id),
            actor_user_id,
        )
    }

    /// Apply one op through the object store (LWW at the assigned seq), then (on
    /// success) record the dedup ack, journal the op, and broadcast.
    ///
    /// `base_revision` is journaled as the "authored against" revision. `op_id`
    /// is `Some` for the envelope path (dedup) and `None` for the bare-op path.
    fn handle_apply(
        &mut self,
        op: ObjectOp,
        base_revision: i64,
        op_id: Option<OpId>,
        actor_user_id: &str,
    ) -> ApplyResult {
        let seq = self.seq + 1;
        let seq_u64 = u64::try_from(seq).unwrap_or(u64::MAX);

        // Canvas logic stays in scene-core; the store drives `apply_object_op_lww`
        // and writes the touched objects through region-indexed.
        match self.store.apply(&self.canvas_id, op.clone(), seq_u64) {
            Ok(_inverse) => {
                self.seq = seq;
            }
            Err(crate::object_store::ObjectStoreError::Apply(e)) => {
                return ApplyResult::Rejected {
                    errors: vec![e.to_string()],
                };
            }
            Err(e) => {
                return ApplyResult::Rejected {
                    errors: vec![e.to_string()],
                };
            }
        }

        // The journal entry records who authored the op (userId-only identity).
        self.journal(JournalEntry {
            seq,
            op_id: op_id.clone(),
            base_revision,
            op: op.clone(),
        });

        let scene = self
            .store
            .scene(&self.canvas_id)
            .expect("scene loads")
            .clone();
        let revision = scene.scene_version;

        if let Some(op_id) = op_id {
            self.dedup.record(op_id, OpAck { seq, revision });
        }

        let _ = self.broadcast_tx.send(PatchBroadcast {
            seq,
            op,
            scene,
            author: Some(actor_user_id.to_string()),
        });

        ApplyResult::Applied { seq, revision }
    }

    /// Lower a Feature request to ops via `handle_feature`, then drive each lowered
    /// op through the actor's apply path (so each is sequenced, persisted, and
    /// fanned out), and return the Feature response.
    fn handle_feature(&mut self, request: FeatureRequest, actor_user_id: &str) -> FeatureResponse {
        // `handle_feature` mutates a scratch scene to derive the ops; we then
        // re-drive those ops through the real apply path so they are journaled,
        // sequenced, and broadcast. A read-only Feature (canvas switch / export)
        // yields no ops and is answered directly.
        let mut scratch = self
            .store
            .scene(&self.canvas_id)
            .expect("scene loads")
            .clone();
        let seq_snapshot = self.seq;
        let revision_snapshot = self.revision();
        let mut alloc_counter = 0u64;
        let mut alloc = || {
            let id = format!("artifact-{seq_snapshot}-{alloc_counter}");
            alloc_counter += 1;
            id
        };
        let now_fn = now;
        let mut ctx = FeatureCtx {
            now: &now_fn,
            seq: u64::try_from(seq_snapshot).unwrap_or(u64::MAX),
            revision: u64::try_from(revision_snapshot).unwrap_or(u64::MAX),
            alloc_id: &mut alloc,
        };
        let (ops, response) = handle_feature(request, &mut scratch, &mut ctx);

        // Re-drive the lowered ops through the real apply path.
        for op in ops {
            let base_revision = self.revision();
            self.handle_apply(op, base_revision, None, actor_user_id);
        }
        response
    }

    /// Append `entry` to the durable journal (every op). The object store already
    /// wrote the touched objects through region-indexed, so the journal is only
    /// needed to rebuild the dedup table and replay any in-flight tail past a
    /// crash.
    fn journal(&mut self, entry: JournalEntry) {
        let payload = serde_json::to_vec(&entry).expect("journal entry serializes");
        let seq_u64 = u64::try_from(entry.seq).unwrap_or(u64::MAX);
        let mut store = self.shared.lock().expect("storage mutex poisoned");
        store
            .save(Record {
                id: journal_record_id(&self.canvas_id, entry.seq),
                kind: "journal".to_string(),
                version: seq_u64,
                payload,
            })
            .expect("journal entry persists");
    }
}

/// Recover a canvas: load the object scene from its per-object Records (the object
/// store hydrates lazily on first access), set the server seq to the canvas
/// revision, then REPLAY every journal entry with `seq` past the loaded scene's
/// revision so ops journaled after the last write-through survive a crash. The
/// dedup table is rebuilt from a bounded journal suffix.
///
/// Because every op writes through region-indexed, the per-object Records are
/// exact-to-the-last-op; the journal tail replay is the safety net for the
/// (normally empty) in-flight window where the journal write landed but a crash
/// followed. Replaying an already-persisted op is harmless: `apply_object_op_lww`
/// at the same seq is idempotent (equal-seq writes lose the LWW race).
fn recover_durable_state(
    canvas_id: &CanvasId,
    store: &SharedStore,
    object_store: &mut ObjectStore<SharedRedb>,
) -> (i64, DedupTable) {
    // Hydrate the scene (and its working set) from the per-object Records.
    let loaded_revision = object_store
        .scene(canvas_id)
        .expect("scene loads")
        .scene_version;

    let entries = load_journal(canvas_id, store);
    let mut seq = entries.iter().map(|e| e.seq).max().unwrap_or(0);

    // Replay journal entries whose op is not yet reflected in the loaded scene.
    // The loaded revision counts winning ops; replaying at the entry's seq is a
    // no-op when the store already holds the winner (equal/lower seq loses LWW).
    for entry in &entries {
        if entry.seq <= loaded_revision {
            continue;
        }
        let seq_u64 = u64::try_from(entry.seq).unwrap_or(u64::MAX);
        let _ = object_store.apply(canvas_id, entry.op.clone(), seq_u64);
    }
    if seq < loaded_revision {
        seq = loaded_revision;
    }

    let dedup = rebuild_dedup_from_suffix(&entries);
    (seq, dedup)
}

/// Rebuild the dedup table from the journal's last [`MAX_SEEN_OPS`] entries, in
/// seq order, recording each entry's `(opId -> ack)`. The journal is never pruned,
/// so the suffix is always available; this keeps idempotent re-apply working
/// across a restart.
fn rebuild_dedup_from_suffix(entries: &[JournalEntry]) -> DedupTable {
    let mut dedup = DedupTable::new();
    let start = entries.len().saturating_sub(MAX_SEEN_OPS);
    for entry in &entries[start..] {
        if let Some(op_id) = &entry.op_id {
            dedup.record(
                op_id.clone(),
                OpAck {
                    seq: entry.seq,
                    revision: entry.seq,
                },
            );
        }
    }
    dedup
}

/// Load every journal entry for `canvas_id`, in ascending seq order.
fn load_journal(canvas_id: &CanvasId, store: &SharedStore) -> Vec<JournalEntry> {
    let store = store.lock().expect("storage mutex poisoned");
    let prefix = format!("{canvas_id}:journal:");
    let mut ids: Vec<String> = store
        .list()
        .expect("journal ids list")
        .into_iter()
        .filter(|id| id.starts_with(&prefix))
        .collect();
    // Sort by the numeric seq suffix so replay is in true seq order (a lexical
    // sort of "...:10" vs "...:2" would misorder).
    ids.sort_by_key(|id| journal_seq_of(id, &prefix));

    let mut entries = Vec::with_capacity(ids.len());
    for id in ids {
        let record = store.load(&id).expect("journal record loads");
        let entry: JournalEntry =
            serde_json::from_slice(&record.payload).expect("journal entry deserializes");
        entries.push(entry);
    }
    entries
}

/// Parse the trailing `:journal:{seq}` integer from a journal record id.
fn journal_seq_of(id: &str, prefix: &str) -> i64 {
    id.strip_prefix(prefix)
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0)
}
