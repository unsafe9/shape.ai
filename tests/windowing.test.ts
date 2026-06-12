import { beforeAll, beforeEach, describe, expect, it } from "vitest";

import {
  SceneClient,
  type CanvasSummary
} from "../platforms/web/runtime/sceneClient";
import {
  WsTransport,
  backoffDelay,
  type ReconnectOptions,
  type WebSocketLike
} from "../platforms/web/runtime/wsTransport";
import { InMemoryOutboxStore } from "../platforms/web/runtime/outbox";
import { ensureSceneCore } from "../platforms/web/bridge/sceneCoreWasm";
import type { Bbox, ClientMessage, ServerMessage, WelcomeMessage } from "../platforms/web/runtime/transport";
import {
  emptyObjectScene,
  translateTransform,
  type Object as SceneObject,
  type ObjectOp,
  type ObjectScene
} from "../platforms/web/shared/object";

beforeAll(async () => {
  await ensureSceneCore();
});

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
  drop(): void {
    this.onclose?.({});
  }
}

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
    fire: () => {
      const p = pending.shift();
      p?.fn();
    },
    fireAll: () => {
      while (pending.length > 0) pending.shift()!.fn();
    },
    count: () => pending.length
  };
}

function objectAt(id: string, x: number, y: number): SceneObject {
  return {
    id,
    order: "a0",
    transform: translateTransform(x, y),
    geometry: { d: "M 0 0 L 50 0 L 50 50 L 0 50 Z", fillRule: "nonZero" }
  };
}

function sceneWithObjects(objects: SceneObject[], sceneVersion: number): ObjectScene {
  const scene = emptyObjectScene();
  scene.sceneVersion = sceneVersion;
  scene.objects = objects;
  return scene;
}

const welcomeFrame = (scene: ObjectScene, seq: number): WelcomeMessage => ({
  type: "welcome",
  scene,
  seq,
  revision: seq
});

const insertObjectOp = (id: string, x: number, y: number): ObjectOp => ({
  kind: "insert-object",
  object: objectAt(id, x, y)
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
  scene: ObjectScene;
  sockets: MockWebSocket[];
};

async function boot(
  opts: {
    initial?: ObjectScene;
    seq?: number;
    region?: { bbox: Bbox };
    reconnect?: ReconnectOptions;
    random?: () => number;
  } = {}
): Promise<Booted> {
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
    viewportMargin: 0,
    viewportDebounceMs: 50,
    createSocket: (url) => {
      const s = new MockWebSocket(url);
      sockets.push(s);
      return s;
    }
  });
  const ready = client.connect("canvas-a", opts.region ? { canvasId: "canvas-a", bbox: opts.region.bbox } : undefined);
  sockets[0].open();
  sockets[0].emit(welcomeFrame(opts.initial ?? sceneWithObjects([], opts.seq ?? 0), opts.seq ?? 0));
  const scene = await ready;
  return { client, outbox, timer, socket: () => sockets[sockets.length - 1], scene, sockets };
}

function helloFrames(socket: MockWebSocket) {
  return socket.sentMessages().filter((m): m is Extract<ClientMessage, { type: "hello" }> => m.type === "hello");
}
function subscribeFrames(socket: MockWebSocket) {
  return socket.sentMessages().filter((m): m is Extract<ClientMessage, { type: "subscribe" }> => m.type === "subscribe");
}
function opsFrames(socket: MockWebSocket) {
  return socket.sentMessages().filter((m): m is Extract<ClientMessage, { type: "ops" }> => m.type === "ops");
}

describe("SceneClient windowed subscribe", () => {
  it("seeds the connection window from the connect region (hello carries the bbox)", async () => {
    const bbox = { x: 0, y: 0, width: 500, height: 500 };
    const { socket, scene, client } = await boot({
      initial: sceneWithObjects([objectAt("g-in", 10, 10)], 1),
      seq: 1,
      region: { bbox }
    });

    expect(helloFrames(socket())[0].region).toEqual({ canvasId: "canvas-a", bbox });
    expect(scene.objects.map((o) => o.id)).toEqual(["g-in"]);
    expect(client.currentWindow).toEqual(bbox);
  });

  it("re-subscribes with a new region on a camera move and reconciles entered/exited objects", async () => {
    const { client, timer, socket } = await boot({
      initial: sceneWithObjects([objectAt("g-left", 0, 0)], 1),
      seq: 1,
      region: { bbox: { x: -50, y: -50, width: 300, height: 300 } }
    });
    expect(client.scene?.objects.map((o) => o.id)).toEqual(["g-left"]);

    client.setViewport({ x: 950, y: -50, width: 300, height: 300 });
    expect(subscribeFrames(socket())).toHaveLength(0);
    timer.fire();

    const subs = subscribeFrames(socket());
    expect(subs).toHaveLength(1);
    expect(subs[0].region.bbox).toEqual({ x: 950, y: -50, width: 300, height: 300 });
    expect(client.currentWindow).toEqual({ x: 950, y: -50, width: 300, height: 300 });

    socket().emit(welcomeFrame(sceneWithObjects([objectAt("g-right", 1000, 0)], 2), 2));
    await Promise.resolve();

    expect(client.scene?.objects.map((o) => o.id)).toEqual(["g-right"]);
  });

  it("coalesces a rapid pan into a single re-subscribe (debounce)", async () => {
    const { client, timer, socket } = await boot({ region: { bbox: { x: 0, y: 0, width: 100, height: 100 } } });
    client.setViewport({ x: 10, y: 0, width: 100, height: 100 });
    client.setViewport({ x: 20, y: 0, width: 100, height: 100 });
    client.setViewport({ x: 30, y: 0, width: 100, height: 100 });
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
    const { client, socket } = await boot({ region: { bbox: { x: 0, y: 0, width: 100, height: 100 } } });
    client.subscribeWholeCanvas();
    const subs = subscribeFrames(socket());
    expect(subs).toHaveLength(1);
    expect(subs[0].region.bbox).toBeUndefined();
    expect(client.currentWindow).toBeNull();
  });
});

describe("backoffDelay schedule", () => {
  const opts = { baseMs: 500, maxMs: 15_000, jitter: 0.3 };

  it("grows exponentially and stays within the jitter bounds per attempt", () => {
    const upper = (n: number) => backoffDelay(n, opts, () => 0);
    expect(upper(0)).toBe(500);
    expect(upper(1)).toBe(1000);
    expect(upper(2)).toBe(2000);
    expect(upper(3)).toBe(4000);
    expect(upper(1)).toBeGreaterThan(upper(0));
    expect(upper(2)).toBeGreaterThan(upper(1));
  });

  it("caps at maxMs", () => {
    expect(backoffDelay(20, opts, () => 0)).toBe(opts.maxMs);
  });

  it("applies jitter inside [base*2^n * (1-jitter), base*2^n]", () => {
    const lower = backoffDelay(2, opts, () => 1);
    const upper = backoffDelay(2, opts, () => 0);
    expect(upper).toBe(2000);
    expect(lower).toBe(Math.round(2000 * (1 - 0.3)));
    expect(lower).toBeLessThan(upper);
  });
});

describe("WsTransport reconnect", () => {
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
    sockets[0].emit(welcomeFrame(sceneWithObjects([], 0), 0));
    await ready;
    expect(transport.connectionStatus).toBe("online");

    sockets[0].drop();
    expect(transport.connectionStatus).toBe("offline");
    expect(delays).toEqual([500]);

    timer.fire();
    expect(sockets).toHaveLength(2);
    sockets[1].drop();
    expect(delays).toEqual([500, 1000]);

    timer.fire();
    expect(sockets).toHaveLength(3);
    sockets[2].open();
    sockets[2].emit(welcomeFrame(sceneWithObjects([], 1), 1));
    expect(transport.connectionStatus).toBe("online");

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
    sockets[0].emit(welcomeFrame(sceneWithObjects([], 0), 0));
    void ready;

    transport.close();
    expect(timer.count()).toBe(0);
    expect(transport.connectionStatus).toBe("offline");
  });
});

describe("SceneClient offline buffering + reconnect replay", () => {
  it("buffers ops while offline (optimistic apply + outbox) and replays them on reconnect to converge", async () => {
    const { client, outbox, timer, socket, sockets } = await boot({ seq: 0, reconnect: { jitter: 0 }, random: () => 0 });
    const dead = socket();

    dead.drop();
    expect(client.connectionStatus).toBe("offline");

    await client.applyObjectOp(insertObjectOp("g1", 0, 0));
    await client.applyObjectOp(insertObjectOp("g2", 200, 0));
    client.flush();
    expect(client.scene?.objects.map((o) => o.id)).toEqual(["g1", "g2"]);
    expect(await outbox.all()).toHaveLength(2);
    expect(opsFrames(dead)).toHaveLength(0);

    timer.fireAll();
    expect(client.connectionStatus).toBe("offline");
    const reconnected = sockets[sockets.length - 1];
    expect(reconnected).not.toBe(dead);
    reconnected.open();
    reconnected.emit(welcomeFrame(sceneWithObjects([], 0), 0));
    await Promise.resolve();
    await Promise.resolve();

    expect(client.connectionStatus).toBe("online");
    expect(client.scene?.objects.map((o) => o.id)).toEqual(["g1", "g2"]);
    const replayed = opsFrames(reconnected).flatMap((f) => f.ops.map((e) => e.opId.localSeq));
    expect(replayed).toEqual([1, 2]);
    expect(await outbox.all()).toHaveLength(2);

    reconnected.emit({ type: "ack", opIds: [{ clientId: "c1", localSeq: 1 }, { clientId: "c1", localSeq: 2 }], seq: 2, revision: 2 });
    await Promise.resolve();
    expect(await outbox.all()).toHaveLength(0);
  });

  it("re-sends the seed region on the reconnect hello so the window is preserved", async () => {
    const bbox = { x: 0, y: 0, width: 400, height: 400 };
    const { client, timer, socket, sockets } = await boot({ region: { bbox }, reconnect: { jitter: 0 }, random: () => 0 });

    socket().drop();
    timer.fireAll();
    const reconnected = sockets[sockets.length - 1];
    reconnected.open();
    expect(helloFrames(reconnected)[0].region).toEqual({ canvasId: "canvas-a", bbox });
    reconnected.emit(welcomeFrame(sceneWithObjects([], 0), 0));
    await Promise.resolve();
    expect(client.connectionStatus).toBe("online");
  });
});

describe("SceneClient canvas switch", () => {
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
      initial: sceneWithObjects([objectAt("g-a", 0, 0)], 1),
      seq: 1,
      region: { bbox }
    });
    expect(client.scene?.objects.map((o) => o.id)).toEqual(["g-a"]);
    const oldSocket = socket();

    const switched = client.switchCanvas("canvas-b");
    const newSocket = sockets[sockets.length - 1];
    expect(newSocket).not.toBe(oldSocket);
    expect(oldSocket.closed).toBe(true);
    newSocket.open();
    newSocket.emit(welcomeFrame(sceneWithObjects([objectAt("g-b", 10, 10)], 7), 7));
    const scene = await switched;

    expect(scene.objects.map((o) => o.id)).toEqual(["g-b"]);
    expect(client.scene?.objects.map((o) => o.id)).toEqual(["g-b"]);
    const hello = helloFrames(newSocket)[0];
    expect(hello.canvasId).toBe("canvas-b");
    expect(hello.region).toEqual({ canvasId: "canvas-b", bbox });
  });

  it("switchCanvas starts the new canvas with a clean outbox (no cross-canvas replay)", async () => {
    const { client, outbox, timer, sockets } = await boot({ seq: 0 });

    await client.applyObjectOp(insertObjectOp("g-a", 0, 0));
    timer.fireAll();
    expect(await outbox.all()).toHaveLength(1);

    const switched = client.switchCanvas("canvas-b");
    const newSocket = sockets[sockets.length - 1];
    newSocket.open();
    newSocket.emit(welcomeFrame(sceneWithObjects([], 0), 0));
    await switched;

    expect(opsFrames(newSocket)).toHaveLength(0);
    expect(await outbox.all()).toHaveLength(1);
  });
});
