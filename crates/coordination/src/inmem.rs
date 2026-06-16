//! In-process [`Coordinator`] for single-process dev (the default). Expiry is
//! checked lazily on read against a pluggable clock so tests can drive time.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{broadcast, Mutex};

use crate::{Coordinator, Lease, Result};

/// Monotonic millisecond clock. Defaults to wall time; tests inject a fake.
pub trait Clock: Send + Sync {
    /// Milliseconds since the Unix epoch.
    fn now_ms(&self) -> u64;
}

#[derive(Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(crate::millis_u64)
            .unwrap_or(0)
    }
}

struct LeaseEntry {
    owner: String,
    token: String,
    expires_at: u64,
}

struct PresenceEntry {
    val: Vec<u8>,
    expires_at: u64,
}

#[derive(Default)]
struct State {
    leases: HashMap<String, LeaseEntry>,
    presence: HashMap<String, HashMap<String, PresenceEntry>>,
}

pub struct InMemoryCoordinator {
    state: Mutex<State>,
    // Channels behind a std mutex (guard never held across an await) so the
    // synchronous `subscribe` can lock it without blocking the runtime.
    channels: StdMutex<HashMap<String, broadcast::Sender<Vec<u8>>>>,
    counter: AtomicU64,
    clock: Arc<dyn Clock>,
    channel_capacity: usize,
}

impl Default for InMemoryCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryCoordinator {
    pub fn new() -> Self {
        Self::with_clock(Arc::new(SystemClock))
    }

    pub fn with_clock(clock: Arc<dyn Clock>) -> Self {
        Self {
            state: Mutex::new(State::default()),
            channels: StdMutex::new(HashMap::new()),
            counter: AtomicU64::new(0),
            clock,
            channel_capacity: 256,
        }
    }

    fn next_token(&self, owner: &str) -> String {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        format!("{owner}-{n}")
    }

    fn sender(&self, canvas_id: &str) -> broadcast::Sender<Vec<u8>> {
        let mut chans = self.channels.lock().expect("channels mutex poisoned");
        chans
            .entry(canvas_id.to_string())
            .or_insert_with(|| broadcast::channel(self.channel_capacity).0)
            .clone()
    }
}

#[async_trait]
impl Coordinator for InMemoryCoordinator {
    async fn acquire_lease(&self, canvas_id: &str, owner: &str, ttl: Duration) -> Result<Lease> {
        let now = self.clock.now_ms();
        let mut state = self.state.lock().await;
        if let Some(existing) = state.leases.get(canvas_id) {
            if existing.expires_at > now && existing.owner != owner {
                anyhow::bail!(
                    "canvas '{canvas_id}' is leased by '{}' until {}",
                    existing.owner,
                    existing.expires_at
                );
            }
        }
        let token = self.next_token(owner);
        let expires_at = now + crate::millis_u64(ttl);
        state.leases.insert(
            canvas_id.to_string(),
            LeaseEntry {
                owner: owner.to_string(),
                token: token.clone(),
                expires_at,
            },
        );
        Ok(Lease {
            canvas_id: canvas_id.to_string(),
            owner: owner.to_string(),
            token,
            expires_at,
        })
    }

    async fn renew(&self, lease: &Lease, ttl: Duration) -> Result<()> {
        let now = self.clock.now_ms();
        let mut state = self.state.lock().await;
        match state.leases.get_mut(&lease.canvas_id) {
            Some(entry)
                if entry.token == lease.token
                    && entry.owner == lease.owner
                    && entry.expires_at > now =>
            {
                entry.expires_at = now + crate::millis_u64(ttl);
                Ok(())
            }
            _ => anyhow::bail!(
                "lease for '{}' (owner '{}') is no longer held",
                lease.canvas_id,
                lease.owner
            ),
        }
    }

    async fn release(&self, lease: Lease) -> Result<()> {
        let mut state = self.state.lock().await;
        if let Some(entry) = state.leases.get(&lease.canvas_id) {
            if entry.token == lease.token && entry.owner == lease.owner {
                state.leases.remove(&lease.canvas_id);
            }
        }
        Ok(())
    }

    async fn find_owner(&self, canvas_id: &str) -> Result<Option<String>> {
        let now = self.clock.now_ms();
        let state = self.state.lock().await;
        Ok(state
            .leases
            .get(canvas_id)
            .filter(|e| e.expires_at > now)
            .map(|e| e.owner.clone()))
    }

    async fn publish(&self, canvas_id: &str, msg: Vec<u8>) -> Result<()> {
        let tx = self.sender(canvas_id);
        let _ = tx.send(msg);
        Ok(())
    }

    fn subscribe(&self, canvas_id: &str) -> broadcast::Receiver<Vec<u8>> {
        self.sender(canvas_id).subscribe()
    }

    async fn presence_put(
        &self,
        canvas_id: &str,
        key: &str,
        val: Vec<u8>,
        ttl: Duration,
    ) -> Result<()> {
        let now = self.clock.now_ms();
        let mut state = self.state.lock().await;
        let entry = state.presence.entry(canvas_id.to_string()).or_default();
        entry.insert(
            key.to_string(),
            PresenceEntry {
                val,
                expires_at: now + crate::millis_u64(ttl),
            },
        );
        Ok(())
    }

    async fn presence_get(&self, canvas_id: &str) -> Result<Vec<(String, Vec<u8>)>> {
        let now = self.clock.now_ms();
        let mut state = self.state.lock().await;
        let mut out = Vec::new();
        if let Some(entries) = state.presence.get_mut(canvas_id) {
            entries.retain(|_, e| e.expires_at > now);
            for (k, e) in entries.iter() {
                out.push((k.clone(), e.val.clone()));
            }
        }
        Ok(out)
    }
}
