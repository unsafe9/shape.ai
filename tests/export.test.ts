import { describe, expect, it } from "vitest";
import { generateLocalExport, seedDesignGraph } from "../src/server/local";
import type { ExportType } from "../src/shared/schema";

const graph = seedDesignGraph("Choose an export format for architecture decisions").graph;

function exportContent(type: ExportType) {
  return generateLocalExport(graph, { type, scope: { kind: "whole_graph" } }, "Export Format Decision");
}

describe("local exports", () => {
  it("generates MADR markdown", () => {
    const output = exportContent("madr");
    expect(output.title).toContain("MADR");
    expect(output.content).toContain("## Context and Problem Statement");
    expect(output.content).toContain("## Decision Drivers");
    expect(output.content).toContain("## Decision Outcome");
  });

  it("generates YADR YAML", () => {
    const output = exportContent("yadr");
    expect(output.title).toContain("YADR");
    expect(output.content).toContain("metadata:");
    expect(output.content).toContain("context-and-problem-statement: |");
    expect(output.content).toContain("decision-outcome:");
  });

  it("generates an image prompt instead of an SVG preview", () => {
    const output = exportContent("image_prompt");
    expect(output.content).toContain("Create a clean architecture decision diagram");
    expect(output.imagePrompt).toBe(output.content);
    expect(output.content).not.toContain("<svg");
  });
});
