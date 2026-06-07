// Windowed replica + canvas switch + reconnect tests (MG9.4, MG9.2, MG8.4).
//
// These drive `SceneClient` / `WsTransport` over a MockWebSocket (the same
// double the ws-transport / scene-client suites use) plus injected manual timers
// and a deterministic random source, so the windowing re-subscribe, the
// reconnect backoff schedule, and the offline buffering are all deterministic.
//
// They prove the Phase-4 data-layer contract:
//
//   - DATA-LAYER WINDOWING (MG9.4): a region subscribe yields only the objects
//     the server ships inside the window; a camera move re-subscribes with a new
//     (margin-grown) region; the resnapshot welcome loads entered objects and
//     evicts exited ones. This is distinct from renderer culling — the client
//     HOLDS only in-window objects; the renderer separately decides which held
//     objects to draw.
//   - RECONNECT (MG8.4): the backoff schedule grows per attempt and stays inside
//     the jitter bounds; an offline burst of ops is buffered (optimistic apply +
//     outbox) and replayed on the reconnect welcome so state converges.
//   - CANVAS SWITCH (MG9.2): switchCanvas reconnects and re-subscribes the same
//     window for the new canvasId.

import { beforeEach, describe, expect, it } from "vitest";

import {
  SceneClient,
  windowFromViewport,
  DEFAULT_VIEWPORT_MARGIN,
  type CanvasSummary
} from "../src/client/lib/sceneClient";
import {
  WsTransport,
  backoffDelay,
  type ReconnectOptions,
  type WebSocketLike
} from "../src/client/lib/wsTransport";
import { InMemoryOutboxStore } from "../src/client/lib/outbox";
import type { ClientMessage, ServerMessage, WelcomeMessage } from "../src/client/lib/transport";
import type { RenderScenePatch } from "../src/shared/renderPatch";
import type { Bounds, Scene, SceneGroup } from "../src/shared/schema";

// ---------------------------------------------------------------------------
// Doubles.
// ---------------------------------------------------------------------------

class MockWebSocket implements WebSocketLike {
  onopen: ((ev: unknown) => void) | null = null;
  onclose: ((ev: unknown) => void) | null = null;
  onerror: ((ev: unknown) => void) | null = null;
  onmessage: ((ev: { data: unknown }) => void) | null = null;
  readonly sent: string[] = [];
  closed = false;
  constructor(readonly url: string) {}
  send(data: string): void {
    this.sent.push(data);
  }
  close(): void {
    this.closed = true;
    this.onclose?.({});
  }
  open(): void {
    this.onopen?.({});
  }
  emit(frame: ServerMessage): void {
    this.onmessage?.({ data: JSON.stringify(frame) });
  }
  sentMessages(): ClientMessage[] {
    return this.sent.map((s) => JSON.parse(s) as ClientMessage);
  }
  /** Simulate an unexpected drop (server/network closed the socket). */
  drop(): void {
    this.onclose?.({});
  }
}

/** A manual timer registry keyed by insertion: arm, then fire the oldest. */
function manualTimer() {
  type Pending = { id: number; fn: () => void };
  let nextId = 1;
  const pending: Pending[] = [];
  return {
    setTimer: (fn: () => void, _ms: number) => {
      const id = nextId++;
      pending.push({ id, fn });
      return id as unknown;
    },
    clearTimer: (h: unknown) => {
      const idx = pending.findIndex((p) => p.id === h);
      if (idx >= 0) pending.splice(idx, 1);
    },
    /** Fire the oldest armed timer. */
    fire: () => {
      const p = pending.shift();
      p?.fn();
    },
    /** Fire every armed timer (in arm order), draining the queue. */
    fireAll: () => {
      while (pending.length > 0) pending.shift()!.fn();
    },
    count: () => pending.length
  };
}

function emptyScene(sceneVersion = 0): Scene {
  return {
    version: 1,
    sceneVersion,
    groups: [],
    nodes: [],
    edges: [],
    tags: [],
    comments: [],
    artifacts: [],
    selection: { kind: "canvas" },
    updatedAt: "1970-01-01T00:00:00Z"
  } as Scene;
}

function group(id: string, bounds: Bounds, sceneVersion = 0): SceneGroup {
  return {
    id,
    parentGroupId: null,
    title: id,
    summary: "",
    bounds,
    tagIds: [],
    zIndex: 0,
    collapsed: false,
    createdAt: "1970-01-01T00:00:00Z",
    updatedAt: "1970-01-01T00:00:00Z"
  } as SceneGroup;
}

/** A region-filtered scene the server would ship for a window: only intersecting groups. */
function sceneWithGroups(groups: SceneGroup[], sceneVersion: number): Scene {
  return { ...emptyScene(sceneVersion), groups };
}

const welcomeFrame = (scene: Scene, seq: number): WelcomeMessage => ({
  type: "welcome",
  scene,
  seq,
  revision: seq
});

const createGroupOp = (id: string, bounds: Bounds): RenderScenePatch => ({
  kind: "create-group",
  group: { id, title: id, summary: "", bounds, tagIds: [], zIndex: 0, styleKey: "" }
});

let nowCounter = 0;
const fixedNow = () => `t${nowCounter++}`;

beforeEach(() => {
  nowCounter = 0;
});

type Booted = {
  client: SceneClient;
  outbox: InMemoryOutboxStore;
  timer: ReturnType<typeof manualTimer>;
  socket: () => MockWebSocket;
  scene: Scene;
};

/**
 * Boot a SceneClient with an injected timer + deterministic random. `sockets`
 * collects every socket the factory builds (one per connect/reconnect), so a
 * reconnect test can drive the latest socket. The initial connect resolves with
 * the supplied welcome scene.
 */
async function boot(opts: {
  initial?: Scene;
  seq?: number;
  region?: { bbox: Bounds };
  reconnect?: ReconnectOptions;
  random?: () => number;
} = {}): Promise<Booted & { sockets: MockWebSocket[] }> {
  const sockets: MockWebSocket[] = [];
  const outbox = new InMemoryOutboxStore();
  const timer = manualTimer();
  const client = new SceneClient({
    url: "ws://127.0.0.1:8787",
    clientId: "c1",
    outbox,
    now: fixedNow,
    setTimer: timer.setTimer,
    clearTimer: timer.clearTimer,
    ...(opts.reconnect ? { reconnect: opts.reconnect } : {}),
    ...(opts.random ? { random: opts.random } : {}),
    viewportMargin: 0, // tests pass pre-grown bboxes; keep the window == viewport
    viewportDebounceMs: 50,
    createSocket: (url) => {
      const s = new MockWebSocket(url);
      sockets.push(s);
      return s;
    }
  });
  const ready = client.connect("canvas-a", opts.region ? { canvasId: "canvas-a", bbox: opts.region.bbox } : undefined);
  sockets[0].open();
  sockets[0].emit(welcomeFrame(opts.initial ?? emptyScene(opts.seq ?? 0), opts.seq ?? 0));
  const scene = await ready;
  return { client, outbox, timer, socket: () => sockets[sockets.length - 1], scene, sockets };
}

function helloFrames(socket: MockWebSocket) {
  return socket.sentMessages().filter((m): m is Extract<ClientMessage, { type: "hello" }> => m.type === "hello");
}
function subscribeFrames(socket: MockWebSocket) {
  return socket
    .sentMessages()
    .filter((m): m is Extract<ClientMessage, { type: "subscribe" }> => m.type === "subscribe");
}
function opsFrames(socket: MockWebSocket) {
  return socket.sentMessages().filter((m): m is Extract<ClientMessage, { type: "ops" }> => m.type === "ops");
}

// ---------------------------------------------------------------------------
// MG9.4 — windowed replica.
// ---------------------------------------------------------------------------

describe("windowFromViewport (MG9.4)", () => {
  it("grows the viewport by the margin fraction on each side", () => {
    const win = windowFromViewport({ x: 0, y: 0, width: 100, height: 200 }, 0.5);
    // 0.5 * 100 = 50 per side on x; 0.5 * 200 = 100 per side on y.
    expect(win).toEqual({ x: -50, y: -100, width: 200, height: 400 });
  });

  it("has a sensible default margin", () => {
    expect(DEFAULT_VIEWPORT_MARGIN).toBeGreaterThan(0);
  });
});

describe("SceneClient windowed subscribe (MG9.4)", () => {
  it("seeds the connection window from the connect region (hello carries the bbox)", async () => {
    const bbox = { x: 0, y: 0, width: 500, height: 500 };
    const inWindow = group("g-in", { x: 10, y: 10, width: 100, height: 100 }, 1);
    const { socket, scene, client } = await boot({
      initial: sceneWithGroups([inWindow], 1),
      seq: 1,
      region: { bbox }
    });

    // hello carried the seed region.
    expect(helloFrames(socket())[0].region).toEqual({ canvasId: "canvas-a", bbox });
    // Only the in-window object is held (the server ships a region-filtered scene).
    expect(scene.groups.map((g) => g.id)).toEqual(["g-in"]);
    expect(client.currentWindow).toEqual(bbox);
  });

  it("re-subscribes with a new region on a camera move and reconciles entered/exited objects", async () => {
    // Window 1 holds g-left; window 2 (after the camera moved right) holds g-right.
    const left = group("g-left", { x: 0, y: 0, width: 100, height: 100 }, 1);
    const right = group("g-right", { x: 1000, y: 0, width: 100, height: 100 }, 2);
    const { client, timer, socket } = await boot({
      initial: sceneWithGroups([left], 1),
      seq: 1,
      region: { bbox: { x: -50, y: -50, width: 300, height: 300 } }
    });
    expect(client.scene?.groups.map((g) => g.id)).toEqual(["g-left"]);

    // Camera moves to a new viewport (margin 0 in this boot, so window == viewport).
    client.setViewport({ x: 950, y: -50, width: 300, height: 300 });
    expect(subscribeFrames(socket())).toHaveLength(0); // debounced, not yet sent
    timer.fire(); // fire the debounce

    const subs = subscribeFrames(socket());
    expect(subs).toHaveLength(1);
    expect(subs[0].region.bbox).toEqual({ x: 950, y: -50, width: 300, height: 300 });
    expect(client.currentWindow).toEqual({ x: 950, y: -50, width: 300, height: 300 });

    // The server replies with the region-filtered resnapshot for the new window:
    // g-left exited (evicted), g-right entered (loaded). seq is the TRUE revision.
    socket().emit(welcomeFrame(sceneWithGroups([right], 2), 2));
    await Promise.resolve();

    expect(client.scene?.groups.map((g) => g.id)).toEqual(["g-right"]);
  });

  it("coalesces a rapid pan into a single re-subscribe (debounce)", async () => {
    const { client, timer, socket } = await boot({
      region: { bbox: { x: 0, y: 0, width: 100, height: 100 } }
    });
    client.setViewport({ x: 10, y: 0, width: 100, height: 100 });
    client.setViewport({ x: 20, y: 0, width: 100, height: 100 });
    client.setViewport({ x: 30, y: 0, width: 100, height: 100 });
    // Only the last debounce timer survives; firing it sends one subscribe.
    timer.fireAll();
    const subs = subscribeFrames(socket());
    expect(subs).toHaveLength(1);
    expect(subs[0].region.bbox).toEqual({ x: 30, y: 0, width: 100, height: 100 });
  });

  it("does not re-subscribe when the window is unchanged", async () => {
    const bbox = { x: 0, y: 0, width: 100, height: 100 };
    const { client, timer, socket } = await boot({ region: { bbox } });
    client.subscribeRegion({ ...bbox });
    timer.fireAll();
    expect(subscribeFrames(socket())).toHaveLength(0);
  });

  it("subscribeWholeCanvas drops the window (subscribe with no bbox)", async () => {
    const { client, socket } = await boot({
      region: { bbox: { x: 0, y: 0, width: 100, height: 100 } }
    });
    client.subscribeWholeCanvas();
    const subs = subscribeFrames(socket());
    expect(subs).toHaveLength(1);
    expect(subs[0].region.bbox).toBeUndefined();
    expect(client.currentWindow).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// MG8.4 — reconnect: backoff schedule + offline buffering.
// ---------------------------------------------------------------------------

describe("backoffDelay schedule (MG8.4)", () => {
  const opts = { baseMs: 500, maxMs: 15_000, jitter: 0.3 };

  it("grows exponentially and stays within the jitter bounds per attempt", () => {
    // With random()==0 the jitter factor is 1 (the upper bound == the raw delay).
    const upper = (n: number) => backoffDelay(n, opts, () => 0);
    expect(upper(0)).toBe(500);
    expect(upper(1)).toBe(1000);
    expect(upper(2)).toBe(2000);
    expect(upper(3)).toBe(4000);
    // Monotonic increase until the cap.
    expect(upper(1)).toBeGreaterThan(upper(0));
    expect(upper(2)).toBeGreaterThan(upper(1));
  });

  it("caps at maxMs", () => {
    expect(backoffDelay(20, opts, () => 0)).toBe(opts.maxMs);
  });

  it("applies jitter inside [base*2^n * (1-jitter), base*2^n]", () => {
    // random()==1 gives the lower bound: delay * (1 - jitter).
    const lower = backoffDelay(2, opts, () => 1);
    const upper = backoffDelay(2, opts, () => 0);
    expect(upper).toBe(2000);
    expect(lower).toBe(Math.round(2000 * (1 - 0.3)));
    expect(lower).toBeLessThan(upper);
    expect(lower).toBeGreaterThanOrEqual(2000 * (1 - 0.3) - 1);
  });
});

describe("WsTransport reconnect (MG8.4)", () => {
  it("schedules a backed-off reconnect after an unexpected drop and grows the delay each attempt", async () => {
    const sockets: MockWebSocket[] = [];
    const timer = manualTimer();
    const delays: number[] = [];
    const transport = new WsTransport({
      url: "ws://127.0.0.1:8787",
      reconnect: { baseMs: 500, maxMs: 15_000, jitter: 0 },
      random: () => 0,
      setTimer: (fn, ms) => {
        delays.push(ms);
        return timer.setTimer(fn, ms);
      },
      clearTimer: timer.clearTimer,
      createSocket: (url) => {
        const s = new MockWebSocket(url);
        sockets.push(s);
        return s;
      }
    });

    const ready = transport.connect("c-ws");
    sockets[0].open();
    sockets[0].emit(welcomeFrame(emptyScene(0), 0));
    await ready;
    expect(transport.connectionStatus).toBe("online");

    // First unexpected drop -> offline + a reconnect armed at base.
    sockets[0].drop();
    expect(transport.connectionStatus).toBe("offline");
    expect(delays).toEqual([500]);

    // Fire the reconnect: a new socket is built but the server drops it again
    // before welcome -> the next backoff doubles.
    timer.fire();
    expect(sockets).toHaveLength(2);
    sockets[1].drop();
    expect(delays).toEqual([500, 1000]);

    // Fire again, this time the reconnect succeeds (welcome) -> online, backoff reset.
    timer.fire();
    expect(sockets).toHaveLength(3);
    sockets[2].open();
    sockets[2].emit(welcomeFrame(emptyScene(1), 1));
    expect(transport.connectionStatus).toBe("online");

    // A subsequent drop starts the schedule over from base.
    sockets[2].drop();
    expect(delays).toEqual([500, 1000, 500]);
  });

  it("does not reconnect after a deliberate close()", () => {
    const sockets: MockWebSocket[] = [];
    const timer = manualTimer();
    const transport = new WsTransport({
      url: "ws://127.0.0.1:8787",
      setTimer: timer.setTimer,
      clearTimer: timer.clearTimer,
      createSocket: (url) => {
        const s = new MockWebSocket(url);
        sockets.push(s);
        return s;
      }
    });
    const ready = transport.connect("c-ws");
    sockets[0].open();
    sockets[0].emit(welcomeFrame(emptyScene(0), 0));
    void ready;

    transport.close();
    expect(timer.count()).toBe(0); // no reconnect armed
    expect(transport.connectionStatus).toBe("offline");
  });
});

describe("SceneClient offline buffering + reconnect replay (MG8.4)", () => {
  it("buffers ops while offline (optimistic apply + outbox) and replays them on reconnect to converge", async () => {
    const { client, outbox, timer, socket, sockets } = await boot({ seq: 0, reconnect: { jitter: 0 }, random: () => 0 });
    const dead = socket();

    // Go offline: the live socket drops unexpectedly (arms the reconnect timer).
    dead.drop();
    expect(client.connectionStatus).toBe("offline");

    // Author two ops while offline: each applies locally and persists to the
    // outbox, but no ops frame leaves (no live socket). The coalesce flush is a
    // no-op offline; only the durable outbox holds them.
    await client.applyRenderPatch(createGroupOp("g1", { x: 0, y: 0, width: 100, height: 100 }));
    await client.applyRenderPatch(createGroupOp("g2", { x: 200, y: 0, width: 100, height: 100 }));
    client.flush(); // drain the engine coalesce buffer (no-op send while offline)
    expect(client.scene?.groups.map((g) => g.id)).toEqual(["g1", "g2"]);
    expect(await outbox.all()).toHaveLength(2);
    // Nothing was sent on the dead socket.
    expect(opsFrames(dead)).toHaveLength(0);

    // Reconnect fires: a fresh socket, welcome arrives with the server's (empty)
    // snapshot. The engine reconciles + replays the unacked outbox on top.
    timer.fireAll(); // fire the reconnect backoff timer
    expect(client.connectionStatus).toBe("offline"); // still offline until welcome
    const reconnected = sockets[sockets.length - 1];
    expect(reconnected).not.toBe(dead);
    reconnected.open();
    reconnected.emit(welcomeFrame(emptyScene(0), 0));
    await Promise.resolve();
    await Promise.resolve();

    expect(client.connectionStatus).toBe("online");
    // Optimistic ops survived the snapshot reconcile (replayed on top).
    expect(client.scene?.groups.map((g) => g.id)).toEqual(["g1", "g2"]);
    // Both were re-sent with their original opIds on the new socket.
    const replayed = opsFrames(reconnected).flatMap((f) => f.ops.map((e) => e.opId.localSeq));
    expect(replayed).toEqual([1, 2]);
    expect(await outbox.all()).toHaveLength(2); // still unacked until acks arrive

    // Acks drain the outbox -> fully converged.
    reconnected.emit({ type: "ack", opIds: [{ clientId: "c1", localSeq: 1 }, { clientId: "c1", localSeq: 2 }], seq: 2, revision: 2 });
    await Promise.resolve();
    expect(await outbox.all()).toHaveLength(0);
  });

  it("re-sends the seed region on the reconnect hello so the window is preserved", async () => {
    const bbox = { x: 0, y: 0, width: 400, height: 400 };
    const { client, timer, socket, sockets } = await boot({ region: { bbox }, reconnect: { jitter: 0 }, random: () => 0 });

    socket().drop();
    timer.fireAll(); // fire reconnect backoff
    const reconnected = sockets[sockets.length - 1];
    reconnected.open();
    // The reconnect hello carries the same window region.
    expect(helloFrames(reconnected)[0].region).toEqual({ canvasId: "canvas-a", bbox });
    reconnected.emit(welcomeFrame(emptyScene(0), 0));
    await Promise.resolve();
    expect(client.connectionStatus).toBe("online");
  });
});

// ---------------------------------------------------------------------------
// MG9.2 — multi-canvas switch.
// ---------------------------------------------------------------------------

describe("SceneClient canvas switch (MG9.2)", () => {
  it("listCanvases / createCanvas hit the REST API and decode the summary", async () => {
    const calls: { url: string; init?: RequestInit }[] = [];
    const original = globalThis.fetch;
    globalThis.fetch = (async (url: string, init?: RequestInit) => {
      calls.push({ url, init });
      if (init?.method === "POST") {
        return new Response(JSON.stringify({ canvas: { id: "c-new", title: "New", updatedAt: "t" } as CanvasSummary }), {
          status: 200,
          headers: { "content-type": "application/json" }
        });
      }
      return new Response(
        JSON.stringify({ canvases: [{ id: "canvas-a", title: "A", updatedAt: "t" }] as CanvasSummary[] }),
        { status: 200, headers: { "content-type": "application/json" } }
      );
    }) as typeof fetch;

    try {
      const { client } = await boot();
      const list = await client.listCanvases();
      expect(list.map((c) => c.id)).toEqual(["canvas-a"]);
      // ws://host -> http://host for the REST base.
      expect(calls[0].url).toBe("http://127.0.0.1:8787/api/canvases");

      const created = await client.createCanvas("New");
      expect(created.id).toBe("c-new");
      expect(calls[1].init?.method).toBe("POST");
    } finally {
      globalThis.fetch = original;
    }
  });

  it("switchCanvas reconnects and re-subscribes the same window for the new canvasId", async () => {
    const bbox = { x: 0, y: 0, width: 400, height: 400 };
    const { client, socket, sockets } = await boot({
      initial: sceneWithGroups([group("g-a", { x: 0, y: 0, width: 50, height: 50 }, 1)], 1),
      seq: 1,
      region: { bbox }
    });
    expect(client.scene?.groups.map((g) => g.id)).toEqual(["g-a"]);
    const oldSocket = socket();

    // Switch to canvas-b: the old socket is closed, a new one opens.
    const switched = client.switchCanvas("canvas-b");
    const newSocket = sockets[sockets.length - 1];
    expect(newSocket).not.toBe(oldSocket); // a fresh socket for the new canvas
    expect(oldSocket.closed).toBe(true); // the old session was torn down
    newSocket.open();
    const sceneB = sceneWithGroups([group("g-b", { x: 10, y: 10, width: 50, height: 50 }, 7)], 7);
    newSocket.emit(welcomeFrame(sceneB, 7));
    const scene = await switched;

    // The new canvas loaded; the hello re-aimed the SAME window at canvas-b.
    expect(scene.groups.map((g) => g.id)).toEqual(["g-b"]);
    expect(client.scene?.groups.map((g) => g.id)).toEqual(["g-b"]);
    const hello = helloFrames(newSocket)[0];
    expect(hello.canvasId).toBe("canvas-b");
    expect(hello.region).toEqual({ canvasId: "canvas-b", bbox });
  });

  it("switchCanvas starts the new canvas with a clean outbox (no cross-canvas replay)", async () => {
    const { client, outbox, timer, socket, sockets } = await boot({ seq: 0 });

    // Author an op on canvas-a but never ack it; it sits in the original outbox.
    await client.applyRenderPatch(createGroupOp("g-a", { x: 0, y: 0, width: 50, height: 50 }));
    timer.fireAll();
    expect(await outbox.all()).toHaveLength(1);

    const switched = client.switchCanvas("canvas-b");
    const newSocket = sockets[sockets.length - 1];
    newSocket.open();
    newSocket.emit(welcomeFrame(emptyScene(0), 0));
    await switched;

    // The new canvas did NOT replay canvas-a's op (fresh outbox).
    expect(opsFrames(newSocket)).toHaveLength(0);
    // The original outbox instance still holds the canvas-a op (it was not touched).
    expect(await outbox.all()).toHaveLength(1);
  });
});
