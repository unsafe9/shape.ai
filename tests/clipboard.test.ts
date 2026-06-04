import { describe, expect, it } from "vitest";
import { cloneNodeForPaste, formatNodeMarkdown } from "../src/client/lib/nodeClipboard";
import type { GraphNode } from "../src/shared/schema";

const node: GraphNode = {
  id: "n1",
  type: "decision_point",
  title: "Choose rendering model",
  summary: "Pick the graph rendering approach.",
  detail: "The canvas must stay editable while keeping decisions readable.",
  status: "viable",
  confidence: 0.82,
  evidenceRefs: ["ADR-001"],
  childDecisionIds: []
};

describe("node clipboard helpers", () => {
  it("formats copied nodes as readable markdown", () => {
    const markdown = formatNodeMarkdown(node);

    expect(markdown).toContain("# Choose rendering model");
    expect(markdown).toContain("- **Type:** Decision");
    expect(markdown).toContain("- **Confidence:** 82%");
    expect(markdown).toContain("## Summary");
    expect(markdown).toContain("- ADR-001");
  });

  it("clones nodes with a new id and copy title", () => {
    const clone = cloneNodeForPaste(node, "n2");

    expect(clone.id).toBe("n2");
    expect(clone.title).toBe("Choose rendering model copy");
    expect(clone.summary).toBe(node.summary);
  });
});
