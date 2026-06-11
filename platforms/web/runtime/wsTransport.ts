// WebSocket implementation of `SceneTransport` (OB4.3).
//
// Mirrors the object-native server WS endpoint (`crates/server/src/ws.rs`): one
// socket at `/ws`, JSON TEXT frames, `hello` first, `welcome` resolves
// `connect()`. Two logical channels share the socket — ops/ack/patch + feature
// are the reliable path, presence is fire-and-forget. The WebSocket constructor
// is injected so tests can drive a mock without a browser global.

import type {
  Ack,
  AckMessage,
  ClientMessage,
  ErrorMessage,
  FeatureServerMessage,
  HelloMessage,
  PatchMessage,
  PresenceServerMessage,
  Region,
  RejectedMessage,
  SceneTransport,
  ServerMessage,
  Unsubscribe,
  WelcomeResult
} from "./transport";
import type { FeatureRequest } from "../shared/object";
import type { OutboxEntry } from "./outbox";
import type { EngineTransport } from "./syncEngine";

/** Minimal slice of the WebSocket API this transport drives (DOM `WebSocket`). */
export type WebSocketLike = {
  send(data: string): void;
  close(): void;
  onopen: ((ev: unknown) => void) | null;
  onclose: ((ev: unknown) => void) | null;
  onerror: ((ev: unknown) => void) | null;
  onmessage: ((ev: { data: unknown }) => void) | null;
};

/** Builds a socket for a given url. Tests inject a mock; prod passes the global. */
export type WebSocketFactory = (url: string) => WebSocketLike;

export type WsTransportOptions = {
  /** Base, e.g. `ws://127.0.0.1:8787` or `wss://host`. Path `/ws` is appended. */
  url: string;
  /** Socket factory; defaults to the browser `WebSocket` global. */
  createSocket?: WebSocketFactory;
  /**
   * Stable author/self-skip identity sent in `hello.userId`. The server
   * attributes this client's ops/presence to it and never echoes them back, so
   * the client treats every inbound `patch`/`presence` as a peer's.
   */
  userId?: string;
  /** Reconnect backoff config; defaults applied per-field (MG8.4). */
  reconnect?: ReconnectOptions;
  /** Random source for backoff jitter; injected for deterministic tests. */
  random?: () => number;
  /** Timer hook for scheduling reconnects; injected so tests drive it manually. */
  setTimer?: (fn: () => void, ms: number) => unknown;
  clearTimer?: (handle: unknown) => void;
};

/**
 * Exponential-backoff-with-jitter schedule for automatic reconnects (MG8.4).
 * The nth attempt waits `min(base * 2^n, max)` ms scaled by a random jitter in
 * `[1 - jitter, 1]` so a fleet of clients does not reconnect in lockstep.
 */
export type ReconnectOptions = {
  /** First-attempt base delay in ms. Default 500. */
  baseMs?: number;
  /** Hard cap on the delay in ms. Default 15000. */
  maxMs?: number;
  /** Jitter fraction in [0, 1]; the delay is multiplied by [1-jitter, 1]. Default 0.3. */
  jitter?: number;
};

/** Connectivity the shell can surface: live socket vs. backing-off reconnect. */
export type ConnectionStatus = "online" | "offline";

type Listener<T> = (msg: T) => void;

/**
 * The nth backoff delay (n starts at 0), jittered. Exported so tests can assert
 * the schedule bounds without reaching into the transport.
 */
export function backoffDelay(attempt: number, opts: Required<ReconnectOptions>, random: () => number): number {
  const exp = opts.baseMs * 2 ** attempt;
  const capped = Math.min(exp, opts.maxMs);
  const factor = 1 - opts.jitter * random();
  return Math.round(capped * factor);
}

function defaultFactory(url: string): WebSocketLike {
  // The browser global; only reached in a real DOM/runtime, never in node tests.
  return new (globalThis as unknown as { WebSocket: new (url: string) => WebSocketLike }).WebSocket(url);
}

export class WsTransport implements SceneTransport, EngineTransport {
  private readonly url: string;
  private readonly createSocket: WebSocketFactory;
  /** Stable author/self-skip identity sent in `hello.userId`. */
  private readonly userId: string | undefined;
  private socket: WebSocketLike | null = null;

  /** Last server seq we observed (welcome/ack/patch); used for `resume`. */
  private lastAckSeq = 0;
  private canvasId: string | null = null;
  /** The window the connection is bound to; re-sent on reconnect (MG9.4). */
  private region: Region | undefined;

  // --- reconnect state (MG8.4) ---
  private readonly reconnectOpts: Required<ReconnectOptions>;
  private readonly random: () => number;
  private readonly setTimer: (fn: () => void, ms: number) => unknown;
  private readonly clearTimer: (handle: unknown) => void;
  /** True once the first connect() succeeded; gates automatic reconnects. */
  private established = false;
  /** True after close(); suppresses reconnect so a deliberate close stays closed. */
  private closing = false;
  /** Count of consecutive failed reconnect attempts (drives the backoff). */
  private reconnectAttempt = 0;
  private reconnectTimer: unknown = null;
  private status: ConnectionStatus = "offline";
  private readonly statusListeners = new Set<Listener<ConnectionStatus>>();

  private readonly patchListeners = new Set<Listener<PatchMessage>>();
  private readonly presenceListeners = new Set<Listener<PresenceServerMessage>>();
  private readonly featureListeners = new Set<Listener<FeatureServerMessage>>();
  private readonly ackListeners = new Set<Listener<AckMessage>>();
  private readonly rejectedListeners = new Set<Listener<RejectedMessage>>();
  private readonly errorListeners = new Set<Listener<ErrorMessage>>();
  /** Fires on EVERY welcome, including the reconnect/resume snapshot. */
  private readonly welcomeListeners = new Set<Listener<WelcomeResult>>();

  /** Pending `connect()` resolution, settled by the first `welcome`. */
  private pendingConnect: {
    resolve: (result: WelcomeResult) => void;
    reject: (err: Error) => void;
  } | null = null;

  constructor(opts: WsTransportOptions) {
    this.url = opts.url;
    this.createSocket = opts.createSocket ?? defaultFactory;
    this.userId = opts.userId;
    this.reconnectOpts = {
      baseMs: opts.reconnect?.baseMs ?? 500,
      maxMs: opts.reconnect?.maxMs ?? 15_000,
      jitter: opts.reconnect?.jitter ?? 0.3
    };
    this.random = opts.random ?? Math.random;
    this.setTimer = opts.setTimer ?? ((fn, ms) => setTimeout(fn, ms) as unknown);
    this.clearTimer = opts.clearTimer ?? ((h) => clearTimeout(h as ReturnType<typeof setTimeout>));
  }

  connect(canvasId: string, region?: Region): Promise<WelcomeResult> {
    if (this.socket) {
      return Promise.reject(new Error("WsTransport already connected"));
    }
    this.canvasId = canvasId;
    this.region = region;
    this.closing = false;
    return this.openSocket();
  }

  /**
   * Open (or re-open) the underlying socket and wait for its first `welcome`.
   * The first call is the user's {@link connect}; subsequent calls are automatic
   * reconnects scheduled after an unexpected close. A successful welcome marks
   * the connection established and online; a close before establishment (or any
   * close while established) schedules a backed-off reconnect (MG8.4).
   */
  private openSocket(): Promise<WelcomeResult> {
    const canvasId = this.canvasId!;
    const socket = this.createSocket(`${this.url.replace(/\/+$/, "")}/ws`);
    this.socket = socket;

    const ready = new Promise<WelcomeResult>((resolve, reject) => {
      this.pendingConnect = { resolve, reject };
    });

    socket.onopen = () => {
      const hello: HelloMessage = {
        type: "hello",
        canvasId,
        ...(this.region ? { region: this.region } : {}),
        lastAckSeq: this.lastAckSeq,
        ...(this.userId !== undefined ? { userId: this.userId } : {})
      };
      this.sendRaw(hello);
    };
    socket.onmessage = (ev) => this.handleMessage(ev.data);
    socket.onerror = () => {
      this.failPending(new Error("WebSocket error"));
    };
    socket.onclose = () => {
      this.failPending(new Error("WebSocket closed before welcome"));
      this.socket = null;
      this.setStatus("offline");
      // A deliberate close() stays closed; any other drop schedules a reconnect
      // once the session was ever established (so the initial connect() promise
      // still rejects normally for a never-established socket).
      if (!this.closing && this.established) this.scheduleReconnect();
    };

    return ready;
  }

  /** Arm the next backed-off reconnect attempt (MG8.4). */
  private scheduleReconnect(): void {
    if (this.reconnectTimer != null) return;
    const delay = backoffDelay(this.reconnectAttempt, this.reconnectOpts, this.random);
    this.reconnectAttempt += 1;
    this.reconnectTimer = this.setTimer(() => {
      this.reconnectTimer = null;
      if (this.closing || this.socket) return;
      // The reconnect welcome rides the welcome stream (no pending connect), so
      // an attached engine reconciles the snapshot and replays the outbox. We do
      // not surface the reconnect promise; failures re-arm via onclose.
      void this.openSocket().catch(() => {
        /* onclose re-arms the backoff */
      });
    }, delay);
  }

  subscribe(region: Region): void {
    if (!this.canvasId) throw new Error("subscribe before connect");
    this.region = region;
    // No live socket (offline): the new window is remembered and seeds the next
    // reconnect hello; no frame is sent now.
    if (!this.socket) return;
    this.sendRaw({ type: "subscribe", canvasId: this.canvasId, region });
  }

  /** Subscribe to connectivity changes (online/offline). Fires on every change. */
  onStatus(cb: Listener<ConnectionStatus>): Unsubscribe {
    return this.subscribeListener(this.statusListeners, cb);
  }

  /** The current connectivity status. */
  get connectionStatus(): ConnectionStatus {
    return this.status;
  }

  private setStatus(next: ConnectionStatus): void {
    if (this.status === next) return;
    this.status = next;
    this.statusListeners.forEach((cb) => cb(next));
  }

  /**
   * Send pre-built `WireOp` envelopes on the reliable channel
   * ({@link EngineTransport}). The sync engine drives this with entries minted
   * from its durable outbox so opIds survive reloads.
   *
   * Offline-safe (MG8.4): with no live socket the send is a no-op — the entries
   * are already durable in the engine's outbox, so the reconnect welcome's
   * reconcile replays them.
   */
  sendEnvelopes(entries: OutboxEntry[]): void {
    if (!this.socket) return;
    this.sendRaw({ type: "ops", ops: entries });
  }

  /** Send a Feature request RPC frame on the reliable channel (OB4.5). */
  sendFeature(request: FeatureRequest): void {
    if (!this.socket) throw new Error("sendFeature before connect");
    this.sendRaw({ type: "feature", request });
  }

  sendPresence(payload: unknown): void {
    if (!this.canvasId) throw new Error("sendPresence before connect");
    // Presence is ephemeral/best-effort: drop it silently while offline.
    if (!this.socket) return;
    this.sendRaw({ type: "presence", canvasId: this.canvasId, payload });
  }

  /**
   * Resume on the open socket: ask the server for a fresh `welcome` snapshot
   * from `lastAckSeq`. The reply rides the welcome stream, which (when an engine
   * is attached) reconciles the snapshot and replays the outbox.
   */
  resume(): void {
    if (!this.canvasId) throw new Error("resume before connect");
    this.sendRaw({ type: "resume", canvasId: this.canvasId, lastAckSeq: this.lastAckSeq });
  }

  onPatch(cb: Listener<PatchMessage>): Unsubscribe {
    return this.subscribeListener(this.patchListeners, cb);
  }

  onPresence(cb: Listener<PresenceServerMessage>): Unsubscribe {
    return this.subscribeListener(this.presenceListeners, cb);
  }

  onFeature(cb: Listener<FeatureServerMessage>): Unsubscribe {
    return this.subscribeListener(this.featureListeners, cb);
  }

  onAck(cb: Listener<AckMessage>): Unsubscribe {
    return this.subscribeListener(this.ackListeners, cb);
  }

  onRejected(cb: Listener<RejectedMessage>): Unsubscribe {
    return this.subscribeListener(this.rejectedListeners, cb);
  }

  onError(cb: Listener<ErrorMessage>): Unsubscribe {
    return this.subscribeListener(this.errorListeners, cb);
  }

  /** Subscribe to every `welcome` (initial + reconnect snapshot). */
  onWelcome(cb: Listener<WelcomeResult>): Unsubscribe {
    return this.subscribeListener(this.welcomeListeners, cb);
  }

  /**
   * Wire a {@link SyncEngine} onto this socket: ack/rejected drop outbox entries,
   * remote patches feed the optimistic/discard path (each `WireOp.propDelta` is
   * the `ObjectOp` to re-apply), and every welcome reconciles the snapshot and
   * replays the outbox. Returns a detach that drops all four subscriptions.
   */
  attachEngine(engine: {
    onAck(r: { opIds: AckMessage["opIds"]; revision?: number }): void | Promise<void>;
    onRejected(opIds: NonNullable<RejectedMessage["opIds"]>): void | Promise<void>;
    applyRemote(op: PatchMessage["ops"][number]["propDelta"]): boolean;
    reconcileSnapshot(scene: WelcomeResult["scene"]): void | Promise<void>;
  }): Unsubscribe {
    const offAck = this.onAck((m) => void engine.onAck({ opIds: m.opIds, revision: m.revision }));
    const offRej = this.onRejected((m) => void engine.onRejected(m.opIds ?? []));
    const offPatch = this.onPatch((m) => {
      for (const wire of m.ops) engine.applyRemote(wire.propDelta);
    });
    const offWelcome = this.onWelcome((w) => void engine.reconcileSnapshot(w.scene));
    return () => {
      offAck();
      offRej();
      offPatch();
      offWelcome();
    };
  }

  close(): void {
    // A deliberate close stays closed: cancel any pending reconnect and suppress
    // the onclose-driven reschedule.
    this.closing = true;
    if (this.reconnectTimer != null) {
      this.clearTimer(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    this.socket?.close();
    this.socket = null;
    this.setStatus("offline");
  }

  /** The last server seq observed; the value sent as `lastAckSeq` on resume. */
  get currentAckSeq(): number {
    return this.lastAckSeq;
  }

  private subscribeListener<T>(set: Set<Listener<T>>, cb: Listener<T>): Unsubscribe {
    set.add(cb);
    return () => set.delete(cb);
  }

  private sendRaw(msg: ClientMessage): void {
    if (!this.socket) throw new Error("WsTransport not connected");
    this.socket.send(JSON.stringify(msg));
  }

  private failPending(err: Error): void {
    if (this.pendingConnect) {
      this.pendingConnect.reject(err);
      this.pendingConnect = null;
    }
  }

  private handleMessage(data: unknown): void {
    if (typeof data !== "string") return;
    let msg: ServerMessage;
    try {
      msg = JSON.parse(data) as ServerMessage;
    } catch {
      return;
    }

    switch (msg.type) {
      case "welcome": {
        this.lastAckSeq = msg.seq;
        // A welcome means the socket is live again: clear the backoff and flip
        // online so the next unexpected drop starts a fresh schedule (MG8.4).
        this.established = true;
        this.reconnectAttempt = 0;
        this.setStatus("online");
        const result: WelcomeResult = { scene: msg.scene, seq: msg.seq, revision: msg.revision };
        if (this.pendingConnect) {
          this.pendingConnect.resolve(result);
          this.pendingConnect = null;
        }
        // A reconnect/resume welcome arrives with no pending connect; the engine
        // still needs it to reconcile + replay, so fire the welcome stream too.
        this.welcomeListeners.forEach((cb) => cb(result));
        return;
      }
      case "ack": {
        this.lastAckSeq = Math.max(this.lastAckSeq, msg.seq);
        this.ackListeners.forEach((cb) => cb(msg));
        return;
      }
      case "rejected": {
        this.rejectedListeners.forEach((cb) => cb(msg));
        return;
      }
      case "patch": {
        this.lastAckSeq = Math.max(this.lastAckSeq, msg.seq);
        this.patchListeners.forEach((cb) => cb(msg));
        return;
      }
      case "feature": {
        this.featureListeners.forEach((cb) => cb(msg));
        return;
      }
      case "presence": {
        this.presenceListeners.forEach((cb) => cb(msg));
        return;
      }
      case "error": {
        this.errorListeners.forEach((cb) => cb(msg));
        this.failPending(new Error(msg.message));
        return;
      }
    }
  }
}

/** Convenience: a `SceneTransport` ready to `connect()`. */
export function createWsTransport(opts: WsTransportOptions): SceneTransport {
  return new WsTransport(opts);
}

/** The `Ack` shape re-exported for callers that handle ops acks directly. */
export type { Ack };
