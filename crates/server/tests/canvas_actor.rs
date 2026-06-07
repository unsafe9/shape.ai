//! MG-2 integration tests for the per-canvas actor + registry + storage embed.
//!
//! Every test uses an in-memory sqlite store and drives the actor through its
//! async handle on a `#[tokio::test]` runtime.

use std::time::Duration;

use shape_scene_core::{CanvasId, RenderCard, RenderGroup, RenderScenePatch, WorldRect};
use shape_server::canvas_actor::SharedStore;
use shape_server::sync::{OpEnvelope, OpId};
use shape_server::{ActorHandle, ApplyResult, CanvasActor, CanvasRegistry};

/// Wrap a render patch in an MG-4 op envelope with an `(clientId, localSeq)` id.
fn envelope(client_id: &str, local_seq: i64, base_revision: i64, patch: RenderScenePatch) -> OpEnvelope {
    OpEnvelope {
        op_id: OpId {
            client_id: client_id.to_string(),
            local_seq,
        },
        base_revision,
        ts: "1970-01-01T00:00:00Z".to_string(),
        patch,
    }
}

fn group_rect() -> WorldRect {
    WorldRect {
        x: 0.0,
        y: 0.0,
        width: 400.0,
        height: 300.0,
    }
}

fn card_rect() -> WorldRect {
    WorldRect {
        x: 10.0,
        y: 10.0,
        width: 120.0,
        height: 80.0,
    }
}

fn create_group(id: &str) -> RenderScenePatch {
    RenderScenePatch::CreateGroup {
        group: RenderGroup {
            id: id.to_string(),
            title: "G".to_string(),
            summary: String::new(),
            bounds: group_rect(),
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
            bounds: card_rect(),
            z_index: 0.0,
            style_key: String::new(),
            accessibility_label: String::new(),
        },
    }
}

/// Spawn a bare actor over a fresh in-memory store and return both its handle
/// and the shared store (so a re-spawn on the SAME store can be tested).
fn spawn_actor(canvas: &str) -> (ActorHandle, SharedStore) {
    let store: SharedStore = std::sync::Arc::new(std::sync::Mutex::new(
        shape_storage_core::SqliteAdapter::open_in_memory().unwrap(),
    ));
    let handle = CanvasActor::spawn(CanvasId::from(canvas), std::sync::Arc::clone(&store));
    (handle, store)
}

#[tokio::test]
async fn lifecycle_create_group_then_card_increments_seq_and_reflects_scene() {
    let (handle, _store) = spawn_actor("c-lifecycle");

    let r1 = handle.apply_patch(create_group("g1"), "user-1").await;
    assert!(
        matches!(r1, ApplyResult::Applied { seq: 1, .. }),
        "first apply should be seq 1, got {r1:?}"
    );

    let r2 = handle.apply_patch(create_card("n1", "g1"), "user-1").await;
    assert!(
        matches!(r2, ApplyResult::Applied { seq: 2, .. }),
        "second apply should be seq 2, got {r2:?}"
    );

    let scene = handle.get_scene().await;
    assert_eq!(scene.groups.len(), 1, "group present");
    assert_eq!(scene.groups[0].id, "g1");
    assert_eq!(scene.nodes.len(), 1, "card present");
    assert_eq!(scene.nodes[0].id, "n1");
    assert_eq!(scene.nodes[0].group_id, "g1");
}

#[tokio::test]
async fn rejected_patch_does_not_bump_seq() {
    let (handle, _store) = spawn_actor("c-reject");

    // create-card against a non-existent group is rejected by scene-core.
    let r = handle.apply_patch(create_card("n1", "missing-group"), "user-1").await;
    match r {
        ApplyResult::Rejected { errors } => assert!(!errors.is_empty()),
        other => panic!("expected rejection, got {other:?}"),
    }

    // A subsequent valid op is still seq 1 (the rejected op didn't advance).
    let ok = handle.apply_patch(create_group("g1"), "user-1").await;
    assert!(matches!(ok, ApplyResult::Applied { seq: 1, .. }), "got {ok:?}");
}

#[tokio::test]
async fn durability_checkpoint_survives_shutdown_and_respawn() {
    let (handle, store) = spawn_actor("c-durable");

    handle.apply_patch(create_group("g1"), "user-1").await;
    handle.apply_patch(create_card("n1", "g1"), "user-1").await;
    let before = handle.get_scene().await;

    // Flush + checkpoint, then re-spawn a brand-new actor on the SAME store.
    handle.shutdown().await;

    let reborn = CanvasActor::spawn(CanvasId::from("c-durable"), std::sync::Arc::clone(&store));
    let after = reborn.get_scene().await;

    assert_eq!(after.groups.len(), 1, "group reloaded from checkpoint");
    assert_eq!(after.nodes.len(), 1, "card reloaded from checkpoint");
    assert_eq!(after, before, "reloaded scene equals the checkpointed scene");

    // The reloaded actor continues the server seq from the checkpoint.
    let next = reborn.apply_patch(create_card("n2", "g1"), "user-1").await;
    assert!(
        matches!(next, ApplyResult::Applied { seq: 3, .. }),
        "seq continues past the 2 checkpointed ops, got {next:?}"
    );
}

#[tokio::test]
async fn idle_evict_removes_then_respawn_reloads_durable_state() {
    let registry = CanvasRegistry::open_in_memory().unwrap();
    let canvas = CanvasId::from("c-evict");

    let handle = registry.get_or_spawn(&canvas).await.unwrap();
    handle.apply_patch(create_group("g1"), "user-1").await;
    handle.apply_patch(create_card("n1", "g1"), "user-1").await;
    assert!(registry.contains(&canvas));

    // Evict idle (zero idle threshold => evict everything now).
    let evicted = registry.evict_idle(Duration::from_secs(0)).await;
    assert_eq!(evicted, vec![canvas.clone()]);
    assert!(!registry.contains(&canvas), "canvas removed from registry");
    assert!(registry.is_empty());

    // Re-get spawns a fresh actor that reloads durable state from the shared db.
    let reborn = registry.get_or_spawn(&canvas).await.unwrap();
    assert!(registry.contains(&canvas));
    let scene = reborn.get_scene().await;
    assert_eq!(scene.groups.len(), 1, "durable group reloaded after evict");
    assert_eq!(scene.nodes.len(), 1, "durable card reloaded after evict");
}

#[tokio::test]
async fn explicit_evict_then_get_spawns_fresh() {
    let registry = CanvasRegistry::open_in_memory().unwrap();
    let canvas = CanvasId::from("c-explicit-evict");

    let handle = registry.get_or_spawn(&canvas).await.unwrap();
    handle.apply_patch(create_group("g1"), "user-1").await;
    assert!(registry.contains(&canvas));

    registry.evict(&canvas).await;
    assert!(!registry.contains(&canvas));

    let reborn = registry.get_or_spawn(&canvas).await.unwrap();
    let scene = reborn.get_scene().await;
    assert_eq!(scene.groups.len(), 1, "reloaded after explicit evict");
}

#[tokio::test]
async fn broadcast_delivers_applied_patch_with_new_seq() {
    let (handle, _store) = spawn_actor("c-broadcast");
    let mut rx = handle.subscribe();

    handle.apply_patch(create_group("g1"), "user-1").await;

    let msg = rx.recv().await.expect("broadcast received");
    assert_eq!(msg.seq, 1, "broadcast carries the new server seq");
    assert!(
        matches!(msg.patch, RenderScenePatch::CreateGroup { .. }),
        "broadcast carries the applied op"
    );
    assert_eq!(msg.scene.groups.len(), 1, "broadcast scene reflects the apply");
}

// ---------------------------------------------------------------------------
// MG-4: opId idempotent dedup.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn duplicate_op_id_does_not_reapply_or_bump_seq() {
    let (handle, _store) = spawn_actor("c-dedup");

    // First apply of (c1, 1): seq 1.
    let first = handle.apply_envelope(envelope("c1", 1, 0, create_group("g1")), "c1").await;
    assert!(matches!(first, ApplyResult::Applied { seq: 1, .. }), "got {first:?}");

    // Re-apply the SAME opId: returns the ORIGINAL ack (seq 1), no second apply.
    let dup = handle.apply_envelope(envelope("c1", 1, 0, create_group("g1")), "c1").await;
    assert!(
        matches!(dup, ApplyResult::Applied { seq: 1, .. }),
        "duplicate op re-acks the original seq, got {dup:?}"
    );

    // The scene was not mutated twice (still one group), and a NEW op is seq 2.
    let scene = handle.get_scene().await;
    assert_eq!(scene.groups.len(), 1, "duplicate op did not create a second group");

    let next = handle.apply_envelope(envelope("c1", 2, 1, create_card("n1", "g1")), "c1").await;
    assert!(
        matches!(next, ApplyResult::Applied { seq: 2, .. }),
        "new op advances to seq 2 (duplicate did not bump), got {next:?}"
    );
}

// ---------------------------------------------------------------------------
// MG-4.1: journal-tail recovery past the last checkpoint.
// ---------------------------------------------------------------------------

/// Simulate a crash: ops are journaled (each apply awaits its durable reply) but
/// the actor never checkpoints (we drop the handle instead of `shutdown()`, so
/// the run loop ends WITHOUT writing a final checkpoint). On respawn the actor
/// must REPLAY the journal tail to recover those ops, not just restore an empty
/// checkpoint.
#[tokio::test]
async fn crash_after_journal_before_checkpoint_recovers_via_journal_replay() {
    let store: SharedStore = std::sync::Arc::new(std::sync::Mutex::new(
        shape_storage_core::SqliteAdapter::open_in_memory().unwrap(),
    ));
    let canvas = CanvasId::from("c-crash");

    {
        let handle = CanvasActor::spawn(canvas.clone(), std::sync::Arc::clone(&store));
        // These are journaled but never checkpointed (interval is 32; we apply 3).
        handle.apply_envelope(envelope("c1", 1, 0, create_group("g1")), "c1").await;
        handle.apply_envelope(envelope("c1", 2, 1, create_card("n1", "g1")), "c1").await;
        handle.apply_envelope(envelope("c1", 3, 2, create_card("n2", "g1")), "c1").await;
        // Drop the handle WITHOUT shutdown => crash (no final checkpoint).
        drop(handle);
    }
    // Let the actor task observe the dropped channel and exit.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Respawn on the SAME store: recovery loads the (empty) checkpoint, then
    // replays journal entries 1..3.
    let reborn = CanvasActor::spawn(canvas.clone(), std::sync::Arc::clone(&store));
    let scene = reborn.get_scene().await;
    assert_eq!(scene.groups.len(), 1, "group recovered from journal replay");
    assert_eq!(scene.nodes.len(), 2, "both cards recovered from journal replay");

    // The server seq continues past the replayed tail.
    let next = reborn.apply_envelope(envelope("c1", 4, 3, create_card("n3", "g1")), "c1").await;
    assert!(
        matches!(next, ApplyResult::Applied { seq: 4, .. }),
        "seq continues past the 3 replayed ops, got {next:?}"
    );

    // Dedup state was rebuilt from the journal: replaying op (c1,2) is idempotent.
    let replayed = reborn.apply_envelope(envelope("c1", 2, 1, create_card("n1", "g1")), "c1").await;
    assert!(
        matches!(replayed, ApplyResult::Applied { seq: 2, .. }),
        "recovered dedup table re-acks the original seq for a replayed opId, got {replayed:?}"
    );
}

/// Once enough ops cross the checkpoint interval, a checkpoint is written and a
/// fresh actor restores from it; ops journaled AFTER that checkpoint still replay
/// on top. Drives 35 ops (interval 32) so exactly one checkpoint exists at seq 33
/// (32 creates + 1 group => checkpoint at 33), then 2 more journaled ops.
#[tokio::test]
async fn checkpoint_then_journal_tail_both_recover() {
    let store: SharedStore = std::sync::Arc::new(std::sync::Mutex::new(
        shape_storage_core::SqliteAdapter::open_in_memory().unwrap(),
    ));
    let canvas = CanvasId::from("c-checkpoint");
    let total: i64 = 35;

    {
        let handle = CanvasActor::spawn(canvas.clone(), std::sync::Arc::clone(&store));
        handle.apply_envelope(envelope("c1", 1, 0, create_group("g1")), "c1").await;
        for i in 2..=total {
            handle
                .apply_envelope(
                    envelope("c1", i, i - 1, create_card(&format!("n{i}"), "g1")),
                    "c1",
                )
                .await;
        }
        drop(handle); // crash: any ops past the last checkpoint live only in the journal.
    }
    tokio::time::sleep(Duration::from_millis(50)).await;

    let reborn = CanvasActor::spawn(canvas.clone(), std::sync::Arc::clone(&store));
    let scene = reborn.get_scene().await;
    assert_eq!(scene.groups.len(), 1, "group recovered");
    assert_eq!(
        scene.nodes.len() as i64,
        total - 1,
        "all {} cards recovered across checkpoint + journal tail",
        total - 1
    );

    let next = reborn
        .apply_envelope(envelope("c1", total + 1, total, create_card("nx", "g1")), "c1")
        .await;
    assert!(
        matches!(next, ApplyResult::Applied { seq, .. } if seq == total + 1),
        "seq continues at {} after recovery, got {next:?}",
        total + 1
    );
}

// ---------------------------------------------------------------------------
// MG-4.6: fractional order keys stamped by the server (additive, golden-safe).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn two_inserts_get_distinct_ordered_fractional_keys() {
    let (handle, _store) = spawn_actor("c-fractional");

    handle.apply_patch(create_group("g1"), "user-1").await;
    handle.apply_patch(create_group("g2"), "user-1").await;

    let scene = handle.get_scene().await;
    let k1 = order_key(&scene.groups[0]).expect("g1 has an order key");
    let k2 = order_key(&scene.groups[1]).expect("g2 has an order key");

    assert_ne!(k1, k2, "two inserts get distinct order keys");
    // The scene preserves insertion order, and append semantics put g2 after g1.
    assert_eq!(scene.groups[0].id, "g1");
    assert_eq!(scene.groups[1].id, "g2");
    assert!(k1 < k2, "second insert's key sorts after the first: {k1} < {k2}");

    // zIndex is untouched (golden-safe): the numeric field is still the default.
    assert_eq!(scene.groups[0].z_index, 0.0, "zIndex is not replaced by MG-4.6");
}

/// Read a group's `meta["orderKey"]`, if present.
fn order_key(group: &shape_scene_core::SceneGroup) -> Option<String> {
    group
        .meta
        .as_ref()?
        .get("orderKey")?
        .as_str()
        .map(|s| s.to_string())
}

// ---------------------------------------------------------------------------
// MG-6.1: per-property LWW convergence by server arrival seq + authored author.
// ---------------------------------------------------------------------------

/// Edit a card's title (an LWW-tracked scalar property).
fn edit_title(id: &str, value: &str) -> RenderScenePatch {
    RenderScenePatch::EditCardText {
        id: id.to_string(),
        field: shape_scene_core::op::EditField::Title,
        value: value.to_string(),
    }
}

/// Two users write the SAME node property; the server serializes them and assigns
/// a monotonic seq. The later-arriving op (higher seq) wins per property, leaving
/// the property at the higher-seq value — provided the later writer is acting on
/// current state (its `baseRevision` is not behind the property's current seq).
#[tokio::test]
async fn concurrent_property_edits_converge_to_higher_seq() {
    let (handle, _store) = spawn_actor("c-lww-converge");

    handle.apply_patch(create_group("g1"), "user-A").await; // seq 1, revision 1
    handle.apply_patch(create_card("n1", "g1"), "user-A").await; // seq 2, revision 2

    // user-A edits title first (arrives first => lower seq 3, revision -> 3).
    let a = handle
        .apply_envelope(envelope("user-A", 1, 2, edit_title("n1", "from-A")), "user-A")
        .await;
    assert!(matches!(a, ApplyResult::Applied { seq: 3, revision: 3 }), "got {a:?}");

    // user-B edits the SAME property, acting on the latest revision (3) it acked
    // (the legitimate last writer, not a straggler). It arrives later => seq 4.
    let b = handle
        .apply_envelope(envelope("user-B", 1, 3, edit_title("n1", "from-B")), "user-B")
        .await;
    assert!(matches!(b, ApplyResult::Applied { seq: 4, .. }), "got {b:?}");

    // The later-arriving (higher-seq) write wins per property.
    let scene = handle.get_scene().await;
    let title = &scene.nodes.iter().find(|n| n.id == "n1").unwrap().title;
    assert_eq!(title, "from-B", "the later-arriving (higher seq) op wins the property");
}

/// A re-ordered / late op carrying a STALE baseRevision for a property a
/// higher-seq op already advanced must NOT clobber it. The server applies the
/// late op last (so it has the highest seq), but because it was authored against
/// an older revision than the winner's seq, LWW convergence restores the winner.
#[tokio::test]
async fn stale_base_revision_does_not_clobber_advanced_property() {
    let (handle, _store) = spawn_actor("c-lww-stale");

    handle.apply_patch(create_group("g1"), "user-A").await; // seq 1
    handle.apply_patch(create_card("n1", "g1"), "user-A").await; // seq 2

    // user-A advances the title at seq 3 (authored against revision 2).
    let winner = handle
        .apply_envelope(envelope("user-A", 1, 2, edit_title("n1", "winner")), "user-A")
        .await;
    assert!(matches!(winner, ApplyResult::Applied { seq: 3, .. }), "got {winner:?}");

    // user-B's op was authored against the OLD revision 2 (it never saw seq 3's
    // result) but arrives LAST, so the server assigns it the highest seq (4).
    // Despite the higher arrival seq, its stale baseRevision (2) < the winner's
    // held seq (3), so it loses the property: the title stays "winner".
    let stale = handle
        .apply_envelope(envelope("user-B", 1, 2, edit_title("n1", "stale")), "user-B")
        .await;
    assert!(matches!(stale, ApplyResult::Applied { seq: 4, .. }), "got {stale:?}");

    let scene = handle.get_scene().await;
    let title = &scene.nodes.iter().find(|n| n.id == "n1").unwrap().title;
    assert_eq!(
        title, "winner",
        "a stale-baseRevision op does not clobber a property a higher-seq op already won"
    );
}

/// MG-6.1 / MG-6.3: the actor tags each broadcast with the authoring userId so
/// the WS layer can self-skip and a write's author is recorded on the wire.
#[tokio::test]
async fn broadcast_carries_authoring_user_id() {
    let (handle, _store) = spawn_actor("c-author");
    let mut rx = handle.subscribe();

    handle.apply_patch(create_group("g1"), "user-42").await;

    let msg = rx.recv().await.expect("broadcast received");
    assert_eq!(
        msg.author.as_deref(),
        Some("user-42"),
        "broadcast records the op's authoring userId"
    );
}

/// MG-6.1 convergence must survive a crash + journal replay: the stale op's loss
/// is reproduced on recovery (the journal replays the same convergence), so the
/// recovered scene matches the originally-converged scene, not the raw op value.
#[tokio::test]
async fn lww_convergence_survives_journal_replay() {
    let store: SharedStore = std::sync::Arc::new(std::sync::Mutex::new(
        shape_storage_core::SqliteAdapter::open_in_memory().unwrap(),
    ));
    let canvas = CanvasId::from("c-lww-replay");

    {
        let handle = CanvasActor::spawn(canvas.clone(), std::sync::Arc::clone(&store));
        handle.apply_envelope(envelope("A", 1, 0, create_group("g1")), "A").await; // seq 1
        handle.apply_envelope(envelope("A", 2, 1, create_card("n1", "g1")), "A").await; // seq 2
        handle.apply_envelope(envelope("A", 3, 2, edit_title("n1", "winner")), "A").await; // seq 3
        // Stale op: authored against revision 2, arrives last (seq 4), loses.
        handle.apply_envelope(envelope("B", 1, 2, edit_title("n1", "stale")), "B").await; // seq 4
        drop(handle); // crash before checkpoint => recovery replays the journal.
    }
    tokio::time::sleep(Duration::from_millis(50)).await;

    let reborn = CanvasActor::spawn(canvas.clone(), std::sync::Arc::clone(&store));
    let scene = reborn.get_scene().await;
    let title = &scene.nodes.iter().find(|n| n.id == "n1").unwrap().title;
    assert_eq!(
        title, "winner",
        "journal replay reproduces the LWW convergence, not the stale op's value"
    );
}

// ---------------------------------------------------------------------------
// MG-5.2a/MG-5.2b: per-object canonical scene store (region-indexed) + recovery.
// ---------------------------------------------------------------------------

use shape_server::scene_store::canonicalize_scene;
use shape_storage_core::SpatialStore;

fn delete_card(id: &str) -> RenderScenePatch {
    RenderScenePatch::DeleteCard { id: id.to_string() }
}

/// After several edits (incl. a delete), a clean shutdown checkpoints the scene
/// as per-object Records; a fresh actor on the SAME store reconstructs the live
/// scene from those Records (MG5.2a) — including pruning the deleted card.
#[tokio::test]
async fn recovery_from_per_object_records_reproduces_live_scene() {
    let (handle, store) = spawn_actor("c-perobject");

    handle.apply_patch(create_group("g1"), "user-1").await;
    handle.apply_patch(create_card("n1", "g1"), "user-1").await;
    handle.apply_patch(create_card("n2", "g1"), "user-1").await;
    handle.apply_patch(create_card("n3", "g1"), "user-1").await;
    // Delete one card so recovery must prune its per-object Record, not just upsert.
    handle.apply_patch(delete_card("n2"), "user-1").await;
    let before = handle.get_scene().await;

    // Clean shutdown writes the per-object checkpoint, then respawn reconstructs
    // the scene purely from the per-object Records (no journal tail to replay).
    handle.shutdown().await;
    let reborn = CanvasActor::spawn(CanvasId::from("c-perobject"), std::sync::Arc::clone(&store));
    let after = reborn.get_scene().await;

    assert_eq!(after.groups.len(), 1, "group reconstructed from its Record");
    assert_eq!(after.nodes.len(), 2, "deleted card pruned; n1 + n3 remain");
    assert!(after.nodes.iter().all(|n| n.id != "n2"), "n2 is gone");
    // Reconstruction is canonical (id-sorted within kind); compare accordingly.
    assert_eq!(
        after,
        canonicalize_scene(&before),
        "reconstructed scene equals the live scene (canonical order)"
    );
}

/// MG5.2b: edits flow through the actor into the canonical store REGION-INDEXED.
/// After a checkpoint, the spatial index answers a region query for the canvas,
/// and a window selects only the objects whose bbox overlaps it.
#[tokio::test]
async fn checkpointed_objects_are_region_indexed_and_queryable() {
    let (handle, store) = spawn_actor("c-region");

    // g1 at (0,0,400,300); two cards inside it at distinct positions.
    handle.apply_patch(create_group("g1"), "user-1").await;
    handle.apply_patch(create_card("n1", "g1"), "user-1").await; // bounds (10,10,120,80)
    handle
        .apply_patch(
            RenderScenePatch::MoveCard {
                id: "n1".to_string(),
                position: shape_scene_core::WorldPoint { x: 10.0, y: 10.0 },
            },
            "user-1",
        )
        .await;
    // Force the per-object checkpoint to be written.
    handle.shutdown().await;

    // The spatial store now answers a whole-canvas region query: group + card.
    let st = store.lock().unwrap();
    let all: Vec<String> = st
        .query_region("c-region", None)
        .unwrap()
        .map(|r| r.unwrap().id)
        .collect();
    assert!(all.contains(&"c-region:group:g1".to_string()), "group indexed");
    assert!(all.contains(&"c-region:node:n1".to_string()), "card indexed");
    // canvas-meta / tags / comments carry no region row, so they never appear.
    assert!(
        !all.iter().any(|id| id.ends_with(":canvas-meta")),
        "canvas-meta is not region-indexed"
    );

    // A window far from everything returns nothing (the index filters by bbox).
    let none: Vec<String> = st
        .query_region("c-region", Some((10_000.0, 10_000.0, 20_000.0, 20_000.0)))
        .unwrap()
        .map(|r| r.unwrap().id)
        .collect();
    assert!(none.is_empty(), "far window selects no objects, got {none:?}");
}

/// MG5.2a + journal: ops past the last checkpoint are recovered by combining the
/// per-object Records (the checkpoint) with the journal tail replay. Drives more
/// than one checkpoint interval, then crashes (drop, no final checkpoint) so the
/// tail past the checkpoint lives only in the journal.
#[tokio::test]
async fn recovery_combines_per_object_checkpoint_and_journal_tail() {
    let store: SharedStore = std::sync::Arc::new(std::sync::Mutex::new(
        shape_storage_core::SqliteAdapter::open_in_memory().unwrap(),
    ));
    let canvas = CanvasId::from("c-combo");
    let total: i64 = 35; // > CHECKPOINT_INTERVAL (32): one checkpoint + a journal tail.

    {
        let handle = CanvasActor::spawn(canvas.clone(), std::sync::Arc::clone(&store));
        handle.apply_envelope(envelope("c1", 1, 0, create_group("g1")), "c1").await;
        for i in 2..=total {
            handle
                .apply_envelope(
                    envelope("c1", i, i - 1, create_card(&format!("n{i}"), "g1")),
                    "c1",
                )
                .await;
        }
        drop(handle); // crash before a final checkpoint.
    }
    tokio::time::sleep(Duration::from_millis(50)).await;

    let reborn = CanvasActor::spawn(canvas.clone(), std::sync::Arc::clone(&store));
    let scene = reborn.get_scene().await;
    assert_eq!(scene.groups.len(), 1, "group recovered");
    assert_eq!(
        scene.nodes.len() as i64,
        total - 1,
        "all cards recovered across the per-object checkpoint + journal tail"
    );
    let next = reborn
        .apply_envelope(envelope("c1", total + 1, total, create_card("nx", "g1")), "c1")
        .await;
    assert!(
        matches!(next, ApplyResult::Applied { seq, .. } if seq == total + 1),
        "seq continues at {} after recovery, got {next:?}",
        total + 1
    );
}

// ---------------------------------------------------------------------------
// MG-9.5: region-scoped scene snapshot (get_scene_region) from live actor memory.
// ---------------------------------------------------------------------------

use shape_scene_core::Bounds;

/// A group placed at arbitrary bounds (so two groups can sit in far-apart regions).
fn create_group_at(id: &str, bounds: WorldRect) -> RenderScenePatch {
    RenderScenePatch::CreateGroup {
        group: RenderGroup {
            id: id.to_string(),
            title: "G".to_string(),
            summary: String::new(),
            bounds,
            tag_ids: vec![],
            z_index: 0.0,
            style_key: String::new(),
        },
    }
}

/// A card placed at arbitrary bounds inside `group_id`.
fn create_card_at(id: &str, group_id: &str, bounds: WorldRect) -> RenderScenePatch {
    RenderScenePatch::CreateCard {
        card: RenderCard {
            id: id.to_string(),
            group_id: group_id.to_string(),
            title: "C".to_string(),
            summary: String::new(),
            detail: String::new(),
            status: String::new(),
            node_type: String::new(),
            bounds,
            z_index: 0.0,
            style_key: String::new(),
            accessibility_label: String::new(),
        },
    }
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> WorldRect {
    WorldRect { x, y, width: w, height: h }
}

/// Build a canvas with two far-apart clusters: region A near the origin, region B
/// ~100k units away. A window over A returns only A's objects; a window over B
/// returns only B's; `None` returns the whole scene; an edge whose endpoints
/// straddle the window is dropped (only edges with both endpoints kept survive).
#[tokio::test]
async fn get_scene_region_filters_to_window() {
    let (handle, _store) = spawn_actor("c-region-filter");

    // Region A cluster.
    handle.apply_patch(create_group_at("gA", rect(0.0, 0.0, 400.0, 300.0)), "u").await;
    handle.apply_patch(create_card_at("nA1", "gA", rect(10.0, 10.0, 120.0, 80.0)), "u").await;
    handle.apply_patch(create_card_at("nA2", "gA", rect(200.0, 10.0, 120.0, 80.0)), "u").await;
    // An edge fully inside region A: both endpoints kept by an A-window.
    handle
        .apply_patch(
            RenderScenePatch::CreateEdge {
                group_id: "gA".to_string(),
                source: "nA1".to_string(),
                target: "nA2".to_string(),
                edge_id: "eA".to_string(),
                label: None,
            },
            "u",
        )
        .await;

    // Region B cluster, far from A.
    handle
        .apply_patch(create_group_at("gB", rect(100_000.0, 100_000.0, 400.0, 300.0)), "u")
        .await;
    handle
        .apply_patch(create_card_at("nB1", "gB", rect(100_010.0, 100_010.0, 120.0, 80.0)), "u")
        .await;

    let window_a = Bounds { x: -50.0, y: -50.0, width: 600.0, height: 500.0 };
    let window_b = Bounds { x: 99_900.0, y: 99_900.0, width: 700.0, height: 600.0 };

    // Window A: only A's group, A's two cards, and the intra-A edge.
    let a = handle.get_scene_region(Some(window_a)).await;
    let a_groups: Vec<&str> = a.groups.iter().map(|g| g.id.as_str()).collect();
    let a_nodes: Vec<&str> = a.nodes.iter().map(|n| n.id.as_str()).collect();
    let a_edges: Vec<&str> = a.edges.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(a_groups, vec!["gA"], "window A keeps only group gA");
    assert_eq!(a_nodes, vec!["nA1", "nA2"], "window A keeps only A's cards");
    assert_eq!(a_edges, vec!["eA"], "intra-A edge kept (both endpoints in window)");

    // Window B: only B's group + card, and no edges (eA's endpoints are off-window).
    let b = handle.get_scene_region(Some(window_b)).await;
    let b_groups: Vec<&str> = b.groups.iter().map(|g| g.id.as_str()).collect();
    let b_nodes: Vec<&str> = b.nodes.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(b_groups, vec!["gB"], "window B keeps only group gB");
    assert_eq!(b_nodes, vec!["nB1"], "window B keeps only B's card");
    assert!(b.edges.is_empty(), "edge dropped: its endpoints are outside window B");

    // None: the whole scene, with the canvas revision intact.
    let all = handle.get_scene_region(None).await;
    assert_eq!(all.groups.len(), 2, "whole-canvas snapshot has both groups");
    assert_eq!(all.nodes.len(), 3, "whole-canvas snapshot has all three cards");
    assert_eq!(all.edges.len(), 1, "whole-canvas snapshot has the edge");
    let full = handle.get_scene().await;
    assert_eq!(all.scene_version, full.scene_version, "None == full scene revision");
    assert_eq!(a.scene_version, full.scene_version, "windowed snapshot reports the true revision");
}
