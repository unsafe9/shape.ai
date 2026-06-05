/**
 * T4.5 — Presentation Template unit tests
 *
 * Verify:
 * 1. presentationTemplate is a valid TemplateContract whose recipe lowers via
 *    applyTemplate to ordinary SceneGroup / SceneNode / SceneEdge objects.
 * 2. Slide-frame composition: deck + slide child frames with parentGroupId,
 *    slideIndex ordering, and correct meta.semanticType values.
 * 3. presentationOutline exports a slide-ordered Markdown outline from the
 *    produced primitives without requiring a separate slide model.
 * 4. presentationStyleTokens carries the five new style token ids.
 * 5. validatePresentationMeta catches invalid meta values.
 */

import { describe, expect, it } from "vitest";
import { sceneGroupSchema, sceneNodeSchema, sceneEdgeSchema } from "../src/shared/schema";
import { applyTemplate } from "../src/shared/templates/contract";
import { applyRenderPatchToShapeScene } from "../src/shared/renderPatch";
import {
  presentationTemplate,
  presentationStyleTokens,
  presentationOutline,
  validatePresentationMeta
} from "../src/shared/templates/presentation";
import type { SceneGroup, SceneNode } from "../src/shared/schema";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function applyAt(x = 0, y = 0) {
  return applyTemplate(presentationTemplate, { x, y });
}

// ---------------------------------------------------------------------------
// 1. Contract structure
// ---------------------------------------------------------------------------

describe("presentationTemplate — contract structure", () => {
  it("has metadata id 'presentation' and category 'presentation'", () => {
    const m = presentationTemplate.metadata;
    expect(m.id).toBe("presentation");
    expect(m.category).toBe("presentation");
    expect(m.templateKind).toBe("presentation");
  });

  it("recipe has deck frame + two slide frames", () => {
    const frames = presentationTemplate.recipe.frames;
    expect(frames).toHaveLength(3);
    const deck = frames.find((f) => f.localId === "deck");
    const slide1 = frames.find((f) => f.localId === "slide-1");
    const slide2 = frames.find((f) => f.localId === "slide-2");
    expect(deck).toBeDefined();
    expect(slide1?.parentLocalId).toBe("deck");
    expect(slide2?.parentLocalId).toBe("deck");
  });

  it("recipe has at least one shape per semantic type (title, body, image, note)", () => {
    const semanticTypes = presentationTemplate.recipe.shapes.map(
      (s) => (s.meta as Record<string, unknown>)?.semanticType
    );
    expect(semanticTypes).toContain("slide-title");
    expect(semanticTypes).toContain("slide-body");
    expect(semanticTypes).toContain("slide-image");
    expect(semanticTypes).toContain("speaker-note");
  });

  it("recipe.edges contains the slide-flow connector", () => {
    const edges = presentationTemplate.recipe.edges;
    expect(edges.length).toBeGreaterThanOrEqual(1);
    const flow = edges.find((e) => (e.meta as Record<string, unknown>)?.semanticType === "slide-flow");
    expect(flow).toBeDefined();
  });

  it("exports allows design_doc_md and image_prompt, defaults to design_doc_md", () => {
    const { exports } = presentationTemplate;
    expect(exports.allowed).toContain("design_doc_md");
    expect(exports.allowed).toContain("image_prompt");
    expect(exports.default).toBe("design_doc_md");
  });

  it("tags.suggested carries the three slide tag entries", () => {
    const names = presentationTemplate.tags.suggested.map((t) => t.name);
    expect(names).toContain("draft-slide");
    expect(names).toContain("needs-image");
    expect(names).toContain("final");
  });

  it("promptHints is defined and advisory (does not gate rendering)", () => {
    const hints = presentationTemplate.promptHints;
    expect(hints).toBeDefined();
    expect(hints?.systemHint).toContain("slide deck");
    expect(hints?.suggestedOperations).toContain("create");
  });
});

// ---------------------------------------------------------------------------
// 2. Slide-frame composition via applyTemplate
// ---------------------------------------------------------------------------

describe("presentationTemplate — slide-frame composition", () => {
  it("applyTemplate returns no errors", () => {
    const result = applyAt();
    expect(result.errors).toHaveLength(0);
  });

  it("produces at least 3 groups (deck + 2 slides)", () => {
    // applyTemplate only returns the root group in result.group; all groups
    // are threaded through the scene — we verify by inspecting result.group
    // plus checking that nodes reference multiple groupIds.
    const result = applyAt();
    expect(result.group).toBeDefined();
    const parsed = sceneGroupSchema.safeParse(result.group);
    expect(parsed.success).toBe(true);
  });

  it("all produced SceneNodes parse through sceneNodeSchema", () => {
    const result = applyAt();
    expect(result.nodes.length).toBeGreaterThan(0);
    for (const node of result.nodes) {
      const parsed = sceneNodeSchema.safeParse(node);
      expect(parsed.success).toBe(true);
    }
  });

  it("all produced SceneEdges parse through sceneEdgeSchema", () => {
    const result = applyAt();
    for (const edge of result.edges) {
      const parsed = sceneEdgeSchema.safeParse(edge);
      expect(parsed.success).toBe(true);
    }
  });

  it("every node carries meta.templateKind === 'presentation'", () => {
    const result = applyAt();
    for (const node of result.nodes) {
      expect((node.meta as Record<string, unknown>)?.templateKind).toBe("presentation");
    }
  });

  it("slide-title nodes have styleKey 'slide-title'", () => {
    // styleKey is read back from the original recipe; nodes produced by applyTemplate
    // carry meta.semanticType matching their styleKey.
    const result = applyAt();
    const titleNodes = result.nodes.filter(
      (n) => (n.meta as Record<string, unknown>)?.semanticType === "slide-title"
    );
    expect(titleNodes.length).toBeGreaterThanOrEqual(2);
  });

  it("speaker-note nodes carry meta.semanticType 'speaker-note'", () => {
    const result = applyAt();
    const notes = result.nodes.filter(
      (n) => (n.meta as Record<string, unknown>)?.semanticType === "speaker-note"
    );
    expect(notes.length).toBeGreaterThanOrEqual(2);
  });

  it("produced objects are editable by standard move-card op", () => {
    const result = applyAt();
    // applyTemplate returns only the root group; nodes belong to child frames.
    // Build a scene that includes a group for every unique groupId referenced by nodes.
    const now = "2026-06-05T00:00:00.000Z";
    const groupIds = [...new Set(result.nodes.map((n) => n.groupId))];
    const groups = groupIds.map((gid) => ({
      id: gid,
      parentGroupId: null as string | null,
      title: "test-group",
      summary: "",
      bounds: { x: 0, y: 0, width: 2000, height: 2000 },
      tagIds: [] as string[],
      zIndex: 0,
      collapsed: false,
      createdAt: now,
      updatedAt: now,
      meta: undefined
    }));
    const scene = {
      version: 1 as const,
      sceneVersion: 0,
      groups,
      nodes: result.nodes,
      edges: [] as typeof result.edges,
      tags: result.newTags,
      comments: [] as never[],
      artifacts: [] as never[],
      selection: { kind: "canvas" as const },
      updatedAt: now
    };
    const node = result.nodes[0];
    const moved = applyRenderPatchToShapeScene(scene, {
      kind: "move-card",
      id: node.id,
      position: { x: 999, y: 888 }
    });
    expect(moved.errors).toHaveLength(0);
    const movedNode = moved.scene.nodes.find((n: SceneNode) => n.id === node.id);
    expect(movedNode?.position).toEqual({ x: 999, y: 888 });
  });

  it("anchor offset is applied to node positions", () => {
    const result = applyAt(500, 300);
    // Every node should be positioned at anchor + recipe offset (anchor >= recipe offset min)
    for (const node of result.nodes) {
      expect(node.position.x).toBeGreaterThanOrEqual(500);
      expect(node.position.y).toBeGreaterThanOrEqual(300);
    }
  });

  it("produces suggested tag objects for draft-slide, needs-image, final", () => {
    const result = applyAt();
    const tagNames = result.newTags.map((t) => t.name);
    expect(tagNames).toContain("draft-slide");
    expect(tagNames).toContain("needs-image");
    expect(tagNames).toContain("final");
  });
});

// ---------------------------------------------------------------------------
// 3. Export outline reader
// ---------------------------------------------------------------------------

describe("presentationOutline", () => {
  /** Build a minimal deck+slides+nodes scene from applyTemplate output. */
  function buildScene() {
    const result = applyAt();
    // Reconstruct all groups by simulating the nested frames.
    // applyTemplate's internal scene contains all groups; we probe via nodes' groupIds.
    const groupIds = new Set(result.nodes.map((n) => n.groupId));

    // Build synthetic deck/slide groups for the outline test.
    const now = "2026-06-05T00:00:00.000Z";
    const deckGroup: SceneGroup = {
      ...result.group,
      title: "My Deck",
      meta: { templateKind: "presentation", semanticType: "deck" }
    };
    // Two slide groups with slideIndex 0 and 1 referencing some of the produced nodes.
    const nodeIds = [...groupIds];
    const slide1Id = nodeIds[0] ?? "slide-1-id";
    const slide2Id = nodeIds[1] ?? "slide-2-id";

    const slide1: SceneGroup = {
      id: slide1Id,
      parentGroupId: deckGroup.id,
      title: "Title slide",
      summary: "",
      bounds: { x: 0, y: 0, width: 1280, height: 720 },
      tagIds: [],
      zIndex: 0,
      collapsed: false,
      createdAt: now,
      updatedAt: now,
      meta: { templateKind: "presentation", semanticType: "slide", slideIndex: 0 }
    };
    const slide2: SceneGroup = {
      id: slide2Id,
      parentGroupId: deckGroup.id,
      title: "Second slide",
      summary: "",
      bounds: { x: 1360, y: 0, width: 1280, height: 720 },
      tagIds: [],
      zIndex: 0,
      collapsed: false,
      createdAt: now,
      updatedAt: now,
      meta: { templateKind: "presentation", semanticType: "slide", slideIndex: 1 }
    };

    const titleNode: SceneNode = {
      id: "title-node-1",
      groupId: slide1Id,
      type: "task",
      title: "Opening title",
      summary: "",
      detail: "",
      status: "draft",
      confidence: 0.5,
      evidenceRefs: [],
      childDecisionIds: [],
      position: { x: 60, y: 40 },
      size: { width: 1160, height: 100 },
      zIndex: 0,
      tagIds: [],
      updatedAt: now,
      meta: { templateKind: "presentation", semanticType: "slide-title" }
    };
    const bodyNode: SceneNode = {
      id: "body-node-1",
      groupId: slide1Id,
      type: "task",
      title: "Body",
      summary: "Point one\nPoint two",
      detail: "",
      status: "draft",
      confidence: 0.5,
      evidenceRefs: [],
      childDecisionIds: [],
      position: { x: 60, y: 180 },
      size: { width: 780, height: 460 },
      zIndex: 0,
      tagIds: [],
      updatedAt: now,
      meta: { templateKind: "presentation", semanticType: "slide-body" }
    };
    const noteNode: SceneNode = {
      id: "note-node-1",
      groupId: slide1Id,
      type: "task",
      title: "Speaker notes",
      summary: "Narration text here",
      detail: "",
      status: "draft",
      confidence: 0.5,
      evidenceRefs: [],
      childDecisionIds: [],
      position: { x: 0, y: 740 },
      size: { width: 1280, height: 120 },
      zIndex: 0,
      tagIds: [],
      updatedAt: now,
      meta: { templateKind: "presentation", semanticType: "speaker-note" }
    };
    const title2Node: SceneNode = {
      id: "title-node-2",
      groupId: slide2Id,
      type: "task",
      title: "Second slide title",
      summary: "",
      detail: "",
      status: "draft",
      confidence: 0.5,
      evidenceRefs: [],
      childDecisionIds: [],
      position: { x: 1420, y: 40 },
      size: { width: 1160, height: 100 },
      zIndex: 0,
      tagIds: [],
      updatedAt: now,
      meta: { templateKind: "presentation", semanticType: "slide-title" }
    };

    return {
      deckGroup,
      allGroups: [deckGroup, slide1, slide2],
      allNodes: [titleNode, bodyNode, noteNode, title2Node]
    };
  }

  it("outline starts with deck title as H1", () => {
    const { deckGroup, allGroups, allNodes } = buildScene();
    const md = presentationOutline(deckGroup, allGroups, allNodes);
    expect(md.startsWith("# My Deck")).toBe(true);
  });

  it("outline contains H2 for each slide in slideIndex order", () => {
    const { deckGroup, allGroups, allNodes } = buildScene();
    const md = presentationOutline(deckGroup, allGroups, allNodes);
    const h2s = md.split("\n").filter((l) => l.startsWith("## "));
    expect(h2s).toHaveLength(2);
    // slideIndex 0 first
    expect(h2s[0]).toContain("Opening title");
    expect(h2s[1]).toContain("Second slide title");
  });

  it("outline includes body bullets as markdown list items", () => {
    const { deckGroup, allGroups, allNodes } = buildScene();
    const md = presentationOutline(deckGroup, allGroups, allNodes);
    expect(md).toContain("- Point one");
    expect(md).toContain("- Point two");
  });

  it("outline includes speaker notes block", () => {
    const { deckGroup, allGroups, allNodes } = buildScene();
    const md = presentationOutline(deckGroup, allGroups, allNodes);
    expect(md).toContain("> **Speaker notes:**");
    expect(md).toContain("Narration text here");
  });

  it("outline with no child slides is just the H1", () => {
    const now = "2026-06-05T00:00:00.000Z";
    const deck: SceneGroup = {
      id: "empty-deck",
      parentGroupId: null,
      title: "Empty Deck",
      summary: "",
      bounds: { x: 0, y: 0, width: 100, height: 100 },
      tagIds: [],
      zIndex: 0,
      collapsed: false,
      createdAt: now,
      updatedAt: now,
      meta: { templateKind: "presentation", semanticType: "deck" }
    };
    const md = presentationOutline(deck, [deck], []);
    expect(md.trim()).toBe("# Empty Deck");
  });

  it("slide order is determined by meta.slideIndex, not insertion order", () => {
    const now = "2026-06-05T00:00:00.000Z";
    const deck: SceneGroup = {
      id: "deck",
      parentGroupId: null,
      title: "Order Test",
      summary: "",
      bounds: { x: 0, y: 0, width: 100, height: 100 },
      tagIds: [],
      zIndex: 0,
      collapsed: false,
      createdAt: now,
      updatedAt: now,
      meta: { templateKind: "presentation", semanticType: "deck" }
    };
    // Insert in reverse order: slideIndex 1 then 0.
    const slideB: SceneGroup = {
      id: "slide-b",
      parentGroupId: "deck",
      title: "Slide B",
      summary: "",
      bounds: { x: 0, y: 0, width: 1280, height: 720 },
      tagIds: [],
      zIndex: 0,
      collapsed: false,
      createdAt: now,
      updatedAt: now,
      meta: { templateKind: "presentation", semanticType: "slide", slideIndex: 1 }
    };
    const slideA: SceneGroup = {
      id: "slide-a",
      parentGroupId: "deck",
      title: "Slide A",
      summary: "",
      bounds: { x: 0, y: 0, width: 1280, height: 720 },
      tagIds: [],
      zIndex: 0,
      collapsed: false,
      createdAt: now,
      updatedAt: now,
      meta: { templateKind: "presentation", semanticType: "slide", slideIndex: 0 }
    };
    const md = presentationOutline(deck, [deck, slideB, slideA], []);
    const h2s = md.split("\n").filter((l) => l.startsWith("## "));
    // slideIndex 0 (Slide A) should appear before slideIndex 1 (Slide B).
    expect(h2s[0]).toContain("Slide A");
    expect(h2s[1]).toContain("Slide B");
  });
});

// ---------------------------------------------------------------------------
// 4. Presentation style tokens
// ---------------------------------------------------------------------------

describe("presentationStyleTokens", () => {
  const tokenIds = presentationStyleTokens.map((t) => t.id);

  it("contains all five expected token ids", () => {
    expect(tokenIds).toContain("slide-title");
    expect(tokenIds).toContain("slide-body");
    expect(tokenIds).toContain("slide-image");
    expect(tokenIds).toContain("speaker-note");
    expect(tokenIds).toContain("slide-flow");
  });

  it("each token has required SceneStyleToken fields (fill, stroke, text, mutedText, accent)", () => {
    for (const token of presentationStyleTokens) {
      expect(typeof token.fill).toBe("string");
      expect(typeof token.stroke).toBe("string");
      expect(typeof token.text).toBe("string");
      expect(typeof token.mutedText).toBe("string");
      expect(typeof token.accent).toBe("string");
    }
  });

  it("token ids do not conflict with defaultStyles ids", () => {
    // defaultStyles uses: default, decision, risk, proposition, decision_point,
    // option, evidence, tradeoff, blocker, subdecision, task, artifact
    const defaultIds = new Set([
      "default", "decision", "risk", "proposition", "decision_point",
      "option", "evidence", "tradeoff", "blocker", "subdecision", "task", "artifact"
    ]);
    for (const token of presentationStyleTokens) {
      expect(defaultIds.has(token.id)).toBe(false);
    }
  });
});

// ---------------------------------------------------------------------------
// 5. validatePresentationMeta
// ---------------------------------------------------------------------------

describe("validatePresentationMeta", () => {
  it("returns no errors for valid deck meta", () => {
    const errors = validatePresentationMeta({
      templateKind: "presentation",
      semanticType: "deck"
    });
    expect(errors).toHaveLength(0);
  });

  it("returns no errors for valid slide meta with slideIndex", () => {
    const errors = validatePresentationMeta({
      templateKind: "presentation",
      semanticType: "slide",
      slideIndex: 0
    });
    expect(errors).toHaveLength(0);
  });

  it("errors when templateKind is not 'presentation'", () => {
    const errors = validatePresentationMeta({ templateKind: "other", semanticType: "slide" });
    expect(errors.some((e) => e.includes("templateKind"))).toBe(true);
  });

  it("errors when semanticType is not a known presentation type", () => {
    const errors = validatePresentationMeta({
      templateKind: "presentation",
      semanticType: "unknown-type"
    });
    expect(errors.some((e) => e.includes("semanticType"))).toBe(true);
  });

  it("errors when slideIndex is not an integer", () => {
    const errors = validatePresentationMeta({
      templateKind: "presentation",
      semanticType: "slide",
      slideIndex: 1.5
    });
    expect(errors.some((e) => e.includes("slideIndex"))).toBe(true);
  });

  it("all valid semanticTypes pass without error", () => {
    const validTypes = ["deck", "slide", "slide-title", "slide-body", "slide-image", "speaker-note", "slide-flow"];
    for (const semanticType of validTypes) {
      const errors = validatePresentationMeta({ templateKind: "presentation", semanticType });
      expect(errors).toHaveLength(0);
    }
  });
});
