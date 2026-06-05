/**
 * T4.4 — ADR & Architecture Diagram Templates tests
 *
 * 1. Recipe shape: each TemplateContract satisfies the T4.1 contract fields.
 * 2. applyTemplate: each template produces valid scene objects.
 * 3. ADR export preset (adrExportPreset): reads primitive content including
 *    the meta.semanticType / meta.status shim.
 */

import { describe, expect, it } from "vitest";
import { sceneGroupSchema, sceneNodeSchema, sceneEdgeSchema } from "../src/shared/schema";
import { applyTemplate } from "../src/shared/templates/contract";
import {
  ADR_TEMPLATE,
  DECISION_MAP_TEMPLATE,
  SERVER_ARCHITECTURE_TEMPLATE,
  DEPENDENCY_DIAGRAM_TEMPLATE,
  INVESTIGATION_MAP_TEMPLATE,
  ADR_ARCHITECTURE_TEMPLATES
} from "../src/shared/templates/adrArchitecture";
import { adrExportPreset, semanticType, semanticStatus } from "../src/server/local";
import type { DecisionGraph, GraphNode } from "../src/shared/schema";

const ANCHOR = { x: 0, y: 0 };

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function allTemplates() {
  return [
    ADR_TEMPLATE,
    DECISION_MAP_TEMPLATE,
    SERVER_ARCHITECTURE_TEMPLATE,
    DEPENDENCY_DIAGRAM_TEMPLATE,
    INVESTIGATION_MAP_TEMPLATE
  ];
}

// ---------------------------------------------------------------------------
// §1 Recipe shape — structural checks
// ---------------------------------------------------------------------------

describe("ADR_ARCHITECTURE_TEMPLATES registry", () => {
  it("contains all five templates by id", () => {
    expect(Object.keys(ADR_ARCHITECTURE_TEMPLATES).sort()).toEqual(
      ["adr", "decision-map", "dependency-diagram", "investigation-map", "server-architecture"].sort()
    );
  });

  it("each registry entry matches the named export", () => {
    expect(ADR_ARCHITECTURE_TEMPLATES["adr"]).toBe(ADR_TEMPLATE);
    expect(ADR_ARCHITECTURE_TEMPLATES["decision-map"]).toBe(DECISION_MAP_TEMPLATE);
    expect(ADR_ARCHITECTURE_TEMPLATES["server-architecture"]).toBe(SERVER_ARCHITECTURE_TEMPLATE);
    expect(ADR_ARCHITECTURE_TEMPLATES["dependency-diagram"]).toBe(DEPENDENCY_DIAGRAM_TEMPLATE);
    expect(ADR_ARCHITECTURE_TEMPLATES["investigation-map"]).toBe(INVESTIGATION_MAP_TEMPLATE);
  });
});

describe("template metadata", () => {
  for (const tpl of allTemplates()) {
    it(`${tpl.metadata.id}: has required metadata fields`, () => {
      expect(tpl.metadata.id).toBeTruthy();
      expect(tpl.metadata.title).toBeTruthy();
      expect(tpl.metadata.category).toBe("engineering");
      expect(tpl.metadata.templateKind).toBe(tpl.metadata.id);
    });
  }
});

describe("template recipe structure", () => {
  it("ADR recipe has 10 shapes and 9 edges (mirrors seedGroupScene)", () => {
    expect(ADR_TEMPLATE.recipe.shapes).toHaveLength(10);
    expect(ADR_TEMPLATE.recipe.edges).toHaveLength(9);
    expect(ADR_TEMPLATE.recipe.frames).toHaveLength(1);
  });

  it("ADR shape localIds match the seedGroupScene node vocab", () => {
    const ids = ADR_TEMPLATE.recipe.shapes.map((s) => s.localId);
    expect(ids).toContain("proposition");
    expect(ids).toContain("decision-points");
    expect(ids).toContain("option-graph");
    expect(ids).toContain("option-freeform");
    expect(ids).toContain("evidence");
    expect(ids).toContain("tradeoff");
    expect(ids).toContain("blocker");
    expect(ids).toContain("subdecision");
    expect(ids).toContain("task");
    expect(ids).toContain("artifact");
  });

  it("ADR shapes carry meta.semanticType", () => {
    for (const shape of ADR_TEMPLATE.recipe.shapes) {
      expect(typeof shape.meta?.semanticType).toBe("string");
    }
  });

  it("ADR edges carry meta.semanticType", () => {
    for (const edge of ADR_TEMPLATE.recipe.edges) {
      expect(typeof edge.meta?.semanticType).toBe("string");
    }
  });

  it("decision-map recipe has lighter shape set (no evidence/tradeoff/blocker)", () => {
    const ids = DECISION_MAP_TEMPLATE.recipe.shapes.map((s) => s.meta?.semanticType);
    expect(ids).not.toContain("evidence");
    expect(ids).not.toContain("tradeoff");
    expect(ids).not.toContain("blocker");
    expect(ids).toContain("decision_point");
    expect(ids).toContain("option");
  });

  it("server-architecture recipe has nested zone frames", () => {
    const frames = SERVER_ARCHITECTURE_TEMPLATE.recipe.frames;
    // root + 3 zone frames
    expect(frames.length).toBeGreaterThanOrEqual(4);
    const zoneFrames = frames.filter((f) => f.meta?.semanticType === "zone");
    expect(zoneFrames.length).toBeGreaterThanOrEqual(3);
  });

  it("dependency-diagram recipe has blocks edge", () => {
    const blockEdge = DEPENDENCY_DIAGRAM_TEMPLATE.recipe.edges.find(
      (e) => e.meta?.semanticType === "blocks"
    );
    expect(blockEdge).toBeDefined();
  });

  it("investigation-map recipe has task next-step shape", () => {
    const taskShape = INVESTIGATION_MAP_TEMPLATE.recipe.shapes.find(
      (s) => s.meta?.semanticType === "task"
    );
    expect(taskShape).toBeDefined();
  });
});

describe("template exports declarations", () => {
  it("ADR template allows madr and is the default", () => {
    expect(ADR_TEMPLATE.exports.allowed).toContain("madr");
    expect(ADR_TEMPLATE.exports.default).toBe("madr");
  });

  it("ADR template allows all required export types", () => {
    const allowed = ADR_TEMPLATE.exports.allowed;
    expect(allowed).toContain("yadr");
    expect(allowed).toContain("design_doc_md");
    expect(allowed).toContain("confluence_html");
    expect(allowed).toContain("mermaid");
  });

  it("decision-map default export is mermaid", () => {
    expect(DECISION_MAP_TEMPLATE.exports.default).toBe("mermaid");
  });

  it("server-architecture allows architecture_image and image_prompt", () => {
    expect(SERVER_ARCHITECTURE_TEMPLATE.exports.allowed).toContain("architecture_image");
    expect(SERVER_ARCHITECTURE_TEMPLATE.exports.allowed).toContain("image_prompt");
  });

  it("investigation-map allows ai_plan_md", () => {
    expect(INVESTIGATION_MAP_TEMPLATE.exports.allowed).toContain("ai_plan_md");
  });

  it("investigation-map default export is design_doc_md", () => {
    expect(INVESTIGATION_MAP_TEMPLATE.exports.default).toBe("design_doc_md");
  });
});

describe("template suggested tags", () => {
  it("ADR template suggests viable/conditional/infeasible/selected tags", () => {
    const names = ADR_TEMPLATE.tags.suggested.map((t) => t.name);
    expect(names).toContain("viable");
    expect(names).toContain("conditional");
    expect(names).toContain("infeasible");
    expect(names).toContain("selected");
  });

  it("investigation-map suggests confirmed/refuted/open tags", () => {
    const names = INVESTIGATION_MAP_TEMPLATE.tags.suggested.map((t) => t.name);
    expect(names).toContain("confirmed");
    expect(names).toContain("refuted");
    expect(names).toContain("open");
  });
});

describe("template promptHints", () => {
  for (const tpl of allTemplates()) {
    it(`${tpl.metadata.id}: has a systemHint`, () => {
      expect(tpl.promptHints?.systemHint).toBeTruthy();
    });
  }

  it("ADR template has fieldHints for core semantic types", () => {
    const hints = ADR_TEMPLATE.promptHints?.fieldHints ?? {};
    expect(hints["proposition"]).toBeTruthy();
    expect(hints["option"]).toBeTruthy();
    expect(hints["evidence"]).toBeTruthy();
  });
});

// ---------------------------------------------------------------------------
// §2 applyTemplate — produces valid scene objects
// ---------------------------------------------------------------------------

describe("applyTemplate — ADR template", () => {
  const result = applyTemplate(ADR_TEMPLATE, ANCHOR);

  it("produces no errors", () => {
    expect(result.errors).toHaveLength(0);
  });

  it("produces a valid SceneGroup", () => {
    expect(sceneGroupSchema.safeParse(result.group).success).toBe(true);
  });

  it("produces 10 SceneNodes", () => {
    expect(result.nodes).toHaveLength(10);
  });

  it("produces 9 SceneEdges", () => {
    expect(result.edges).toHaveLength(9);
  });

  it("all nodes parse through sceneNodeSchema", () => {
    for (const node of result.nodes) {
      expect(sceneNodeSchema.safeParse(node).success).toBe(true);
    }
  });

  it("all edges parse through sceneEdgeSchema", () => {
    for (const edge of result.edges) {
      expect(sceneEdgeSchema.safeParse(edge).success).toBe(true);
    }
  });

  it("nodes carry meta.templateKind = 'adr'", () => {
    for (const node of result.nodes) {
      expect((node.meta as Record<string, unknown>)?.templateKind).toBe("adr");
    }
  });

  it("produces suggested tags (viable/conditional/infeasible/selected)", () => {
    const names = result.newTags.map((t) => t.name);
    expect(names).toContain("viable");
    expect(names).toContain("selected");
  });
});

describe("applyTemplate — decision-map template", () => {
  const result = applyTemplate(DECISION_MAP_TEMPLATE, ANCHOR);

  it("produces no errors", () => {
    expect(result.errors).toHaveLength(0);
  });

  it("produces a valid group and scene objects", () => {
    expect(sceneGroupSchema.safeParse(result.group).success).toBe(true);
    for (const n of result.nodes) expect(sceneNodeSchema.safeParse(n).success).toBe(true);
    for (const e of result.edges) expect(sceneEdgeSchema.safeParse(e).success).toBe(true);
  });
});

describe("applyTemplate — server-architecture template", () => {
  const result = applyTemplate(SERVER_ARCHITECTURE_TEMPLATE, ANCHOR);

  it("produces no errors", () => {
    expect(result.errors).toHaveLength(0);
  });

  it("produces component shapes", () => {
    expect(result.nodes.length).toBeGreaterThanOrEqual(5);
  });
});

describe("applyTemplate — dependency-diagram template", () => {
  const result = applyTemplate(DEPENDENCY_DIAGRAM_TEMPLATE, ANCHOR);

  it("produces no errors", () => {
    expect(result.errors).toHaveLength(0);
  });

  it("produces unit shapes and dependency edges", () => {
    expect(result.nodes.length).toBeGreaterThanOrEqual(4);
    expect(result.edges.length).toBeGreaterThanOrEqual(3);
  });
});

describe("applyTemplate — investigation-map template", () => {
  const result = applyTemplate(INVESTIGATION_MAP_TEMPLATE, ANCHOR);

  it("produces no errors", () => {
    expect(result.errors).toHaveLength(0);
  });

  it("produces question, evidence, hypothesis, finding, next-step shapes", () => {
    expect(result.nodes.length).toBeGreaterThanOrEqual(7);
  });

  it("nodes carry meta.templateKind = 'investigation-map'", () => {
    for (const node of result.nodes) {
      expect((node.meta as Record<string, unknown>)?.templateKind).toBe("investigation-map");
    }
  });
});

// ---------------------------------------------------------------------------
// §3 ADR export preset — reads primitive content (T4.4 §7)
// ---------------------------------------------------------------------------

describe("semanticType / semanticStatus shim", () => {
  it("returns meta.semanticType when present", () => {
    const node = { type: "artifact", status: "draft", meta: { semanticType: "option" } } as unknown as GraphNode;
    expect(semanticType(node)).toBe("option");
  });

  it("falls back to node.type when meta.semanticType is absent", () => {
    const node = { type: "evidence", status: "draft" } as GraphNode;
    expect(semanticType(node)).toBe("evidence");
  });

  it("returns meta.status when present", () => {
    const node = { type: "artifact", status: "draft", meta: { status: "selected" } } as unknown as GraphNode;
    expect(semanticStatus(node)).toBe("selected");
  });

  it("falls back to node.status when meta.status is absent", () => {
    const node = { type: "artifact", status: "viable" } as GraphNode;
    expect(semanticStatus(node)).toBe("viable");
  });

  it("ignores empty string meta.semanticType and falls back", () => {
    const node = { type: "task", status: "draft", meta: { semanticType: "" } } as unknown as GraphNode;
    expect(semanticType(node)).toBe("task");
  });
});

describe("adrExportPreset — reads primitive content", () => {
  // Build a minimal DecisionGraph that uses meta.semanticType / meta.status
  // instead of top-level type/status fields (post-demotion shape).
  const metaGraph: DecisionGraph = {
    version: 1,
    nodes: [
      {
        id: "n1",
        type: "artifact",       // legacy type — ignored because meta.semanticType is set
        title: "Design question",
        summary: "What format to use?",
        detail: "",
        status: "draft",         // legacy status — ignored because meta.status is set
        confidence: 0.7,
        evidenceRefs: [],
        childDecisionIds: [],
        // meta carries post-demotion semantics
        meta: { semanticType: "proposition", status: "selected" }
      } as GraphNode,
      {
        id: "n2",
        type: "artifact",
        title: "MADR",
        summary: "Use MADR markdown format.",
        detail: "",
        status: "draft",
        confidence: 0.8,
        evidenceRefs: [],
        childDecisionIds: [],
        meta: { semanticType: "option", status: "viable" }
      } as GraphNode,
      {
        id: "n3",
        type: "artifact",
        title: "Setup task",
        summary: "Write the first ADR.",
        detail: "",
        status: "draft",
        confidence: 0.6,
        evidenceRefs: [],
        childDecisionIds: [],
        meta: { semanticType: "task", status: "draft" }
      } as GraphNode
    ],
    edges: [
      {
        id: "e1",
        type: "depends_on",
        source: "n1",
        target: "n2",
        label: "considers",
        rationale: "",
        confidence: 0.7
      }
    ]
  };

  it("produces MADR output that reads option via meta.semanticType", () => {
    const output = adrExportPreset(metaGraph, { type: "madr" }, "Test Decision");
    expect(output.title).toContain("MADR");
    // Option title should appear in output because semanticType("artifact" node) → "option"
    expect(output.content).toContain("MADR");
    expect(output.content).toContain("## Considered Options");
  });

  it("produces ai_plan_md that reads task via meta.semanticType", () => {
    const output = adrExportPreset(metaGraph, { type: "ai_plan_md" }, "Test Decision");
    expect(output.title).toContain("AI Task Plan");
    // task node should appear
    expect(output.content).toContain("Setup task");
  });

  it("produces mermaid output from primitive graph", () => {
    const output = adrExportPreset(metaGraph, { type: "mermaid" }, "Test Decision");
    expect(output.content).toContain("flowchart LR");
    expect(output.content).toContain("considers");
  });

  it("legacy graph (no meta) still works via fallback", () => {
    const legacyGraph: DecisionGraph = {
      version: 1,
      nodes: [
        {
          id: "n1",
          type: "proposition",
          title: "Legacy node",
          summary: "Summary.",
          detail: "",
          status: "selected",
          confidence: 0.7,
          evidenceRefs: [],
          childDecisionIds: []
        },
        {
          id: "n2",
          type: "option",
          title: "Legacy option",
          summary: "Option summary.",
          detail: "",
          status: "viable",
          confidence: 0.8,
          evidenceRefs: [],
          childDecisionIds: []
        }
      ],
      edges: []
    };
    const output = adrExportPreset(legacyGraph, { type: "madr" }, "Legacy Decision");
    expect(output.content).toContain("## Considered Options");
    // Should not throw; legacy data still produces correct output
    expect(output.content).toContain("Legacy option");
  });
});
