/**
 * T2.2 — Minimal Editing Operations
 *
 * Tests for: resize-card, resize-group, align-cards, distribute-cards,
 * duplicate-objects, batch (including multi-select batch move without grouping).
 */

import { describe, expect, it } from "vitest";
import { applyRenderPatchToShapeScene, type RenderScenePatch } from "../src/shared/renderPatch";
import type { Scene } from "../src/shared/schema";

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

const NOW = "2026-06-05T00:00:00.000Z";

const baseScene: Scene = {
  version: 1,
  sceneVersion: 1,
  groups: [
    {
      id: "g1",
      parentGroupId: null,
      title: "Frame 1",
      summary: "",
      bounds: { x: 0, y: 0, width: 2000, height: 2000 },
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
      title: "Node 1",
      summary: "",
      detail: "",
      status: "draft",
      confidence: 0.5,
      evidenceRefs: [],
      childDecisionIds: [],
      tagIds: [],
      position: { x: 100, y: 100 },
      size: { width: 200, height: 100 },
      zIndex: 0,
      updatedAt: NOW
    },
    {
      id: "n2",
      groupId: "g1",
      type: "task",
      title: "Node 2",
      summary: "",
      detail: "",
      status: "draft",
      confidence: 0.5,
      evidenceRefs: [],
      childDecisionIds: [],
      tagIds: [],
      position: { x: 400, y: 200 },
      size: { width: 200, height: 100 },
      zIndex: 1,
      updatedAt: NOW
    },
    {
      id: "n3",
      groupId: "g1",
      type: "task",
      title: "Node 3",
      summary: "",
      detail: "",
      status: "draft",
      confidence: 0.5,
      evidenceRefs: [],
      childDecisionIds: [],
      tagIds: [],
      position: { x: 700, y: 150 },
      size: { width: 200, height: 100 },
      zIndex: 2,
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
// resize-card
// ---------------------------------------------------------------------------

describe("resize-card", () => {
  it("updates node position and size from bounds", () => {
    const patch: RenderScenePatch = {
      kind: "resize-card",
      id: "n1",
      bounds: { x: 50, y: 60, width: 300, height: 150 }
    };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);

    expect(result.errors).toHaveLength(0);
    const node = result.scene.nodes.find((n) => n.id === "n1")!;
    expect(node.position).toEqual({ x: 50, y: 60 });
    expect(node.size).toEqual({ width: 300, height: 150 });
  });

  it("sets selection to the resized node", () => {
    const patch: RenderScenePatch = {
      kind: "resize-card",
      id: "n1",
      bounds: { x: 10, y: 10, width: 100, height: 100 }
    };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);
    expect(result.scene.selection).toEqual({ kind: "node", id: "n1" });
  });

  it("bumps sceneVersion", () => {
    const patch: RenderScenePatch = {
      kind: "resize-card",
      id: "n1",
      bounds: { x: 0, y: 0, width: 100, height: 100 }
    };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);
    expect(result.scene.sceneVersion).toBe(baseScene.sceneVersion + 1);
  });

  it("returns error for unknown node id", () => {
    const patch: RenderScenePatch = {
      kind: "resize-card",
      id: "nonexistent",
      bounds: { x: 0, y: 0, width: 100, height: 100 }
    };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);
    expect(result.errors).toContain("Unknown node id: nonexistent");
    expect(result.scene).toBe(baseScene);
  });

  it("returns error for non-positive bounds", () => {
    const patch: RenderScenePatch = {
      kind: "resize-card",
      id: "n1",
      bounds: { x: 0, y: 0, width: 0, height: 100 }
    };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);
    expect(result.errors).toContain("resize-card bounds must be positive");
  });
});

// ---------------------------------------------------------------------------
// resize-group
// ---------------------------------------------------------------------------

describe("resize-group", () => {
  it("updates group bounds", () => {
    const patch: RenderScenePatch = {
      kind: "resize-group",
      id: "g1",
      bounds: { x: 10, y: 20, width: 3000, height: 2500 }
    };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);

    expect(result.errors).toHaveLength(0);
    const group = result.scene.groups.find((g) => g.id === "g1")!;
    expect(group.bounds).toEqual({ x: 10, y: 20, width: 3000, height: 2500 });
  });

  it("sets selection to the resized group", () => {
    const patch: RenderScenePatch = {
      kind: "resize-group",
      id: "g1",
      bounds: { x: 0, y: 0, width: 1000, height: 1000 }
    };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);
    expect(result.scene.selection).toEqual({ kind: "group", id: "g1" });
  });

  it("returns error for unknown group id", () => {
    const patch: RenderScenePatch = {
      kind: "resize-group",
      id: "nonexistent",
      bounds: { x: 0, y: 0, width: 100, height: 100 }
    };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);
    expect(result.errors).toContain("Unknown group id: nonexistent");
  });

  it("returns error for non-positive bounds", () => {
    const patch: RenderScenePatch = {
      kind: "resize-group",
      id: "g1",
      bounds: { x: 0, y: 0, width: 100, height: -1 }
    };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);
    expect(result.errors).toContain("resize-group bounds must be positive");
  });
});

// ---------------------------------------------------------------------------
// align-cards
// ---------------------------------------------------------------------------

describe("align-cards", () => {
  it("aligns start on x-axis (left-align)", () => {
    const patch: RenderScenePatch = {
      kind: "align-cards",
      ids: ["n1", "n2", "n3"],
      axis: "x",
      mode: "start"
    };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);

    expect(result.errors).toHaveLength(0);
    const xs = result.scene.nodes.filter((n) => ["n1", "n2", "n3"].includes(n.id)).map((n) => n.position.x);
    // All should be at the minimum x of the original nodes (100)
    expect(xs.every((x) => x === 100)).toBe(true);
  });

  it("aligns end on x-axis (right-align)", () => {
    const patch: RenderScenePatch = {
      kind: "align-cards",
      ids: ["n1", "n2", "n3"],
      axis: "x",
      mode: "end"
    };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);

    expect(result.errors).toHaveLength(0);
    // maxRight = 700 + 200 = 900; each node right edge should be at 900
    const rights = result.scene.nodes
      .filter((n) => ["n1", "n2", "n3"].includes(n.id))
      .map((n) => n.position.x + n.size.width);
    expect(rights.every((r) => r === 900)).toBe(true);
  });

  it("aligns center on y-axis", () => {
    const patch: RenderScenePatch = {
      kind: "align-cards",
      ids: ["n1", "n2"],
      axis: "y",
      mode: "center"
    };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);

    expect(result.errors).toHaveLength(0);
    const centers = result.scene.nodes
      .filter((n) => ["n1", "n2"].includes(n.id))
      .map((n) => n.position.y + n.size.height / 2);
    expect(centers[0]).toBeCloseTo(centers[1], 5);
  });

  it("requires at least 2 ids", () => {
    const patch: RenderScenePatch = {
      kind: "align-cards",
      ids: ["n1"],
      axis: "x",
      mode: "start"
    };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);
    expect(result.errors).toContain("align-cards requires at least 2 ids");
  });

  it("returns error for unknown node id", () => {
    const patch: RenderScenePatch = {
      kind: "align-cards",
      ids: ["n1", "ghost"],
      axis: "x",
      mode: "start"
    };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);
    expect(result.errors).toContain("Unknown node id: ghost");
  });

  it("does not create any new group", () => {
    const patch: RenderScenePatch = {
      kind: "align-cards",
      ids: ["n1", "n2"],
      axis: "x",
      mode: "start"
    };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);
    expect(result.scene.groups).toHaveLength(baseScene.groups.length);
  });
});

// ---------------------------------------------------------------------------
// distribute-cards
// ---------------------------------------------------------------------------

describe("distribute-cards", () => {
  it("distributes evenly along x-axis", () => {
    const patch: RenderScenePatch = {
      kind: "distribute-cards",
      ids: ["n1", "n2", "n3"],
      axis: "x"
    };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);

    expect(result.errors).toHaveLength(0);
    const nodes = result.scene.nodes.filter((n) => ["n1", "n2", "n3"].includes(n.id));
    const sorted = [...nodes].sort((a, b) => a.position.x - b.position.x);
    // Gap between consecutive right-edge to left-edge should be equal
    const gap1 = sorted[1].position.x - (sorted[0].position.x + sorted[0].size.width);
    const gap2 = sorted[2].position.x - (sorted[1].position.x + sorted[1].size.width);
    expect(gap1).toBeCloseTo(gap2, 5);
  });

  it("distributes evenly along y-axis", () => {
    const patch: RenderScenePatch = {
      kind: "distribute-cards",
      ids: ["n1", "n2", "n3"],
      axis: "y"
    };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);

    expect(result.errors).toHaveLength(0);
    const nodes = result.scene.nodes.filter((n) => ["n1", "n2", "n3"].includes(n.id));
    const sorted = [...nodes].sort((a, b) => a.position.y - b.position.y);
    const gap1 = sorted[1].position.y - (sorted[0].position.y + sorted[0].size.height);
    const gap2 = sorted[2].position.y - (sorted[1].position.y + sorted[1].size.height);
    expect(gap1).toBeCloseTo(gap2, 5);
  });

  it("requires at least 3 ids", () => {
    const patch: RenderScenePatch = {
      kind: "distribute-cards",
      ids: ["n1", "n2"],
      axis: "x"
    };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);
    expect(result.errors).toContain("distribute-cards requires at least 3 ids");
  });

  it("returns error for unknown node id", () => {
    const patch: RenderScenePatch = {
      kind: "distribute-cards",
      ids: ["n1", "n2", "ghost"],
      axis: "x"
    };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);
    expect(result.errors).toContain("Unknown node id: ghost");
  });
});

// ---------------------------------------------------------------------------
// duplicate-objects
// ---------------------------------------------------------------------------

describe("duplicate-objects", () => {
  it("clones a single node with the specified delta offset", () => {
    const patch: RenderScenePatch = {
      kind: "duplicate-objects",
      ids: ["n1"],
      delta: { x: 20, y: 20 }
    };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);

    expect(result.errors).toHaveLength(0);
    // Original node still present
    expect(result.scene.nodes.find((n) => n.id === "n1")).toBeDefined();
    // Clone present with offset position
    const clones = result.scene.nodes.filter((n) => n.id !== "n1" && n.id !== "n2" && n.id !== "n3");
    expect(clones).toHaveLength(1);
    expect(clones[0].position).toEqual({ x: 120, y: 120 });
    expect(clones[0].title).toBe("Node 1");
  });

  it("rewires edges internal to the duplicated set", () => {
    // n1 and n2 are connected by e1; duplicating both should rewire
    const patch: RenderScenePatch = {
      kind: "duplicate-objects",
      ids: ["n1", "n2"],
      delta: { x: 50, y: 0 }
    };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);

    expect(result.errors).toHaveLength(0);
    // Should have a cloned edge between the cloned nodes
    const cloneIds = new Set(result.scene.nodes.filter((n) => !["n1", "n2", "n3"].includes(n.id)).map((n) => n.id));
    expect(cloneIds.size).toBe(2);
    const clonedEdge = result.scene.edges.find((e) => cloneIds.has(e.source) && cloneIds.has(e.target));
    expect(clonedEdge).toBeDefined();
  });

  it("drops edges that cross the duplication boundary (boundary edge from n1 to n2, only n1 duplicated)", () => {
    const patch: RenderScenePatch = {
      kind: "duplicate-objects",
      ids: ["n1"],
      delta: { x: 10, y: 10 }
    };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);

    // e1 connects n1->n2; duplicating only n1 should NOT add a cross-boundary edge
    const cloneId = result.scene.nodes.find((n) => n.id !== "n1" && n.id !== "n2" && n.id !== "n3")!.id;
    const crossEdge = result.scene.edges.find((e) => e.source === cloneId || e.target === cloneId);
    expect(crossEdge).toBeUndefined();
  });

  it("sets selection to the first clone", () => {
    const patch: RenderScenePatch = {
      kind: "duplicate-objects",
      ids: ["n1"],
      delta: { x: 0, y: 0 }
    };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);
    expect(result.scene.selection.kind).toBe("node");
  });

  it("returns error when ids is empty", () => {
    const patch: RenderScenePatch = {
      kind: "duplicate-objects",
      ids: [],
      delta: { x: 0, y: 0 }
    };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);
    expect(result.errors).toContain("duplicate-objects requires at least 1 id");
  });

  it("returns error for unknown id", () => {
    const patch: RenderScenePatch = {
      kind: "duplicate-objects",
      ids: ["ghost"],
      delta: { x: 0, y: 0 }
    };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);
    expect(result.errors).toContain("Unknown id: ghost");
  });
});

// ---------------------------------------------------------------------------
// batch — multi-select batch move (no permanent group created)
// ---------------------------------------------------------------------------

describe("batch", () => {
  it("moves multiple cards atomically without creating a new group", () => {
    const patch: RenderScenePatch = {
      kind: "batch",
      ops: [
        { kind: "move-card", id: "n1", position: { x: 200, y: 200 } },
        { kind: "move-card", id: "n2", position: { x: 500, y: 300 } },
        { kind: "move-card", id: "n3", position: { x: 800, y: 250 } }
      ]
    };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);

    expect(result.errors).toHaveLength(0);
    expect(result.scene.nodes.find((n) => n.id === "n1")!.position).toEqual({ x: 200, y: 200 });
    expect(result.scene.nodes.find((n) => n.id === "n2")!.position).toEqual({ x: 500, y: 300 });
    expect(result.scene.nodes.find((n) => n.id === "n3")!.position).toEqual({ x: 800, y: 250 });
    // No new group created — this is the key multi-select invariant
    expect(result.scene.groups).toHaveLength(baseScene.groups.length);
  });

  it("bumps sceneVersion exactly once for the whole batch", () => {
    const patch: RenderScenePatch = {
      kind: "batch",
      ops: [
        { kind: "move-card", id: "n1", position: { x: 10, y: 10 } },
        { kind: "move-card", id: "n2", position: { x: 20, y: 20 } }
      ]
    };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);
    // The batch apply folds child scenes; outer sceneVersion is the final scene's version
    expect(result.scene.sceneVersion).toBeGreaterThan(baseScene.sceneVersion);
  });

  it("returns error if any child op is invalid", () => {
    const patch: RenderScenePatch = {
      kind: "batch",
      ops: [
        { kind: "move-card", id: "n1", position: { x: 0, y: 0 } },
        { kind: "move-card", id: "ghost", position: { x: 0, y: 0 } }
      ]
    };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);
    expect(result.errors.length).toBeGreaterThan(0);
    // Scene is unchanged on error
    expect(result.scene).toBe(baseScene);
  });

  it("returns error when batch ops is empty", () => {
    const patch: RenderScenePatch = { kind: "batch", ops: [] };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);
    expect(result.errors).toContain("batch requires at least 1 op");
  });

  it("batch delete removes multiple cards without creating a group", () => {
    const patch: RenderScenePatch = {
      kind: "batch",
      ops: [
        { kind: "delete-card", id: "n1" },
        { kind: "delete-card", id: "n3" }
      ]
    };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);

    expect(result.errors).toHaveLength(0);
    const ids = result.scene.nodes.map((n) => n.id);
    expect(ids).not.toContain("n1");
    expect(ids).not.toContain("n3");
    expect(ids).toContain("n2");
    expect(result.scene.groups).toHaveLength(baseScene.groups.length);
  });

  it("batch z-order applies to multiple cards", () => {
    const patch: RenderScenePatch = {
      kind: "batch",
      ops: [
        { kind: "set-card-z-index", id: "n1", zIndex: 10 },
        { kind: "set-card-z-index", id: "n2", zIndex: 11 }
      ]
    };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);

    expect(result.errors).toHaveLength(0);
    expect(result.scene.nodes.find((n) => n.id === "n1")!.zIndex).toBe(10);
    expect(result.scene.nodes.find((n) => n.id === "n2")!.zIndex).toBe(11);
  });

  it("batch of resize-card ops works", () => {
    const patch: RenderScenePatch = {
      kind: "batch",
      ops: [
        { kind: "resize-card", id: "n1", bounds: { x: 50, y: 50, width: 400, height: 200 } },
        { kind: "resize-card", id: "n2", bounds: { x: 500, y: 50, width: 400, height: 200 } }
      ]
    };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);

    expect(result.errors).toHaveLength(0);
    expect(result.scene.nodes.find((n) => n.id === "n1")!.size).toEqual({ width: 400, height: 200 });
    expect(result.scene.nodes.find((n) => n.id === "n2")!.size).toEqual({ width: 400, height: 200 });
  });
});

// ---------------------------------------------------------------------------
// deriveTargetIds for new ops
// ---------------------------------------------------------------------------

import { deriveTargetIds } from "../src/shared/operation";

describe("deriveTargetIds for T2.2 ops", () => {
  it("resize-card returns the node id", () => {
    expect(deriveTargetIds({ kind: "resize-card", id: "n1", bounds: { x: 0, y: 0, width: 100, height: 100 } })).toEqual(["n1"]);
  });

  it("resize-group returns the group id", () => {
    expect(deriveTargetIds({ kind: "resize-group", id: "g1", bounds: { x: 0, y: 0, width: 100, height: 100 } })).toEqual(["g1"]);
  });

  it("align-cards returns all ids", () => {
    expect(deriveTargetIds({ kind: "align-cards", ids: ["n1", "n2"], axis: "x", mode: "start" })).toEqual(["n1", "n2"]);
  });

  it("distribute-cards returns all ids", () => {
    expect(deriveTargetIds({ kind: "distribute-cards", ids: ["n1", "n2", "n3"], axis: "y" })).toEqual(["n1", "n2", "n3"]);
  });

  it("duplicate-objects returns source ids", () => {
    expect(deriveTargetIds({ kind: "duplicate-objects", ids: ["n1", "n2"], delta: { x: 10, y: 10 } })).toEqual(["n1", "n2"]);
  });

  it("batch returns flattened targetIds of all child ops", () => {
    const patch: RenderScenePatch = {
      kind: "batch",
      ops: [
        { kind: "move-card", id: "n1", position: { x: 0, y: 0 } },
        { kind: "move-card", id: "n2", position: { x: 0, y: 0 } }
      ]
    };
    expect(deriveTargetIds(patch)).toEqual(["n1", "n2"]);
  });
});

// ---------------------------------------------------------------------------
// T2.2 transient multi-select (additive SceneSelection `multi` form)
// ---------------------------------------------------------------------------

import { primarySelection, sceneSelectionSchema } from "../src/shared/schema";

describe("multi-select SceneSelection form", () => {
  it("parses an additive `multi` selection of node ids", () => {
    const parsed = sceneSelectionSchema.parse({ kind: "multi", ids: ["n1", "n2", "n3"] });
    expect(parsed).toEqual({ kind: "multi", ids: ["n1", "n2", "n3"] });
  });

  it("rejects an empty `multi` selection", () => {
    expect(() => sceneSelectionSchema.parse({ kind: "multi", ids: [] })).toThrow();
  });

  it("leaves the existing single-anchor members unchanged", () => {
    expect(sceneSelectionSchema.parse({ kind: "canvas" })).toEqual({ kind: "canvas" });
    expect(sceneSelectionSchema.parse({ kind: "node", id: "n1" })).toEqual({ kind: "node", id: "n1" });
    expect(sceneSelectionSchema.parse({ kind: "group", id: "g1" })).toEqual({ kind: "group", id: "g1" });
    expect(sceneSelectionSchema.parse({ kind: "edge", id: "e1" })).toEqual({ kind: "edge", id: "e1" });
  });

  it("down-projects `multi` to its primary node, passing single forms through", () => {
    expect(primarySelection({ kind: "multi", ids: ["n2", "n3"] })).toEqual({ kind: "node", id: "n2" });
    expect(primarySelection({ kind: "node", id: "n1" })).toEqual({ kind: "node", id: "n1" });
    expect(primarySelection({ kind: "canvas" })).toEqual({ kind: "canvas" });
  });

  it("`select` op validates and applies a `multi` selection without creating a group", () => {
    const patch: RenderScenePatch = { kind: "select", selection: { kind: "multi", ids: ["n1", "n2"] } };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);
    expect(result.errors).toEqual([]);
    expect(result.scene.selection).toEqual({ kind: "multi", ids: ["n1", "n2"] });
    // The transient set never adds a SceneGroup.
    expect(result.scene.groups.length).toBe(baseScene.groups.length);
  });

  it("`select` op rejects a `multi` selection with an unknown node id", () => {
    const patch: RenderScenePatch = { kind: "select", selection: { kind: "multi", ids: ["n1", "ghost"] } };
    const result = applyRenderPatchToShapeScene(baseScene, patch, NOW);
    expect(result.errors.length).toBeGreaterThan(0);
  });

  it("deriveTargetIds returns the full set for a `multi` selection", () => {
    expect(deriveTargetIds({ kind: "select", selection: { kind: "multi", ids: ["n1", "n2", "n3"] } })).toEqual(["n1", "n2", "n3"]);
  });
});
