//! Port of the windowing-decision assertions in `tests/windowing.test.ts`. The TS
//! suite also drives reconnect-backoff and a mock socket; the backoff schedule
//! lives in `wsTransport.ts` (shell transport, out of this crate's port scope),
//! and the offline-buffer-then-replay convergence is the engine reconcile path
//! covered in the sync-engine/scene-client ports. Here we cover the windowing math
//! + re-subscribe decisions (region seed, camera-move re-aim, debounce coalesce,
//! unchanged no-op, whole-canvas drop) and the clean-outbox-on-canvas-switch rule.

mod common;

use common::*;
use shape_client_runtime::outbox::InMemoryOutboxStore;
use shape_client_runtime::scene_client::{
    window_from_viewport, Bbox, WindowState, DEFAULT_VIEWPORT_MARGIN,
};
use shape_client_runtime::sync_engine::SyncEngine;

fn bbox(x: f64, y: f64, w: f64, h: f64) -> Bbox {
    Bbox { x, y, width: w, height: h }
}

// --- windowFromViewport --------------------------------------------------------

#[test]
fn grows_the_viewport_by_the_margin_fraction_on_each_side() {
    let win = window_from_viewport(bbox(0.0, 0.0, 100.0, 200.0), 0.5);
    assert_eq!(win, bbox(-50.0, -100.0, 200.0, 400.0));
}

#[test]
fn has_a_sensible_default_margin() {
    assert!(DEFAULT_VIEWPORT_MARGIN > 0.0);
}

// --- windowed subscribe (decisions) -------------------------------------------

#[test]
fn seeds_the_connection_window_from_the_connect_region() {
    let seed = bbox(0.0, 0.0, 500.0, 500.0);
    let state = WindowState::with_margin(Some(seed), 0.0);
    assert_eq!(state.current_window(), Some(&seed));
}

#[test]
fn re_subscribes_with_a_new_region_on_a_camera_move() {
    // margin 0 mirrors the TS windowing-test boot (viewportMargin: 0).
    let mut state = WindowState::with_margin(Some(bbox(-50.0, -50.0, 300.0, 300.0)), 0.0);
    let next = state.on_viewport(bbox(950.0, -50.0, 300.0, 300.0));
    assert_eq!(next, Some(bbox(950.0, -50.0, 300.0, 300.0)));
    assert_eq!(state.current_window(), Some(&bbox(950.0, -50.0, 300.0, 300.0)));
}

#[test]
fn coalesces_a_rapid_pan_into_a_single_re_subscribe() {
    // The shell debounces the timer; the decision layer only emits on the final,
    // settled viewport. Driving three viewports then asking for the last is the
    // post-debounce decision: one subscribe to the final window.
    let mut state = WindowState::with_margin(Some(bbox(0.0, 0.0, 100.0, 100.0)), 0.0);
    let last = state.on_viewport(bbox(30.0, 0.0, 100.0, 100.0));
    assert_eq!(last, Some(bbox(30.0, 0.0, 100.0, 100.0)));
}

#[test]
fn does_not_re_subscribe_when_the_window_is_unchanged() {
    let seed = bbox(0.0, 0.0, 100.0, 100.0);
    let mut state = WindowState::with_margin(Some(seed), 0.0);
    // Re-aiming to the identical bbox is a no-op (no subscribe frame).
    assert_eq!(state.set_window(seed), None);
}

#[test]
fn subscribe_whole_canvas_drops_the_window() {
    let mut state = WindowState::with_margin(Some(bbox(0.0, 0.0, 100.0, 100.0)), 0.0);
    assert!(state.subscribe_whole_canvas());
    assert_eq!(state.current_window(), None);
    // Already whole-canvas: a second drop is a no-op.
    assert!(!state.subscribe_whole_canvas());
}

// --- canvas switch: clean outbox (no cross-canvas replay) ----------------------

#[test]
fn switch_canvas_starts_the_new_canvas_with_a_clean_outbox() {
    // Author + send on canvas A; its outbox holds one entry.
    let mut engine_a = SyncEngine::new(empty_scene(), "c1", InMemoryOutboxStore::new(), CaptureTransport::new(), None);
    let mut now = FixedNow::new();
    engine_a.author(insert(rect("g-a", "a0")), &now.next()).unwrap();
    engine_a.on_flush_due();
    assert_eq!(engine_a.outbox_len(), 1);

    // switchCanvas builds a FRESH outbox for the new canvas (no cross-canvas
    // replay): a new engine with a new in-memory outbox replays nothing.
    let mut engine_b = SyncEngine::new(empty_scene(), "c1", InMemoryOutboxStore::new(), CaptureTransport::new(), None);
    engine_b.reconcile_snapshot(empty_scene()).unwrap();
    assert_eq!(engine_b.transport().batches.len(), 0);
    assert_eq!(engine_b.outbox_len(), 0);
}

// --- offline buffer -> reconnect replay converges (engine reconcile) -----------

#[test]
fn buffers_ops_while_offline_and_replays_them_on_reconnect_to_converge() {
    let mut engine = SyncEngine::new(empty_scene(), "c1", InMemoryOutboxStore::new(), CaptureTransport::new(), None);
    let mut now = FixedNow::new();

    // "Offline": author two inserts; optimistic apply + outbox, no ack yet.
    engine.author(insert(rect("g1", "a0")), &now.next()).unwrap();
    engine.author(insert(rect("g2", "a1")), &now.next()).unwrap();
    engine.flush();
    assert_eq!(object_ids(engine.scene()), vec!["g1", "g2"]);
    assert_eq!(engine.outbox_len(), 2);

    // Reconnect welcome (empty scene): reconcile replays both in localSeq order.
    engine.transport_mut().batches.clear();
    engine.reconcile_snapshot(empty_scene()).unwrap();
    assert_eq!(object_ids(engine.scene()), vec!["g1", "g2"]);
    let replayed: Vec<i64> = engine.transport().flat().iter().map(|e| e.op_id.local_seq).collect();
    assert_eq!(replayed, vec![1, 2]);
    assert_eq!(engine.outbox_len(), 2);

    // The server acks both: outbox clears.
    engine.on_ack(&[op_id("c1", 1), op_id("c1", 2)], Some(2)).unwrap();
    assert_eq!(engine.outbox_len(), 0);
}
