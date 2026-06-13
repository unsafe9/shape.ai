// The single client data path: the shell loads and mutates the canvas through the WS transport,
// composing `WsTransport` (the wire), `SyncEngine` (optimistic op-apply + durable outbox + coalescing
// + reconnect reconcile), and an `OutboxStore` into one handle.
//
// Routing: object ops flow through `engine.author()` (optimistic apply, outbox, `ops` envelope, dropped
// on ack). Selection-only changes are presence, NOT document ops — `saveSelection` broadcasts a best-
// effort presence frame, never entering the outbox or bumping the revision. Feature traffic rides the
// single WS `feature` RPC channel. Stale-snapshot guards live in `SyncEngine.reconcileSnapshot`.

import type { ObjectScene, ObjectOp, ObjectSelection, FeatureRequest, FeatureResponse } from "../shared/object";
import type { WorldPoint, WorldRect } from "../shared/geometry";
import { WsTransport, type ConnectionStatus, type ReconnectOptions, type WebSocketFactory } from "./wsTransport";
import { SyncEngine, type AuthorResult } from "./syncEngine";
import { ensureSceneCore, createWasmWindow, type WasmWindow } from "../bridge/sceneCoreWasm";
import { InMemoryOutboxStore, type OutboxStore } from "./outbox";
import { PeerRegistry, type PeerPresence } from "./peers";
import type { Bbox, FeatureServerMessage, PatchMessage, Region } from "./transport";

export type SceneClientOptions = {
  // Wire base, e.g. `ws://127.0.0.1:8787`; the transport appends `/ws`.
  url: string;
  // Authoring identity stamped into every opId.clientId.
  clientId: string;
  // Stable user identity, sent in `hello.userId` and stamped on every presence frame so the server
  // self-skips this client's ops/presence and peers surface by userId. Defaults to `clientId`.
  userId?: string;
  // Durable outbox; defaults to an in-memory store (non-durable fallback).
  outbox?: OutboxStore;
  createSocket?: WebSocketFactory;
  coalesceMs?: number;
  // Injected for deterministic tests.
  now?: () => string;
  setTimer?: (fn: () => void, ms: number) => unknown;
  clearTimer?: (handle: unknown) => void;
  reconnect?: ReconnectOptions;
  random?: () => number;
  // The viewport bbox is grown by this fraction on each side before it becomes the subscribed region, so
  // a small pan/zoom does not immediately re-subscribe. Omit for the core default.
  viewportMargin?: number;
  viewportDebounceMs?: number;
  // Stale-peer cursor TTL in ms.
  peerTtlMs?: number;
  nowMs?: () => number;
};

export type Unsubscribe = () => void;

// A camera-derived viewport in WORLD coordinates (before the window margin).
export type Viewport = Bbox;

export const DEFAULT_VIEWPORT_DEBOUNCE_MS = 200;

export const DEFAULT_PEER_TTL_MS = 10_000;

// A transport-backed object scene store. Open with `connect`, author ops with `applyObjectOp`, move the
// selection (presence-only) with `saveSelection`, drive feature RPCs with `sendFeature`.
export class SceneClient {
  private readonly url: string;
  private readonly clientId: string;
  private readonly userId: string;
  // Re-pointed on switchCanvas so the new canvas starts with a clean outbox.
  private outbox: OutboxStore;
  private readonly createSocket: SceneClientOptions["createSocket"];
  private readonly coalesceMs?: number;
  private readonly now: () => string;
  private readonly setTimer: (fn: () => void, ms: number) => unknown;
  private readonly clearTimer: (handle: unknown) => void;
  private readonly reconnect?: ReconnectOptions;
  private readonly random?: () => number;
  // Window margin handed to the core decision state; `-1` = core default.
  private readonly viewportMargin: number;
  private readonly viewportDebounceMs: number;

  private transport: WsTransport | null = null;
  private engine: SyncEngine | null = null;
  private detachEngine: Unsubscribe | null = null;
  private canvasId: string | null = null;

  // The viewport-windowing DECISION state, owned by the core. Null before `connect` initializes the
  // wasm; the shell only drives the debounce timer + transport off its decisions.
  private windowState: WasmWindow | null = null;
  private viewportTimer: unknown = null;

  private readonly sceneListeners = new Set<(scene: ObjectScene, settledKeys: string[]) => void>();
  private readonly patchListeners = new Set<(patch: PatchMessage) => void>();
  private readonly featureListeners = new Set<(response: FeatureResponse) => void>();
  private readonly statusListeners = new Set<(status: ConnectionStatus) => void>();
  // Peer cursor registry + subscribers; rebuilt per connection.
  private peers: PeerRegistry;
  private readonly peerListeners = new Set<(peers: PeerPresence[]) => void>();
  private readonly peerTtlMs: number;
  private readonly nowMs: () => number;
  private offEnginePatch: Unsubscribe | null = null;
  private offTransportPatch: Unsubscribe | null = null;
  private offTransportFeature: Unsubscribe | null = null;
  private offTransportStatus: Unsubscribe | null = null;
  private offTransportPresence: Unsubscribe | null = null;

  constructor(opts: SceneClientOptions) {
    this.url = opts.url;
    this.clientId = opts.clientId;
    this.userId = opts.userId ?? opts.clientId;
    this.outbox = opts.outbox ?? new InMemoryOutboxStore();
    this.createSocket = opts.createSocket;
    this.coalesceMs = opts.coalesceMs;
    this.now = opts.now ?? (() => new Date().toISOString());
    this.setTimer = opts.setTimer ?? ((fn, ms) => setTimeout(fn, ms) as unknown);
    this.clearTimer = opts.clearTimer ?? ((h) => clearTimeout(h as ReturnType<typeof setTimeout>));
    this.reconnect = opts.reconnect;
    this.random = opts.random;
    this.viewportMargin = opts.viewportMargin ?? -1;
    this.viewportDebounceMs = opts.viewportDebounceMs ?? DEFAULT_VIEWPORT_DEBOUNCE_MS;
    this.peerTtlMs = opts.peerTtlMs ?? DEFAULT_PEER_TTL_MS;
    this.nowMs = opts.nowMs ?? (() => Date.now());
    this.peers = this.newPeerRegistry();
  }

  // A fresh peer registry seeded with this client's self-skip identity.
  private newPeerRegistry(): PeerRegistry {
    return new PeerRegistry({ selfUserId: this.userId, ttlMs: this.peerTtlMs, now: this.nowMs });
  }

  // Open the session: connect the transport, send `hello`, resolve with the welcome `ObjectScene`. The
  // engine is created from the welcome scene and attached so acks/rejects/patches/reconnect-welcomes reconcile automatically.
  async connect(canvasId: string, region?: Region): Promise<ObjectScene> {
    if (this.transport) throw new Error("SceneClient already connected");
    this.canvasId = canvasId;
    const seed = region?.bbox;
    const transport = new WsTransport({
      url: this.url,
      createSocket: this.createSocket,
      userId: this.userId,
      ...(this.reconnect ? { reconnect: this.reconnect } : {}),
      ...(this.random ? { random: this.random } : {}),
      setTimer: this.setTimer,
      clearTimer: this.clearTimer
    });
    this.transport = transport;

    // The engine's optimistic op-apply is the scene-core wasm core; init it before the engine can author.
    const sceneCoreReady = ensureSceneCore();
    const welcome = await transport.connect(canvasId, this.regionFor(seed));
    await sceneCoreReady;
    // Windowing decisions live in the core; seed its state from the connect region now the wasm is initialized.
    this.windowState = createWasmWindow({ seed, margin: this.viewportMargin });

    const engine = new SyncEngine(welcome.scene, {
      clientId: this.clientId,
      outbox: this.outbox,
      transport,
      ...(this.coalesceMs !== undefined ? { coalesceMs: this.coalesceMs } : {}),
      now: this.now,
      setTimer: this.setTimer,
      clearTimer: this.clearTimer
    });
    this.engine = engine;

    this.detachEngine = transport.attachEngine(engine);

    this.offEnginePatch = engine.onScene((scene, settledKeys) => this.emitScene(scene, settledKeys));
    this.offTransportPatch = transport.onPatch((patch) => this.emitPatch(patch));
    this.offTransportFeature = transport.onFeature((frame) => this.emitFeature(frame));
    this.offTransportStatus = transport.onStatus((status) => this.emitStatus(status));
    this.offTransportPresence = transport.onPresence((frame) => this.ingestPresence(frame.payload));

    return welcome.scene;
  }

  // Build a `Region` for the current canvas from an optional window bbox.
  private regionFor(bbox: Bbox | undefined): Region | undefined {
    if (!this.canvasId) return undefined;
    return bbox ? { canvasId: this.canvasId, bbox } : undefined;
  }

  // Re-aim the subscription to the bbox derived from a camera viewport + margin, debounced so a
  // continuous pan/zoom does not spam re-subscribes. The margin + re-subscribe DECISION lives in the core.
  setViewport(viewport: Viewport): void {
    if (this.viewportTimer != null) this.clearTimer(this.viewportTimer);
    this.viewportTimer = this.setTimer(() => {
      this.viewportTimer = null;
      const next = this.windowState?.on_viewport(JSON.stringify(viewport));
      this.subscribeWindowDecision(next);
    }, this.viewportDebounceMs);
  }

  // Immediately re-aim the window to `bbox` (no debounce); for tests/programmatic moves.
  subscribeRegion(bbox: Bbox): void {
    if (this.viewportTimer != null) {
      this.clearTimer(this.viewportTimer);
      this.viewportTimer = null;
    }
    this.subscribeWindowDecision(this.windowState?.set_window(JSON.stringify(bbox)));
  }

  // Drop the window: re-subscribe to the whole canvas (no bbox).
  subscribeWholeCanvas(): void {
    if (this.viewportTimer != null) {
      this.clearTimer(this.viewportTimer);
      this.viewportTimer = null;
    }
    if (!this.windowState?.subscribe_whole_canvas()) return;
    if (this.canvasId) this.transport?.subscribe({ canvasId: this.canvasId });
  }

  // Act on a core windowing decision: `"null"`/undefined = no change (emit nothing); a non-null bbox is the new window to `subscribe` to.
  private subscribeWindowDecision(decisionJson: string | undefined): void {
    if (decisionJson === undefined) return;
    const bbox = JSON.parse(decisionJson) as Bbox | null;
    if (bbox === null) return;
    if (this.canvasId) this.transport?.subscribe({ canvasId: this.canvasId, bbox });
  }

  // The window bbox currently subscribed, or null for whole-canvas.
  get currentWindow(): Bbox | null {
    if (!this.windowState) return null;
    return JSON.parse(this.windowState.current_window()) as Bbox | null;
  }

  // The current optimistic object scene, or null before `connect`.
  get scene(): ObjectScene | null {
    return this.engine?.getScene() ?? null;
  }

  // Author an object op (optimistic apply + outbox + coalesced send). Returns rejecting-core errors
  // (empty on success), the minted opId, and the captured inverse op (the undo entry).
  async applyObjectOp(op: ObjectOp): Promise<AuthorResult> {
    if (!this.engine) throw new Error("applyObjectOp before connect");
    return this.engine.author(op);
  }

  // Move the selection WITHOUT a document op: broadcast a best-effort presence frame only. Selection is
  // ephemeral — it never enters the outbox, produces an `ops` frame, or bumps the revision.
  saveSelection(selection: ObjectSelection): void {
    if (!this.transport) throw new Error("saveSelection before connect");
    this.transport.sendPresence({ kind: "select", selection });
  }

  // Send a Feature request RPC frame on the single WS feature channel (comment upsert, template apply,
  // export, canvas switch). The response arrives on `onFeature`.
  sendFeature(request: FeatureRequest): void {
    if (!this.transport) throw new Error("sendFeature before connect");
    this.transport.sendFeature(request);
  }

  // Broadcast this client's live cursor/viewport as a presence frame, stamped with its `userId`. Ephemeral, best-effort.
  sendCursor(cursor: WorldPoint, viewport?: WorldRect): void {
    if (!this.transport) throw new Error("sendCursor before connect");
    this.transport.sendPresence({
      userId: this.userId,
      cursor,
      ...(viewport ? { viewport } : {})
    });
  }

  // Subscribe to the live peer cursor set; returns an unsubscribe.
  onPeers(cb: (peers: PeerPresence[]) => void): Unsubscribe {
    this.peerListeners.add(cb);
    return () => this.peerListeners.delete(cb);
  }

  // The live (non-expired) peer cursors. Expires stale peers lazily on read.
  get peerCursors(): PeerPresence[] {
    this.peers.expire();
    return this.peers.list();
  }

  // Subscribe to optimistic scene updates. The second arg is the `(object,field)` keys whose preview
  // SETTLED with this update (an ack/reject released the last unacked write); the shell clears its
  // optimistic preview off these, not a value compare. Returns an unsubscribe.
  onScene(cb: (scene: ObjectScene, settledKeys: string[]) => void): Unsubscribe {
    this.sceneListeners.add(cb);
    return () => this.sceneListeners.delete(cb);
  }

  // Subscribe to remote applied patches (already folded into the scene).
  onPatch(cb: (patch: PatchMessage) => void): Unsubscribe {
    this.patchListeners.add(cb);
    return () => this.patchListeners.delete(cb);
  }

  onFeature(cb: (response: FeatureResponse) => void): Unsubscribe {
    this.featureListeners.add(cb);
    return () => this.featureListeners.delete(cb);
  }

  // Subscribe to connectivity changes (online/offline) for the shell banner.
  onStatus(cb: (status: ConnectionStatus) => void): Unsubscribe {
    this.statusListeners.add(cb);
    return () => this.statusListeners.delete(cb);
  }

  get connectionStatus(): ConnectionStatus {
    return this.transport?.connectionStatus ?? "offline";
  }

  async listCanvases(): Promise<CanvasSummary[]> {
    const data = await httpJson<{ canvases: CanvasSummary[] }>(this.apiBase(), "/api/canvases");
    return data.canvases;
  }

  async createCanvas(title?: string): Promise<CanvasSummary> {
    const data = await httpJson<{ canvas: CanvasSummary }>(this.apiBase(), "/api/canvases", {
      method: "POST",
      body: JSON.stringify({ title })
    });
    return data.canvas;
  }

  async deleteCanvas(canvasId: string): Promise<void> {
    await httpJson(this.apiBase(), `/api/canvases/${encodeURIComponent(canvasId)}`, { method: "DELETE" });
  }

  // Switch canvas: tear down the current session and reconnect to `canvasId`, re-subscribing the SAME
  // window. A fresh outbox avoids replaying the previous canvas's ops. Resolves with the new welcome snapshot.
  async switchCanvas(canvasId: string, outbox?: OutboxStore): Promise<ObjectScene> {
    const window = this.currentWindow;
    this.teardown();
    this.outbox = outbox ?? new InMemoryOutboxStore();
    return this.connect(canvasId, window ? { canvasId, bbox: window } : undefined);
  }

  private apiBase(): string {
    // The WS base shares its host with the HTTP API; map ws(s):// -> http(s)://.
    return this.url.replace(/^ws/, "http").replace(/\/+$/, "");
  }

  // Flush any buffered coalesced frame immediately (e.g. on gesture end).
  flush(): void {
    this.engine?.flush();
  }

  // Re-request the authoritative snapshot on the open socket; the server replies with a fresh `welcome`
  // the attached engine reconciles. No-op while offline.
  resync(): void {
    this.transport?.resume();
  }

  // Close the socket and drop all subscriptions.
  close(): void {
    this.engine?.flush();
    this.teardown();
  }

  // Detach engine/listeners and close the socket, leaving the client reusable.
  private teardown(): void {
    if (this.viewportTimer != null) {
      this.clearTimer(this.viewportTimer);
      this.viewportTimer = null;
    }
    this.detachEngine?.();
    this.offEnginePatch?.();
    this.offTransportPatch?.();
    this.offTransportFeature?.();
    this.offTransportStatus?.();
    this.offTransportPresence?.();
    this.detachEngine = null;
    this.offEnginePatch = null;
    this.offTransportPatch = null;
    this.offTransportFeature = null;
    this.offTransportStatus = null;
    this.offTransportPresence = null;
    this.transport?.close();
    this.transport = null;
    this.engine = null;
    this.peers = this.newPeerRegistry();
    this.emitPeers();
  }

  // Ingest one inbound presence frame and re-emit the peer set if it changed. Each frame also expires
  // stale peers, so a quiet peer drops the next time ANY peer moves — no background sweep timer is needed.
  private ingestPresence(payload: unknown): void {
    const added = this.peers.ingest(payload);
    const expired = this.peers.expire();
    if (added || expired) this.emitPeers();
  }

  private emitScene(scene: ObjectScene, settledKeys: string[]): void {
    for (const cb of this.sceneListeners) cb(scene, settledKeys);
  }

  private emitPatch(patch: PatchMessage): void {
    for (const cb of this.patchListeners) cb(patch);
  }

  private emitFeature(frame: FeatureServerMessage): void {
    for (const cb of this.featureListeners) cb(frame.response);
  }

  private emitStatus(status: ConnectionStatus): void {
    for (const cb of this.statusListeners) cb(status);
  }

  private emitPeers(): void {
    const peers = this.peers.list();
    for (const cb of this.peerListeners) cb(peers);
  }
}

// Mirrors the server `CanvasSummary` (camelCase serde); `GET /api/canvases` returns `{ canvases: CanvasSummary[] }`.
export type CanvasSummary = {
  id: string;
  title: string;
  updatedAt: string;
};

async function httpJson<T>(base: string, path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`${base}${path}`, {
    headers: { "content-type": "application/json" },
    ...init
  });
  if (!response.ok) {
    const error = await response.json().catch(() => ({ message: response.statusText }));
    throw new Error(error.message || response.statusText);
  }
  return (await response.json()) as T;
}
