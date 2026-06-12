//! Durable outbox for unacked client ops.
//!
//! Every authored op is appended here BEFORE the wire send and removed only on
//! ack, so the unacked tail survives a reload/reconnect: on (re)connect the
//! engine replays every entry in `local_seq` order. Re-sending the same `op_id`
//! is safe — the server dedups by it and re-acks idempotently.
//!
//! Persistence sits behind the [`OutboxStore`] port (the web shell keeps an
//! IndexedDB impl; this crate keeps only the in-memory one). The port is
//! synchronous: append/remove/replay/seq are pure decisions, the backend's async
//! IO is the shell's concern.

pub use shape_scene_core::wire::OpId;
use shape_scene_core::wire::WireOp;

/// One outbox row: the `op_id`-stamped [`WireOp`] envelope the WS protocol
/// carries, so an entry re-sends verbatim with no re-encoding.
pub type OutboxEntry = WireOp;

/// A persistence-backend failure surfaced by an [`OutboxStore`]. The in-memory
/// impl never fails; a durable backend maps its errors onto this.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutboxError(pub String);

impl core::fmt::Display for OutboxError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "outbox store error: {}", self.0)
    }
}

/// Stable string key for an `op_id` (set/map membership): `clientId:localSeq`.
pub fn op_id_key(op_id: &OpId) -> String {
    format!("{}:{}", op_id.client_id, op_id.local_seq)
}

/// Durable append-only log of unacked ops, keyed by `op_id`. Implementations
/// must keep `local_seq` monotonic per `client_id` and persist that counter so
/// it never repeats across reloads.
pub trait OutboxStore {
    /// Persist an entry (call before sending it on the wire).
    fn append(&mut self, entry: OutboxEntry) -> Result<(), OutboxError>;
    /// All unacked entries, ascending by `local_seq` (replay order).
    fn all(&self) -> Result<Vec<OutboxEntry>, OutboxError>;
    /// Drop the entries whose `op_id` is in `op_ids` (on ack/rejected).
    fn remove(&mut self, op_ids: &[OpId]) -> Result<(), OutboxError>;
    /// Drop everything (e.g. a hard reset).
    fn clear(&mut self) -> Result<(), OutboxError>;
    /// Next monotonic `local_seq` for this client; advances and persists.
    fn next_local_seq(&mut self) -> Result<i64, OutboxError>;
    /// Replace the contents with durable rows read back on a fresh-session
    /// reconnect, so the engine can replay them. A durable backend persists its
    /// own `local_seq` high-water, so the default only clears + re-appends; the
    /// in-memory store overrides to also reset the counter.
    fn reseed(&mut self, entries: Vec<OutboxEntry>) -> Result<(), OutboxError> {
        self.clear()?;
        for entry in entries {
            self.append(entry)?;
        }
        Ok(())
    }
}

/// Non-durable [`OutboxStore`] backed by a `Vec`. Entries are lost on reload;
/// tests simulate a "reconnect" by reusing the SAME instance (the durable case).
#[derive(Default)]
pub struct InMemoryOutboxStore {
    entries: Vec<OutboxEntry>,
    seq: i64,
}

impl InMemoryOutboxStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl OutboxStore for InMemoryOutboxStore {
    fn append(&mut self, entry: OutboxEntry) -> Result<(), OutboxError> {
        self.entries.push(entry);
        Ok(())
    }

    fn all(&self) -> Result<Vec<OutboxEntry>, OutboxError> {
        let mut out = self.entries.clone();
        out.sort_by_key(|e| e.op_id.local_seq);
        Ok(out)
    }

    fn remove(&mut self, op_ids: &[OpId]) -> Result<(), OutboxError> {
        if op_ids.is_empty() {
            return Ok(());
        }
        let drop: std::collections::HashSet<String> = op_ids.iter().map(op_id_key).collect();
        self.entries.retain(|e| !drop.contains(&op_id_key(&e.op_id)));
        Ok(())
    }

    fn clear(&mut self) -> Result<(), OutboxError> {
        self.entries.clear();
        Ok(())
    }

    fn next_local_seq(&mut self) -> Result<i64, OutboxError> {
        self.seq += 1;
        Ok(self.seq)
    }

    fn reseed(&mut self, entries: Vec<OutboxEntry>) -> Result<(), OutboxError> {
        self.seq = entries.iter().map(|e| e.op_id.local_seq).max().unwrap_or(0);
        self.entries = entries;
        Ok(())
    }
}
