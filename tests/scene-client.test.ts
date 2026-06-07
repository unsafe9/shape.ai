// Transport-backed object scene client tests (OB4.3).
//
// These drive `SceneClient` over a MockWebSocket plus an injected manual timer so
// the coalescing flush is deterministic. They prove the data-layer contract the
// shell consumes:
//
//   - connect() resolves with the welcome ObjectScene snapshot (fresh empty for new);
//   - an object op applies optimistically (via the wasm core), enqueues to the
//     outbox as a WireOp, sends it, and is dropped on the server ack;
//   - a selection-only change produces NO document op (no outbox entry, no `ops`
//     frame, no revision bump) — it rides presence;
//   - a feature request rides the single WS feature channel (OB4.5);
//   - a reconnect welcome reloads the snapshot and replays unacked ops.

import { beforeAll, beforeEach, describe, expect, it } from "vitest";

import { SceneClient } from "../src/client/lib/sceneClient";
import { InMemoryOutboxStore } from "../src/client/lib/outbox";
import { ensureSceneCore } from "../src/client/scene/sceneCoreWasm";
import type { WebSocketLike } from "../src/client/lib/wsTransport";
import type { ClientMessage, ServerMessage, WelcomeMessage } from "../src/client/lib/transport";
import {
  emptyObjectScene,
  translateTransform,
  type ObjectOp,
  type ObjectScene
} from "../src/shared/object";

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
}

function manualTimer() {
  let cb: (() => void) | null = null;
  return {
    setTimer: (fn: () => void, _ms: number) => {
      cb = fn;
      return 1 as unknown;
    },
    clearTimer: (_h: unknown) => {
      cb = null;
    },
    fire: () => {
      const f = cb;
      cb = null;
      f?.();
    },
    armed: () => cb != null
  };
}

const welcomeFrame = (scene: ObjectScene, seq: number): WelcomeMessage => ({
  type: "welcome",
  scene,
  seq,
  revision: seq
});

const insertA: ObjectOp = {
  kind: "insert-object",
  object: { id: "a", order: "a0", transform: translateTransform(0, 0), geometry: { d: "M 0 0 L 80 0 L 80 40 L 0 40 Z" } }
};

let nowCounter = 0;
const fixedNow = () => `t${nowCounter++}`;

beforeEach(() => {
  nowCounter = 0;
});

type Booted = {
  client: SceneClient;
  outbox: InMemoryOutboxStore;
  timer: ReturnType<typeof manualTimer>;
  socket: MockWebSocket;
  scene: ObjectScene;
};

async function boot(initial: ObjectScene = emptyObjectScene(), seq = 0): Promise<Booted> {
  let socket: MockWebSocket | null = null;
  const outbox = new InMemoryOutboxStore();
  const timer = manualTimer();
  const client = new SceneClient({
    url: "ws://127.0.0.1:8787",
    clientId: "c1",
    outbox,
    now: fixedNow,
    setTimer: timer.setTimer,
    clearTimer: timer.clearTimer,
    createSocket: (url) => (socket = new MockWebSocket(url))
  });
  const ready = client.connect("c-ob");
  socket!.open();
  socket!.emit(welcomeFrame(initial, seq));
  const scene = await ready;
  return { client, outbox, timer, socket: socket!, scene };
}

function opsFrames(socket: MockWebSocket) {
  return socket.sentMessages().filter((m): m is Extract<ClientMessage, { type: "ops" }> => m.type === "ops");
}

describe("SceneClient.connect", () => {
  it("resolves with the welcome object scene snapshot (fresh empty for a new canvas)", async () => {
    const { scene, socket } = await boot();
    expect(scene).toEqual(emptyObjectScene());

    const hello = socket.sentMessages()[0];
    expect(hello).toEqual({ type: "hello", canvasId: "c-ob", lastAckSeq: 0, userId: "c1" });
  });

  it("exposes the welcome scene as the current scene", async () => {
    const seeded = emptyObjectScene();
    seeded.sceneVersion = 2;
    const { client } = await boot(seeded, 2);
    expect(client.scene).toEqual(seeded);
  });

  it("rejects a second connect on the same client", async () => {
    const { client } = await boot();
    await expect(client.connect("c-other")).rejects.toThrow("already connected");
  });
});

describe("SceneClient object ops", () => {
  it("applies an object op optimistically, enqueues a WireOp, sends it, and drops it on ack", async () => {
    const { client, outbox, timer, socket } = await boot();

    const scenes: ObjectScene[] = [];
    client.onScene((s) => scenes.push(s));

    const { errors, opId, inverse } = await client.applyObjectOp(insertA);
    expect(errors).toEqual([]);
    expect(opId).toEqual({ clientId: "c1", localSeq: 1 });
    // The inverse op is captured (the undo entry, D21).
    expect(inverse).toEqual({ kind: "delete", id: "a" });

    // Optimistic local apply happened immediately.
    expect(client.scene?.objects.map((o) => o.id)).toEqual(["a"]);
    expect(scenes.at(-1)?.objects.map((o) => o.id)).toEqual(["a"]);

    // Persisted to the outbox as a WireOp before any send.
    expect(await outbox.all()).toHaveLength(1);
    expect(opsFrames(socket)).toHaveLength(0);

    timer.fire();
    const frames = opsFrames(socket);
    expect(frames).toHaveLength(1);
    expect(frames[0].ops[0].opId).toEqual(opId);
    expect(frames[0].ops[0].propDelta).toEqual(insertA);

    socket.emit({ type: "ack", opIds: [opId!], seq: 1, revision: 1 });
    await Promise.resolve();
    expect(await outbox.all()).toHaveLength(0);
  });
});

describe("SceneClient selection (presence-only)", () => {
  it("does not produce a document op for a selection-only change", async () => {
    const { client, outbox, timer, socket } = await boot();

    const before = client.scene?.sceneVersion ?? -1;
    client.saveSelection({ kind: "canvas" });
    timer.fire();

    expect(await outbox.all()).toHaveLength(0);
    expect(opsFrames(socket)).toHaveLength(0);
    expect(client.scene?.sceneVersion).toBe(before);

    const presence = socket
      .sentMessages()
      .filter((m): m is Extract<ClientMessage, { type: "presence" }> => m.type === "presence");
    expect(presence).toHaveLength(1);
    expect(presence[0].payload).toEqual({ kind: "select", selection: { kind: "canvas" } });
  });
});

describe("SceneClient feature channel (OB4.5)", () => {
  it("sends a feature request on the single WS feature channel", async () => {
    const { client, socket } = await boot();
    client.sendFeature({ feature: "canvasSwitch", canvas_id: "c-ob" });
    const feature = socket
      .sentMessages()
      .filter((m): m is Extract<ClientMessage, { type: "feature" }> => m.type === "feature");
    expect(feature).toHaveLength(1);
    expect(feature[0].request).toEqual({ feature: "canvasSwitch", canvas_id: "c-ob" });
  });

  it("surfaces a feature response to onFeature subscribers", async () => {
    const { client, socket } = await boot();
    const received: unknown[] = [];
    client.onFeature((response) => received.push(response));
    socket.emit({ type: "feature", response: { feature: "templateApplied", object_ids: ["o1"] } });
    expect(received).toEqual([{ feature: "templateApplied", object_ids: ["o1"] }]);
  });
});

describe("SceneClient reconnect", () => {
  it("reloads the snapshot on a reconnect welcome and replays unacked ops", async () => {
    const { client, outbox, timer, socket } = await boot();

    await client.applyObjectOp(insertA);
    timer.fire();
    socket.sent.length = 0; // forget the first send

    const reconnected = emptyObjectScene();
    reconnected.sceneVersion = 5;
    socket.emit(welcomeFrame(reconnected, 5));
    await Promise.resolve();
    await Promise.resolve();

    // The unacked insert is replayed on top of the snapshot, so "a" survives.
    expect(client.scene?.objects.map((o) => o.id)).toEqual(["a"]);

    const replayed = opsFrames(socket).flatMap((f) => f.ops.map((e) => e.opId.localSeq));
    expect(replayed).toContain(1);
    expect(await outbox.all()).toHaveLength(1);
  });
});
