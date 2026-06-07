// Client sync engine tests (MG4.3 outbox, MG4.4 optimistic + unacked discard,
// MG4.5 coalescing + reconnect reconcile).
//
// These drive the engine with the InMemoryOutboxStore and a controllable mock
// transport/timer so coalescing and reconnect are deterministic. A second block
// drives the real WsTransport.attachEngine wiring through a MockWebSocket to
// prove the integration end to end.

import { beforeAll, beforeEach, describe, expect, it } from "vitest";

import {
  COALESCE_MS,
  SyncEngine,
  type EngineTransport
} from "../src/client/lib/syncEngine";
import {
  InMemoryOutboxStore,
  opIdKey,
  type OpId,
  type OutboxEntry
} from "../src/client/lib/outbox";
import { WsTransport, type WebSocketLike } from "../src/client/lib/wsTransport";
import { ensureSceneCore } from "../src/client/scene/sceneCoreWasm";
import type { RenderScenePatch } from "../src/shared/renderPatch";
import type { Scene } from "../src/shared/schema";
import type { ClientMessage, ServerMessage, WelcomeMessage } from "../src/client/lib/transport";

// The engine's default op-apply is the scene-core wasm (the same Rust the server
// runs); it must be initialized before any `author`. Under Node/vitest the loader
// reads the prebuilt `--target web` `.wasm` from disk and inits synchronously, so
// these tests exercise the WASM op-apply — not the TS one — under Node.
beforeAll(async () => {
  await ensureSceneCore();
});

// ---------------------------------------------------------------------------
// Fixtures.
// ---------------------------------------------------------------------------

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

const cardN1: RenderScenePatch = {
  kind: "create-card",
  card: {
    id: "n1",
    groupId: "g1",
    title: "C",
    summary: "",
    detail: "",
    status: "",
    type: "",
    bounds: { x: 10, y: 10, width: 120, height: 80 },
    zIndex: 0,
    styleKey: "",
    accessibilityLabel: ""
  }
};

const moveN1 = (x: number, y: number): RenderScenePatch => ({
  kind: "move-card",
  id: "n1",
  position: { x, y }
});

const editN1Title = (value: string): RenderScenePatch => ({
  kind: "edit-card-text",
  id: "n1",
  field: "title",
  value
});

/** A mock EngineTransport that records flushed batches. */
class CaptureTransport implements EngineTransport {
  readonly batches: OutboxEntry[][] = [];
  sendEnvelopes(entries: OutboxEntry[]): void {
    this.batches.push(entries);
  }
  /** Every envelope flushed so far, flattened. */
  flat(): OutboxEntry[] {
    return this.batches.flat();
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

let nowCounter = 0;
const fixedNow = () => `t${nowCounter++}`;

beforeEach(() => {
  nowCounter = 0;
});

// ---------------------------------------------------------------------------
// MG4.3 — durable outbox: append -> send -> ack -> remove; replay on reconnect.
// ---------------------------------------------------------------------------

describe("outbox lifecycle (MG4.3)", () => {
  it("appends before send, removes on ack", async () => {
    const outbox = new InMemoryOutboxStore();
    const transport = new CaptureTransport();
    const timer = manualTimer();
    const engine = new SyncEngine(emptyScene(), {
      clientId: "c1",
      outbox,
      transport,
      now: fixedNow,
      setTimer: timer.setTimer,
      clearTimer: timer.clearTimer
    });

    const { opId } = await engine.author(groupG1);
    expect(opId).toEqual({ clientId: "c1", localSeq: 1 });

    // Persisted before any send (the coalescing timer has not fired yet).
    expect(await outbox.all()).toHaveLength(1);
    expect(transport.batches).toHaveLength(0);

    timer.fire();
    expect(transport.batches).toHaveLength(1);
    expect(transport.flat()[0].opId).toEqual(opId);

    await engine.onAck({ opIds: [opId!], revision: 1 });
    expect(await outbox.all()).toHaveLength(0);
  });

  it("persists across a simulated reconnect and replays in localSeq order, dedup-safe", async () => {
    // The SAME outbox instance survives the reconnect (durable case). Two ops
    // are authored but never acked; on reconnect they must be re-sent in order
    // with the SAME opIds (so the server dedups).
    const outbox = new InMemoryOutboxStore();
    const t1 = new CaptureTransport();
    const timer1 = manualTimer();
    const engine1 = new SyncEngine(emptyScene(), {
      clientId: "c1",
      outbox,
      transport: t1,
      now: fixedNow,
      setTimer: timer1.setTimer,
      clearTimer: timer1.clearTimer
    });

    await engine1.author(groupG1);
    await engine1.author({ ...cardN1 });
    timer1.fire();
    const firstSendIds = t1.flat().map((e) => opIdKey(e.opId));
    expect(firstSendIds).toEqual(["c1:1", "c1:2"]);

    // Reconnect: a fresh engine over the same durable outbox replays it.
    const t2 = new CaptureTransport();
    const timer2 = manualTimer();
    const engine2 = new SyncEngine(emptyScene(), {
      clientId: "c1",
      outbox,
      transport: t2,
      now: fixedNow,
      setTimer: timer2.setTimer,
      clearTimer: timer2.clearTimer
    });

    await engine2.reconcileSnapshot(emptyScene(0));
    const replayed = t2.flat();
    expect(replayed.map((e) => opIdKey(e.opId))).toEqual(["c1:1", "c1:2"]);
    // Same opIds as the first send: re-sending is idempotent on the server.
    expect(replayed.map((e) => opIdKey(e.opId))).toEqual(firstSendIds);
  });
});

// ---------------------------------------------------------------------------
// MG4.4 — optimistic apply + transient ownership / unacked discard.
// ---------------------------------------------------------------------------

describe("optimistic apply + unacked discard (MG4.4)", () => {
  function bootEngine() {
    const outbox = new InMemoryOutboxStore();
    const transport = new CaptureTransport();
    const timer = manualTimer();
    const engine = new SyncEngine(emptyScene(), {
      clientId: "c1",
      outbox,
      transport,
      now: fixedNow,
      setTimer: timer.setTimer,
      clearTimer: timer.clearTimer
    });
    return { engine, outbox, transport, timer };
  }

  it("reflects an authored op in the local scene immediately", async () => {
    const { engine } = bootEngine();
    await engine.author(groupG1);
    expect(engine.getScene().groups.map((g) => g.id)).toEqual(["g1"]);

    await engine.author(cardN1);
    await engine.author(moveN1(99, 99));
    const node = engine.getScene().nodes.find((n) => n.id === "n1");
    expect(node?.position).toEqual({ x: 99, y: 99 });
  });

  it("ignores a remote write to an unacked property, then applies it after ack", async () => {
    const { engine } = bootEngine();
    await engine.author(groupG1);
    await engine.author(cardN1);

    // Local optimistic move — we now OWN (n1, position) until it is acked.
    const { opId } = await engine.author(moveN1(50, 50));
    expect(engine.getScene().nodes[0].position).toEqual({ x: 50, y: 50 });

    // A peer's remote move to the SAME property is dropped (transient ownership).
    const applied = engine.applyRemote(moveN1(7, 7));
    expect(applied).toBe(false);
    expect(engine.getScene().nodes[0].position).toEqual({ x: 50, y: 50 });

    // After our op is acked, ownership releases and a remote write applies.
    await engine.onAck({ opIds: [opId!], revision: 4 });
    const applied2 = engine.applyRemote(moveN1(7, 7));
    expect(applied2).toBe(true);
    expect(engine.getScene().nodes[0].position).toEqual({ x: 7, y: 7 });
  });

  it("lets a remote write to a DIFFERENT field on the same object through", async () => {
    const { engine } = bootEngine();
    await engine.author(groupG1);
    await engine.author(cardN1);

    // Own (n1, position) only.
    await engine.author(moveN1(50, 50));

    // A remote TITLE edit on the same node touches a different field -> applies.
    const applied = engine.applyRemote(editN1Title("from-peer"));
    expect(applied).toBe(true);
    expect(engine.getScene().nodes[0].title).toBe("from-peer");
    // Our owned position is untouched.
    expect(engine.getScene().nodes[0].position).toEqual({ x: 50, y: 50 });
  });
});

// ---------------------------------------------------------------------------
// MG4.5 — coalescing.
// ---------------------------------------------------------------------------

describe("coalescing (MG4.5)", () => {
  it("batches N rapid ops within one window into a single frame", async () => {
    const outbox = new InMemoryOutboxStore();
    const transport = new CaptureTransport();
    const timer = manualTimer();
    const engine = new SyncEngine(emptyScene(), {
      clientId: "c1",
      outbox,
      transport,
      coalesceMs: COALESCE_MS,
      now: fixedNow,
      setTimer: timer.setTimer,
      clearTimer: timer.clearTimer
    });

    await engine.author(groupG1);
    await engine.author(cardN1);
    // A burst of rapid moves (continuous drag) within one coalescing window.
    await engine.author(moveN1(1, 1));
    await engine.author(moveN1(2, 2));
    await engine.author(moveN1(3, 3));

    // Nothing has flushed yet (timer still armed).
    expect(transport.batches).toHaveLength(0);
    expect(timer.armed()).toBe(true);

    timer.fire();
    // All 5 ops ride a SINGLE ops frame.
    expect(transport.batches).toHaveLength(1);
    expect(transport.batches[0]).toHaveLength(5);

    // A new op after the flush arms a fresh window -> a second frame.
    await engine.author(moveN1(4, 4));
    timer.fire();
    expect(transport.batches).toHaveLength(2);
    expect(transport.batches[1]).toHaveLength(1);
  });

  it("flush() drains the buffer immediately without the timer", async () => {
    const outbox = new InMemoryOutboxStore();
    const transport = new CaptureTransport();
    const timer = manualTimer();
    const engine = new SyncEngine(emptyScene(), {
      clientId: "c1",
      outbox,
      transport,
      now: fixedNow,
      setTimer: timer.setTimer,
      clearTimer: timer.clearTimer
    });

    await engine.author(groupG1);
    expect(transport.batches).toHaveLength(0);
    engine.flush();
    expect(transport.batches).toHaveLength(1);
    expect(timer.armed()).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// MG4.5 — reconnect reconcile: welcome snapshot + outbox replay converges.
// ---------------------------------------------------------------------------

describe("reconnect reconcile (MG4.5)", () => {
  it("rebases on the snapshot, replays unacked ops, and converges", async () => {
    const outbox = new InMemoryOutboxStore();
    const transport = new CaptureTransport();
    const timer = manualTimer();
    const engine = new SyncEngine(emptyScene(), {
      clientId: "c1",
      outbox,
      transport,
      now: fixedNow,
      setTimer: timer.setTimer,
      clearTimer: timer.clearTimer
    });

    // The client created g1 + n1 and moved n1; n1's move is still unacked when
    // the socket drops (g1/n1 creates were acked and removed from the outbox).
    await engine.author(groupG1);
    await engine.author(cardN1);
    const create1 = { clientId: "c1", localSeq: 1 } as OpId;
    const create2 = { clientId: "c1", localSeq: 2 } as OpId;
    await engine.onAck({ opIds: [create1, create2], revision: 2 });
    const { opId: moveOp } = await engine.author(moveN1(80, 80));
    expect(await outbox.all()).toHaveLength(1);

    // Reconnect: the server welcome carries g1 + n1 at their pre-move positions,
    // plus a peer's title edit the client never saw. Reconcile must keep the
    // server's title yet preserve the client's unacked optimistic move.
    const snapshot = engine.getScene();
    const serverScene: Scene = {
      ...snapshot,
      sceneVersion: 3,
      nodes: snapshot.nodes.map((n) =>
        n.id === "n1" ? { ...n, title: "peer-title", position: { x: 10, y: 10 } } : n
      )
    };

    await engine.reconcileSnapshot(serverScene);

    const n1 = engine.getScene().nodes.find((n) => n.id === "n1")!;
    // Server's title survives (no unacked local write on title)...
    expect(n1.title).toBe("peer-title");
    // ...and the client's unacked move is replayed on top of the snapshot.
    expect(n1.position).toEqual({ x: 80, y: 80 });

    // The unacked move was re-sent on reconnect with its original opId.
    expect(transport.flat().map((e) => opIdKey(e.opId))).toEqual([opIdKey(moveOp!)]);

    // Acking the replayed move clears the outbox; convergence reached.
    await engine.onAck({ opIds: [moveOp!], revision: 4 });
    expect(await outbox.all()).toHaveLength(0);
  });
});

// ---------------------------------------------------------------------------
// Integration — engine wired onto a real WsTransport over a MockWebSocket.
// ---------------------------------------------------------------------------

class MockWebSocket implements WebSocketLike {
  onopen: ((ev: unknown) => void) | null = null;
  onclose: ((ev: unknown) => void) | null = null;
  onerror: ((ev: unknown) => void) | null = null;
  onmessage: ((ev: { data: unknown }) => void) | null = null;
  readonly sent: string[] = [];
  constructor(readonly url: string) {}
  send(data: string): void {
    this.sent.push(data);
  }
  close(): void {
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

const welcomeFrame = (scene: Scene, seq: number): WelcomeMessage => ({
  type: "welcome",
  scene,
  seq,
  revision: seq
});

describe("WsTransport + SyncEngine integration", () => {
  async function bootIntegration() {
    let socket: MockWebSocket | null = null;
    const transport = new WsTransport({
      url: "ws://127.0.0.1:8787",
      createSocket: (url) => (socket = new MockWebSocket(url))
    });
    const ready = transport.connect("c-int");
    socket!.open();
    socket!.emit(welcomeFrame(emptyScene(0), 0));
    const welcome = await ready;

    const outbox = new InMemoryOutboxStore();
    const timer = manualTimer();
    const engine = new SyncEngine(welcome.scene, {
      clientId: "c1",
      outbox,
      transport,
      now: fixedNow,
      setTimer: timer.setTimer,
      clearTimer: timer.clearTimer
    });
    const detach = transport.attachEngine(engine);
    return { transport, engine, outbox, timer, socket: socket!, detach };
  }

  it("sends authored envelopes through the socket and clears the outbox on the server ack", async () => {
    const { engine, outbox, timer, socket } = await bootIntegration();

    const { opId } = await engine.author(groupG1);
    expect(await outbox.all()).toHaveLength(1);

    timer.fire();
    // [0] hello, [1] ops envelope frame.
    const opsFrame = socket.sentMessages()[1] as { type: string; ops: OutboxEntry[] };
    expect(opsFrame.type).toBe("ops");
    expect(opsFrame.ops[0].opId).toEqual(opId);

    // Server acks the op -> attachEngine drops it from the outbox.
    socket.emit({ type: "ack", opIds: [opId!], seq: 1, revision: 1 });
    await Promise.resolve();
    expect(await outbox.all()).toHaveLength(0);
  });

  it("feeds remote patches through the discard path", async () => {
    const { engine, timer, socket } = await bootIntegration();
    await engine.author(groupG1);
    await engine.author(cardN1);
    await engine.author(moveN1(50, 50));
    timer.fire();

    // A remote move to the owned (n1, position) is dropped by attachEngine's
    // applyRemote, leaving the optimistic value intact.
    socket.emit({ type: "patch", ops: [moveN1(7, 7)], seq: 2 });
    expect(engine.getScene().nodes[0].position).toEqual({ x: 50, y: 50 });
  });

  it("reconciles + replays the outbox on a reconnect welcome", async () => {
    const { transport, engine, outbox, timer, socket } = await bootIntegration();
    await engine.author(groupG1);
    const { opId } = await engine.author(cardN1);
    timer.fire();
    socket.sent.length = 0; // forget the first send

    // resume -> the server replies with a fresh welcome; attachEngine reconciles
    // and the engine replays the still-unacked outbox over the socket.
    transport.resume();
    socket.emit(welcomeFrame(engine.getScene(), 5));
    await Promise.resolve();
    await Promise.resolve();

    // The replay re-sent the unacked ops as an ops frame.
    const opsFrames = socket
      .sentMessages()
      .filter((m): m is Extract<ClientMessage, { type: "ops" }> => m.type === "ops");
    const replayedIds = opsFrames.flatMap((f) => f.ops.map((e) => opIdKey(e.opId)));
    expect(replayedIds).toContain(opIdKey({ clientId: "c1", localSeq: 1 }));
    expect(replayedIds).toContain(opIdKey(opId!));
    expect(await outbox.all()).toHaveLength(2);
  });
});
