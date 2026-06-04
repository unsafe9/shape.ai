import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { SceneGroup, SceneNode } from "../src/shared/schema";

afterEach(() => {
  delete process.env.SHAPE_AI_DATA_DIR;
  vi.resetModules();
});

describe("scene LOD performance contracts", () => {
  it("keeps 10k-object reads bounded by zoom and viewport", async () => {
    process.env.SHAPE_AI_DATA_DIR = await mkdtemp(join(tmpdir(), "shape-ai-perf-"));
    vi.resetModules();
    const storage = await import("../src/server/storage");
    const now = "2026-06-05T00:00:00.000Z";
    const group: SceneGroup = {
      id: "group-10k",
      parentGroupId: null,
      title: "10k fixture",
      summary: "Performance fixture",
      bounds: { x: -120, y: -120, width: 14200, height: 14200 },
      tagIds: [],
      zIndex: 0,
      collapsed: false,
      createdAt: now,
      updatedAt: now
    };
    const nodes: SceneNode[] = Array.from({ length: 10_000 }, (_, index) => {
      const x = (index % 100) * 140;
      const y = Math.floor(index / 100) * 140;
      return {
        id: `node-${index}`,
        groupId: group.id,
        type: index % 3 === 0 ? "decision_point" : index % 3 === 1 ? "option" : "evidence",
        title: `Node ${index}`,
        summary: "Fixture node",
        detail: "Fixture node",
        status: "draft",
        confidence: 0.5,
        evidenceRefs: [],
        childDecisionIds: [],
        position: { x, y },
        size: { width: 80, height: 80 },
        zIndex: index,
        updatedAt: now
      };
    });

    await storage.saveScenePatch({ groups: [group], nodes });

    const overview = await storage.readScene({
      viewport: { x: 0, y: 0, width: 1200, height: 900 },
      zoom: 0.05
    });
    expect(overview.groups).toHaveLength(1);
    expect(overview.nodes).toHaveLength(0);
    expect(overview.edges).toHaveLength(0);

    const detail = await storage.readScene({
      viewport: { x: 0, y: 0, width: 1200, height: 900 },
      zoom: 1
    });
    expect(detail.nodes.length).toBeGreaterThan(0);
    expect(detail.nodes.length).toBeLessThan(10_000);
  });
});
