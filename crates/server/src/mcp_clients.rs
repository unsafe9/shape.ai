//! MG2.3: in-memory MCP client registry + per-client trace ring.
//!
//! Port of the dock-facing half of the Node `src/server/mcpClients.ts`: a live
//! map of connected MCP companions (one entry per session), a stable per-client
//! colour derived from a FNV-like hash, and a bounded ring of recent read/write
//! trace events the companion dock polls over HTTP. This is display-only state
//! that is never persisted — it disappears with the process.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

/// 12 distinct hues, deterministic per clientId — identical palette to the Node
/// dock so a companion keeps its colour across the Node→Rust cutover.
const COLOR_PALETTE: [&str; 12] = [
    "#e05252", "#e0893d", "#d4be34", "#52b765", "#3db8b8", "#3d82e0", "#7b52e0", "#c252c2",
    "#e07b7b", "#7be07b", "#7bc4e0", "#c2a852",
];

/// FNV-like 32-bit hash matching the Node `hashClientId` (`h = imul(31,h)+c`).
fn hash_client_id(client_id: &str) -> u32 {
    let mut h: u32 = 0;
    for byte in client_id.bytes() {
        h = 31u32.wrapping_mul(h).wrapping_add(byte as u32);
    }
    h
}

/// Stable colour for a client id from the 12-hue palette.
pub fn color_from_client_id(client_id: &str) -> &'static str {
    COLOR_PALETTE[(hash_client_id(client_id) as usize) % COLOR_PALETTE.len()]
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// One connected MCP companion, as the dock sees it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpClientIdentity {
    pub client_id: String,
    pub actor_type: &'static str,
    pub label: String,
    pub name: String,
    pub version: String,
    pub color: &'static str,
    pub transport: &'static str,
    pub dock_state: &'static str,
    pub connected_at: u64,
    pub last_activity_at: u64,
}

/// Kind of a projected trace event (a subset of the Node `TraceKind`; the Rust
/// server tracks read/write/comment/export/error).
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TraceKind {
    Read,
    Write,
    Comment,
    Export,
    Error,
}

/// One display-only trace event. Recomputed on read; never persisted.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceEvent {
    pub client_id: String,
    pub kind: TraceKind,
    pub verb: String,
    pub at: u64,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// Maximum number of trace events kept per client ring (matches the Node
/// `READ_RING_CAPACITY`).
const RING_CAPACITY: usize = 50;

struct ClientEntry {
    identity: McpClientIdentity,
    ring: Vec<TraceEvent>,
}

/// Shared, clone-able registry of connected MCP companions + their trace rings.
#[derive(Clone, Default)]
pub struct ClientRegistry {
    inner: Arc<Mutex<HashMap<String, ClientEntry>>>,
}

impl ClientRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register (or refresh) a companion from its advertised name/version.
    /// `label` de-duplicates by connect order, mirroring the Node registry.
    pub fn register(&self, client_id: &str, name: &str, version: &str, transport: &'static str) {
        let mut map = self.inner.lock().expect("client registry mutex poisoned");
        let label = deduplicate_label(&map, name, client_id);
        let now = now_ms();
        let identity = McpClientIdentity {
            client_id: client_id.to_string(),
            actor_type: "mcp",
            label,
            name: name.to_string(),
            version: version.to_string(),
            color: color_from_client_id(client_id),
            transport,
            dock_state: "idle",
            connected_at: now,
            last_activity_at: now,
        };
        map.entry(client_id.to_string())
            .and_modify(|e| {
                e.identity.name = name.to_string();
                e.identity.version = version.to_string();
                e.identity.last_activity_at = now;
                e.identity.dock_state = "idle";
            })
            .or_insert(ClientEntry {
                identity,
                ring: Vec::new(),
            });
    }

    /// Snapshot every connected companion (dock list).
    pub fn list(&self) -> Vec<McpClientIdentity> {
        self.inner
            .lock()
            .expect("client registry mutex poisoned")
            .values()
            .map(|e| e.identity.clone())
            .collect()
    }

    /// Push one trace event into a client's ring, evicting the oldest when full,
    /// and bump its last-activity / dock state.
    pub fn push_trace(
        &self,
        client_id: &str,
        kind: TraceKind,
        verb: &str,
        summary: String,
        error_message: Option<String>,
    ) {
        let mut map = self.inner.lock().expect("client registry mutex poisoned");
        let Some(entry) = map.get_mut(client_id) else {
            return;
        };
        let at = now_ms();
        entry.identity.last_activity_at = at;
        entry.identity.dock_state = if error_message.is_some() {
            "error"
        } else {
            "active"
        };
        entry.ring.push(TraceEvent {
            client_id: client_id.to_string(),
            kind,
            verb: verb.to_string(),
            at,
            summary,
            error_message,
        });
        if entry.ring.len() > RING_CAPACITY {
            entry.ring.remove(0);
        }
    }

    /// Recent trace for one client, newest-first, capped at `limit`.
    pub fn trace(&self, client_id: &str, limit: usize) -> Vec<TraceEvent> {
        let map = self.inner.lock().expect("client registry mutex poisoned");
        let Some(entry) = map.get(client_id) else {
            return Vec::new();
        };
        let mut out: Vec<TraceEvent> = entry.ring.iter().rev().cloned().collect();
        out.truncate(limit);
        out
    }

    /// Mark a companion disconnected (kept so a late trace read still resolves).
    pub fn mark_disconnected(&self, client_id: &str) {
        if let Some(entry) = self
            .inner
            .lock()
            .expect("client registry mutex poisoned")
            .get_mut(client_id)
        {
            entry.identity.dock_state = "disconnected";
            entry.identity.last_activity_at = now_ms();
        }
    }
}

/// Append " (2)", " (3)" … when a companion's name collides with a connected
/// one, by connect order — a faithful port of the Node `deduplicateLabel`.
fn deduplicate_label(map: &HashMap<String, ClientEntry>, base: &str, new_id: &str) -> String {
    let prefix = format!("{base} (");
    let existing = map
        .values()
        .filter(|e| {
            e.identity.client_id != new_id
                && (e.identity.label == base || e.identity.label.starts_with(&prefix))
        })
        .count();
    if existing == 0 {
        base.to_string()
    } else {
        format!("{base} ({})", existing + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_is_stable_and_from_palette() {
        let c1 = color_from_client_id("abc");
        let c2 = color_from_client_id("abc");
        assert_eq!(c1, c2);
        assert!(COLOR_PALETTE.contains(&c1));
    }

    #[test]
    fn hash_matches_node_fnv_like() {
        // h = imul(31, h) + charCode, unsigned 32-bit.
        // "a" => 97
        assert_eq!(hash_client_id("a"), 97);
        // "ab" => 31*97 + 98 = 3105
        assert_eq!(hash_client_id("ab"), 3105);
    }

    #[test]
    fn register_then_list_and_trace() {
        let reg = ClientRegistry::new();
        reg.register("s1", "claude", "1.0", "http");
        reg.register("s1", "claude", "1.0", "http");
        let list = reg.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].label, "claude");

        reg.push_trace("s1", TraceKind::Write, "create-group", "made g1".into(), None);
        let trace = reg.trace("s1", 50);
        assert_eq!(trace.len(), 1);
        assert_eq!(trace[0].verb, "create-group");
    }

    #[test]
    fn label_dedup_on_collision() {
        let reg = ClientRegistry::new();
        reg.register("s1", "claude", "1", "http");
        reg.register("s2", "claude", "1", "http");
        let labels: Vec<String> = reg.list().into_iter().map(|c| c.label).collect();
        assert!(labels.contains(&"claude".to_string()));
        assert!(labels.contains(&"claude (2)".to_string()));
    }
}
