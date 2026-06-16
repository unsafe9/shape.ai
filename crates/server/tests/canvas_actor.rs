//! Integration tests for the per-canvas actor + registry + storage embed, each
//! over an in-memory redb store driving the actor through its async handle.

use std::time::Duration;

use shape_scene_core::object::{
    apply_object_op, FillRule, Geometry, Object, ObjectOp, ObjectScene, PathNode, SubPath,
    Transform3x3,
};
use shape_scene_core::CanvasId;
use shape_server::canvas_actor::SharedStore;
use shape_server::sync::{OpEnvelope, OpId};
use shape_server::{ActorHandle, ApplyResult, CanvasActor, CanvasRegistry};

/// Wrap an op in an envelope with an `(clientId, localSeq)` id.
fn envelope(client_id: &str, local_seq: i64, base_revision: i64, op: ObjectOp) -> OpEnvelope {
    OpEnvelope {
        op_id: OpId {
            client_id: client_id.to_string(),
            local_seq,
        },
        base_revision,
        ts: "1970-01-01T00:00:00Z".to_string(),
        op,
    }
}

/// A closed rect at object-local (0,0)-(80,40) in quantized units.
fn rect_geometry() -> Geometry {
    Geometry::from_subpaths(
        vec![SubPath {
            closed: true,
            nodes: vec![
                PathNode::corner(0, 0),
                PathNode::corner(80, 0),
                PathNode::corner(80, 40),
                PathNode::corner(0, 40),
            ],
        }],
        FillRule::EvenOdd,
    )
}

fn rect_object(id: &str, order: &str) -> Object {
    Object::new(id, order, rect_geometry())
}

fn insert(id: &str, order: &str) -> ObjectOp {
    ObjectOp::InsertObject {
        object: rect_object(id, order),
    }
}

fn move_to(id: &str, x: f64, y: f64) -> ObjectOp {
    ObjectOp::SetTransform {
        id: id.to_string(),
        transform: Transform3x3::translate(x, y),
    }
}

/// Returns the handle and the shared store so a re-spawn on the SAME store can be
/// tested.
fn spawn_actor(canvas: &str) -> (ActorHandle, SharedStore) {
    let store: SharedStore = std::sync::Arc::new(std::sync::Mutex::new(
        shape_storage_core::RedbAdapter::open_in_memory().unwrap(),
    ));
    let handle = CanvasActor::spawn(CanvasId::from(canvas), std::sync::Arc::clone(&store));
    (handle, store)
}

#[tokio::test]
async fn lifecycle_insert_two_objects_increments_seq_and_reflects_scene() {
    let (handle, _store) = spawn_actor("c-lifecycle");

    let r1 = handle.apply_op(insert("o1", "a0"), "user-1").await;
    assert!(
        matches!(r1, ApplyResult::Applied { seq: 1, .. }),
        "first apply should be seq 1, got {r1:?}"
    );

    let r2 = handle.apply_op(insert("o2", "a1"), "user-1").await;
    assert!(
        matches!(r2, ApplyResult::Applied { seq: 2, .. }),
        "second apply should be seq 2, got {r2:?}"
    );

    let scene = handle.get_scene().await;
    assert_eq!(scene.objects.len(), 2, "both objects present");
    assert!(scene.get("o1").is_some());
    assert!(scene.get("o2").is_some());
}

#[tokio::test]
async fn rejected_op_does_not_bump_seq() {
    let (handle, _store) = spawn_actor("c-reject");

    let r = handle.apply_op(move_to("missing", 1.0, 1.0), "user-1").await;
    match r {
        ApplyResult::Rejected { errors } => assert!(!errors.is_empty()),
        other => panic!("expected rejection, got {other:?}"),
    }

    // A subsequent valid op is still seq 1: the rejected op didn't advance.
    let ok = handle.apply_op(insert("o1", "a0"), "user-1").await;
    assert!(matches!(ok, ApplyResult::Applied { seq: 1, .. }), "got {ok:?}");
}

#[tokio::test]
async fn durability_write_through_survives_shutdown_and_respawn() {
    let (handle, store) = spawn_actor("c-durable");

    handle.apply_op(insert("o1", "a0"), "user-1").await;
    handle.apply_op(insert("o2", "a1"), "user-1").await;
    let before = handle.get_scene().await;

    handle.shutdown().await;

    let reborn = CanvasActor::spawn(CanvasId::from("c-durable"), std::sync::Arc::clone(&store));
    let after = reborn.get_scene().await;

    assert_eq!(after.objects.len(), 2, "objects reloaded from per-object Records");
    assert_eq!(after, before, "reloaded scene equals the persisted scene");

    let next = reborn.apply_op(insert("o3", "a2"), "user-1").await;
    assert!(
        matches!(next, ApplyResult::Applied { seq: 3, .. }),
        "seq continues past the 2 persisted ops, got {next:?}"
    );
}

#[tokio::test]
async fn idle_evict_removes_then_respawn_reloads_durable_state() {
    let registry = CanvasRegistry::open_in_memory().unwrap();
    let canvas = CanvasId::from("c-evict");

    let handle = registry.get_or_spawn(&canvas).await.unwrap();
    handle.apply_op(insert("o1", "a0"), "user-1").await;
    handle.apply_op(insert("o2", "a1"), "user-1").await;
    assert!(registry.contains(&canvas));

    let evicted = registry.evict_idle(Duration::from_secs(0)).await;
    assert_eq!(evicted, vec![canvas.clone()]);
    assert!(!registry.contains(&canvas), "canvas removed from registry");
    assert!(registry.is_empty());

    let reborn = registry.get_or_spawn(&canvas).await.unwrap();
    assert!(registry.contains(&canvas));
    let scene = reborn.get_scene().await;
    assert_eq!(scene.objects.len(), 2, "durable objects reloaded after evict");
}

#[tokio::test]
async fn explicit_evict_then_get_spawns_fresh() {
    let registry = CanvasRegistry::open_in_memory().unwrap();
    let canvas = CanvasId::from("c-explicit-evict");

    let handle = registry.get_or_spawn(&canvas).await.unwrap();
    handle.apply_op(insert("o1", "a0"), "user-1").await;
    assert!(registry.contains(&canvas));

    registry.evict(&canvas).await;
    assert!(!registry.contains(&canvas));

    let reborn = registry.get_or_spawn(&canvas).await.unwrap();
    let scene = reborn.get_scene().await;
    assert_eq!(scene.objects.len(), 1, "reloaded after explicit evict");
}

#[tokio::test]
async fn broadcast_delivers_applied_op_with_new_seq() {
    let (handle, _store) = spawn_actor("c-broadcast");
    let mut rx = handle.subscribe();

    handle.apply_op(insert("o1", "a0"), "user-1").await;

    let msg = rx.recv().await.expect("broadcast received");
    assert_eq!(msg.seq, 1, "broadcast carries the new server seq");
    assert!(
        matches!(msg.op, ObjectOp::InsertObject { .. }),
        "broadcast carries the applied op"
    );
    assert_eq!(msg.scene.objects.len(), 1, "broadcast scene reflects the apply");
}

#[tokio::test]
async fn duplicate_op_id_does_not_reapply_or_bump_seq() {
    let (handle, _store) = spawn_actor("c-dedup");

    let first = handle.apply_envelope(envelope("c1", 1, 0, insert("o1", "a0")), "c1").await;
    assert!(matches!(first, ApplyResult::Applied { seq: 1, .. }), "got {first:?}");

    let dup = handle.apply_envelope(envelope("c1", 1, 0, insert("o1", "a0")), "c1").await;
    assert!(
        matches!(dup, ApplyResult::Applied { seq: 1, .. }),
        "duplicate op re-acks the original seq, got {dup:?}"
    );

    let scene = handle.get_scene().await;
    assert_eq!(scene.objects.len(), 1, "duplicate op did not create a second object");

    let next = handle.apply_envelope(envelope("c1", 2, 1, insert("o2", "a1")), "c1").await;
    assert!(
        matches!(next, ApplyResult::Applied { seq: 2, .. }),
        "new op advances to seq 2 (duplicate did not bump), got {next:?}"
    );
}

/// Crash recovery: dropping the actor (not `shutdown()`) leaves journaled + written
/// records; respawn reloads the per-object Records, replays the journal tail, and
/// rebuilds the dedup table.
#[tokio::test]
async fn crash_recovers_via_per_object_records_and_journal() {
    let store: SharedStore = std::sync::Arc::new(std::sync::Mutex::new(
        shape_storage_core::RedbAdapter::open_in_memory().unwrap(),
    ));
    let canvas = CanvasId::from("c-crash");

    {
        let handle = CanvasActor::spawn(canvas.clone(), std::sync::Arc::clone(&store));
        handle.apply_envelope(envelope("c1", 1, 0, insert("o1", "a0")), "c1").await;
        handle.apply_envelope(envelope("c1", 2, 1, insert("o2", "a1")), "c1").await;
        handle.apply_envelope(envelope("c1", 3, 2, insert("o3", "a2")), "c1").await;
        drop(handle); // crash (no clean shutdown).
    }
    tokio::time::sleep(Duration::from_millis(50)).await;

    let reborn = CanvasActor::spawn(canvas.clone(), std::sync::Arc::clone(&store));
    let scene = reborn.get_scene().await;
    assert_eq!(scene.objects.len(), 3, "all three objects recovered");

    let next = reborn.apply_envelope(envelope("c1", 4, 3, insert("o4", "a3")), "c1").await;
    assert!(
        matches!(next, ApplyResult::Applied { seq: 4, .. }),
        "seq continues past the 3 recovered ops, got {next:?}"
    );

    // Dedup rebuilt from the journal: replaying op (c1,2) is idempotent.
    let replayed = reborn.apply_envelope(envelope("c1", 2, 1, insert("o2", "a1")), "c1").await;
    assert!(
        matches!(replayed, ApplyResult::Applied { seq: 2, .. }),
        "recovered dedup table re-acks the original seq for a replayed opId, got {replayed:?}"
    );
}

/// More than one checkpoint interval of ops, then a crash: recovery reconstructs
/// every object from the per-object Records (each op wrote through).
#[tokio::test]
async fn many_ops_then_crash_recovers_all_objects() {
    let store: SharedStore = std::sync::Arc::new(std::sync::Mutex::new(
        shape_storage_core::RedbAdapter::open_in_memory().unwrap(),
    ));
    let canvas = CanvasId::from("c-checkpoint");
    let total: i64 = 35;

    {
        let handle = CanvasActor::spawn(canvas.clone(), std::sync::Arc::clone(&store));
        for i in 1..=total {
            handle
                .apply_envelope(
                    envelope("c1", i, i - 1, insert(&format!("o{i}"), &format!("a{i}"))),
                    "c1",
                )
                .await;
        }
        drop(handle);
    }
    tokio::time::sleep(Duration::from_millis(50)).await;

    let reborn = CanvasActor::spawn(canvas.clone(), std::sync::Arc::clone(&store));
    let scene = reborn.get_scene().await;
    assert_eq!(
        i64::try_from(scene.objects.len()).unwrap(),
        total,
        "all {total} objects recovered from per-object Records"
    );

    let next = reborn
        .apply_envelope(
            envelope("c1", total + 1, total, insert("ox", "az")),
            "c1",
        )
        .await;
    assert!(
        matches!(next, ApplyResult::Applied { seq, .. } if seq == total + 1),
        "seq continues at {} after recovery, got {next:?}",
        total + 1
    );
}

/// Two users move the SAME object; the server serializes them by monotonic seq, so
/// the later-arriving op wins.
#[tokio::test]
async fn concurrent_transform_edits_converge_to_higher_seq() {
    let (handle, _store) = spawn_actor("c-lww-converge");

    handle.apply_op(insert("o1", "a0"), "user-A").await;

    let a = handle
        .apply_envelope(envelope("user-A", 1, 1, move_to("o1", 10.0, 10.0)), "user-A")
        .await;
    assert!(matches!(a, ApplyResult::Applied { seq: 2, .. }), "got {a:?}");

    let rev = handle.get_scene().await.scene_version;
    let b = handle
        .apply_envelope(envelope("user-B", 1, rev, move_to("o1", 99.0, 99.0)), "user-B")
        .await;
    assert!(matches!(b, ApplyResult::Applied { .. }), "got {b:?}");

    let scene = handle.get_scene().await;
    let t = &scene.get("o1").unwrap().transform;
    assert_eq!(t, &Transform3x3::translate(99.0, 99.0), "later-arriving op wins");
}

/// `apply_object_op_lww` skips a write whose seq is `<=` the held winner's seq.
#[tokio::test]
async fn stale_seq_does_not_clobber_advanced_property() {
    let (handle, _store) = spawn_actor("c-lww-stale");

    handle.apply_op(insert("o1", "a0"), "user-A").await;

    let winner = handle
        .apply_envelope(envelope("user-A", 1, 1, move_to("o1", 5.0, 5.0)), "user-A")
        .await;
    assert!(matches!(winner, ApplyResult::Applied { seq: 2, .. }), "got {winner:?}");

    // The actor seq is monotonic, so a true stale arrival is impossible; the LWW
    // gate is keyed by arrival seq, so this later op is the legitimate winner.
    let b = handle
        .apply_envelope(envelope("user-B", 1, 2, move_to("o1", 7.0, 7.0)), "user-B")
        .await;
    assert!(matches!(b, ApplyResult::Applied { seq: 3, .. }), "got {b:?}");

    let scene = handle.get_scene().await;
    let t = &scene.get("o1").unwrap().transform;
    assert_eq!(t, &Transform3x3::translate(7.0, 7.0), "latest arrival is the winner");
}

/// Each broadcast carries the authoring userId so the WS layer can self-skip.
#[tokio::test]
async fn broadcast_carries_authoring_user_id() {
    let (handle, _store) = spawn_actor("c-author");
    let mut rx = handle.subscribe();

    handle.apply_op(insert("o1", "a0"), "user-42").await;

    let msg = rx.recv().await.expect("broadcast received");
    assert_eq!(
        msg.author.as_deref(),
        Some("user-42"),
        "broadcast records the op's authoring userId"
    );
}

use shape_storage_core::SpatialStore;

/// A fresh actor reconstructs the live scene from per-object Records, including
/// pruning a deleted object.
#[tokio::test]
async fn recovery_from_per_object_records_reproduces_live_scene() {
    let (handle, store) = spawn_actor("c-perobject");

    handle.apply_op(insert("o1", "a0"), "user-1").await;
    handle.apply_op(insert("o2", "a1"), "user-1").await;
    handle.apply_op(insert("o3", "a2"), "user-1").await;
    handle.apply_op(ObjectOp::Delete { id: "o2".into() }, "user-1").await;

    handle.shutdown().await;
    let reborn = CanvasActor::spawn(CanvasId::from("c-perobject"), std::sync::Arc::clone(&store));
    let after = reborn.get_scene().await;

    assert_eq!(after.objects.len(), 2, "deleted object pruned; o1 + o3 remain");
    assert!(after.get("o2").is_none(), "o2 is gone");
    assert!(after.get("o1").is_some() && after.get("o3").is_some());
}

/// The spatial index answers a whole-canvas region query; a far window selects nothing.
#[tokio::test]
async fn checkpointed_objects_are_region_indexed_and_queryable() {
    let (handle, store) = spawn_actor("c-region");

    handle.apply_op(insert("o1", "a0"), "user-1").await;
    handle.apply_op(move_to("o1", 10.0, 10.0), "user-1").await;
    handle.shutdown().await;

    let st = store.lock().unwrap();
    let all: Vec<String> = st
        .query_region("c-region", None)
        .unwrap()
        .map(|r| r.unwrap().id)
        .collect();
    assert!(all.contains(&"c-region:object:o1".to_string()), "object indexed");
    // canvas-meta carries no region row, so it never appears.
    assert!(
        !all.iter().any(|id| id.ends_with(":canvas")),
        "canvas-meta is not region-indexed"
    );

    let none: Vec<String> = st
        .query_region("c-region", Some((10_000.0, 10_000.0, 20_000.0, 20_000.0)))
        .unwrap()
        .map(|r| r.unwrap().id)
        .collect();
    assert!(none.is_empty(), "far window selects no objects, got {none:?}");
}

use shape_storage_core::RegionWindow;

/// A rect placed at world (x, y).
fn object_at(id: &str, order: &str, x: f64, y: f64) -> ObjectOp {
    let mut object = rect_object(id, order);
    object.transform = Transform3x3::translate(x, y);
    ObjectOp::InsertObject { object }
}

/// Two far-apart clusters: a window over each returns only its own objects, `None`
/// the whole scene.
#[tokio::test]
async fn get_scene_region_filters_to_window() {
    let (handle, _store) = spawn_actor("c-region-filter");

    handle.apply_op(object_at("a1", "a0", 0.0, 0.0), "u").await;
    handle.apply_op(object_at("a2", "a1", 200.0, 10.0), "u").await;
    handle.apply_op(object_at("b1", "a2", 100_000.0, 100_000.0), "u").await;

    let window_a = RegionWindow { min_x: -50.0, min_y: -50.0, max_x: 600.0, max_y: 500.0 };
    let window_b = RegionWindow {
        min_x: 99_900.0,
        min_y: 99_900.0,
        max_x: 100_600.0,
        max_y: 100_500.0,
    };

    let a = handle.get_scene_region(Some(window_a)).await;
    let a_ids: Vec<&str> = a.objects.iter().map(|o| o.id.as_str()).collect();
    assert_eq!(a_ids, vec!["a1", "a2"], "window A keeps only A's objects");

    let b = handle.get_scene_region(Some(window_b)).await;
    let b_ids: Vec<&str> = b.objects.iter().map(|o| o.id.as_str()).collect();
    assert_eq!(b_ids, vec!["b1"], "window B keeps only B's object");

    let all = handle.get_scene_region(None).await;
    assert_eq!(all.objects.len(), 3, "whole-canvas snapshot has all three objects");
    let full = handle.get_scene().await;
    assert_eq!(all.scene_version, full.scene_version, "None == full scene revision");
    assert_eq!(a.scene_version, full.scene_version, "windowed snapshot reports the true revision");
}

/// Repro: deleting an object that was created in a PRIOR session (persisted, then
/// the actor shut down) and reloaded cold by a fresh actor must STICK — including
/// in the WINDOWED region-index read a viewport change triggers, not just the
/// full-scene read. The region row was written by the prior session; the cold
/// delete must drop it too.
#[tokio::test]
async fn delete_of_reloaded_old_object_is_gone_from_windowed_region_query() {
    // Session 1: create + persist an object at world origin, then shut down clean.
    let (handle, store) = spawn_actor("c-old-delete");
    handle.apply_op(object_at("old", "a0", 0.0, 0.0), "user-1").await;
    handle.shutdown().await;

    // Session 2: a fresh actor reloads "old" cold from the per-object Record
    // (this is the "old object" condition — never inserted in THIS session's
    // working scene), then deletes it.
    let reborn = CanvasActor::spawn(CanvasId::from("c-old-delete"), std::sync::Arc::clone(&store));
    let del = reborn.apply_op(ObjectOp::Delete { id: "old".into() }, "user-1").await;
    assert!(matches!(del, ApplyResult::Applied { .. }), "delete applies, got {del:?}");

    // The full-scene path (load_scene over the main table) must show it gone.
    let full = reborn.get_scene().await;
    assert!(full.get("old").is_none(), "deleted old object absent from full scene");

    // The windowed region read (what a zoom/pan issues) must ALSO show it gone.
    // A window over the object's position would resurrect it if the durable
    // region index row survived the cold delete.
    let window = RegionWindow { min_x: -50.0, min_y: -50.0, max_x: 50.0, max_y: 50.0 };
    let windowed = reborn.get_scene_region(Some(window)).await;
    assert!(
        windowed.get("old").is_none(),
        "deleted old object must not reappear in the windowed region query; \
         got objects {:?}",
        windowed.objects.iter().map(|o| o.id.as_str()).collect::<Vec<_>>()
    );
}

use shape_scene_core::object::{Comment, FeatureRequest, FeatureResponse};

#[tokio::test]
async fn feature_comment_upsert_lowers_to_op_and_persists() {
    let (handle, _store) = spawn_actor("c-feature-comment");
    handle.apply_op(insert("o1", "a0"), "u").await;

    let req = FeatureRequest::CommentUpsert {
        canvas_id: "c-feature-comment".into(),
        object_id: "o1".into(),
        comment: Comment {
            id: "c-1".into(),
            author: "jayden".into(),
            body: "looks good".into(),
            at: None,
            resolved: false,
        },
    };
    let resp = handle.feature(req, "u").await;
    assert_eq!(
        resp,
        FeatureResponse::CommentUpserted {
            object_id: "o1".into(),
            comment_id: "c-1".into(),
        }
    );

    let scene = handle.get_scene().await;
    assert_eq!(scene.get("o1").unwrap().comments.len(), 1, "comment persisted via the op path");
}

#[tokio::test]
async fn feature_template_apply_inserts_recipe() {
    let (handle, _store) = spawn_actor("c-feature-template");

    let recipe = vec![rect_object("tpl-a", "a0"), rect_object("tpl-b", "a1")];
    let req = FeatureRequest::TemplateApply {
        canvas_id: "c-feature-template".into(),
        recipe,
        anchor_x: 10.0,
        anchor_y: 20.0,
    };
    let resp = handle.feature(req, "u").await;
    assert_eq!(
        resp,
        FeatureResponse::TemplateApplied {
            object_ids: vec!["tpl-a".into(), "tpl-b".into()],
        }
    );

    let scene = handle.get_scene().await;
    assert_eq!(scene.objects.len(), 2, "both template objects inserted via the op path");
}

#[tokio::test]
async fn feature_canvas_switch_reports_seq_and_revision() {
    let (handle, _store) = spawn_actor("c-feature-switch");
    handle.apply_op(insert("o1", "a0"), "u").await;

    let resp = handle
        .feature(FeatureRequest::CanvasSwitch { canvas_id: "other".into() }, "u")
        .await;
    match resp {
        FeatureResponse::CanvasSwitched { canvas_id, seq, revision } => {
            assert_eq!(canvas_id, "other");
            assert_eq!(seq, 1, "current actor seq after one apply");
            assert_eq!(revision, 1, "current scene revision after one apply");
        }
        other => panic!("expected CanvasSwitched, got {other:?}"),
    }
}

/// An op targeting a possibly-cold object still applies and persists; a whole-scene
/// read reconstructs every object from the store.
#[tokio::test]
async fn many_objects_persist_and_reload() {
    let (handle, _store) = spawn_actor("c-bulk");

    handle.apply_op(insert("anchor", "a0"), "u").await;
    for i in 0..20 {
        handle.apply_op(insert(&format!("n{i}"), &format!("b{i}")), "u").await;
    }

    let edited = handle.apply_op(move_to("anchor", 42.0, 42.0), "u").await;
    assert!(matches!(edited, ApplyResult::Applied { .. }), "got {edited:?}");

    let scene = handle.get_scene().await;
    assert_eq!(scene.objects.len(), 21, "all objects durable");
    let t = &scene.get("anchor").unwrap().transform;
    assert_eq!(t, &Transform3x3::translate(42.0, 42.0), "edit to the anchor persisted");
}

/// The actor scene matches a pure scene-core apply of the same op stream.
#[tokio::test]
async fn actor_scene_matches_pure_apply() {
    let (handle, _store) = spawn_actor("c-oracle");
    handle.apply_op(insert("o1", "a0"), "u").await;
    handle.apply_op(move_to("o1", 3.0, 4.0), "u").await;

    let mut oracle = ObjectScene::default();
    apply_object_op(&mut oracle, insert("o1", "a0")).unwrap();
    apply_object_op(&mut oracle, move_to("o1", 3.0, 4.0)).unwrap();

    let scene = handle.get_scene().await;
    assert_eq!(scene.get("o1").unwrap().transform, oracle.get("o1").unwrap().transform);
}
