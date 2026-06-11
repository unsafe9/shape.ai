// Client object sync engine (OB4.3): durable outbox + optimistic apply +
// transient ownership/unacked discard + coalescing + reconnect reconcile.
//
// The engine sits between the shell and the wire transport. It owns:
//
//   1. the durable outbox — every authored op is persisted (as the `WireOp`
//      envelope) before it is sent, removed on ack, and replayed in localSeq
//      order on (re)connect;
//   2. the optimistic local `ObjectScene` — ops apply locally the instant they
//      are authored, so the shell never waits for a round-trip;
//   3. transient ownership / unacked discard — while a local write on an
//      (object, field) is still unacked, an incoming REMOTE op for that same
//      (object, field) is ignored, so a peer/echo can't clobber the value the
//      user is still dragging;
//   4. coalescing — rapid ops (continuous move/transform) are batched into one
//      `ops` frame on a ~33ms timer instead of one frame per pointer event;
//   5. reconnect reconcile — on a fresh `welcome` snapshot the local base is
//      reset to the snapshot and the outbox is replayed on top.
//
// The op-apply is THE scene-core object op-apply (Rust compiled to wasm, the same
// logic the server runs) via `applyObjectOpSync`; the wasm instance must be
// initialized via `ensureSceneCore` before the engine authors. It is transport-
// shaped via the `EngineTransport` seam so tests drive it with a mock socket.

import type { ObjectScene, ObjectOp, WireOp } from "../shared/object";
import { opPrimaryTargetId } from "../shared/object";
import { applyObjectOpSync, type ObjectApplyResult } from "../bridge/sceneCoreWasm";
import {
  opIdKey,
  type OpId,
  type OutboxEntry,
  type OutboxStore
} from "./outbox";

/**
 * The synchronous object op-apply the engine drives. It is THE scene-core
 * op-apply (Rust compiled to wasm, the same logic the server runs); the wasm
 * instance must be initialized — via {@link ensureSceneCore} — before the engine
 * authors. The seam is injectable so tests can substitute a double, but
 * production uses the wasm apply in both the browser and Node/vitest.
 */
export type ApplyObjectOp = (scene: ObjectScene, op: ObjectOp) => ObjectApplyResult;

/** Default coalescing window: rapid ops within this many ms ride one frame. */
export const COALESCE_MS = 33;

/**
 * The narrow transport the engine drives. `WsTransport` implements it; tests
 * supply a mock. The engine never touches the socket directly.
 */
export interface EngineTransport {
  /** Send a batch of `WireOp` envelopes on the reliable channel. */
  sendEnvelopes(entries: OutboxEntry[]): void;
}

export type SyncEngineOptions = {
  /** Authoring identity; stamped into every opId.clientId + WireOp.actor. */
  clientId: string;
  /** Durable outbox; defaults to an in-memory store if omitted by the caller. */
  outbox: OutboxStore;
  /** Where coalesced frames are flushed. */
  transport: EngineTransport;
  /**
   * The synchronous op-apply; defaults to the scene-core wasm apply
   * ({@link applyObjectOpSync}). Injectable for tests. When the default is used,
   * the wasm must already be initialized via `ensureSceneCore()`.
   */
  applyOp?: ApplyObjectOp;
  /** Coalescing window in ms; defaults to {@link COALESCE_MS}. */
  coalesceMs?: number;
  /** Clock source for envelope `ts`; injected for deterministic tests. */
  now?: () => string;
  /** Timer hooks; injected so tests can drive coalescing without real time. */
  setTimer?: (fn: () => void, ms: number) => unknown;
  clearTimer?: (handle: unknown) => void;
};

/** Result of {@link SyncEngine.author}: errors (empty on success), the minted
 *  opId, and the captured inverse op (the undo entry, D21). */
export type AuthorResult = {
  errors: string[];
  opId?: OpId;
  inverse?: ObjectOp | null;
};

/**
 * Granularity of transient ownership: per (objectId, field).
 *
 * Ownership protects an in-flight CONTINUOUS field edit (transform/geometry/
 * text) so a peer or self-echo can't clobber the value the user is still
 * authoring before our op is acked. A `set-transform` owns `(id, "transform")`;
 * a `set-text` owns `(id, "text")`; `edit-geometry` owns `(id, "geometry")`.
 *
 * Structural ops (insert/delete/reparent/reorder/tags/...) take NO ownership:
 * they are not single-field property writes, so a later remote field edit on the
 * same object is a legitimate concurrent change, and create/delete conflicts are
 * settled by the server's authoritative seq ordering. A batch contributes the
 * union of its members' field keys.
 */
function ownedKeys(op: ObjectOp): string[] {
  switch (op.kind) {
    case "set-transform":
      return [`${op.id}:transform`];
    case "edit-geometry":
      return [`${op.id}:geometry`];
    case "set-text":
      return [`${op.id}:text`];
    case "set-style":
      return [`${op.id}:style`];
    case "batch":
      return op.ops.flatMap(ownedKeys);
    default:
      return [];
  }
}

/** Does a remote op write any (object, field) key the client currently owns? */
function remoteTouchesOwnedKey(op: ObjectOp, owned: Set<string>): boolean {
  if (owned.size === 0) return false;
  return ownedKeys(op).some((key) => owned.has(key));
}

export class SyncEngine {
  private readonly clientId: string;
  private readonly outbox: OutboxStore;
  private readonly transport: EngineTransport;
  private readonly applyOp: ApplyObjectOp;
  private readonly coalesceMs: number;
  private readonly now: () => string;
  private readonly setTimer: (fn: () => void, ms: number) => unknown;
  private readonly clearTimer: (handle: unknown) => void;

  /** Local optimistic scene the shell renders. */
  private scene: ObjectScene;
  /** Revision the next authored op is based on (server revision + local lead). */
  private baseRevision = 0;

  /** Buffer of envelopes waiting on the coalescing flush. */
  private pending: OutboxEntry[] = [];
  private timer: unknown = null;

  /** (object,field) keys with an unacked local write, -> count of owning ops. */
  private readonly ownership = new Map<string, number>();
  /** opId key -> the keys that op owns, so we release them on ack/reject. */
  private readonly opOwnedKeys = new Map<string, string[]>();

  private sceneListeners = new Set<(scene: ObjectScene) => void>();

  constructor(initialScene: ObjectScene, opts: SyncEngineOptions) {
    this.scene = initialScene;
    this.baseRevision = initialScene.sceneVersion;
    this.clientId = opts.clientId;
    this.outbox = opts.outbox;
    this.transport = opts.transport;
    this.applyOp = opts.applyOp ?? applyObjectOpSync;
    this.coalesceMs = opts.coalesceMs ?? COALESCE_MS;
    this.now = opts.now ?? (() => new Date().toISOString());
    this.setTimer =
      opts.setTimer ?? ((fn, ms) => setTimeout(fn, ms) as unknown);
    this.clearTimer =
      opts.clearTimer ?? ((h) => clearTimeout(h as ReturnType<typeof setTimeout>));
  }

  /** The current optimistic scene. */
  getScene(): ObjectScene {
    return this.scene;
  }

  /** The (object,field) keys currently held under transient ownership. */
  ownedKeySet(): Set<string> {
    return new Set(this.ownership.keys());
  }

  /** Subscribe to optimistic scene updates; returns an unsubscribe. */
  onScene(cb: (scene: ObjectScene) => void): () => void {
    this.sceneListeners.add(cb);
    return () => this.sceneListeners.delete(cb);
  }

  /**
   * Build the `WireOp` envelope for an `ObjectOp` authored at `localSeq`. The
   * `propDelta` carries the full op; `objectId`/`kind` are descriptive (mirroring
   * the server's `op_to_wire`).
   */
  private wireOp(op: ObjectOp, localSeq: number): WireOp {
    return {
      opId: { clientId: this.clientId, localSeq },
      objectId: opPrimaryTargetId(op),
      kind: op.kind,
      propDelta: op,
      baseRevision: this.baseRevision,
      actor: this.clientId,
      ts: this.now()
    };
  }

  /**
   * Author a local op: apply optimistically, take transient ownership of its
   * keys, persist its `WireOp` envelope to the outbox, then schedule a coalesced
   * send. Rejected-by-core ops never enter the outbox or the wire. Returns the
   * captured inverse op (the undo entry, D21) on success.
   */
  async author(op: ObjectOp): Promise<AuthorResult> {
    const applied = this.applyOp(this.scene, op);
    if (applied.errors.length > 0) return { errors: applied.errors };

    const localSeq = await this.outbox.nextLocalSeq();
    const entry = this.wireOp(op, localSeq);
    const opId = entry.opId;

    this.scene = applied.scene;
    this.takeOwnership(opId, ownedKeys(op));
    await this.outbox.append(entry);
    this.enqueue(entry);
    this.emitScene();
    return { errors: [], opId, inverse: applied.inverse };
  }

  /**
   * Apply a REMOTE op (peer or self-echo) to the optimistic scene, honoring
   * transient ownership: a remote write to an (object,field) key we still own is
   * dropped until our local op is acked. Returns true if applied.
   */
  applyRemote(op: ObjectOp): boolean {
    if (remoteTouchesOwnedKey(op, this.ownedKeySet())) return false;
    const applied = this.applyOp(this.scene, op);
    if (applied.errors.length > 0) return false;
    this.scene = applied.scene;
    this.emitScene();
    return true;
  }

  /**
   * Reconcile an ack: drop the acked entries from the outbox, release their
   * ownership, and advance the base revision. A duplicate ack is harmless.
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
   * the local scene until the next snapshot/patch corrects it.
   */
  async onRejected(opIds: OpId[]): Promise<void> {
    await this.outbox.remove(opIds);
    for (const id of opIds) this.releaseOwnership(id);
  }

  /**
   * Reconnect reconcile: reset the local base to a fresh `welcome` snapshot, then
   * REPLAY every outbox entry (re-send unacked ops). Ownership is rebuilt from the
   * replayed entries so transient ownership survives a reconnect. The snapshot is
   * authoritative for everything NOT under a surviving unacked write.
   */
  async reconcileSnapshot(snapshot: ObjectScene): Promise<void> {
    this.scene = snapshot;
    this.baseRevision = snapshot.sceneVersion;
    this.ownership.clear();
    this.opOwnedKeys.clear();

    const entries = await this.outbox.all();
    for (const entry of entries) {
      const applied = this.applyOp(this.scene, entry.propDelta);
      if (applied.errors.length === 0) this.scene = applied.scene;
      this.takeOwnership(entry.opId, ownedKeys(entry.propDelta));
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
