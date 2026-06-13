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

#[test]
fn offline_author_reflects_in_scene_and_undo_uses_core_inverse() {
    // C1: with NO transport flush driven (the offline / pre-connect path), the
    // engine — which IS the one Rust core both server and client run — still owns
    // the scene and the undo inverse. The shell never re-implements either offline.
    let (mut engine, mut now) = boot();

    engine.author(insert(rect("a", "a0")), &now.next()).unwrap();
    // The authored move lands in the core's optimistic scene with no round-trip.
    let res = engine.author(move_op("a", 25.0, 25.0), &now.next()).unwrap();
    assert_eq!(object_transform(engine.scene(), "a"), Some(translate(25.0, 25.0)));

    // The inverse the core captured restores the pre-move transform — applying it
    // (the undo) through the SAME core op-apply must revert the scene. If the shell
    // had owned undo with its own apply, this inverse would be absent/wrong.
    let inverse = res.inverse.expect("move captures an inverse");
    engine.author(inverse, &now.next()).unwrap();
    assert_eq!(object_transform(engine.scene(), "a"), Some(translate(0.0, 0.0)));
}

#[test]
fn coincidental_peer_transform_does_not_settle_until_ack_lands() {
    // C2: a coincidentally-equal peer transform arriving BEFORE the real ack must
    // NOT report the (object,transform) preview as settled — only the ack (which
    // releases transient ownership) settles it. This is the exact case a shell-side
    // transform-value compare gets wrong: the values match, so it would clear the
    // preview early and snap the object back when the still-pending local op's
    // optimistic state is reconciled.
    let (mut engine, mut now) = boot();
    engine.author(insert(rect("a", "a0")), &now.next()).unwrap();

    let res = engine.author(move_op("a", 50.0, 50.0), &now.next()).unwrap();
    assert_eq!(engine.owned_key_set(), vec!["a:transform".to_string()]);

    // A peer's remote move to the SAME position arrives before our ack. It is
    // dropped by transient ownership (we still own the key) and settles nothing.
    assert!(!engine.apply_remote(move_op("a", 50.0, 50.0)));
    assert!(
        engine.take_settled_keys().is_empty(),
        "a coincidentally-equal peer write must not settle the preview before the ack"
    );

    // The real ack releases ownership and reports the key as settled exactly once.
    engine.on_ack(&[res.op_id.unwrap()], Some(2)).unwrap();
    assert_eq!(engine.take_settled_keys(), vec!["a:transform".to_string()]);
    // Drained: a second read reports nothing.
    assert!(engine.take_settled_keys().is_empty());
}

#[test]
fn reject_settles_the_owned_key() {
    // A rejected op also releases ownership, so its key settles too (the shell must
    // clear the rejected preview just as it clears an acked one).
    let (mut engine, mut now) = boot();
    engine.author(insert(rect("a", "a0")), &now.next()).unwrap();
    let res = engine.author(move_op("a", 9.0, 9.0), &now.next()).unwrap();

    engine.on_rejected(&[res.op_id.unwrap()]).unwrap();
    assert_eq!(engine.take_settled_keys(), vec!["a:transform".to_string()]);
}

#[test]
fn identical_text_commit_is_a_noop_and_authors_nothing() {
    // A re-committed unchanged text edit is a whole-op no-op (op-apply returns an
    // empty-Batch inverse). On the connected path it must author NOTHING — no
    // `${id}:text` ownership, no outbox row, no wire envelope — so it can't gate
    // or clobber a concurrent peer text edit during the round-trip. This is the
    // core-side replacement for the deleted TS shell dedup; if `author` stops
    // suppressing the empty-Batch case, every assertion below breaks.
    let (mut engine, mut now) = boot();
    engine.author(insert(rect("a", "a0")), &now.next()).unwrap();

    // A REAL text change: ownership taken, outbox grows, an envelope is enqueued.
    let res = engine.author(text_op("a", "hello"), &now.next()).unwrap();
    assert!(res.op_id.is_some(), "a real text change authors an op");
    assert_eq!(object_text(engine.scene(), "a").as_deref(), Some("hello"));
    assert_eq!(engine.owned_key_set(), vec!["a:text".to_string()]);
    let outbox_after_real = engine.outbox_len();
    assert_eq!(outbox_after_real, 2, "insert + the real text op are in the outbox");
    engine.flush();
    let envelopes_after_real = engine.transport().flat().len();
    assert_eq!(envelopes_after_real, 2, "insert + the real text op went on the wire");

    // The IDENTICAL text commit: op-apply changes nothing, so author suppresses
    // it. No new ownership key, no new outbox row, no new wire envelope, and the
    // local scene is byte-for-byte unchanged.
    let scene_before_noop = engine.scene().clone();
    let noop = engine.author(text_op("a", "hello"), &now.next()).unwrap();

    assert!(noop.op_id.is_none(), "a no-op text commit mints no op_id");
    assert_eq!(engine.scene(), &scene_before_noop, "the local scene is unchanged");
    assert_eq!(
        engine.owned_key_set(),
        vec!["a:text".to_string()],
        "no NEW ownership key — only the real op's `a:text` remains"
    );
    assert_eq!(
        engine.outbox_len(),
        outbox_after_real,
        "no new outbox entry was appended for the no-op"
    );
    engine.flush();
    assert_eq!(
        engine.transport().flat().len(),
        envelopes_after_real,
        "no new transport envelope was enqueued for the no-op"
    );
}

#[test]
fn deleting_an_old_object_then_reconciling_a_stale_windowed_welcome_must_not_resurrect_it() {
    // Repro of the reported bug: an OLD object (created in a prior session, so it
    // exists in the server's welcome snapshot but is NOT represented by any client
    // outbox entry) is deleted; the delete is authored, sent, and ACKED (dropped
    // from the outbox). Then a viewport change triggers a windowed `subscribe`,
    // and the server answers with a welcome whose snapshot was captured BEFORE the
    // delete landed (the subscribe and the delete-ack race on the wire / coalescing
    // delays the delete past the immediately-fired subscribe). reconcile_snapshot
    // must NOT bring the deleted object back.
    //
    // Contrast with a NEW object (next test): its insert+delete are both
    // client-authored, so the snapshot never carries it — which is exactly why
    // new objects delete fine while old ones come back.
    let outbox = InMemoryOutboxStore::new();
    let mut now = FixedNow::new();

    // Boot from a welcome that already contains the old object (prior session).
    let mut welcome = empty_scene();
    welcome.objects.push(rect("old", "a0"));
    welcome.scene_version = 7;
    let mut engine = SyncEngine::new(welcome.clone(), "c1", outbox, CaptureTransport::new(), None);
    assert_eq!(object_ids(engine.scene()), vec!["old"]);

    // Delete the old object: optimistic local remove + outbox row.
    let del = engine
        .author(ObjectOp::Delete { id: "old".into() }, &now.next())
        .unwrap();
    assert_eq!(object_ids(engine.scene()), Vec::<String>::new(), "deleted locally");
    assert_eq!(engine.outbox_len(), 1);

    // The server applies the delete and ACKs it; the client drops the outbox row.
    engine.on_ack(&[del.op_id.clone().unwrap()], Some(8)).unwrap();
    assert_eq!(engine.outbox_len(), 0, "delete acked, outbox empty");

    // A viewport change triggers a windowed `subscribe`. The welcome the server
    // returns was captured at the pre-delete revision (it still carries "old").
    // The outbox no longer holds the delete to replay on top.
    engine.reconcile_snapshot(welcome).unwrap();

    assert_eq!(
        object_ids(engine.scene()),
        Vec::<String>::new(),
        "a deleted-and-acked old object must NOT reappear after reconciling a stale \
         windowed welcome; it resurrected"
    );
}

#[test]
fn deleting_a_new_object_then_reconciling_a_stale_welcome_stays_deleted() {
    // Contrast: a NEW object (inserted THIS session) deleted and both ops acked.
    // The server's snapshot never carries it (the server applied both insert and
    // delete), so reconcile cannot resurrect it — which is why new objects stay
    // deleted while old ones (above) come back. This passes today; it pins the
    // asymmetry so a fix to the old-object case must not regress it.
    let outbox = InMemoryOutboxStore::new();
    let mut now = FixedNow::new();
    let mut engine = SyncEngine::new(empty_scene(), "c1", outbox, CaptureTransport::new(), None);

    engine.author(insert(rect("new", "a0")), &now.next()).unwrap();
    engine.on_ack(&[op_id("c1", 1)], Some(1)).unwrap();
    let del = engine
        .author(ObjectOp::Delete { id: "new".into() }, &now.next())
        .unwrap();
    engine.on_ack(&[del.op_id.clone().unwrap()], Some(2)).unwrap();
    assert_eq!(engine.outbox_len(), 0);

    // The server's welcome reflects the post-delete scene: "new" is absent.
    let mut welcome = empty_scene();
    welcome.scene_version = 2;
    engine.reconcile_snapshot(welcome).unwrap();

    assert_eq!(
        object_ids(engine.scene()),
        Vec::<String>::new(),
        "a new object's delete sticks (the snapshot never carried it)"
    );
}

#[test]
fn a_stale_welcome_is_ignored_but_a_subsequent_current_welcome_is_still_adopted() {
    // The staleness guard must skip ONLY the regressing welcome, not wedge reconcile:
    // after a stale welcome is ignored (keeping an acked delete), the server's NEXT
    // current welcome (scene_version >= base) must be adopted normally, loading its
    // windowed object set. This pins that the guard is strictly `<` (a same/greater
    // revision still adopts) so windowed pans and reconnects keep working.
    let outbox = InMemoryOutboxStore::new();
    let mut now = FixedNow::new();

    let mut welcome = empty_scene();
    welcome.objects.push(rect("old", "a0"));
    welcome.scene_version = 7;
    let mut engine = SyncEngine::new(welcome.clone(), "c1", outbox, CaptureTransport::new(), None);

    let del = engine
        .author(ObjectOp::Delete { id: "old".into() }, &now.next())
        .unwrap();
    engine.on_ack(&[del.op_id.clone().unwrap()], Some(8)).unwrap();

    // Stale windowed welcome (rev 7 < base 8): ignored, the delete stands.
    engine.reconcile_snapshot(welcome).unwrap();
    assert_eq!(object_ids(engine.scene()), Vec::<String>::new(), "stale welcome ignored");

    // A current welcome (rev 8) the pan reveals: "old" is gone server-side and a
    // sibling "other" is now in view. It must be adopted: "other" loads, "old" stays gone.
    let mut current = empty_scene();
    current.objects.push(rect("other", "a1"));
    current.scene_version = 8;
    engine.reconcile_snapshot(current).unwrap();
    assert_eq!(
        object_ids(engine.scene()),
        vec!["other"],
        "a current welcome (>= base) is still adopted after a stale one was skipped"
    );
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
