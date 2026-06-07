import { describe, expect, it } from "vitest";
import type { Scene } from "../src/shared/schema";
import { sceneGraphForGroup, selectedSubgraph } from "../src/shared/graph";
import { defaultStyles, excludedBusinessFields, shapeSceneToRenderSnapshot } from "../src/shared/renderScene";
import { generateLocalExport } from "../src/server/local";
import { ShapeCanvasEngine, type EngineEvent } from "../src/client/renderer/engine";
import {
  createShapeSceneFixture,
  shapeSceneToFilteredRenderSnapshot
} from "../src/client/renderer/adapter";
// Op-apply is the golden-oracle TS (test-only); imported directly from its module
// rather than re-exported through the client-runtime adapter.
import {
  addShapeSceneComment,
  applyRenderPatchToShapeScene,
  updateShapeSceneGroupTags
} from "../src/shared/renderPatch";
import { createBenchmarkFixture } from "../src/client/renderer/fixtures";
import {
  applyScenePatch,
  type CameraState,
  type DomOverlayRequest,
  type ScenePatch,
  type SceneSnapshot,
  type WorldRect,
  validateScenePatch
} from "../src/client/renderer/scene";
import type { RustDebugSnapshot, RustWebGpuFrameStats, RustWebGpuRenderer } from "../src/client/renderer/wasmLoader";

describe("infinite canvas renderer contract", () => {
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
          tagIds: [],
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

  it("carries rich shape.ai visual tokens through the render adapter", () => {
    const scene = createShapeSceneFixture();
    const snapshot = shapeSceneToRenderSnapshot(scene);
    const decisionPoint = snapshot.styles.find((style) => style.id === "decision_point");

    expect(defaultStyles.map((style) => style.id)).toEqual(
      expect.arrayContaining(["default", "decision", "risk", "decision_point", "blocker", "task", "artifact"])
    );
    expect(snapshot.cards.find((card) => card.id === "shape-node-contract")?.styleKey).toBe("decision_point");
    expect(decisionPoint?.pastel).toBe("#ebf4ff");
    expect(decisionPoint?.radius?.card).toBe(16);
    expect(decisionPoint?.strokeWidths?.edgeSelected).toBe(5);
    expect(decisionPoint?.typography?.cardTitleSize).toBe(19);
    expect(decisionPoint?.typography).not.toHaveProperty("titleWeight");
    expect(decisionPoint?.typography).not.toHaveProperty("labelWeight");
    expect(decisionPoint?.shadow?.[0]).toMatchObject({ offsetY: 18, blur: 36, color: "#192430" });
    expect(decisionPoint?.badge?.minWidth).toBe(54);
    expect(JSON.parse(JSON.stringify(snapshot)).styles.find((style: { id: string }) => style.id === "decision_point")?.port?.strokeAlpha).toBe(0.46);
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

    const invalidCard = applyRenderPatchToShapeScene(scene, {
      kind: "create-card",
      card: {
        id: "shape-node-invalid-size",
        groupId: "shape-group-renderer",
        title: "Invalid size",
        summary: "Renderer patches must not create non-positive app nodes.",
        detail: "",
        status: "draft",
        type: "task",
        bounds: { x: 0, y: 0, width: 0, height: 172 },
        zIndex: 0,
        styleKey: "default",
        accessibilityLabel: "task Invalid size"
      }
    });

    expect(invalidCard.errors).toContain("Card bounds must be positive");
    expect(invalidCard.scene).toBe(scene);
    expect(invalidCard.appPatch).toEqual({});
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

  it("mounts a focused native textarea with Rust-provided overlay style", () => {
    const fakeDocument = installFakeTextAreaDocument();
    try {
      const fixture = createBenchmarkFixture({ seed: 8, cards: 4, edges: 2 });
      const card = fixture.cards[0];
      const appended: HTMLElement[] = [];
      const events: EngineEvent[] = [];
      const engine = new ShapeCanvasEngine({
        canvas: testCanvas(),
        overlayRoot: testOverlayRoot(appended),
        backend: "test",
        webGpuRenderer: createOverlayTestRenderer(fixture),
        onEvent: (event) => events.push(event)
      });

      engine.loadScene(fixture);
      const request = engine.beginTextEdit({
        kind: "text",
        id: card.id,
        groupId: card.groupId,
        field: "summary",
        world: { x: card.bounds.x, y: card.bounds.y },
        screen: { x: card.bounds.x, y: card.bounds.y }
      });
      const textarea = fakeDocument.created[0];

      expect(request?.target.field).toBe("summary");
      expect(appended[0]).toBe(textarea as unknown as HTMLElement);
      expect(textarea.value).toBe(card.summary);
      expect(textarea.focused).toBe(true);
      expect(textarea.selected).toBe(true);
      expect(textarea.style.left).toBe(`${request?.screenRect.x}px`);
      expect(textarea.style.fontFamily).toContain("Noto Sans KR");
      expect(textarea.style.color).toBe("rgba(90, 113, 136, 0.840)");
      expect(textarea.style.background).toBe("rgba(255, 255, 255, 0.980)");
      expect(textarea.style.border).toBe("0.18px solid rgba(47, 126, 230, 0.520)");
      expect(textarea.style.outline).toBe("0.72px solid rgba(47, 126, 230, 0.120)");
      expect(textarea.style.caretColor).toBe("rgba(47, 126, 230, 0.920)");
      expect(textarea.style.getPropertyValue("accent-color")).toBe("rgba(47, 126, 230, 0.920)");
      expect(textarea.style.getPropertyValue("--renderer-edit-selection-bg")).toBe("rgba(47, 126, 230, 0.200)");
      expect(textarea.dataset.overlayState).toBe("selected");
      expect(textarea.dataset.maxLines).toBe("3");
      expect(events.filter((event) => event.type === "overlay" && event.request === null)).toHaveLength(0);
      expect(events.filter((event) => event.type === "status" && event.message === "Text edit cancelled")).toHaveLength(0);
    } finally {
      fakeDocument.restore();
    }
  });

  it("commits typed overlay text on Cmd/Ctrl+Enter and removes the textarea", () => {
    const fakeDocument = installFakeTextAreaDocument();
    try {
      const fixture = createBenchmarkFixture({ seed: 9, cards: 4, edges: 2 });
      const card = fixture.cards[0];
      const events: EngineEvent[] = [];
      const engine = new ShapeCanvasEngine({
        canvas: testCanvas(),
        overlayRoot: testOverlayRoot(),
        backend: "test",
        webGpuRenderer: createOverlayTestRenderer(fixture),
        onEvent: (event) => events.push(event)
      });

      engine.loadScene(fixture);
      engine.beginTextEdit({
        kind: "text",
        id: card.id,
        groupId: card.groupId,
        field: "summary",
        world: { x: card.bounds.x, y: card.bounds.y },
        screen: { x: card.bounds.x, y: card.bounds.y }
      });
      const textarea = fakeDocument.created[0];
      textarea.value = "한국어 요약 committed natively";
      const keyEvent = textarea.dispatch("keydown", { key: "Enter", metaKey: true, ctrlKey: false });

      expect(keyEvent.defaultPrevented).toBe(true);
      expect(engine.getSnapshot()?.cards.find((candidate) => candidate.id === card.id)?.summary).toBe("한국어 요약 committed natively");
      expect(textarea.removed).toBe(true);
      expect(events.filter((event) => event.type === "overlay" && event.request === null)).toHaveLength(1);
      expect(events.filter((event) => event.type === "status" && event.message === "Text edit cancelled")).toHaveLength(0);
      const editPatches = events.filter(
        (event) => event.type === "patch" && event.patch.kind === "edit-card-text" && event.patch.value === "한국어 요약 committed natively"
      );
      expect(editPatches).toHaveLength(1);
    } finally {
      fakeDocument.restore();
    }
  });

  it("commits overlay text on blur and keeps browser blur behavior native", () => {
    const fakeDocument = installFakeTextAreaDocument();
    try {
      const fixture = createBenchmarkFixture({ seed: 10, cards: 4, edges: 2 });
      const card = fixture.cards[0];
      const engine = new ShapeCanvasEngine({
        canvas: testCanvas(),
        overlayRoot: testOverlayRoot(),
        backend: "test",
        webGpuRenderer: createOverlayTestRenderer(fixture),
        onEvent() {}
      });

      engine.loadScene(fixture);
      engine.beginTextEdit({
        kind: "text",
        id: card.id,
        groupId: card.groupId,
        field: "title",
        world: { x: card.bounds.x, y: card.bounds.y },
        screen: { x: card.bounds.x, y: card.bounds.y }
      });
      const textarea = fakeDocument.created[0];
      textarea.value = "Blur committed title";
      textarea.dispatch("blur");

      expect(engine.getSnapshot()?.cards.find((candidate) => candidate.id === card.id)?.title).toBe("Blur committed title");
      expect(textarea.removed).toBe(true);
    } finally {
      fakeDocument.restore();
    }
  });

  it("cancels overlay text on Escape without persisting edits", () => {
    const fakeDocument = installFakeTextAreaDocument();
    try {
      const fixture = createBenchmarkFixture({ seed: 11, cards: 4, edges: 2 });
      const card = fixture.cards[0];
      const events: EngineEvent[] = [];
      const engine = new ShapeCanvasEngine({
        canvas: testCanvas(),
        overlayRoot: testOverlayRoot(),
        backend: "test",
        webGpuRenderer: createOverlayTestRenderer(fixture),
        onEvent: (event) => events.push(event)
      });

      engine.loadScene(fixture);
      engine.beginTextEdit({
        kind: "text",
        id: card.id,
        groupId: card.groupId,
        field: "summary",
        world: { x: card.bounds.x, y: card.bounds.y },
        screen: { x: card.bounds.x, y: card.bounds.y }
      });
      const textarea = fakeDocument.created[0];
      textarea.value = "This should not persist";
      const keyEvent = textarea.dispatch("keydown", { key: "Escape", metaKey: false, ctrlKey: false });

      expect(keyEvent.defaultPrevented).toBe(true);
      expect(engine.getSnapshot()?.cards.find((candidate) => candidate.id === card.id)?.summary).toBe(card.summary);
      expect(textarea.removed).toBe(true);
      expect(events.filter((event) => event.type === "overlay" && event.request === null)).toHaveLength(1);
      expect(events.filter((event) => event.type === "status" && event.message === "Text edit cancelled")).toHaveLength(1);
      expect(events.filter((event) => event.type === "patch" && event.patch.kind === "edit-card-text")).toHaveLength(0);
    } finally {
      fakeDocument.restore();
    }
  });

  it("keeps composing Escape native without cancelling the overlay", () => {
    const fakeDocument = installFakeTextAreaDocument();
    try {
      const fixture = createBenchmarkFixture({ seed: 13, cards: 4, edges: 2 });
      const card = fixture.cards[0];
      const events: EngineEvent[] = [];
      const engine = new ShapeCanvasEngine({
        canvas: testCanvas(),
        overlayRoot: testOverlayRoot(),
        backend: "test",
        webGpuRenderer: createOverlayTestRenderer(fixture),
        onEvent: (event) => events.push(event)
      });

      engine.loadScene(fixture);
      engine.beginTextEdit({
        kind: "text",
        id: card.id,
        groupId: card.groupId,
        field: "summary",
        world: { x: card.bounds.x, y: card.bounds.y },
        screen: { x: card.bounds.x, y: card.bounds.y }
      });
      const textarea = fakeDocument.created[0];
      textarea.value = "composition text still active";
      const keyEvent = textarea.dispatch("keydown", { key: "Escape", isComposing: true });

      expect(keyEvent.defaultPrevented).toBe(false);
      expect(textarea.removed).toBe(false);
      expect(engine.getSnapshot()?.cards.find((candidate) => candidate.id === card.id)?.summary).toBe(card.summary);
      expect(events.filter((event) => event.type === "overlay" && event.request === null)).toHaveLength(0);
      expect(events.filter((event) => event.type === "status" && event.message === "Text edit cancelled")).toHaveLength(0);
      expect(events.filter((event) => event.type === "patch" && event.patch.kind === "edit-card-text")).toHaveLength(0);
    } finally {
      fakeDocument.restore();
    }
  });

  it("refreshes active overlay geometry and style after Rust camera and card movement", () => {
    const fakeDocument = installFakeTextAreaDocument();
    try {
      const fixture = createBenchmarkFixture({ seed: 12, cards: 4, edges: 2 });
      const card = fixture.cards[0];
      const engine = new ShapeCanvasEngine({
        canvas: testCanvas(),
        overlayRoot: testOverlayRoot(),
        backend: "test",
        webGpuRenderer: createOverlayTestRenderer(fixture),
        onEvent() {}
      });

      engine.loadScene(fixture);
      engine.beginTextEdit({
        kind: "text",
        id: card.id,
        groupId: card.groupId,
        field: "summary",
        world: { x: card.bounds.x, y: card.bounds.y },
        screen: { x: card.bounds.x, y: card.bounds.y }
      });
      const textarea = fakeDocument.created[0];
      const initialLeft = textarea.style.left;
      const initialFontSize = textarea.style.fontSize;

      engine.setCamera({ x: 32, y: 48, zoom: 0.5 });

      expect(textarea.style.left).not.toBe(initialLeft);
      expect(textarea.style.fontSize).not.toBe(initialFontSize);

      const cameraLeft = textarea.style.left;
      engine.applyPatch({ kind: "move-card", id: card.id, position: { x: card.bounds.x + 80, y: card.bounds.y + 40 } });

      expect(textarea.style.left).not.toBe(cameraLeft);
      expect(textarea.style.top).toBe(`${(card.bounds.y + 40 + 52 - 7.03) * 0.5 + 48}px`);
    } finally {
      fakeDocument.restore();
    }
  });

  it("routes focus bounds through the Rust input boundary", () => {
    const fixture = createBenchmarkFixture({ seed: 17, cards: 4, edges: 2 });
    const card = fixture.cards[0];
    const renderer = createOverlayTestRenderer(fixture);
    const inputBatch = renderer.inputBatch.bind(renderer);
    let inputEvents: unknown[] = [];
    renderer.inputBatch = (eventsJson) => {
      inputEvents = JSON.parse(eventsJson) as unknown[];
      return inputBatch(eventsJson);
    };
    const engine = new ShapeCanvasEngine({
      canvas: testCanvas(),
      overlayRoot: testOverlayRoot(),
      backend: "test",
      webGpuRenderer: renderer,
      onEvent() {}
    });

    engine.loadScene(fixture);
    engine.focusBounds(card.bounds, { screen: { x: 400, y: 240 }, zoom: 0.9 });

    expect(inputEvents).toEqual([{ kind: "focus-bounds", bounds: card.bounds, screen: { x: 400, y: 240 }, zoom: 0.9 }]);
    expect(engine.getCamera()).toEqual({
      zoom: 0.9,
      x: 400 - (card.bounds.x + card.bounds.width / 2) * 0.9,
      y: 240 - (card.bounds.y + card.bounds.height / 2) * 0.9
    });
  });

  it("syncs external selection without reloading the Rust scene or emitting app patches", () => {
    const fixture = createBenchmarkFixture({ seed: 18, cards: 4, edges: 2 });
    const card = fixture.cards[0];
    const events: EngineEvent[] = [];
    const renderer = createOverlayTestRenderer(fixture);
    const loadScene = renderer.loadScene.bind(renderer);
    const applyPatchBatch = renderer.applyPatchBatch.bind(renderer);
    let loadSceneCalls = 0;
    const patchBatches: ScenePatch[][] = [];
    renderer.loadScene = (sceneJson) => {
      loadSceneCalls += 1;
      loadScene(sceneJson);
    };
    renderer.applyPatchBatch = (patchesJson) => {
      patchBatches.push(JSON.parse(patchesJson) as ScenePatch[]);
      applyPatchBatch(patchesJson);
    };
    const engine = new ShapeCanvasEngine({
      canvas: testCanvas(),
      overlayRoot: testOverlayRoot(),
      backend: "test",
      webGpuRenderer: renderer,
      onEvent: (event) => events.push(event)
    });

    engine.loadScene(fixture);
    const loadSceneCallsAfterInitialLoad = loadSceneCalls;
    const errors = engine.syncSelection({ kind: "node", id: card.id });

    expect(errors).toEqual([]);
    expect(loadSceneCalls).toBe(loadSceneCallsAfterInitialLoad);
    expect(patchBatches).toEqual([[{ kind: "select", selection: { kind: "node", id: card.id } }]]);
    expect(engine.getSnapshot()?.selection).toEqual({ kind: "node", id: card.id });
    expect(events.filter((event) => event.type === "patch")).toHaveLength(0);

    patchBatches.length = 0;
    engine.syncSelection({ kind: "node", id: card.id });

    expect(patchBatches).toHaveLength(0);
  });

  it("defers full Rust scene reloads while a mouse drag is active", () => {
    const fixture = createBenchmarkFixture({ seed: 19, cards: 4, edges: 2 });
    const canvas = testCanvasWithListeners();
    const renderer = createOverlayTestRenderer(fixture);
    const loadScene = renderer.loadScene.bind(renderer);
    let loadSceneCalls = 0;
    renderer.loadScene = (sceneJson) => {
      loadSceneCalls += 1;
      loadScene(sceneJson);
    };
    const raf = installFakeAnimationFrame();
    try {
      const engine = new ShapeCanvasEngine({
        canvas: canvas.element,
        overlayRoot: testOverlayRoot(),
        backend: "test",
        webGpuRenderer: renderer,
        onEvent() {}
      });
      const nextScene: SceneSnapshot = {
        ...fixture,
        sceneId: "drag-updated-scene",
        camera: { ...fixture.camera, x: fixture.camera.x + 40 }
      };

      engine.loadScene(fixture);
      canvas.dispatch("mousedown", { button: 0, clientX: 120, clientY: 160 });
      engine.loadScene(nextScene);

      expect(loadSceneCalls).toBe(1);

      canvas.dispatch("mouseup", { clientX: 150, clientY: 180 });

      expect(loadSceneCalls).toBe(1);
      expect(raf.pending()).toBe(1);

      raf.flush();

      expect(loadSceneCalls).toBe(2);

      engine.stop();
    } finally {
      raf.restore();
    }
  });

  it("keeps the engine mirror unchanged when Rust rejects a patch batch", () => {
    const fixture = createBenchmarkFixture({ seed: 7, cards: 20, edges: 10 });
    const card = fixture.cards[0];
    const events: EngineEvent[] = [];
    let loadedScene = fixture;
    const renderer: RustWebGpuRenderer = {
      resize() {},
      loadScene(sceneJson) {
        loadedScene = JSON.parse(sceneJson);
      },
      applyPatchBatch() {
        throw new Error("Unknown card id: missing-card");
      },
      renderFrame() {
        return emptyRustFrameStats(loadedScene);
      },
      inputBatch() {
        throw new Error("unused input batch");
      },
      overlayRequest() {
        return null;
      },
      debugSnapshot() {
        return rustDebugSnapshot(loadedScene);
      }
    };
    const engine = new ShapeCanvasEngine({
      canvas: testCanvas(),
      overlayRoot: testOverlayRoot(),
      backend: "test",
      webGpuRenderer: renderer,
      onEvent: (event) => events.push(event)
    });

    engine.loadScene(fixture);
    const errors = engine.applyPatchBatch([
      { kind: "move-card", id: card.id, position: { x: card.bounds.x + 100, y: card.bounds.y + 50 } },
      { kind: "move-card", id: "missing-card", position: { x: 0, y: 0 } }
    ]);

    expect(errors[0]).toContain("Rust patch batch failed");
    expect(engine.getSnapshot()?.cards.find((candidate) => candidate.id === card.id)?.bounds).toEqual(card.bounds);
    expect(events.some((event) => event.type === "patch" && event.errors[0]?.includes("Rust patch batch failed"))).toBe(true);
  });

  it("does not fall back to a TypeScript canvas renderer when WebGPU is unavailable", () => {
    const fixture = createBenchmarkFixture({ seed: 15, cards: 4, edges: 2 });
    const card = fixture.cards[0];
    const events: EngineEvent[] = [];
    const engine = new ShapeCanvasEngine({
      canvas: testCanvas(),
      overlayRoot: testOverlayRoot(),
      backend: "webgpu-wasm-unavailable",
      webGpuRenderer: null,
      onEvent: (event) => events.push(event)
    });

    engine.loadScene(fixture);
    const statsEvent = [...events].reverse().find((event): event is Extract<EngineEvent, { type: "stats" }> => event.type === "stats");
    const errors = engine.applyPatch({ kind: "move-card", id: card.id, position: { x: card.bounds.x + 80, y: card.bounds.y + 40 } });

    expect(events.some((event) => event.type === "status" && event.message.includes("WebGPU renderer unavailable"))).toBe(true);
    expect(statsEvent?.stats.drawBackend).toBe("rust-wgpu-visible");
    expect(statsEvent?.stats.webGpuRendererAvailable).toBe(false);
    expect(statsEvent?.stats.visibleCards).toBe(0);
    expect(errors).toEqual(["WebGPU renderer unavailable"]);
    expect(engine.getSnapshot()?.cards.find((candidate) => candidate.id === card.id)?.bounds).toEqual(card.bounds);
    expect(events.some((event) => event.type === "patch" && event.errors.includes("WebGPU renderer unavailable"))).toBe(true);
  });

  it("exposes Rust last-hit debug data in frame stats for diagnostics", () => {
    const fixture = createBenchmarkFixture({ seed: 16, cards: 4, edges: 2 });
    const card = fixture.cards[0];
    const events: EngineEvent[] = [];
    const renderer = createOverlayTestRenderer(fixture);
    renderer.debugSnapshot = () => ({
      ...rustDebugSnapshot(fixture),
      lastHit: {
        id: card.id,
        kind: "text",
        groupId: card.groupId,
        field: "summary",
        port: null,
        worldX: card.bounds.x + 24,
        worldY: card.bounds.y + 70,
        screenX: 144,
        screenY: 212
      }
    });
    const engine = new ShapeCanvasEngine({
      canvas: testCanvas(),
      overlayRoot: testOverlayRoot(),
      backend: "test",
      webGpuRenderer: renderer,
      onEvent: (event) => events.push(event)
    });

    engine.loadScene(fixture);
    const statsEvent = [...events].reverse().find((event): event is Extract<EngineEvent, { type: "stats" }> => event.type === "stats");

    expect(statsEvent?.stats.rustLastHitKind).toBe("text");
    expect(statsEvent?.stats.rustLastHitId).toBe(card.id);
    expect(statsEvent?.stats.rustLastHitField).toBe("summary");
    expect(statsEvent?.stats.rustLastHitPort).toBeNull();
    expect(statsEvent?.stats.rustLastHitScreenX).toBe(144);
    expect(statsEvent?.stats.rustLastHitScreenY).toBe(212);
  });

  it("removes canvas input listeners when stopped", () => {
    const fixture = createBenchmarkFixture({ seed: 14, cards: 4, edges: 2 });
    const canvas = testCanvasWithListeners();
    const renderer = createOverlayTestRenderer(fixture);
    const inputBatch = renderer.inputBatch.bind(renderer);
    let inputCalls = 0;
    renderer.inputBatch = (eventsJson) => {
      inputCalls += 1;
      return inputBatch(eventsJson);
    };
    const engine = new ShapeCanvasEngine({
      canvas: canvas.element,
      overlayRoot: testOverlayRoot(),
      backend: "test",
      webGpuRenderer: renderer,
      onEvent() {}
    });

    expect(canvas.listenerCount("pointerdown")).toBe(1);
    expect(canvas.listenerCount("mousedown")).toBe(1);
    expect(canvas.listenerCount("wheel")).toBe(1);

    engine.stop();

    expect(canvas.listenerCount("pointerdown")).toBe(0);
    expect(canvas.listenerCount("mousedown")).toBe(0);
    expect(canvas.listenerCount("wheel")).toBe(0);
    canvas.dispatch("pointerdown", { pointerId: 1, clientX: 120, clientY: 160 });
    canvas.dispatch("mousedown", { button: 0, clientX: 120, clientY: 160 });
    canvas.dispatch("wheel", { deltaY: 80, clientX: 120, clientY: 160 });
    expect(inputCalls).toBe(0);
  });

  it("routes mouse drag fallback through the Rust input boundary", () => {
    const fixture = createBenchmarkFixture({ seed: 15, cards: 4, edges: 2 });
    const canvas = testCanvasWithListeners();
    const renderer = createOverlayTestRenderer(fixture);
    const inputEvents: Array<{ kind: string; pointerId: number; screen?: { x: number; y: number } }> = [];
    const inputBatch = renderer.inputBatch.bind(renderer);
    renderer.inputBatch = (eventsJson) => {
      inputEvents.push(...JSON.parse(eventsJson));
      return inputBatch(eventsJson);
    };
    const engine = new ShapeCanvasEngine({
      canvas: canvas.element,
      overlayRoot: testOverlayRoot(),
      backend: "test",
      webGpuRenderer: renderer,
      onEvent() {}
    });

    canvas.dispatch("mousedown", { button: 0, clientX: 120, clientY: 160 });
    expect(canvas.listenerCount("mousemove")).toBe(1);
    expect(canvas.listenerCount("mouseup")).toBe(1);
    canvas.dispatch("mousemove", { clientX: 165, clientY: 190 });
    canvas.dispatch("mouseup", { clientX: 180, clientY: 210 });

    expect(inputEvents.map((event) => event.kind)).toEqual(["pointer-down", "pointer-move", "pointer-up"]);
    expect(inputEvents.map((event) => event.pointerId)).toEqual([-1, -1, -1]);
    expect(inputEvents[0].screen).toEqual({ x: 120, y: 160 });
    expect(inputEvents[1].screen).toEqual({ x: 165, y: 190 });
    expect(inputEvents[2].screen).toEqual({ x: 180, y: 210 });
    expect(canvas.listenerCount("mousemove")).toBe(0);
    expect(canvas.listenerCount("mouseup")).toBe(0);

    engine.stop();
  });

  it("lets mouse fallback own mouse pointer drags", () => {
    const fixture = createBenchmarkFixture({ seed: 16, cards: 4, edges: 2 });
    const canvas = testCanvasWithListeners();
    const renderer = createOverlayTestRenderer(fixture);
    const inputEvents: Array<{ kind: string; pointerId: number }> = [];
    const inputBatch = renderer.inputBatch.bind(renderer);
    renderer.inputBatch = (eventsJson) => {
      inputEvents.push(...JSON.parse(eventsJson));
      return inputBatch(eventsJson);
    };
    const engine = new ShapeCanvasEngine({
      canvas: canvas.element,
      overlayRoot: testOverlayRoot(),
      backend: "test",
      webGpuRenderer: renderer,
      onEvent() {}
    });

    canvas.dispatch("pointerdown", { pointerType: "mouse", pointerId: 7, clientX: 120, clientY: 160 });
    canvas.dispatch("pointermove", { pointerType: "mouse", pointerId: 7, clientX: 140, clientY: 180 });
    canvas.dispatch("mousedown", { button: 0, clientX: 120, clientY: 160 });
    canvas.dispatch("mousemove", { clientX: 140, clientY: 180 });
    canvas.dispatch("mouseup", { clientX: 160, clientY: 200 });

    expect(inputEvents.map((event) => `${event.kind}:${event.pointerId}`)).toEqual(["pointer-down:-1", "pointer-move:-1", "pointer-up:-1"]);

    engine.stop();
  });
});

function testCanvas(): HTMLCanvasElement {
  return testCanvasWithListeners().element;
}

function testCanvasWithListeners(): {
  element: HTMLCanvasElement;
  listenerCount: (type: string) => number;
  dispatch: (type: string, event?: Record<string, unknown>) => void;
} {
  const listeners = new Map<string, Set<EventListener>>();
  const element = {
    width: 0,
    height: 0,
    addEventListener(type: string, listener: EventListenerOrEventListenerObject) {
      if (typeof listener !== "function") return;
      const typeListeners = listeners.get(type) ?? new Set();
      typeListeners.add(listener);
      listeners.set(type, typeListeners);
    },
    removeEventListener(type: string, listener: EventListenerOrEventListenerObject) {
      if (typeof listener !== "function") return;
      listeners.get(type)?.delete(listener);
    },
    setPointerCapture() {},
    releasePointerCapture() {},
    getBoundingClientRect() {
      return { left: 0, top: 0, width: 800, height: 600 };
    }
  };
  return {
    element: element as unknown as HTMLCanvasElement,
    listenerCount(type: string) {
      return listeners.get(type)?.size ?? 0;
    },
    dispatch(type: string, event: Record<string, unknown> = {}) {
      const dispatched = {
        clientX: 0,
        clientY: 0,
        pointerId: 1,
        preventDefault() {},
        ...event
      };
      for (const listener of listeners.get(type) ?? []) listener(dispatched as unknown as Event);
    }
  };
}

function testOverlayRoot(appended: HTMLElement[] = []): HTMLElement {
  return {
    append(element: HTMLElement) {
      appended.push(element);
    }
  } as unknown as HTMLElement;
}

function installFakeAnimationFrame(): { flush: () => void; pending: () => number; restore: () => void } {
  const callbacks: FrameRequestCallback[] = [];
  const previousRequest = Object.getOwnPropertyDescriptor(globalThis, "requestAnimationFrame");
  const previousCancel = Object.getOwnPropertyDescriptor(globalThis, "cancelAnimationFrame");
  Object.defineProperty(globalThis, "requestAnimationFrame", {
    configurable: true,
    value: (callback: FrameRequestCallback) => {
      callbacks.push(callback);
      return callbacks.length;
    }
  });
  Object.defineProperty(globalThis, "cancelAnimationFrame", {
    configurable: true,
    value: () => {}
  });
  return {
    flush() {
      const pending = callbacks.splice(0);
      for (const callback of pending) callback(performance.now());
    },
    pending() {
      return callbacks.length;
    },
    restore() {
      if (previousRequest) {
        Object.defineProperty(globalThis, "requestAnimationFrame", previousRequest);
      } else {
        Reflect.deleteProperty(globalThis, "requestAnimationFrame");
      }
      if (previousCancel) {
        Object.defineProperty(globalThis, "cancelAnimationFrame", previousCancel);
      } else {
        Reflect.deleteProperty(globalThis, "cancelAnimationFrame");
      }
    }
  };
}

function installFakeTextAreaDocument(): { created: FakeTextArea[]; restore: () => void } {
  const created: FakeTextArea[] = [];
  const previous = Object.getOwnPropertyDescriptor(globalThis, "document");
  Object.defineProperty(globalThis, "document", {
    configurable: true,
    value: {
      createElement(tagName: string) {
        if (tagName !== "textarea") throw new Error(`Unexpected fake element request: ${tagName}`);
        const textarea = new FakeTextArea();
        created.push(textarea);
        return textarea as unknown as HTMLTextAreaElement;
      }
    }
  });
  return {
    created,
    restore() {
      if (previous) {
        Object.defineProperty(globalThis, "document", previous);
      } else {
        Reflect.deleteProperty(globalThis, "document");
      }
    }
  };
}

class FakeTextArea {
  className = "";
  value = "";
  autocomplete = "";
  spellcheck = false;
  focused = false;
  selected = false;
  removed = false;
  dataset: Record<string, string> = {};
  style = fakeStyle();
  private listeners = new Map<string, Array<(event: FakeTextAreaEvent) => void>>();

  addEventListener(type: string, listener: (event: FakeTextAreaEvent) => void) {
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), listener]);
  }

  focus() {
    this.focused = true;
  }

  select() {
    this.selected = true;
  }

  remove() {
    if (this.removed) return;
    const wasFocused = this.focused;
    this.removed = true;
    if (wasFocused) {
      this.focused = false;
      this.dispatch("blur");
    }
  }

  dispatch(type: string, event: Record<string, unknown> = {}): FakeTextAreaEvent {
    if (type === "blur") this.focused = false;
    const dispatched = fakeTextAreaEvent(event);
    for (const listener of this.listeners.get(type) ?? []) listener(dispatched);
    return dispatched;
  }
}

type FakeTextAreaEvent = Record<string, unknown> & {
  defaultPrevented: boolean;
  preventDefault: () => void;
};

function fakeTextAreaEvent(event: Record<string, unknown>): FakeTextAreaEvent {
  const dispatched = {
    ...event,
    defaultPrevented: false,
    preventDefault: () => {
      dispatched.defaultPrevented = true;
    }
  };
  return dispatched;
}

function fakeStyle(): Record<string, string> & {
  setProperty: (name: string, value: string) => void;
  getPropertyValue: (name: string) => string;
} {
  const properties = new Map<string, string>();
  const style = {} as Record<string, string> & {
    setProperty: (name: string, value: string) => void;
    getPropertyValue: (name: string) => string;
  };
  style.setProperty = (name, value) => {
    properties.set(name, value);
  };
  style.getPropertyValue = (name) => properties.get(name) ?? "";
  return style;
}

function createOverlayTestRenderer(initialScene: SceneSnapshot): RustWebGpuRenderer {
  let loadedScene = initialScene;
  return {
    resize() {},
    loadScene(sceneJson) {
      loadedScene = JSON.parse(sceneJson);
    },
    applyPatchBatch(patchesJson) {
      const patches = JSON.parse(patchesJson) as ScenePatch[];
      for (const patch of patches) loadedScene = applyScenePatch(loadedScene, patch);
    },
    renderFrame() {
      return emptyRustFrameStats(loadedScene);
    },
    inputBatch(eventsJson) {
      const events = JSON.parse(eventsJson) as Array<{ kind: string; camera?: CameraState; bounds?: WorldRect; screen?: { x: number; y: number }; zoom?: number }>;
      for (const event of events) {
        if (event.kind === "set-camera" && event.camera) loadedScene = { ...loadedScene, camera: event.camera };
        if (event.kind === "focus-bounds" && event.bounds && event.screen && typeof event.zoom === "number") {
          loadedScene = {
            ...loadedScene,
            camera: {
              zoom: event.zoom,
              x: event.screen.x - (event.bounds.x + event.bounds.width / 2) * event.zoom,
              y: event.screen.y - (event.bounds.y + event.bounds.height / 2) * event.zoom
            }
          };
        }
      }
      return {
        camera: loadedScene.camera,
        hit: null,
        selection: loadedScene.selection,
        patches: [],
        overlay: null
      };
    },
    overlayRequest(cardId, field) {
      return testOverlayRequest(loadedScene, cardId, field);
    },
    debugSnapshot() {
      return rustDebugSnapshot(loadedScene);
    }
  };
}

function testOverlayRequest(scene: SceneSnapshot, cardId: string, field: string): DomOverlayRequest | null {
  if (field !== "title" && field !== "summary" && field !== "detail") return null;
  const card = scene.cards.find((candidate) => candidate.id === cardId);
  if (!card) return null;
  const textRect = testTextRect(card.bounds, field);
  const worldRect = expandRect(textRect, 9, 9 * 0.67, 1);
  return {
    target: { kind: "card-text", id: card.id, field },
    value: card[field],
    worldRect,
    screenRect: worldRectToScreenRect(worldRect, scene.camera),
    style: testOverlayStyle(scene.camera, field)
  };
}

function testTextRect(card: WorldRect, field: "title" | "summary" | "detail"): WorldRect {
  if (field === "title") return { x: card.x + 14, y: card.y + 34, width: card.width - 28, height: 22.42 };
  if (field === "summary") return { x: card.x + 14, y: card.y + 52, width: card.width - 28, height: 56.94 };
  return { x: card.x + 14, y: card.y + 52, width: card.width - 28, height: Math.max(24, card.height - 66) };
}

function expandRect(rect: WorldRect, paddingX: number, paddingY: number, borderWidth: number): WorldRect {
  const xInset = paddingX + borderWidth;
  const yInset = paddingY + borderWidth;
  return {
    x: rect.x - xInset,
    y: rect.y - yInset,
    width: rect.width + xInset * 2,
    height: rect.height + yInset * 2
  };
}

function worldRectToScreenRect(rect: WorldRect, camera: CameraState): WorldRect {
  return {
    x: rect.x * camera.zoom + camera.x,
    y: rect.y * camera.zoom + camera.y,
    width: rect.width * camera.zoom,
    height: rect.height * camera.zoom
  };
}

function testOverlayStyle(camera: CameraState, field: "title" | "summary" | "detail"): DomOverlayRequest["style"] {
  const fontSize = field === "title" ? 19 : 13;
  return {
    fontFamily: "\"Noto Sans KR\", Inter, ui-sans-serif",
    fontSize: fontSize * camera.zoom,
    fontWeight: 400,
    lineHeight: fontSize * (field === "title" ? 1.18 : 1.46) * camera.zoom,
    letterSpacing: 0,
    paddingX: 9 * camera.zoom,
    paddingY: 9 * 0.67 * camera.zoom,
    textColor: field === "title" ? "rgba(16, 32, 51, 0.920)" : "rgba(90, 113, 136, 0.840)",
    backgroundColor: "rgba(255, 255, 255, 0.980)",
    borderColor: "rgba(47, 126, 230, 0.520)",
    borderWidth: 1 * camera.zoom,
    borderRadius: 7 * camera.zoom,
    focusRingColor: "rgba(47, 126, 230, 0.120)",
    focusRingWidth: 4 * camera.zoom,
    boxShadow: `0.00px 0.00px 0.00px ${4 * camera.zoom}px rgba(47, 126, 230, 0.120)`,
    caretColor: "rgba(47, 126, 230, 0.920)",
    accentColor: "rgba(47, 126, 230, 0.920)",
    selectionBackgroundColor: "rgba(47, 126, 230, 0.200)",
    maxLines: field === "title" ? 1 : field === "summary" ? 3 : 6,
    overflowX: "hidden",
    overflowY: field === "title" ? "hidden" : "auto",
    state: "selected"
  };
}

function emptyRustFrameStats(scene: SceneSnapshot): RustWebGpuFrameStats {
  return {
    totalGroups: scene.groups.length,
    totalCards: scene.cards.length,
    totalEdges: scene.edges.length,
    visibleGroupCount: 0,
    visibleCardCount: 0,
    visibleEdgeCount: 0,
    vertexCount: 0,
    drawnVertexCount: 0,
    drawRangeCount: 0,
    textGlyphCount: 0,
    fallbackTextGlyphCount: 0,
    cjkTextGlyphCount: 0,
    fontFallbackRunCount: 0,
    missingTextGlyphCount: 0,
    textAtlasOverflowGlyphCount: 0,
    textMissingRasterGlyphCount: 0,
    textAtlasGlyphCount: 0,
    textRasterCacheHits: 0,
    textRasterCacheMisses: 0,
    textLayoutCacheHits: 0,
    textLayoutCacheMisses: 0,
    styleTokenCount: scene.styles.length,
    patchUpdateCount: 0,
    dirtyRangeWriteCount: 0,
    fullBufferRebuildCount: 0,
    vertexTruncationCount: 0,
    truncatedVertexCount: 0,
    edgeCapacityGrowCount: 0,
    edgeCompactionCount: 0,
    edgeSlotCount: 0,
    edgeSlotFreeCount: 0,
    cardCapacityGrowCount: 0,
    cardCompactionCount: 0,
    cardSlotCount: 0,
    cardSlotFreeCount: 0,
    groupCapacityGrowCount: 0,
    groupCompactionCount: 0,
    groupSlotCount: 0,
    groupSlotFreeCount: 0,
    backend: "test"
  };
}

function rustDebugSnapshot(scene: SceneSnapshot): RustDebugSnapshot {
  return {
    camera: scene.camera,
    selection: scene.selection,
    selectionWorldRect: null,
    selectionScreenRect: null,
    lastHit: null,
    totalGroups: scene.groups.length,
    totalCards: scene.cards.length,
    totalEdges: scene.edges.length,
    patchUpdateCount: 0,
    dirtyRangeWriteCount: 0,
    fullBufferRebuildCount: 0
  };
}
