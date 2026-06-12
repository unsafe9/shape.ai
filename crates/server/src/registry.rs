//! Canvas registry: the lease-guarded, routable map from `canvasId` to its
//! running actor. One shared [`RedbAdapter`] backs every canvas; idle canvases
//! are evicted (flush + checkpoint) and respawned on demand, reloading durable
//! state.
//!
//! Single-writer lease: before spawning, the registry acquires a
//! [`Lease`](shape_coordination::Lease) from a shared
//! [`Coordinator`](shape_coordination::Coordinator) for this `owner`. While held,
//! no other owner can spawn the same canvas — [`get_or_spawn`] returns
//! [`SpawnError::NotOwner`]. A background task renews the lease;
//! eviction/shutdown releases it for handoff. The default [`InMemoryCoordinator`]
//! makes this a no-op in single-process dev.
//!
//! Routing: [`resolve_owner`] consults an in-memory `canvasId -> owner` cache (0
//! coordination hops on the hot path); on a miss it asks the coordinator and, if
//! free, claims the canvas. This generalizes to N app instances behind a plain
//! TCP load balancer.
//!
//! Graceful shutdown: [`shutdown`] flips a drain flag (rejecting new spawns with
//! [`SpawnError::Draining`]), then flushes + checkpoints every actor and releases
//! every lease so a successor recovers with no data loss.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use shape_coordination::{Coordinator, InMemoryCoordinator, Lease};
use shape_scene_core::{CanvasId, CanvasSummary};
use shape_storage_core::RedbAdapter;
use tokio::sync::broadcast;

use crate::canvas_actor::{ActorHandle, CanvasActor, SharedStore};
use crate::canvas_index;

/// Presence rides a broadcast separate from the actor's op fan-out: lossy by
/// design (a lagging receiver drops the oldest frames) and never persisted.
const PRESENCE_CHANNEL_CAPACITY: usize = 64;

/// One ephemeral presence frame. The `from` author rides along so the WS fan-out
/// can skip echoing a frame back to its own originator.
#[derive(Clone, Debug)]
pub struct PresenceFrame {
    pub from: String,
    pub payload: serde_json::Value,
}

/// Short enough to steal a crashed owner's canvas promptly; long enough that the
/// renew task refreshes well before expiry.
const LEASE_TTL: Duration = Duration::from_secs(30);

/// Well under [`LEASE_TTL`] so a single missed tick never lets the lease lapse.
const LEASE_RENEW_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpawnError {
    /// Leased by a different live owner; `owner` is who the routing layer defers to.
    NotOwner { owner: String },
    /// The registry is draining; no new actors are spawned.
    Draining,
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpawnError::NotOwner { owner } => {
                write!(f, "canvas is owned by '{owner}'")
            }
            SpawnError::Draining => write!(f, "registry is draining"),
        }
    }
}

impl std::error::Error for SpawnError {}

struct Entry {
    handle: ActorHandle,
    lease: Lease,
    /// Dropping this sender stops the renew task (its `changed()` wakes on close).
    renew_stop: tokio::sync::watch::Sender<()>,
    last_activity: Instant,
}

#[derive(Clone)]
pub struct CanvasRegistry {
    inner: Arc<Mutex<HashMap<CanvasId, Entry>>>,
    store: SharedStore,
    /// `InMemoryCoordinator` by default; a `FileCoordinator` or networked impl
    /// swaps in for multi-host.
    coordinator: Arc<dyn Coordinator>,
    /// Unique per process/instance; the lease owner and cached value for owned canvases.
    owner: String,
    /// `canvasId -> owner`: 0 coordination hops after the first lookup.
    route_cache: Arc<Mutex<HashMap<CanvasId, String>>>,
    draining: Arc<AtomicBool>,
    /// Per-canvas presence broadcasters, never tied to actor lifetime so presence
    /// keeps flowing across actor evict/respawn.
    presence: Arc<Mutex<HashMap<CanvasId, broadcast::Sender<PresenceFrame>>>>,
}

impl CanvasRegistry {
    /// Default in-process coordinator and owner id. Single-process dev.
    pub fn new(store: RedbAdapter) -> Self {
        Self::with_coordinator(store, Arc::new(InMemoryCoordinator::new()), default_owner())
    }

    /// Explicit coordinator + owner id over a freshly wrapped store.
    pub fn with_coordinator(
        store: RedbAdapter,
        coordinator: Arc<dyn Coordinator>,
        owner: impl Into<String>,
    ) -> Self {
        Self::with_coordinator_store(Arc::new(Mutex::new(store)), coordinator, owner)
    }

    /// Over an already-shared [`SharedStore`]: handoff tests share one coordinator
    /// AND one backing store (distinct owner ids) so a successor can recover the
    /// predecessor's durable scene after a lease release.
    pub fn with_coordinator_store(
        store: SharedStore,
        coordinator: Arc<dyn Coordinator>,
        owner: impl Into<String>,
    ) -> Self {
        CanvasRegistry {
            inner: Arc::new(Mutex::new(HashMap::new())),
            store,
            coordinator,
            owner: owner.into(),
            route_cache: Arc::new(Mutex::new(HashMap::new())),
            draining: Arc::new(AtomicBool::new(false)),
            presence: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// On-disk redb db at `path` (default coordinator + owner).
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let store = RedbAdapter::open(path)?;
        Ok(Self::new(store))
    }

    /// In-memory redb db (tests).
    pub fn open_in_memory() -> anyhow::Result<Self> {
        let store = RedbAdapter::open_in_memory()?;
        Ok(Self::new(store))
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// Get the handle for `canvas_id`, spawning its lease-guarded actor on first
    /// use. Acquires the single-writer lease first: a live different-owner lease
    /// returns [`SpawnError::NotOwner`]; a draining registry returns
    /// [`SpawnError::Draining`].
    pub async fn get_or_spawn(&self, canvas_id: &CanvasId) -> Result<ActorHandle, SpawnError> {
        // Fast path: already live for this instance.
        {
            let mut map = self.inner.lock().expect("registry mutex poisoned");
            if let Some(entry) = map.get_mut(canvas_id) {
                entry.last_activity = Instant::now();
                return Ok(entry.handle.clone());
            }
        }

        if self.draining.load(Ordering::SeqCst) {
            return Err(SpawnError::Draining);
        }

        let lease = match self
            .coordinator
            .acquire_lease(&canvas_id.0, &self.owner, LEASE_TTL)
            .await
        {
            Ok(lease) => lease,
            Err(_) => {
                let owner = self
                    .coordinator
                    .find_owner(&canvas_id.0)
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                return Err(SpawnError::NotOwner { owner });
            }
        };

        let handle = CanvasActor::spawn(canvas_id.clone(), Arc::clone(&self.store));
        let renew_stop = self.spawn_renew_task(lease.clone());

        // A racing spawn may have inserted while we awaited the lease. Decide
        // winner/loser under the lock, then await the loser's shutdown/release
        // after the guard drops so the future stays `Send`.
        enum Outcome {
            Inserted(ActorHandle),
            RacedOut { winner: ActorHandle },
        }
        let outcome = {
            let mut map = self.inner.lock().expect("registry mutex poisoned");
            if let Some(entry) = map.get_mut(canvas_id) {
                entry.last_activity = Instant::now();
                Outcome::RacedOut {
                    winner: entry.handle.clone(),
                }
            } else {
                map.insert(
                    canvas_id.clone(),
                    Entry {
                        handle: handle.clone(),
                        lease: lease.clone(),
                        renew_stop,
                        last_activity: Instant::now(),
                    },
                );
                Outcome::Inserted(handle.clone())
            }
        };

        let handle = match outcome {
            Outcome::Inserted(handle) => handle,
            Outcome::RacedOut { winner } => {
                // Tear down our redundant actor + lease; the inserter owns this
                // canvas. `renew_stop` is dropped here on the raced path.
                handle.shutdown().await;
                let _ = self.coordinator.release(lease).await;
                winner
            }
        };

        self.route_cache
            .lock()
            .expect("route cache mutex poisoned")
            .insert(canvas_id.clone(), self.owner.clone());

        Ok(handle)
    }

    /// Resolve the owner of `canvas_id`. A cached owner returns with 0
    /// coordination hops; on a miss the coordinator names the live owner, and if
    /// free this instance claims+releases the lease (it only answers "who owns
    /// this"; `get_or_spawn` re-acquires idempotently on actual open).
    pub async fn resolve_owner(&self, canvas_id: &CanvasId) -> String {
        if let Some(owner) = self
            .route_cache
            .lock()
            .expect("route cache mutex poisoned")
            .get(canvas_id)
            .cloned()
        {
            return owner;
        }

        let owner = match self.coordinator.find_owner(&canvas_id.0).await {
            Ok(Some(owner)) => owner,
            _ => match self
                .coordinator
                .acquire_lease(&canvas_id.0, &self.owner, LEASE_TTL)
                .await
            {
                Ok(lease) => {
                    let _ = self.coordinator.release(lease).await;
                    self.owner.clone()
                }
                // Someone claimed between find_owner and acquire: re-read.
                Err(_) => self
                    .coordinator
                    .find_owner(&canvas_id.0)
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| self.owner.clone()),
            },
        };

        self.route_cache
            .lock()
            .expect("route cache mutex poisoned")
            .insert(canvas_id.clone(), owner.clone());
        owner
    }

    /// Renew the lease on [`LEASE_RENEW_INTERVAL`]; exit when the stop sender is
    /// dropped (evict/shutdown) or a renew fails (the lease was stolen).
    fn spawn_renew_task(&self, lease: Lease) -> tokio::sync::watch::Sender<()> {
        let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(());
        let coordinator = Arc::clone(&self.coordinator);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(LEASE_RENEW_INTERVAL);
            ticker.tick().await; // first tick is immediate; skip it.
            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        if coordinator.renew(&lease, LEASE_TTL).await.is_err() {
                            break;
                        }
                    }
                    _ = stop_rx.changed() => break,
                }
            }
        });
        stop_tx
    }

    pub fn create_canvas(&self, title: &str) -> anyhow::Result<CanvasSummary> {
        let id = new_canvas_id();
        self.create_canvas_with_id(&id, title)
    }

    pub fn create_canvas_with_id(&self, id: &str, title: &str) -> anyhow::Result<CanvasSummary> {
        let mut store = self.store.lock().expect("storage mutex poisoned");
        canvas_index::create_canvas(&mut *store, id, title, &now_rfc3339())
    }

    pub fn list_canvases(&self) -> Vec<CanvasSummary> {
        let store = self.store.lock().expect("storage mutex poisoned");
        canvas_index::list_canvases(&*store)
    }

    /// Evict the actor (releasing the lease), remove from the index, and prune
    /// all scene Records. Returns whether the canvas existed in the index.
    pub async fn delete_canvas(&self, canvas_id: &CanvasId) -> anyhow::Result<bool> {
        self.evict(canvas_id).await;
        let removed = {
            let mut store = self.store.lock().expect("storage mutex poisoned");
            let removed = canvas_index::delete_canvas(&mut *store, &canvas_id.0)?;
            prune_canvas_records(&mut store, canvas_id);
            removed
        };
        self.route_cache
            .lock()
            .expect("route cache mutex poisoned")
            .remove(canvas_id);
        Ok(removed)
    }

    /// A receiver on `canvas_id`'s presence channel, creating it on first use.
    pub fn presence_subscribe(
        &self,
        canvas_id: &CanvasId,
    ) -> broadcast::Receiver<PresenceFrame> {
        self.presence_sender(canvas_id).subscribe()
    }

    /// Returns 0 when nobody is listening; the sender never blocks.
    pub fn presence_publish(
        &self,
        canvas_id: &CanvasId,
        from: &str,
        payload: serde_json::Value,
    ) -> usize {
        self.presence_sender(canvas_id)
            .send(PresenceFrame {
                from: from.to_string(),
                payload,
            })
            .unwrap_or(0)
    }

    fn presence_sender(&self, canvas_id: &CanvasId) -> broadcast::Sender<PresenceFrame> {
        let mut map = self.presence.lock().expect("presence mutex poisoned");
        map.entry(canvas_id.clone())
            .or_insert_with(|| broadcast::channel(PRESENCE_CHANNEL_CAPACITY).0)
            .clone()
    }

    pub fn contains(&self, canvas_id: &CanvasId) -> bool {
        self.inner
            .lock()
            .expect("registry mutex poisoned")
            .contains_key(canvas_id)
    }

    pub fn len(&self) -> usize {
        self.inner.lock().expect("registry mutex poisoned").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Remove from the map, stop the renew task, shut the actor down (flush +
    /// checkpoint), and release the lease. No-op if not live.
    pub async fn evict(&self, canvas_id: &CanvasId) {
        let entry = {
            let mut map = self.inner.lock().expect("registry mutex poisoned");
            map.remove(canvas_id)
        };
        if let Some(entry) = entry {
            drop(entry.renew_stop);
            entry.handle.shutdown().await;
            let _ = self.coordinator.release(entry.lease).await;
        }
        self.route_cache
            .lock()
            .expect("route cache mutex poisoned")
            .remove(canvas_id);
    }

    /// Returns the evicted ids.
    pub async fn evict_idle(&self, max_idle: Duration) -> Vec<CanvasId> {
        let stale: Vec<(CanvasId, Entry)> = {
            let mut map = self.inner.lock().expect("registry mutex poisoned");
            let now = Instant::now();
            let victims: Vec<CanvasId> = map
                .iter()
                .filter(|(_, e)| now.duration_since(e.last_activity) >= max_idle)
                .map(|(id, _)| id.clone())
                .collect();
            victims
                .into_iter()
                .filter_map(|id| map.remove(&id).map(|e| (id, e)))
                .collect()
        };
        let mut evicted = Vec::with_capacity(stale.len());
        for (id, entry) in stale {
            drop(entry.renew_stop);
            entry.handle.shutdown().await;
            let _ = self.coordinator.release(entry.lease).await;
            self.route_cache
                .lock()
                .expect("route cache mutex poisoned")
                .remove(&id);
            evicted.push(id);
        }
        evicted
    }

    /// Reject new spawns, then flush + checkpoint every actor and release every
    /// lease, so a successor can recover with no data loss.
    pub async fn shutdown(&self) {
        self.draining.store(true, Ordering::SeqCst);
        let entries: Vec<(CanvasId, Entry)> = {
            let mut map = self.inner.lock().expect("registry mutex poisoned");
            map.drain().collect()
        };
        for (id, entry) in entries {
            drop(entry.renew_stop);
            entry.handle.shutdown().await;
            let _ = self.coordinator.release(entry.lease).await;
            self.route_cache
                .lock()
                .expect("route cache mutex poisoned")
                .remove(&id);
        }
    }
}

/// Delete every Record under the `"{canvasId}:"` prefix so a deleted canvas
/// leaves no scene state behind.
fn prune_canvas_records(store: &mut RedbAdapter, canvas_id: &CanvasId) {
    use shape_storage_core::StorageAdapter;
    let prefix = format!("{canvas_id}:");
    let ids: Vec<String> = store
        .list()
        .unwrap_or_default()
        .into_iter()
        .filter(|id| id.starts_with(&prefix))
        .collect();
    for id in ids {
        let _ = store.delete(&id);
    }
}

/// `owner-<pid>-<counter>`: pid + per-process monotonic suffix so two in-process
/// registries (handoff tests) differ. No rand.
fn default_owner() -> String {
    use std::sync::atomic::AtomicU64;
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("owner-{}-{n}", std::process::id())
}

/// `"canvas-<pid>-<counter>"`, deterministic within a process (no rand).
fn new_canvas_id() -> String {
    use std::sync::atomic::AtomicU64;
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("canvas-{}-{n}", std::process::id())
}

/// Wall-clock timestamp for canvas metadata. Ambient time is fine here: this is
/// platform-layer metadata and never touches a Scene (scene-core stays time-free).
fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Coarse epoch-seconds is enough for list ordering; the format is not a wire contract.
    format!("{secs}")
}
