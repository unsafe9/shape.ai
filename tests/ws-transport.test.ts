import { describe, expect, it } from "vitest";

import { WsTransport, type WebSocketLike } from "../platforms/web/runtime/wsTransport";
import type {
  AckMessage,
  ClientMessage,
  FeatureServerMessage,
  PatchMessage,
  PresenceServerMessage,
  ServerMessage,
  WelcomeMessage
} from "../platforms/web/runtime/transport";
import type { OutboxEntry } from "../platforms/web/runtime/outbox";
import { emptyObjectScene, translateTransform, type ObjectOp, type WireOp } from "../platforms/web/shared/object";

// open() fires onopen, emit(frame) delivers a server frame, sent records the JSON
// strings the transport wrote.
class MockWebSocket implements WebSocketLike {
  onopen: ((ev: unknown) => void) | null = null;
  onclose: ((ev: unknown) => void) | null = null;
  onerror: ((ev: unknown) => void) | null = null;
  onmessage: ((ev: { data: unknown }) => void) | null = null;

  readonly url: string;
  readonly sent: string[] = [];
  closed = false;

  constructor(url: string) {
    this.url = url;
  }

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

  emit(frame: ServerMessage | string): void {
    const data = typeof frame === "string" ? frame : JSON.stringify(frame);
    this.onmessage?.({ data });
  }

  sentMessages(): ClientMessage[] {
    return this.sent.map((s) => JSON.parse(s) as ClientMessage);
  }
}

type Connected = {
  transport: WsTransport;
  getSocket: () => MockWebSocket;
};

function makeTransport(): Connected {
  let socket: MockWebSocket | null = null;
  const transport = new WsTransport({
    url: "ws://127.0.0.1:8787",
    createSocket: (url) => {
      socket = new MockWebSocket(url);
      return socket;
    }
  });
  return {
    transport,
    getSocket: () => {
      if (!socket) throw new Error("socket not created yet");
      return socket;
    }
  };
}

async function connected(canvasId = "c-ws"): Promise<{ transport: WsTransport; socket: MockWebSocket }> {
  const { transport, getSocket } = makeTransport();
  const ready = transport.connect(canvasId);
  const socket = getSocket();
  socket.open();
  socket.emit(emptyWelcome);
  await ready;
  return { transport, socket };
}

const emptyWelcome: WelcomeMessage = {
  type: "welcome",
  scene: emptyObjectScene(),
  seq: 0,
  revision: 0
};

// A WireOp envelope around an ObjectOp delta (the outbox/wire shape).
function wireOp(op: ObjectOp, clientId: string, localSeq: number): WireOp {
  return {
    opId: { clientId, localSeq },
    objectId: op.kind === "insert-object" ? op.object.id : op.kind === "delete" ? op.id : "",
    kind: op.kind,
    propDelta: op,
    baseRevision: 0,
    actor: clientId,
    ts: "t"
  };
}

const insertA: ObjectOp = {
  kind: "insert-object",
  object: { id: "a", order: "a0", transform: translateTransform(0, 0), geometry: { d: "M 0 0 L 80 0 L 80 40 L 0 40 Z", fillRule: "nonZero" } }
};

describe("WsTransport handshake", () => {
  it("appends /ws to the base url for the socket", () => {
    const { transport, getSocket } = makeTransport();
    void transport.connect("c-ws");
    expect(getSocket().url).toBe("ws://127.0.0.1:8787/ws");
  });

  it("sends a hello frame with the right shape on open and resolves on welcome", async () => {
    const { transport, getSocket } = makeTransport();
    const ready = transport.connect("c-ws");
    const socket = getSocket();

    expect(socket.sent).toHaveLength(0);
    socket.open();

    const sent = socket.sentMessages();
    expect(sent).toHaveLength(1);
    expect(sent[0]).toEqual({ type: "hello", canvasId: "c-ws", lastAckSeq: 0 });

    socket.emit(emptyWelcome);
    const welcome = await ready;
    expect(welcome).toEqual({ scene: emptyWelcome.scene, seq: 0, revision: 0 });
  });

  it("includes region in hello when provided", async () => {
    const { transport, getSocket } = makeTransport();
    const region = { canvasId: "c-ws", bbox: { x: 0, y: 0, width: 800, height: 600 } };
    const ready = transport.connect("c-ws", region);
    const socket = getSocket();
    socket.open();

    expect(socket.sentMessages()[0]).toEqual({ type: "hello", canvasId: "c-ws", region, lastAckSeq: 0 });

    socket.emit(emptyWelcome);
    await ready;
  });

  it("rejects connect() when an error frame arrives instead of welcome", async () => {
    const { transport, getSocket } = makeTransport();
    const ready = transport.connect("c-ws");
    const socket = getSocket();
    socket.open();
    socket.emit({ type: "error", message: "expected hello as the first message" });
    await expect(ready).rejects.toThrow("expected hello as the first message");
  });
});

describe("WsTransport outbound frames", () => {
  it("emits an ops frame of WireOp envelopes matching the protocol", async () => {
    const { transport, socket } = await connected();

    const entries: OutboxEntry[] = [wireOp(insertA, "user-1", 1)];
    transport.sendEnvelopes(entries);

    // [0] is the hello; [1] is the ops frame carrying WireOps verbatim.
    const frame = socket.sentMessages()[1] as { type: string; ops: WireOp[] };
    expect(frame.type).toBe("ops");
    expect(frame.ops).toHaveLength(1);
    expect(frame.ops[0]).toMatchObject({
      opId: { clientId: "user-1", localSeq: 1 },
      kind: "insert-object",
      propDelta: insertA
    });
  });

  it("emits a feature request frame on the single feature channel", async () => {
    const { transport, socket } = await connected();

    transport.sendFeature({ feature: "canvasSwitch", canvas_id: "c-ws" });
    expect(socket.sentMessages()[1]).toEqual({
      type: "feature",
      request: { feature: "canvasSwitch", canvas_id: "c-ws" }
    });
  });

  it("emits a presence frame with the canvas binding and verbatim payload", async () => {
    const { transport, socket } = await connected();

    const payload = { cursor: { x: 12, y: 34 }, userId: "user-1" };
    transport.sendPresence(payload);
    expect(socket.sentMessages()[1]).toEqual({ type: "presence", canvasId: "c-ws", payload });
  });
});

describe("WsTransport inbound routing", () => {
  it("routes a patch frame to onPatch with decoded WireOps", async () => {
    const { transport, socket } = await connected();

    const received: PatchMessage[] = [];
    transport.onPatch((p) => received.push(p));

    const patch: PatchMessage = { type: "patch", ops: [wireOp(insertA, "peer", 1)], seq: 1 };
    socket.emit(patch);

    expect(received).toHaveLength(1);
    expect(received[0].ops[0].propDelta).toEqual(insertA);
    expect(received[0].seq).toBe(1);
    expect(transport.currentAckSeq).toBe(1);
  });

  it("routes a feature response frame to onFeature", async () => {
    const { transport, socket } = await connected();

    const received: FeatureServerMessage[] = [];
    transport.onFeature((f) => received.push(f));

    socket.emit({ type: "feature", response: { feature: "templateApplied", object_ids: ["o1", "o2"] } });
    expect(received).toHaveLength(1);
    expect(received[0].response).toEqual({ feature: "templateApplied", object_ids: ["o1", "o2"] });
  });

  it("routes presence frames to onPresence", async () => {
    const { transport, socket } = await connected();

    const received: PresenceServerMessage[] = [];
    transport.onPresence((p) => received.push(p));

    socket.emit({ type: "presence", payload: { cursor: { x: 1, y: 2 }, userId: "peer-9" } });
    expect(received).toHaveLength(1);
    expect(received[0].payload).toEqual({ cursor: { x: 1, y: 2 }, userId: "peer-9" });
  });

  it("routes ack frames to onAck and advances the resume cursor", async () => {
    const { transport, socket } = await connected();

    const acks: AckMessage[] = [];
    transport.onAck((a) => acks.push(a));

    socket.emit({ type: "ack", opIds: [{ clientId: "user-1", localSeq: 1 }], seq: 1, revision: 1 });
    expect(acks).toEqual([{ type: "ack", opIds: [{ clientId: "user-1", localSeq: 1 }], seq: 1, revision: 1 }]);
    expect(transport.currentAckSeq).toBe(1);
  });

  it("routes rejected frames to onRejected", async () => {
    const { transport, socket } = await connected();

    const rejections: string[][] = [];
    transport.onRejected((r) => rejections.push(r.errors));

    socket.emit({ type: "rejected", errors: ["unknown object id: missing"] });
    expect(rejections).toEqual([["unknown object id: missing"]]);
  });

  it("unsubscribe stops further patch deliveries", async () => {
    const { transport, socket } = await connected();

    let count = 0;
    const off = transport.onPatch(() => count++);
    socket.emit({ type: "patch", ops: [wireOp({ kind: "delete", id: "a" }, "peer", 1)], seq: 1 });
    off();
    socket.emit({ type: "patch", ops: [wireOp({ kind: "delete", id: "b" }, "peer", 2)], seq: 2 });
    expect(count).toBe(1);
  });
});

describe("wire round-trip against Rust-emitted JSON", () => {
  it("parses a Rust-emitted welcome frame for an empty new canvas", async () => {
    const { transport, getSocket } = makeTransport();
    const ready = transport.connect("c-ws");
    const socket = getSocket();
    socket.open();
    socket.emit(
      '{"type":"welcome","scene":{"sceneVersion":0,"objects":[],"tags":[],"selection":{"kind":"canvas"},"updatedAt":"1970-01-01T00:00:00Z"},"seq":0,"revision":0}'
    );
    const welcome = await ready;
    expect(welcome.seq).toBe(0);
    expect(welcome.scene.sceneVersion).toBe(0);
    expect(welcome.scene.objects).toEqual([]);
  });

  it("parses a Rust-emitted patch frame of WireOps with serde key order", async () => {
    const { transport, socket } = await connected();

    const received: PatchMessage[] = [];
    transport.onPatch((p) => received.push(p));

    socket.emit(
      '{"ops":[{"actor":"peer","baseRevision":1,"kind":"delete","objectId":"a","opId":{"clientId":"peer","localSeq":1},"propDelta":{"id":"a","kind":"delete"},"ts":"t"}],"seq":2,"type":"patch"}'
    );

    expect(received).toHaveLength(1);
    const op = received[0].ops[0];
    expect(op.kind).toBe("delete");
    expect(op.propDelta).toEqual({ kind: "delete", id: "a" });
    expect(received[0].seq).toBe(2);
  });

  it("parses a Rust-emitted ack frame", async () => {
    const { transport, socket } = await connected();

    const acks: AckMessage[] = [];
    transport.onAck((a) => acks.push(a));
    socket.emit('{"type":"ack","opIds":[{"clientId":"c1","localSeq":7}],"seq":1,"revision":1}');
    expect(acks).toEqual([{ type: "ack", opIds: [{ clientId: "c1", localSeq: 7 }], seq: 1, revision: 1 }]);
  });
});
