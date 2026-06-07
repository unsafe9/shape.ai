// Client sync engine (MG4.3 outbox, MG4.4 optimistic + unacked discard,
// MG4.5 coalescing + reconnect reconcile).
//
// The engine sits between the shell and the wire transport. It owns:
//
//   1. the durable outbox (MG4.3) — every authored op is persisted before it is
//      sent, removed on ack, and replayed in localSeq order on (re)connect;
//   2. the optimistic local scene (MG4.4) — ops apply locally the instant they
//      are authored, so the shell never waits for a round-trip;
//   3. transient ownership / unacked discard (MG4.4) — while a local write on an
//      (object, field) is still unacked, an incoming REMOTE patch for that same
//      (object, field) is ignored, so a peer/echo can't clobber the value the
//      user is still dragging;
//   4. coalescing (MG4.5) — rapid ops (continuous move/resize) are batched into
//      one `ops` frame on a ~33ms timer instead of one frame per pointer event;
//   5. reconnect reconcile (MG4.5) — on a fresh `welcome` snapshot the local
//      base is reset to the snapshot and the outbox is replayed on top.
//
// It is transport-shaped via the `EngineTransport` seam so tests drive it with a
// mock socket and the real `WsTransport` wires onto the same calls.

import type { Scene } from "../../shared/schema";
import { applyRenderPatchToShapeScene, type RenderScenePatch } from "../../shared/renderPatch";
import {
  opIdKey,
  type OpId,
  type OutboxEntry,
  type OutboxStore
} from "./outbox";

/** Default coalescing window: rapid ops within this many ms ride one frame. */
export const COALESCE_MS = 33;

/**
 * The narrow transport the engine drives. `WsTransport` implements it; tests
 * supply a mock. The engine never touches the socket directly.
 */
export interface EngineTransport {
  /** Send a batch of opId-stamped envelopes on the reliable channel. */
  sendEnvelopes(entries: OutboxEntry[]): void;
}

/**
 * An applied ack/rejected the engine must reconcile. `applied` distinguishes an
 * ack (drop + commit) from a rejected (drop + the optimistic write was wrong; we
 * leave reconciliation against the next snapshot/patch to the caller).
 */
export type ResolveResult = {
  opIds: OpId[];
  applied: boolean;
  seq?: number;
  revision?: number;
};

export type SyncEngineOptions = {
  /** Authoring identity; stamped into every opId.clientId. */
  clientId: string;
  /** Durable outbox; defaults to an in-memory store if omitted by the caller. */
  outbox: OutboxStore;
  /** Where coalesced frames are flushed. */
  transport: EngineTransport;
  /** Coalescing window in ms; defaults to {@link COALESCE_MS}. */
  coalesceMs?: number;
  /** Clock source for envelope `ts`; injected for deterministic tests. */
  now?: () => string;
  /** Timer hooks; injected so tests can drive coalescing without real time. */
  setTimer?: (fn: () => void, ms: number) => unknown;
  clearTimer?: (handle: unknown) => void;
};

/**
 * Granularity of transient ownership: per (objectId, field).
 *
 * Ownership protects an in-flight CONTINUOUS field edit (drag/resize/reorder/
 * text) so a peer or self-echo can't clobber the value the user is still
 * authoring before our op is acked. A move owns `(id, "position")`; a resize
 * owns `(id, "position")` (geometry); a text edit owns `(id, <field>)`.
 *
 * Structural ops (create/delete/group/tag/select/...) take NO ownership: they
 * are not property writes, so a later remote field edit on the same object is a
 * legitimate concurrent change, and create/delete conflicts are settled by the
 * server's authoritative seq ordering, not by transient client ownership. A
 * batch contributes the union of its members' field keys.
 */
function ownedKeys(patch: RenderScenePatch): string[] {
  switch (patch.kind) {
    case "move-card":
    case "resize-card":
      return [`${patch.id}:position`];
    case "move-group":
    case "resize-group":
      return [`${patch.id}:bounds`];
    case "set-card-z-index":
      return [`${patch.id}:zIndex`];
    case "edit-card-text":
      return [`${patch.id}:${patch.field}`];
    case "batch":
      return patch.ops.flatMap(ownedKeys);
    default:
      return [];
  }
}

/** Does a remote patch write any (object, field) key the client currently owns? */
function remoteTouchesOwnedKey(patch: RenderScenePatch, owned: Set<string>): boolean {
  if (owned.size === 0) return false;
  return ownedKeys(patch).some((key) => owned.has(key));
}

export class SyncEngine {
  private readonly clientId: string;
  private readonly outbox: OutboxStore;
  private readonly transport: EngineTransport;
  private readonly coalesceMs: number;
  private readonly now: () => string;
  private readonly setTimer: (fn: () => void, ms: number) => unknown;
  private readonly clearTimer: (handle: unknown) => void;

  /** Local optimistic scene the shell renders. */
  private scene: Scene;
  /** Revision the next authored op is based on (server revision + local lead). */
  private baseRevision = 0;

  /** Buffer of envelopes waiting on the coalescing flush. */
  private pending: OutboxEntry[] = [];
  private timer: unknown = null;

  /** (object,field) keys with an unacked local write, -> count of owning ops. */
  private readonly ownership = new Map<string, number>();
  /** opId key -> the keys that op owns, so we release them on ack/reject. */
  private readonly opOwnedKeys = new Map<string, string[]>();

  private sceneListeners = new Set<(scene: Scene) => void>();

  constructor(initialScene: Scene, opts: SyncEngineOptions) {
    this.scene = initialScene;
    this.baseRevision = initialScene.sceneVersion;
    this.clientId = opts.clientId;
    this.outbox = opts.outbox;
    this.transport = opts.transport;
    this.coalesceMs = opts.coalesceMs ?? COALESCE_MS;
    this.now = opts.now ?? (() => new Date().toISOString());
    this.setTimer =
      opts.setTimer ?? ((fn, ms) => setTimeout(fn, ms) as unknown);
    this.clearTimer =
      opts.clearTimer ?? ((h) => clearTimeout(h as ReturnType<typeof setTimeout>));
  }

  /** The current optimistic scene. */
  getScene(): Scene {
    return this.scene;
  }

  /** The (object,field) keys currently held under transient ownership. */
  ownedKeySet(): Set<string> {
    return new Set(this.ownership.keys());
  }

  /** Subscribe to optimistic scene updates; returns an unsubscribe. */
  onScene(cb: (scene: Scene) => void): () => void {
    this.sceneListeners.add(cb);
    return () => this.sceneListeners.delete(cb);
  }

  /**
   * Author a local op (MG4.4): apply optimistically, take transient ownership of
   * its keys, persist it to the outbox, then schedule a coalesced send (MG4.5).
   * Rejected-by-core ops never enter the outbox or the wire.
   */
  async author(patch: RenderScenePatch): Promise<{ errors: string[]; opId?: OpId }> {
    const applied = applyRenderPatchToShapeScene(this.scene, patch, this.now());
    if (applied.errors.length > 0) return { errors: applied.errors };

    const localSeq = await this.outbox.nextLocalSeq();
    const opId: OpId = { clientId: this.clientId, localSeq };
    const entry: OutboxEntry = {
      opId,
      baseRevision: this.baseRevision,
      ts: this.now(),
      patch
    };

    this.scene = applied.scene;
    this.takeOwnership(opId, ownedKeys(patch));
    await this.outbox.append(entry);
    this.enqueue(entry);
    this.emitScene();
    return { errors: [], opId };
  }

  /**
   * Apply a REMOTE patch (peer or self-echo) to the optimistic scene, honoring
   * transient ownership (MG4.4): a remote write to an (object,field) key we still
   * own is dropped until our local op is acked. Returns true if applied.
   */
  applyRemote(patch: RenderScenePatch): boolean {
    if (remoteTouchesOwnedKey(patch, this.ownedKeySet())) return false;
    const applied = applyRenderPatchToShapeScene(this.scene, patch, this.now());
    if (applied.errors.length > 0) return false;
    this.scene = applied.scene;
    this.emitScene();
    return true;
  }

  /**
   * Reconcile an ack: drop the acked entries from the outbox, release their
   * ownership, and advance the base revision. A duplicate ack is harmless
   * (already-removed entries are a no-op).
   */
  async onAck(result: { opIds: OpId[]; revision?: number }): Promise<void> {
    await this.outbox.remove(result.opIds);
    for (const id of result.opIds) this.releaseOwnership(id);
    if (typeof result.revision === "number") {
      this.baseRevision = Math.max(this.baseRevision, result.revision);
    }
  }

  /**
   * Reconcile a rejected op: drop it from the outbox and release its ownership so
   * subsequent remote writes for those keys apply. The optimistic write stays in
   * the local scene until the next snapshot/patch corrects it (the caller may
   * re-snapshot); the engine does not attempt a local rollback in MG-4.
   */
  async onRejected(opIds: OpId[]): Promise<void> {
    await this.outbox.remove(opIds);
    for (const id of opIds) this.releaseOwnership(id);
  }

  /**
   * Reconnect reconcile (MG4.5): reset the local base to a fresh `welcome`
   * snapshot, then REPLAY every outbox entry (re-send unacked ops). Ownership is
   * rebuilt from the replayed entries so transient ownership survives a
   * reconnect. The snapshot is taken as authoritative for everything NOT under a
   * surviving unacked write.
   */
  async reconcileSnapshot(snapshot: Scene): Promise<void> {
    this.scene = snapshot;
    this.baseRevision = snapshot.sceneVersion;
    this.ownership.clear();
    this.opOwnedKeys.clear();

    const entries = await this.outbox.all();
    for (const entry of entries) {
      // Re-apply optimistically on top of the snapshot and re-take ownership.
      const applied = applyRenderPatchToShapeScene(this.scene, entry.patch, this.now());
      if (applied.errors.length === 0) this.scene = applied.scene;
      this.takeOwnership(entry.opId, ownedKeys(entry.patch));
    }
    this.emitScene();
    if (entries.length > 0) this.transport.sendEnvelopes(entries);
  }

  /** Flush any buffered coalesced frame immediately (e.g. on shutdown). */
  flush(): void {
    if (this.timer != null) {
      this.clearTimer(this.timer);
      this.timer = null;
    }
    if (this.pending.length === 0) return;
    const batch = this.pending;
    this.pending = [];
    this.transport.sendEnvelopes(batch);
  }

  // --- internals -----------------------------------------------------------

  /** Buffer an envelope and arm the coalescing timer if not already armed. */
  private enqueue(entry: OutboxEntry): void {
    this.pending.push(entry);
    if (this.timer != null) return;
    this.timer = this.setTimer(() => {
      this.timer = null;
      this.flushPending();
    }, this.coalesceMs);
  }

  private flushPending(): void {
    if (this.pending.length === 0) return;
    const batch = this.pending;
    this.pending = [];
    this.transport.sendEnvelopes(batch);
  }

  private takeOwnership(opId: OpId, keys: string[]): void {
    this.opOwnedKeys.set(opIdKey(opId), keys);
    for (const key of keys) {
      this.ownership.set(key, (this.ownership.get(key) ?? 0) + 1);
    }
  }

  private releaseOwnership(opId: OpId): void {
    const key = opIdKey(opId);
    const keys = this.opOwnedKeys.get(key);
    if (!keys) return;
    this.opOwnedKeys.delete(key);
    for (const k of keys) {
      const count = (this.ownership.get(k) ?? 0) - 1;
      if (count <= 0) this.ownership.delete(k);
      else this.ownership.set(k, count);
    }
  }

  private emitScene(): void {
    for (const cb of this.sceneListeners) cb(this.scene);
  }
}
