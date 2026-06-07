import { describe, expect, it } from "vitest";

import { WsTransport, type WebSocketLike } from "../src/client/lib/wsTransport";
import type {
  AckMessage,
  ClientMessage,
  PatchMessage,
  PresenceServerMessage,
  ServerMessage,
  WelcomeMessage
} from "../src/client/lib/transport";

// A minimal WebSocket double implementing the slice WsTransport drives. Tests
// drive the lifecycle manually: `open()` fires onopen, `emit(frame)` delivers a
// server frame, `sent` records the JSON strings the transport wrote.
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

  // --- test driver helpers ---
  open(): void {
    this.onopen?.({});
  }

  emit(frame: ServerMessage | string): void {
    const data = typeof frame === "string" ? frame : JSON.stringify(frame);
    this.onmessage?.({ data });
  }

  /** The decoded client frames the transport has sent so far. */
  sentMessages(): ClientMessage[] {
    return this.sent.map((s) => JSON.parse(s) as ClientMessage);
  }
}

type Connected = {
  transport: WsTransport;
  /** The mock socket created by the factory; valid after connect() is called. */
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

/** connect() + open + welcome; returns the live transport and its socket. */
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
  scene: {
    version: 1,
    sceneVersion: 0,
    groups: [],
    nodes: [],
    edges: [],
    tags: [],
    comments: [],
    artifacts: [],
    selection: { kind: "canvas" },
    updatedAt: "1970-01-01T00:00:00Z"
  } as unknown as WelcomeMessage["scene"],
  seq: 0,
  revision: 0
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

    // No frame sent until the socket actually opens.
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

    expect(socket.sentMessages()[0]).toEqual({
      type: "hello",
      canvasId: "c-ws",
      region,
      lastAckSeq: 0
    });

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
  it("emits an ops frame matching the protocol", async () => {
    const { transport, socket } = await connected();

    const createGroup = {
      kind: "create-group" as const,
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
    transport.sendOps([createGroup], { clientId: "user-1", baseRevision: 0 });

    // [0] is the hello; [1] is the ops frame. Each op is now an opId-stamped
    // envelope (evolved protocol): clientId/baseRevision live per-op.
    const frame = socket.sentMessages()[1] as { type: string; ops: unknown[] };
    expect(frame.type).toBe("ops");
    expect(frame.ops).toHaveLength(1);
    expect(frame.ops[0]).toMatchObject({
      opId: { clientId: "user-1", localSeq: 1 },
      baseRevision: 0,
      patch: createGroup
    });
  });

  it("mints a monotonic localSeq per envelope and a default baseRevision of 0", async () => {
    const { transport, socket } = await connected();

    const noop = { kind: "select" as const, selection: { kind: "canvas" as const } };
    transport.sendOps([noop], { clientId: "user-1" });
    transport.sendOps([noop], { clientId: "user-1" });
    const first = socket.sentMessages()[1] as { ops: { opId: { localSeq: number }; baseRevision: number }[] };
    const second = socket.sentMessages()[2] as { ops: { opId: { localSeq: number } }[] };
    expect(first.ops[0].baseRevision).toBe(0);
    expect(first.ops[0].opId.localSeq).toBe(1);
    expect(second.ops[0].opId.localSeq).toBe(2);
  });

  it("emits a presence frame with the canvas binding and verbatim payload", async () => {
    const { transport, socket } = await connected();

    const payload = { cursor: { x: 12, y: 34 }, userId: "user-1" };
    transport.sendPresence(payload);
    expect(socket.sentMessages()[1]).toEqual({ type: "presence", canvasId: "c-ws", payload });
  });
});

describe("WsTransport inbound routing", () => {
  it("routes a patch frame to onPatch with decoded ops", async () => {
    const { transport, socket } = await connected();

    const received: PatchMessage[] = [];
    transport.onPatch((p) => received.push(p));

    const patch: PatchMessage = {
      type: "patch",
      ops: [
        {
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
        }
      ],
      seq: 1
    };
    socket.emit(patch);

    expect(received).toHaveLength(1);
    expect(received[0].ops[0]).toEqual(patch.ops[0]);
    expect(received[0].seq).toBe(1);
    // patch.seq advances the resume cursor.
    expect(transport.currentAckSeq).toBe(1);
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

    socket.emit({ type: "rejected", errors: ['create-card target group "missing-group" not found'] });
    expect(rejections).toEqual([['create-card target group "missing-group" not found']]);
  });

  it("tolerates the self-echo patch arriving before its ack (interleave-agnostic)", async () => {
    const { transport, socket } = await connected();

    const events: string[] = [];
    transport.onPatch(() => events.push("patch"));
    transport.onAck(() => events.push("ack"));

    // Self patch fans out before the ack — the client must not assume order.
    socket.emit({
      type: "patch",
      ops: [{ kind: "delete-group", id: "g1" }],
      seq: 2
    });
    socket.emit({ type: "ack", opIds: [{ clientId: "user-1", localSeq: 1 }], seq: 2, revision: 2 });
    expect(events).toEqual(["patch", "ack"]);
  });

  it("unsubscribe stops further patch deliveries", async () => {
    const { transport, socket } = await connected();

    let count = 0;
    const off = transport.onPatch(() => count++);
    socket.emit({ type: "patch", ops: [{ kind: "delete-group", id: "g1" }], seq: 1 });
    off();
    socket.emit({ type: "patch", ops: [{ kind: "delete-group", id: "g2" }], seq: 2 });
    expect(count).toBe(1);
  });
});

describe("wire round-trip against Rust-emitted JSON", () => {
  // These strings are hard-coded from the documented server (serde) output. The
  // client must parse by key, not position — serde key order is not significant.
  it("parses a Rust-emitted welcome frame for an empty new canvas", async () => {
    const { transport, getSocket } = makeTransport();
    const ready = transport.connect("c-ws");
    const socket = getSocket();
    socket.open();
    socket.emit(
      '{"type":"welcome","scene":{"version":1,"sceneVersion":0,"groups":[],"nodes":[],"edges":[],"tags":[],"comments":[],"artifacts":[],"selection":"canvas","updatedAt":"1970-01-01T00:00:00Z"},"seq":0,"revision":0}'
    );
    const welcome = await ready;
    expect(welcome.seq).toBe(0);
    expect(welcome.revision).toBe(0);
    expect(welcome.scene.sceneVersion).toBe(0);
    expect(welcome.scene.groups).toEqual([]);
  });

  it("parses a Rust-emitted patch frame with serde key order", async () => {
    const { transport, socket } = await connected();

    const received: PatchMessage[] = [];
    transport.onPatch((p) => received.push(p));

    // Exact bytes as serde emits (keys alphabetised), proving key-based parsing.
    socket.emit(
      '{"ops":[{"group":{"bounds":{"height":300.0,"width":400.0,"x":0.0,"y":0.0},"id":"g1","styleKey":"","summary":"","tagIds":[],"title":"G","zIndex":0.0},"kind":"create-group"}],"seq":1,"type":"patch"}'
    );

    expect(received).toHaveLength(1);
    const op = received[0].ops[0];
    expect(op.kind).toBe("create-group");
    if (op.kind === "create-group") {
      expect(op.group.id).toBe("g1");
      expect(op.group.title).toBe("G");
      expect(op.group.bounds).toEqual({ x: 0, y: 0, width: 400, height: 300 });
    }
    expect(received[0].seq).toBe(1);
  });

  it("parses a Rust-emitted ack frame", async () => {
    const { transport, socket } = await connected();

    const acks: AckMessage[] = [];
    transport.onAck((a) => acks.push(a));
    // Exact bytes as serde emits the evolved ack (opIds + seq + revision).
    socket.emit('{"type":"ack","opIds":[{"clientId":"c1","localSeq":7}],"seq":1,"revision":1}');
    expect(acks).toEqual([
      { type: "ack", opIds: [{ clientId: "c1", localSeq: 7 }], seq: 1, revision: 1 }
    ]);
  });

  it("encodes an ops frame the server can decode (create-card min-accepted shape)", async () => {
    const { transport, socket } = await connected();

    transport.sendOps(
      [
        {
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
        }
      ],
      { clientId: "user-1", baseRevision: 0 }
    );

    const raw = JSON.parse(socket.sent[1]);
    expect(raw.type).toBe("ops");
    // Evolved protocol: clientId/baseRevision live inside each op envelope.
    const env = raw.ops[0];
    expect(env.opId).toEqual({ clientId: "user-1", localSeq: 1 });
    expect(env.baseRevision).toBe(0);
    expect(env.patch.kind).toBe("create-card");
    // The node "type" field is serde-renamed to "type" inside the card object.
    expect(env.patch.card.type).toBe("");
    expect(env.patch.card.id).toBe("n1");
    expect(env.patch.card.groupId).toBe("g1");
  });
});
