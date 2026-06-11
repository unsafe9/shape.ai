// Peer presence registry (MG6.2/MG6.3) — ephemeral, shell-only state.
//
// A pure, framework-agnostic latest-wins-per-user store for peer cursors. It
// owns no document state: the server tags every presence frame with its sender
// and never echoes a client its own frame (self-skip on hello.userId), so every
// frame this registry ingests is a PEER's. The registry keeps the freshest
// cursor/viewport per userId, expires peers whose last frame is older than the
// stale window, and assigns each peer a stable color so the overlay can paint a
// distinct cursor without coordinating identity with the server.
//
// On the wire presence `payload` is opaque JSON; this module reads the shape the
// shell publishes: { cursor?: {x,y}, viewport?: {x,y,width,height}, userId }.
// A frame missing a userId is dropped (it cannot be attributed to a peer lane).

import type { WorldPoint, WorldRect } from "../shared/geometry";

/** The presence payload shape the shell publishes/consumes (opaque on the wire). */
export type PresencePayload = {
  /** Author/identity lane; latest-wins is keyed on it. Required to be tracked. */
  userId: string;
  /** Live pointer position in WORLD coordinates. */
  cursor?: WorldPoint;
  /** Live camera viewport in WORLD coordinates (for follow framing). */
  viewport?: WorldRect;
};

/** One tracked peer: its latest presence plus the time we last heard from it. */
export type PeerPresence = {
  userId: string;
  cursor: WorldPoint | null;
  viewport: WorldRect | null;
  /** Stable per-peer color for the cursor overlay. */
  color: string;
  /** Wall-clock ms of the last frame; drives staleness expiry. */
  lastSeen: number;
};

/** Default window after which a silent peer is dropped from the registry. */
export const DEFAULT_PEER_TTL_MS = 10_000;

/**
 * A fixed palette cycled by insertion order so each peer gets a distinct,
 * stable cursor color for the session. Order-stable: the nth distinct userId
 * always lands on the nth palette slot until it expires.
 */
const PEER_COLORS = [
  "#6b8df2",
  "#12a594",
  "#d17b31",
  "#b65fcf",
  "#d84d66",
  "#3aa655",
  "#e0a92e",
  "#5b6df0"
];

/** True when a value is a usable presence payload (has a userId string). */
function isPresencePayload(value: unknown): value is PresencePayload {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as { userId?: unknown }).userId === "string"
  );
}

/**
 * Latest-wins-per-user peer cursor registry. Feed it inbound presence frames
 * with {@link ingest}; read the live peers with {@link list}; drop silent peers
 * with {@link expire}. It never tracks the local user — the server self-skip
 * guarantees the local frame never arrives, but {@link ingest} also drops a
 * frame whose userId matches the configured `selfUserId` as a belt-and-braces
 * guard for the case where no userId was negotiated (no self-skip).
 */
export class PeerRegistry {
  private readonly peers = new Map<string, PeerPresence>();
  private readonly selfUserId: string | undefined;
  private readonly ttlMs: number;
  private readonly now: () => number;
  /** Next palette slot; advances only when a brand-new peer appears. */
  private colorCursor = 0;

  constructor(opts: { selfUserId?: string; ttlMs?: number; now?: () => number } = {}) {
    this.selfUserId = opts.selfUserId;
    this.ttlMs = opts.ttlMs ?? DEFAULT_PEER_TTL_MS;
    this.now = opts.now ?? (() => Date.now());
  }

  /**
   * Ingest one inbound presence frame. Returns true if it updated the registry.
   * A frame with no userId, or whose userId is the local user, is ignored (the
   * latter only reachable when no userId self-skip was negotiated).
   */
  ingest(payload: unknown): boolean {
    if (!isPresencePayload(payload)) return false;
    if (this.selfUserId !== undefined && payload.userId === this.selfUserId) return false;

    const existing = this.peers.get(payload.userId);
    const color = existing?.color ?? PEER_COLORS[this.colorCursor++ % PEER_COLORS.length];
    this.peers.set(payload.userId, {
      userId: payload.userId,
      cursor: payload.cursor ?? null,
      viewport: payload.viewport ?? null,
      color,
      lastSeen: this.now()
    });
    return true;
  }

  /**
   * Drop peers whose last frame is older than the TTL. Returns true if any peer
   * was removed (so a caller can re-emit). Call on a timer and/or before list().
   */
  expire(): boolean {
    const cutoff = this.now() - this.ttlMs;
    let removed = false;
    for (const [userId, peer] of this.peers) {
      if (peer.lastSeen < cutoff) {
        this.peers.delete(userId);
        removed = true;
      }
    }
    return removed;
  }

  /** The live (non-expired) peers, stable-ordered by userId. */
  list(): PeerPresence[] {
    return [...this.peers.values()].sort((a, b) => a.userId.localeCompare(b.userId));
  }

  /** The tracked peer for a userId, or null. */
  get(userId: string): PeerPresence | null {
    return this.peers.get(userId) ?? null;
  }

  /** Drop everything (e.g. on canvas switch / disconnect). */
  clear(): void {
    this.peers.clear();
    this.colorCursor = 0;
  }
}
