//! Durable outbox, optimistic apply + unacked discard, coalescing, and reconnect
//! reconcile: author -> outbox -> coalesced send -> ack-clears-outbox, remote
//! discard, and reconnect replay.

mod common;

use common::*;
use shape_client_runtime::outbox::{op_id_key, InMemoryOutboxStore};
use shape_client_runtime::sync_engine::{SyncEngine, COALESCE_MS};
use shape_scene_core::object::model::Transform3x3;
use shape_scene_core::object::op::ObjectOp;

fn boot() -> (SyncEngine<CaptureTransport, InMemoryOutboxStore>, FixedNow) {
    let engine = SyncEngine::new(
        empty_scene(),
        "c1",
        InMemoryOutboxStore::new(),
        CaptureTransport::new(),
        None,
    );
    (engine, FixedNow::new())
}

#[test]
fn appends_before_send_removes_on_ack() {
    let (mut engine, mut now) = boot();

    let res = engine.author(insert(rect("a", "a0")), &now.next()).unwrap();
    assert_eq!(res.op_id, Some(op_id("c1", 1)));

    // Persisted before any send (the coalescing flush has not fired yet).
    assert_eq!(engine.outbox_len(), 1);
    assert_eq!(engine.transport().batches.len(), 0);
    assert!(engine.flush_armed());

    engine.on_flush_due();
    assert_eq!(engine.transport().batches.len(), 1);
    assert_eq!(engine.transport().flat()[0].op_id, op_id("c1", 1));
    let delta: ObjectOp =
        serde_json::from_value(engine.transport().flat()[0].prop_delta.clone()).unwrap();
    assert_eq!(delta, insert(rect("a", "a0")));

    engine.on_ack(&[op_id("c1", 1)], Some(1)).unwrap();
    assert_eq!(engine.outbox_len(), 0);
}

#[test]
fn captures_inverse_op_for_undo() {
    let (mut engine, mut now) = boot();
    let res = engine.author(insert(rect("a", "a0")), &now.next()).unwrap();
    assert_eq!(
        res.inverse,
        Some(ObjectOp::Delete {
            id: "a".to_string()
        })
    );
}

#[test]
fn persists_across_reconnect_and_replays_in_localseq_order_dedup_safe() {
    let outbox = InMemoryOutboxStore::new();
    let mut now = FixedNow::new();

    let mut engine1 = SyncEngine::new(empty_scene(), "c1", outbox, CaptureTransport::new(), None);
    engine1.author(insert(rect("a", "a0")), &now.next()).unwrap();
    engine1.author(insert(rect("b", "a1")), &now.next()).unwrap();
    engine1.on_flush_due();
    let first_send_ids: Vec<String> = engine1.transport().flat().iter().map(|e| op_id_key(&e.op_id)).collect();
    assert_eq!(first_send_ids, vec!["c1:1", "c1:2"]);

    // Reconnect: a fresh engine reusing the SAME outbox replays on a snapshot.
    let outbox = engine1.into_outbox();
    let mut engine2 = SyncEngine::new(empty_scene(), "c1", outbox, CaptureTransport::new(), None);
    engine2.reconcile_snapshot(empty_scene()).unwrap();
    let replayed: Vec<String> = engine2.transport().flat().iter().map(|e| op_id_key(&e.op_id)).collect();
    assert_eq!(replayed, vec!["c1:1", "c1:2"]);
    assert_eq!(replayed, first_send_ids);
}

#[test]
fn reflects_authored_op_in_local_scene_immediately() {
    let (mut engine, mut now) = boot();
    engine.author(insert(rect("a", "a0")), &now.next()).unwrap();
    assert_eq!(object_ids(engine.scene()), vec!["a"]);

    engine.author(move_op("a", 99.0, 99.0), &now.next()).unwrap();
    assert_eq!(object_transform(engine.scene(), "a"), Some(translate(99.0, 99.0)));
}

#[test]
fn ignores_remote_write_to_unacked_field_then_applies_after_ack() {
    let (mut engine, mut now) = boot();
    engine.author(insert(rect("a", "a0")), &now.next()).unwrap();

    // We now OWN (a, transform) until it is acked.
    let res = engine.author(move_op("a", 50.0, 50.0), &now.next()).unwrap();
    assert_eq!(object_transform(engine.scene(), "a"), Some(translate(50.0, 50.0)));

    // A peer's remote move to the same field is dropped (transient ownership).
    assert!(!engine.apply_remote(move_op("a", 7.0, 7.0)));
    assert_eq!(object_transform(engine.scene(), "a"), Some(translate(50.0, 50.0)));

    // After our op is acked, ownership releases and a remote write applies.
    engine.on_ack(&[res.op_id.unwrap()], Some(3)).unwrap();
    assert!(engine.apply_remote(move_op("a", 7.0, 7.0)));
    assert_eq!(object_transform(engine.scene(), "a"), Some(translate(7.0, 7.0)));
}

#[test]
fn lets_remote_write_to_different_field_on_same_object_through() {
    let (mut engine, mut now) = boot();
    engine.author(insert(rect("a", "a0")), &now.next()).unwrap();

    // Own (a, transform) only.
    engine.author(move_op("a", 50.0, 50.0), &now.next()).unwrap();

    // A remote text edit touches a different field -> applies.
    assert!(engine.apply_remote(text_op("a", "from-peer")));
    assert_eq!(object_text(engine.scene(), "a").as_deref(), Some("from-peer"));
    assert_eq!(object_transform(engine.scene(), "a"), Some(translate(50.0, 50.0)));
}

#[test]
fn batches_n_rapid_ops_within_one_window_into_a_single_frame() {
    let mut engine = SyncEngine::new(
        empty_scene(),
        "c1",
        InMemoryOutboxStore::new(),
        CaptureTransport::new(),
        Some(COALESCE_MS),
    );
    let mut now = FixedNow::new();

    engine.author(insert(rect("a", "a0")), &now.next()).unwrap();
    // A burst of rapid moves within one coalescing window.
    engine.author(move_op("a", 1.0, 1.0), &now.next()).unwrap();
    engine.author(move_op("a", 2.0, 2.0), &now.next()).unwrap();
    engine.author(move_op("a", 3.0, 3.0), &now.next()).unwrap();

    assert_eq!(engine.transport().batches.len(), 0);
    assert!(engine.flush_armed());

    engine.on_flush_due();
    assert_eq!(engine.transport().batches.len(), 1);
    assert_eq!(engine.transport().batches[0].len(), 4);

    engine.author(move_op("a", 4.0, 4.0), &now.next()).unwrap();
    engine.on_flush_due();
    assert_eq!(engine.transport().batches.len(), 2);
    assert_eq!(engine.transport().batches[1].len(), 1);
}

#[test]
fn flush_drains_the_buffer_immediately_without_the_timer() {
    let (mut engine, mut now) = boot();
    engine.author(insert(rect("a", "a0")), &now.next()).unwrap();
    assert_eq!(engine.transport().batches.len(), 0);
    engine.flush();
    assert_eq!(engine.transport().batches.len(), 1);
    assert!(!engine.flush_armed());
}

#[test]
fn rebases_on_snapshot_replays_unacked_ops_and_converges() {
    let (mut engine, mut now) = boot();

    // The client created "a" (acked) then moved it; the move is still unacked
    // when the socket drops.
    engine.author(insert(rect("a", "a0")), &now.next()).unwrap();
    engine.on_ack(&[op_id("c1", 1)], Some(1)).unwrap();
    let res = engine.author(move_op("a", 80.0, 80.0), &now.next()).unwrap();
    assert_eq!(engine.outbox_len(), 1);

    // The server welcome carries "a" at its pre-move position plus a peer's text
    // edit the client never saw.
    let mut server_scene = engine.scene().clone();
    server_scene.scene_version = 3;
    for o in &mut server_scene.objects {
        if o.id == "a" {
            o.text = text_object_value("peer-text");
            o.transform = Transform3x3::IDENTITY;
        }
    }

    // Forget the prior send so we observe only the replay.
    engine.transport_mut().batches.clear();
    engine.reconcile_snapshot(server_scene).unwrap();

    // Server's text survives (no unacked local write on text)...
    assert_eq!(object_text(engine.scene(), "a").as_deref(), Some("peer-text"));
    // ...and the client's unacked move replays on top of the snapshot.
    assert_eq!(object_transform(engine.scene(), "a"), Some(translate(80.0, 80.0)));

    let replayed: Vec<String> = engine.transport().flat().iter().map(|e| op_id_key(&e.op_id)).collect();
    assert_eq!(replayed, vec![op_id_key(&res.op_id.unwrap())]);

    engine.on_ack(&[op_id("c1", 2)], Some(4)).unwrap();
    assert_eq!(engine.outbox_len(), 0);
}

fn text_object_value(value: &str) -> Option<shape_scene_core::object::model::Text> {
    use shape_scene_core::object::model::{Text, TextRun};
    Some(Text {
        runs: vec![TextRun {
            text: value.to_string(),
            color: None,
            size: None,
            bold: false,
            italic: false,
            font: None,
        }],
        align: Default::default(),
        valign: Default::default(),
    })
}
