/**
 * T4.3 — Wiki Note Cluster & Idea Board Templates
 *
 * Tests verify:
 * 1. Template composition: both contracts produce valid SceneGroup/SceneNode/SceneEdge
 *    objects (parse through Zod schemas) with the correct structure.
 * 2. Source/evidence cards carry the right styleKey and meta.
 * 3. Reference edges carry labels and meta.semanticType.
 * 4. Spatial reorganization (move-card) preserves text, tags, and meta (source refs).
 * 5. Tags are produced correctly for both templates.
 */

import { describe, expect, it } from "vitest";
import { sceneGroupSchema, sceneNodeSchema, sceneEdgeSchema } from "../src/shared/schema";
import { applyRenderPatchToShapeScene } from "../src/shared/renderPatch";
import { applyTemplate, type AppliedTemplate } from "../src/shared/templates/contract";
import { wikiNoteTemplate, ideaBoardTemplate } from "../src/shared/templates/wikiNote";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const ANCHOR = { x: 0, y: 0 };
const NOW = "2026-06-05T00:00:00.000Z";

/**
 * Build a minimal Scene from an AppliedTemplate result.
 * Includes all groups referenced by nodes (not just the root group), so
 * commitAppPatch's group-membership filter keeps all nodes and edges.
 */
function sceneFrom(result: AppliedTemplate) {
  // Collect all unique groupIds referenced by nodes and edges.
  const neededGroupIds = new Set<string>();
  for (const node of result.nodes) neededGroupIds.add(node.groupId);
  for (const edge of result.edges) neededGroupIds.add(edge.groupId);

  // Build synthetic group objects for any groupId not covered by result.group.
  // result.group is the root frame; child frames need stub entries.
  const groups = result.group ? [result.group] : [];
  const existingIds = new Set(groups.map((g) => g.id));
  for (const gid of neededGroupIds) {
    if (!existingIds.has(gid)) {
      groups.push({
        id: gid,
        parentGroupId: result.group?.id ?? null,
        title: "Child Frame",
        summary: "",
        bounds: { x: 0, y: 0, width: 900, height: 900 },
        tagIds: [],
        zIndex: 0,
        collapsed: false,
        createdAt: NOW,
        updatedAt: NOW
      });
    }
  }

  return {
    version: 1 as const,
    sceneVersion: 0,
    groups,
    nodes: result.nodes,
    edges: result.edges,
    tags: result.newTags,
    comments: [],
    artifacts: [],
    selection: { kind: "canvas" as const },
    updatedAt: NOW
  };
}

// ---------------------------------------------------------------------------
// Wiki Note Cluster
// ---------------------------------------------------------------------------

describe("wikiNoteTemplate — metadata", () => {
  it("has correct id and templateKind", () => {
    expect(wikiNoteTemplate.metadata.id).toBe("wiki-note");
    expect(wikiNoteTemplate.metadata.templateKind).toBe("wiki-note");
    expect(wikiNoteTemplate.metadata.category).toBe("knowledge");
  });

  it("allowed exports include design_doc_md, confluence_html, madr, mermaid", () => {
    const { allowed } = wikiNoteTemplate.exports;
    expect(allowed).toContain("design_doc_md");
    expect(allowed).toContain("confluence_html");
    expect(allowed).toContain("madr");
    expect(allowed).toContain("mermaid");
    expect(wikiNoteTemplate.exports.default).toBe("design_doc_md");
  });

  it("has three suggested tags: topic, source, draft", () => {
    const names = wikiNoteTemplate.tags.suggested.map((t) => t.name);
    expect(names).toContain("topic");
    expect(names).toContain("source");
    expect(names).toContain("draft");
  });

  it("promptHints are advisory and present", () => {
    expect(wikiNoteTemplate.promptHints?.systemHint).toContain("wiki note cluster");
    expect(wikiNoteTemplate.promptHints?.suggestedOperations).toContain("comment");
  });
});

describe("wikiNoteTemplate — recipe structure", () => {
  it("recipe has two frames: f-cluster (root) and f-section-a (child)", () => {
    const { frames } = wikiNoteTemplate.recipe;
    expect(frames).toHaveLength(2);
    const root = frames.find((f) => f.localId === "f-cluster");
    const section = frames.find((f) => f.localId === "f-section-a");
    expect(root).toBeDefined();
    expect(section?.parentLocalId).toBe("f-cluster");
  });

  it("recipe has four shapes with correct localIds", () => {
    const ids = wikiNoteTemplate.recipe.shapes.map((s) => s.localId);
    expect(ids).toContain("n-overview");
    expect(ids).toContain("n-note-1");
    expect(ids).toContain("n-note-2");
    expect(ids).toContain("n-source-1");
  });

  it("source card has styleKey 'evidence'", () => {
    const src = wikiNoteTemplate.recipe.shapes.find((s) => s.localId === "n-source-1")!;
    expect(src.styleKey).toBe("evidence");
  });

  it("source card meta has semanticType 'source'", () => {
    const src = wikiNoteTemplate.recipe.shapes.find((s) => s.localId === "n-source-1")!;
    expect((src.meta as Record<string, unknown>)?.semanticType).toBe("source");
  });

  it("note cards have styleKey 'proposition'", () => {
    for (const localId of ["n-overview", "n-note-1", "n-note-2"]) {
      const s = wikiNoteTemplate.recipe.shapes.find((x) => x.localId === localId)!;
      expect(s.styleKey).toBe("proposition");
    }
  });

  it("recipe has two edges: e-ref-1 and e-cite-1", () => {
    const ids = wikiNoteTemplate.recipe.edges.map((e) => e.localId);
    expect(ids).toContain("e-ref-1");
    expect(ids).toContain("e-cite-1");
  });

  it("reference edge has semanticType 'reference'", () => {
    const e = wikiNoteTemplate.recipe.edges.find((x) => x.localId === "e-ref-1")!;
    expect((e.meta as Record<string, unknown>)?.semanticType).toBe("reference");
    expect(e.label).toBe("see also");
  });

  it("citation edge has semanticType 'citation'", () => {
    const e = wikiNoteTemplate.recipe.edges.find((x) => x.localId === "e-cite-1")!;
    expect((e.meta as Record<string, unknown>)?.semanticType).toBe("citation");
    expect(e.label).toBe("cited by");
  });
});

describe("wikiNoteTemplate — applyTemplate produces valid primitives", () => {
  const result = applyTemplate(wikiNoteTemplate, ANCHOR, "wn");

  it("no errors on apply", () => {
    expect(result.errors).toHaveLength(0);
  });

  it("root group parses through sceneGroupSchema", () => {
    const parsed = sceneGroupSchema.safeParse(result.group);
    expect(parsed.success).toBe(true);
  });

  it("produces 4 nodes", () => {
    expect(result.nodes).toHaveLength(4);
  });

  it("all nodes parse through sceneNodeSchema", () => {
    for (const node of result.nodes) {
      const parsed = sceneNodeSchema.safeParse(node);
      expect(parsed.success).toBe(true);
    }
  });

  it("produces 2 edges", () => {
    expect(result.edges).toHaveLength(2);
  });

  it("all edges parse through sceneEdgeSchema", () => {
    for (const edge of result.edges) {
      const parsed = sceneEdgeSchema.safeParse(edge);
      expect(parsed.success).toBe(true);
    }
  });

  it("all nodes carry meta.templateKind = 'wiki-note'", () => {
    for (const node of result.nodes) {
      expect((node.meta as Record<string, unknown>)?.templateKind).toBe("wiki-note");
    }
  });

  it("produces 3 suggested tags", () => {
    expect(result.newTags).toHaveLength(3);
    const names = result.newTags.map((t) => t.name);
    expect(names).toContain("topic");
    expect(names).toContain("source");
    expect(names).toContain("draft");
  });
});

// ---------------------------------------------------------------------------
// Verify §7 — spatial reorganization preserves content
// ---------------------------------------------------------------------------

describe("wikiNoteTemplate — spatial reorg preserves content (§7 Verify)", () => {
  it("move-card preserves title, summary, detail, and meta", () => {
    const result = applyTemplate(wikiNoteTemplate, ANCHOR, "wn-move");
    const node = result.nodes.find((n) => n.title === "Note 1")!;
    expect(node).toBeDefined();

    // Capture content before move.
    const beforeTitle = node.title;
    const beforeSummary = node.summary;
    const beforeDetail = node.detail;
    const beforeMeta = node.meta;

    const scene = sceneFrom(result);
    const moved = applyRenderPatchToShapeScene(scene, {
      kind: "move-card",
      id: node.id,
      position: { x: 999, y: 888 }
    });
    expect(moved.errors).toHaveLength(0);

    const movedNode = moved.scene.nodes.find((n) => n.id === node.id)!;
    expect(movedNode).toBeDefined();
    // Position changed.
    expect(movedNode.position).toEqual({ x: 999, y: 888 });
    // Content fields are untouched.
    expect(movedNode.title).toBe(beforeTitle);
    expect(movedNode.summary).toBe(beforeSummary);
    expect(movedNode.detail).toBe(beforeDetail);
    expect(movedNode.meta).toEqual(beforeMeta);
  });

  it("move-card preserves meta (which carries semantic type) on overview node", () => {
    const result = applyTemplate(wikiNoteTemplate, ANCHOR, "wn-tags");
    const overviewNode = result.nodes.find((n) => n.title === "Overview")!;
    expect(overviewNode).toBeDefined();
    const beforeMeta = overviewNode.meta;

    const scene = sceneFrom(result);
    const moved = applyRenderPatchToShapeScene(scene, {
      kind: "move-card",
      id: overviewNode.id,
      position: { x: 100, y: 100 }
    });
    expect(moved.errors).toHaveLength(0);
    const movedNode = moved.scene.nodes.find((n) => n.id === overviewNode.id)!;
    // meta (including semanticType and templateKind) survives the move.
    expect(movedNode.meta).toEqual(beforeMeta);
  });

  it("move-card preserves source card evidenceRefsHint in meta", () => {
    const result = applyTemplate(wikiNoteTemplate, ANCHOR, "wn-src");
    const sourceNode = result.nodes.find(
      (n) => (n.meta as Record<string, unknown>)?.semanticType === "source"
    )!;
    expect(sourceNode).toBeDefined();

    const scene = sceneFrom(result);
    const moved = applyRenderPatchToShapeScene(scene, {
      kind: "move-card",
      id: sourceNode.id,
      position: { x: 1000, y: 2000 }
    });
    expect(moved.errors).toHaveLength(0);
    const movedSrc = moved.scene.nodes.find((n) => n.id === sourceNode.id)!;
    expect(movedSrc).toBeDefined();
    // Source meta is preserved.
    const meta = movedSrc.meta as Record<string, unknown>;
    expect(meta?.semanticType).toBe("source");
    expect(meta?.citationKind).toBe("url");
    expect(meta?.evidenceRefsHint).toEqual(["https://example.com"]);
  });

  it("edges survive node moves — source/target ids and label are stable", () => {
    const result = applyTemplate(wikiNoteTemplate, ANCHOR, "wn-edge");
    const edgeBefore = result.edges[0];
    expect(edgeBefore).toBeDefined();

    const scene = sceneFrom(result);

    // Move both endpoints.
    let s = applyRenderPatchToShapeScene(scene, {
      kind: "move-card",
      id: edgeBefore.source,
      position: { x: 500, y: 500 }
    }).scene;
    s = applyRenderPatchToShapeScene(s, {
      kind: "move-card",
      id: edgeBefore.target,
      position: { x: 800, y: 800 }
    }).scene;

    const edgeAfter = s.edges.find((e) => e.id === edgeBefore.id);
    expect(edgeAfter).toBeDefined();
    // Edge identity and endpoints are unchanged.
    expect(edgeAfter!.source).toBe(edgeBefore.source);
    expect(edgeAfter!.target).toBe(edgeBefore.target);
    expect(edgeAfter!.label).toBe(edgeBefore.label);
  });
});

// ---------------------------------------------------------------------------
// Idea Board
// ---------------------------------------------------------------------------

describe("ideaBoardTemplate — metadata", () => {
  it("has correct id and templateKind", () => {
    expect(ideaBoardTemplate.metadata.id).toBe("idea-board");
    expect(ideaBoardTemplate.metadata.templateKind).toBe("idea-board");
    expect(ideaBoardTemplate.metadata.category).toBe("knowledge");
  });

  it("allowed exports include design_doc_md, mermaid, image_prompt", () => {
    const { allowed } = ideaBoardTemplate.exports;
    expect(allowed).toContain("design_doc_md");
    expect(allowed).toContain("mermaid");
    expect(allowed).toContain("image_prompt");
    expect(ideaBoardTemplate.exports.default).toBe("design_doc_md");
  });

  it("has four suggested tags: theme, spark, parked, source", () => {
    const names = ideaBoardTemplate.tags.suggested.map((t) => t.name);
    expect(names).toContain("theme");
    expect(names).toContain("spark");
    expect(names).toContain("parked");
    expect(names).toContain("source");
  });
});

describe("ideaBoardTemplate — recipe structure", () => {
  it("recipe has two frames: f-board (root) and f-theme-1 (child)", () => {
    const { frames } = ideaBoardTemplate.recipe;
    expect(frames).toHaveLength(2);
    const root = frames.find((f) => f.localId === "f-board");
    const theme = frames.find((f) => f.localId === "f-theme-1");
    expect(root).toBeDefined();
    expect(theme?.parentLocalId).toBe("f-board");
  });

  it("idea cards have styleKey 'option'", () => {
    const ideas = ideaBoardTemplate.recipe.shapes.filter(
      (s) => (s.meta as Record<string, unknown>)?.semanticType === "idea"
    );
    expect(ideas.length).toBeGreaterThanOrEqual(2);
    for (const idea of ideas) {
      expect(idea.styleKey).toBe("option");
    }
  });

  it("has a 'relates to' edge", () => {
    const e = ideaBoardTemplate.recipe.edges.find((x) => x.localId === "e-rel-1")!;
    expect(e.label).toBe("relates to");
    expect((e.meta as Record<string, unknown>)?.semanticType).toBe("relates-to");
  });
});

describe("ideaBoardTemplate — applyTemplate produces valid primitives", () => {
  const result = applyTemplate(ideaBoardTemplate, ANCHOR, "ib");

  it("no errors on apply", () => {
    expect(result.errors).toHaveLength(0);
  });

  it("root group parses through sceneGroupSchema", () => {
    expect(sceneGroupSchema.safeParse(result.group).success).toBe(true);
  });

  it("produces 4 nodes", () => {
    expect(result.nodes).toHaveLength(4);
  });

  it("all nodes parse through sceneNodeSchema", () => {
    for (const node of result.nodes) {
      expect(sceneNodeSchema.safeParse(node).success).toBe(true);
    }
  });

  it("produces 1 edge", () => {
    expect(result.edges).toHaveLength(1);
  });

  it("all edges parse through sceneEdgeSchema", () => {
    for (const edge of result.edges) {
      expect(sceneEdgeSchema.safeParse(edge).success).toBe(true);
    }
  });

  it("all nodes carry meta.templateKind = 'idea-board'", () => {
    for (const node of result.nodes) {
      expect((node.meta as Record<string, unknown>)?.templateKind).toBe("idea-board");
    }
  });

  it("produces 4 suggested tags", () => {
    expect(result.newTags).toHaveLength(4);
  });
});

describe("ideaBoardTemplate — spatial reorg preserves content (§7 Verify)", () => {
  it("move-card preserves idea title, summary, and meta after spatial move", () => {
    const result = applyTemplate(ideaBoardTemplate, ANCHOR, "ib-move");
    const idea = result.nodes.find((n) => n.title === "Idea 1")!;
    expect(idea).toBeDefined();

    const scene = sceneFrom(result);
    const moved = applyRenderPatchToShapeScene(scene, {
      kind: "move-card",
      id: idea.id,
      position: { x: 1200, y: 600 }
    });
    expect(moved.errors).toHaveLength(0);
    const movedIdea = moved.scene.nodes.find((n) => n.id === idea.id)!;
    expect(movedIdea).toBeDefined();
    expect(movedIdea.title).toBe("Idea 1");
    expect(movedIdea.summary).toBe("Describe the idea.");
    expect((movedIdea.meta as Record<string, unknown>)?.semanticType).toBe("idea");
    expect((movedIdea.meta as Record<string, unknown>)?.templateKind).toBe("idea-board");
  });

  it("relates-to edge survives both endpoint moves", () => {
    const result = applyTemplate(ideaBoardTemplate, ANCHOR, "ib-edge");
    const edgeBefore = result.edges[0];
    expect(edgeBefore).toBeDefined();
    expect(edgeBefore.label).toBe("relates to");

    const scene = sceneFrom(result);
    let s = applyRenderPatchToShapeScene(scene, {
      kind: "move-card",
      id: edgeBefore.source,
      position: { x: 300, y: 300 }
    }).scene;
    s = applyRenderPatchToShapeScene(s, {
      kind: "move-card",
      id: edgeBefore.target,
      position: { x: 700, y: 700 }
    }).scene;

    const edgeAfter = s.edges.find((e) => e.id === edgeBefore.id);
    expect(edgeAfter).toBeDefined();
    expect(edgeAfter!.source).toBe(edgeBefore.source);
    expect(edgeAfter!.target).toBe(edgeBefore.target);
    expect(edgeAfter!.label).toBe("relates to");
  });
});
