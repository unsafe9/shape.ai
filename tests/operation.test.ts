/**
 * T2.5 — Operation envelope + operation log tests
 *
 * Tests cover:
 *  1. synthesiseLocalEnvelope: correct defaults (actorType, actorId, clientId)
 *  2. deriveTargetIds: per-kind semantic target derivation
 *  3. appendToOperationLog: append-only immutability
 *  4. applyRenderPatchToShapeScene: envelope is threaded through (synthesised when omitted)
 *  5. applyRenderPatchToShapeScene: caller-supplied envelope is preserved unchanged
 *  6. Extended patch types are accepted in OperationEnvelope.patch
 */

import { describe, expect, it } from "vitest";
import {
  synthesiseLocalEnvelope,
  deriveTargetIds,
  createOperationLog,
  appendToOperationLog,
  entriesByActor,
  entriesByTarget,
  type OperationEnvelope,
  type ActorType
} from "../src/shared/operation";
import { applyRenderPatchToShapeScene, type RenderScenePatch } from "../src/shared/renderPatch";
import type { Scene, SceneSelection } from "../src/shared/schema";

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

const NOW = "2026-06-05T00:00:00.000Z";

const baseScene: Scene = {
  version: 1,
  sceneVersion: 7,
  groups: [
    {
      id: "g1",
      parentGroupId: null,
      title: "Frame 1",
      summary: "",
      bounds: { x: 0, y: 0, width: 800, height: 600 },
      tagIds: [],
      zIndex: 0,
      collapsed: false,
      createdAt: NOW,
      updatedAt: NOW
    }
  ],
  nodes: [
    {
      id: "n1",
      groupId: "g1",
      type: "task",
      title: "Task 1",
      summary: "",
      detail: "",
      status: "draft",
      confidence: 0.5,
      evidenceRefs: [],
      childDecisionIds: [],
      tagIds: [],
      position: { x: 100, y: 100 },
      size: { width: 390, height: 390 },
      zIndex: 0,
      updatedAt: NOW
    },
    {
      id: "n2",
      groupId: "g1",
      type: "task",
      title: "Task 2",
      summary: "",
      detail: "",
      status: "draft",
      confidence: 0.5,
      evidenceRefs: [],
      childDecisionIds: [],
      tagIds: [],
      position: { x: 500, y: 100 },
      size: { width: 390, height: 390 },
      zIndex: 0,
      updatedAt: NOW
    }
  ],
  edges: [
    {
      id: "e1",
      groupId: "g1",
      type: "supports",
      source: "n1",
      target: "n2",
      label: "supports",
      rationale: "",
      confidence: 0.5,
      tagIds: [],
      updatedAt: NOW
    }
  ],
  tags: [],
  comments: [],
  artifacts: [],
  selection: { kind: "canvas" },
  updatedAt: NOW
};

// ---------------------------------------------------------------------------
// 1. synthesiseLocalEnvelope
// ---------------------------------------------------------------------------

describe("synthesiseLocalEnvelope", () => {
  it("produces human actorType and local-shell clientId by default", () => {
    const patch: RenderScenePatch = { kind: "move-card", id: "n1", position: { x: 200, y: 200 } };
    const env = synthesiseLocalEnvelope(patch, 5, NOW);

    expect(env.actorType).toBe<ActorType>("human");
    expect(env.actorId).toBe("human");
    expect(env.clientId).toBe("local-shell");
  });

  it("sets baseRevision from the supplied value", () => {
    const patch: RenderScenePatch = { kind: "delete-card", id: "n1" };
    const env = synthesiseLocalEnvelope(patch, 42, NOW);
    expect(env.baseRevision).toBe(42);
  });

  it("sets timestamp to the supplied now string", () => {
    const patch: RenderScenePatch = { kind: "delete-edge", id: "e1" };
    const env = synthesiseLocalEnvelope(patch, 0, NOW);
    expect(env.timestamp).toBe(NOW);
  });

  it("generates a non-empty operationId", () => {
    const patch: RenderScenePatch = { kind: "delete-group", id: "g1" };
    const env = synthesiseLocalEnvelope(patch, 0, NOW);
    expect(env.operationId.length).toBeGreaterThan(0);
  });

  it("two calls produce different operationIds", () => {
    const patch: RenderScenePatch = { kind: "delete-group", id: "g1" };
    const a = synthesiseLocalEnvelope(patch, 0, NOW);
    const b = synthesiseLocalEnvelope(patch, 0, NOW);
    expect(a.operationId).not.toBe(b.operationId);
  });

  it("does not set sourceToolCall", () => {
    const patch: RenderScenePatch = { kind: "move-card", id: "n1", position: { x: 0, y: 0 } };
    const env = synthesiseLocalEnvelope(patch, 0, NOW);
    expect(env.sourceToolCall).toBeUndefined();
  });
});

// ---------------------------------------------------------------------------
// 2. deriveTargetIds
// ---------------------------------------------------------------------------

describe("deriveTargetIds", () => {
  it("create-card returns the card id", () => {
    const patch: RenderScenePatch = {
      kind: "create-card",
      card: {
        id: "n-new",
        groupId: "g1",
        type: "task",
        title: "New",
        summary: "",
        detail: "",
        status: "draft",
        bounds: { x: 0, y: 0, width: 100, height: 100 },
        zIndex: 0,
        styleKey: "default",
        accessibilityLabel: "New"
      }
    };
    expect(deriveTargetIds(patch)).toEqual(["n-new"]);
  });

  it("create-edge returns edgeId plus source and target", () => {
    const patch: RenderScenePatch = {
      kind: "create-edge",
      groupId: "g1",
      source: "n1",
      target: "n2",
      edgeId: "e-new"
    };
    expect(deriveTargetIds(patch)).toEqual(["e-new", "n1", "n2"]);
  });

  it("move-card returns the single node id", () => {
    const patch: RenderScenePatch = { kind: "move-card", id: "n1", position: { x: 0, y: 0 } };
    expect(deriveTargetIds(patch)).toEqual(["n1"]);
  });

  it("delete-card returns the single node id", () => {
    expect(deriveTargetIds({ kind: "delete-card", id: "n1" })).toEqual(["n1"]);
  });

  it("delete-edge returns the edge id", () => {
    expect(deriveTargetIds({ kind: "delete-edge", id: "e1" })).toEqual(["e1"]);
  });

  it("delete-group returns the group id", () => {
    expect(deriveTargetIds({ kind: "delete-group", id: "g1" })).toEqual(["g1"]);
  });

  it("canvas select returns empty targetIds", () => {
    const patch: RenderScenePatch = { kind: "select", selection: { kind: "canvas" } };
    expect(deriveTargetIds(patch)).toEqual([]);
  });

  it("node select returns the selected node id", () => {
    const patch: RenderScenePatch = { kind: "select", selection: { kind: "node", id: "n1" } };
    expect(deriveTargetIds(patch)).toEqual(["n1"]);
  });

  it("add-comment with canvas target returns empty targetIds", () => {
    expect(deriveTargetIds({ kind: "add-comment", target: { kind: "canvas" }, body: "hi" })).toEqual([]);
  });

  it("add-comment with node target returns the node id", () => {
    expect(deriveTargetIds({ kind: "add-comment", target: { kind: "node", id: "n1" }, body: "hi" })).toEqual(["n1"]);
  });

  it("export returns scopeIds", () => {
    expect(deriveTargetIds({ kind: "export", scopeIds: ["g1", "g2"], exportType: "madr" })).toEqual(["g1", "g2"]);
  });

  it("accept-proposal returns proposalId", () => {
    expect(deriveTargetIds({ kind: "accept-proposal", proposalId: "p1" })).toEqual(["p1"]);
  });

  it("reject-proposal returns proposalId", () => {
    expect(deriveTargetIds({ kind: "reject-proposal", proposalId: "p2" })).toEqual(["p2"]);
  });
});

// ---------------------------------------------------------------------------
// 3. Operation log — append-only immutability
// ---------------------------------------------------------------------------

describe("operation log", () => {
  it("starts empty", () => {
    const log = createOperationLog();
    expect(log.entries).toHaveLength(0);
  });

  it("appendToOperationLog returns a new log without mutating the original", () => {
    const log = createOperationLog();
    const patch: RenderScenePatch = { kind: "delete-card", id: "n1" };
    const env = synthesiseLocalEnvelope(patch, 0, NOW);
    const log2 = appendToOperationLog(log, env);

    expect(log.entries).toHaveLength(0);  // original unchanged
    expect(log2.entries).toHaveLength(1);
    expect(log2.entries[0]).toBe(env);
  });

  it("multiple appends preserve order", () => {
    let log = createOperationLog();
    const e1 = synthesiseLocalEnvelope({ kind: "delete-card", id: "n1" }, 0, NOW);
    const e2 = synthesiseLocalEnvelope({ kind: "delete-edge", id: "e1" }, 1, NOW);
    log = appendToOperationLog(log, e1);
    log = appendToOperationLog(log, e2);

    expect(log.entries).toHaveLength(2);
    expect(log.entries[0]).toBe(e1);
    expect(log.entries[1]).toBe(e2);
  });

  it("entriesByActor filters by actorId", () => {
    let log = createOperationLog();
    const humanEnv = synthesiseLocalEnvelope({ kind: "delete-card", id: "n1" }, 0, NOW);
    const mcpEnv: OperationEnvelope = {
      ...synthesiseLocalEnvelope({ kind: "delete-edge", id: "e1" }, 1, NOW),
      actorId: "mcp-agent",
      actorType: "mcp",
      clientId: "mcp-unknown"
    };
    log = appendToOperationLog(log, humanEnv);
    log = appendToOperationLog(log, mcpEnv);

    expect(entriesByActor(log, "human")).toHaveLength(1);
    expect(entriesByActor(log, "mcp-agent")).toHaveLength(1);
    expect(entriesByActor(log, "nobody")).toHaveLength(0);
  });

  it("entriesByTarget filters by targetId", () => {
    let log = createOperationLog();
    const env1 = synthesiseLocalEnvelope({ kind: "move-card", id: "n1", position: { x: 0, y: 0 } }, 0, NOW);
    const env2 = synthesiseLocalEnvelope({ kind: "move-card", id: "n2", position: { x: 0, y: 0 } }, 1, NOW);
    log = appendToOperationLog(log, env1);
    log = appendToOperationLog(log, env2);

    expect(entriesByTarget(log, "n1")).toHaveLength(1);
    expect(entriesByTarget(log, "n2")).toHaveLength(1);
    expect(entriesByTarget(log, "n3")).toHaveLength(0);
  });
});

// ---------------------------------------------------------------------------
// 4. applyRenderPatchToShapeScene — envelope synthesis when not supplied
// ---------------------------------------------------------------------------

describe("applyRenderPatchToShapeScene envelope synthesis", () => {
  it("result includes an envelope even when none is supplied", () => {
    const patch: RenderScenePatch = { kind: "move-card", id: "n1", position: { x: 200, y: 300 } };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);

    expect(result.envelope).toBeDefined();
    expect(result.envelope.actorType).toBe<ActorType>("human");
    expect(result.envelope.clientId).toBe("local-shell");
  });

  it("synthesised envelope has correct baseRevision from scene.sceneVersion", () => {
    const patch: RenderScenePatch = { kind: "move-card", id: "n1", position: { x: 0, y: 0 } };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);
    expect(result.envelope.baseRevision).toBe(baseScene.sceneVersion);
  });

  it("synthesised envelope targetIds match the patch", () => {
    const patch: RenderScenePatch = { kind: "move-card", id: "n1", position: { x: 0, y: 0 } };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);
    expect(result.envelope.targetIds).toEqual(["n1"]);
  });

  it("synthesised envelope is returned even on validation error", () => {
    const patch: RenderScenePatch = { kind: "move-card", id: "nonexistent", position: { x: 0, y: 0 } };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);
    expect(result.errors.length).toBeGreaterThan(0);
    expect(result.envelope).toBeDefined();
    expect(result.envelope.actorType).toBe("human");
  });
});

// ---------------------------------------------------------------------------
// 5. applyRenderPatchToShapeScene — caller-supplied envelope is preserved
// ---------------------------------------------------------------------------

describe("applyRenderPatchToShapeScene caller-supplied envelope", () => {
  it("returns the exact caller-supplied envelope", () => {
    const patch: RenderScenePatch = { kind: "delete-card", id: "n1" };
    const supplied: OperationEnvelope = {
      operationId: "my-op-id",
      actorId: "mcp-agent-99",
      actorType: "mcp",
      clientId: "mcp-unknown",
      targetIds: ["n1"],
      timestamp: NOW,
      baseRevision: baseScene.sceneVersion,
      sourceToolCall: { tool: "patch_scene", callId: "call-42" },
      patch
    };

    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW, supplied);
    expect(result.envelope).toBe(supplied);  // identity, not copy
    expect(result.envelope.operationId).toBe("my-op-id");
    expect(result.envelope.actorType).toBe("mcp");
    expect(result.envelope.sourceToolCall?.callId).toBe("call-42");
  });
});

// ---------------------------------------------------------------------------
// 6. ExtendedRenderPatch types are accepted in OperationEnvelope.patch
// ---------------------------------------------------------------------------

describe("OperationEnvelope accepts extended patch kinds", () => {
  it("add-comment patch is valid in the envelope", () => {
    const env: OperationEnvelope = {
      operationId: "op-comment",
      actorId: "human",
      actorType: "human",
      clientId: "local-shell",
      targetIds: ["n1"],
      timestamp: NOW,
      baseRevision: 0,
      patch: { kind: "add-comment", target: { kind: "node", id: "n1" }, body: "looks good" }
    };
    expect(env.patch.kind).toBe("add-comment");
  });

  it("export patch is valid in the envelope", () => {
    const env: OperationEnvelope = {
      operationId: "op-export",
      actorId: "mcp",
      actorType: "mcp",
      clientId: "mcp-unknown",
      targetIds: ["g1"],
      timestamp: NOW,
      baseRevision: 0,
      sourceToolCall: { tool: "export_group" },
      patch: { kind: "export", scopeIds: ["g1"], exportType: "madr" }
    };
    expect(env.patch.kind).toBe("export");
    expect(env.sourceToolCall?.tool).toBe("export_group");
  });

  it("accept-proposal patch is valid in the envelope", () => {
    const env: OperationEnvelope = {
      operationId: "op-accept",
      actorId: "human",
      actorType: "human",
      clientId: "local-shell",
      targetIds: ["p1"],
      timestamp: NOW,
      baseRevision: 0,
      patch: { kind: "accept-proposal", proposalId: "p1" }
    };
    expect(env.patch.kind).toBe("accept-proposal");
  });

  it("reject-proposal patch is valid in the envelope", () => {
    const env: OperationEnvelope = {
      operationId: "op-reject",
      actorId: "human",
      actorType: "human",
      clientId: "local-shell",
      targetIds: ["p2"],
      timestamp: NOW,
      baseRevision: 0,
      patch: { kind: "reject-proposal", proposalId: "p2" }
    };
    expect(env.patch.kind).toBe("reject-proposal");
  });
});
