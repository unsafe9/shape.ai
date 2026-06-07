// Transport-backed scene client tests (MG5.3).
//
// These drive `SceneClient` over a MockWebSocket (the same double the
// ws-transport / sync-engine suites use) plus an injected manual timer so the
// coalescing flush is deterministic. They prove the data-layer contract the
// shell consumes in place of `fetchScene` / `saveScenePatch`:
//
//   - connect() resolves with the welcome Scene snapshot (fresh empty for new);
//   - a renderer op applies optimistically, enqueues to the outbox, and is
//     dropped on the server ack;
//   - a CRUD ScenePatch persists via the transport as op envelope(s);
//   - a selection-only change produces NO document op (no outbox entry, no `ops`
//     frame, no revision bump) — it rides presence;
//   - a reconnect welcome reloads the snapshot.

import { beforeEach, describe, expect, it } from "vitest";

import { SceneClient, scenePatchToRenderOps } from "../src/client/lib/sceneClient";
import { InMemoryOutboxStore } from "../src/client/lib/outbox";
import type { WebSocketLike } from "../src/client/lib/wsTransport";
import type { ClientMessage, ServerMessage, WelcomeMessage } from "../src/client/lib/transport";
import type { RenderScenePatch } from "../src/shared/renderPatch";
import type { Scene, ScenePatch, SceneNode } from "../src/shared/schema";

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
}

/** A manual timer: the engine arms a callback; the test fires it on demand. */
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

const welcomeFrame = (scene: Scene, seq: number): WelcomeMessage => ({
  type: "welcome",
  scene,
  seq,
  revision: seq
});

const groupG1: RenderScenePatch = {
  kind: "create-group",
  group: {
    id: "g1",
    title: "G",
    summary: "",
    bounds: { x: 0, y: 0, width: 400, height: 300 },
    tagIds: [],
    zIndex: 0,
    styleKey: ""
  }
};

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
    clientId: "c1",
    outbox,
    now: fixedNow,
    setTimer: timer.setTimer,
    clearTimer: timer.clearTimer,
    createSocket: (url) => (socket = new MockWebSocket(url))
  });
  const ready = client.connect("c-mg5");
  socket!.open();
  socket!.emit(welcomeFrame(initial, seq));
  const scene = await ready;
  return { client, outbox, timer, socket: socket!, scene };
}

/** All `ops` frames the socket has sent so far. */
function opsFrames(socket: MockWebSocket) {
  return socket
    .sentMessages()
    .filter((m): m is Extract<ClientMessage, { type: "ops" }> => m.type === "ops");
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

describe("SceneClient.connect", () => {
  it("resolves with the welcome scene snapshot (fresh empty for a new canvas)", async () => {
    const { scene, socket } = await boot();
    expect(scene).toEqual(emptyScene(0));

    // hello was the first frame and carried the canvas id.
    const hello = socket.sentMessages()[0];
    // MG6.3: userId defaults to clientId and rides the hello handshake.
    expect(hello).toEqual({ type: "hello", canvasId: "c-mg5", lastAckSeq: 0, userId: "c1" });
  });

  it("exposes the welcome scene as the current scene", async () => {
    const seeded: Scene = { ...emptyScene(2), groups: [] };
    const { client } = await boot(seeded, 2);
    expect(client.scene).toEqual(seeded);
  });

  it("rejects a second connect on the same client", async () => {
    const { client } = await boot();
    await expect(client.connect("c-other")).rejects.toThrow("already connected");
  });
});

describe("SceneClient renderer ops", () => {
  it("applies a renderer op optimistically, enqueues it to the outbox, sends it, and drops it on ack", async () => {
    const { client, outbox, timer, socket } = await boot();

    const scenes: Scene[] = [];
    client.onScene((s) => scenes.push(s));

    const { errors, opId } = await client.applyRenderPatch(groupG1);
    expect(errors).toEqual([]);
    expect(opId).toEqual({ clientId: "c1", localSeq: 1 });

    // Optimistic local apply happened immediately.
    expect(client.scene?.groups.map((g) => g.id)).toEqual(["g1"]);
    expect(scenes.at(-1)?.groups.map((g) => g.id)).toEqual(["g1"]);

    // Persisted to the outbox before any send.
    expect(await outbox.all()).toHaveLength(1);
    expect(opsFrames(socket)).toHaveLength(0);

    // The coalescing flush sends one ops envelope frame.
    timer.fire();
    const frames = opsFrames(socket);
    expect(frames).toHaveLength(1);
    expect(frames[0].ops[0].opId).toEqual(opId);
    expect(frames[0].ops[0].patch).toEqual(groupG1);

    // Server ack drops the op from the outbox.
    socket.emit({ type: "ack", opIds: [opId!], seq: 1, revision: 1 });
    await Promise.resolve();
    expect(await outbox.all()).toHaveLength(0);
  });
});

describe("SceneClient CRUD ScenePatch", () => {
  it("persists a CRUD change via the transport as op envelope(s)", async () => {
    // Start from a scene that already has g1 so the create-card validates.
    const seeded: Scene = {
      ...emptyScene(1),
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
    const { client, outbox, timer, socket } = await boot(seeded, 1);

    const patch: ScenePatch = { nodes: [sceneNode("n1", "g1")] };
    const { errors } = await client.applyScenePatch(patch);
    expect(errors).toEqual([]);

    // Optimistically present in the local scene.
    expect(client.scene?.nodes.map((n) => n.id)).toEqual(["n1"]);
    // Persisted before flush.
    expect(await outbox.all()).toHaveLength(1);

    timer.fire();
    const frames = opsFrames(socket);
    expect(frames).toHaveLength(1);
    expect(frames[0].ops).toHaveLength(1);
    expect(frames[0].ops[0].patch.kind).toBe("create-card");
  });
});

describe("SceneClient selection (presence-only)", () => {
  it("does not produce a document op for a selection-only change", async () => {
    const { client, outbox, timer, socket } = await boot();

    const before = client.scene?.sceneVersion ?? -1;
    client.saveSelection({ kind: "canvas" });
    timer.fire();

    // No outbox entry, no ops frame, no revision bump.
    expect(await outbox.all()).toHaveLength(0);
    expect(opsFrames(socket)).toHaveLength(0);
    expect(client.scene?.sceneVersion).toBe(before);

    // It rode a presence frame instead.
    const presence = socket
      .sentMessages()
      .filter((m): m is Extract<ClientMessage, { type: "presence" }> => m.type === "presence");
    expect(presence).toHaveLength(1);
    expect(presence[0].payload).toEqual({ kind: "select", selection: { kind: "canvas" } });
  });

  it("a selection carried on a ScenePatch rides presence, not the outbox", async () => {
    const { client, outbox, timer, socket } = await boot();

    // Selection-only ScenePatch: no document mutations -> no ops, only presence.
    await client.applyScenePatch({ selection: { kind: "group", id: "g1" } });
    timer.fire();

    expect(await outbox.all()).toHaveLength(0);
    expect(opsFrames(socket)).toHaveLength(0);
    const presence = socket
      .sentMessages()
      .filter((m): m is Extract<ClientMessage, { type: "presence" }> => m.type === "presence");
    expect(presence).toHaveLength(1);
  });
});

describe("SceneClient reconnect", () => {
  it("reloads the snapshot on a reconnect welcome and replays unacked ops", async () => {
    const { client, outbox, timer, socket } = await boot();

    await client.applyRenderPatch(groupG1);
    timer.fire();
    socket.sent.length = 0; // forget the first send

    // A reconnect welcome (no pending connect) arrives with a fresh snapshot.
    const reconnected: Scene = { ...emptyScene(5), selection: { kind: "canvas" } };
    socket.emit(welcomeFrame(reconnected, 5));
    await Promise.resolve();
    await Promise.resolve();

    // The snapshot was reloaded as the new base, with the unacked op replayed on
    // top, so g1 (still unacked) survives.
    expect(client.scene?.groups.map((g) => g.id)).toEqual(["g1"]);

    // The unacked op was re-sent with its original opId.
    const replayed = opsFrames(socket).flatMap((f) => f.ops.map((e) => e.opId.localSeq));
    expect(replayed).toContain(1);
    expect(await outbox.all()).toHaveLength(1);
  });
});

describe("scenePatchToRenderOps", () => {
  it("decomposes document mutations into ops and ignores the selection field", () => {
    const patch: ScenePatch = {
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
          createdAt: "t",
          updatedAt: "t"
        }
      ],
      nodes: [sceneNode("n1", "g1")],
      removeNodeIds: ["n0"],
      selection: { kind: "node", id: "n1" }
    };
    const ops = scenePatchToRenderOps(patch);
    expect(ops.map((o) => o.kind)).toEqual(["create-group", "create-card", "delete-card"]);
    // The selection field never becomes a document op.
    expect(ops.some((o) => o.kind === "select")).toBe(false);
  });

  it("returns no ops for a selection-only patch", () => {
    expect(scenePatchToRenderOps({ selection: { kind: "canvas" } })).toEqual([]);
  });
});
