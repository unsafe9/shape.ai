import { describe, expect, it } from "vitest";
import { applyGraphPatch, makeMermaid, selectedSubgraph } from "../src/shared/graph";
import type { DecisionGraph } from "../src/shared/schema";

const graph: DecisionGraph = {
  version: 1,
  nodes: [
    {
      id: "a",
      type: "proposition",
      title: "A",
      summary: "root",
      detail: "root",
      status: "draft",
      confidence: 0.5,
      evidenceRefs: [],
      childDecisionIds: []
    },
    {
      id: "b",
      type: "option",
      title: "B",
      summary: "option",
      detail: "option",
      status: "viable",
      confidence: 0.7,
      evidenceRefs: [],
      childDecisionIds: []
    }
  ],
  edges: [
    {
      id: "e",
      type: "supports",
      source: "a",
      target: "b",
      label: "supports",
      rationale: "because",
      confidence: 0.7
    }
  ]
};

describe("graph helpers", () => {
  it("applies graph patches and drops dangling edges", () => {
    const next = applyGraphPatch(graph, {
      addNodes: [],
      updateNodes: [],
      removeNodeIds: ["b"],
      addEdges: [],
      updateEdges: [],
      removeEdgeIds: []
    });

    expect(next.nodes.map((node) => node.id)).toEqual(["a"]);
    expect(next.edges).toEqual([]);
  });

  it("creates scoped subgraphs around a selected node", () => {
    const scoped = selectedSubgraph(graph, { kind: "node", id: "a" });
    expect(scoped.nodes.map((node) => node.id).sort()).toEqual(["a", "b"]);
    expect(scoped.edges).toHaveLength(1);
  });

  it("exports valid mermaid flowchart syntax", () => {
    expect(makeMermaid(graph)).toContain("flowchart LR");
    expect(makeMermaid(graph)).toContain('a -->|"supports" b');
  });
});
