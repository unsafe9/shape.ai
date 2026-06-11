// OB4.3 client cutover tests.
//
// These prove the shell's data layer is the object-native stack (WS transport +
// scene-core OBJECT op-apply via SceneClient), not the retired HTTP/TS-renderPatch
// path. The Svelte shell (App.svelte) drives SceneClient exactly the way these
// tests do — connect for the LOAD snapshot, applyObjectOp for the SAVE,
// saveSelection for presence, sendFeature for the single feature channel — so
// asserting the contract here asserts the cutover without instantiating Svelte.
//
// The op-apply itself is the scene-core WASM object core, exercised end-to-end in
// object-op-apply.test.ts (the op-apply oracle); here we assert the wire shape.

import { beforeAll, beforeEach, describe, expect, it } from "vitest";

import { SceneClient } from "../platforms/web/runtime/sceneClient";
import { InMemoryOutboxStore } from "../platforms/web/runtime/outbox";
import { ensureSceneCore } from "../platforms/web/bridge/sceneCoreWasm";
import type { WebSocketLike } from "../platforms/web/runtime/wsTransport";
import type { ClientMessage, ServerMessage, WelcomeMessage } from "../platforms/web/runtime/transport";
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
    }
  };
}

function sceneWithA(sceneVersion = 1): ObjectScene {
  const scene = emptyObjectScene();
  scene.sceneVersion = sceneVersion;
  scene.objects = [rect("a")];
  return scene;
}

function rect(id: string): SceneObject {
  // fillRule is the canonical scene-core default; spell it out so the op/scene
  // round-tripped through the Rust session (which materializes serde defaults)
  // deep-equals this fixture.
  return { id, order: "a0", transform: translateTransform(0, 0), geometry: { d: "M 0 0 L 80 0 L 80 40 L 0 40 Z", fillRule: "evenOdd" } };
}

const insertB: ObjectOp = { kind: "insert-object", object: rect("b") };

const welcomeFrame = (scene: ObjectScene, seq: number): WelcomeMessage => ({ type: "welcome", scene, seq, revision: seq });

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
    clientId: "shell-cutover",
    outbox,
    now: fixedNow,
    setTimer: timer.setTimer,
    clearTimer: timer.clearTimer,
    createSocket: (url) => (socket = new MockWebSocket(url))
  });
  const ready = client.connect("default");
  socket!.open();
  socket!.emit(welcomeFrame(initial, seq));
  const scene = await ready;
  return { client, outbox, timer, socket: socket!, scene };
}

function opsFrames(socket: MockWebSocket) {
  return socket.sentMessages().filter((m): m is Extract<ClientMessage, { type: "ops" }> => m.type === "ops");
}
function presenceFrames(socket: MockWebSocket) {
  return socket.sentMessages().filter((m): m is Extract<ClientMessage, { type: "presence" }> => m.type === "presence");
}
function featureFrames(socket: MockWebSocket) {
  return socket.sentMessages().filter((m): m is Extract<ClientMessage, { type: "feature" }> => m.type === "feature");
}

describe("cutover: object scene LOAD via WS welcome", () => {
  it("adopts the server-authoritative welcome ObjectScene snapshot as the initial scene", async () => {
    const seeded = sceneWithA(3);
    const { scene, client, socket } = await boot(seeded, 3);

    expect(scene).toEqual(seeded);
    expect(client.scene).toEqual(seeded);

    const hello = socket.sentMessages()[0];
    expect(hello).toEqual({ type: "hello", canvasId: "default", lastAckSeq: 0, userId: "shell-cutover" });
  });
});

describe("cutover: object-op SAVE via WS ops", () => {
  it("authors an object op optimistically (wasm core) and sends it as one ops envelope of WireOps", async () => {
    const { client, outbox, timer, socket } = await boot(sceneWithA(1), 1);

    const { errors, opId } = await client.applyObjectOp(insertB);
    expect(errors).toEqual([]);
    expect(opId).toEqual({ clientId: "shell-cutover", localSeq: 1 });

    expect(client.scene?.objects.map((o) => o.id).sort()).toEqual(["a", "b"]);
    expect(await outbox.all()).toHaveLength(1);
    expect(opsFrames(socket)).toHaveLength(0);

    timer.fire();
    const frames = opsFrames(socket);
    expect(frames).toHaveLength(1);
    expect(frames[0].ops[0].propDelta).toEqual(insertB);

    socket.emit({ type: "ack", opIds: [opId!], seq: 2, revision: 2 });
    await Promise.resolve();
    expect(await outbox.all()).toHaveLength(0);
  });
});

describe("cutover: selection-only does not bump the revision", () => {
  it("rides presence with no outbox entry, no ops frame, no revision bump", async () => {
    const { client, outbox, timer, socket } = await boot(sceneWithA(7), 7);

    const before = client.scene?.sceneVersion;
    client.saveSelection({ kind: "object", id: "a" });
    timer.fire();

    expect(await outbox.all()).toHaveLength(0);
    expect(opsFrames(socket)).toHaveLength(0);
    expect(client.scene?.sceneVersion).toBe(before);

    const presence = presenceFrames(socket);
    expect(presence).toHaveLength(1);
    expect(presence[0].payload).toEqual({ kind: "select", selection: { kind: "object", id: "a" } });
  });
});

describe("cutover: feature traffic via the single WS feature channel (OB4.5)", () => {
  it("a template apply rides a feature frame, not bespoke REST", async () => {
    const { client, socket } = await boot(sceneWithA(1), 1);
    client.sendFeature({
      feature: "templateApply",
      canvas_id: "default",
      recipe: [rect("t1")],
      anchor_x: 10,
      anchor_y: 20
    });
    const features = featureFrames(socket);
    expect(features).toHaveLength(1);
    expect(features[0].request.feature).toBe("templateApply");
  });
});
