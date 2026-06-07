//! Per-canvas actor (MG2.2): a tokio task that owns one canvas's working state.
//!
//! Each [`CanvasActor`] serializes every edit to one canvas through an mpsc
//! command channel, so its `Scene` is never touched concurrently. Canvas logic
//! itself is *not* reimplemented here: the actor calls
//! [`apply_render_patch_to_shape_scene`](shape_scene_core::apply_render_patch_to_shape_scene)
//! from scene-core and only orchestrates persistence (a scene checkpoint plus a
//! durable journal entry per applied op) and fan-out (a broadcast channel for
//! future WebSocket subscribers).
//!
//! Storage is shared across canvases via a single adapter behind a `Mutex`; the
//! actor namespaces its `Record` ids by `canvasId` so ids stay globally unique.
//!
//! MG-4 layers a server-authoritative sync engine on top (see [`crate::sync`]):
//! opId dedup (idempotent re-apply), journal-tail recovery past the last
//! checkpoint, a per-property LWW [`PropertyStore`] consulted for concurrent
//! property writes, and additive fractional order keys stamped into created
//! objects' `meta["orderKey"]`.
//!
//! MG-6.1 makes that LWW store AUTHORITATIVE on the apply path (Figma-style
//! server-authoritative convergence). The convergence rule, in one line: **the
//! server arrival `seq` is the authority — last arrival wins per property.** The
//! actor serializes every op and stamps a strictly-increasing `seq`, so a
//! later-arriving op always carries a higher `seq` than an earlier one. After
//! scene-core's pure apply (which blindly writes the op's value), the actor
//! CONSULTS the [`PropertyStore`] per touched property: if a strictly-higher-`seq`
//! write already won that property, the op LOSES that property and the actor
//! restores the winning value into the post-apply scene; otherwise the op wins
//! and its value is recorded at this op's `seq`. The op's `baseRevision` is the
//! LWW token compared against the held winner's `seq`, so a re-ordered or late op
//! that was authored against an older revision cannot resurrect a stale property
//! value. See [`CanvasActor::converge_lww`].
//!
//! MG-9.5 adds region-scoped reads ([`CanvasActor::get_scene_region`]): a windowed
//! subscriber is answered from the actor's resident working set, hydrated for the
//! requested window from the storage region index first so the read covers cold
//! objects too.
//!
//! MG2.2 / MG9.3 — bounded working set + cold LRU eviction (PC10/C10: no
//! full-canvas-in-memory assumption). `self.scene` is no longer the WHOLE canvas;
//! it is the **resident working set**: a small always-resident "spine" (scene
//! meta, tags, comments, artifacts, proposals) plus the placement objects
//! (groups/nodes/edges) that are in or near actively-touched/queried regions. The
//! complete canvas always lives durably in the per-object, region-indexed store
//! (`SpatialStore`), and the actor loads placement objects on demand.
//!
//! The blocker the old design named — "a reloaded cold region could miss
//! un-checkpointed tail ops" — is removed by **per-op write-through**: every
//! applied op checkpoints immediately (see [`CanvasActor::persist`]), upserting
//! the Records it touched (region-indexed) and deleting the Records it removed, so
//! any object reloaded from the store is exact-to-the-last-op. The journal still
//! records every op for crash recovery of the in-flight tail; the periodic
//! whole-scene checkpoint (and its recovery floor in `canvas-meta`) is kept.
//!
//! Two load-on-demand shapes:
//! * **Region read / windowed op** — hydrate only the queried window from the
//!   region index ([`CanvasActor::hydrate_region`]) and mark it hot.
//! * **Whole-scene op / read** (`get_scene`, bulk scene patch, create order-key
//!   append, anything that must see the full canvas) — transiently hydrate the
//!   full canvas ([`CanvasActor::hydrate_full`]), do the work, then
//!   [`CanvasActor::evict_cold`] trims the working set back to
//!   [`WORKING_SET_BUDGET`]. So a large canvas never *stays* resident: between ops
//!   and after region reads only the hot working set is held. This transient
//!   whole-canvas hydrate on a whole-scene write is the deliberate tradeoff that
//!   keeps every existing invariant (delete/reparent/LWW/order-key all see the
//!   complete scene) while still bounding steady-state memory.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use shape_scene_core::{
    add_shape_scene_comment, apply_render_patch_to_shape_scene, apply_scene_patch,
    bounds_intersect, node_bounds, update_shape_scene_comment, Bounds, CanvasId, PropertyStore,
    RenderScenePatch, Scene, SceneArtifact, SceneComment, ScenePatch, SceneSelection,
};
use shape_storage_core::{Record, SpatialStore, SqliteAdapter, StorageAdapter};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::scene_store::{
    canvas_meta_record_id, canvas_record_prefix, object_record_id, records_to_scene,
    scene_to_records, KIND_ARTIFACT, KIND_COMMENT, KIND_EDGE, KIND_GROUP, KIND_NODE, KIND_TAG,
};
use crate::sync::{
    created_object_id, next_order_key, stamp_order_key, DedupTable, JournalEntry, OpAck,
    OpEnvelope, OpId, MAX_SEEN_OPS,
};

/// A single shared storage adapter, guarded so concurrent canvas actors can
/// persist into the same backing store without racing.
pub type SharedStore = Arc<Mutex<SqliteAdapter>>;

/// How many placement objects (groups + nodes + edges) the resident working set
/// holds before [`CanvasActor::evict_cold`] starts dropping the least-recently-
/// used ones back to the durable store. The "spine" (scene meta, tags, comments,
/// artifacts, proposals) is always resident and is not counted against this
/// budget. A whole-scene op transiently exceeds it while hydrated, then trims.
pub const WORKING_SET_BUDGET: usize = 1024;

/// Outcome of an [`CanvasCommand::ApplyPatch`].
#[derive(Clone, Debug, PartialEq)]
pub enum ApplyResult {
    /// The patch applied and was persisted.
    Applied {
        /// The actor's monotonic server sequence after this apply.
        seq: i64,
        /// The new scene revision (`scene.scene_version`) after this apply.
        revision: i64,
    },
    /// scene-core rejected the patch; nothing was persisted or broadcast.
    Rejected { errors: Vec<String> },
}

/// Outcome of an [`CanvasCommand::AddComment`].
#[derive(Clone, Debug, PartialEq)]
pub enum CommentResult {
    /// The comment was added and persisted.
    Added { comment: SceneComment },
    /// scene-core rejected the comment (empty body / unknown target).
    Rejected { errors: Vec<String> },
}

/// Outcome of an [`CanvasCommand::AddArtifact`].
#[derive(Clone, Debug, PartialEq)]
pub enum ArtifactResult {
    /// The artifact was added and persisted.
    Added { artifact: SceneArtifact },
    /// The target group did not exist.
    Rejected { errors: Vec<String> },
}

/// What gets fanned out to subscribers when a patch is applied.
#[derive(Clone, Debug)]
pub struct PatchBroadcast {
    /// The actor's server sequence assigned to this apply.
    pub seq: i64,
    /// The op that was applied, as the canonical render patch.
    pub patch: RenderScenePatch,
    /// The full scene after the apply. Whole-scene fan-out is fine for MG-2;
    /// MG-6 narrows this to per-op deltas for the multiuser tail.
    pub scene: Scene,
    /// MG-6.1: who authored this op (the `userId` / `clientId` passed on the
    /// write path). The WS fan-out (see [`crate::ws`]) skips echoing a broadcast
    /// back to its originating connection — the originator already applied it
    /// optimistically. `None` for server-internal ops with no client author.
    pub author: Option<String>,
}

/// Commands the actor task accepts over its mpsc channel.
pub enum CanvasCommand {
    ApplyPatch {
        patch: RenderScenePatch,
        actor_user_id: String,
        reply: oneshot::Sender<ApplyResult>,
    },
    ApplyEnvelope {
        envelope: OpEnvelope,
        actor_user_id: String,
        reply: oneshot::Sender<ApplyResult>,
    },
    AddComment {
        target: SceneSelection,
        body: String,
        actor_user_id: String,
        reply: oneshot::Sender<CommentResult>,
    },
    UpdateComment {
        comment_id: String,
        body: Option<String>,
        resolved: Option<bool>,
        actor_user_id: String,
        reply: oneshot::Sender<CommentResult>,
    },
    ApplyScenePatch {
        patch: ScenePatch,
        actor_user_id: String,
        reply: oneshot::Sender<ApplyResult>,
    },
    AddArtifact {
        group_id: String,
        artifact: SceneArtifact,
        reply: oneshot::Sender<ArtifactResult>,
    },
    UpdateTag {
        tag: shape_scene_core::Tag,
        reply: oneshot::Sender<ApplyResult>,
    },
    DeleteTag {
        tag_id: String,
        reply: oneshot::Sender<ApplyResult>,
    },
    GetScene {
        reply: oneshot::Sender<Scene>,
    },
    GetSceneRegion {
        bbox: Option<Bounds>,
        reply: oneshot::Sender<Scene>,
    },
    /// MG2.2 observability: the current resident placement-object count (groups +
    /// nodes + edges in memory), WITHOUT hydrating. Lets a test prove cold
    /// eviction actually shed objects from the working set.
    ResidentCount {
        reply: oneshot::Sender<usize>,
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
    /// Apply a render patch authored by `user_id`. Returns the apply outcome.
    pub async fn apply_patch(&self, patch: RenderScenePatch, user_id: &str) -> ApplyResult {
        let (reply, rx) = oneshot::channel();
        // If the actor task is gone, surface it as a rejection rather than panic.
        if self
            .tx
            .send(CanvasCommand::ApplyPatch {
                patch,
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

    /// Apply an op ENVELOPE (MG4.2): carries an `opId` for idempotent dedup, the
    /// `baseRevision` it was authored against, and the patch. A duplicate `opId`
    /// returns the ORIGINAL ack without re-applying (see [`CanvasActor`]).
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

    /// Add a comment on `target` authored by `user_id`. Canvas logic stays in
    /// scene-core (`add_shape_scene_comment`); the actor only persists + fans out.
    pub async fn add_comment(
        &self,
        target: SceneSelection,
        body: &str,
        user_id: &str,
    ) -> CommentResult {
        let (reply, rx) = oneshot::channel();
        if self
            .tx
            .send(CanvasCommand::AddComment {
                target,
                body: body.to_string(),
                actor_user_id: user_id.to_string(),
                reply,
            })
            .await
            .is_err()
        {
            return CommentResult::Rejected {
                errors: vec!["canvas actor unavailable".to_string()],
            };
        }
        rx.await.unwrap_or(CommentResult::Rejected {
            errors: vec!["canvas actor dropped reply".to_string()],
        })
    }

    /// Update a comment's body / resolved flag (MG-7 HTTP `PATCH /api/comments/:id`).
    /// Canvas logic stays in scene-core (`update_shape_scene_comment`); the actor
    /// only persists + fans out.
    pub async fn update_comment(
        &self,
        comment_id: &str,
        body: Option<String>,
        resolved: Option<bool>,
        user_id: &str,
    ) -> CommentResult {
        let (reply, rx) = oneshot::channel();
        if self
            .tx
            .send(CanvasCommand::UpdateComment {
                comment_id: comment_id.to_string(),
                body,
                resolved,
                actor_user_id: user_id.to_string(),
                reply,
            })
            .await
            .is_err()
        {
            return CommentResult::Rejected {
                errors: vec!["canvas actor unavailable".to_string()],
            };
        }
        rx.await.unwrap_or(CommentResult::Rejected {
            errors: vec!["canvas actor dropped reply".to_string()],
        })
    }

    /// Apply a bulk [`ScenePatch`] (MG-7 HTTP `PATCH /api/scene` + the seeded
    /// `POST /api/groups`). Canvas logic stays in scene-core (`apply_scene_patch`);
    /// the actor only persists + fans out.
    pub async fn apply_scene_patch(&self, patch: ScenePatch, user_id: &str) -> ApplyResult {
        let (reply, rx) = oneshot::channel();
        if self
            .tx
            .send(CanvasCommand::ApplyScenePatch {
                patch,
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

    /// Persist a generated export artifact onto the scene (MG-7 HTTP
    /// `POST /api/groups/:id/export`). `artifact.id`/`created_at`/`scene_version`
    /// are stamped by the actor; the caller supplies type/title/target/path/
    /// content_type. Returns the stamped artifact or a rejection if the group is
    /// unknown.
    pub async fn add_artifact(
        &self,
        group_id: &str,
        artifact: SceneArtifact,
    ) -> ArtifactResult {
        let (reply, rx) = oneshot::channel();
        if self
            .tx
            .send(CanvasCommand::AddArtifact {
                group_id: group_id.to_string(),
                artifact,
                reply,
            })
            .await
            .is_err()
        {
            return ArtifactResult::Rejected {
                errors: vec!["canvas actor unavailable".to_string()],
            };
        }
        rx.await.unwrap_or(ArtifactResult::Rejected {
            errors: vec!["canvas actor dropped reply".to_string()],
        })
    }

    /// Replace one tag's Record in the registry (MG-7 HTTP `PATCH /api/tags/:id`).
    /// scene-core has no update-tag op (tags are global metadata, not a scene
    /// object with an op), so the actor mutates the tag registry directly, then
    /// persists + broadcasts. The caller has already validated the tag exists.
    pub async fn update_tag(&self, tag: shape_scene_core::Tag) -> ApplyResult {
        let (reply, rx) = oneshot::channel();
        if self
            .tx
            .send(CanvasCommand::UpdateTag { tag, reply })
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

    /// Remove one tag from the registry (MG-7 HTTP `DELETE /api/tags/:id`). The
    /// caller has already enforced the "not attached to any group" guard.
    pub async fn delete_tag(&self, tag_id: &str) -> ApplyResult {
        let (reply, rx) = oneshot::channel();
        if self
            .tx
            .send(CanvasCommand::DeleteTag {
                tag_id: tag_id.to_string(),
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

    /// Snapshot the current scene.
    pub async fn get_scene(&self) -> Scene {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(CanvasCommand::GetScene { reply })
            .await
            .expect("canvas actor task dropped");
        rx.await.expect("canvas actor dropped reply")
    }

    /// Snapshot the scene filtered to a region (MG9.5). `bbox == None` returns the
    /// whole scene; otherwise only objects intersecting `bbox` (plus the
    /// metadata they reference) are kept. See [`CanvasActor::get_scene_region`].
    pub async fn get_scene_region(&self, bbox: Option<Bounds>) -> Scene {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(CanvasCommand::GetSceneRegion { bbox, reply })
            .await
            .expect("canvas actor task dropped");
        rx.await.expect("canvas actor dropped reply")
    }

    /// The actor's current resident placement-object count (groups + nodes +
    /// edges held in memory), without triggering a hydrate. MG2.2 observability:
    /// lets a caller/test see that cold eviction bounded the working set.
    pub async fn resident_count(&self) -> usize {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(CanvasCommand::ResidentCount { reply })
            .await
            .expect("canvas actor task dropped");
        rx.await.expect("canvas actor dropped reply")
    }

    /// Subscribe to the fan-out of applied patches.
    pub fn subscribe(&self) -> broadcast::Receiver<PatchBroadcast> {
        self.broadcast_tx.subscribe()
    }

    /// Flush + checkpoint and stop the actor task.
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

/// The actor task body: owns one canvas's working state for `canvas_id`.
pub struct CanvasActor {
    canvas_id: CanvasId,
    scene: Scene,
    /// Monotonic server sequence; bumped once per successfully applied op.
    seq: i64,
    /// The server seq of the last per-object checkpoint written. Recovery
    /// replays journal entries with `seq > checkpoint_seq`.
    checkpoint_seq: i64,
    /// The scene last written to the canonical per-object store, so a checkpoint
    /// can DELETE the Records of objects that were removed since the last write
    /// (the spatial store upserts but never auto-prunes). Starts at whatever
    /// recovery reconstructed from the per-object Records.
    persisted_scene: Scene,
    /// Seen-opId table for idempotent dedup (MG4.2). Session-scoped + bounded;
    /// rebuilt from the journal suffix on recovery (see
    /// [`rebuild_dedup_from_suffix`]).
    dedup: DedupTable,
    /// Per-property LWW authority (MG4.4 remote side): records each property
    /// write keyed by server `seq`, so a lower-seq op cannot clobber a property a
    /// higher-seq op already won. Built + unit-tested now; the multi-writer
    /// convergence that consults it lands in MG-6.
    props: PropertyStore,
    /// MG2.2 working-set LRU: a logical clock and the last-touch tick of every
    /// resident placement object (group/node/edge) keyed by its store Record id.
    /// [`CanvasActor::evict_cold`] drops the lowest-tick entries first. The spine
    /// (tags/comments/artifacts/meta) is never tracked here — it is always
    /// resident — so cold eviction only ever sheds placement bulk.
    lru: HashMap<String, u64>,
    /// Monotonic touch counter feeding `lru`. Bumped once per touch so ties break
    /// by insertion order.
    lru_clock: u64,
    /// Resident placement-object budget before `evict_cold` sheds the coldest.
    /// Defaults to [`WORKING_SET_BUDGET`]; lowered by tests to force eviction.
    budget: usize,
    store: SharedStore,
    rx: mpsc::Receiver<CanvasCommand>,
    broadcast_tx: broadcast::Sender<PatchBroadcast>,
}

/// A deterministic RFC3339-ish timestamp seam. scene-core stays ambient-time
/// free, so the actor injects `now`; MG-4 will source this from the transport
/// boundary. For MG-2 a fixed value keeps applies reproducible.
fn now() -> String {
    // TODO(MG-4): inject a real clock from the transport boundary.
    "1970-01-01T00:00:00Z".to_string()
}

/// An empty scene for a brand-new canvas: matches scene-core's serde defaults
/// (`version: 1`, everything else empty), with `updated_at` set to `now`.
fn empty_scene(now: &str) -> Scene {
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
        updated_at: now.to_string(),
    }
}

/// The storage id of one journal entry (the durable op log).
fn journal_record_id(canvas_id: &CanvasId, seq: i64) -> String {
    format!("{canvas_id}:journal:{seq}")
}

impl CanvasActor {
    /// Spawn the actor task for `canvas_id`, loading durable state from `store`,
    /// and return a clone-able handle to it. Uses the default
    /// [`WORKING_SET_BUDGET`].
    pub fn spawn(canvas_id: CanvasId, store: SharedStore) -> ActorHandle {
        Self::spawn_with_budget(canvas_id, store, WORKING_SET_BUDGET)
    }

    /// Spawn with an explicit resident placement-object `budget`. The production
    /// path uses [`spawn`](CanvasActor::spawn) (default budget); tests use a small
    /// budget to force cold eviction without a huge fixture.
    pub fn spawn_with_budget(canvas_id: CanvasId, store: SharedStore, budget: usize) -> ActorHandle {
        let (tx, rx) = mpsc::channel(64);
        let (broadcast_tx, _) = broadcast::channel(256);

        let recovered = recover_durable_state(&canvas_id, &store);

        let mut actor = CanvasActor {
            canvas_id,
            scene: recovered.scene.clone(),
            seq: recovered.seq,
            checkpoint_seq: recovered.checkpoint_seq,
            persisted_scene: recovered.persisted_scene,
            dedup: recovered.dedup,
            props: recovered.props,
            lru: HashMap::new(),
            lru_clock: 0,
            budget,
            store,
            rx,
            broadcast_tx: broadcast_tx.clone(),
        };
        // Recovery rebuilds the full live scene in memory; trim it down to the
        // working-set budget immediately so a large recovered canvas does not stay
        // fully resident. Everything trimmed is already durable in the store.
        actor.touch_all_resident();
        actor.evict_cold();
        tokio::spawn(actor.run());

        ActorHandle { tx, broadcast_tx }
    }

    async fn run(mut self) {
        while let Some(cmd) = self.rx.recv().await {
            match cmd {
                CanvasCommand::ApplyPatch {
                    patch,
                    actor_user_id,
                    reply,
                } => {
                    let result = self.handle_apply(patch, self.scene.scene_version, None, &actor_user_id);
                    let _ = reply.send(result);
                }
                CanvasCommand::ApplyEnvelope {
                    envelope,
                    actor_user_id,
                    reply,
                } => {
                    let result = self.handle_apply_envelope(envelope, &actor_user_id);
                    let _ = reply.send(result);
                }
                CanvasCommand::AddComment {
                    target,
                    body,
                    actor_user_id,
                    reply,
                } => {
                    let result = self.handle_add_comment(&target, &body, &actor_user_id);
                    let _ = reply.send(result);
                }
                CanvasCommand::UpdateComment {
                    comment_id,
                    body,
                    resolved,
                    actor_user_id,
                    reply,
                } => {
                    let result = self.handle_update_comment(
                        &comment_id,
                        body.as_deref(),
                        resolved,
                        &actor_user_id,
                    );
                    let _ = reply.send(result);
                }
                CanvasCommand::ApplyScenePatch {
                    patch,
                    actor_user_id,
                    reply,
                } => {
                    let result = self.handle_apply_scene_patch(patch, &actor_user_id);
                    let _ = reply.send(result);
                }
                CanvasCommand::AddArtifact {
                    group_id,
                    artifact,
                    reply,
                } => {
                    let result = self.handle_add_artifact(&group_id, artifact);
                    let _ = reply.send(result);
                }
                CanvasCommand::UpdateTag { tag, reply } => {
                    let result = self.handle_update_tag(tag);
                    let _ = reply.send(result);
                }
                CanvasCommand::DeleteTag { tag_id, reply } => {
                    let result = self.handle_delete_tag(&tag_id);
                    let _ = reply.send(result);
                }
                CanvasCommand::GetScene { reply } => {
                    // A whole-canvas read must see every object, including cold
                    // ones: hydrate the full canvas, snapshot, then trim back.
                    self.hydrate_full();
                    let _ = reply.send(self.scene.clone());
                    self.evict_cold();
                }
                CanvasCommand::GetSceneRegion { bbox, reply } => {
                    // A windowed read hydrates only the requested window from the
                    // region index (load-on-demand), so it covers cold objects too
                    // without pulling the whole canvas resident.
                    self.hydrate_region(bbox);
                    let _ = reply.send(scene_in_region(&self.scene, bbox));
                    self.evict_cold();
                }
                CanvasCommand::ResidentCount { reply } => {
                    let count = self.scene.groups.len()
                        + self.scene.nodes.len()
                        + self.scene.edges.len();
                    let _ = reply.send(count);
                }
                CanvasCommand::Shutdown { reply } => {
                    // Every op already writes through to the per-object store, so a
                    // clean shutdown is durable without a final checkpoint. Still
                    // hydrate fully and re-checkpoint so the pruning diff runs over
                    // the COMPLETE scene (a trimmed working set must never make the
                    // shutdown checkpoint delete cold objects' durable Records).
                    self.hydrate_full();
                    self.checkpoint_scene();
                    let _ = reply.send(());
                    break;
                }
            }
        }
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
            envelope.patch,
            envelope.base_revision,
            Some(envelope.op_id),
            actor_user_id,
        )
    }

    /// Apply one op via scene-core, then (on success) stamp the order key, record
    /// the dedup ack + LWW property writes, persist the journal entry, checkpoint
    /// on the interval, and broadcast.
    ///
    /// `base_revision` is the revision the op was authored against (journaled as
    /// the "authored against" revision). `op_id` is `Some` for the envelope path
    /// (dedup) and `None` for the legacy whole-patch path.
    fn handle_apply(
        &mut self,
        patch: RenderScenePatch,
        base_revision: i64,
        op_id: Option<OpId>,
        actor_user_id: &str,
    ) -> ApplyResult {
        let now = now();
        // A render op may reference, reparent, delete, or order objects anywhere
        // on the canvas (and the fractional order-key append scans all siblings of
        // a kind), so the apply path must see the COMPLETE scene, not just the hot
        // working set. Hydrate the full canvas (load-on-demand from the store),
        // apply, write the touched objects through, then trim back to budget.
        self.hydrate_full();
        // Canvas logic stays in scene-core; the actor never reimplements apply.
        let applied = apply_render_patch_to_shape_scene(&self.scene, &patch, &now, None);
        if !applied.errors.is_empty() {
            self.evict_cold();
            return ApplyResult::Rejected {
                errors: applied.errors,
            };
        }

        self.seq += 1;
        self.scene = applied.scene;

        // MG4.6 (additive): the SERVER is the authority for concurrent-insert
        // ordering. Stamp a deterministic fractional order key into the created
        // object's meta["orderKey"]. zIndex is untouched (golden-safe).
        if let (Some(key), Some(object_id)) =
            (next_order_key(&self.scene, &patch), created_object_id(&patch))
        {
            stamp_order_key(&mut self.scene, &object_id, &key);
        }

        // MG-6.1: CONSULT the per-property LWW store so concurrent writers
        // converge by server arrival seq (last arrival wins per property). For
        // each property this op touches: if a strictly-higher-seq write already
        // won it, the op loses that property and the winning value is restored
        // into the post-apply scene; otherwise the op wins and is recorded at
        // this op's seq. `base_revision` is the LWW token, so a re-ordered/late
        // op authored against an older revision cannot resurrect a stale value.
        self.converge_lww(&patch, base_revision, self.seq);

        // The journal entry records who authored the op (userId-only identity).
        // TODO(auth): real authn/authz attaches actor_user_id here.
        self.persist(JournalEntry {
            seq: self.seq,
            op_id: op_id.clone(),
            base_revision,
            patch: Some(patch.clone()),
        });

        let revision = self.scene.scene_version;
        if let Some(op_id) = op_id {
            self.dedup.record(op_id, OpAck { seq: self.seq, revision });
        }

        let _ = self.broadcast_tx.send(PatchBroadcast {
            seq: self.seq,
            patch,
            scene: self.scene.clone(),
            author: Some(actor_user_id.to_string()),
        });

        // `persist` already wrote this op through to the per-object store (every
        // op checkpoints now, see `persist`), so every Record is exact to this op
        // — the property that makes cold reload-on-demand lossless. Mark any object
        // created by this op hot, then trim the working set back to budget; evicted
        // objects stay durable in the store.
        self.touch_all_resident();
        self.evict_cold();

        ApplyResult::Applied {
            seq: self.seq,
            revision,
        }
    }

    /// MG-6.1: consult + update the per-property LWW [`PropertyStore`] so
    /// concurrent writers converge by server arrival `seq` (last arrival wins per
    /// property). Operates on the actor's live scene and store.
    ///
    /// For each scalar property `patch` touches:
    /// - If a strictly-higher-`seq` value already won the property (the held
    ///   winner's `seq > base_revision`), the op LOSES it: the winning value is
    ///   restored into `self.scene`, leaving the property at the higher-`seq`
    ///   value rather than the stale op's value.
    /// - Otherwise the op WINS: its value is recorded at this op's `seq`, so a
    ///   later lower-seq op cannot clobber it.
    ///
    /// Only scalar property edits that concurrent writers can race on are tracked;
    /// create/delete/structural ops converge through scene-core apply and need no
    /// per-property entry.
    fn converge_lww(&mut self, patch: &RenderScenePatch, base_revision: i64, seq: i64) {
        converge_lww_into(&mut self.props, &mut self.scene, patch, base_revision, seq);
    }

    /// Add one comment via scene-core, then (on success) persist + broadcast.
    fn handle_add_comment(
        &mut self,
        target: &SceneSelection,
        body: &str,
        actor_user_id: &str,
    ) -> CommentResult {
        // A comment targets an object that may be cold; the checkpoint after this
        // op diffs the full scene to prune removed objects, so it must see the
        // complete canvas. Hydrate fully, then trim back after.
        self.hydrate_full();
        let now = now();
        // Canvas logic stays in scene-core; the actor never reimplements comment add.
        let applied = add_shape_scene_comment(&self.scene, target, body, &now);
        if !applied.errors.is_empty() {
            self.evict_cold();
            return CommentResult::Rejected {
                errors: applied.errors,
            };
        }
        let comment = match applied.comment {
            Some(c) => c,
            None => {
                return CommentResult::Rejected {
                    errors: vec!["comment was not produced".to_string()],
                }
            }
        };

        self.seq += 1;
        self.scene = applied.scene;

        // A comment is a non-render op: it has no RenderScenePatch to replay, so
        // the journal entry carries `patch: None` and recovery reconstructs the
        // comment from the checkpoint only.
        // TODO(auth): real authn/authz attaches actor_user_id here.
        self.persist(JournalEntry {
            seq: self.seq,
            op_id: None,
            base_revision: self.scene.scene_version - 1,
            patch: None,
        });

        let _ = self.broadcast_tx.send(PatchBroadcast {
            seq: self.seq,
            patch: RenderScenePatch::Select {
                selection: target.clone(),
            },
            scene: self.scene.clone(),
            author: Some(actor_user_id.to_string()),
        });

        self.evict_cold();
        CommentResult::Added { comment }
    }

    /// Update one comment via scene-core, then (on success) persist + broadcast.
    fn handle_update_comment(
        &mut self,
        comment_id: &str,
        body: Option<&str>,
        resolved: Option<bool>,
        actor_user_id: &str,
    ) -> CommentResult {
        self.hydrate_full();
        let now = now();
        let applied = update_shape_scene_comment(&self.scene, comment_id, body, resolved, &now);
        if !applied.errors.is_empty() {
            self.evict_cold();
            return CommentResult::Rejected {
                errors: applied.errors,
            };
        }
        let comment = match applied.comment {
            Some(c) => c,
            None => {
                return CommentResult::Rejected {
                    errors: vec!["comment was not produced".to_string()],
                }
            }
        };

        self.seq += 1;
        self.scene = applied.scene;
        // A comment update is a non-render op (no RenderScenePatch to replay).
        self.persist(JournalEntry {
            seq: self.seq,
            op_id: None,
            base_revision: self.scene.scene_version - 1,
            patch: None,
        });
        let _ = self.broadcast_tx.send(PatchBroadcast {
            seq: self.seq,
            patch: RenderScenePatch::Select {
                selection: comment.target.clone(),
            },
            scene: self.scene.clone(),
            author: Some(actor_user_id.to_string()),
        });
        self.evict_cold();
        CommentResult::Added { comment }
    }

    /// Apply a bulk [`ScenePatch`] via scene-core, then persist + broadcast.
    ///
    /// Canvas logic stays in scene-core (`apply_scene_patch`). Like the comment
    /// path, this is journaled with `patch: None` (the bulk diff is not a single
    /// `RenderScenePatch` to replay) and folded into the next checkpoint.
    fn handle_apply_scene_patch(
        &mut self,
        patch: ScenePatch,
        actor_user_id: &str,
    ) -> ApplyResult {
        // A bulk patch can add/remove/reorder objects anywhere; apply against the
        // complete scene, persist through, then trim back.
        self.hydrate_full();
        let now = now();
        let next = apply_scene_patch(&self.scene, &patch, &now);
        self.seq += 1;
        self.scene = next;
        self.persist(JournalEntry {
            seq: self.seq,
            op_id: None,
            base_revision: self.scene.scene_version - 1,
            patch: None,
        });
        let revision = self.scene.scene_version;
        let _ = self.broadcast_tx.send(PatchBroadcast {
            seq: self.seq,
            patch: RenderScenePatch::Select {
                selection: self.scene.selection.clone(),
            },
            scene: self.scene.clone(),
            author: Some(actor_user_id.to_string()),
        });
        self.evict_cold();
        ApplyResult::Applied {
            seq: self.seq,
            revision,
        }
    }

    /// Push a generated export artifact onto the scene, stamping its id /
    /// created_at / scene_version (mirrors the Node `addArtifact`), then persist +
    /// broadcast. Rejects an unknown target group.
    fn handle_add_artifact(
        &mut self,
        group_id: &str,
        mut artifact: SceneArtifact,
    ) -> ArtifactResult {
        // The target group may be cold; hydrate fully so the lookup and the
        // post-op checkpoint both see the complete canvas.
        self.hydrate_full();
        if !self.scene.groups.iter().any(|g| g.id == group_id) {
            self.evict_cold();
            return ArtifactResult::Rejected {
                errors: vec![format!("Group not found: {group_id}")],
            };
        }
        let now = now();
        artifact.id = format!("artifact-{now}-{}", self.seq + 1);
        artifact.created_at = now.clone();
        artifact.scene_version = self.scene.scene_version;

        self.seq += 1;
        let mut artifacts = vec![artifact.clone()];
        artifacts.extend(self.scene.artifacts.iter().cloned());
        self.scene = Scene {
            scene_version: self.scene.scene_version + 1,
            artifacts,
            updated_at: now.clone(),
            ..self.scene.clone()
        };
        self.persist(JournalEntry {
            seq: self.seq,
            op_id: None,
            base_revision: self.scene.scene_version - 1,
            patch: None,
        });
        let _ = self.broadcast_tx.send(PatchBroadcast {
            seq: self.seq,
            patch: RenderScenePatch::Select {
                selection: artifact.target.clone(),
            },
            scene: self.scene.clone(),
            // Server-internal op (HTTP export): no client author to self-skip.
            author: None,
        });
        self.evict_cold();
        ArtifactResult::Added { artifact }
    }

    /// Replace one tag in the registry, then persist + broadcast. No-op (still
    /// `Applied`) if the id is absent. Tags are global metadata, so this is a
    /// direct registry mutation, not a scene-core op.
    fn handle_update_tag(&mut self, tag: shape_scene_core::Tag) -> ApplyResult {
        // The rebuild below spreads `self.scene`; hydrate fully first so it carries
        // the complete placement set (else the post-op checkpoint would prune cold
        // objects), then trim back after.
        self.hydrate_full();
        let now = now();
        let tags: Vec<_> = self
            .scene
            .tags
            .iter()
            .map(|t| if t.id == tag.id { tag.clone() } else { t.clone() })
            .collect();
        self.seq += 1;
        self.scene = Scene {
            scene_version: self.scene.scene_version + 1,
            tags,
            updated_at: now,
            ..self.scene.clone()
        };
        self.persist(JournalEntry {
            seq: self.seq,
            op_id: None,
            base_revision: self.scene.scene_version - 1,
            patch: None,
        });
        let revision = self.scene.scene_version;
        let _ = self.broadcast_tx.send(PatchBroadcast {
            seq: self.seq,
            patch: RenderScenePatch::Select {
                selection: self.scene.selection.clone(),
            },
            scene: self.scene.clone(),
            // Server-internal op (HTTP tag PATCH): no client author to self-skip.
            author: None,
        });
        self.evict_cold();
        ApplyResult::Applied { seq: self.seq, revision }
    }

    /// Remove one tag from the registry, then persist + broadcast.
    fn handle_delete_tag(&mut self, tag_id: &str) -> ApplyResult {
        // See `handle_update_tag`: hydrate fully so the scene rebuild + checkpoint
        // carry the complete placement set.
        self.hydrate_full();
        let now = now();
        let tags: Vec<_> = self
            .scene
            .tags
            .iter()
            .filter(|t| t.id != tag_id)
            .cloned()
            .collect();
        self.seq += 1;
        self.scene = Scene {
            scene_version: self.scene.scene_version + 1,
            tags,
            updated_at: now,
            ..self.scene.clone()
        };
        self.persist(JournalEntry {
            seq: self.seq,
            op_id: None,
            base_revision: self.scene.scene_version - 1,
            patch: None,
        });
        let revision = self.scene.scene_version;
        let _ = self.broadcast_tx.send(PatchBroadcast {
            seq: self.seq,
            patch: RenderScenePatch::Select {
                selection: self.scene.selection.clone(),
            },
            scene: self.scene.clone(),
            // Server-internal op (HTTP tag DELETE): no client author to self-skip.
            author: None,
        });
        self.evict_cold();
        ApplyResult::Applied { seq: self.seq, revision }
    }

    /// Append `entry` to the durable journal (every op), then checkpoint the
    /// per-object store.
    ///
    /// MG2.2 changes the cadence: with a bounded working set, a cold object can be
    /// evicted and reloaded mid-session, so its per-object Record must be exact to
    /// the last op that touched it — not merely current as of the last interval
    /// checkpoint. The actor therefore checkpoints (write-through) on EVERY op.
    /// Because the apply path hydrates the full scene before applying,
    /// `checkpoint_scene` sees the complete canvas and writes every changed object
    /// through region-indexed, so a later reload-on-demand is lossless. The
    /// journal is still written per op (and never pruned) so the dedup table can be
    /// rebuilt from its suffix on recovery (see [`rebuild_dedup_from_suffix`]).
    ///
    /// This also subsumes the older "non-render ops force a checkpoint" fix:
    /// comment/tag/bulk-patch/artifact ops are folded in immediately, so a crash
    /// never loses one nor leaves the server `seq` ahead of `scene_version`.
    fn persist(&mut self, entry: JournalEntry) {
        let journal_payload = serde_json::to_vec(&entry).expect("journal entry serializes");
        {
            let mut store = self.store.lock().expect("storage mutex poisoned");
            store
                .save(Record {
                    id: journal_record_id(&self.canvas_id, self.seq),
                    kind: "journal".to_string(),
                    version: self.seq as u64,
                    payload: journal_payload,
                })
                .expect("journal entry persists");
        }

        self.checkpoint_scene();
    }

    /// Checkpoint the scene as per-object Records (MG5.2a/MG5.2b): upsert one
    /// region-indexed [`Record`] per current object via
    /// [`SpatialStore::save_indexed`], delete the Records of objects removed since
    /// the last write, and rewrite the `canvas-meta` Record (which carries the
    /// recovery floor in its `version`). Remember the current seq as the floor.
    /// Called on the interval and again on shutdown.
    ///
    /// Each Record's `version` is the current server seq, so the `canvas-meta`
    /// Record's `version` is the recovery floor (replacing the old whole-scene
    /// Record's `version`).
    fn checkpoint_scene(&mut self) {
        let seq = self.seq;
        let records = scene_to_records(&self.canvas_id, &self.scene, seq as u64);

        // Objects that were in the last persisted scene but are gone now: their
        // per-object Records must be deleted so the canonical store matches the
        // live scene (the spatial store upserts but never auto-prunes).
        let live_ids: std::collections::HashSet<&String> =
            records.iter().map(|(r, _)| &r.id).collect();
        let stale_ids: Vec<String> = persisted_object_record_ids(&self.canvas_id, &self.persisted_scene)
            .into_iter()
            .filter(|id| !live_ids.contains(id))
            .collect();

        {
            let mut store = self.store.lock().expect("storage mutex poisoned");
            for id in &stale_ids {
                store.delete(id).expect("stale object record deletes");
            }
            for (record, region) in records {
                store
                    .save_indexed(record, region)
                    .expect("per-object record persists");
            }
        }

        self.persisted_scene = self.scene.clone();
        self.checkpoint_seq = seq;
    }

    // --- MG2.2 bounded working set + cold LRU eviction -----------------------

    /// The store Record ids of every resident placement object (group/node/edge) —
    /// the LRU keys. Mirrors the ids `checkpoint_scene` writes, so an evicted
    /// object reloads by the same id. Spine kinds (tags/comments/artifacts/meta)
    /// are excluded: they are never tracked or evicted.
    fn placement_record_ids(&self) -> Vec<String> {
        let mut ids = Vec::new();
        for g in &self.scene.groups {
            ids.push(object_record_id(&self.canvas_id, KIND_GROUP, &g.id));
        }
        for n in &self.scene.nodes {
            ids.push(object_record_id(&self.canvas_id, KIND_NODE, &n.id));
        }
        for e in &self.scene.edges {
            ids.push(object_record_id(&self.canvas_id, KIND_EDGE, &e.id));
        }
        ids
    }

    /// Mark every currently-resident placement object as touched. Used after a
    /// full/region hydrate so freshly loaded objects are hot, and at spawn after
    /// recovery so the initial trim has tick data for all of them.
    fn touch_all_resident(&mut self) {
        for id in self.placement_record_ids() {
            self.lru.entry(id).or_insert_with(|| {
                self.lru_clock += 1;
                self.lru_clock
            });
        }
    }

    /// Replace the resident placement objects with the ones reconstructed from
    /// `records` (a store read), preserving the spine (meta/tags/comments/
    /// artifacts) from the current `self.scene`. The records carry the latest
    /// per-object state because every op writes through (see `persist`), so this
    /// is lossless. Each loaded placement object is marked hot.
    fn install_placement_from_records(&mut self, records: Vec<Record>) {
        // `records_to_scene` rebuilds a whole scene from records; we only adopt its
        // placement collections, then re-stitch the live spine + meta back on so a
        // partial (region) read never clobbers global metadata.
        let loaded = records_to_scene(&self.canvas_id, records);
        self.scene.groups = loaded.groups;
        self.scene.nodes = loaded.nodes;
        self.scene.edges = loaded.edges;
        // Reset LRU to exactly the now-resident placement set; touch them all hot.
        self.lru.clear();
        self.touch_all_resident();
    }

    /// Hydrate the COMPLETE canvas into the working set from the durable store.
    /// After this, `self.scene` holds every object, so a whole-scene op/read is
    /// correct. Callers trim back with `evict_cold` afterward.
    fn hydrate_full(&mut self) {
        let records = self.load_canvas_records(None);
        self.install_placement_from_records(records);
    }

    /// Hydrate only the placement objects overlapping `bbox` (load-on-demand for a
    /// windowed read). `None` degrades to a full hydrate. The loaded window
    /// REPLACES the resident placement set: a windowed reader holds only its
    /// window, never the whole canvas.
    fn hydrate_region(&mut self, bbox: Option<Bounds>) {
        let window = bbox.map(|b| (b.x, b.y, b.x + b.width, b.y + b.height));
        let records = self.load_canvas_records(window);
        self.install_placement_from_records(records);
    }

    /// Load this canvas's per-object placement Records from the store, optionally
    /// filtered to a region window. Region-indexed kinds (groups/nodes/edges) come
    /// from `query_region`; the spine stays in memory and is not reloaded here.
    fn load_canvas_records(&self, window: Option<(f64, f64, f64, f64)>) -> Vec<Record> {
        let store = self.store.lock().expect("storage mutex poisoned");
        store
            .query_region(&self.canvas_id.0, window)
            .expect("region query")
            .map(|r| r.expect("record loads"))
            .collect()
    }

    /// Trim the resident placement set down to `self.budget` (default
    /// [`WORKING_SET_BUDGET`]) by dropping the least-recently-touched objects.
    /// Evicted objects are NOT deleted from
    /// the store (every op already wrote them through); they are only removed from
    /// `self.scene` and the LRU map, so memory is bounded while durability is not
    /// affected. The spine (meta/tags/comments/artifacts) is never evicted.
    ///
    /// An edge is kept only while BOTH its endpoint nodes are resident, so the
    /// resident scene never holds a dangling edge after a node eviction; the edge
    /// reloads with its endpoints on the next hydrate.
    fn evict_cold(&mut self) {
        let resident = self.scene.groups.len() + self.scene.nodes.len() + self.scene.edges.len();
        if resident <= self.budget {
            return;
        }

        // Rank resident placement objects by last-touch tick (ascending = coldest
        // first) and choose the coldest to drop until we are within budget.
        let mut ranked: Vec<(u64, String)> = self
            .placement_record_ids()
            .into_iter()
            .map(|id| (self.lru.get(&id).copied().unwrap_or(0), id))
            .collect();
        ranked.sort_by_key(|(tick, _)| *tick);

        let evict_count = resident - self.budget;
        let mut evicted: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (_, id) in ranked.into_iter().take(evict_count) {
            evicted.insert(id);
        }

        let group_evicted = |id: &str| evicted.contains(&object_record_id(&self.canvas_id, KIND_GROUP, id));
        let node_evicted = |id: &str| evicted.contains(&object_record_id(&self.canvas_id, KIND_NODE, id));
        let edge_evicted = |id: &str| evicted.contains(&object_record_id(&self.canvas_id, KIND_EDGE, id));

        self.scene.groups.retain(|g| !group_evicted(&g.id));
        self.scene.nodes.retain(|n| !node_evicted(&n.id));
        // Drop an edge if it was selected for eviction OR either endpoint node was
        // evicted (no dangling edges in the resident set).
        let resident_nodes: std::collections::HashSet<String> =
            self.scene.nodes.iter().map(|n| n.id.clone()).collect();
        self.scene.edges.retain(|e| {
            !edge_evicted(&e.id)
                && resident_nodes.contains(&e.source)
                && resident_nodes.contains(&e.target)
        });

        // Forget the LRU ticks of everything no longer resident.
        let still_resident: std::collections::HashSet<String> =
            self.placement_record_ids().into_iter().collect();
        self.lru.retain(|id, _| still_resident.contains(id));
    }
}

/// Every per-object Record id that `scene` would persist (groups/nodes/edges/
/// tags/comments/artifacts), excluding the `canvas-meta` Record (which is always
/// rewritten, never deleted). Used to compute which Records to prune on a
/// checkpoint after objects were removed.
fn persisted_object_record_ids(canvas_id: &CanvasId, scene: &Scene) -> Vec<String> {
    let mut ids = Vec::new();
    for g in &scene.groups {
        ids.push(object_record_id(canvas_id, KIND_GROUP, &g.id));
    }
    for n in &scene.nodes {
        ids.push(object_record_id(canvas_id, KIND_NODE, &n.id));
    }
    for e in &scene.edges {
        ids.push(object_record_id(canvas_id, KIND_EDGE, &e.id));
    }
    for t in &scene.tags {
        ids.push(object_record_id(canvas_id, KIND_TAG, &t.id));
    }
    for c in &scene.comments {
        ids.push(object_record_id(canvas_id, KIND_COMMENT, &c.id));
    }
    for a in &scene.artifacts {
        ids.push(object_record_id(canvas_id, KIND_ARTIFACT, &a.id));
    }
    ids
}

/// What recovery rebuilds for a canvas: the live scene, the server seq, the seq
/// of the checkpoint it was loaded from, the scene as last persisted to the
/// per-object store (so the next checkpoint can prune removed objects), plus the
/// dedup + LWW state.
struct RecoveredState {
    scene: Scene,
    seq: i64,
    checkpoint_seq: i64,
    persisted_scene: Scene,
    dedup: DedupTable,
    props: PropertyStore,
}

/// Recover a canvas (MG4.1 + MG5.2a): reconstruct the checkpointed scene from the
/// per-object Records (via [`records_to_scene`]), then REPLAY every journal entry
/// with `seq > checkpoint.seq` (in seq order) through scene-core apply to rebuild
/// the live scene. This is what makes a crash after the last checkpoint
/// recoverable, not just a checkpoint-only restore.
///
/// Replay also rebuilds the LWW property store for the tail (so per-property seq
/// ordering survives recovery). The dedup table is rebuilt SEPARATELY from a
/// bounded journal suffix (not just the tail), because MG2.2 checkpoints every op
/// — the tail is normally empty, yet a reconnecting client's replayed outbox must
/// still be deduplicated across a restart. See [`rebuild_dedup_from_suffix`].
fn recover_durable_state(canvas_id: &CanvasId, store: &SharedStore) -> RecoveredState {
    let (mut scene, checkpoint_seq) = load_checkpoint(canvas_id, store);
    let persisted_scene = scene.clone();
    let mut seq = checkpoint_seq;
    let mut props = PropertyStore::new();

    for entry in load_journal_tail(canvas_id, store, checkpoint_seq) {
        seq = entry.seq;
        let Some(patch) = entry.patch else {
            // Non-render op (e.g. comment): already folded into the checkpoint if
            // it predates it; nothing to replay onto the scene here.
            continue;
        };
        let now = now();
        let applied = apply_render_patch_to_shape_scene(&scene, &patch, &now, None);
        if !applied.errors.is_empty() {
            // A journaled op must have applied cleanly when first accepted; if it
            // does not replay, stop rather than corrupt the scene.
            break;
        }
        scene = applied.scene;
        if let (Some(key), Some(object_id)) =
            (next_order_key(&scene, &patch), created_object_id(&patch))
        {
            stamp_order_key(&mut scene, &object_id, &key);
        }
        // Replay re-runs the SAME MG-6.1 convergence (consult + restore the
        // higher-seq winner), so the rebuilt scene + PropertyStore match the
        // originally-applied state byte for byte, not the raw op value.
        converge_lww_into(&mut props, &mut scene, &patch, entry.base_revision, entry.seq);
    }

    let dedup = rebuild_dedup_from_suffix(canvas_id, store);

    RecoveredState {
        scene,
        seq,
        checkpoint_seq,
        persisted_scene,
        dedup,
        props,
    }
}

/// Rebuild the dedup table from the journal's last [`MAX_SEEN_OPS`] entries, in
/// seq order, recording each entry's `(opId -> ack)`. The journal is never pruned,
/// so the suffix is always available; this keeps idempotent re-apply working even
/// though the per-op checkpoint usually leaves the replay tail empty. Mirrors the
/// in-memory dedup bound so the recovered table matches a long-lived one.
fn rebuild_dedup_from_suffix(canvas_id: &CanvasId, store: &SharedStore) -> DedupTable {
    let mut dedup = DedupTable::new();
    for entry in load_journal_suffix(canvas_id, store, MAX_SEEN_OPS) {
        if let Some(op_id) = entry.op_id {
            dedup.record(
                op_id,
                OpAck {
                    seq: entry.seq,
                    revision: entry.seq,
                },
            );
        }
    }
    dedup
}

/// Load `(scene, checkpoint_seq)` by reconstructing the canvas from its per-object
/// Records (MG5.2a): a prefix scan of `"{canvasId}:"` Records fed through
/// [`records_to_scene`]. The recovery floor is the `canvas-meta` Record's
/// `version` (the server seq stamped at the last checkpoint). A canvas with no
/// `canvas-meta` Record yet is brand new: a fresh empty scene at seq 0.
fn load_checkpoint(canvas_id: &CanvasId, store: &SharedStore) -> (Scene, i64) {
    let store = store.lock().expect("storage mutex poisoned");
    let prefix = canvas_record_prefix(canvas_id);
    let meta_id = canvas_meta_record_id(canvas_id);

    // Brand-new canvas: no canvas-meta Record means nothing was ever checkpointed.
    let checkpoint_seq = match store.load(&meta_id) {
        Ok(record) => record.version as i64,
        Err(_) => return (empty_scene(&now()), 0),
    };

    // Prefix-scan this canvas's object + meta Records (journal Records share the
    // prefix but are filtered out by kind inside `records_to_scene`).
    let object_records: Vec<Record> = store
        .list()
        .expect("record ids list")
        .into_iter()
        .filter(|id| id.starts_with(&prefix) && is_object_or_meta_record(id, &prefix))
        .map(|id| store.load(&id).expect("canvas record loads"))
        .collect();

    let scene = records_to_scene(canvas_id, object_records);
    (scene, checkpoint_seq)
}

/// Whether `id` (already known to carry the `"{canvasId}:"` prefix) is a
/// per-object or canvas-meta Record rather than a journal Record. Journal ids are
/// `"{canvasId}:journal:{seq}"`; everything else under the prefix is loaded and
/// then dispatched by `kind` in [`records_to_scene`].
fn is_object_or_meta_record(id: &str, prefix: &str) -> bool {
    match id.strip_prefix(prefix) {
        Some(rest) => !rest.starts_with("journal:"),
        None => false,
    }
}

/// Load every journal entry with `seq > checkpoint_seq`, in ascending seq order.
fn load_journal_tail(
    canvas_id: &CanvasId,
    store: &SharedStore,
    checkpoint_seq: i64,
) -> Vec<JournalEntry> {
    let store = store.lock().expect("storage mutex poisoned");
    let prefix = format!("{canvas_id}:journal:");
    let mut ids: Vec<String> = store
        .list()
        .expect("journal ids list")
        .into_iter()
        .filter(|id| id.starts_with(&prefix))
        .collect();
    // Sort by the numeric seq suffix so replay is in true seq order (lexical sort
    // of "...:10" vs "...:2" would misorder).
    ids.sort_by_key(|id| journal_seq_of(id, &prefix));

    let mut tail = Vec::new();
    for id in ids {
        let seq = journal_seq_of(&id, &prefix);
        if seq <= checkpoint_seq {
            continue;
        }
        let record = store.load(&id).expect("journal record loads");
        let entry: JournalEntry =
            serde_json::from_slice(&record.payload).expect("journal entry deserializes");
        tail.push(entry);
    }
    tail
}

/// Load the last `max` journal entries by seq, in ascending seq order. Used to
/// rebuild the bounded dedup table independent of the checkpoint floor (the
/// per-op checkpoint usually leaves the replay tail empty, but recorded opIds must
/// still survive a restart).
fn load_journal_suffix(canvas_id: &CanvasId, store: &SharedStore, max: usize) -> Vec<JournalEntry> {
    let store = store.lock().expect("storage mutex poisoned");
    let prefix = format!("{canvas_id}:journal:");
    let mut ids: Vec<String> = store
        .list()
        .expect("journal ids list")
        .into_iter()
        .filter(|id| id.starts_with(&prefix))
        .collect();
    ids.sort_by_key(|id| journal_seq_of(id, &prefix));
    let start = ids.len().saturating_sub(max);

    let mut suffix = Vec::new();
    for id in &ids[start..] {
        let record = store.load(id).expect("journal record loads");
        let entry: JournalEntry =
            serde_json::from_slice(&record.payload).expect("journal entry deserializes");
        suffix.push(entry);
    }
    suffix
}

/// Parse the trailing `:journal:{seq}` integer from a journal record id.
fn journal_seq_of(id: &str, prefix: &str) -> i64 {
    id.strip_prefix(prefix)
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0)
}

/// Filter a scene to the objects intersecting `bbox` (MG9.5). Applied to the
/// actor's resident working set, which the `GetSceneRegion` command first hydrates
/// for the requested window from the storage region index (MG2.2), so the filter
/// covers cold objects without keeping the whole canvas resident.
///
/// `bbox == None` returns the whole scene unchanged. Otherwise:
/// - **groups**: kept when `group.bounds` intersects `bbox`.
/// - **nodes**: kept when `node_bounds(node)` intersects `bbox`.
/// - **edges**: kept only when BOTH endpoint nodes are kept, so a windowed client
///   never receives a dangling edge referencing an off-window node.
/// - **tags**: ALL tags are kept. Tags are small global metadata referenced by id
///   from kept objects; shipping them all guarantees a kept object's `tagIds`
///   always resolve, and avoids a second round-trip when an object enters view.
/// - **comments / artifacts**: kept when their `target` selection references a
///   kept object, OR is canvas-level (`Canvas`/`Multi`) which is not bound to a
///   single object's bbox.
/// - scalar scene fields (`version`, `scene_version`, `proposals`, `selection`,
///   `updated_at`) are carried through unchanged so the windowed snapshot still
///   reports the canvas revision the client must reconcile against.
fn scene_in_region(scene: &Scene, bbox: Option<Bounds>) -> Scene {
    let bbox = match bbox {
        None => return scene.clone(),
        Some(b) => b,
    };

    let groups: Vec<_> = scene
        .groups
        .iter()
        .filter(|g| bounds_intersect(&g.bounds, &bbox))
        .cloned()
        .collect();

    let nodes: Vec<_> = scene
        .nodes
        .iter()
        .filter(|n| bounds_intersect(&node_bounds(n), &bbox))
        .cloned()
        .collect();

    let kept_node_ids: std::collections::HashSet<&str> =
        nodes.iter().map(|n| n.id.as_str()).collect();
    let kept_group_ids: std::collections::HashSet<&str> =
        groups.iter().map(|g| g.id.as_str()).collect();

    let edges: Vec<_> = scene
        .edges
        .iter()
        .filter(|e| kept_node_ids.contains(e.source.as_str()) && kept_node_ids.contains(e.target.as_str()))
        .cloned()
        .collect();
    let kept_edge_ids: std::collections::HashSet<&str> =
        edges.iter().map(|e| e.id.as_str()).collect();

    let target_in_region = |target: &SceneSelection| match target {
        // Canvas/Multi targets are not bound to a single object's bbox; keep them.
        SceneSelection::Canvas | SceneSelection::Multi { .. } => true,
        SceneSelection::Group { id } => kept_group_ids.contains(id.as_str()),
        SceneSelection::Node { id } => kept_node_ids.contains(id.as_str()),
        SceneSelection::Edge { id } => kept_edge_ids.contains(id.as_str()),
    };

    let comments: Vec<_> = scene
        .comments
        .iter()
        .filter(|c| target_in_region(&c.target))
        .cloned()
        .collect();
    let artifacts: Vec<_> = scene
        .artifacts
        .iter()
        .filter(|a| target_in_region(&a.target))
        .cloned()
        .collect();

    Scene {
        version: scene.version,
        scene_version: scene.scene_version,
        groups,
        nodes,
        edges,
        tags: scene.tags.clone(),
        comments,
        artifacts,
        proposals: scene.proposals.clone(),
        selection: scene.selection.clone(),
        updated_at: scene.updated_at.clone(),
    }
}

/// Whether a broadcast patch should be delivered to a connection windowed to
/// `bbox` (MG9.5 fan-out filter). Decided against `scene_after`, the actor's
/// scene snapshot AFTER the op applied, so created/moved objects are present to
/// be located.
///
/// Conservative by design: deliver unless we can prove the op is fully outside
/// the region. A whole-canvas subscriber (`None`) always receives everything.
/// Property/structural ops we cannot localize to a bbox (deletes, reparents,
/// batches, selection, tag/comment ops) are sent through so a windowed client
/// never silently misses an edit that affects an object it holds.
pub(crate) fn patch_touches_region(
    scene_after: &Scene,
    patch: &RenderScenePatch,
    bbox: Option<Bounds>,
) -> bool {
    let bbox = match bbox {
        None => return true,
        Some(b) => b,
    };
    object_in_region(scene_after, patch, &bbox).unwrap_or(true)
}

/// `Some(true/false)` when the patch targets a single object we can locate in the
/// post-apply scene and test against `bbox`; `None` when the op is structural or
/// otherwise not localizable (caller treats `None` as "send through").
fn object_in_region(scene: &Scene, patch: &RenderScenePatch, bbox: &Bounds) -> Option<bool> {
    let group_hits = |id: &str| {
        scene
            .groups
            .iter()
            .find(|g| g.id == id)
            .map(|g| bounds_intersect(&g.bounds, bbox))
    };
    let node_hits = |id: &str| {
        scene
            .nodes
            .iter()
            .find(|n| n.id == id)
            .map(|n| bounds_intersect(&node_bounds(n), bbox))
    };

    match patch {
        RenderScenePatch::CreateGroup { group } => Some(bounds_intersect(&group.bounds, bbox)),
        RenderScenePatch::CreateCard { card } => Some(bounds_intersect(&card.bounds, bbox)),
        RenderScenePatch::MoveGroup { id, .. }
        | RenderScenePatch::ResizeGroup { id, .. } => group_hits(id),
        RenderScenePatch::MoveCard { id, .. }
        | RenderScenePatch::ResizeCard { id, .. }
        | RenderScenePatch::SetCardZIndex { id, .. }
        | RenderScenePatch::EditCardText { id, .. } => node_hits(id),
        // Deletes, reparents, grouping, edges, tags, selection, alignment,
        // distribution, duplication, and batches are not reducible to a single
        // in-bounds test here — be conservative and send through.
        _ => None,
    }
}

/// One scalar property an op wrote: its name and the value (as JSON) the op tried
/// to set. The actor's MG-6.1 convergence compares this against the LWW winner.
struct PropWrite {
    property: &'static str,
    value: serde_json::Value,
}

/// The `(object_id, writes)` a `patch` makes to LWW-tracked scalar properties, or
/// `None` for create/delete/structural ops that converge through scene-core apply
/// and need no per-property entry.
fn lww_writes_of(patch: &RenderScenePatch) -> Option<(&str, Vec<PropWrite>)> {
    use serde_json::json;
    let (id, property, value) = match patch {
        RenderScenePatch::EditCardText { id, field, value } => {
            let property = match field {
                shape_scene_core::op::EditField::Title => "title",
                shape_scene_core::op::EditField::Summary => "summary",
                shape_scene_core::op::EditField::Detail => "detail",
            };
            (id.as_str(), property, json!(value))
        }
        RenderScenePatch::MoveCard { id, position } => (id.as_str(), "position", json!(position)),
        RenderScenePatch::SetCardZIndex { id, z_index } => (id.as_str(), "zIndex", json!(z_index)),
        RenderScenePatch::ResizeCard { id, bounds } | RenderScenePatch::ResizeGroup { id, bounds } => {
            (id.as_str(), "bounds", json!(bounds))
        }
        _ => return None,
    };
    Some((id, vec![PropWrite { property, value }]))
}

/// MG-6.1 per-property LWW convergence (free function so recovery replay can run
/// the SAME logic without an actor). The server arrival `seq` is the authority —
/// last arrival wins per property.
///
/// For each property the op touches: if the [`PropertyStore`] already holds a
/// strictly-higher-`seq` winner (`held.seq > base_revision`), the op LOSES and
/// the winner's value is restored into `scene` (so a re-ordered/late op authored
/// against an older revision cannot resurrect a stale value); otherwise the op
/// WINS and its value is recorded at `seq`.
fn converge_lww_into(
    props: &mut PropertyStore,
    scene: &mut Scene,
    patch: &RenderScenePatch,
    base_revision: i64,
    seq: i64,
) {
    let Some((id, writes)) = lww_writes_of(patch) else {
        return;
    };
    let id = id.to_string();
    for write in writes {
        match props.get(&id, write.property) {
            // A strictly-higher-seq write already won this property: the op is
            // stale for it. Keep the winner and restore it into the scene.
            Some(held) if held.seq > base_revision => {
                let winner = held.value.clone();
                set_scene_property(scene, &id, write.property, &winner);
            }
            // First writer, or this op outranks the held value: the op wins.
            _ => {
                props.apply(&id, write.property, write.value, seq);
            }
        }
    }
}

/// Write `value` (the LWW winner, as JSON) into object `id`'s scalar property in
/// `scene`. Mirrors how scene-core apply sets each field, so a restored winner is
/// byte-identical to having applied the winning op. No-op if the id/value cannot
/// be resolved (the loser's already-applied value then stands, which is safe —
/// only an in-flight property a winner actually advanced is ever restored).
fn set_scene_property(scene: &mut Scene, id: &str, property: &str, value: &serde_json::Value) {
    match property {
        "title" | "summary" | "detail" => {
            let Some(s) = value.as_str() else { return };
            if let Some(n) = scene.nodes.iter_mut().find(|n| n.id == id) {
                match property {
                    "title" => n.title = s.to_string(),
                    "summary" => n.summary = s.to_string(),
                    "detail" => n.detail = s.to_string(),
                    _ => {}
                }
            }
        }
        "position" => {
            let Ok(p) = serde_json::from_value::<shape_scene_core::Point>(value.clone()) else {
                return;
            };
            if let Some(n) = scene.nodes.iter_mut().find(|n| n.id == id) {
                n.position = p;
            }
        }
        "zIndex" => {
            let Some(z) = value.as_f64() else { return };
            if let Some(n) = scene.nodes.iter_mut().find(|n| n.id == id) {
                n.z_index = z;
            }
        }
        "bounds" => {
            let Ok(b) = serde_json::from_value::<shape_scene_core::Bounds>(value.clone()) else {
                return;
            };
            // ResizeCard targets a node's size; ResizeGroup targets a group's
            // bounds. Try the node first, then the group.
            if let Some(n) = scene.nodes.iter_mut().find(|n| n.id == id) {
                n.position = shape_scene_core::Point { x: b.x, y: b.y };
                n.size = shape_scene_core::Size {
                    width: b.width,
                    height: b.height,
                };
            } else if let Some(g) = scene.groups.iter_mut().find(|g| g.id == id) {
                g.bounds = b;
            }
        }
        _ => {}
    }
}
