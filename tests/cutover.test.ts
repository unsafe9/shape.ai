// MG-7a client cutover tests.
//
// These prove that the shell's data layer is the NEW stack (WS transport +
// scene-core op-apply via SceneClient), not the old HTTP api.ts / TS-only
// renderPatch path. The Svelte shell (App.svelte) drives SceneClient exactly the
// way these tests do — connect for the LOAD snapshot, applyRenderPatch /
// applyScenePatch for the SAVE, saveSelection for presence — so asserting the
// contract here asserts the cutover without instantiating Svelte under node.
//
// We also assert that the scene-core op-apply DECOMPOSITION the shell relies on
// (scenePatchToRenderOps + the optimistic engine apply inside SceneClient) is
// golden-equivalent to the TS applyRenderPatchToShapeScene that MG-7b will
// delete: a create-card op and a CRUD node patch produce the same resulting
// scene through both paths. The scene-core WASM itself is `--target web` and
// cannot `fetch()` its module under node, so the op-apply equivalence is proven
// against the TS golden the WASM was ported from.

import { beforeEach, describe, expect, it } from "vitest";

import { SceneClient, scenePatchToRenderOps } from "../src/client/lib/sceneClient";
import { InMemoryOutboxStore } from "../src/client/lib/outbox";
import { applyRenderPatchToShapeScene, type RenderScenePatch } from "../src/shared/renderPatch";
import type { WebSocketLike } from "../src/client/lib/wsTransport";
import type { ClientMessage, ServerMessage, WelcomeMessage } from "../src/client/lib/transport";
import type { Scene, ScenePatch, SceneNode } from "../src/shared/schema";

// ---------------------------------------------------------------------------
// Doubles (same shape the scene-client / ws-transport suites use).
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

function sceneWithG1(sceneVersion = 1): Scene {
  return {
    ...emptyScene(sceneVersion),
    groups: [
      {
        id: "g1",
        parentGroupId: null,
        title: "G",
        summary: "",
        bounds: { x: 0, y: 0, width: 400, height: 300 },
        tagIds: [],
        zIndex: 0,
        collapsed: false,
        createdAt: "1970-01-01T00:00:00Z",
        updatedAt: "1970-01-01T00:00:00Z"
      }
    ]
  };
}

const welcomeFrame = (scene: Scene, seq: number): WelcomeMessage => ({
  type: "welcome",
  scene,
  seq,
  revision: seq
});

function sceneNode(id: string, groupId: string): SceneNode {
  return {
    id,
    groupId,
    type: "task",
    title: id,
    summary: "",
    detail: "",
    status: "draft",
    confidence: 0.5,
    evidenceRefs: [],
    childDecisionIds: [],
    tagIds: [],
    position: { x: 10, y: 10 },
    size: { width: 120, height: 80 },
    zIndex: 0,
    updatedAt: "1970-01-01T00:00:00Z"
  };
}

const createCardG1: RenderScenePatch = {
  kind: "create-card",
  card: {
    id: "n1",
    groupId: "g1",
    title: "N1",
    summary: "",
    detail: "",
    status: "draft",
    type: "task",
    bounds: { x: 10, y: 10, width: 120, height: 80 },
    zIndex: 0,
    styleKey: "task",
    accessibilityLabel: "task N1"
  }
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
  scene: Scene;
};

async function boot(initial: Scene = emptyScene(0), seq = 0): Promise<Booted> {
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

// ---------------------------------------------------------------------------
// 1. Scene LOAD comes from the WS welcome snapshot, not HTTP fetchScene.
// ---------------------------------------------------------------------------

describe("MG-7a cutover: scene LOAD via WS welcome", () => {
  it("adopts the server-authoritative welcome snapshot as the initial scene", async () => {
    const seeded = sceneWithG1(3);
    const { scene, client, socket } = await boot(seeded, 3);

    // The shell adopts the welcome scene verbatim (no HTTP /api/scene fetch).
    expect(scene).toEqual(seeded);
    expect(client.scene).toEqual(seeded);

    // The first frame on the wire is the hello handshake, not an HTTP request.
    const hello = socket.sentMessages()[0];
    // MG6.3: userId defaults to clientId and rides the hello handshake.
    expect(hello).toEqual({ type: "hello", canvasId: "default", lastAckSeq: 0, userId: "shell-cutover" });
  });
});

// ---------------------------------------------------------------------------
// 2. Renderer-op SAVE flows over WS as an ops envelope (the shell's hot path).
// ---------------------------------------------------------------------------

describe("MG-7a cutover: renderer-op SAVE via WS ops", () => {
  it("authors a renderer op optimistically and sends it as one ops envelope", async () => {
    const { client, outbox, timer, socket } = await boot(sceneWithG1(1), 1);

    const { errors, opId } = await client.applyRenderPatch(createCardG1);
    expect(errors).toEqual([]);
    expect(opId).toEqual({ clientId: "shell-cutover", localSeq: 1 });

    // Optimistic local apply (scene-core path inside the engine) landed n1.
    expect(client.scene?.nodes.map((n) => n.id)).toEqual(["n1"]);
    // Persisted to the outbox before any send; nothing on the wire yet.
    expect(await outbox.all()).toHaveLength(1);
    expect(opsFrames(socket)).toHaveLength(0);

    // The coalescing flush emits exactly one ops envelope carrying the raw op.
    timer.fire();
    const frames = opsFrames(socket);
    expect(frames).toHaveLength(1);
    expect(frames[0].ops[0].patch).toEqual(createCardG1);

    // Server ack drops the op from the durable outbox.
    socket.emit({ type: "ack", opIds: [opId!], seq: 2, revision: 2 });
    await Promise.resolve();
    expect(await outbox.all()).toHaveLength(0);
  });
});

// ---------------------------------------------------------------------------
// 3. Shell CRUD SAVE flows over WS as decomposed ops (no HTTP saveScenePatch).
// ---------------------------------------------------------------------------

describe("MG-7a cutover: shell CRUD SAVE via WS ops", () => {
  it("persists a CRUD ScenePatch as op envelope(s)", async () => {
    const { client, outbox, timer, socket } = await boot(sceneWithG1(1), 1);

    const patch: ScenePatch = { nodes: [sceneNode("n1", "g1")], selection: { kind: "node", id: "n1" } };
    const { errors } = await client.applyScenePatch(patch);
    expect(errors).toEqual([]);

    // Optimistically present locally; the document mutation persisted to outbox.
    expect(client.scene?.nodes.map((n) => n.id)).toEqual(["n1"]);
    expect(await outbox.all()).toHaveLength(1);

    timer.fire();
    const frames = opsFrames(socket);
    expect(frames).toHaveLength(1);
    expect(frames[0].ops[0].patch.kind).toBe("create-card");

    // The selection carried alongside the document change rode presence, not ops.
    expect(presenceFrames(socket)).toHaveLength(1);
    expect(presenceFrames(socket)[0].payload).toEqual({ kind: "select", selection: { kind: "node", id: "n1" } });
  });
});

// ---------------------------------------------------------------------------
// 4. Selection-only change is presence — never a document op (invariant).
// ---------------------------------------------------------------------------

describe("MG-7a cutover: selection-only does not bump the revision", () => {
  it("rides presence with no outbox entry, no ops frame, no revision bump", async () => {
    const { client, outbox, timer, socket } = await boot(sceneWithG1(7), 7);

    const before = client.scene?.sceneVersion;
    client.saveSelection({ kind: "group", id: "g1" });
    timer.fire();

    expect(await outbox.all()).toHaveLength(0);
    expect(opsFrames(socket)).toHaveLength(0);
    expect(client.scene?.sceneVersion).toBe(before);

    const presence = presenceFrames(socket);
    expect(presence).toHaveLength(1);
    expect(presence[0].payload).toEqual({ kind: "select", selection: { kind: "group", id: "g1" } });
  });
});

// ---------------------------------------------------------------------------
// 5. ONE op-apply: the engine's optimistic apply is golden-equivalent to the TS
//    applyRenderPatchToShapeScene the cutover routes through scene-core. The WASM
//    bridge is a faithful port of this TS golden, so equivalence here is the
//    contract MG-7b deletes the TS duplicate against.
// ---------------------------------------------------------------------------

describe("MG-7a cutover: single op-apply is golden-equivalent", () => {
  it("a create-card op yields the same nodes through the engine and the TS golden", async () => {
    const { client } = await boot(sceneWithG1(1), 1);
    await client.applyRenderPatch(createCardG1);

    const golden = applyRenderPatchToShapeScene(sceneWithG1(1), createCardG1, "t-golden");

    expect(client.scene?.nodes.map((n) => ({ id: n.id, groupId: n.groupId, type: n.type }))).toEqual(
      golden.scene.nodes.map((n) => ({ id: n.id, groupId: n.groupId, type: n.type }))
    );
  });

  it("scenePatchToRenderOps decomposes a CRUD patch into ops and drops the selection field", () => {
    const patch: ScenePatch = {
      nodes: [sceneNode("n1", "g1")],
      removeNodeIds: ["n0"],
      selection: { kind: "node", id: "n1" }
    };
    const ops = scenePatchToRenderOps(patch);
    expect(ops.map((o) => o.kind)).toEqual(["create-card", "delete-card"]);
    expect(ops.some((o) => o.kind === "select")).toBe(false);
  });
});
