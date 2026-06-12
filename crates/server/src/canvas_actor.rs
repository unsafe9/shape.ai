//! Per-canvas actor: a tokio task that owns one canvas's object scene.
//!
//! Each [`CanvasActor`] serializes every edit through an mpsc command channel, so
//! its [`ObjectScene`] is never touched concurrently. Canvas logic is not
//! reimplemented here: the actor drives [`ObjectStore`] (which calls scene-core's
//! `apply_object_op_lww`) and only orchestrates the server sequence, journaling,
//! dedup, and fan-out. Storage is shared via one [`RedbAdapter`] behind a `Mutex`,
//! with every `Record` id namespaced by `canvasId`.
//!
//! Convergence is server-authoritative: the actor stamps a strictly-increasing
//! `seq` per op (last arrival wins per property). An op that loses the LWW race
//! for a property applies as a no-op and persists nothing.
//!
//! The working set is bounded: every applied op writes through region-indexed
//! immediately, so any reloaded object is exact-to-the-last-op; the journal
//! records every op only to recover the in-flight tail past a crash. Feature
//! frames are lowered to [`ObjectOp`]s through the same op-apply path, so they
//! never grow a second way to mutate the scene.

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

pub type SharedStore = Arc<Mutex<RedbAdapter>>;

/// Delegates [`StorageAdapter`] + [`SpatialStore`] to the shared [`RedbAdapter`]
/// behind the `Mutex`. Cursor methods collect under the lock and return an owned
/// iterator, so the lock is never held across the cursor's lifetime.
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

#[derive(Clone, Debug, PartialEq)]
pub enum ApplyResult {
    Applied {
        seq: i64,
        revision: i64,
    },
    /// scene-core rejected the op; nothing was persisted or broadcast.
    Rejected { errors: Vec<String> },
}

#[derive(Clone, Debug)]
pub struct PatchBroadcast {
    pub seq: i64,
    pub op: ObjectOp,
    pub scene: ObjectScene,
    /// Author of this op. The WS fan-out skips echoing back to the originating
    /// connection, which already applied optimistically. `None` for
    /// server-internal ops.
    pub author: Option<String>,
}

pub enum CanvasCommand {
    ApplyEnvelope {
        envelope: OpEnvelope,
        actor_user_id: String,
        reply: oneshot::Sender<ApplyResult>,
    },
    /// Bare op (no envelope/dedup): the object MCP write path and tests.
    ApplyOp {
        op: ObjectOp,
        actor_user_id: String,
        reply: oneshot::Sender<ApplyResult>,
    },
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

#[derive(Clone)]
pub struct ActorHandle {
    tx: mpsc::Sender<CanvasCommand>,
    broadcast_tx: broadcast::Sender<PatchBroadcast>,
}

impl ActorHandle {
    /// A duplicate `opId` returns the original ack without re-applying.
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

    /// Bare [`ObjectOp`] (no dedup envelope): object MCP tools and tests.
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

    pub async fn get_scene(&self) -> ObjectScene {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(CanvasCommand::GetScene { reply })
            .await
            .expect("canvas actor task dropped");
        rx.await.expect("canvas actor dropped reply")
    }

    /// `None` window = whole canvas.
    pub async fn get_scene_region(&self, window: Option<RegionWindow>) -> ObjectScene {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(CanvasCommand::GetSceneRegion { window, reply })
            .await
            .expect("canvas actor task dropped");
        rx.await.expect("canvas actor dropped reply")
    }

    pub fn subscribe(&self) -> broadcast::Receiver<PatchBroadcast> {
        self.broadcast_tx.subscribe()
    }

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

pub struct CanvasActor {
    canvas_id: CanvasId,
    store: ObjectStore<SharedRedb>,
    shared: SharedStore,
    /// Monotonic server sequence; bumped once per accepted op.
    seq: i64,
    /// Seen-opId dedup table; bounded, rebuilt from the journal suffix on recovery.
    dedup: DedupTable,
    rx: mpsc::Receiver<CanvasCommand>,
    broadcast_tx: broadcast::Sender<PatchBroadcast>,
}

/// Deterministic timestamp seam: scene-core stays ambient-time free, so the
/// actor injects `now`.
fn now() -> String {
    "1970-01-01T00:00:00Z".to_string()
}

fn journal_record_id(canvas_id: &CanvasId, seq: i64) -> String {
    format!("{canvas_id}:journal:{seq}")
}

impl CanvasActor {
    /// Spawn the actor task for `canvas_id`, loading durable state from `store`.
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
                    // A windowed read hits the region index, covering cold objects
                    // without pulling the whole canvas resident.
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
                    // Every op writes through region-indexed, so shutdown is
                    // durable without a final checkpoint.
                    let _ = reply.send(());
                    break;
                }
            }
        }
    }

    fn revision(&mut self) -> i64 {
        self.store
            .scene(&self.canvas_id)
            .expect("scene loads")
            .scene_version
    }

    /// Dedup by `opId` first: a duplicate returns the original ack without
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

    /// Apply one op (LWW at the assigned seq), then on success record the dedup
    /// ack, journal the op, and broadcast. `op_id` is `Some` for the envelope
    /// path, `None` for the bare-op path.
    fn handle_apply(
        &mut self,
        op: ObjectOp,
        base_revision: i64,
        op_id: Option<OpId>,
        actor_user_id: &str,
    ) -> ApplyResult {
        let seq = self.seq + 1;
        let seq_u64 = u64::try_from(seq).unwrap_or(u64::MAX);

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

    /// Lower a Feature request to ops via `handle_feature` against a scratch scene,
    /// then re-drive each lowered op through the real apply path so it is
    /// sequenced, persisted, and broadcast. A read-only Feature yields no ops.
    fn handle_feature(&mut self, request: FeatureRequest, actor_user_id: &str) -> FeatureResponse {
        let mut scratch = self
            .store
            .scene(&self.canvas_id)
            .expect("scene loads")
            .clone();
        let seq_snapshot = self.seq;
        let revision_snapshot = self.revision();
        let now_fn = now;
        let mut ctx = FeatureCtx {
            now: &now_fn,
            seq: u64::try_from(seq_snapshot).unwrap_or(u64::MAX),
            revision: u64::try_from(revision_snapshot).unwrap_or(u64::MAX),
        };
        let (ops, response) = handle_feature(request, &mut scratch, &mut ctx);

        for op in ops {
            let base_revision = self.revision();
            self.handle_apply(op, base_revision, None, actor_user_id);
        }
        response
    }

    /// Append `entry` to the durable journal. Objects are already written through
    /// region-indexed, so the journal only rebuilds the dedup table and replays
    /// the in-flight tail past a crash.
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

/// Recover a canvas: hydrate the scene from its per-object Records, set the seq
/// to the canvas revision, then replay every journal entry past that revision so
/// ops journaled after the last write-through survive a crash. The dedup table is
/// rebuilt from a bounded journal suffix.
///
/// Per-object Records are exact-to-the-last-op (every op writes through), so the
/// tail replay only covers the normally-empty window where the journal write
/// landed but a crash followed. Replay is harmless: `apply_object_op_lww` at the
/// same seq is idempotent (equal-seq writes lose the LWW race).
fn recover_durable_state(
    canvas_id: &CanvasId,
    store: &SharedStore,
    object_store: &mut ObjectStore<SharedRedb>,
) -> (i64, DedupTable) {
    let loaded_revision = object_store
        .scene(canvas_id)
        .expect("scene loads")
        .scene_version;

    let entries = load_journal(canvas_id, store);
    let mut seq = entries.iter().map(|e| e.seq).max().unwrap_or(0);

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

/// Rebuild the dedup table from the journal's last [`MAX_SEEN_OPS`] entries, so
/// idempotent re-apply keeps working across a restart.
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

/// Every journal entry for `canvas_id`, in ascending seq order.
fn load_journal(canvas_id: &CanvasId, store: &SharedStore) -> Vec<JournalEntry> {
    let store = store.lock().expect("storage mutex poisoned");
    let prefix = format!("{canvas_id}:journal:");
    let mut ids: Vec<String> = store
        .list()
        .expect("journal ids list")
        .into_iter()
        .filter(|id| id.starts_with(&prefix))
        .collect();
    // Sort by numeric seq suffix; a lexical sort of "...:10" vs "...:2" misorders.
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

fn journal_seq_of(id: &str, prefix: &str) -> i64 {
    id.strip_prefix(prefix)
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0)
}
