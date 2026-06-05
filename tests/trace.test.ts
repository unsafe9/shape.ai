/**
 * T5.4 — Operation Trace & Write Preview tests
 *
 * Covers:
 *  1. TraceKind / TraceEvent / ReadTraceEvent types exist and are correct shape
 *  2. pushReadTrace: writes to the per-client ring, evicts oldest at capacity
 *  3. pushErrorTrace: writes an error entry to the ring
 *  4. projectReadRing: projects the ring into TraceEvent records (kind read/error)
 *  5. classifyWriteRisk: destructive ops → "destructive"; wide targetIds → "wide"; safe → null
 *  6. deriveTraceSummary: produces expected human strings per kind
 *  7. projectEventRow: maps raw event rows to TraceEvent projection
 *  8. WritePreview shape: all fields present and typed correctly
 *  9. READ_RING_CAPACITY: ring is bounded
 */

import { afterEach, describe, expect, it } from "vitest";
import {
  _resetClientRegistry,
  classifyWriteRisk,
  deriveTraceSummary,
  projectEventRow,
  projectReadRing,
  pushErrorTrace,
  pushReadTrace,
  READ_RING_CAPACITY,
  registerClient,
  type CanvasTarget,
  type ReadTraceEvent,
  type TraceEvent,
  type TraceKind,
  type WritePreview
} from "../src/server/mcpClients";
import type { Implementation } from "@modelcontextprotocol/sdk/types.js";

const mockImpl = (name: string): Implementation => ({ name, version: "1.0.0" });

afterEach(() => {
  _resetClientRegistry();
});

// ---------------------------------------------------------------------------
// 1. Type shape sanity (compile-time verified; these assertions confirm runtime shape)
// ---------------------------------------------------------------------------

describe("TraceEvent shape", () => {
  it("constructs a valid TraceEvent for kind:read", () => {
    const target: CanvasTarget = { kind: "group", id: "g-1" };
    const event: TraceEvent = {
      clientId: "client-1",
      kind: "read",
      target,
      verb: "get_group",
      operationId: null,
      callId: null,
      at: Date.now(),
      summary: "read group g-1",
      errorMessage: null
    };
    expect(event.kind).toBe("read");
    expect(event.operationId).toBeNull();
    expect(event.callId).toBeNull();
    expect(event.errorMessage).toBeNull();
  });

  it("constructs a valid TraceEvent for kind:write with operationId", () => {
    const event: TraceEvent = {
      clientId: "client-1",
      kind: "write",
      target: { kind: "node", id: "n-1" },
      verb: "move-card",
      operationId: "op-abc",
      callId: "call-42",
      at: Date.now(),
      summary: "move-card node n-1",
      errorMessage: null
    };
    expect(event.kind).toBe("write");
    expect(event.operationId).toBe("op-abc");
    expect(event.callId).toBe("call-42");
  });

  it("all TraceKind values are valid strings", () => {
    const kinds: TraceKind[] = [
      "read", "write", "comment", "export",
      "proposal-created", "proposal-accepted", "proposal-rejected", "error"
    ];
    expect(kinds).toHaveLength(8);
    for (const k of kinds) {
      expect(typeof k).toBe("string");
    }
  });
});

// ---------------------------------------------------------------------------
// 2. pushReadTrace: ring mutation and retrieval
// ---------------------------------------------------------------------------

describe("pushReadTrace", () => {
  it("pushes a read event into the client ring", () => {
    const client = registerClient(mockImpl("agent"), "http");
    const target: CanvasTarget = { kind: "group", id: "g-1" };
    pushReadTrace(client.clientId, { tool: "get_group", target, at: 1000 });

    const events = projectReadRing(client.clientId);
    expect(events).toHaveLength(1);
    expect(events[0].kind).toBe("read");
    expect(events[0].verb).toBe("get_group");
    expect(events[0].target).toEqual(target);
    expect(events[0].at).toBe(1000);
    expect(events[0].operationId).toBeNull();
  });

  it("pushes multiple events in order", () => {
    const client = registerClient(mockImpl("agent"), "http");
    const target: CanvasTarget = { kind: "canvas" };
    pushReadTrace(client.clientId, { tool: "list_groups", target, at: 1000 });
    pushReadTrace(client.clientId, { tool: "query_scene", target, at: 2000 });

    const events = projectReadRing(client.clientId);
    expect(events).toHaveLength(2);
    expect(events[0].verb).toBe("list_groups");
    expect(events[1].verb).toBe("query_scene");
  });

  it("is a no-op for unknown clientId", () => {
    expect(() =>
      pushReadTrace("nonexistent", { tool: "query_scene", target: { kind: "canvas" }, at: 0 })
    ).not.toThrow();
  });
});

// ---------------------------------------------------------------------------
// 3. pushErrorTrace: error entries in the ring
// ---------------------------------------------------------------------------

describe("pushErrorTrace", () => {
  it("pushes an error event and projects it as kind:error", () => {
    const client = registerClient(mockImpl("agent"), "http");
    const target: CanvasTarget = { kind: "group", id: "g-9" };
    pushErrorTrace(client.clientId, "get_group", target, "Group not found: g-9");

    const events = projectReadRing(client.clientId);
    expect(events).toHaveLength(1);
    expect(events[0].kind).toBe("error");
    expect(events[0].errorMessage).toBe("Group not found: g-9");
    expect(events[0].summary).toBe("error: Group not found: g-9");
    expect(events[0].target).toEqual(target);
  });

  it("is a no-op for unknown clientId", () => {
    expect(() =>
      pushErrorTrace("nonexistent", "get_group", { kind: "canvas" }, "fail")
    ).not.toThrow();
  });
});

// ---------------------------------------------------------------------------
// 4. projectReadRing: full projection
// ---------------------------------------------------------------------------

describe("projectReadRing", () => {
  it("returns empty array for unknown clientId", () => {
    expect(projectReadRing("nonexistent")).toEqual([]);
  });

  it("returns empty array for a fresh client", () => {
    const client = registerClient(mockImpl("fresh"), "http");
    expect(projectReadRing(client.clientId)).toEqual([]);
  });

  it("projects read events with null operationId and callId", () => {
    const client = registerClient(mockImpl("reader"), "http");
    pushReadTrace(client.clientId, {
      tool: "query_scene",
      target: { kind: "canvas" },
      at: 5000
    });
    const events = projectReadRing(client.clientId);
    expect(events[0].operationId).toBeNull();
    expect(events[0].callId).toBeNull();
    expect(events[0].clientId).toBe(client.clientId);
  });

  it("sets summary as 'read <target>' for read events", () => {
    const client = registerClient(mockImpl("reader"), "http");
    pushReadTrace(client.clientId, {
      tool: "get_group",
      target: { kind: "group", id: "g-auth" },
      at: 1
    });
    const [e] = projectReadRing(client.clientId);
    expect(e.summary).toBe("read group g-auth");
  });

  it("sets summary as 'error: <msg>' for error events", () => {
    const client = registerClient(mockImpl("reader"), "http");
    pushErrorTrace(client.clientId, "get_group", { kind: "canvas" }, "not found");
    const [e] = projectReadRing(client.clientId);
    expect(e.summary).toBe("error: not found");
  });
});

// ---------------------------------------------------------------------------
// 5. READ_RING_CAPACITY: ring is bounded
// ---------------------------------------------------------------------------

describe("read ring capacity", () => {
  it("evicts oldest entry when ring exceeds READ_RING_CAPACITY", () => {
    const client = registerClient(mockImpl("overflow"), "http");
    const target: CanvasTarget = { kind: "canvas" };

    // Fill ring to capacity + 1 (first entry should be evicted).
    for (let i = 0; i <= READ_RING_CAPACITY; i++) {
      pushReadTrace(client.clientId, { tool: `tool-${i}`, target, at: i });
    }

    const events = projectReadRing(client.clientId);
    expect(events).toHaveLength(READ_RING_CAPACITY);
    // tool-0 was evicted; tool-1 is now the oldest.
    expect(events[0].verb).toBe("tool-1");
    expect(events[READ_RING_CAPACITY - 1].verb).toBe(`tool-${READ_RING_CAPACITY}`);
  });

  it("READ_RING_CAPACITY is a positive number", () => {
    expect(READ_RING_CAPACITY).toBeGreaterThan(0);
  });
});

// ---------------------------------------------------------------------------
// 6. classifyWriteRisk: risk classifier
// ---------------------------------------------------------------------------

describe("classifyWriteRisk", () => {
  it("classifies delete-group as destructive", () => {
    expect(classifyWriteRisk({ kind: "delete-group", id: "g-1" }, ["g-1"])).toBe("destructive");
  });

  it("classifies delete-card as destructive", () => {
    expect(classifyWriteRisk({ kind: "delete-card", id: "n-1" }, ["n-1"])).toBe("destructive");
  });

  it("classifies delete-edge as destructive", () => {
    expect(classifyWriteRisk({ kind: "delete-edge", id: "e-1" }, ["e-1"])).toBe("destructive");
  });

  it("classifies a wide move-card (>=5 targets) as wide", () => {
    const ids = ["n-1", "n-2", "n-3", "n-4", "n-5"];
    expect(classifyWriteRisk({ kind: "move-card", id: "n-1", position: { x: 0, y: 0 } }, ids)).toBe("wide");
  });

  it("classifies a narrow move-card (<5 targets) as null (safe)", () => {
    expect(classifyWriteRisk({ kind: "move-card", id: "n-1", position: { x: 0, y: 0 } }, ["n-1"])).toBeNull();
  });

  it("classifies create-card as null (safe / additive)", () => {
    expect(classifyWriteRisk(
      {
        kind: "create-card",
        card: {
          id: "n-new", groupId: "g-1", type: "task", title: "T", summary: "", detail: "",
          status: "draft", bounds: { x: 0, y: 0, width: 100, height: 100 }, zIndex: 0,
          styleKey: "default", accessibilityLabel: "T"
        }
      },
      ["n-new"]
    )).toBeNull();
  });

  it("classifies edit-card-text as null (safe)", () => {
    expect(classifyWriteRisk(
      { kind: "edit-card-text", id: "n-1", field: "title", value: "New Title" },
      ["n-1"]
    )).toBeNull();
  });

  it("exactly 5 targets is classified as wide", () => {
    const ids = ["a", "b", "c", "d", "e"];
    expect(classifyWriteRisk({ kind: "edit-card-text", id: "a", field: "title", value: "x" }, ids)).toBe("wide");
  });

  it("4 targets is not wide (null)", () => {
    const ids = ["a", "b", "c", "d"];
    expect(classifyWriteRisk({ kind: "edit-card-text", id: "a", field: "title", value: "x" }, ids)).toBeNull();
  });

  it("destructive wins over wide: delete-group with many targets is still destructive", () => {
    const ids = Array.from({ length: 10 }, (_, i) => `g-${i}`);
    expect(classifyWriteRisk({ kind: "delete-group", id: "g-0" }, ids)).toBe("destructive");
  });
});

// ---------------------------------------------------------------------------
// 7. deriveTraceSummary
// ---------------------------------------------------------------------------

describe("deriveTraceSummary", () => {
  it("read + group target", () => {
    expect(deriveTraceSummary("read", "get_group", { kind: "group", id: "auth" }))
      .toBe("read group auth");
  });

  it("read + canvas target falls back to verb", () => {
    expect(deriveTraceSummary("read", "query_scene", { kind: "canvas" }))
      .toBe("read query_scene");
  });

  it("write + node target", () => {
    expect(deriveTraceSummary("write", "move-card", { kind: "node", id: "n-1" }))
      .toBe("move-card node n-1");
  });

  it("comment + group target", () => {
    expect(deriveTraceSummary("comment", "add-comment", { kind: "group", id: "g-1" }))
      .toBe("commented on group g-1");
  });

  it("comment + canvas target", () => {
    expect(deriveTraceSummary("comment", "add-comment", { kind: "canvas" }))
      .toBe("commented");
  });

  it("export + artifact target", () => {
    expect(deriveTraceSummary("export", "export", { kind: "artifact", artifactId: "art-1", groupId: "g-1" }))
      .toBe("exported artifact art-1");
  });

  it("proposal-created", () => {
    expect(deriveTraceSummary("proposal-created", "delete-card", { kind: "node", id: "n-2" }))
      .toBe("proposed: delete-card node n-2 (pending)");
  });

  it("proposal-accepted", () => {
    expect(deriveTraceSummary("proposal-accepted", "delete-card", { kind: "node", id: "n-2" }))
      .toBe("accepted proposal → delete-card node n-2");
  });

  it("proposal-rejected (target is irrelevant)", () => {
    expect(deriveTraceSummary("proposal-rejected", "delete-card", { kind: "canvas" }))
      .toBe("rejected proposal (no change)");
  });

  it("error + group target", () => {
    expect(deriveTraceSummary("error", "get_group", { kind: "group", id: "g-9" }))
      .toBe("error on group g-9");
  });

  it("selection target description includes count", () => {
    const sel: CanvasTarget = {
      kind: "selection",
      ids: [{ kind: "node", id: "n-1" }, { kind: "node", id: "n-2" }]
    };
    expect(deriveTraceSummary("write", "batch", sel))
      .toBe("batch selection (2 items)");
  });

  it("viewport target", () => {
    const vp: CanvasTarget = { kind: "viewport", rect: { x: 0, y: 0, width: 1000, height: 800 } };
    expect(deriveTraceSummary("read", "query_scene", vp))
      .toBe("read viewport");
  });
});

// ---------------------------------------------------------------------------
// 8. projectEventRow: events table row → TraceEvent
// ---------------------------------------------------------------------------

describe("projectEventRow", () => {
  const clientId = "client-test";

  it("projects a write event row", () => {
    const row = {
      id: "op-1",
      type: "move-card",
      payloadJson: JSON.stringify({
        clientId,
        targetIds: ["n-1"],
        sourceToolCall: { tool: "patch_scene", callId: "call-99" }
      }),
      createdAt: "2026-06-05T10:00:00.000Z"
    };
    const event = projectEventRow(clientId, row);
    expect(event.kind).toBe("write");
    expect(event.operationId).toBe("op-1");
    expect(event.verb).toBe("move-card");
    expect(event.callId).toBe("call-99");
    expect(event.errorMessage).toBeNull();
    expect(event.clientId).toBe(clientId);
    expect(event.at).toBe(new Date("2026-06-05T10:00:00.000Z").getTime());
  });

  it("projects an add-comment row as kind:comment", () => {
    const row = {
      id: "op-2",
      type: "add-comment",
      payloadJson: JSON.stringify({ clientId, targetIds: ["n-1"] }),
      createdAt: "2026-06-05T10:01:00.000Z"
    };
    const event = projectEventRow(clientId, row);
    expect(event.kind).toBe("comment");
    expect(event.summary).toContain("comment");
  });

  it("projects an export row as kind:export", () => {
    const row = {
      id: "op-3",
      type: "export",
      payloadJson: JSON.stringify({ clientId, targetIds: ["g-1"] }),
      createdAt: "2026-06-05T10:02:00.000Z"
    };
    expect(projectEventRow(clientId, row).kind).toBe("export");
  });

  it("projects an accept-proposal row as kind:proposal-accepted", () => {
    const row = {
      id: "op-4",
      type: "accept-proposal",
      payloadJson: JSON.stringify({ clientId, targetIds: ["p-1"] }),
      createdAt: "2026-06-05T10:03:00.000Z"
    };
    expect(projectEventRow(clientId, row).kind).toBe("proposal-accepted");
  });

  it("projects a reject-proposal row as kind:proposal-rejected", () => {
    const row = {
      id: "op-5",
      type: "reject-proposal",
      payloadJson: JSON.stringify({ clientId, targetIds: ["p-2"] }),
      createdAt: "2026-06-05T10:04:00.000Z"
    };
    expect(projectEventRow(clientId, row).kind).toBe("proposal-rejected");
  });

  it("handles malformed payloadJson gracefully (no throw, target defaults to canvas)", () => {
    const row = {
      id: "op-bad",
      type: "move-card",
      payloadJson: "not-json{{{",
      createdAt: "2026-06-05T10:05:00.000Z"
    };
    expect(() => projectEventRow(clientId, row)).not.toThrow();
    const event = projectEventRow(clientId, row);
    expect(event.target).toEqual({ kind: "canvas" });
  });

  it("handles empty targetIds (canvas fallback)", () => {
    const row = {
      id: "op-6",
      type: "create-tag",
      payloadJson: JSON.stringify({ clientId, targetIds: [] }),
      createdAt: "2026-06-05T10:06:00.000Z"
    };
    const event = projectEventRow(clientId, row);
    expect(event.target).toEqual({ kind: "canvas" });
  });

  it("resolves group target by 'g' prefix convention", () => {
    const row = {
      id: "op-7",
      type: "delete-group",
      payloadJson: JSON.stringify({ clientId, targetIds: ["g-auth"] }),
      createdAt: "2026-06-05T10:07:00.000Z"
    };
    const event = projectEventRow(clientId, row);
    expect(event.target).toEqual({ kind: "group", id: "g-auth" });
  });

  it("resolves edge target by 'e' prefix convention", () => {
    const row = {
      id: "op-8",
      type: "delete-edge",
      payloadJson: JSON.stringify({ clientId, targetIds: ["e-1"] }),
      createdAt: "2026-06-05T10:08:00.000Z"
    };
    const event = projectEventRow(clientId, row);
    expect(event.target).toEqual({ kind: "edge", id: "e-1" });
  });

  it("resolves multi-target as selection", () => {
    const row = {
      id: "op-9",
      type: "batch",
      payloadJson: JSON.stringify({ clientId, targetIds: ["n-1", "n-2"] }),
      createdAt: "2026-06-05T10:09:00.000Z"
    };
    const event = projectEventRow(clientId, row);
    expect(event.target.kind).toBe("selection");
    if (event.target.kind === "selection") {
      expect(event.target.ids).toHaveLength(2);
    }
  });
});

// ---------------------------------------------------------------------------
// 9. WritePreview type shape
// ---------------------------------------------------------------------------

describe("WritePreview shape", () => {
  it("constructs a valid WritePreview for a destructive op", () => {
    const preview: WritePreview = {
      proposalId: "proposal-1",
      clientId: "client-1",
      riskClass: "destructive",
      verb: "delete-group",
      targetIds: ["g-1"],
      stagedAt: Date.now()
    };
    expect(preview.riskClass).toBe("destructive");
    expect(preview.verb).toBe("delete-group");
    expect(preview.targetIds).toEqual(["g-1"]);
  });

  it("accepts all three riskClass values", () => {
    const destructive: WritePreview["riskClass"] = "destructive";
    const wide: WritePreview["riskClass"] = "wide";
    const agentFlagged: WritePreview["riskClass"] = "agent-flagged";
    expect([destructive, wide, agentFlagged]).toHaveLength(3);
  });
});

// ---------------------------------------------------------------------------
// 10. McpClientIdentity.readRing initializes empty
// ---------------------------------------------------------------------------

describe("McpClientIdentity readRing", () => {
  it("starts with an empty readRing", () => {
    const client = registerClient(mockImpl("fresh"), "http");
    // Access via projectReadRing which reflects the internal ring.
    const events = projectReadRing(client.clientId);
    expect(events).toHaveLength(0);
  });

  it("ring is per-client (two clients don't share entries)", () => {
    const a = registerClient(mockImpl("agent-a"), "http");
    const b = registerClient(mockImpl("agent-b"), "http");
    pushReadTrace(a.clientId, { tool: "query_scene", target: { kind: "canvas" }, at: 1 });

    expect(projectReadRing(a.clientId)).toHaveLength(1);
    expect(projectReadRing(b.clientId)).toHaveLength(0);
  });
});
