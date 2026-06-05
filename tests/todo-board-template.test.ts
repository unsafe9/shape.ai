/**
 * T4.2 — Todo / Task Board Template unit tests
 *
 * Verifies that:
 * 1. The todoBoardTemplate TemplateContract is well-formed.
 * 2. Applying it yields editable SceneGroup / SceneNode / SceneEdge primitives.
 * 3. The expected frame/card/edge composition (4 frames, 3 cards, 1 edge) is present.
 * 4. Status / owner / priority are in meta (not enum fields).
 * 5. Tags (status + priority chips) are seeded.
 * 6. Produced objects are editable by both human-style ops (move-card, delete-card,
 *    edit-card-text) and MCP-style ops (same shared applyRenderPatchToShapeScene path).
 */

import { describe, expect, it } from "vitest";
import { sceneGroupSchema, sceneNodeSchema, sceneEdgeSchema } from "../src/shared/schema";
import { applyRenderPatchToShapeScene } from "../src/shared/renderPatch";
import { applyTemplate } from "../src/shared/templates/contract";
import {
  todoBoardTemplate,
  todoBoardNodeMetaSchema,
  todoBoardFrameMetaSchema
} from "../src/shared/templates/todoBoard";
import type { SceneGroup, Scene } from "../src/shared/schema";

const ANCHOR = { x: 0, y: 0 };
const NOW = "2026-06-05T00:00:00.000Z";

/**
 * Build a minimal scene from an applied template result for op testing.
 *
 * AppliedTemplate exposes only the root group. To ensure nodes remain live
 * after commitAppPatch (which filters nodes by groupsById), we synthesise
 * a stub group for every unique groupId that appears on a node.  Stubs are
 * identical in shape to a valid SceneGroup so sceneGroupSchema still passes.
 */
function buildScene(applied: ReturnType<typeof applyTemplate>): Scene {
  // Collect all groupIds referenced by nodes and edges.
  const knownGroupIds = new Set<string>();
  if (applied.group) knownGroupIds.add(applied.group.id);
  for (const node of applied.nodes) knownGroupIds.add(node.groupId);
  for (const edge of applied.edges) knownGroupIds.add(edge.groupId);

  // Build a minimal stub group for any id not already covered by root group.
  const groups: SceneGroup[] = [];
  for (const gid of knownGroupIds) {
    if (applied.group && applied.group.id === gid) {
      groups.push(applied.group);
    } else {
      groups.push({
        id: gid,
        title: "stub-column",
        summary: "",
        bounds: { x: 0, y: 0, width: 400, height: 400 },
        tagIds: [],
        zIndex: 0,
        collapsed: false,
        parentGroupId: null,
        createdAt: NOW,
        updatedAt: NOW
      });
    }
  }

  return {
    version: 1,
    sceneVersion: 0,
    groups,
    nodes: applied.nodes,
    edges: applied.edges,
    tags: applied.newTags,
    comments: [],
    artifacts: [],
    selection: { kind: "canvas" },
    updatedAt: NOW
  };
}

// ---------------------------------------------------------------------------
// §1 — Template metadata
// ---------------------------------------------------------------------------

describe("todoBoardTemplate metadata", () => {
  it("has the correct id and templateKind", () => {
    expect(todoBoardTemplate.metadata.id).toBe("todo-board");
    expect(todoBoardTemplate.metadata.templateKind).toBe("todo");
  });

  it("is in the planning category", () => {
    expect(todoBoardTemplate.metadata.category).toBe("planning");
  });

  it("has a non-empty title and description", () => {
    expect(todoBoardTemplate.metadata.title.length).toBeGreaterThan(0);
    expect(todoBoardTemplate.metadata.description.length).toBeGreaterThan(0);
  });
});

// ---------------------------------------------------------------------------
// §2 — Recipe composition (frames / cards / edges)
// ---------------------------------------------------------------------------

describe("todoBoardTemplate recipe composition", () => {
  it("has 4 frames: 1 root board + 3 column frames", () => {
    expect(todoBoardTemplate.recipe.frames).toHaveLength(4);
    const ids = todoBoardTemplate.recipe.frames.map((f) => f.localId);
    expect(ids).toContain("f-board");
    expect(ids).toContain("f-todo");
    expect(ids).toContain("f-doing");
    expect(ids).toContain("f-done");
  });

  it("column frames have f-board as parentLocalId", () => {
    const columns = todoBoardTemplate.recipe.frames.filter((f) => f.parentLocalId);
    expect(columns).toHaveLength(3);
    for (const col of columns) {
      expect(col.parentLocalId).toBe("f-board");
    }
  });

  it("root frame has no parentLocalId", () => {
    const root = todoBoardTemplate.recipe.frames.find((f) => f.localId === "f-board");
    expect(root?.parentLocalId).toBeUndefined();
  });

  it("has 3 seed task cards", () => {
    expect(todoBoardTemplate.recipe.shapes).toHaveLength(3);
  });

  it("each card has styleKey task", () => {
    for (const shape of todoBoardTemplate.recipe.shapes) {
      expect(shape.styleKey).toBe("task");
    }
  });

  it("cards are distributed across all 3 columns", () => {
    const columns = todoBoardTemplate.recipe.shapes.map((s) => s.frameLocalId);
    expect(columns).toContain("f-todo");
    expect(columns).toContain("f-doing");
    expect(columns).toContain("f-done");
  });

  it("has 1 dependency edge with styleKey depends_on", () => {
    expect(todoBoardTemplate.recipe.edges).toHaveLength(1);
    const edge = todoBoardTemplate.recipe.edges[0];
    expect(edge.styleKey).toBe("depends_on");
    expect((edge.meta as Record<string, unknown>)?.semanticType).toBe("depends_on");
  });

  it("dependency edge connects t-2 (source) to t-1 (target)", () => {
    const edge = todoBoardTemplate.recipe.edges[0];
    expect(edge.sourceLocalId).toBe("t-2");
    expect(edge.targetLocalId).toBe("t-1");
  });

  it("recipe shapes carry tagLocalIds referencing status and priority chips", () => {
    for (const shape of todoBoardTemplate.recipe.shapes) {
      expect(Array.isArray(shape.tagLocalIds)).toBe(true);
      expect((shape.tagLocalIds ?? []).length).toBeGreaterThan(0);
    }
  });
});

// ---------------------------------------------------------------------------
// §4 — Exports
// ---------------------------------------------------------------------------

describe("todoBoardTemplate exports", () => {
  it("allows ai_plan_md and mermaid exports", () => {
    expect(todoBoardTemplate.exports.allowed).toContain("ai_plan_md");
    expect(todoBoardTemplate.exports.allowed).toContain("mermaid");
  });

  it("defaults to ai_plan_md", () => {
    expect(todoBoardTemplate.exports.default).toBe("ai_plan_md");
  });
});

// ---------------------------------------------------------------------------
// §5 — Suggested tags (status + priority chips)
// ---------------------------------------------------------------------------

describe("todoBoardTemplate tags", () => {
  it("seeds status chips for all 4 statuses", () => {
    const ids = todoBoardTemplate.tags.suggested.map((t) => t.localId);
    expect(ids).toContain("tg-status-todo");
    expect(ids).toContain("tg-status-doing");
    expect(ids).toContain("tg-status-done");
    expect(ids).toContain("tg-status-blocked");
  });

  it("seeds priority chips for low / normal / high", () => {
    const ids = todoBoardTemplate.tags.suggested.map((t) => t.localId);
    expect(ids).toContain("tg-priority-low");
    expect(ids).toContain("tg-priority-normal");
    expect(ids).toContain("tg-priority-high");
  });

  it("every suggested tag has a name and a hex color", () => {
    for (const tag of todoBoardTemplate.tags.suggested) {
      expect(tag.name.length).toBeGreaterThan(0);
      expect(tag.color).toMatch(/^#[0-9a-fA-F]{3,8}$/);
    }
  });
});

// ---------------------------------------------------------------------------
// §6 — Prompt hints
// ---------------------------------------------------------------------------

describe("todoBoardTemplate promptHints", () => {
  it("has a systemHint referencing todo board", () => {
    expect(todoBoardTemplate.promptHints?.systemHint).toContain("todo");
  });

  it("includes create-card in suggestedOperations", () => {
    expect(todoBoardTemplate.promptHints?.suggestedOperations).toContain("create-card");
  });

  it("includes add-comment in suggestedOperations (MCP add_comment path)", () => {
    expect(todoBoardTemplate.promptHints?.suggestedOperations).toContain("add-comment");
  });
});

// ---------------------------------------------------------------------------
// applyTemplate — produces normal canvas primitives
// ---------------------------------------------------------------------------

describe("applyTemplate(todoBoardTemplate) — primitive output", () => {
  it("applies without errors", () => {
    const result = applyTemplate(todoBoardTemplate, ANCHOR);
    expect(result.errors).toHaveLength(0);
  });

  it("produces a valid root SceneGroup", () => {
    const result = applyTemplate(todoBoardTemplate, ANCHOR);
    expect(result.group).toBeDefined();
    const parsed = sceneGroupSchema.safeParse(result.group);
    expect(parsed.success).toBe(true);
  });

  it("produces 3 SceneNodes (one per seed task card)", () => {
    const result = applyTemplate(todoBoardTemplate, ANCHOR);
    expect(result.nodes).toHaveLength(3);
  });

  it("produces 1 SceneEdge (the dependency)", () => {
    const result = applyTemplate(todoBoardTemplate, ANCHOR);
    expect(result.edges).toHaveLength(1);
  });

  it("all SceneNodes parse through sceneNodeSchema", () => {
    const result = applyTemplate(todoBoardTemplate, ANCHOR);
    for (const node of result.nodes) {
      const parsed = sceneNodeSchema.safeParse(node);
      expect(parsed.success).toBe(true);
    }
  });

  it("all SceneEdges parse through sceneEdgeSchema", () => {
    const result = applyTemplate(todoBoardTemplate, ANCHOR);
    for (const edge of result.edges) {
      const parsed = sceneEdgeSchema.safeParse(edge);
      expect(parsed.success).toBe(true);
    }
  });

  it("all nodes carry meta.templateKind=todo", () => {
    const result = applyTemplate(todoBoardTemplate, ANCHOR);
    for (const node of result.nodes) {
      expect((node.meta as Record<string, unknown>)?.templateKind).toBe("todo");
    }
  });

  it("produces 7 suggested Tag objects (4 status + 3 priority)", () => {
    const result = applyTemplate(todoBoardTemplate, ANCHOR);
    expect(result.newTags).toHaveLength(7);
  });

  it("edge connects two distinct node ids", () => {
    const result = applyTemplate(todoBoardTemplate, ANCHOR);
    const edge = result.edges[0];
    const nodeIds = new Set(result.nodes.map((n) => n.id));
    expect(nodeIds.has(edge.source)).toBe(true);
    expect(nodeIds.has(edge.target)).toBe(true);
    expect(edge.source).not.toBe(edge.target);
    expect(edge.label).toBe("blocks");
  });
});

// ---------------------------------------------------------------------------
// §4 — Status, owner, priority are in meta (not top-level schema enum fields)
// ---------------------------------------------------------------------------

describe("applyTemplate(todoBoardTemplate) — meta carries status/owner/priority", () => {
  it("task nodes carry meta.semanticType=task", () => {
    const result = applyTemplate(todoBoardTemplate, ANCHOR);
    for (const node of result.nodes) {
      expect((node.meta as Record<string, unknown>)?.semanticType).toBe("task");
    }
  });

  it("task nodes carry meta.status covering all three seed statuses", () => {
    const result = applyTemplate(todoBoardTemplate, ANCHOR);
    const statuses = result.nodes.map((n) => (n.meta as Record<string, unknown>)?.status);
    expect(statuses).toContain("todo");
    expect(statuses).toContain("doing");
    expect(statuses).toContain("done");
  });

  it("meta validates against todoBoardNodeMetaSchema for every task node", () => {
    const result = applyTemplate(todoBoardTemplate, ANCHOR);
    for (const node of result.nodes) {
      const parsed = todoBoardNodeMetaSchema.safeParse(node.meta);
      expect(parsed.success).toBe(true);
    }
  });

  it("task nodes carry meta.priority", () => {
    const result = applyTemplate(todoBoardTemplate, ANCHOR);
    for (const node of result.nodes) {
      const meta = node.meta as Record<string, unknown>;
      expect(["low", "normal", "high"]).toContain(meta?.priority);
    }
  });

  it("task nodes carry meta.owner as a string", () => {
    const result = applyTemplate(todoBoardTemplate, ANCHOR);
    for (const node of result.nodes) {
      const meta = node.meta as Record<string, unknown>;
      expect(typeof meta?.owner).toBe("string");
    }
  });
});

// ---------------------------------------------------------------------------
// §7 — Shared operation model: human and MCP edits use the same ops
// ---------------------------------------------------------------------------

describe("applyTemplate(todoBoardTemplate) — produced objects editable by standard ops", () => {
  it("move-card (human drag / MCP translate) moves a task node", () => {
    const result = applyTemplate(todoBoardTemplate, ANCHOR);
    const scene = buildScene(result);
    const node = result.nodes[0];
    const moved = applyRenderPatchToShapeScene(scene, {
      kind: "move-card",
      id: node.id,
      position: { x: 999, y: 888 }
    });
    expect(moved.errors).toHaveLength(0);
    const movedNode = moved.scene.nodes.find((n) => n.id === node.id)!;
    expect(movedNode).toBeDefined();
    expect(movedNode.position).toEqual({ x: 999, y: 888 });
  });

  it("edit-card-text (human text overlay / MCP patch_scene) renames a task", () => {
    const result = applyTemplate(todoBoardTemplate, ANCHOR);
    const scene = buildScene(result);
    const node = result.nodes[0];
    const edited = applyRenderPatchToShapeScene(scene, {
      kind: "edit-card-text",
      id: node.id,
      field: "title",
      value: "Renamed task"
    });
    expect(edited.errors).toHaveLength(0);
    const editedNode = edited.scene.nodes.find((n) => n.id === node.id)!;
    expect(editedNode).toBeDefined();
    expect(editedNode.title).toBe("Renamed task");
  });

  it("delete-card (human delete / MCP removeNodeIds) removes a task", () => {
    const result = applyTemplate(todoBoardTemplate, ANCHOR);
    const scene = buildScene(result);
    const node = result.nodes[0];
    const deleted = applyRenderPatchToShapeScene(scene, { kind: "delete-card", id: node.id });
    expect(deleted.errors).toHaveLength(0);
    expect(deleted.scene.nodes.find((n) => n.id === node.id)).toBeUndefined();
  });

  it("delete-card on the edge source removes the incident dependency edge", () => {
    const result = applyTemplate(todoBoardTemplate, ANCHOR);
    const scene = buildScene(result);
    // The "doing" task is the edge source (t-2 in the recipe → local id maps to some generated id)
    const t2 = result.nodes.find(
      (n) => (n.meta as Record<string, unknown>)?.status === "doing"
    )!;
    expect(t2).toBeDefined();
    const deleted = applyRenderPatchToShapeScene(scene, { kind: "delete-card", id: t2.id });
    expect(deleted.errors).toHaveLength(0);
    expect(deleted.scene.nodes.find((n) => n.id === t2.id)).toBeUndefined();
    // The dependency edge sourced at t2 should be removed.
    expect(deleted.scene.edges).toHaveLength(0);
  });

  it("create-edge (add dependency / MCP patch_scene) adds a new edge", () => {
    const result = applyTemplate(todoBoardTemplate, ANCHOR);
    const scene = buildScene(result);
    const [n0, n1] = result.nodes;
    // Use the root board group as the edge container (its id is known from result.group).
    const groupId = result.group.id;
    const newEdge = applyRenderPatchToShapeScene(scene, {
      kind: "create-edge",
      groupId,
      source: n1.id,
      target: n0.id,
      edgeId: "test-new-edge"
    });
    expect(newEdge.errors).toHaveLength(0);
    expect(newEdge.scene.edges.length).toBeGreaterThan(scene.edges.length);
  });

  it("delete-edge removes only the target edge; nodes remain intact", () => {
    const result = applyTemplate(todoBoardTemplate, ANCHOR);
    const scene = buildScene(result);
    const edge = result.edges[0];
    const deleted = applyRenderPatchToShapeScene(scene, { kind: "delete-edge", id: edge.id });
    expect(deleted.errors).toHaveLength(0);
    expect(deleted.scene.edges.find((e) => e.id === edge.id)).toBeUndefined();
    expect(deleted.scene.nodes).toHaveLength(3);
  });
});

// ---------------------------------------------------------------------------
// Meta schema validation helpers
// ---------------------------------------------------------------------------

describe("todoBoardNodeMetaSchema", () => {
  it("validates a complete task meta object", () => {
    const parsed = todoBoardNodeMetaSchema.safeParse({
      templateKind: "todo",
      semanticType: "task",
      status: "doing",
      owner: "alice",
      priority: "high"
    });
    expect(parsed.success).toBe(true);
  });

  it("applies defaults for status, owner, and priority", () => {
    const parsed = todoBoardNodeMetaSchema.safeParse({
      templateKind: "todo",
      semanticType: "task"
    });
    expect(parsed.success).toBe(true);
    if (parsed.success) {
      expect(parsed.data.status).toBe("todo");
      expect(parsed.data.owner).toBe("");
      expect(parsed.data.priority).toBe("normal");
    }
  });

  it("rejects unknown status values", () => {
    const parsed = todoBoardNodeMetaSchema.safeParse({
      templateKind: "todo",
      semanticType: "task",
      status: "pending"
    });
    expect(parsed.success).toBe(false);
  });

  it("rejects unknown priority values", () => {
    const parsed = todoBoardNodeMetaSchema.safeParse({
      templateKind: "todo",
      semanticType: "task",
      priority: "urgent"
    });
    expect(parsed.success).toBe(false);
  });
});

describe("todoBoardFrameMetaSchema", () => {
  it("validates a board frame meta", () => {
    const parsed = todoBoardFrameMetaSchema.safeParse({
      templateKind: "todo",
      semanticType: "board"
    });
    expect(parsed.success).toBe(true);
  });

  it("validates a column frame meta with column field", () => {
    const parsed = todoBoardFrameMetaSchema.safeParse({
      templateKind: "todo",
      semanticType: "column",
      column: "doing"
    });
    expect(parsed.success).toBe(true);
  });

  it("rejects unknown column values", () => {
    const parsed = todoBoardFrameMetaSchema.safeParse({
      templateKind: "todo",
      semanticType: "column",
      column: "backlog"
    });
    expect(parsed.success).toBe(false);
  });
});
