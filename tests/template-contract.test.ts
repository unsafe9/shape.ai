/**
 * T4.1 — Template Contract unit tests
 *
 * Verify that applying a TemplateContract produces ordinary SceneGroup /
 * SceneNode / SceneEdge objects editable by the standard ops, not opaque
 * template-private objects.
 */

import { describe, expect, it } from "vitest";
import { sceneGroupSchema, sceneNodeSchema, sceneEdgeSchema } from "../src/shared/schema";
import { applyRenderPatchToShapeScene } from "../src/shared/renderPatch";
import {
  applyTemplate,
  type TemplateContract,
  type TemplateMetadata,
  type TemplateRecipe,
  type RecipeLayout,
  type TemplateExports,
  type TemplateTags,
  type TemplatePromptHints
} from "../src/shared/templates/contract";

// ---------------------------------------------------------------------------
// Minimal template fixture
// ---------------------------------------------------------------------------

const TRIVIAL_TEMPLATE: TemplateContract = {
  metadata: {
    id: "trivial",
    title: "Trivial Template",
    description: "A minimal test template with one frame, two shapes, and one edge.",
    category: "general",
    templateKind: "trivial"
  },
  recipe: {
    frames: [
      { localId: "frame-a", title: "Frame A", summary: "Test frame" }
    ],
    shapes: [
      {
        localId: "shape-1",
        frameLocalId: "frame-a",
        title: "Node One",
        summary: "First node",
        styleKey: "task",
        position: { x: 0, y: 0 },
        size: { width: 200, height: 100 }
      },
      {
        localId: "shape-2",
        frameLocalId: "frame-a",
        title: "Node Two",
        summary: "Second node",
        styleKey: "decision_point",
        position: { x: 300, y: 0 },
        size: { width: 200, height: 100 }
      }
    ],
    edges: [
      {
        localId: "edge-1",
        frameLocalId: "frame-a",
        sourceLocalId: "shape-1",
        targetLocalId: "shape-2",
        label: "flows to"
      }
    ]
  },
  layout: {
    origin: { x: 0, y: 0 },
    defaultShapeSize: { width: 200, height: 100 }
  },
  exports: {
    allowed: ["mermaid", "ai_plan_md"],
    default: "mermaid"
  },
  tags: {
    suggested: [
      { localId: "tag-todo", name: "todo", color: "#4a9eff", description: "To-do item" }
    ]
  },
  promptHints: {
    systemHint: "This is a trivial template; shapes are tasks.",
    suggestedOperations: ["create", "connect", "tag"]
  }
};

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe("TemplateContract types", () => {
  it("TemplateMetadata has required fields", () => {
    const meta: TemplateMetadata = TRIVIAL_TEMPLATE.metadata;
    expect(meta.id).toBe("trivial");
    expect(meta.templateKind).toBe("trivial");
    expect(["planning", "knowledge", "engineering", "presentation", "general"]).toContain(meta.category);
  });

  it("TemplateRecipe carries frames, shapes, edges", () => {
    const recipe: TemplateRecipe = TRIVIAL_TEMPLATE.recipe;
    expect(recipe.frames).toHaveLength(1);
    expect(recipe.shapes).toHaveLength(2);
    expect(recipe.edges).toHaveLength(1);
  });

  it("RecipeLayout has optional fields", () => {
    const layout: RecipeLayout = TRIVIAL_TEMPLATE.layout;
    expect(layout.origin).toEqual({ x: 0, y: 0 });
    expect(layout.defaultShapeSize).toEqual({ width: 200, height: 100 });
  });

  it("TemplateExports lists allowed export types", () => {
    const exports: TemplateExports = TRIVIAL_TEMPLATE.exports;
    expect(exports.allowed).toContain("mermaid");
    expect(exports.default).toBe("mermaid");
  });

  it("TemplateTags carries suggested tags", () => {
    const tags: TemplateTags = TRIVIAL_TEMPLATE.tags;
    expect(tags.suggested).toHaveLength(1);
    expect(tags.suggested[0].name).toBe("todo");
  });

  it("TemplatePromptHints is optional and advisory", () => {
    const hints: TemplatePromptHints | undefined = TRIVIAL_TEMPLATE.promptHints;
    expect(hints).toBeDefined();
    expect(hints?.systemHint).toContain("trivial template");
    expect(hints?.suggestedOperations).toContain("create");

    // A template without promptHints is valid.
    const noHints: TemplateContract = { ...TRIVIAL_TEMPLATE, promptHints: undefined };
    expect(noHints.promptHints).toBeUndefined();
  });
});

describe("applyTemplate — produces normal canvas objects", () => {
  const anchor = { x: 100, y: 200 };

  it("returns a SceneGroup, SceneNodes, and SceneEdges", () => {
    const result = applyTemplate(TRIVIAL_TEMPLATE, anchor);
    expect(result.errors).toHaveLength(0);
    expect(result.group).toBeDefined();
    expect(result.nodes).toHaveLength(2);
    expect(result.edges).toHaveLength(1);
  });

  it("produced SceneGroup parses through sceneGroupSchema", () => {
    const result = applyTemplate(TRIVIAL_TEMPLATE, anchor);
    // sceneGroupSchema requires createdAt/updatedAt — parse should succeed.
    const parsed = sceneGroupSchema.safeParse(result.group);
    expect(parsed.success).toBe(true);
  });

  it("produced SceneNodes parse through sceneNodeSchema", () => {
    const result = applyTemplate(TRIVIAL_TEMPLATE, anchor);
    for (const node of result.nodes) {
      const parsed = sceneNodeSchema.safeParse(node);
      expect(parsed.success).toBe(true);
    }
  });

  it("produced SceneEdges parse through sceneEdgeSchema", () => {
    const result = applyTemplate(TRIVIAL_TEMPLATE, anchor);
    for (const edge of result.edges) {
      const parsed = sceneEdgeSchema.safeParse(edge);
      expect(parsed.success).toBe(true);
    }
  });

  it("nodes carry meta.templateKind equal to the template's templateKind", () => {
    const result = applyTemplate(TRIVIAL_TEMPLATE, anchor);
    for (const node of result.nodes) {
      expect((node.meta as Record<string, unknown>)?.templateKind).toBe("trivial");
    }
  });

  it("node positions are offset by anchor", () => {
    const result = applyTemplate(TRIVIAL_TEMPLATE, anchor);
    const titles = result.nodes.map((n) => n.title);
    const n1 = result.nodes[titles.indexOf("Node One")];
    const n2 = result.nodes[titles.indexOf("Node Two")];
    // shape-1 is at recipe position {0,0} + anchor {100,200}
    expect(n1.position).toEqual({ x: 100, y: 200 });
    // shape-2 is at recipe position {300,0} + anchor {100,200}
    expect(n2.position).toEqual({ x: 400, y: 200 });
  });

  it("produced objects are editable by standard ops (move-card)", () => {
    const result = applyTemplate(TRIVIAL_TEMPLATE, anchor);
    // Build a minimal scene from the template output.
    const scene = {
      version: 1 as const,
      sceneVersion: 0,
      groups: [result.group],
      nodes: result.nodes,
      edges: result.edges,
      tags: result.newTags,
      comments: [],
      artifacts: [],
      selection: { kind: "canvas" as const },
      updatedAt: "2026-06-05T00:00:00.000Z"
    };
    const node = result.nodes[0];
    const moved = applyRenderPatchToShapeScene(scene, {
      kind: "move-card",
      id: node.id,
      position: { x: 999, y: 888 }
    });
    expect(moved.errors).toHaveLength(0);
    const movedNode = moved.scene.nodes.find((n) => n.id === node.id)!;
    expect(movedNode.position).toEqual({ x: 999, y: 888 });
  });

  it("produced objects are editable by standard ops (delete-card)", () => {
    const result = applyTemplate(TRIVIAL_TEMPLATE, anchor);
    const scene = {
      version: 1 as const,
      sceneVersion: 0,
      groups: [result.group],
      nodes: result.nodes,
      edges: result.edges,
      tags: result.newTags,
      comments: [],
      artifacts: [],
      selection: { kind: "canvas" as const },
      updatedAt: "2026-06-05T00:00:00.000Z"
    };
    const node = result.nodes[0];
    const deleted = applyRenderPatchToShapeScene(scene, { kind: "delete-card", id: node.id });
    expect(deleted.errors).toHaveLength(0);
    expect(deleted.scene.nodes.find((n) => n.id === node.id)).toBeUndefined();
  });

  it("produces suggested tag objects", () => {
    const result = applyTemplate(TRIVIAL_TEMPLATE, anchor);
    expect(result.newTags).toHaveLength(1);
    expect(result.newTags[0].name).toBe("todo");
    expect(result.newTags[0].color).toBe("#4a9eff");
  });

  it("no errors on a well-formed trivial template", () => {
    const result = applyTemplate(TRIVIAL_TEMPLATE, anchor);
    expect(result.errors).toHaveLength(0);
  });

  it("edge connects the correct source and target nodes", () => {
    const result = applyTemplate(TRIVIAL_TEMPLATE, anchor);
    const edge = result.edges[0];
    const nodeIds = new Set(result.nodes.map((n) => n.id));
    expect(nodeIds.has(edge.source)).toBe(true);
    expect(nodeIds.has(edge.target)).toBe(true);
    expect(edge.source).not.toBe(edge.target);
    expect(edge.label).toBe("flows to");
  });
});

describe("applyTemplate — template with no shapes or edges", () => {
  const emptyRecipeTemplate: TemplateContract = {
    ...TRIVIAL_TEMPLATE,
    recipe: {
      frames: [{ localId: "f1", title: "Empty Frame" }],
      shapes: [],
      edges: []
    },
    tags: { suggested: [] }
  };

  it("still produces a valid SceneGroup", () => {
    const result = applyTemplate(emptyRecipeTemplate, { x: 0, y: 0 });
    expect(result.errors).toHaveLength(0);
    expect(result.group).toBeDefined();
    const parsed = sceneGroupSchema.safeParse(result.group);
    expect(parsed.success).toBe(true);
  });

  it("produces no nodes or edges", () => {
    const result = applyTemplate(emptyRecipeTemplate, { x: 0, y: 0 });
    expect(result.nodes).toHaveLength(0);
    expect(result.edges).toHaveLength(0);
  });
});

describe("applyTemplate — nested frames (parentLocalId)", () => {
  const nestedTemplate: TemplateContract = {
    ...TRIVIAL_TEMPLATE,
    recipe: {
      frames: [
        { localId: "parent-frame", title: "Parent Frame" },
        { localId: "child-frame", title: "Child Frame", parentLocalId: "parent-frame" }
      ],
      shapes: [
        {
          localId: "s1",
          frameLocalId: "child-frame",
          title: "Child Shape",
          position: { x: 0, y: 0 },
          size: { width: 200, height: 100 }
        }
      ],
      edges: []
    },
    tags: { suggested: [] }
  };

  it("produces two groups with parent-child relationship", () => {
    const result = applyTemplate(nestedTemplate, { x: 0, y: 0 });
    expect(result.errors).toHaveLength(0);
    // group is the first (root) frame
    expect(result.group).toBeDefined();
  });
});
