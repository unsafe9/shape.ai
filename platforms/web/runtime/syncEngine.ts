// Client object sync engine — a THIN adapter over the Rust collaboration session
// (crates/client-runtime, compiled to wasm). The engine semantics — optimistic
// apply, transient ownership / unacked discard, the coalescing flush DECISION, and
// reconnect reconcile — all run in Rust now; this class keeps only the TS-side IO
// seams the OS forces up here:
//
//   1. PERSISTENCE — the session does the outbox BOOKKEEPING (monotonic localSeq,
//      append/remove, replay order); durability is the `OutboxStore` port (the web
//      shell keeps IndexedDB). `author` persists the entry the session minted;
//      `onAck`/`onRejected` drop the acked rows.
//   2. TRANSPORT — the session buffers coalesced envelopes; this drains them with
//      `takePending()` and hands them to the `EngineTransport` sink.
//   3. TIMERS — the session reports `flushArmed`; this arms the injected coalescing
//      timer and fires `onFlushDue` when it elapses, then drains + sends.
//   4. CLOCK — `now()` stamps each envelope `ts` (injected for deterministic tests).
//
// The wasm instance must be initialized via `ensureSceneCore()` before an engine is
// constructed (the session is created synchronously). The session runs THE
// scene-core object op-apply (the same Rust the server runs).

import type { ObjectScene, ObjectOp, WireOp } from "../shared/object";
import { createWasmSession, type WasmSession } from "../bridge/sceneCoreWasm";
import { type OpId, type OutboxEntry, type OutboxStore } from "./outbox";

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
  /** Durable outbox; the persistence port the session bookkeeping is mirrored to. */
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

/** Result of {@link SyncEngine.author}: errors (empty on success), the minted
 *  opId, and the captured inverse op (the undo entry, D21). */
export type AuthorResult = {
  errors: string[];
  opId?: OpId;
  inverse?: ObjectOp | null;
};

/** The session `author` wire shape: errors + opId + inverse + the durable entry. */
type AuthorWire = {
  errors: string[];
  opId?: OpId | null;
  inverse?: ObjectOp | null;
  entry?: WireOp | null;
};

export class SyncEngine {
  private readonly session: WasmSession;
  private readonly outbox: OutboxStore;
  private readonly transport: EngineTransport;
  private readonly coalesceMs: number;
  private readonly now: () => string;
  private readonly setTimer: (fn: () => void, ms: number) => unknown;
  private readonly clearTimer: (handle: unknown) => void;

  private timer: unknown = null;
  private sceneListeners = new Set<(scene: ObjectScene) => void>();

  constructor(initialScene: ObjectScene, opts: SyncEngineOptions) {
    this.outbox = opts.outbox;
    this.transport = opts.transport;
    this.coalesceMs = opts.coalesceMs ?? COALESCE_MS;
    this.now = opts.now ?? (() => new Date().toISOString());
    this.setTimer = opts.setTimer ?? ((fn, ms) => setTimeout(fn, ms) as unknown);
    this.clearTimer =
      opts.clearTimer ?? ((h) => clearTimeout(h as ReturnType<typeof setTimeout>));
    this.session = createWasmSession({
      welcomeScene: initialScene,
      clientId: opts.clientId,
      coalesceMs: this.coalesceMs
    });
  }

  /** The current optimistic scene. */
  getScene(): ObjectScene {
    return JSON.parse(this.session.scene()) as ObjectScene;
  }

  /** The (object,field) keys currently held under transient ownership. */
  ownedKeySet(): Set<string> {
    return new Set(JSON.parse(this.session.owned_key_set()) as string[]);
  }

  /** Subscribe to optimistic scene updates; returns an unsubscribe. */
  onScene(cb: (scene: ObjectScene) => void): () => void {
    this.sceneListeners.add(cb);
    return () => this.sceneListeners.delete(cb);
  }

  /**
   * Author a local op: optimistic apply (in the session), take transient
   * ownership, persist the minted `WireOp` envelope, then schedule a coalesced
   * send. Rejected-by-core ops never enter the outbox or the wire. Returns the
   * captured inverse op (the undo entry, D21) on success.
   */
  async author(op: ObjectOp): Promise<AuthorResult> {
    const result = JSON.parse(this.session.author(JSON.stringify(op), this.now())) as AuthorWire;
    if (result.errors.length > 0) return { errors: result.errors };

    if (result.entry) await this.outbox.append(result.entry);
    this.emitScene();
    this.armTimer();
    return { errors: [], opId: result.opId ?? undefined, inverse: result.inverse ?? null };
  }

  /**
   * Apply a REMOTE op (peer or self-echo) to the optimistic scene, honoring
   * transient ownership: a remote write to an (object,field) key we still own is
   * dropped until our local op is acked. Returns true if applied.
   */
  applyRemote(op: ObjectOp): boolean {
    const applied = JSON.parse(this.session.apply_remote(JSON.stringify(op))) as boolean;
    if (applied) this.emitScene();
    return applied;
  }

  /**
   * Reconcile an ack: drop the acked entries from the outbox, release their
   * ownership (session-side), and advance the base revision. A duplicate ack is
   * harmless.
   */
  async onAck(result: { opIds: OpId[]; revision?: number }): Promise<void> {
    const removed = JSON.parse(
      this.session.on_ack(JSON.stringify(result.opIds), result.revision ?? -1)
    ) as OpId[];
    await this.outbox.remove(removed);
  }

  /**
   * Reconcile a rejected op: drop it from the outbox and release its ownership so
   * subsequent remote writes for those keys apply. The optimistic write stays in
   * the local scene until the next snapshot/patch corrects it.
   */
  async onRejected(opIds: OpId[]): Promise<void> {
    const removed = JSON.parse(this.session.on_rejected(JSON.stringify(opIds))) as OpId[];
    await this.outbox.remove(removed);
  }

  /**
   * Reconnect reconcile: reset the local base to a fresh `welcome` snapshot, then
   * REPLAY every persisted outbox entry (re-send unacked ops). Ownership is rebuilt
   * from the replayed entries so transient ownership survives a reconnect. The
   * snapshot is authoritative for everything NOT under a surviving unacked write.
   */
  async reconcileSnapshot(snapshot: ObjectScene): Promise<void> {
    const entries = await this.outbox.all();
    this.session.reconcile_snapshot(JSON.stringify(snapshot), JSON.stringify(entries));
    this.emitScene();
    const pending = JSON.parse(this.session.take_pending()) as OutboxEntry[];
    if (pending.length > 0) this.transport.sendEnvelopes(pending);
  }

  /** Flush any buffered coalesced frame immediately (e.g. on shutdown). */
  flush(): void {
    if (this.timer != null) {
      this.clearTimer(this.timer);
      this.timer = null;
    }
    this.session.flush();
    this.drainPending();
  }

  // --- internals -----------------------------------------------------------

  /** Arm the coalescing timer if the session has a flush due and it is not armed. */
  private armTimer(): void {
    if (this.timer != null || !this.session.flush_armed()) return;
    this.timer = this.setTimer(() => {
      this.timer = null;
      this.session.on_flush_due();
      this.drainPending();
    }, this.coalesceMs);
  }

  /** Drain the session's buffered envelopes onto the transport sink. */
  private drainPending(): void {
    const batch = JSON.parse(this.session.take_pending()) as OutboxEntry[];
    if (batch.length > 0) this.transport.sendEnvelopes(batch);
  }

  private emitScene(): void {
    if (this.sceneListeners.size === 0) return;
    const scene = this.getScene();
    for (const cb of this.sceneListeners) cb(scene);
  }
}
