/**
 * T2.4 — Grouping And Labeling (op-level)
 *
 * Tests for: group-objects, ungroup, set-object-group, set-object-tags, create-tag.
 * Verifies:
 *   - Membership changes are predictable (correct groupId after ops)
 *   - Operations are reversible (ungroup restores pre-group state identities)
 *   - Object identity is preserved across group/ungroup/move
 *   - Label/tag changes produce normal operation events (sceneVersion bumps)
 *   - tagIds on nodes and edges are additive and default to []
 */

import { describe, expect, it } from "vitest";
import { applyRenderPatchToShapeScene } from "../src/shared/renderPatch";
import type { Scene, SceneEdge, SceneNode } from "../src/shared/schema";

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

const NOW = "2026-06-05T00:00:00.000Z";

const tag1 = {
  id: "tag-1",
  name: "Important",
  color: "#ff0000",
  description: "",
  createdAt: NOW,
  updatedAt: NOW
};

const tag2 = {
  id: "tag-2",
  name: "Review",
  color: "#00ff00",
  description: "",
  createdAt: NOW,
  updatedAt: NOW
};

function makeNode(id: string, groupId: string, x = 100, y = 100): SceneNode {
  return {
    id,
    groupId,
    type: "task",
    title: `Node ${id}`,
    summary: "",
    detail: "",
    status: "draft",
    confidence: 0.5,
    evidenceRefs: [],
    childDecisionIds: [],
    tagIds: [],
    position: { x, y },
    size: { width: 200, height: 100 },
    zIndex: 0,
    updatedAt: NOW
  };
}

function makeEdge(id: string, groupId: string, source: string, target: string): SceneEdge {
  return {
    id,
    groupId,
    type: "supports",
    source,
    target,
    label: "supports",
    rationale: "",
    confidence: 0.5,
    tagIds: [],
    updatedAt: NOW
  };
}

const baseScene: Scene = {
  version: 1,
  sceneVersion: 1,
  groups: [
    {
      id: "root",
      parentGroupId: null,
      title: "Root Frame",
      summary: "",
      bounds: { x: 0, y: 0, width: 3000, height: 3000 },
      tagIds: [],
      zIndex: 0,
      collapsed: false,
      createdAt: NOW,
      updatedAt: NOW
    }
  ],
  nodes: [
    makeNode("n1", "root", 100, 100),
    makeNode("n2", "root", 400, 100),
    makeNode("n3", "root", 700, 100)
  ],
  edges: [makeEdge("e1", "root", "n1", "n2")],
  tags: [tag1, tag2],
  comments: [],
  artifacts: [],
  selection: { kind: "canvas" },
  updatedAt: NOW
};

// ---------------------------------------------------------------------------
// group-objects
// ---------------------------------------------------------------------------

describe("group-objects", () => {
  it("creates a new frame and re-points member nodes to it", () => {
    const { scene, errors } = applyRenderPatchToShapeScene(
      baseScene,
      { kind: "group-objects", ids: ["n1", "n2"], frameId: "g-new", title: "My Group" },
      NOW
    );
    expect(errors).toHaveLength(0);
    expect(scene.groups.some((g) => g.id === "g-new")).toBe(true);
    expect(scene.nodes.find((n) => n.id === "n1")?.groupId).toBe("g-new");
    expect(scene.nodes.find((n) => n.id === "n2")?.groupId).toBe("g-new");
    // n3 is untouched
    expect(scene.nodes.find((n) => n.id === "n3")?.groupId).toBe("root");
  });

  it("preserves node ids, geometry, and text across group-objects", () => {
    const { scene } = applyRenderPatchToShapeScene(
      baseScene,
      { kind: "group-objects", ids: ["n1", "n2"], frameId: "g-new" },
      NOW
    );
    const n1 = scene.nodes.find((n) => n.id === "n1")!;
    expect(n1.id).toBe("n1");
    expect(n1.position).toEqual(baseScene.nodes[0].position);
    expect(n1.title).toBe(baseScene.nodes[0].title);
  });

  it("selects the new group after creation", () => {
    const { scene } = applyRenderPatchToShapeScene(
      baseScene,
      { kind: "group-objects", ids: ["n1"], frameId: "g-new" },
      NOW
    );
    expect(scene.selection).toEqual({ kind: "group", id: "g-new" });
  });

  it("computes bounds from member positions when not supplied", () => {
    const { scene } = applyRenderPatchToShapeScene(
      baseScene,
      { kind: "group-objects", ids: ["n1", "n2"], frameId: "g-new" },
      NOW
    );
    const newGroup = scene.groups.find((g) => g.id === "g-new")!;
    // n1 at (100,100) size (200,100), n2 at (400,100) size (200,100)
    // union = (100,100)→(600,200), padded by 40 => x=60,y=60,w=580,h=180
    expect(newGroup.bounds.x).toBe(60);
    expect(newGroup.bounds.y).toBe(60);
    expect(newGroup.bounds.width).toBe(580);
    expect(newGroup.bounds.height).toBe(180);
  });

  it("uses supplied bounds when provided", () => {
    const suppliedBounds = { x: 10, y: 10, width: 500, height: 300 };
    const { scene } = applyRenderPatchToShapeScene(
      baseScene,
      { kind: "group-objects", ids: ["n1"], frameId: "g-new", bounds: suppliedBounds },
      NOW
    );
    expect(scene.groups.find((g) => g.id === "g-new")?.bounds).toEqual(suppliedBounds);
  });

  it("sets parentGroupId on the new group when supplied", () => {
    const { scene } = applyRenderPatchToShapeScene(
      baseScene,
      { kind: "group-objects", ids: ["n1"], frameId: "g-child", parentGroupId: "root" },
      NOW
    );
    expect(scene.groups.find((g) => g.id === "g-child")?.parentGroupId).toBe("root");
  });

  it("bumps sceneVersion (normal operation event)", () => {
    const { scene } = applyRenderPatchToShapeScene(
      baseScene,
      { kind: "group-objects", ids: ["n1"], frameId: "g-new" },
      NOW
    );
    expect(scene.sceneVersion).toBe(baseScene.sceneVersion + 1);
  });

  it("rejects duplicate frameId", () => {
    const { errors } = applyRenderPatchToShapeScene(
      baseScene,
      { kind: "group-objects", ids: ["n1"], frameId: "root" },
      NOW
    );
    expect(errors).toContain("Duplicate group id: root");
  });

  it("rejects unknown member ids", () => {
    const { errors } = applyRenderPatchToShapeScene(
      baseScene,
      { kind: "group-objects", ids: ["does-not-exist"], frameId: "g-new" },
      NOW
    );
    expect(errors.some((e) => e.includes("Unknown id"))).toBe(true);
  });

  it("rejects unknown parentGroupId", () => {
    const { errors } = applyRenderPatchToShapeScene(
      baseScene,
      { kind: "group-objects", ids: ["n1"], frameId: "g-new", parentGroupId: "no-such-group" },
      NOW
    );
    expect(errors.some((e) => e.includes("Unknown parentGroupId"))).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// ungroup
// ---------------------------------------------------------------------------

describe("ungroup", () => {
  // Build a scene with a child group containing n1 and n2
  function makeGroupedScene() {
    const { scene: s1 } = applyRenderPatchToShapeScene(
      baseScene,
      { kind: "group-objects", ids: ["n1", "n2"], frameId: "child-group", title: "Child" },
      NOW
    );
    return s1;
  }

  it("removes the frame but preserves member nodes", () => {
    const grouped = makeGroupedScene();
    const { scene, errors } = applyRenderPatchToShapeScene(
      grouped,
      { kind: "ungroup", id: "child-group" },
      NOW
    );
    expect(errors).toHaveLength(0);
    expect(scene.groups.some((g) => g.id === "child-group")).toBe(false);
    expect(scene.nodes.some((n) => n.id === "n1")).toBe(true);
    expect(scene.nodes.some((n) => n.id === "n2")).toBe(true);
  });

  it("re-parents member nodes to the frame's parentGroupId", () => {
    // child-group has parentGroupId=null so members fall back to "root"
    const grouped = makeGroupedScene();
    const { scene } = applyRenderPatchToShapeScene(
      grouped,
      { kind: "ungroup", id: "child-group" },
      NOW
    );
    // child-group.parentGroupId was null, fallback = first available group = root
    const n1 = scene.nodes.find((n) => n.id === "n1")!;
    expect(n1.groupId).toBe("root");
  });

  it("preserves member geometry (position, size, title) exactly", () => {
    const grouped = makeGroupedScene();
    const { scene } = applyRenderPatchToShapeScene(
      grouped,
      { kind: "ungroup", id: "child-group" },
      NOW
    );
    const n1 = scene.nodes.find((n) => n.id === "n1")!;
    expect(n1.position).toEqual(baseScene.nodes[0].position);
    expect(n1.size).toEqual(baseScene.nodes[0].size);
    expect(n1.title).toBe(baseScene.nodes[0].title);
  });

  it("preserves member tagIds across ungroup", () => {
    // Tag n1, then group and ungroup
    const { scene: tagged } = applyRenderPatchToShapeScene(
      baseScene,
      { kind: "set-object-tags", targetKind: "card", id: "n1", tagIds: ["tag-1"] },
      NOW
    );
    const { scene: grouped } = applyRenderPatchToShapeScene(
      tagged,
      { kind: "group-objects", ids: ["n1"], frameId: "g-temp" },
      NOW
    );
    const { scene: ungrouped } = applyRenderPatchToShapeScene(
      grouped,
      { kind: "ungroup", id: "g-temp" },
      NOW
    );
    expect(ungrouped.nodes.find((n) => n.id === "n1")?.tagIds).toEqual(["tag-1"]);
  });

  it("is reversible: group → ungroup restores original membership", () => {
    const original = baseScene;
    const { scene: grouped } = applyRenderPatchToShapeScene(
      original,
      { kind: "group-objects", ids: ["n1", "n2"], frameId: "g-temp" },
      NOW
    );
    const { scene: ungrouped } = applyRenderPatchToShapeScene(
      grouped,
      { kind: "ungroup", id: "g-temp" },
      NOW
    );
    // Both nodes back in root
    expect(ungrouped.nodes.find((n) => n.id === "n1")?.groupId).toBe("root");
    expect(ungrouped.nodes.find((n) => n.id === "n2")?.groupId).toBe("root");
    // g-temp removed; original groups intact
    expect(ungrouped.groups.some((g) => g.id === "g-temp")).toBe(false);
    expect(ungrouped.groups.some((g) => g.id === "root")).toBe(true);
  });

  it("bumps sceneVersion", () => {
    const grouped = makeGroupedScene();
    const before = grouped.sceneVersion;
    const { scene } = applyRenderPatchToShapeScene(grouped, { kind: "ungroup", id: "child-group" }, NOW);
    expect(scene.sceneVersion).toBe(before + 1);
  });

  it("rejects unknown group id", () => {
    const { errors } = applyRenderPatchToShapeScene(baseScene, { kind: "ungroup", id: "no-such" }, NOW);
    expect(errors.some((e) => e.includes("Unknown group id"))).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// set-object-group
// ---------------------------------------------------------------------------

describe("set-object-group", () => {
  // Scene with a child group
  function makeSceneWithChildGroup() {
    return applyRenderPatchToShapeScene(
      baseScene,
      { kind: "group-objects", ids: ["n1"], frameId: "child", title: "Child" },
      NOW
    ).scene;
  }

  it("moves a node to a different frame", () => {
    const s = makeSceneWithChildGroup();
    // n2 is in root; move it to child
    const { scene, errors } = applyRenderPatchToShapeScene(
      s,
      { kind: "set-object-group", ids: ["n2"], frameId: "child" },
      NOW
    );
    expect(errors).toHaveLength(0);
    expect(scene.nodes.find((n) => n.id === "n2")?.groupId).toBe("child");
  });

  it("preserves node identity, geometry, and tags when moving between groups", () => {
    const s = makeSceneWithChildGroup();
    const { scene } = applyRenderPatchToShapeScene(
      s,
      { kind: "set-object-group", ids: ["n2"], frameId: "child" },
      NOW
    );
    const n2 = scene.nodes.find((n) => n.id === "n2")!;
    expect(n2.id).toBe("n2");
    expect(n2.position).toEqual(baseScene.nodes[1].position);
    expect(n2.tagIds).toEqual([]);
  });

  it("can move multiple nodes in one op", () => {
    const s = makeSceneWithChildGroup();
    const { scene } = applyRenderPatchToShapeScene(
      s,
      { kind: "set-object-group", ids: ["n2", "n3"], frameId: "child" },
      NOW
    );
    expect(scene.nodes.find((n) => n.id === "n2")?.groupId).toBe("child");
    expect(scene.nodes.find((n) => n.id === "n3")?.groupId).toBe("child");
  });

  it("moves an edge to a different frame", () => {
    const s = makeSceneWithChildGroup();
    // e1 is in root; move it to child
    const { scene, errors } = applyRenderPatchToShapeScene(
      s,
      { kind: "set-object-group", ids: ["e1"], frameId: "child" },
      NOW
    );
    expect(errors).toHaveLength(0);
    expect(scene.edges.find((e) => e.id === "e1")?.groupId).toBe("child");
  });

  it("bumps sceneVersion", () => {
    const s = makeSceneWithChildGroup();
    const { scene } = applyRenderPatchToShapeScene(
      s,
      { kind: "set-object-group", ids: ["n2"], frameId: "child" },
      NOW
    );
    expect(scene.sceneVersion).toBe(s.sceneVersion + 1);
  });

  it("rejects unknown target frame", () => {
    const { errors } = applyRenderPatchToShapeScene(
      baseScene,
      { kind: "set-object-group", ids: ["n1"], frameId: "no-such-frame" },
      NOW
    );
    expect(errors.some((e) => e.includes("Unknown group id"))).toBe(true);
  });

  it("rejects unknown member ids", () => {
    const { errors } = applyRenderPatchToShapeScene(
      baseScene,
      { kind: "set-object-group", ids: ["ghost"], frameId: "root" },
      NOW
    );
    expect(errors.some((e) => e.includes("Unknown id"))).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// set-object-tags
// ---------------------------------------------------------------------------

describe("set-object-tags", () => {
  it("applies tags to a card (node)", () => {
    const { scene, errors } = applyRenderPatchToShapeScene(
      baseScene,
      { kind: "set-object-tags", targetKind: "card", id: "n1", tagIds: ["tag-1", "tag-2"] },
      NOW
    );
    expect(errors).toHaveLength(0);
    expect(scene.nodes.find((n) => n.id === "n1")?.tagIds).toEqual(["tag-1", "tag-2"]);
  });

  it("applies tags to an edge", () => {
    const { scene, errors } = applyRenderPatchToShapeScene(
      baseScene,
      { kind: "set-object-tags", targetKind: "edge", id: "e1", tagIds: ["tag-1"] },
      NOW
    );
    expect(errors).toHaveLength(0);
    expect(scene.edges.find((e) => e.id === "e1")?.tagIds).toEqual(["tag-1"]);
  });

  it("applies tags to a frame (group)", () => {
    const { scene, errors } = applyRenderPatchToShapeScene(
      baseScene,
      { kind: "set-object-tags", targetKind: "frame", id: "root", tagIds: ["tag-2"] },
      NOW
    );
    expect(errors).toHaveLength(0);
    expect(scene.groups.find((g) => g.id === "root")?.tagIds).toEqual(["tag-2"]);
  });

  it("clears tags when tagIds is empty", () => {
    // First set tags, then clear
    const { scene: withTags } = applyRenderPatchToShapeScene(
      baseScene,
      { kind: "set-object-tags", targetKind: "card", id: "n1", tagIds: ["tag-1"] },
      NOW
    );
    const { scene: cleared } = applyRenderPatchToShapeScene(
      withTags,
      { kind: "set-object-tags", targetKind: "card", id: "n1", tagIds: [] },
      NOW
    );
    expect(cleared.nodes.find((n) => n.id === "n1")?.tagIds).toEqual([]);
  });

  it("is a normal operation event (bumps sceneVersion)", () => {
    const { scene } = applyRenderPatchToShapeScene(
      baseScene,
      { kind: "set-object-tags", targetKind: "card", id: "n1", tagIds: ["tag-1"] },
      NOW
    );
    expect(scene.sceneVersion).toBe(baseScene.sceneVersion + 1);
  });

  it("rejects unknown tag ids", () => {
    const { errors } = applyRenderPatchToShapeScene(
      baseScene,
      { kind: "set-object-tags", targetKind: "card", id: "n1", tagIds: ["tag-ghost"] },
      NOW
    );
    expect(errors.some((e) => e.includes("Unknown tag id"))).toBe(true);
  });

  it("rejects unknown node id for card target", () => {
    const { errors } = applyRenderPatchToShapeScene(
      baseScene,
      { kind: "set-object-tags", targetKind: "card", id: "ghost", tagIds: [] },
      NOW
    );
    expect(errors.some((e) => e.includes("Unknown node id"))).toBe(true);
  });

  it("rejects unknown edge id for edge target", () => {
    const { errors } = applyRenderPatchToShapeScene(
      baseScene,
      { kind: "set-object-tags", targetKind: "edge", id: "ghost-edge", tagIds: [] },
      NOW
    );
    expect(errors.some((e) => e.includes("Unknown edge id"))).toBe(true);
  });

  it("rejects unknown group id for frame target", () => {
    const { errors } = applyRenderPatchToShapeScene(
      baseScene,
      { kind: "set-object-tags", targetKind: "frame", id: "ghost-group", tagIds: [] },
      NOW
    );
    expect(errors.some((e) => e.includes("Unknown group id"))).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// create-tag
// ---------------------------------------------------------------------------

describe("create-tag", () => {
  const newTag = {
    id: "tag-new",
    name: "Fresh Tag",
    color: "#0000ff",
    description: "A brand new tag",
    createdAt: NOW,
    updatedAt: NOW
  };

  it("adds a tag to the scene registry", () => {
    const { scene, errors } = applyRenderPatchToShapeScene(
      baseScene,
      { kind: "create-tag", tag: newTag },
      NOW
    );
    expect(errors).toHaveLength(0);
    expect(scene.tags.some((t) => t.id === "tag-new")).toBe(true);
  });

  it("is a normal operation event (bumps sceneVersion)", () => {
    const { scene } = applyRenderPatchToShapeScene(
      baseScene,
      { kind: "create-tag", tag: newTag },
      NOW
    );
    expect(scene.sceneVersion).toBe(baseScene.sceneVersion + 1);
  });

  it("allows the new tag to be applied to objects after creation", () => {
    const { scene: withTag } = applyRenderPatchToShapeScene(
      baseScene,
      { kind: "create-tag", tag: newTag },
      NOW
    );
    const { scene, errors } = applyRenderPatchToShapeScene(
      withTag,
      { kind: "set-object-tags", targetKind: "card", id: "n1", tagIds: ["tag-new"] },
      NOW
    );
    expect(errors).toHaveLength(0);
    expect(scene.nodes.find((n) => n.id === "n1")?.tagIds).toEqual(["tag-new"]);
  });

  it("rejects duplicate tag ids", () => {
    const { errors } = applyRenderPatchToShapeScene(
      baseScene,
      { kind: "create-tag", tag: { ...newTag, id: "tag-1" } },
      NOW
    );
    expect(errors.some((e) => e.includes("Duplicate tag id"))).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// tagIds on SceneNode and SceneEdge defaults to []
// ---------------------------------------------------------------------------

describe("tagIds schema defaults", () => {
  it("new nodes from create-card have tagIds defaulting to []", () => {
    const { scene } = applyRenderPatchToShapeScene(
      baseScene,
      {
        kind: "create-card",
        card: {
          id: "n-new",
          groupId: "root",
          type: "task",
          title: "New card",
          summary: "",
          detail: "",
          status: "draft",
          bounds: { x: 100, y: 100, width: 200, height: 100 },
          zIndex: 0,
          styleKey: "default",
          accessibilityLabel: "New card"
        }
      },
      NOW
    );
    expect(scene.nodes.find((n) => n.id === "n-new")?.tagIds).toEqual([]);
  });

  it("new edges from create-edge have tagIds defaulting to []", () => {
    const { scene } = applyRenderPatchToShapeScene(
      baseScene,
      {
        kind: "create-edge",
        groupId: "root",
        source: "n1",
        target: "n3",
        edgeId: "e-new"
      },
      NOW
    );
    expect(scene.edges.find((e) => e.id === "e-new")?.tagIds).toEqual([]);
  });
});

// ---------------------------------------------------------------------------
// Nested group operations
// ---------------------------------------------------------------------------

describe("nested groups", () => {
  it("group-objects with parentGroupId creates a nested frame", () => {
    // Create child frame inside root
    const { scene } = applyRenderPatchToShapeScene(
      baseScene,
      {
        kind: "group-objects",
        ids: ["n1", "n2"],
        frameId: "inner",
        parentGroupId: "root",
        title: "Inner Group"
      },
      NOW
    );
    const inner = scene.groups.find((g) => g.id === "inner")!;
    expect(inner.parentGroupId).toBe("root");
    // Members belong to inner
    expect(scene.nodes.find((n) => n.id === "n1")?.groupId).toBe("inner");
  });

  it("ungroup of a nested frame re-parents members to the parent frame", () => {
    // Build: root → inner → n1, n2
    const { scene: withInner } = applyRenderPatchToShapeScene(
      baseScene,
      {
        kind: "group-objects",
        ids: ["n1", "n2"],
        frameId: "inner",
        parentGroupId: "root",
        title: "Inner"
      },
      NOW
    );
    const { scene: ungrouped } = applyRenderPatchToShapeScene(
      withInner,
      { kind: "ungroup", id: "inner" },
      NOW
    );
    // inner's parentGroupId = root, so members go to root
    expect(ungrouped.nodes.find((n) => n.id === "n1")?.groupId).toBe("root");
    expect(ungrouped.nodes.find((n) => n.id === "n2")?.groupId).toBe("root");
    // inner is gone, root remains
    expect(ungrouped.groups.some((g) => g.id === "inner")).toBe(false);
    expect(ungrouped.groups.some((g) => g.id === "root")).toBe(true);
  });

  it("move-between nested groups: set-object-group from inner to root", () => {
    const { scene: withInner } = applyRenderPatchToShapeScene(
      baseScene,
      {
        kind: "group-objects",
        ids: ["n1"],
        frameId: "inner",
        parentGroupId: "root"
      },
      NOW
    );
    // Move n1 from inner back to root
    const { scene, errors } = applyRenderPatchToShapeScene(
      withInner,
      { kind: "set-object-group", ids: ["n1"], frameId: "root" },
      NOW
    );
    expect(errors).toHaveLength(0);
    expect(scene.nodes.find((n) => n.id === "n1")?.groupId).toBe("root");
  });
});
