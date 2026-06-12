import { beforeAll, beforeEach, describe, expect, it } from "vitest";

import {
  COALESCE_MS,
  SyncEngine,
  type EngineTransport
} from "../runtime/syncEngine";
import {
  InMemoryOutboxStore,
  opIdKey,
  type OpId,
  type OutboxEntry
} from "../runtime/outbox";
import { WsTransport, type WebSocketLike } from "../runtime/wsTransport";
import { ensureSceneCore } from "../bridge/sceneCoreWasm";
import {
  emptyObjectScene,
  translateTransform,
  IDENTITY_TRANSFORM,
  type Object as SceneObject,
  type ObjectOp,
  type ObjectScene
} from "../shared/object";
import type { ClientMessage, ServerMessage, WelcomeMessage } from "../runtime/transport";

beforeAll(async () => {
  await ensureSceneCore();
});

function rect(id: string, order = "a0"): SceneObject {
  return {
    id,
    order,
    transform: IDENTITY_TRANSFORM,
    geometry: { d: "M 0 0 L 80 0 L 80 40 L 0 40 Z", fillRule: "evenOdd" }
  };
}

const insertA: ObjectOp = { kind: "insert-object", object: rect("a", "a0") };
const insertB: ObjectOp = { kind: "insert-object", object: rect("b", "a1") };
const moveA = (x: number, y: number): ObjectOp => ({ kind: "set-transform", id: "a", transform: translateTransform(x, y) });
const textA = (value: string): ObjectOp => ({ kind: "set-text", id: "a", text: { runs: [{ text: value, bold: false, italic: false }], align: "start", valign: "top" } });

class CaptureTransport implements EngineTransport {
  readonly batches: OutboxEntry[][] = [];
  sendEnvelopes(entries: OutboxEntry[]): void {
    this.batches.push(entries);
  }
  flat(): OutboxEntry[] {
    return this.batches.flat();
  }
}

// The engine arms a callback; the test fires it on demand.
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

function objectIds(scene: ObjectScene): string[] {
  return scene.objects.map((o) => o.id);
}

function objectTransform(scene: ObjectScene, id: string) {
  return scene.objects.find((o) => o.id === id)?.transform;
}

describe("outbox lifecycle", () => {
  it("appends before send, removes on ack", async () => {
    const outbox = new InMemoryOutboxStore();
    const transport = new CaptureTransport();
    const timer = manualTimer();
    const engine = new SyncEngine(emptyObjectScene(), {
      clientId: "c1",
      outbox,
      transport,
      now: fixedNow,
      setTimer: timer.setTimer,
      clearTimer: timer.clearTimer
    });

    const { opId } = await engine.author(insertA);
    expect(opId).toEqual({ clientId: "c1", localSeq: 1 });

    // Persisted before any send (the coalescing timer has not fired).
    expect(await outbox.all()).toHaveLength(1);
    expect(transport.batches).toHaveLength(0);

    timer.fire();
    expect(transport.batches).toHaveLength(1);
    expect(transport.flat()[0].opId).toEqual(opId);
    expect(transport.flat()[0].propDelta).toEqual(insertA);

    await engine.onAck({ opIds: [opId!], revision: 1 });
    expect(await outbox.all()).toHaveLength(0);
  });

  it("captures the inverse op for undo", async () => {
    const outbox = new InMemoryOutboxStore();
    const engine = new SyncEngine(emptyObjectScene(), {
      clientId: "c1",
      outbox,
      transport: new CaptureTransport(),
      now: fixedNow,
      setTimer: manualTimer().setTimer,
      clearTimer: manualTimer().clearTimer
    });
    const { inverse } = await engine.author(insertA);
    expect(inverse).toEqual({ kind: "delete", id: "a" });
  });

  it("persists across a simulated reconnect and replays in localSeq order, dedup-safe", async () => {
    const outbox = new InMemoryOutboxStore();
    const t1 = new CaptureTransport();
    const timer1 = manualTimer();
    const engine1 = new SyncEngine(emptyObjectScene(), {
      clientId: "c1",
      outbox,
      transport: t1,
      now: fixedNow,
      setTimer: timer1.setTimer,
      clearTimer: timer1.clearTimer
    });

    await engine1.author(insertA);
    await engine1.author(insertB);
    timer1.fire();
    const firstSendIds = t1.flat().map((e) => opIdKey(e.opId));
    expect(firstSendIds).toEqual(["c1:1", "c1:2"]);

    const t2 = new CaptureTransport();
    const timer2 = manualTimer();
    const engine2 = new SyncEngine(emptyObjectScene(), {
      clientId: "c1",
      outbox,
      transport: t2,
      now: fixedNow,
      setTimer: timer2.setTimer,
      clearTimer: timer2.clearTimer
    });

    await engine2.reconcileSnapshot(emptyObjectScene());
    const replayed = t2.flat();
    expect(replayed.map((e) => opIdKey(e.opId))).toEqual(["c1:1", "c1:2"]);
    expect(replayed.map((e) => opIdKey(e.opId))).toEqual(firstSendIds);
  });
});

describe("optimistic apply + unacked discard", () => {
  function bootEngine() {
    const outbox = new InMemoryOutboxStore();
    const transport = new CaptureTransport();
    const timer = manualTimer();
    const engine = new SyncEngine(emptyObjectScene(), {
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
    await engine.author(insertA);
    expect(objectIds(engine.getScene())).toEqual(["a"]);

    await engine.author(moveA(99, 99));
    expect(objectTransform(engine.getScene(), "a")).toEqual(translateTransform(99, 99));
  });

  it("ignores a remote write to an unacked field, then applies it after ack", async () => {
    const { engine } = bootEngine();
    await engine.author(insertA);

    // Local optimistic move: we own (a, transform) until it is acked.
    const { opId } = await engine.author(moveA(50, 50));
    expect(objectTransform(engine.getScene(), "a")).toEqual(translateTransform(50, 50));

    // A peer's remote move to the SAME field is dropped (transient ownership).
    const applied = engine.applyRemote(moveA(7, 7));
    expect(applied).toBe(false);
    expect(objectTransform(engine.getScene(), "a")).toEqual(translateTransform(50, 50));

    // After the ack, ownership releases and a remote write applies.
    await engine.onAck({ opIds: [opId!], revision: 3 });
    const applied2 = engine.applyRemote(moveA(7, 7));
    expect(applied2).toBe(true);
    expect(objectTransform(engine.getScene(), "a")).toEqual(translateTransform(7, 7));
  });

  it("lets a remote write to a DIFFERENT field on the same object through", async () => {
    const { engine } = bootEngine();
    await engine.author(insertA);

    // Own (a, transform) only.
    await engine.author(moveA(50, 50));

    // A remote TEXT edit touches a different field -> applies.
    const applied = engine.applyRemote(textA("from-peer"));
    expect(applied).toBe(true);
    expect(engine.getScene().objects[0].text?.runs[0].text).toBe("from-peer");
    expect(objectTransform(engine.getScene(), "a")).toEqual(translateTransform(50, 50));
  });
});

describe("coalescing", () => {
  it("batches N rapid ops within one window into a single frame", async () => {
    const outbox = new InMemoryOutboxStore();
    const transport = new CaptureTransport();
    const timer = manualTimer();
    const engine = new SyncEngine(emptyObjectScene(), {
      clientId: "c1",
      outbox,
      transport,
      coalesceMs: COALESCE_MS,
      now: fixedNow,
      setTimer: timer.setTimer,
      clearTimer: timer.clearTimer
    });

    await engine.author(insertA);
    // A burst of rapid moves within one coalescing window.
    await engine.author(moveA(1, 1));
    await engine.author(moveA(2, 2));
    await engine.author(moveA(3, 3));

    expect(transport.batches).toHaveLength(0);
    expect(timer.armed()).toBe(true);

    timer.fire();
    expect(transport.batches).toHaveLength(1);
    expect(transport.batches[0]).toHaveLength(4);

    await engine.author(moveA(4, 4));
    timer.fire();
    expect(transport.batches).toHaveLength(2);
    expect(transport.batches[1]).toHaveLength(1);
  });

  it("flush() drains the buffer immediately without the timer", async () => {
    const outbox = new InMemoryOutboxStore();
    const transport = new CaptureTransport();
    const timer = manualTimer();
    const engine = new SyncEngine(emptyObjectScene(), {
      clientId: "c1",
      outbox,
      transport,
      now: fixedNow,
      setTimer: timer.setTimer,
      clearTimer: timer.clearTimer
    });

    await engine.author(insertA);
    expect(transport.batches).toHaveLength(0);
    engine.flush();
    expect(transport.batches).toHaveLength(1);
    expect(timer.armed()).toBe(false);
  });
});

describe("reconnect reconcile", () => {
  it("rebases on the snapshot, replays unacked ops, and converges", async () => {
    const outbox = new InMemoryOutboxStore();
    const transport = new CaptureTransport();
    const timer = manualTimer();
    const engine = new SyncEngine(emptyObjectScene(), {
      clientId: "c1",
      outbox,
      transport,
      now: fixedNow,
      setTimer: timer.setTimer,
      clearTimer: timer.clearTimer
    });

    // Create acked + removed; the move is still unacked when the socket drops.
    await engine.author(insertA);
    const create1 = { clientId: "c1", localSeq: 1 } as OpId;
    await engine.onAck({ opIds: [create1], revision: 1 });
    const { opId: moveOp } = await engine.author(moveA(80, 80));
    expect(await outbox.all()).toHaveLength(1);

    // Welcome carries "a" at its pre-move position plus a peer's unseen text edit.
    // Reconcile keeps the server's text yet preserves the client's unacked move.
    const snapshot = engine.getScene();
    const serverScene: ObjectScene = {
      ...snapshot,
      sceneVersion: 3,
      objects: snapshot.objects.map((o) =>
        o.id === "a" ? { ...o, text: { runs: [{ text: "peer-text", bold: false, italic: false }], align: "start", valign: "top" }, transform: IDENTITY_TRANSFORM } : o
      )
    };

    await engine.reconcileSnapshot(serverScene);

    const a = engine.getScene().objects.find((o) => o.id === "a")!;
    // Server text survives (no unacked local write on text); the unacked move
    // replays on top of the snapshot.
    expect(a.text?.runs[0].text).toBe("peer-text");
    expect(a.transform).toEqual(translateTransform(80, 80));

    // The unacked move was re-sent with its original opId.
    expect(transport.flat().map((e) => opIdKey(e.opId))).toEqual([opIdKey(moveOp!)]);

    await engine.onAck({ opIds: [moveOp!], revision: 4 });
    expect(await outbox.all()).toHaveLength(0);
  });
});

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

const welcomeFrame = (scene: ObjectScene, seq: number): WelcomeMessage => ({
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
    socket!.emit(welcomeFrame(emptyObjectScene(), 0));
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

  it("sends authored WireOp envelopes through the socket and clears the outbox on the server ack", async () => {
    const { engine, outbox, timer, socket } = await bootIntegration();

    const { opId } = await engine.author(insertA);
    expect(await outbox.all()).toHaveLength(1);

    timer.fire();
    // [0] hello, [1] ops envelope frame.
    const opsFrame = socket.sentMessages()[1] as { type: string; ops: OutboxEntry[] };
    expect(opsFrame.type).toBe("ops");
    expect(opsFrame.ops[0].opId).toEqual(opId);
    expect(opsFrame.ops[0].propDelta).toEqual(insertA);

    socket.emit({ type: "ack", opIds: [opId!], seq: 1, revision: 1 });
    await Promise.resolve();
    expect(await outbox.all()).toHaveLength(0);
  });

  it("feeds remote patches through the discard path", async () => {
    const { engine, timer, socket } = await bootIntegration();
    await engine.author(insertA);
    await engine.author(moveA(50, 50));
    timer.fire();

    // A remote move to the owned (a, transform) is dropped by applyRemote.
    socket.emit({
      type: "patch",
      ops: [
        {
          opId: { clientId: "peer", localSeq: 1 },
          objectId: "a",
          kind: "set-transform",
          propDelta: moveA(7, 7),
          baseRevision: 2,
          actor: "peer",
          ts: "t"
        }
      ],
      seq: 2
    });
    expect(objectTransform(engine.getScene(), "a")).toEqual(translateTransform(50, 50));
  });

  it("reconciles + replays the outbox on a reconnect welcome", async () => {
    const { transport, engine, outbox, timer, socket } = await bootIntegration();
    await engine.author(insertA);
    const { opId } = await engine.author(insertB);
    timer.fire();
    socket.sent.length = 0;

    transport.resume();
    socket.emit(welcomeFrame(engine.getScene(), 5));
    await Promise.resolve();
    await Promise.resolve();

    const opsFrames = socket
      .sentMessages()
      .filter((m): m is Extract<ClientMessage, { type: "ops" }> => m.type === "ops");
    const replayedIds = opsFrames.flatMap((f) => f.ops.map((e) => opIdKey(e.opId)));
    expect(replayedIds).toContain(opIdKey({ clientId: "c1", localSeq: 1 }));
    expect(replayedIds).toContain(opIdKey(opId!));
    expect(await outbox.all()).toHaveLength(2);
  });
});
