import { describe, expect, it } from "vitest";
import type { Scene } from "../src/shared/schema";
import { sceneGraphForGroup, selectedSubgraph } from "../src/shared/graph";
import { excludedBusinessFields, shapeSceneToRenderSnapshot } from "../src/shared/renderScene";
import { generateLocalExport } from "../src/server/local";
import {
  addShapeSceneComment,
  applyRenderPatchToShapeScene,
  createShapeSceneFixture,
  shapeSceneToFilteredRenderSnapshot,
  updateShapeSceneGroupTags
} from "../poc/infinite-canvas/web/src/adapter";
import { createBenchmarkFixture } from "../poc/infinite-canvas/web/src/fixtures";
import { applyScenePatch, buildSpatialIndex, querySpatialIndex, validateScenePatch } from "../poc/infinite-canvas/web/src/scene";

describe("infinite canvas POC contract", () => {
  it("generates deterministic 1,000+ card and edge fixtures", () => {
    const first = createBenchmarkFixture({ seed: 11, cards: 1_050, edges: 1_080 });
    const second = createBenchmarkFixture({ seed: 11, cards: 1_050, edges: 1_080 });

    expect(first.cards).toHaveLength(1_050);
    expect(first.edges).toHaveLength(1_080);
    expect(first.cards[42]).toEqual(second.cards[42]);
    expect(first.edges[42]).toEqual(second.edges[42]);
  });

  it("keeps business-only fields out of the render scene adapter", () => {
    const scene: Scene = {
      version: 1,
      sceneVersion: 3,
      groups: [
        {
          id: "group-a",
          parentGroupId: null,
          title: "Group A",
          summary: "Group summary",
          bounds: { x: 0, y: 0, width: 900, height: 700 },
          tagIds: ["tag-a"],
          zIndex: 0,
          collapsed: false,
          createdAt: "2026-06-05T00:00:00.000Z",
          updatedAt: "2026-06-05T00:00:00.000Z"
        }
      ],
      nodes: [
        {
          id: "node-a",
          groupId: "group-a",
          type: "decision_point",
          title: "Decision A",
          summary: "Render this summary",
          detail: "Render this detail only while editing",
          status: "draft",
          confidence: 0.91,
          evidenceRefs: ["secret-source"],
          childDecisionIds: ["child-a"],
          position: { x: 120, y: 160 },
          size: { width: 270, height: 178 },
          zIndex: 4,
          updatedAt: "2026-06-05T00:00:00.000Z"
        }
      ],
      edges: [],
      tags: [
        {
          id: "tag-a",
          name: "Research",
          color: "#123456",
          description: "Business tag registry",
          createdAt: "2026-06-05T00:00:00.000Z",
          updatedAt: "2026-06-05T00:00:00.000Z"
        }
      ],
      comments: [
        {
          id: "comment-a",
          target: { kind: "node", id: "node-a" },
          body: "Renderer must not know this",
          author: "human",
          resolved: false,
          createdAt: "2026-06-05T00:00:00.000Z",
          updatedAt: "2026-06-05T00:00:00.000Z"
        }
      ],
      artifacts: [],
      selection: { kind: "node", id: "node-a" },
      updatedAt: "2026-06-05T00:00:00.000Z"
    };

    const snapshot = shapeSceneToRenderSnapshot(scene);
    const serialized = JSON.stringify(snapshot);

    expect(snapshot.cards[0].title).toBe("Decision A");
    expect(snapshot.cards[0]).not.toHaveProperty("confidence");
    expect(serialized).not.toContain("secret-source");
    expect(serialized).not.toContain("Renderer must not know this");
    expect(excludedBusinessFields()).toContain("Scene.comments");
  });

  it("loads a deterministic Shape scene fixture through the render adapter", () => {
    const scene = createShapeSceneFixture();
    const snapshot = shapeSceneToRenderSnapshot(scene);
    const serialized = JSON.stringify(snapshot);

    expect(snapshot.metadata.source).toBe("shape-scene-adapter");
    expect(snapshot.cards.map((card) => card.id)).toEqual(scene.nodes.map((node) => node.id));
    expect(snapshot.edges.map((edge) => edge.id)).toEqual(scene.edges.map((edge) => edge.id));
    expect(serialized).not.toContain(scene.comments[0].body);
    expect(serialized).not.toContain(scene.artifacts[0].path);
  });

  it("filters app scene render snapshots by group tags", () => {
    const scene = createShapeSceneFixture();
    const rendererOnly = shapeSceneToFilteredRenderSnapshot(scene, ["tag-renderer"]);
    const productOnly = shapeSceneToFilteredRenderSnapshot(scene, ["tag-product"]);

    expect(rendererOnly.groups.map((group) => group.id)).toEqual(["shape-group-renderer"]);
    expect(rendererOnly.cards.map((card) => card.groupId)).toEqual(["shape-group-renderer", "shape-group-renderer"]);
    expect(rendererOnly.edges.map((edge) => edge.id)).toEqual(["shape-edge-contract-wgpu"]);
    expect(rendererOnly.selection).toEqual({ kind: "node", id: "shape-node-contract" });

    expect(productOnly.groups.map((group) => group.id)).toEqual(["shape-group-parity"]);
    expect(productOnly.cards.map((card) => card.groupId)).toEqual(["shape-group-parity", "shape-group-parity"]);
    expect(productOnly.edges.map((edge) => edge.id)).toEqual(["shape-edge-overlay-export"]);
    expect(productOnly.selection).toEqual({ kind: "canvas" });
  });

  it("updates group tags in the app-owned scene shell", () => {
    const scene = createShapeSceneFixture();
    const updated = updateShapeSceneGroupTags(
      scene,
      "shape-group-renderer",
      ["tag-renderer", "tag-product"],
      "2026-06-05T02:45:00.000Z"
    );

    expect(updated.errors).toEqual([]);
    expect(updated.scene.sceneVersion).toBe(scene.sceneVersion + 1);
    expect(updated.scene.groups.find((group) => group.id === "shape-group-renderer")?.tagIds).toEqual(["tag-renderer", "tag-product"]);
    expect(updated.appPatch.groups?.[0]).toMatchObject({
      id: "shape-group-renderer",
      tagIds: ["tag-renderer", "tag-product"]
    });
    expect(updated.appPatch.selection).toEqual({ kind: "group", id: "shape-group-renderer" });

    const invalid = updateShapeSceneGroupTags(scene, "shape-group-renderer", ["missing-tag"]);
    expect(invalid.errors).toEqual(["Unknown tag id: missing-tag"]);
    expect(invalid.scene).toBe(scene);
  });

  it("keeps comments in the app-owned shell outside renderer snapshots", () => {
    const scene = createShapeSceneFixture();
    const commented = addShapeSceneComment(
      scene,
      { kind: "node", id: "shape-node-contract" },
      "Renderer replacement should not serialize this comment.",
      "2026-06-05T02:50:00.000Z"
    );
    const snapshot = shapeSceneToRenderSnapshot(commented.scene);

    expect(commented.errors).toEqual([]);
    expect(commented.scene.sceneVersion).toBe(scene.sceneVersion + 1);
    expect(commented.comment?.target).toEqual({ kind: "node", id: "shape-node-contract" });
    expect(commented.scene.comments[0].body).toBe("Renderer replacement should not serialize this comment.");
    expect(JSON.stringify(snapshot)).not.toContain("Renderer replacement should not serialize this comment.");
  });

  it("translates renderer drag and text patches back into app scene patches", () => {
    const scene = createShapeSceneFixture();
    const moved = applyRenderPatchToShapeScene(
      scene,
      { kind: "move-card", id: "shape-node-contract", position: { x: 320, y: 440 } },
      "2026-06-05T01:00:00.000Z"
    );

    expect(moved.errors).toEqual([]);
    expect(moved.scene.sceneVersion).toBe(scene.sceneVersion + 1);
    expect(moved.scene.nodes.find((node) => node.id === "shape-node-contract")?.position).toEqual({ x: 320, y: 440 });
    expect(moved.appPatch.nodes?.[0]?.id).toBe("shape-node-contract");
    expect(moved.appPatch.selection).toEqual({ kind: "node", id: "shape-node-contract" });

    const edited = applyRenderPatchToShapeScene(
      moved.scene,
      { kind: "edit-card-text", id: "shape-node-contract", field: "summary", value: "Updated from the renderer overlay." },
      "2026-06-05T01:05:00.000Z"
    );

    expect(edited.errors).toEqual([]);
    expect(edited.scene.nodes.find((node) => node.id === "shape-node-contract")?.summary).toBe("Updated from the renderer overlay.");
    expect(edited.appPatch.nodes?.[0]?.summary).toBe("Updated from the renderer overlay.");
  });

  it("translates renderer edge create and delete patches into app scene patches", () => {
    const scene = createShapeSceneFixture();
    const created = applyRenderPatchToShapeScene(
      scene,
      {
        kind: "create-edge",
        groupId: "shape-group-renderer",
        source: "shape-node-contract",
        target: "shape-node-overlay",
        edgeId: "shape-edge-new",
        label: "connects"
      },
      "2026-06-05T02:00:00.000Z"
    );

    expect(created.errors).toEqual([]);
    expect(created.scene.edges.some((edge) => edge.id === "shape-edge-new")).toBe(true);
    expect(created.appPatch.edges?.[0]).toMatchObject({
      id: "shape-edge-new",
      type: "supports",
      rationale: "",
      confidence: 0.5
    });

    const removed = applyRenderPatchToShapeScene(
      created.scene,
      { kind: "delete-edge", id: "shape-edge-new" },
      "2026-06-05T02:05:00.000Z"
    );

    expect(removed.errors).toEqual([]);
    expect(removed.scene.edges.some((edge) => edge.id === "shape-edge-new")).toBe(false);
    expect(removed.appPatch.removeEdgeIds).toEqual(["shape-edge-new"]);
    expect(removed.appPatch.selection).toEqual({ kind: "canvas" });
  });

  it("translates renderer card create and delete patches into app scene patches", () => {
    const scene = createShapeSceneFixture();
    const created = applyRenderPatchToShapeScene(
      scene,
      {
        kind: "create-card",
        card: {
          id: "shape-node-created",
          groupId: "shape-group-renderer",
          title: "Renderer-created node",
          summary: "Created through the render patch contract.",
          detail: "Business defaults are assigned by the app adapter.",
          status: "draft",
          type: "task",
          bounds: { x: 360, y: 300, width: 320, height: 172 },
          zIndex: 9,
          styleKey: "decision",
          accessibilityLabel: "task Renderer-created node. Created through the render patch contract."
        }
      },
      "2026-06-05T02:10:00.000Z"
    );

    expect(created.errors).toEqual([]);
    expect(created.scene.nodes.some((node) => node.id === "shape-node-created")).toBe(true);
    expect(created.appPatch.nodes?.[0]).toMatchObject({
      id: "shape-node-created",
      groupId: "shape-group-renderer",
      type: "task",
      confidence: 0.5,
      position: { x: 360, y: 300 },
      size: { width: 320, height: 172 }
    });
    expect(created.appPatch.selection).toEqual({ kind: "node", id: "shape-node-created" });

    const removed = applyRenderPatchToShapeScene(
      created.scene,
      { kind: "delete-card", id: "shape-node-wgpu" },
      "2026-06-05T02:15:00.000Z"
    );

    expect(removed.errors).toEqual([]);
    expect(removed.scene.nodes.some((node) => node.id === "shape-node-wgpu")).toBe(false);
    expect(removed.scene.edges.some((edge) => edge.source === "shape-node-wgpu" || edge.target === "shape-node-wgpu")).toBe(false);
    expect(removed.appPatch.removeNodeIds).toEqual(["shape-node-wgpu"]);
    expect(removed.appPatch.removeEdgeIds?.sort()).toEqual(["shape-edge-contract-wgpu", "shape-edge-wgpu-overlay"]);
    expect(removed.appPatch.selection).toEqual({ kind: "canvas" });
  });

  it("translates renderer group create and delete patches into app scene patches", () => {
    const scene = createShapeSceneFixture();
    const created = applyRenderPatchToShapeScene(
      scene,
      {
        kind: "create-group",
        group: {
          id: "shape-group-created",
          title: "Renderer-created group",
          summary: "Created through the render patch contract.",
          bounds: { x: 2600, y: 180, width: 960, height: 640 },
          tagIds: [],
          zIndex: 3,
          styleKey: "default"
        }
      },
      "2026-06-05T02:20:00.000Z"
    );

    expect(created.errors).toEqual([]);
    expect(created.scene.groups.some((group) => group.id === "shape-group-created")).toBe(true);
    expect(created.appPatch.groups?.[0]).toMatchObject({
      id: "shape-group-created",
      parentGroupId: null,
      title: "Renderer-created group",
      collapsed: false,
      bounds: { x: 2600, y: 180, width: 960, height: 640 }
    });
    expect(created.appPatch.selection).toEqual({ kind: "group", id: "shape-group-created" });

    const removed = applyRenderPatchToShapeScene(
      created.scene,
      { kind: "delete-group", id: "shape-group-renderer" },
      "2026-06-05T02:25:00.000Z"
    );

    expect(removed.errors).toEqual([]);
    expect(removed.scene.groups.some((group) => group.id === "shape-group-renderer")).toBe(false);
    expect(removed.scene.nodes.some((node) => node.groupId === "shape-group-renderer")).toBe(false);
    expect(removed.scene.edges.some((edge) => edge.groupId === "shape-group-renderer" || edge.source === "shape-node-wgpu" || edge.target === "shape-node-wgpu")).toBe(false);
    expect(removed.appPatch.removeGroupIds).toEqual(["shape-group-renderer"]);
    expect([...(removed.appPatch.removeNodeIds ?? [])].sort()).toEqual(["shape-node-contract", "shape-node-wgpu"]);
    expect([...(removed.appPatch.removeEdgeIds ?? [])].sort()).toEqual(["shape-edge-contract-wgpu", "shape-edge-wgpu-overlay"]);
    expect(removed.appPatch.selection).toEqual({ kind: "canvas" });
  });

  it("translates renderer group move patches into app translateGroups patches", () => {
    const scene = createShapeSceneFixture();
    const beforeGroup = scene.groups.find((group) => group.id === "shape-group-renderer")!;
    const beforeNodes = scene.nodes.filter((node) => node.groupId === "shape-group-renderer");
    const moved = applyRenderPatchToShapeScene(
      scene,
      { kind: "move-group", id: "shape-group-renderer", delta: { x: 125, y: -80 } },
      "2026-06-05T02:30:00.000Z"
    );

    expect(moved.errors).toEqual([]);
    expect(moved.appPatch.translateGroups).toEqual([{ groupId: "shape-group-renderer", dx: 125, dy: -80 }]);
    expect(moved.appPatch.selection).toEqual({ kind: "group", id: "shape-group-renderer" });
    expect(moved.scene.groups.find((group) => group.id === "shape-group-renderer")?.bounds).toMatchObject({
      x: beforeGroup.bounds.x + 125,
      y: beforeGroup.bounds.y - 80
    });
    for (const beforeNode of beforeNodes) {
      const afterNode = moved.scene.nodes.find((node) => node.id === beforeNode.id);
      expect(afterNode?.position).toEqual({ x: beforeNode.position.x + 125, y: beforeNode.position.y - 80 });
    }
  });

  it("translates renderer z-order patches into app node patches", () => {
    const scene = createShapeSceneFixture();
    const layered = applyRenderPatchToShapeScene(
      scene,
      { kind: "set-card-z-index", id: "shape-node-contract", zIndex: 44 },
      "2026-06-05T02:35:00.000Z"
    );

    expect(layered.errors).toEqual([]);
    expect(layered.scene.nodes.find((node) => node.id === "shape-node-contract")?.zIndex).toBe(44);
    expect(layered.appPatch.nodes?.[0]).toMatchObject({
      id: "shape-node-contract",
      zIndex: 44
    });
    expect(layered.appPatch.selection).toEqual({ kind: "node", id: "shape-node-contract" });
  });

  it("uses create-card patches for duplicate and paste parity", () => {
    const scene = createShapeSceneFixture();
    const snapshot = shapeSceneToRenderSnapshot(scene);
    const source = snapshot.cards.find((card) => card.id === "shape-node-contract")!;
    const duplicated = {
      ...source,
      id: "shape-node-contract-copy",
      title: `${source.title} copy`,
      bounds: { ...source.bounds, x: source.bounds.x + 450, y: source.bounds.y + 430 },
      zIndex: 12,
      accessibilityLabel: "decision_point Scene contract copy. Renderer receives stable ids, bounds, text snippets, style keys, and selection seed."
    };

    const snapshotAfterDuplicate = applyScenePatch(snapshot, { kind: "create-card", card: duplicated });
    const applied = applyRenderPatchToShapeScene(scene, { kind: "create-card", card: duplicated }, "2026-06-05T02:40:00.000Z");

    expect(snapshotAfterDuplicate.cards.some((card) => card.id === duplicated.id)).toBe(true);
    expect(snapshotAfterDuplicate.selection).toEqual({ kind: "node", id: duplicated.id });
    expect(applied.errors).toEqual([]);
    expect(applied.appPatch.nodes?.[0]).toMatchObject({
      id: duplicated.id,
      title: "Scene contract copy",
      position: { x: source.bounds.x + 450, y: source.bounds.y + 430 },
      zIndex: 12
    });
    expect(applied.appPatch.selection).toEqual({ kind: "node", id: duplicated.id });
  });

  it("rejects invalid renderer patches before mutating app scene state", () => {
    const scene = createShapeSceneFixture();
    const result = applyRenderPatchToShapeScene(scene, {
      kind: "create-edge",
      groupId: "shape-group-renderer",
      source: "shape-node-contract",
      target: "shape-node-contract",
      edgeId: "shape-edge-self"
    });

    expect(result.errors).toContain("Edge source and target must differ");
    expect(result.scene).toBe(scene);
    expect(result.appPatch).toEqual({});
  });

  it("keeps app export semantics independent from render snapshots", () => {
    const scene = createShapeSceneFixture();
    const snapshot = shapeSceneToRenderSnapshot(scene);
    const graph = sceneGraphForGroup(scene, "shape-group-renderer");
    const mermaid = generateLocalExport(graph, { type: "mermaid", scope: { kind: "group", id: "shape-group-renderer" } }, "Renderer migration");
    const selected = selectedSubgraph(graph, { kind: "node", id: "shape-node-contract" });

    expect(snapshot.metadata.source).toBe("shape-scene-adapter");
    expect(JSON.stringify(snapshot)).not.toContain("exports/renderer-decision.md");
    expect(mermaid.content).toContain("flowchart LR");
    expect(mermaid.content).toContain("shape_node_contract");
    expect(mermaid.content).not.toContain("shape-comment-renderer");
    expect(selected.nodes.map((node) => node.id).sort()).toEqual(["shape-node-contract", "shape-node-wgpu"]);
    expect(selected.edges.map((edge) => edge.id)).toEqual(["shape-edge-contract-wgpu"]);
  });

  it("uses the spatial index to query visible card candidates", () => {
    const fixture = createBenchmarkFixture({ seed: 5, cards: 1_100, edges: 1_100 });
    const index = buildSpatialIndex(fixture.cards, 640);
    const ids = querySpatialIndex(index, { x: 0, y: 0, width: 1400, height: 900 });

    expect(ids.size).toBeGreaterThan(0);
    expect(ids.size).toBeLessThan(fixture.cards.length);
  });

  it("rejects invalid engine patches at the app boundary", () => {
    const fixture = createBenchmarkFixture({ seed: 3, cards: 20, edges: 10 });
    const errors = validateScenePatch(fixture, {
      kind: "create-edge",
      groupId: "missing-group",
      source: "fixture-card-1",
      target: "missing-card",
      edgeId: "edge-x"
    });

    expect(errors).toContain("Unknown target card id: missing-card");
    expect(errors).toContain("Unknown group id: missing-group");
  });
});
