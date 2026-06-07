// Client realtime multiuser tests (MG-6 phase 2).
//
// These drive `SceneClient` over a MockWebSocket plus an injected manual timer,
// the same doubles the scene-client / ws-transport suites use. They prove the
// client side of the realtime tail:
//
//   - a peer `patch` frame applies to the local optimistic scene (MG6.1);
//   - a peer `patch` to a property the client is MID-DRAG on (an unacked local
//     write) is IGNORED until the local op is acked, then a later peer write
//     applies — transient ownership / no snap-back during a drag (MG6.4/MG4.4);
//   - a peer `presence` frame renders a peer cursor (MG6.2);
//   - the client never surfaces its OWN presence as a peer (self-skip, MG6.3);
//   - concurrent edits converge to the server-ordered (arrival-seq) result.
//
// The server is server-authoritative by arrival seq: it self-skips the sender on
// hello.userId, so a client never receives a `patch`/`presence` echo for what it
// authored — it reconciles its own ops from optimistic state + the `ack`, and
// every inbound `patch`/`presence` it sees is a peer's. These tests model that:
// the mock server only ever emits a peer's patch/presence to this client.

import { beforeEach, describe, expect, it } from "vitest";

import { SceneClient } from "../src/client/lib/sceneClient";
import { InMemoryOutboxStore } from "../src/client/lib/outbox";
import type { WebSocketLike } from "../src/client/lib/wsTransport";
import type { ClientMessage, ServerMessage, WelcomeMessage } from "../src/client/lib/transport";
import type { PeerPresence } from "../src/client/lib/peers";
import type { RenderScenePatch } from "../src/shared/renderPatch";
import type { Scene, SceneNode } from "../src/shared/schema";

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

/** A manual timer: armed callbacks are fired on demand by the test. */
function manualTimer() {
  const callbacks: (() => void)[] = [];
  return {
    setTimer: (fn: () => void, _ms: number) => {
      callbacks.push(fn);
      return callbacks.length as unknown;
    },
    clearTimer: (_h: unknown) => {
      /* no-op: fired callbacks are removed by fire() */
    },
    fireAll: () => {
      const pending = callbacks.splice(0);
      for (const cb of pending) cb();
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

function sceneNode(id: string, groupId: string, x = 10, y = 10): SceneNode {
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
    position: { x, y },
    size: { width: 120, height: 80 },
    zIndex: 0,
    updatedAt: "1970-01-01T00:00:00Z"
  };
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

const createN1: RenderScenePatch = {
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

/** A controllable wall clock for peer freshness/expiry. */
function manualClock() {
  let t = 1_000;
  return {
    nowMs: () => t,
    advance: (ms: number) => {
      t += ms;
    }
  };
}

async function boot(
  initial: Scene = emptyScene(0),
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

/** A scene with g1 + n1 already present, so move/edit ops validate. */
function seededScene(seq = 2): Scene {
  return {
    ...emptyScene(seq),
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
    ],
    nodes: [sceneNode("n1", "g1")]
  } as Scene;
}

function nodeById(scene: Scene | null, id: string): SceneNode | undefined {
  return scene?.nodes.find((n) => n.id === id);
}

// ---------------------------------------------------------------------------
// MG6.1 — remote apply.
// ---------------------------------------------------------------------------

describe("peer patch apply (MG6.1)", () => {
  it("applies a peer patch to the local scene", async () => {
    const { client, socket } = await boot();

    // A peer created g1 (the server fanned its op out to us as a `patch`).
    socket.emit({ type: "patch", ops: [groupG1], seq: 1 });

    expect(client.scene?.groups.map((g) => g.id)).toEqual(["g1"]);
  });

  it("applies a peer's create then move in arrival order", async () => {
    const { client, socket } = await boot();

    socket.emit({ type: "patch", ops: [groupG1], seq: 1 });
    socket.emit({ type: "patch", ops: [createN1], seq: 2 });
    socket.emit({ type: "patch", ops: [moveN1(99, 88)], seq: 3 });

    expect(nodeById(client.scene, "n1")?.position).toEqual({ x: 99, y: 88 });
  });
});

// ---------------------------------------------------------------------------
// MG6.4 / MG4.4 — transient ownership: no snap-back during a drag.
// ---------------------------------------------------------------------------

describe("mid-drag transient ownership (MG6.4)", () => {
  it("ignores a peer write to a property the client is mid-drag on, then applies after ack", async () => {
    const { client, timer, socket } = await boot(seededScene(2), 2);

    // The local user starts dragging n1: an optimistic, still-unacked move.
    const { opId } = await client.applyRenderPatch(moveN1(200, 200));
    expect(nodeById(client.scene, "n1")?.position).toEqual({ x: 200, y: 200 });
    timer.fireAll(); // flush the coalesced op onto the wire

    // A peer concurrently moves n1 to a different spot. Because the local move on
    // n1.position is unacked (mid-drag), the remote value is IGNORED — no
    // snap-back away from where the user is dragging.
    socket.emit({ type: "patch", ops: [moveN1(7, 7)], seq: 3 });
    expect(nodeById(client.scene, "n1")?.position).toEqual({ x: 200, y: 200 });

    // The server acks the local move; ownership is released.
    socket.emit({ type: "ack", opIds: [opId!], seq: 4, revision: 4 });
    await Promise.resolve();

    // A subsequent peer write to n1.position now applies (LWW convergence).
    socket.emit({ type: "patch", ops: [moveN1(7, 7)], seq: 5 });
    expect(nodeById(client.scene, "n1")?.position).toEqual({ x: 7, y: 7 });
  });

  it("applies a peer write to a DIFFERENT property while a drag is in flight", async () => {
    const { client, timer, socket } = await boot(seededScene(2), 2);

    // Local drag owns n1.position only.
    await client.applyRenderPatch(moveN1(200, 200));
    timer.fireAll();

    // A peer edits n1.title — a different (object,field) key — so it applies even
    // though the position drag is still unacked.
    socket.emit({
      type: "patch",
      ops: [{ kind: "edit-card-text", id: "n1", field: "title", value: "from-peer" }],
      seq: 3
    });

    expect(nodeById(client.scene, "n1")?.title).toBe("from-peer");
    // The unacked drag position is untouched.
    expect(nodeById(client.scene, "n1")?.position).toEqual({ x: 200, y: 200 });
  });
});

// ---------------------------------------------------------------------------
// MG6.2 / MG6.3 — presence cursors + self-skip.
// ---------------------------------------------------------------------------

describe("peer presence cursors (MG6.2/MG6.3)", () => {
  it("renders a peer cursor from an inbound presence frame", async () => {
    const { client, socket } = await boot();

    const updates: PeerPresence[][] = [];
    client.onPeers((peers) => updates.push(peers));

    socket.emit({ type: "presence", payload: { userId: "peer-9", cursor: { x: 12, y: 34 } } });

    expect(client.peerCursors).toHaveLength(1);
    expect(client.peerCursors[0]).toMatchObject({ userId: "peer-9", cursor: { x: 12, y: 34 } });
    // Subscribers were notified with the live peer set.
    expect(updates.at(-1)?.map((p) => p.userId)).toEqual(["peer-9"]);
    // Each peer gets a stable cursor color for the overlay.
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
    // The mock server self-skips, so the local frame normally never arrives — but
    // even if a frame tagged with our own userId reaches us, it must be dropped.
    const { client, socket } = await boot(emptyScene(0), 0, { userId: "user-1" });

    socket.emit({ type: "presence", payload: { userId: "user-1", cursor: { x: 5, y: 5 } } });
    socket.emit({ type: "presence", payload: { userId: "peer-2", cursor: { x: 9, y: 9 } } });

    expect(client.peerCursors.map((p) => p.userId)).toEqual(["peer-2"]);
  });

  it("stamps the local userId on the cursor frame it sends", async () => {
    const { client, socket } = await boot(emptyScene(0), 0, { userId: "user-1" });

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
    const { client, socket } = await boot(emptyScene(0), 0, { clock });

    socket.emit({ type: "presence", payload: { userId: "peer-9", cursor: { x: 1, y: 1 } } });
    expect(client.peerCursors).toHaveLength(1);

    // Advance past the default 10s TTL: the next read expires the stale peer
    // lazily (no background sweep timer — peerCursors expires on access).
    clock.advance(11_000);
    expect(client.peerCursors).toHaveLength(0);
  });

  it("expires a stale peer when a fresh peer frame arrives", async () => {
    const clock = manualClock();
    const { client, socket } = await boot(emptyScene(0), 0, { clock });

    socket.emit({ type: "presence", payload: { userId: "peer-9", cursor: { x: 1, y: 1 } } });
    clock.advance(11_000);
    // A different peer moves; ingest expires peer-9 and surfaces peer-2 only.
    socket.emit({ type: "presence", payload: { userId: "peer-2", cursor: { x: 2, y: 2 } } });

    expect(client.peerCursors.map((p) => p.userId)).toEqual(["peer-2"]);
  });
});

// ---------------------------------------------------------------------------
// Convergence — server-ordered result.
// ---------------------------------------------------------------------------

describe("concurrent-edit convergence", () => {
  it("converges to the server arrival-order result for concurrent edits on a shared property", async () => {
    // Two clients edit n1.position concurrently. The server linearises them by
    // arrival seq; THIS client, with no local write on n1, applies each peer
    // patch in seq order, so it lands on the last-arriving value.
    const { client, socket } = await boot(seededScene(2), 2);

    // Peer A then Peer B both moved n1; the server delivers them in arrival order.
    socket.emit({ type: "patch", ops: [moveN1(100, 100)], seq: 3 }); // peer A
    socket.emit({ type: "patch", ops: [moveN1(300, 400)], seq: 4 }); // peer B (later)

    // The later (server-ordered) write wins on both clients.
    expect(nodeById(client.scene, "n1")?.position).toEqual({ x: 300, y: 400 });
  });

  it("a self-acked op plus a later peer op converge to the peer's value", async () => {
    const { client, timer, socket } = await boot(seededScene(2), 2);

    // Local op (no echo back — server self-skips); acked so ownership releases.
    const { opId } = await client.applyRenderPatch(moveN1(50, 50));
    timer.fireAll();
    socket.emit({ type: "ack", opIds: [opId!], seq: 3, revision: 3 });
    await Promise.resolve();

    // A peer's later write arrives and converges (no longer owned).
    socket.emit({ type: "patch", ops: [moveN1(900, 900)], seq: 4 });
    expect(nodeById(client.scene, "n1")?.position).toEqual({ x: 900, y: 900 });
  });
});
