// Peer presence registry — a THIN adapter over the Rust collaboration session
// (crates/client-runtime, compiled to wasm). The latest-wins-per-user store,
// stale-peer TTL expiry, self-skip, and stable color assignment all run in Rust
// now; this class keeps only the injected clock seam (`now()` stamps each frame's
// freshness and drives expiry) and marshals JSON across the FFI boundary.
//
// It owns no document state: the server tags every presence frame with its sender
// and never echoes a client its own frame (self-skip on hello.userId), so every
// frame this registry ingests is a PEER's. On the wire presence `payload` is
// opaque JSON; the session reads the shape the shell publishes:
// { cursor?: {x,y}, viewport?: {x,y,width,height}, userId }. A frame missing a
// userId is dropped (it cannot be attributed to a peer lane).
//
// The wasm instance must be initialized via `ensureSceneCore()` before a registry
// is constructed (the session is created synchronously).

import type { WorldPoint, WorldRect } from "../shared/geometry";
import { createWasmSession, type WasmSession } from "../bridge/sceneCoreWasm";

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
 * Latest-wins-per-user peer cursor registry. Feed it inbound presence frames
 * with {@link ingest}; read the live peers with {@link list}; drop silent peers
 * with {@link expire}. It never tracks the local user — the server self-skip
 * guarantees the local frame never arrives, but {@link ingest} also drops a
 * frame whose userId matches the configured `selfUserId` as a belt-and-braces
 * guard for the case where no userId was negotiated (no self-skip).
 */
export class PeerRegistry {
  private readonly now: () => number;
  private readonly selfUserId: string | undefined;
  private readonly ttlMs: number;
  /** Lazily created on first use: the registry is constructed before connect (so
   *  before `ensureSceneCore` resolves), but only touched after it. */
  private sessionHandle: WasmSession | null = null;

  constructor(opts: { selfUserId?: string; ttlMs?: number; now?: () => number } = {}) {
    this.now = opts.now ?? (() => Date.now());
    this.selfUserId = opts.selfUserId;
    this.ttlMs = opts.ttlMs ?? DEFAULT_PEER_TTL_MS;
  }

  /** The session's peer half does the registry work; its engine half sits idle. */
  private session(): WasmSession {
    if (!this.sessionHandle) {
      this.sessionHandle = createWasmSession({
        welcomeScene: { sceneVersion: 0, objects: [], tags: [], selection: { kind: "canvas" }, updatedAt: "" },
        clientId: "",
        selfUserId: this.selfUserId,
        peerTtlMs: this.ttlMs
      });
    }
    return this.sessionHandle;
  }

  /**
   * Ingest one inbound presence frame. Returns true if it updated the registry.
   * A frame with no userId, or whose userId is the local user, is ignored (the
   * latter only reachable when no userId self-skip was negotiated).
   */
  ingest(payload: unknown): boolean {
    return JSON.parse(this.session().ingest_presence(JSON.stringify(payload ?? null), this.now())) as boolean;
  }

  /**
   * Drop peers whose last frame is older than the TTL. Returns true if any peer
   * was removed (so a caller can re-emit). Call on a timer and/or before list().
   */
  expire(): boolean {
    if (!this.sessionHandle) return false;
    return JSON.parse(this.sessionHandle.expire_peers(this.now())) as boolean;
  }

  /** The live (non-expired) peers, stable-ordered by userId. */
  list(): PeerPresence[] {
    if (!this.sessionHandle) return [];
    return JSON.parse(this.sessionHandle.peers()) as PeerPresence[];
  }

  /** The tracked peer for a userId, or null. */
  get(userId: string): PeerPresence | null {
    return this.list().find((p) => p.userId === userId) ?? null;
  }

  /** Drop everything (e.g. on canvas switch / disconnect). */
  clear(): void {
    this.sessionHandle?.clear_peers();
  }
}
