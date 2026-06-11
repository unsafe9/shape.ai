//! Port of `tests/multiuser.test.ts`: the client side of the realtime tail on the
//! object model. The TS suite drives `SceneClient` over a mock socket; the
//! behavioral assertions are engine decisions (`apply_remote` discard path) plus
//! the peer registry (presence latest-wins, self-skip, TTL expiry), covered here
//! directly since the socket itself is shell-side.

mod common;

use common::*;
use serde_json::json;
use shape_client_runtime::outbox::InMemoryOutboxStore;
use shape_client_runtime::peers::PeerRegistry;
use shape_client_runtime::sync_engine::SyncEngine;

fn boot_engine(
    initial: shape_scene_core::object::model::ObjectScene,
) -> (SyncEngine<CaptureTransport, InMemoryOutboxStore>, FixedNow) {
    let engine = SyncEngine::new(initial, "c1", InMemoryOutboxStore::new(), CaptureTransport::new(), None);
    (engine, FixedNow::new())
}

/// A scene with object "a" already present so move/text ops validate.
fn seeded_scene(version: i64) -> shape_scene_core::object::model::ObjectScene {
    let mut scene = empty_scene();
    scene.scene_version = version;
    scene.objects = vec![rect("a", "a0")];
    scene
}

// --- peer patch apply ----------------------------------------------------------

#[test]
fn applies_a_peer_patch_to_the_local_scene() {
    let (mut engine, _now) = boot_engine(empty_scene());
    assert!(engine.apply_remote(insert(rect("a", "a0"))));
    assert_eq!(object_ids(engine.scene()), vec!["a"]);
}

#[test]
fn applies_a_peers_create_then_move_in_arrival_order() {
    let (mut engine, _now) = boot_engine(empty_scene());
    assert!(engine.apply_remote(insert(rect("a", "a0"))));
    assert!(engine.apply_remote(move_op("a", 99.0, 88.0)));
    assert_eq!(object_transform(engine.scene(), "a"), Some(translate(99.0, 88.0)));
}

// --- mid-drag transient ownership ---------------------------------------------

#[test]
fn ignores_peer_write_to_field_client_is_mid_drag_on_then_applies_after_ack() {
    let (mut engine, mut now) = boot_engine(seeded_scene(2));

    let res = engine.author(move_op("a", 200.0, 200.0), &now.next()).unwrap();
    assert_eq!(object_transform(engine.scene(), "a"), Some(translate(200.0, 200.0)));
    engine.on_flush_due();

    // A peer move to the owned (a, transform) is IGNORED while unacked.
    assert!(!engine.apply_remote(move_op("a", 7.0, 7.0)));
    assert_eq!(object_transform(engine.scene(), "a"), Some(translate(200.0, 200.0)));

    engine.on_ack(&[res.op_id.unwrap()], Some(4)).unwrap();

    // After ack, a later peer write applies.
    assert!(engine.apply_remote(move_op("a", 7.0, 7.0)));
    assert_eq!(object_transform(engine.scene(), "a"), Some(translate(7.0, 7.0)));
}

#[test]
fn applies_peer_write_to_a_different_field_while_a_drag_is_in_flight() {
    let (mut engine, mut now) = boot_engine(seeded_scene(2));
    engine.author(move_op("a", 200.0, 200.0), &now.next()).unwrap();
    engine.on_flush_due();

    assert!(engine.apply_remote(text_op("a", "from-peer")));
    assert_eq!(object_text(engine.scene(), "a").as_deref(), Some("from-peer"));
    assert_eq!(object_transform(engine.scene(), "a"), Some(translate(200.0, 200.0)));
}

// --- peer presence cursors -----------------------------------------------------

#[test]
fn renders_a_peer_cursor_from_an_inbound_presence_frame() {
    let mut peers = PeerRegistry::new(Some("user-1".to_string()), None);
    let now = 1_000;
    assert!(peers.ingest(&json!({ "userId": "peer-9", "cursor": { "x": 12.0, "y": 34.0 } }), now));

    let live = peers.list();
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].user_id, "peer-9");
    assert_eq!(live[0].cursor, Some(shape_scene_core::model::Point { x: 12.0, y: 34.0 }));
    assert!(!live[0].color.is_empty());
}

#[test]
fn keeps_the_latest_cursor_per_peer_latest_wins() {
    let mut peers = PeerRegistry::new(Some("user-1".to_string()), None);
    peers.ingest(&json!({ "userId": "peer-9", "cursor": { "x": 1.0, "y": 1.0 } }), 1_000);
    peers.ingest(&json!({ "userId": "peer-9", "cursor": { "x": 50.0, "y": 60.0 } }), 1_001);
    let live = peers.list();
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].cursor, Some(shape_scene_core::model::Point { x: 50.0, y: 60.0 }));
}

#[test]
fn does_not_surface_the_clients_own_presence_as_a_peer() {
    let mut peers = PeerRegistry::new(Some("user-1".to_string()), None);
    assert!(!peers.ingest(&json!({ "userId": "user-1", "cursor": { "x": 5.0, "y": 5.0 } }), 1_000));
    assert!(peers.ingest(&json!({ "userId": "peer-2", "cursor": { "x": 9.0, "y": 9.0 } }), 1_000));
    assert_eq!(peers.list().iter().map(|p| p.user_id.clone()).collect::<Vec<_>>(), vec!["peer-2"]);
}

#[test]
fn expires_a_peer_cursor_after_the_ttl_window() {
    let mut peers = PeerRegistry::new(None, None);
    peers.ingest(&json!({ "userId": "peer-9", "cursor": { "x": 1.0, "y": 1.0 } }), 1_000);
    assert_eq!(peers.list().len(), 1);
    // Lazily expire at +11s (TTL is 10s).
    peers.expire(12_000);
    assert_eq!(peers.list().len(), 0);
}

#[test]
fn expires_a_stale_peer_when_a_fresh_peer_frame_arrives() {
    let mut peers = PeerRegistry::new(None, None);
    peers.ingest(&json!({ "userId": "peer-9", "cursor": { "x": 1.0, "y": 1.0 } }), 1_000);
    // A fresh frame at +11s, then expire vs the same clock: the stale peer drops.
    peers.ingest(&json!({ "userId": "peer-2", "cursor": { "x": 2.0, "y": 2.0 } }), 12_000);
    peers.expire(12_000);
    assert_eq!(peers.list().iter().map(|p| p.user_id.clone()).collect::<Vec<_>>(), vec!["peer-2"]);
}

// --- concurrent-edit convergence ----------------------------------------------

#[test]
fn converges_to_server_arrival_order_for_concurrent_edits_on_a_shared_field() {
    let (mut engine, _now) = boot_engine(seeded_scene(2));
    assert!(engine.apply_remote(move_op("a", 100.0, 100.0)));
    assert!(engine.apply_remote(move_op("a", 300.0, 400.0)));
    assert_eq!(object_transform(engine.scene(), "a"), Some(translate(300.0, 400.0)));
}

#[test]
fn a_self_acked_op_plus_a_later_peer_op_converge_to_the_peers_value() {
    let (mut engine, mut now) = boot_engine(seeded_scene(2));
    let res = engine.author(move_op("a", 50.0, 50.0), &now.next()).unwrap();
    engine.on_flush_due();
    engine.on_ack(&[res.op_id.unwrap()], Some(3)).unwrap();
    assert!(engine.apply_remote(move_op("a", 900.0, 900.0)));
    assert_eq!(object_transform(engine.scene(), "a"), Some(translate(900.0, 900.0)));
}
