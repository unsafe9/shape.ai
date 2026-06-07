//! Canvas registry (MG2.4 + MG8 scale-out): the map from `canvasId` to its
//! running actor, now lease-guarded and routable.
//!
//! One shared [`SqliteAdapter`] backs every canvas; the registry hands each
//! actor a clone of that shared store and tracks per-canvas last-activity so an
//! idle canvas can be evicted (which flushes + checkpoints it). A re-`get` after
//! eviction spawns a fresh actor that reloads the canvas's durable state.
//!
//! ## Single-writer lease (MG8.2a)
//!
//! Before spawning an actor for a canvas, the registry acquires a single-writer
//! [`Lease`](shape_coordination::Lease) from a shared
//! [`Coordinator`](shape_coordination::Coordinator) on behalf of this registry's
//! `owner`. While held, no other owner sharing the same coordinator can spawn the
//! same canvas — [`get_or_spawn`](CanvasRegistry::get_or_spawn) returns
//! [`SpawnError::NotOwner`] for them. A per-canvas background task renews the
//! lease on an interval; eviction/shutdown releases it so another owner can take
//! over and recover the durable scene (handoff, MG8.5). The default
//! [`InMemoryCoordinator`] makes this a no-op cost in single-process dev.
//!
//! ## Routing (MG8.2b)
//!
//! [`resolve_owner`](CanvasRegistry::resolve_owner) is the per-connection owner
//! lookup: it consults an in-memory `canvasId -> owner` cache first (0
//! coordination hops on the hot path); on a miss it asks the coordinator for the
//! current owner and, if the canvas is free, claims it (becoming the owner) and
//! caches the result. For single-process dev the owner is always `self`. The
//! intended deployment is a plain TCP load balancer in front of N app instances:
//! the LB spreads connections arbitrarily, and each instance uses this lookup to
//! decide whether it owns a canvas or must defer to the owner the coordinator
//! names. The lookup+cache+claim structure is what generalizes to multi-process.
//!
//! ## Graceful shutdown (MG8.3)
//!
//! [`shutdown`](CanvasRegistry::shutdown) flips a drain flag (rejecting new
//! spawns with [`SpawnError::Draining`]), then flushes + checkpoints every live
//! actor and releases every lease so a successor can recover with no data loss.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use shape_coordination::{Coordinator, InMemoryCoordinator, Lease};
use shape_scene_core::{CanvasId, CanvasSummary};
use shape_storage_core::SqliteAdapter;
use tokio::sync::broadcast;

use crate::canvas_actor::{ActorHandle, CanvasActor, SharedStore};
use crate::canvas_index;

/// Per-canvas presence fan-out (MG-3 ephemeral/best-effort channel). Presence
/// rides a separate broadcast from the actor's ordered op fan-out: it is lossy by
/// design (a full lagging receiver drops the oldest frames) and never persisted,
/// matching the `ephemeral_besteffort` logical channel.
const PRESENCE_CHANNEL_CAPACITY: usize = 64;

/// One presence frame on the ephemeral channel (MG-6.2). Carries the sender's
/// attributed author (`from`) alongside the opaque `payload`, so the WS fan-out
/// can skip echoing a frame back to its own originator (a connection should not
/// render its own cursor). Latest-wins per user is the client's concern; the
/// server only fans out best-effort and never persists.
#[derive(Clone, Debug)]
pub struct PresenceFrame {
    pub from: String,
    pub payload: serde_json::Value,
}

/// How long a freshly-acquired lease is valid before it must be renewed. Short
/// enough that a crashed owner's canvas can be stolen promptly; long enough that
/// the renew task comfortably refreshes it well before expiry.
const LEASE_TTL: Duration = Duration::from_secs(30);

/// How often the per-canvas renew task extends the lease. Must be well under
/// [`LEASE_TTL`] so a single missed tick never lets the lease lapse.
const LEASE_RENEW_INTERVAL: Duration = Duration::from_secs(10);

/// Why a spawn was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpawnError {
    /// The canvas is leased by a different, still-live owner. The named owner is
    /// the one the routing layer should defer to.
    NotOwner { owner: String },
    /// The registry is draining (graceful shutdown); no new actors are spawned.
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

/// One live canvas: its handle, the lease that authorizes this owner to run it,
/// a stop signal for its renew task, and when it was last touched.
struct Entry {
    handle: ActorHandle,
    lease: Lease,
    /// Dropping this sender tells the renew task to stop (its `changed()` wakes
    /// on the channel closing) so the task exits when the canvas is evicted.
    renew_stop: tokio::sync::watch::Sender<()>,
    last_activity: Instant,
}

/// Shared, clone-able registry of canvas actors over one storage backend.
#[derive(Clone)]
pub struct CanvasRegistry {
    inner: Arc<Mutex<HashMap<CanvasId, Entry>>>,
    store: SharedStore,
    /// The coordination seam (lease / routing / presence). `InMemoryCoordinator`
    /// by default; a `FileCoordinator` or networked impl swaps in for multi-host.
    coordinator: Arc<dyn Coordinator>,
    /// This registry instance's owner id — the lease owner and the value cached
    /// for canvases this instance owns. Unique per process/instance.
    owner: String,
    /// Routing cache (MG8.2b): `canvasId -> owner`, so the hot path resolves an
    /// owner with 0 coordination hops after the first lookup.
    route_cache: Arc<Mutex<HashMap<CanvasId, String>>>,
    /// Set by [`shutdown`](CanvasRegistry::shutdown); once true, no new actors
    /// spawn (graceful drain, MG8.3).
    draining: Arc<AtomicBool>,
    /// Lazily-created per-canvas presence broadcasters (MG-3). Separate from the
    /// actor's op fan-out and never tied to actor lifetime, so presence keeps
    /// flowing across actor evict/respawn.
    presence: Arc<Mutex<HashMap<CanvasId, broadcast::Sender<PresenceFrame>>>>,
}

impl CanvasRegistry {
    /// Build a registry over an open sqlite store with the default in-process
    /// coordinator and a default owner id. Single-process dev.
    pub fn new(store: SqliteAdapter) -> Self {
        Self::with_coordinator(store, Arc::new(InMemoryCoordinator::new()), default_owner())
    }

    /// Build a registry with an explicit coordinator + owner id over a freshly
    /// wrapped store.
    pub fn with_coordinator(
        store: SqliteAdapter,
        coordinator: Arc<dyn Coordinator>,
        owner: impl Into<String>,
    ) -> Self {
        Self::with_coordinator_store(Arc::new(Mutex::new(store)), coordinator, owner)
    }

    /// Build a registry over an already-shared [`SharedStore`] with an explicit
    /// coordinator + owner id. The MG8.5 handoff tests use this so two registries
    /// share one coordinator AND one backing store (distinct owner ids), letting
    /// the successor recover the predecessor's durable scene after a lease
    /// release — which an `open_in_memory` sqlite per registry could not do.
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

    /// Build a registry backed by an on-disk sqlite db at `path` (default
    /// coordinator + owner).
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let store = SqliteAdapter::open(path)?;
        Ok(Self::new(store))
    }

    /// Build a registry backed by an in-memory sqlite db (tests).
    pub fn open_in_memory() -> anyhow::Result<Self> {
        let store = SqliteAdapter::open_in_memory()?;
        Ok(Self::new(store))
    }

    /// This registry instance's owner id.
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// Get the handle for `canvas_id`, spawning its lease-guarded actor (and
    /// loading durable state) on first use. Bumps last-activity.
    ///
    /// Acquires the single-writer lease before spawning (MG8.2a): if another
    /// live owner holds it, returns [`SpawnError::NotOwner`]; if the registry is
    /// draining, returns [`SpawnError::Draining`]. A successful spawn starts a
    /// background renew task and caches this owner in the routing map.
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

        // Acquire the single-writer lease before spawning. A live different-owner
        // lease denies us; the routing layer surfaces who to defer to.
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
        // winner/loser under the lock, then do all awaits (shutdown/release of
        // the loser) AFTER the guard is dropped so the future stays `Send`.
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
                // canvas. (renew_stop was moved into the Entry only on insert; on
                // the raced path it is dropped here, stopping our renew task.)
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

    /// Resolve the owner of `canvas_id` (MG8.2b routing).
    ///
    /// Hot path: a cached owner returns with 0 coordination hops. On a cache miss
    /// the coordinator is asked for the live owner; if the canvas is free, this
    /// instance claims the lease (becoming the owner) and caches itself. The
    /// claimed lease is released immediately — `resolve_owner` only answers "who
    /// owns this", and `get_or_spawn` re-acquires (idempotently, as the same
    /// owner) when a connection actually opens the canvas.
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
                // Someone claimed between our find_owner and acquire: re-read.
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

    /// Spawn the background renew task for `lease` and return its stop sender.
    /// The task extends the lease on [`LEASE_RENEW_INTERVAL`] and exits when its
    /// stop sender is dropped (on evict/shutdown) or a renew fails (the lease was
    /// stolen, so this owner no longer runs the canvas).
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

    // ----- Canvas CRUD (MG9.1) ------------------------------------------------

    /// Create a canvas titled `title` and persist it in the durable index.
    /// `canvasId` is derived from the title's slot in the index; callers that
    /// need a specific id use [`create_canvas_with_id`](Self::create_canvas_with_id).
    pub fn create_canvas(&self, title: &str) -> anyhow::Result<CanvasSummary> {
        let id = new_canvas_id();
        self.create_canvas_with_id(&id, title)
    }

    /// Create a canvas with an explicit `id` + `title`, persisted in the index.
    pub fn create_canvas_with_id(&self, id: &str, title: &str) -> anyhow::Result<CanvasSummary> {
        let mut store = self.store.lock().expect("storage mutex poisoned");
        canvas_index::create_canvas(&mut *store, id, title, &now_rfc3339())
    }

    /// List all canvases from the durable index, in insertion order.
    pub fn list_canvases(&self) -> Vec<CanvasSummary> {
        let store = self.store.lock().expect("storage mutex poisoned");
        canvas_index::list_canvases(&*store)
    }

    /// Delete a canvas: evict its actor (releasing the lease), remove it from the
    /// durable index, and prune all of its scene Records. Returns whether the
    /// canvas existed in the index.
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

    // ----- Templates (CC3.2) --------------------------------------------------

    /// Seed the builtin templates once (idempotent). Safe to call on every
    /// startup; the seed guard makes a second call a no-op. Returns how many
    /// builtins were written on this call.
    pub fn seed_templates(&self) -> anyhow::Result<usize> {
        let mut store = self.store.lock().expect("storage mutex poisoned");
        crate::template_store::seed_builtins(&mut *store)
    }

    /// Create (or overwrite) a user template from `contract`.
    pub fn create_template(&self, contract: &shape_scene_core::TemplateContract) -> anyhow::Result<()> {
        let mut store = self.store.lock().expect("storage mutex poisoned");
        crate::template_store::create_template(&mut *store, contract)
    }

    /// List every stored template (seeded builtins minus tombstoned + user
    /// templates), id-sorted.
    pub fn list_templates(&self) -> Vec<shape_scene_core::TemplateContract> {
        let store = self.store.lock().expect("storage mutex poisoned");
        crate::template_store::list_templates(&*store)
    }

    /// Delete a template by id (writes a tombstone so a deleted builtin is not
    /// re-seeded). Returns whether a template Record existed.
    pub fn delete_template(&self, template_id: &str) -> anyhow::Result<bool> {
        let mut store = self.store.lock().expect("storage mutex poisoned");
        crate::template_store::delete_template(&mut *store, template_id)
    }

    // ----- Presence (MG-3) ----------------------------------------------------

    /// A receiver on `canvas_id`'s presence channel (MG-3 ephemeral fan-out),
    /// creating the channel on first use. Best-effort: a slow receiver lags and
    /// drops the oldest frames rather than back-pressuring senders.
    pub fn presence_subscribe(
        &self,
        canvas_id: &CanvasId,
    ) -> broadcast::Receiver<PresenceFrame> {
        self.presence_sender(canvas_id).subscribe()
    }

    /// Publish a presence frame from `from` to every current subscriber of
    /// `canvas_id` (MG-6.2). No-op (returns 0) when nobody is listening; the
    /// sender never blocks. The `from` author rides along so a subscriber can
    /// skip echoing the frame back to its own originating connection.
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

    /// Get (or lazily create) the presence broadcaster for `canvas_id`.
    fn presence_sender(&self, canvas_id: &CanvasId) -> broadcast::Sender<PresenceFrame> {
        let mut map = self.presence.lock().expect("presence mutex poisoned");
        map.entry(canvas_id.clone())
            .or_insert_with(|| broadcast::channel(PRESENCE_CHANNEL_CAPACITY).0)
            .clone()
    }

    // ----- Lifecycle ----------------------------------------------------------

    /// Whether a canvas currently has a live actor in the registry.
    pub fn contains(&self, canvas_id: &CanvasId) -> bool {
        self.inner
            .lock()
            .expect("registry mutex poisoned")
            .contains_key(canvas_id)
    }

    /// Number of live canvas actors.
    pub fn len(&self) -> usize {
        self.inner.lock().expect("registry mutex poisoned").len()
    }

    /// Whether no canvas actors are live.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Evict one canvas: remove it from the map, stop its renew task, shut its
    /// actor down (which flushes + checkpoints), and release its lease so another
    /// owner can take over. No-op if it isn't live.
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

    /// Evict every canvas idle for at least `max_idle`. Returns the evicted ids.
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

    /// Graceful shutdown / drain (MG8.3): reject new spawns, then flush +
    /// checkpoint every live actor and release every lease. After this returns,
    /// every canvas is durably persisted and its lease freed, so a successor
    /// instance can recover it with no data loss.
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

/// Delete every Record under the `"{canvasId}:"` prefix (per-object, canvas-meta,
/// and journal) so a deleted canvas leaves no scene state behind.
fn prune_canvas_records(store: &mut SqliteAdapter, canvas_id: &CanvasId) {
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

/// A unique-enough owner id for a single-process instance: pid + a per-process
/// monotonic suffix so two in-process registries (handoff tests) differ.
fn default_owner() -> String {
    use std::sync::atomic::AtomicU64;
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("owner-{}-{n}", std::process::id())
}

/// A fresh `canvasId` for [`create_canvas`](CanvasRegistry::create_canvas):
/// `"canvas-<pid>-<counter>"`, deterministic within a process (no rand).
fn new_canvas_id() -> String {
    use std::sync::atomic::AtomicU64;
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("canvas-{}-{n}", std::process::id())
}

/// Current wall-clock time as an RFC3339 string for canvas metadata timestamps.
/// Canvas CRUD is platform-layer metadata, so ambient time is acceptable here
/// (scene-core stays time-free; this never touches a Scene).
fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // A coarse epoch-seconds timestamp is enough for list ordering metadata; the
    // exact format is not load-bearing for any test or wire contract.
    format!("{secs}")
}
