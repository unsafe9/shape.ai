import { beforeAll, beforeEach, describe, expect, it } from "vitest";

import { SceneClient } from "../platforms/web/runtime/sceneClient";
import { InMemoryOutboxStore } from "../platforms/web/runtime/outbox";
import { ensureSceneCore } from "../platforms/web/bridge/sceneCoreWasm";
import type { WebSocketLike } from "../platforms/web/runtime/wsTransport";
import type { ClientMessage, ServerMessage, WelcomeMessage } from "../platforms/web/runtime/transport";
import type { PeerPresence } from "../platforms/web/runtime/peers";
import {
  emptyObjectScene,
  translateTransform,
  type Object as SceneObject,
  type ObjectOp,
  type ObjectScene,
  type WireOp
} from "../platforms/web/shared/object";

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
}

function manualTimer() {
  const callbacks: (() => void)[] = [];
  return {
    setTimer: (fn: () => void, _ms: number) => {
      callbacks.push(fn);
      return callbacks.length as unknown;
    },
    clearTimer: (_h: unknown) => {},
    fireAll: () => {
      const pending = callbacks.splice(0);
      for (const cb of pending) cb();
    }
  };
}

beforeAll(async () => {
  await ensureSceneCore();
});

function rect(id: string): SceneObject {
  return { id, order: "a0", transform: translateTransform(0, 0), geometry: { d: "M 0 0 L 80 0 L 80 40 L 0 40 Z", fillRule: "nonZero" } };
}

// Wrap an ObjectOp as a peer WireOp for a `patch` fan-out frame.
function peerWire(op: ObjectOp, seq: number): WireOp {
  return {
    opId: { clientId: "peer", localSeq: seq },
    objectId: op.kind === "insert-object" ? op.object.id : op.kind === "delete" ? op.id : "",
    kind: op.kind,
    propDelta: op,
    baseRevision: seq,
    actor: "peer",
    ts: "t"
  };
}

const insertA: ObjectOp = { kind: "insert-object", object: rect("a") };
const moveA = (x: number, y: number): ObjectOp => ({ kind: "set-transform", id: "a", transform: translateTransform(x, y) });
const textA = (value: string): ObjectOp => ({ kind: "set-text", id: "a", text: { runs: [{ text: value, bold: false, italic: false }], align: "start", valign: "top" } });

const welcomeFrame = (scene: ObjectScene, seq: number): WelcomeMessage => ({ type: "welcome", scene, seq, revision: seq });

let nowCounter = 0;
const fixedNow = () => `t${nowCounter++}`;

beforeEach(() => {
  nowCounter = 0;
});

function manualClock() {
  let t = 1_000;
  return {
    nowMs: () => t,
    advance: (ms: number) => {
      t += ms;
    }
  };
}

type Booted = {
  client: SceneClient;
  outbox: InMemoryOutboxStore;
  timer: ReturnType<typeof manualTimer>;
  socket: MockWebSocket;
  scene: ObjectScene;
};

async function boot(
  initial: ObjectScene = emptyObjectScene(),
  seq = 0,
  opts: { userId?: string; clock?: ReturnType<typeof manualClock> } = {}
): Promise<Booted> {
  let socket: MockWebSocket | null = null;
  const outbox = new InMemoryOutboxStore();
  const timer = manualTimer();
  const client = new SceneClient({
    url: "ws://127.0.0.1:8787",
    clientId: "c1",
    userId: opts.userId ?? "user-1",
    outbox,
    now: fixedNow,
    setTimer: timer.setTimer,
    clearTimer: timer.clearTimer,
    nowMs: opts.clock?.nowMs,
    createSocket: (url) => (socket = new MockWebSocket(url))
  });
  const ready = client.connect("c-mu");
  socket!.open();
  socket!.emit(welcomeFrame(initial, seq));
  const scene = await ready;
  return { client, outbox, timer, socket: socket!, scene };
}

// Scene with object "a" already present so move/text ops validate.
function seededScene(seq = 2): ObjectScene {
  const scene = emptyObjectScene();
  scene.sceneVersion = seq;
  scene.objects = [rect("a")];
  return scene;
}

function objectById(scene: ObjectScene | null, id: string): SceneObject | undefined {
  return scene?.objects.find((o) => o.id === id);
}

describe("peer patch apply", () => {
  it("applies a peer patch to the local scene", async () => {
    const { client, socket } = await boot();
    socket.emit({ type: "patch", ops: [peerWire(insertA, 1)], seq: 1 });
    expect(client.scene?.objects.map((o) => o.id)).toEqual(["a"]);
  });

  it("applies a peer's create then move in arrival order", async () => {
    const { client, socket } = await boot();
    socket.emit({ type: "patch", ops: [peerWire(insertA, 1)], seq: 1 });
    socket.emit({ type: "patch", ops: [peerWire(moveA(99, 88), 2)], seq: 2 });
    expect(objectById(client.scene, "a")?.transform).toEqual(translateTransform(99, 88));
  });
});

describe("mid-drag transient ownership", () => {
  it("ignores a peer write to a field the client is mid-drag on, then applies after ack", async () => {
    const { client, timer, socket } = await boot(seededScene(2), 2);

    const { opId } = await client.applyObjectOp(moveA(200, 200));
    expect(objectById(client.scene, "a")?.transform).toEqual(translateTransform(200, 200));
    timer.fireAll();

    // A peer move to the owned (a, transform) is IGNORED while unacked.
    socket.emit({ type: "patch", ops: [peerWire(moveA(7, 7), 3)], seq: 3 });
    expect(objectById(client.scene, "a")?.transform).toEqual(translateTransform(200, 200));

    socket.emit({ type: "ack", opIds: [opId!], seq: 4, revision: 4 });
    await Promise.resolve();

    socket.emit({ type: "patch", ops: [peerWire(moveA(7, 7), 5)], seq: 5 });
    expect(objectById(client.scene, "a")?.transform).toEqual(translateTransform(7, 7));
  });

  it("applies a peer write to a DIFFERENT field while a drag is in flight", async () => {
    const { client, timer, socket } = await boot(seededScene(2), 2);

    await client.applyObjectOp(moveA(200, 200));
    timer.fireAll();

    socket.emit({ type: "patch", ops: [peerWire(textA("from-peer"), 3)], seq: 3 });

    expect(objectById(client.scene, "a")?.text?.runs[0].text).toBe("from-peer");
    expect(objectById(client.scene, "a")?.transform).toEqual(translateTransform(200, 200));
  });
});

describe("peer presence cursors", () => {
  it("renders a peer cursor from an inbound presence frame", async () => {
    const { client, socket } = await boot();

    const updates: PeerPresence[][] = [];
    client.onPeers((peers) => updates.push(peers));

    socket.emit({ type: "presence", payload: { userId: "peer-9", cursor: { x: 12, y: 34 } } });

    expect(client.peerCursors).toHaveLength(1);
    expect(client.peerCursors[0]).toMatchObject({ userId: "peer-9", cursor: { x: 12, y: 34 } });
    expect(updates.at(-1)?.map((p) => p.userId)).toEqual(["peer-9"]);
    expect(typeof client.peerCursors[0].color).toBe("string");
  });

  it("keeps the latest cursor per peer (latest-wins)", async () => {
    const { client, socket } = await boot();
    socket.emit({ type: "presence", payload: { userId: "peer-9", cursor: { x: 1, y: 1 } } });
    socket.emit({ type: "presence", payload: { userId: "peer-9", cursor: { x: 50, y: 60 } } });
    expect(client.peerCursors).toHaveLength(1);
    expect(client.peerCursors[0].cursor).toEqual({ x: 50, y: 60 });
  });

  it("does not surface the client's OWN presence as a peer", async () => {
    const { client, socket } = await boot(emptyObjectScene(), 0, { userId: "user-1" });
    socket.emit({ type: "presence", payload: { userId: "user-1", cursor: { x: 5, y: 5 } } });
    socket.emit({ type: "presence", payload: { userId: "peer-2", cursor: { x: 9, y: 9 } } });
    expect(client.peerCursors.map((p) => p.userId)).toEqual(["peer-2"]);
  });

  it("stamps the local userId on the cursor frame it sends", async () => {
    const { client, socket } = await boot(emptyObjectScene(), 0, { userId: "user-1" });
    client.sendCursor({ x: 100, y: 200 }, { x: 0, y: 0, width: 800, height: 600 });
    const presence = socket
      .sentMessages()
      .filter((m): m is Extract<ClientMessage, { type: "presence" }> => m.type === "presence");
    expect(presence).toHaveLength(1);
    expect(presence[0].payload).toEqual({
      userId: "user-1",
      cursor: { x: 100, y: 200 },
      viewport: { x: 0, y: 0, width: 800, height: 600 }
    });
  });

  it("expires a peer cursor after the TTL window", async () => {
    const clock = manualClock();
    const { client, socket } = await boot(emptyObjectScene(), 0, { clock });
    socket.emit({ type: "presence", payload: { userId: "peer-9", cursor: { x: 1, y: 1 } } });
    expect(client.peerCursors).toHaveLength(1);
    clock.advance(11_000);
    expect(client.peerCursors).toHaveLength(0);
  });

  it("expires a stale peer when a fresh peer frame arrives", async () => {
    const clock = manualClock();
    const { client, socket } = await boot(emptyObjectScene(), 0, { clock });
    socket.emit({ type: "presence", payload: { userId: "peer-9", cursor: { x: 1, y: 1 } } });
    clock.advance(11_000);
    socket.emit({ type: "presence", payload: { userId: "peer-2", cursor: { x: 2, y: 2 } } });
    expect(client.peerCursors.map((p) => p.userId)).toEqual(["peer-2"]);
  });
});

describe("concurrent-edit convergence", () => {
  it("converges to the server arrival-order result for concurrent edits on a shared field", async () => {
    const { client, socket } = await boot(seededScene(2), 2);
    socket.emit({ type: "patch", ops: [peerWire(moveA(100, 100), 3)], seq: 3 });
    socket.emit({ type: "patch", ops: [peerWire(moveA(300, 400), 4)], seq: 4 });
    expect(objectById(client.scene, "a")?.transform).toEqual(translateTransform(300, 400));
  });

  it("a self-acked op plus a later peer op converge to the peer's value", async () => {
    const { client, timer, socket } = await boot(seededScene(2), 2);
    const { opId } = await client.applyObjectOp(moveA(50, 50));
    timer.fireAll();
    socket.emit({ type: "ack", opIds: [opId!], seq: 3, revision: 3 });
    await Promise.resolve();
    socket.emit({ type: "patch", ops: [peerWire(moveA(900, 900), 4)], seq: 4 });
    expect(objectById(client.scene, "a")?.transform).toEqual(translateTransform(900, 900));
  });
});
