//! Port of `tests/scene-client.test.ts`: the data-layer contract the shell
//! consumes. The TS `SceneClient` composes transport + engine + outbox; the
//! behavioral core (optimistic object-op apply -> outbox WireOp -> send -> drop on
//! ack; reconnect reload + unacked replay) is engine logic and is covered here.
//! The selection-is-presence-only and feature-channel assertions are pure routing
//! decisions of the shell wrapper (no document op, no outbox entry), captured by
//! the structural fact that only an `ObjectOp` ever reaches `author`.

mod common;

use common::*;
use shape_client_runtime::outbox::InMemoryOutboxStore;
use shape_client_runtime::sync_engine::SyncEngine;
use shape_scene_core::object::op::ObjectOp;

fn boot() -> (SyncEngine<CaptureTransport, InMemoryOutboxStore>, FixedNow) {
    let engine = SyncEngine::new(empty_scene(), "c1", InMemoryOutboxStore::new(), CaptureTransport::new(), None);
    (engine, FixedNow::new())
}

#[test]
fn welcome_scene_is_the_current_scene() {
    let mut seeded = empty_scene();
    seeded.scene_version = 2;
    let engine = SyncEngine::new(seeded.clone(), "c1", InMemoryOutboxStore::new(), CaptureTransport::new(), None);
    assert_eq!(engine.scene(), &seeded);
    assert_eq!(engine.base_revision(), 2);
}

#[test]
fn applies_object_op_optimistically_enqueues_wireop_sends_it_and_drops_on_ack() {
    let (mut engine, mut now) = boot();

    let res = engine.author(insert(rect("a", "a0")), &now.next()).unwrap();
    assert_eq!(res.errors, Vec::<String>::new());
    assert_eq!(res.op_id, Some(op_id("c1", 1)));
    // The inverse op is captured (the undo entry, D21).
    assert_eq!(res.inverse, Some(ObjectOp::Delete { id: "a".to_string() }));

    // Optimistic local apply happened immediately.
    assert_eq!(object_ids(engine.scene()), vec!["a"]);

    // Persisted to the outbox as a WireOp before any send.
    assert_eq!(engine.outbox_len(), 1);
    assert_eq!(engine.transport().batches.len(), 0);

    engine.on_flush_due();
    let frames = &engine.transport().batches;
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0][0].op_id, op_id("c1", 1));
    let delta: ObjectOp = serde_json::from_value(frames[0][0].prop_delta.clone()).unwrap();
    assert_eq!(delta, insert(rect("a", "a0")));

    engine.on_ack(&[op_id("c1", 1)], Some(1)).unwrap();
    assert_eq!(engine.outbox_len(), 0);
}

#[test]
fn a_rejected_op_never_enters_the_outbox_or_the_wire() {
    let (mut engine, mut now) = boot();
    // Move a non-existent object: scene-core rejects (NotFound); nothing persists.
    let res = engine.author(move_op("ghost", 10.0, 10.0), &now.next()).unwrap();
    assert!(!res.errors.is_empty());
    assert_eq!(res.op_id, None);
    assert_eq!(res.inverse, None);
    assert_eq!(engine.outbox_len(), 0);
    assert!(!engine.flush_armed());
    // The scene is untouched.
    assert_eq!(object_ids(engine.scene()), Vec::<String>::new());
}

#[test]
fn reloads_the_snapshot_on_a_reconnect_welcome_and_replays_unacked_ops() {
    let (mut engine, mut now) = boot();

    engine.author(insert(rect("a", "a0")), &now.next()).unwrap();
    engine.on_flush_due();
    engine.transport_mut().batches.clear(); // forget the first send

    let mut reconnected = empty_scene();
    reconnected.scene_version = 5;
    engine.reconcile_snapshot(reconnected).unwrap();

    // The unacked insert is replayed on top of the snapshot, so "a" survives.
    assert_eq!(object_ids(engine.scene()), vec!["a"]);

    let replayed: Vec<i64> = engine.transport().flat().iter().map(|e| e.op_id.local_seq).collect();
    assert!(replayed.contains(&1));
    assert_eq!(engine.outbox_len(), 1);
}
