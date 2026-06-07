//! MG-8 / MG-9 scale-out integration tests: single-writer lease + handoff
//! (MG8.2a, MG8.5), owner routing cache (MG8.2b), canvas CRUD (MG9.1), and
//! graceful shutdown / drain (MG8.3) — object-native (OB4.1).
//!
//! Two registries share one [`InMemoryCoordinator`] and one [`SharedStore`] to
//! model two app instances in front of the same coordinator + storage.

use std::sync::{Arc, Mutex};

use shape_coordination::{Coordinator, InMemoryCoordinator};
use shape_scene_core::object::{FillRule, Geometry, Object, ObjectOp, PathNode, SubPath};
use shape_scene_core::CanvasId;
use shape_server::canvas_actor::SharedStore;
use shape_server::{CanvasRegistry, SpawnError};
use shape_storage_core::RedbAdapter;

fn shared_store() -> SharedStore {
    Arc::new(Mutex::new(RedbAdapter::open_in_memory().unwrap()))
}

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

fn insert(id: &str, order: &str) -> ObjectOp {
    ObjectOp::InsertObject {
        object: Object::new(id, order, rect_geometry()),
    }
}

// ---------------------------------------------------------------------------
// MG8.2a + MG8.5: single-writer lease across two registries + handoff recovery.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn second_registry_cannot_spawn_while_first_holds_lease() {
    let coordinator: Arc<dyn Coordinator> = Arc::new(InMemoryCoordinator::new());
    let store = shared_store();
    let canvas = CanvasId::from("c-handoff");

    let reg_a = CanvasRegistry::with_coordinator_store(
        Arc::clone(&store),
        Arc::clone(&coordinator),
        "owner-a",
    );
    let reg_b = CanvasRegistry::with_coordinator_store(
        Arc::clone(&store),
        Arc::clone(&coordinator),
        "owner-b",
    );

    let handle_a = reg_a.get_or_spawn(&canvas).await.expect("A acquires lease");
    handle_a.apply_op(insert("o1", "a0"), "user-1").await;

    match reg_b.get_or_spawn(&canvas).await {
        Err(SpawnError::NotOwner { owner }) => {
            assert_eq!(owner, "owner-a", "B is told A owns the canvas");
        }
        Ok(_) => panic!("B should be denied while A holds the lease, but it spawned"),
        Err(e) => panic!("B should get NotOwner, got {e:?}"),
    }
    assert!(!reg_b.contains(&canvas), "B never spawned the actor");
}

#[tokio::test]
async fn handoff_after_release_recovers_durable_scene_with_no_data_loss() {
    let coordinator: Arc<dyn Coordinator> = Arc::new(InMemoryCoordinator::new());
    let store = shared_store();
    let canvas = CanvasId::from("c-handoff-recover");

    let reg_a = CanvasRegistry::with_coordinator_store(
        Arc::clone(&store),
        Arc::clone(&coordinator),
        "owner-a",
    );
    let reg_b = CanvasRegistry::with_coordinator_store(
        Arc::clone(&store),
        Arc::clone(&coordinator),
        "owner-b",
    );

    let handle_a = reg_a.get_or_spawn(&canvas).await.expect("A acquires lease");
    handle_a.apply_op(insert("o1", "a0"), "user-1").await;
    handle_a.apply_op(insert("o2", "a1"), "user-1").await;

    reg_a.evict(&canvas).await; // releases the lease.
    assert!(!reg_a.contains(&canvas));

    let handle_b = reg_b
        .get_or_spawn(&canvas)
        .await
        .expect("B acquires the freed lease");
    let scene = handle_b.get_scene().await;
    assert_eq!(scene.objects.len(), 2, "B recovered both objects (no data loss)");
    assert!(scene.get("o1").is_some() && scene.get("o2").is_some(), "both ids present");

    let next = handle_b.apply_op(insert("o3", "a2"), "user-2").await;
    assert!(
        matches!(next, shape_server::ApplyResult::Applied { seq: 3, .. }),
        "seq continues past the 2 recovered ops, got {next:?}"
    );
}

// ---------------------------------------------------------------------------
// MG8.2b: owner routing cache returns the owner without re-hitting coordination.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn routing_cache_returns_owner_without_rehitting_coordination() {
    let coordinator: Arc<dyn Coordinator> = Arc::new(InMemoryCoordinator::new());
    let store = shared_store();
    let canvas = CanvasId::from("c-route");

    let reg = CanvasRegistry::with_coordinator_store(
        Arc::clone(&store),
        Arc::clone(&coordinator),
        "owner-self",
    );

    let first = reg.resolve_owner(&canvas).await;
    assert_eq!(first, "owner-self", "free canvas is claimed by this instance");

    assert!(
        coordinator.find_owner(&canvas.0).await.unwrap().is_none(),
        "resolve_owner released its transient claim, so the coordinator is free"
    );
    let cached = reg.resolve_owner(&canvas).await;
    assert_eq!(cached, "owner-self", "cached owner is returned (0 coordination hops)");
}

// ---------------------------------------------------------------------------
// MG9.1: canvas CRUD create / list / delete round-trip.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn canvas_crud_create_list_delete_round_trip() {
    let registry = CanvasRegistry::open_in_memory().unwrap();

    assert!(registry.list_canvases().is_empty(), "no canvases initially");

    let a = registry.create_canvas("Alpha").unwrap();
    let b = registry.create_canvas("Beta").unwrap();
    assert_ne!(a.id, b.id, "each create gets a distinct id");

    let listed = registry.list_canvases();
    assert_eq!(listed.len(), 2, "both canvases listed");
    assert_eq!(listed[0].title, "Alpha");
    assert_eq!(listed[1].title, "Beta");

    let deleted = registry.delete_canvas(&a.id).await.unwrap();
    assert!(deleted, "delete reports the canvas existed");

    let after = registry.list_canvases();
    assert_eq!(after.len(), 1, "one canvas remains after delete");
    assert_eq!(after[0].id, b.id, "Beta remains");

    let missing = registry.delete_canvas(&CanvasId::from("nope")).await.unwrap();
    assert!(!missing, "deleting an absent canvas returns false");
}

#[tokio::test]
async fn delete_canvas_prunes_scene_records() {
    let store = shared_store();
    let coordinator: Arc<dyn Coordinator> = Arc::new(InMemoryCoordinator::new());
    let registry = CanvasRegistry::with_coordinator_store(
        Arc::clone(&store),
        Arc::clone(&coordinator),
        "owner-self",
    );

    let summary = registry.create_canvas_with_id("c-prune", "Pruned").unwrap();
    let handle = registry.get_or_spawn(&summary.id).await.unwrap();
    handle.apply_op(insert("o1", "a0"), "user-1").await;
    handle.shutdown().await; // write-through left per-object Records.

    {
        use shape_storage_core::StorageAdapter;
        let st = store.lock().unwrap();
        let pre: Vec<String> = st
            .list()
            .unwrap()
            .into_iter()
            .filter(|id| id.starts_with("c-prune:"))
            .collect();
        assert!(!pre.is_empty(), "scene records exist before delete");
    }

    registry.delete_canvas(&summary.id).await.unwrap();

    {
        use shape_storage_core::StorageAdapter;
        let st = store.lock().unwrap();
        let post: Vec<String> = st
            .list()
            .unwrap()
            .into_iter()
            .filter(|id| id.starts_with("c-prune:"))
            .collect();
        assert!(post.is_empty(), "all scene records pruned after delete, found {post:?}");
    }
}

// ---------------------------------------------------------------------------
// MG8.3: graceful shutdown / drain flushes + releases.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graceful_shutdown_flushes_and_frees_leases() {
    let coordinator: Arc<dyn Coordinator> = Arc::new(InMemoryCoordinator::new());
    let store = shared_store();
    let canvas = CanvasId::from("c-drain");

    let reg_a = CanvasRegistry::with_coordinator_store(
        Arc::clone(&store),
        Arc::clone(&coordinator),
        "owner-a",
    );

    let handle = reg_a.get_or_spawn(&canvas).await.expect("A acquires lease");
    handle.apply_op(insert("o1", "a0"), "user-1").await;

    assert_eq!(
        coordinator.find_owner(&canvas.0).await.unwrap().as_deref(),
        Some("owner-a"),
        "lease held while live"
    );

    reg_a.shutdown().await;
    assert!(reg_a.is_empty(), "all actors drained");

    assert!(
        coordinator.find_owner(&canvas.0).await.unwrap().is_none(),
        "lease released by graceful shutdown"
    );

    match reg_a.get_or_spawn(&canvas).await {
        Err(SpawnError::Draining) => {}
        Ok(_) => panic!("a drained registry should reject spawns, but it spawned"),
        Err(e) => panic!("a drained registry should return Draining, got {e:?}"),
    }

    let reg_b = CanvasRegistry::with_coordinator_store(
        Arc::clone(&store),
        Arc::clone(&coordinator),
        "owner-b",
    );
    let handle_b = reg_b.get_or_spawn(&canvas).await.expect("B acquires freed lease");
    let scene = handle_b.get_scene().await;
    assert_eq!(scene.objects.len(), 1, "successor recovered the flushed scene");
}
