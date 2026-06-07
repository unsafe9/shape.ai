// Transport-backed scene data layer (MG5.3).
//
// This is the seam that begins retiring the HTTP `api.ts` data path: instead of
// `fetchScene` / `saveScenePatch` over REST, the shell loads and mutates the
// scene through the WS transport client. It composes the existing pieces —
// `WsTransport` (the wire), `SyncEngine` (optimistic apply + durable outbox +
// coalescing + reconnect reconcile), and an `OutboxStore` — into one handle the
// shell can drive in place of the REST calls.
//
// Routing rules, mirroring the semantics the Svelte shell already relies on:
//
//   - Renderer ops (`RenderScenePatch`) and shell CRUD both flow through
//     `engine.author()`: optimistic local apply, persisted to the outbox, sent
//     as an `ops` envelope, dropped on the server ack. Shell CRUD arrives as a
//     `ScenePatch` (the app-patch shape) and is mapped to the equivalent
//     `RenderScenePatch` op(s) before authoring — there is no separate
//     bulk-ScenePatch apply on the wire (scene-core deferred that; see CLAUDE.md
//     phase-1 notes), so we decompose into the existing ops the server accepts.
//
//   - Selection-only changes do NOT bump the document revision: a selection move
//     is presence, not a document op. `saveSelection` applies the selection to
//     the local scene and broadcasts it as a best-effort presence frame; it
//     never enters the outbox and never produces an `ops` frame.
//
//   - LWW / stale-snapshot guards live in `SyncEngine.reconcileSnapshot`, which
//     rebases on the freshest `welcome` and replays the unacked outbox on top.
//
// The shell consumes `scene` (current optimistic scene), `onScene` (reactive
// updates), and `onPatch` (remote applied patches, for incremental renderer
// feeds) in place of `fetchScene` / `saveScenePatch`.

import type { Bounds, Scene, ScenePatch, SceneSelection } from "../../shared/schema";
import type { RenderScenePatch } from "../../shared/renderPatch";
import type { WorldPoint, WorldRect } from "../../shared/renderScene";
import { WsTransport, type ConnectionStatus, type ReconnectOptions, type WebSocketFactory } from "./wsTransport";
import { SyncEngine } from "./syncEngine";
import { InMemoryOutboxStore, type OpId, type OutboxStore } from "./outbox";
import { PeerRegistry, type PeerPresence } from "./peers";
import type { Bbox, PatchMessage, Region } from "./transport";

export type SceneClientOptions = {
  /** Wire base, e.g. `ws://127.0.0.1:8787`; the transport appends `/ws`. */
  url: string;
  /** Authoring identity stamped into every opId.clientId. */
  clientId: string;
  /**
   * Stable user identity (MG6.3). Sent in `hello.userId` and stamped on every
   * presence frame so the server self-skips this client's ops/presence and peers
   * can be surfaced by userId. Defaults to {@link SceneClientOptions.clientId}.
   */
  userId?: string;
  /** Durable outbox; defaults to an in-memory store (non-durable fallback). */
  outbox?: OutboxStore;
  /** Socket factory; tests inject a mock, prod uses the browser `WebSocket`. */
  createSocket?: WebSocketFactory;
  /** Coalescing window in ms; forwarded to the engine. */
  coalesceMs?: number;
  /** Clock source for envelope `ts`; injected for deterministic tests. */
  now?: () => string;
  /** Timer hooks; injected so tests can drive coalescing without real time. */
  setTimer?: (fn: () => void, ms: number) => unknown;
  clearTimer?: (handle: unknown) => void;
  /** Reconnect backoff config; forwarded to the transport (MG8.4). */
  reconnect?: ReconnectOptions;
  /** Random source for reconnect jitter; forwarded to the transport. */
  random?: () => number;
  /**
   * Window margin factor (MG9.4). The viewport bbox is grown by this fraction of
   * its width/height on each side before it becomes the subscribed region, so a
   * small pan/zoom does not immediately re-subscribe. Default {@link DEFAULT_VIEWPORT_MARGIN}.
   */
  viewportMargin?: number;
  /**
   * Debounce in ms for {@link SceneClient.setViewport} re-subscribes (MG9.4).
   * Default {@link DEFAULT_VIEWPORT_DEBOUNCE_MS}.
   */
  viewportDebounceMs?: number;
  /** Stale-peer cursor TTL in ms (MG6.2). Default {@link DEFAULT_PEER_TTL_MS}. */
  peerTtlMs?: number;
  /** Wall-clock source for peer freshness/expiry; injected for tests. */
  nowMs?: () => number;
};

export type Unsubscribe = () => void;

/** A camera-derived viewport in WORLD coordinates (before the window margin). */
export type Viewport = Bbox;

/** Default fraction the viewport is grown on each side to form the window. */
export const DEFAULT_VIEWPORT_MARGIN = 0.5;
/** Default debounce for viewport-driven re-subscribes. */
export const DEFAULT_VIEWPORT_DEBOUNCE_MS = 200;

/**
 * Grow a viewport bbox by `margin` of its size on each side. This is the
 * data-layer WINDOW the client subscribes to — distinct from renderer culling.
 *
 * Data-layer windowing (this) controls which objects the client HOLDS at all:
 * the server only ships objects inside the window, the engine only stores those,
 * and the outbox/optimistic scene only ever contains them. Renderer culling
 * (the Rust core's LOD/visibility) decides which of the HELD objects to draw
 * each frame. The margin here keeps a ring of off-screen-but-nearby objects
 * loaded so a pan reveals them instantly without a round-trip; culling then
 * trims that ring down to the actually-visible pixels. The two are independent
 * knobs: a generous window with aggressive culling is the normal large-canvas
 * configuration.
 */
export function windowFromViewport(viewport: Viewport, margin: number): Bbox {
  const padX = viewport.width * margin;
  const padY = viewport.height * margin;
  return {
    x: viewport.x - padX,
    y: viewport.y - padY,
    width: viewport.width + padX * 2,
    height: viewport.height + padY * 2
  };
}

/** True when two bboxes are equal enough that a re-subscribe would be a no-op. */
function bboxEquals(a: Bbox | undefined, b: Bbox | undefined): boolean {
  if (!a || !b) return a === b;
  return a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height;
}

/**
 * A transport-backed scene store. Open it with {@link connect}; author renderer
 * ops with {@link applyRenderPatch} and shell CRUD with {@link applyScenePatch};
 * move the selection (presence-only) with {@link saveSelection}.
 */
/** Default window after which a silent peer's cursor is expired. */
export const DEFAULT_PEER_TTL_MS = 10_000;

export class SceneClient {
  private readonly url: string;
  private readonly clientId: string;
  /** Stable user identity (MG6.3); sent in hello + stamped on presence frames. */
  private readonly userId: string;
  /** Re-pointed on switchCanvas so the new canvas starts with a clean outbox. */
  private outbox: OutboxStore;
  private readonly createSocket: SceneClientOptions["createSocket"];
  private readonly coalesceMs?: number;
  private readonly now: () => string;
  private readonly setTimer: (fn: () => void, ms: number) => unknown;
  private readonly clearTimer: (handle: unknown) => void;
  private readonly reconnect?: ReconnectOptions;
  private readonly random?: () => number;
  private readonly viewportMargin: number;
  private readonly viewportDebounceMs: number;

  private transport: WsTransport | null = null;
  private engine: SyncEngine | null = null;
  private detachEngine: Unsubscribe | null = null;
  private canvasId: string | null = null;

  /** The window bbox currently subscribed (undefined = whole canvas). */
  private window: Bbox | undefined;
  /** Debounce handle for the pending viewport-driven re-subscribe. */
  private viewportTimer: unknown = null;

  private readonly sceneListeners = new Set<(scene: Scene) => void>();
  private readonly patchListeners = new Set<(patch: PatchMessage) => void>();
  private readonly statusListeners = new Set<(status: ConnectionStatus) => void>();
  /** Peer cursor registry (MG6.2) + its subscribers; rebuilt per connection. */
  private peers: PeerRegistry;
  private readonly peerListeners = new Set<(peers: PeerPresence[]) => void>();
  private readonly peerTtlMs: number;
  private readonly nowMs: () => number;
  private offEnginePatch: Unsubscribe | null = null;
  private offTransportPatch: Unsubscribe | null = null;
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
    this.viewportMargin = opts.viewportMargin ?? DEFAULT_VIEWPORT_MARGIN;
    this.viewportDebounceMs = opts.viewportDebounceMs ?? DEFAULT_VIEWPORT_DEBOUNCE_MS;
    this.peerTtlMs = opts.peerTtlMs ?? DEFAULT_PEER_TTL_MS;
    this.nowMs = opts.nowMs ?? (() => Date.now());
    this.peers = this.newPeerRegistry();
  }

  /** A fresh peer registry seeded with this client's self-skip identity. */
  private newPeerRegistry(): PeerRegistry {
    return new PeerRegistry({ selfUserId: this.userId, ttlMs: this.peerTtlMs, now: this.nowMs });
  }

  /**
   * Open the session: connect the transport, send `hello`, and resolve with the
   * welcome `Scene` snapshot (a fresh empty scene for a new canvas). The engine
   * is created from the welcome scene and attached to the socket so subsequent
   * acks/rejects/patches/reconnect-welcomes are reconciled automatically.
   */
  async connect(canvasId: string, region?: Region): Promise<Scene> {
    if (this.transport) throw new Error("SceneClient already connected");
    this.canvasId = canvasId;
    this.window = region?.bbox;
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

    const welcome = await transport.connect(canvasId, this.regionFor(this.window));

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

    // Wire ack/rejected -> outbox drop, remote patch -> discard path, every
    // welcome (initial + reconnect + region resnapshot) -> reconcile + replay.
    // A region resnapshot welcome is region-filtered, so reconcileSnapshot
    // (which adopts the snapshot as authoritative) loads objects newly in-window
    // and evicts those now out-of-window — the windowed replica reconcile (MG9.4).
    this.detachEngine = transport.attachEngine(engine);

    // Fan the engine's optimistic scene out to shell subscribers.
    this.offEnginePatch = engine.onScene((scene) => this.emitScene(scene));
    // Forward remote applied patches so the shell can feed the renderer
    // incrementally (the engine already applied them to the optimistic scene).
    this.offTransportPatch = transport.onPatch((patch) => this.emitPatch(patch));
    // Surface connectivity so the shell can show an offline banner (MG8.4).
    this.offTransportStatus = transport.onStatus((status) => this.emitStatus(status));
    // MG6.2: ingest peer presence (cursor/viewport) into the registry and fan the
    // live peer set out to overlay subscribers. The server self-skips this
    // client, so every presence frame here is a peer's.
    this.offTransportPresence = transport.onPresence((frame) => this.ingestPresence(frame.payload));

    return welcome.scene;
  }

  /** Build a `Region` for the current canvas from an optional window bbox. */
  private regionFor(bbox: Bbox | undefined): Region | undefined {
    if (!this.canvasId) return undefined;
    return bbox ? { canvasId: this.canvasId, bbox } : undefined;
  }

  /**
   * Windowed replica (MG9.4): re-aim the subscription to the bbox derived from a
   * camera viewport plus the window margin, debounced so a continuous pan/zoom
   * does not spam re-subscribes. The server replies with a region-filtered
   * welcome (the resnapshot); the engine reconciles it — loading objects that
   * entered the window and evicting those that left. A no-op (the window did not
   * change) is skipped. Pass a viewport that already covers the whole canvas (or
   * use {@link subscribeWholeCanvas}) to drop the window.
   */
  setViewport(viewport: Viewport): void {
    const next = windowFromViewport(viewport, this.viewportMargin);
    if (this.viewportTimer != null) this.clearTimer(this.viewportTimer);
    this.viewportTimer = this.setTimer(() => {
      this.viewportTimer = null;
      this.applyWindow(next);
    }, this.viewportDebounceMs);
  }

  /** Immediately re-aim the window to `bbox` (no debounce); for tests/programmatic moves. */
  subscribeRegion(bbox: Bbox): void {
    if (this.viewportTimer != null) {
      this.clearTimer(this.viewportTimer);
      this.viewportTimer = null;
    }
    this.applyWindow(bbox);
  }

  /** Drop the window: re-subscribe to the whole canvas (no bbox). */
  subscribeWholeCanvas(): void {
    if (this.viewportTimer != null) {
      this.clearTimer(this.viewportTimer);
      this.viewportTimer = null;
    }
    if (this.window === undefined) return;
    this.window = undefined;
    if (this.canvasId) this.transport?.subscribe({ canvasId: this.canvasId });
  }

  private applyWindow(bbox: Bbox): void {
    if (bboxEquals(this.window, bbox)) return;
    this.window = bbox;
    if (this.canvasId) this.transport?.subscribe({ canvasId: this.canvasId, bbox });
  }

  /** The window bbox currently subscribed, or null for whole-canvas. */
  get currentWindow(): Bbox | null {
    return this.window ?? null;
  }

  /** The current optimistic scene, or null before {@link connect}. */
  get scene(): Scene | null {
    return this.engine?.getScene() ?? null;
  }

  /**
   * Author a renderer op: optimistic local apply + outbox + coalesced send.
   * Returns the rejecting-core errors (empty on success) and the minted opId.
   */
  async applyRenderPatch(patch: RenderScenePatch): Promise<{ errors: string[]; opId?: OpId }> {
    if (!this.engine) throw new Error("applyRenderPatch before connect");
    return this.engine.author(patch);
  }

  /**
   * Apply a shell CRUD `ScenePatch` by decomposing it into the equivalent
   * renderer op(s) and authoring each through the engine. A selection field on
   * the patch is applied as presence (no document op); the document mutations
   * (groups/nodes/edges/translateGroups/removes) become `ops` envelopes.
   * Returns the concatenated rejecting-core errors across the decomposed ops.
   */
  async applyScenePatch(patch: ScenePatch): Promise<{ errors: string[] }> {
    if (!this.engine) throw new Error("applyScenePatch before connect");
    const ops = scenePatchToRenderOps(patch);
    const errors: string[] = [];
    for (const op of ops) {
      const result = await this.engine.author(op);
      errors.push(...result.errors);
    }
    // A selection carried alongside document changes rides as presence (it does
    // not bump the revision); a selection-only patch produced no ops above and
    // is handled entirely here.
    if (patch.selection) this.saveSelection(patch.selection);
    return { errors };
  }

  /**
   * Move the selection WITHOUT a document op: broadcast it as a best-effort
   * presence frame only. Selection is ephemeral, so it never enters the outbox,
   * never produces an `ops` frame, and never bumps the document revision. The
   * shell holds the live selection as its own state; the canvas document store
   * stays untouched.
   */
  saveSelection(selection: SceneSelection): void {
    if (!this.transport) throw new Error("saveSelection before connect");
    this.transport.sendPresence({ kind: "select", selection });
  }

  /**
   * Broadcast this client's live cursor/viewport as a presence frame (MG6.2),
   * stamped with this client's `userId` so peers can lane it. Ephemeral and
   * best-effort: dropped while offline, never enters the outbox. The shell calls
   * this on (throttled) pointer move; coordinates are WORLD space so a peer with
   * a different camera frames the same point.
   */
  sendCursor(cursor: WorldPoint, viewport?: WorldRect): void {
    if (!this.transport) throw new Error("sendCursor before connect");
    this.transport.sendPresence({
      userId: this.userId,
      cursor,
      ...(viewport ? { viewport } : {})
    });
  }

  /** Subscribe to the live peer cursor set (MG6.2); returns an unsubscribe. */
  onPeers(cb: (peers: PeerPresence[]) => void): Unsubscribe {
    this.peerListeners.add(cb);
    return () => this.peerListeners.delete(cb);
  }

  /** The live (non-expired) peer cursors. Expires stale peers lazily on read. */
  get peerCursors(): PeerPresence[] {
    this.peers.expire();
    return this.peers.list();
  }

  /** Subscribe to optimistic scene updates; returns an unsubscribe. */
  onScene(cb: (scene: Scene) => void): Unsubscribe {
    this.sceneListeners.add(cb);
    return () => this.sceneListeners.delete(cb);
  }

  /** Subscribe to remote applied patches (already folded into the scene). */
  onPatch(cb: (patch: PatchMessage) => void): Unsubscribe {
    this.patchListeners.add(cb);
    return () => this.patchListeners.delete(cb);
  }

  /** Subscribe to connectivity changes (online/offline) for the shell banner. */
  onStatus(cb: (status: ConnectionStatus) => void): Unsubscribe {
    this.statusListeners.add(cb);
    return () => this.statusListeners.delete(cb);
  }

  /** The current connectivity status (offline before connect). */
  get connectionStatus(): ConnectionStatus {
    return this.transport?.connectionStatus ?? "offline";
  }

  // --- multi-canvas (MG9.2) ------------------------------------------------

  /** List every canvas from the durable index (`GET /api/canvases`). */
  async listCanvases(): Promise<CanvasSummary[]> {
    const data = await httpJson<{ canvases: CanvasSummary[] }>(this.apiBase(), "/api/canvases");
    return data.canvases;
  }

  /** Create a canvas and return its summary (`POST /api/canvases`). */
  async createCanvas(title?: string): Promise<CanvasSummary> {
    const data = await httpJson<{ canvas: CanvasSummary }>(this.apiBase(), "/api/canvases", {
      method: "POST",
      body: JSON.stringify({ title })
    });
    return data.canvas;
  }

  /** Delete a canvas (`DELETE /api/canvases/:id`). */
  async deleteCanvas(canvasId: string): Promise<void> {
    await httpJson(this.apiBase(), `/api/canvases/${encodeURIComponent(canvasId)}`, { method: "DELETE" });
  }

  /**
   * Switch to a different canvas (MG9.2): tear down the current session and
   * reconnect to `canvasId`, re-subscribing the SAME window so the new canvas
   * loads region-filtered. The unacked outbox is per-client, not per-canvas, so
   * a fresh outbox is created for the new canvas to avoid replaying the previous
   * canvas's ops against it. Resolves with the new canvas's welcome snapshot.
   */
  async switchCanvas(canvasId: string, outbox?: OutboxStore): Promise<Scene> {
    const window = this.window;
    this.teardown();
    // Re-point the outbox: ops are keyed by (clientId, localSeq) for THIS canvas,
    // so a switch must not replay the old canvas's outbox into the new one.
    this.outbox = outbox ?? new InMemoryOutboxStore();
    return this.connect(canvasId, window ? { canvasId, bbox: window } : undefined);
  }

  private apiBase(): string {
    // The WS base shares its host with the HTTP API; map ws(s):// -> http(s)://.
    return this.url.replace(/^ws/, "http").replace(/\/+$/, "");
  }

  /** Flush any buffered coalesced frame immediately (e.g. on gesture end). */
  flush(): void {
    this.engine?.flush();
  }

  /** Close the socket and drop all subscriptions. */
  close(): void {
    this.engine?.flush();
    this.teardown();
  }

  /** Detach engine/listeners and close the socket, leaving the client reusable. */
  private teardown(): void {
    if (this.viewportTimer != null) {
      this.clearTimer(this.viewportTimer);
      this.viewportTimer = null;
    }
    this.detachEngine?.();
    this.offEnginePatch?.();
    this.offTransportPatch?.();
    this.offTransportStatus?.();
    this.offTransportPresence?.();
    this.detachEngine = null;
    this.offEnginePatch = null;
    this.offTransportPatch = null;
    this.offTransportStatus = null;
    this.offTransportPresence = null;
    this.transport?.close();
    this.transport = null;
    this.engine = null;
    // Peers are per-connection: a switchCanvas/close drops every peer cursor so a
    // new canvas (or reconnect) starts with an empty overlay.
    this.peers = this.newPeerRegistry();
    this.emitPeers();
  }

  /**
   * Ingest one inbound presence frame and re-emit the peer set if it changed.
   * Each frame also expires stale peers, so a quiet peer drops the next time
   * ANY peer moves — no background sweep timer is needed (which would otherwise
   * leak a recurring timer into the host event loop).
   */
  private ingestPresence(payload: unknown): void {
    const added = this.peers.ingest(payload);
    const expired = this.peers.expire();
    if (added || expired) this.emitPeers();
  }

  private emitScene(scene: Scene): void {
    for (const cb of this.sceneListeners) cb(scene);
  }

  private emitPatch(patch: PatchMessage): void {
    for (const cb of this.patchListeners) cb(patch);
  }

  private emitStatus(status: ConnectionStatus): void {
    for (const cb of this.statusListeners) cb(status);
  }

  private emitPeers(): void {
    const peers = this.peers.list();
    for (const cb of this.peerListeners) cb(peers);
  }
}

/**
 * The canvas list-item shape mirroring the server `CanvasSummary` (camelCase
 * serde): `GET /api/canvases` returns `{ canvases: CanvasSummary[] }`.
 */
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

/**
 * Decompose a shell `ScenePatch` (the app-patch document shape) into the
 * equivalent `RenderScenePatch` ops the server already accepts. The order is
 * additive-then-removals so a created object is present before a later op
 * references it; group translations come before per-node moves; the optional
 * `selection` field is intentionally ignored here (it is presence, not a
 * document op — see {@link SceneClient.saveSelection}).
 *
 * Each app object carries its full record, so a create vs. update is the same
 * wire op family: an existing renderer op replaces the whole object. The server
 * upserts by id, so re-sending a full node/group/edge is the create-or-update
 * primitive without a separate update op kind.
 */
export function scenePatchToRenderOps(patch: ScenePatch): RenderScenePatch[] {
  const ops: RenderScenePatch[] = [];

  for (const group of patch.groups ?? []) {
    ops.push({
      kind: "create-group",
      group: {
        id: group.id,
        title: group.title,
        summary: group.summary,
        bounds: group.bounds,
        tagIds: group.tagIds,
        zIndex: group.zIndex,
        styleKey: "default"
      }
    });
  }

  for (const node of patch.nodes ?? []) {
    ops.push({
      kind: "create-card",
      card: {
        id: node.id,
        groupId: node.groupId,
        title: node.title,
        summary: node.summary,
        detail: node.detail,
        status: node.status,
        type: node.type,
        bounds: { x: node.position.x, y: node.position.y, width: node.size.width, height: node.size.height },
        zIndex: node.zIndex,
        styleKey: node.type,
        accessibilityLabel: `${node.type} ${node.title}`
      }
    });
  }

  for (const edge of patch.edges ?? []) {
    ops.push({
      kind: "create-edge",
      groupId: edge.groupId,
      source: edge.source,
      target: edge.target,
      edgeId: edge.id,
      label: edge.label
    });
  }

  for (const movement of patch.translateGroups ?? []) {
    ops.push({ kind: "move-group", id: movement.groupId, delta: { x: movement.dx, y: movement.dy } });
  }

  for (const id of patch.removeEdgeIds ?? []) ops.push({ kind: "delete-edge", id });
  for (const id of patch.removeNodeIds ?? []) ops.push({ kind: "delete-card", id });
  for (const id of patch.removeGroupIds ?? []) ops.push({ kind: "delete-group", id });

  return ops;
}
