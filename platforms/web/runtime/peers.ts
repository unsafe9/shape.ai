// Peer presence registry — a THIN adapter over the Rust collaboration session (compiled to wasm). The
// latest-wins-per-user store, TTL expiry, self-skip, and color assignment all run in Rust; this class
// keeps only the injected clock seam (`now()`) and marshals JSON across the FFI boundary.
//
// On the wire presence `payload` is opaque JSON; the session reads `{ cursor?, viewport?, userId }`. A
// frame missing a userId is dropped. The wasm must be initialized via `ensureSceneCore()` before a
// registry is constructed (the session is created synchronously).

import type { WorldPoint, WorldRect } from "../shared/geometry";
import { createWasmSession, type WasmSession } from "../bridge/sceneCoreWasm";

// The presence payload shape the shell publishes/consumes (opaque on the wire).
export type PresencePayload = {
  // Author/identity lane; latest-wins is keyed on it.
  userId: string;
  // WORLD coordinates.
  cursor?: WorldPoint;
  viewport?: WorldRect;
};

export type PeerPresence = {
  userId: string;
  cursor: WorldPoint | null;
  viewport: WorldRect | null;
  color: string;
  // Wall-clock ms of the last frame; drives staleness expiry.
  lastSeen: number;
};

export const DEFAULT_PEER_TTL_MS = 10_000;

// Latest-wins-per-user peer cursor registry. Never tracks the local user: the server self-skip keeps the
// local frame from arriving, and `ingest` also drops a frame whose userId is `selfUserId` (belt-and-braces).
export class PeerRegistry {
  private readonly now: () => number;
  private readonly selfUserId: string | undefined;
  private readonly ttlMs: number;
  // Lazily created on first use: the registry is constructed before connect (before `ensureSceneCore` resolves), touched only after.
  private sessionHandle: WasmSession | null = null;

  constructor(opts: { selfUserId?: string; ttlMs?: number; now?: () => number } = {}) {
    this.now = opts.now ?? (() => Date.now());
    this.selfUserId = opts.selfUserId;
    this.ttlMs = opts.ttlMs ?? DEFAULT_PEER_TTL_MS;
  }

  // The session's peer half does the registry work; its engine half sits idle.
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

  // Ingest one inbound presence frame; true if it updated the registry. A frame with no userId, or whose userId is the local user, is ignored.
  ingest(payload: unknown): boolean {
    return JSON.parse(this.session().ingest_presence(JSON.stringify(payload ?? null), this.now())) as boolean;
  }

  // Drop peers whose last frame is older than the TTL; true if any was removed. Call on a timer and/or before list().
  expire(): boolean {
    if (!this.sessionHandle) return false;
    return JSON.parse(this.sessionHandle.expire_peers(this.now())) as boolean;
  }

  // The live (non-expired) peers, stable-ordered by userId.
  list(): PeerPresence[] {
    if (!this.sessionHandle) return [];
    return JSON.parse(this.sessionHandle.peers()) as PeerPresence[];
  }

  get(userId: string): PeerPresence | null {
    return this.list().find((p) => p.userId === userId) ?? null;
  }

  // Drop everything (e.g. on canvas switch / disconnect).
  clear(): void {
    this.sessionHandle?.clear_peers();
  }
}
