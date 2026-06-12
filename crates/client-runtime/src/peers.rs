//! Pure latest-wins-per-user peer presence registry: keeps the freshest
//! cursor/viewport per `userId`, expires peers past the stale window (vs an
//! injected `now_ms`), and assigns each a stable color. A frame missing a
//! `userId` is dropped (it can't be attributed to a peer lane).

use std::collections::HashMap;

use serde::Deserialize;
use shape_scene_core::model::{Bounds, WorldPoint};

/// The presence payload shape (camelCase on the wire, opaque JSON).
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresencePayload {
    /// Identity lane latest-wins is keyed on; required to be tracked.
    pub user_id: String,
    /// Live pointer position in WORLD coordinates.
    #[serde(default)]
    pub cursor: Option<WorldPoint>,
    /// Live camera viewport in WORLD coordinates.
    #[serde(default)]
    pub viewport: Option<Bounds>,
}

/// One tracked peer: its latest presence plus the time we last heard from it.
#[derive(Clone, Debug, PartialEq)]
pub struct PeerPresence {
    pub user_id: String,
    pub cursor: Option<WorldPoint>,
    pub viewport: Option<Bounds>,
    pub color: &'static str,
    /// Wall-clock ms of the last frame; drives staleness expiry.
    pub last_seen: i64,
}

/// Default window (ms) after which a silent peer is dropped.
pub const DEFAULT_PEER_TTL_MS: i64 = 10_000;

/// A palette cycled by insertion order: the nth distinct `userId` lands on the
/// nth slot until it expires, so each peer gets a stable cursor color.
const PEER_COLORS: [&str; 8] = [
    "#6b8df2", "#12a594", "#d17b31", "#b65fcf", "#d84d66", "#3aa655", "#e0a92e", "#5b6df0",
];

/// Latest-wins-per-user peer cursor registry. It never tracks the local user:
/// the server self-skip keeps the local frame from arriving, and `ingest` also
/// drops a frame whose `userId` matches `self_user_id` as a belt-and-braces
/// guard when no `userId` self-skip was negotiated.
pub struct PeerRegistry {
    peers: HashMap<String, PeerPresence>,
    self_user_id: Option<String>,
    ttl_ms: i64,
    /// Next palette slot; advances only when a brand-new peer appears.
    color_cursor: usize,
}

impl PeerRegistry {
    /// `ttl_ms = None` uses the [`DEFAULT_PEER_TTL_MS`] window.
    pub fn new(self_user_id: Option<String>, ttl_ms: Option<i64>) -> Self {
        Self {
            peers: HashMap::new(),
            self_user_id,
            ttl_ms: ttl_ms.unwrap_or(DEFAULT_PEER_TTL_MS),
            color_cursor: 0,
        }
    }

    /// Ingest one presence frame, stamping `now_ms` as its `last_seen`. Returns
    /// true if it updated the registry. A frame with no `userId`, or one whose
    /// `userId` is the local user, is ignored.
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
    /// Returns true if any peer was removed (so a caller can re-emit).
    pub fn expire(&mut self, now_ms: i64) -> bool {
        let cutoff = now_ms - self.ttl_ms;
        let before = self.peers.len();
        self.peers.retain(|_, peer| peer.last_seen >= cutoff);
        self.peers.len() != before
    }

    /// The live peers, stable-ordered by `userId`.
    pub fn list(&self) -> Vec<PeerPresence> {
        let mut out: Vec<PeerPresence> = self.peers.values().cloned().collect();
        out.sort_by(|a, b| a.user_id.cmp(&b.user_id));
        out
    }

    pub fn get(&self, user_id: &str) -> Option<&PeerPresence> {
        self.peers.get(user_id)
    }

    /// Drop everything (e.g. on canvas switch / disconnect).
    pub fn clear(&mut self) {
        self.peers.clear();
        self.color_cursor = 0;
    }
}
