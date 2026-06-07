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
//! subscriber is answered from the actor's LIVE in-memory scene filtered by bbox,
//! not from the storage region index. The in-memory scene is always current and
//! correct, so this is the right source for a *read*; the spatial store
//! (`SpatialStore::query_region`) exists to back a *different*, future capability.
//!
//! Follow-up — actor-memory eviction (not yet built). Today the actor holds the
//! WHOLE canvas in memory; a very large canvas should hold only its hot regions
//! and LRU-evict cold ones, reloading a region on demand via
//! `SpatialStore::query_region(canvas_id, bbox)`. That is the reason the
//! checkpoint persists per-object Records region-indexed (MG5.2b). It is NOT live
//! yet because eviction needs per-op region-indexed persistence so a reloaded cold
//! region is exact-to-the-last-op; the current store persists at the checkpoint
//! cadence (MG-5), so a freshly reloaded region could miss un-checkpointed tail
//! ops. Region *reads* (MG-9.5) sidestep this entirely by reading live memory.

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
    OpEnvelope, OpId, CHECKPOINT_INTERVAL,
};

/// A single shared storage adapter, guarded so concurrent canvas actors can
/// persist into the same backing store without racing.
pub type SharedStore = Arc<Mutex<SqliteAdapter>>;

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
    /// rebuilt from the journal tail on recovery.
    dedup: DedupTable,
    /// Per-property LWW authority (MG4.4 remote side): records each property
    /// write keyed by server `seq`, so a lower-seq op cannot clobber a property a
    /// higher-seq op already won. Built + unit-tested now; the multi-writer
    /// convergence that consults it lands in MG-6.
    props: PropertyStore,
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
    /// and return a clone-able handle to it.
    pub fn spawn(canvas_id: CanvasId, store: SharedStore) -> ActorHandle {
        let (tx, rx) = mpsc::channel(64);
        let (broadcast_tx, _) = broadcast::channel(256);

        let recovered = recover_durable_state(&canvas_id, &store);

        let actor = CanvasActor {
            canvas_id,
            scene: recovered.scene.clone(),
            seq: recovered.seq,
            checkpoint_seq: recovered.checkpoint_seq,
            persisted_scene: recovered.persisted_scene,
            dedup: recovered.dedup,
            props: recovered.props,
            store,
            rx,
            broadcast_tx: broadcast_tx.clone(),
        };
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
                    let _ = reply.send(self.scene.clone());
                }
                CanvasCommand::GetSceneRegion { bbox, reply } => {
                    let _ = reply.send(scene_in_region(&self.scene, bbox));
                }
                CanvasCommand::Shutdown { reply } => {
                    // Checkpoints are written on the interval, so the journal tail
                    // since the last one is only durable as journal entries. Force
                    // a final checkpoint on clean shutdown so a graceful stop never
                    // needs journal replay; an unclean stop (dropped handle) still
                    // recovers via the journal tail.
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
        // Canvas logic stays in scene-core; the actor never reimplements apply.
        let applied = apply_render_patch_to_shape_scene(&self.scene, &patch, &now, None);
        if !applied.errors.is_empty() {
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
        let now = now();
        // Canvas logic stays in scene-core; the actor never reimplements comment add.
        let applied = add_shape_scene_comment(&self.scene, target, body, &now);
        if !applied.errors.is_empty() {
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
        let now = now();
        let applied = update_shape_scene_comment(&self.scene, comment_id, body, resolved, &now);
        if !applied.errors.is_empty() {
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
        if !self.scene.groups.iter().any(|g| g.id == group_id) {
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
        ArtifactResult::Added { artifact }
    }

    /// Replace one tag in the registry, then persist + broadcast. No-op (still
    /// `Applied`) if the id is absent. Tags are global metadata, so this is a
    /// direct registry mutation, not a scene-core op.
    fn handle_update_tag(&mut self, tag: shape_scene_core::Tag) -> ApplyResult {
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
        ApplyResult::Applied { seq: self.seq, revision }
    }

    /// Remove one tag from the registry, then persist + broadcast.
    fn handle_delete_tag(&mut self, tag_id: &str) -> ApplyResult {
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
        ApplyResult::Applied { seq: self.seq, revision }
    }

    /// Append `entry` to the durable journal (every op), then checkpoint the
    /// whole scene only every [`CHECKPOINT_INTERVAL`] ops (MG4.1). Between
    /// checkpoints, recovery replays the journal tail past the last checkpoint,
    /// so a crash after the last checkpoint still recovers every journaled op.
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

        if self.seq - self.checkpoint_seq >= CHECKPOINT_INTERVAL {
            self.checkpoint_scene();
        }
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
/// Replay also rebuilds the dedup table (so a reconnecting client's replayed
/// outbox stays idempotent across a restart) and the LWW property store (so
/// per-property seq ordering survives recovery).
fn recover_durable_state(canvas_id: &CanvasId, store: &SharedStore) -> RecoveredState {
    let (mut scene, checkpoint_seq) = load_checkpoint(canvas_id, store);
    let persisted_scene = scene.clone();
    let mut seq = checkpoint_seq;
    let mut dedup = DedupTable::new();
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
        if let Some(op_id) = entry.op_id {
            dedup.record(op_id, OpAck { seq: entry.seq, revision: scene.scene_version });
        }
    }

    RecoveredState {
        scene,
        seq,
        checkpoint_seq,
        persisted_scene,
        dedup,
        props,
    }
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

/// Parse the trailing `:journal:{seq}` integer from a journal record id.
fn journal_seq_of(id: &str, prefix: &str) -> i64 {
    id.strip_prefix(prefix)
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0)
}

/// Filter a scene to the objects intersecting `bbox` (MG9.5). Answered from the
/// actor's live in-memory scene (correct + up to date), NOT from the storage
/// region index — the spatial store backs a future actor-memory-eviction path,
/// documented as a follow-up, not this region read.
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
