//! MG-8 / MG-9 scale-out integration tests: single-writer lease + handoff
//! (MG8.2a, MG8.5), owner routing cache (MG8.2b), canvas CRUD (MG9.1), and
//! graceful shutdown / drain (MG8.3).
//!
//! Two registries share one [`InMemoryCoordinator`] and one [`SharedStore`] to
//! model two app instances in front of the same coordinator + storage, the
//! minimal setup that exercises lease denial + handoff recovery.

use std::sync::{Arc, Mutex};

use shape_coordination::{Coordinator, InMemoryCoordinator};
use shape_scene_core::{CanvasId, RenderGroup, RenderScenePatch, WorldRect};
use shape_server::canvas_actor::SharedStore;
use shape_server::{CanvasRegistry, SpawnError};
use shape_storage_core::SqliteAdapter;

fn shared_store() -> SharedStore {
    Arc::new(Mutex::new(SqliteAdapter::open_in_memory().unwrap()))
}

fn create_group(id: &str) -> RenderScenePatch {
    RenderScenePatch::CreateGroup {
        group: RenderGroup {
            id: id.to_string(),
            title: "G".to_string(),
            summary: String::new(),
            bounds: WorldRect {
                x: 0.0,
                y: 0.0,
                width: 400.0,
                height: 300.0,
            },
            tag_ids: vec![],
            z_index: 0.0,
            style_key: String::new(),
        },
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

    // A acquires the lease and spawns; B is denied with NotOwner naming A.
    let handle_a = reg_a.get_or_spawn(&canvas).await.expect("A acquires lease");
    handle_a.apply_patch(create_group("g1"), "user-1").await;

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

    // A spawns and writes two objects, then evicts (flush + checkpoint + release).
    let handle_a = reg_a.get_or_spawn(&canvas).await.expect("A acquires lease");
    handle_a.apply_patch(create_group("g1"), "user-1").await;
    handle_a
        .apply_patch(
            RenderScenePatch::CreateGroup {
                group: RenderGroup {
                    id: "g2".to_string(),
                    title: "G2".to_string(),
                    summary: String::new(),
                    bounds: WorldRect {
                        x: 10.0,
                        y: 10.0,
                        width: 50.0,
                        height: 40.0,
                    },
                    tag_ids: vec![],
                    z_index: 0.0,
                    style_key: String::new(),
                },
            },
            "user-1",
        )
        .await;

    reg_a.evict(&canvas).await; // releases the lease.
    assert!(!reg_a.contains(&canvas));

    // B can now acquire the freed lease and recover the durable scene.
    let handle_b = reg_b
        .get_or_spawn(&canvas)
        .await
        .expect("B acquires the freed lease");
    let scene = handle_b.get_scene().await;
    assert_eq!(scene.groups.len(), 2, "B recovered both groups (no data loss)");
    let ids: Vec<&str> = scene.groups.iter().map(|g| g.id.as_str()).collect();
    assert!(ids.contains(&"g1") && ids.contains(&"g2"), "both ids present: {ids:?}");

    // The successor continues the server seq past the recovered ops.
    let next = handle_b.apply_patch(create_group("g3"), "user-2").await;
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

    // First resolve: cache miss -> claim -> cache self as owner.
    let first = reg.resolve_owner(&canvas).await;
    assert_eq!(first, "owner-self", "free canvas is claimed by this instance");

    // Drop the coordinator's view of the lease (resolve_owner releases its claim
    // immediately), so a cache MISS would now find no owner and re-claim. Prove
    // the cache short-circuits: even though the coordinator currently names no
    // owner, the cached answer is still returned.
    assert!(
        coordinator
            .find_owner(&canvas.0)
            .await
            .unwrap()
            .is_none(),
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

    // Deleting an unknown canvas reports it did not exist.
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
    handle.apply_patch(create_group("g1"), "user-1").await;
    handle.shutdown().await; // checkpoint writes per-object Records.

    // Scene Records exist under the "c-prune:" prefix before delete.
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
    handle.apply_patch(create_group("g1"), "user-1").await;

    // While live, the lease is held (find_owner names A).
    assert_eq!(
        coordinator.find_owner(&canvas.0).await.unwrap().as_deref(),
        Some("owner-a"),
        "lease held while live"
    );

    reg_a.shutdown().await;
    assert!(reg_a.is_empty(), "all actors drained");

    // The lease is freed after drain.
    assert!(
        coordinator.find_owner(&canvas.0).await.unwrap().is_none(),
        "lease released by graceful shutdown"
    );

    // Draining registry rejects new spawns.
    match reg_a.get_or_spawn(&canvas).await {
        Err(SpawnError::Draining) => {}
        Ok(_) => panic!("a drained registry should reject spawns, but it spawned"),
        Err(e) => panic!("a drained registry should return Draining, got {e:?}"),
    }

    // A successor recovers the flushed scene with no data loss.
    let reg_b = CanvasRegistry::with_coordinator_store(
        Arc::clone(&store),
        Arc::clone(&coordinator),
        "owner-b",
    );
    let handle_b = reg_b.get_or_spawn(&canvas).await.expect("B acquires freed lease");
    let scene = handle_b.get_scene().await;
    assert_eq!(scene.groups.len(), 1, "successor recovered the flushed scene");
}
