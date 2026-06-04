import { describe, expect, it } from "vitest";
import { sceneGraphForGroup } from "../src/shared/graph";
import { generateLocalExport, seedGroupScene } from "../src/server/local";
import type { ExportType, Scene } from "../src/shared/schema";

const seed = seedGroupScene("Choose an export format for architecture decisions", {
  groupId: "group-test",
  now: "2026-06-05T00:00:00.000Z"
});
const scene: Scene = {
  version: 1,
  sceneVersion: 0,
  groups: [seed.group],
  nodes: seed.nodes,
  edges: seed.edges,
  tags: [],
  comments: [],
  artifacts: [],
  selection: { kind: "canvas" },
  updatedAt: "2026-06-05T00:00:00.000Z"
};
const graph = sceneGraphForGroup(scene, seed.group.id);

function exportContent(type: ExportType) {
  return generateLocalExport(graph, { type, scope: { kind: "group", id: seed.group.id } }, "Export Format Decision");
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
