//! Peer presence registry (port of `runtime/peers.ts`) — ephemeral shell state.
//!
//! A pure latest-wins-per-user store for peer cursors. It owns no document state:
//! the server tags every presence frame with its sender and never echoes a client
//! its own frame (self-skip on `hello.userId`), so every frame this registry
//! ingests is a PEER's. The registry keeps the freshest cursor/viewport per
//! `userId`, expires peers whose last frame is older than the stale window (vs an
//! injected `now_ms` — no ambient clock), and assigns each peer a stable color so
//! the overlay can paint a distinct cursor.
//!
//! On the wire presence `payload` is opaque JSON; this module reads the shape the
//! shell publishes: `{ cursor?: {x,y}, viewport?: {x,y,width,height}, userId }`.
//! A frame missing a `userId` is dropped (it cannot be attributed to a peer lane).

use std::collections::HashMap;

use serde::Deserialize;
use shape_scene_core::model::{Bounds, WorldPoint};

/// The presence payload shape the shell publishes/consumes (opaque on the wire).
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresencePayload {
    /// Author/identity lane; latest-wins is keyed on it. Required to be tracked.
    pub user_id: String,
    /// Live pointer position in WORLD coordinates.
    #[serde(default)]
    pub cursor: Option<WorldPoint>,
    /// Live camera viewport in WORLD coordinates (for follow framing).
    #[serde(default)]
    pub viewport: Option<Bounds>,
}

/// One tracked peer: its latest presence plus the time we last heard from it.
#[derive(Clone, Debug, PartialEq)]
pub struct PeerPresence {
    pub user_id: String,
    pub cursor: Option<WorldPoint>,
    pub viewport: Option<Bounds>,
    /// Stable per-peer color for the cursor overlay.
    pub color: &'static str,
    /// Wall-clock ms of the last frame; drives staleness expiry.
    pub last_seen: i64,
}

/// Default window (ms) after which a silent peer is dropped from the registry.
pub const DEFAULT_PEER_TTL_MS: i64 = 10_000;

/// A fixed palette cycled by insertion order so each peer gets a distinct, stable
/// cursor color for the session. Order-stable: the nth distinct `userId` always
/// lands on the nth palette slot until it expires.
const PEER_COLORS: [&str; 8] = [
    "#6b8df2", "#12a594", "#d17b31", "#b65fcf", "#d84d66", "#3aa655", "#e0a92e", "#5b6df0",
];

/// Latest-wins-per-user peer cursor registry. Feed it inbound presence frames with
/// [`ingest`](PeerRegistry::ingest); read the live peers with
/// [`list`](PeerRegistry::list); drop silent peers with
/// [`expire`](PeerRegistry::expire). It never tracks the local user — the server
/// self-skip guarantees the local frame never arrives, but `ingest` also drops a
/// frame whose `userId` matches the configured `self_user_id` as a belt-and-braces
/// guard for the case where no `userId` was negotiated (no self-skip).
pub struct PeerRegistry {
    peers: HashMap<String, PeerPresence>,
    self_user_id: Option<String>,
    ttl_ms: i64,
    /// Next palette slot; advances only when a brand-new peer appears.
    color_cursor: usize,
}

impl PeerRegistry {
    /// A registry with `self_user_id` self-skip and `ttl_ms` stale window.
    /// Pass `ttl_ms = None` for the [`DEFAULT_PEER_TTL_MS`] window.
    pub fn new(self_user_id: Option<String>, ttl_ms: Option<i64>) -> Self {
        Self {
            peers: HashMap::new(),
            self_user_id,
            ttl_ms: ttl_ms.unwrap_or(DEFAULT_PEER_TTL_MS),
            color_cursor: 0,
        }
    }

    /// Ingest one inbound presence frame, stamping `now_ms` as its `last_seen`.
    /// Returns true if it updated the registry. A frame that is not a usable
    /// presence payload (no `userId`), or whose `userId` is the local user, is
    /// ignored (the latter only reachable when no `userId` self-skip was
    /// negotiated).
    pub fn ingest(&mut self, payload: &serde_json::Value, now_ms: i64) -> bool {
        let payload: PresencePayload = match serde_json::from_value(payload.clone()) {
            Ok(p) => p,
            Err(_) => return false,
        };
        if self.self_user_id.as_deref() == Some(payload.user_id.as_str()) {
            return false;
        }

        let color = match self.peers.get(&payload.user_id) {
            Some(existing) => existing.color,
            None => {
                let c = PEER_COLORS[self.color_cursor % PEER_COLORS.len()];
                self.color_cursor += 1;
                c
            }
        };
        self.peers.insert(
            payload.user_id.clone(),
            PeerPresence {
                user_id: payload.user_id,
                cursor: payload.cursor,
                viewport: payload.viewport,
                color,
                last_seen: now_ms,
            },
        );
        true
    }

    /// Drop peers whose last frame is older than the TTL relative to `now_ms`.
    /// Returns true if any peer was removed (so a caller can re-emit). Call on a
    /// timer and/or before [`list`](Self::list).
    pub fn expire(&mut self, now_ms: i64) -> bool {
        let cutoff = now_ms - self.ttl_ms;
        let before = self.peers.len();
        self.peers.retain(|_, peer| peer.last_seen >= cutoff);
        self.peers.len() != before
    }

    /// The live (currently-tracked) peers, stable-ordered by `userId`.
    pub fn list(&self) -> Vec<PeerPresence> {
        let mut out: Vec<PeerPresence> = self.peers.values().cloned().collect();
        out.sort_by(|a, b| a.user_id.cmp(&b.user_id));
        out
    }

    /// The tracked peer for a `userId`, or `None`.
    pub fn get(&self, user_id: &str) -> Option<&PeerPresence> {
        self.peers.get(user_id)
    }

    /// Drop everything (e.g. on canvas switch / disconnect).
    pub fn clear(&mut self) {
        self.peers.clear();
        self.color_cursor = 0;
    }
}
