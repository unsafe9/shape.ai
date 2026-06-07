/**
 * Golden-vector generator for the scene-core op-apply port (MG0.2a).
 *
 * Imports the canonical TS implementation and emits (scene, patch, now) ->
 * (expected scene, errors) tuples. The Rust port in crates/scene-core runs the
 * same inputs and asserts equivalence (numbers normalized to f64; comment ids
 * masked where non-deterministic).
 *
 * Run: tsx crates/scene-core/tests/golden/generate.ts
 */
import {
  applyRenderPatchToShapeScene,
  updateShapeSceneGroupTags,
  addShapeSceneComment,
  type RenderScenePatch,
} from "../../../../src/shared/renderPatch";
import { sceneSchema, type Scene, type SceneSelection } from "../../../../src/shared/schema";
import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const NOW = "2026-06-07T12:00:00.000Z";

type Case =
  | { name: string; fn: "apply"; now: string; scene: Scene; patch: RenderScenePatch; expected: { scene: Scene; errors: string[] } }
  | { name: string; fn: "updateGroupTags"; now: string; scene: Scene; groupId: string; tagIds: string[]; expected: { scene: Scene; errors: string[] } }
  | { name: string; fn: "addComment"; now: string; scene: Scene; target: SceneSelection; body: string; maskCommentIds: true; expected: { scene: Scene; errors: string[] } };

function baseScene(): Scene {
  return sceneSchema.parse({
    version: 1,
    sceneVersion: 7,
    updatedAt: "2026-01-01T00:00:00.000Z",
    groups: [
      { id: "g1", parentGroupId: null, title: "Group 1", bounds: { x: 0, y: 0, width: 800, height: 600 }, zIndex: 1, createdAt: "2026-01-01T00:00:00.000Z", updatedAt: "2026-01-01T00:00:00.000Z" },
      { id: "g2", parentGroupId: "g1", title: "Group 2 (child)", bounds: { x: 50, y: 50, width: 300, height: 300 }, zIndex: 2, createdAt: "2026-01-01T00:00:00.000Z", updatedAt: "2026-01-01T00:00:00.000Z" },
      { id: "g3", parentGroupId: null, title: "Group 3", bounds: { x: 1000, y: 0, width: 400, height: 400 }, zIndex: 3, createdAt: "2026-01-01T00:00:00.000Z", updatedAt: "2026-01-01T00:00:00.000Z" },
    ],
    nodes: [
      { id: "n1", type: "task", title: "Node 1", groupId: "g1", position: { x: 10, y: 10 }, size: { width: 100, height: 50 }, zIndex: 1, updatedAt: "2026-01-01T00:00:00.000Z" },
      { id: "n2", type: "option", title: "Node 2", groupId: "g1", position: { x: 200, y: 20 }, size: { width: 120, height: 60 }, zIndex: 2 },
      { id: "n3", type: "evidence", title: "Node 3", groupId: "g2", position: { x: 60, y: 70 }, size: { width: 80, height: 40 }, zIndex: 1 },
      { id: "n4", type: "task", title: "Node 4", groupId: "g1", position: { x: 400, y: 300 }, size: { width: 90, height: 90 }, zIndex: 3 },
    ],
    edges: [
      { id: "e1", type: "supports", source: "n1", target: "n2", groupId: "g1" },
      { id: "e2", type: "depends_on", source: "n2", target: "n3", groupId: "g1" },
    ],
    tags: [
      { id: "t1", name: "Tag 1", color: "#ff0000", createdAt: "2026-01-01T00:00:00.000Z", updatedAt: "2026-01-01T00:00:00.000Z" },
      { id: "t2", name: "Tag 2", color: "#00ff00", createdAt: "2026-01-01T00:00:00.000Z", updatedAt: "2026-01-01T00:00:00.000Z" },
    ],
    comments: [],
    artifacts: [],
    selection: { kind: "canvas" },
  });
}

const rect = (x: number, y: number, width: number, height: number) => ({ x, y, width, height });

function applyCase(name: string, patch: RenderScenePatch, scene: Scene = baseScene()): Case {
  const result = applyRenderPatchToShapeScene(scene, patch, NOW);
  return { name, fn: "apply", now: NOW, scene, patch, expected: { scene: result.scene, errors: result.errors } };
}

function groupTagsCase(name: string, groupId: string, tagIds: string[], scene: Scene = baseScene()): Case {
  const result = updateShapeSceneGroupTags(scene, groupId, tagIds, NOW);
  return { name, fn: "updateGroupTags", now: NOW, scene, groupId, tagIds, expected: { scene: result.scene, errors: result.errors } };
}

function commentCase(name: string, target: SceneSelection, body: string, scene: Scene = baseScene()): Case {
  const result = addShapeSceneComment(scene, target, body, NOW);
  return { name, fn: "addComment", now: NOW, scene, target, body, maskCommentIds: true, expected: { scene: result.scene, errors: result.errors } };
}

const cases: Case[] = [
  // ---- success: create ----
  applyCase("create-group", { kind: "create-group", group: { id: "gx", title: "New Group", summary: "s", bounds: rect(900, 900, 200, 150), tagIds: ["t1"], zIndex: 5, styleKey: "default" } }),
  applyCase("create-group-empty-title", { kind: "create-group", group: { id: "gx", title: "", summary: "", bounds: rect(900, 900, 200, 150), tagIds: [], zIndex: 0, styleKey: "default" } }),
  applyCase("create-card", { kind: "create-card", card: { id: "nx", groupId: "g1", title: "New Card", summary: "sum", detail: "det", status: "viable", type: "decision_point", bounds: rect(500, 500, 220, 140), zIndex: 4, styleKey: "default", accessibilityLabel: "New Card" } }),
  applyCase("create-card-coerce-type-status", { kind: "create-card", card: { id: "nx", groupId: "g1", title: "C", summary: "", detail: "", status: "bogus", type: "weird", bounds: rect(500, 500, 220, 140), zIndex: 0, styleKey: "default", accessibilityLabel: "C" } }),
  applyCase("create-edge", { kind: "create-edge", groupId: "g1", source: "n1", target: "n4", edgeId: "ex" }),
  applyCase("create-edge-with-label", { kind: "create-edge", groupId: "g1", source: "n2", target: "n4", edgeId: "ex2", label: "custom" }),

  // ---- success: move/edit/resize ----
  applyCase("move-card", { kind: "move-card", id: "n1", position: { x: 999, y: 888 } }),
  applyCase("move-group", { kind: "move-group", id: "g1", delta: { x: 25, y: -10 } }),
  applyCase("set-card-z-index", { kind: "set-card-z-index", id: "n2", zIndex: 99 }),
  applyCase("edit-card-text-title", { kind: "edit-card-text", id: "n1", field: "title", value: "Renamed" }),
  applyCase("edit-card-text-summary", { kind: "edit-card-text", id: "n1", field: "summary", value: "New summary" }),
  applyCase("edit-card-text-detail", { kind: "edit-card-text", id: "n1", field: "detail", value: "New detail" }),
  applyCase("resize-card", { kind: "resize-card", id: "n1", bounds: rect(11, 12, 333, 222) }),
  applyCase("resize-group", { kind: "resize-group", id: "g3", bounds: rect(1000, 0, 500, 500) }),

  // ---- success: delete (cascade) ----
  applyCase("delete-card-cascade-edges", { kind: "delete-card", id: "n2" }),
  applyCase("delete-edge", { kind: "delete-edge", id: "e1" }),
  applyCase("delete-group-cascade-subtree", { kind: "delete-group", id: "g1" }),

  // ---- success: align/distribute ----
  applyCase("align-cards-x-start", { kind: "align-cards", ids: ["n1", "n2", "n4"], axis: "x", mode: "start" }),
  applyCase("align-cards-y-center", { kind: "align-cards", ids: ["n1", "n2", "n4"], axis: "y", mode: "center" }),
  applyCase("align-cards-x-end", { kind: "align-cards", ids: ["n1", "n2"], axis: "x", mode: "end" }),
  applyCase("distribute-cards-x", { kind: "distribute-cards", ids: ["n1", "n2", "n4"], axis: "x" }),
  applyCase("distribute-cards-y", { kind: "distribute-cards", ids: ["n1", "n2", "n4"], axis: "y" }),

  // ---- success: duplicate / batch ----
  applyCase("duplicate-objects", { kind: "duplicate-objects", ids: ["n1", "n2"], delta: { x: 30, y: 30 } }),
  applyCase("batch", { kind: "batch", ops: [
    { kind: "create-card", card: { id: "nx", groupId: "g1", title: "B", summary: "", detail: "", status: "draft", type: "task", bounds: rect(700, 100, 100, 100), zIndex: 0, styleKey: "default", accessibilityLabel: "B" } },
    { kind: "move-card", id: "nx", position: { x: 750, y: 150 } },
  ] }),

  // ---- success: grouping/labeling ----
  applyCase("group-objects", { kind: "group-objects", ids: ["n1", "n4"], frameId: "frame-x", title: "Framed" }),
  applyCase("ungroup", { kind: "ungroup", id: "g2" }),
  applyCase("set-object-group", { kind: "set-object-group", ids: ["n3"], frameId: "g3" }),
  applyCase("set-object-tags-frame", { kind: "set-object-tags", targetKind: "frame", id: "g1", tagIds: ["t1", "t2"] }),
  applyCase("set-object-tags-card", { kind: "set-object-tags", targetKind: "card", id: "n1", tagIds: ["t1"] }),
  applyCase("set-object-tags-edge", { kind: "set-object-tags", targetKind: "edge", id: "e1", tagIds: ["t2"] }),
  applyCase("create-tag", { kind: "create-tag", tag: { id: "t3", name: "Tag 3", color: "#0000ff", description: "desc", createdAt: NOW, updatedAt: "ignored" } }),

  // ---- success: select ----
  applyCase("select-node", { kind: "select", selection: { kind: "node", id: "n1" } }),
  applyCase("select-canvas", { kind: "select", selection: { kind: "canvas" } }),
  applyCase("select-multi", { kind: "select", selection: { kind: "multi", ids: ["n1", "n2"] } }),
  applyCase("select-group", { kind: "select", selection: { kind: "group", id: "g1" } }),
  applyCase("select-edge", { kind: "select", selection: { kind: "edge", id: "e1" } }),

  // ---- side functions ----
  groupTagsCase("update-group-tags", "g1", ["t1", "t2"]),
  groupTagsCase("update-group-tags-unknown-group", "nope", ["t1"]),
  groupTagsCase("update-group-tags-unknown-tag", "g1", ["nope"]),
  commentCase("add-comment-node", { kind: "node", id: "n1" }, "  hello world  "),
  commentCase("add-comment-canvas", { kind: "canvas" }, "global comment"),
  commentCase("add-comment-empty", { kind: "node", id: "n1" }, "   "),
  commentCase("add-comment-unknown", { kind: "node", id: "nope" }, "x"),

  // ---- validation failures ----
  applyCase("err-create-group-dup-and-bounds", { kind: "create-group", group: { id: "g1", title: "Dup", summary: "", bounds: rect(0, 0, 0, 10), tagIds: [], zIndex: 0, styleKey: "default" } }),
  applyCase("err-create-card-unknown-group", { kind: "create-card", card: { id: "nx", groupId: "ghost", title: "C", summary: "", detail: "", status: "draft", type: "task", bounds: rect(0, 0, 10, 10), zIndex: 0, styleKey: "default", accessibilityLabel: "C" } }),
  applyCase("err-create-edge-selfloop-unknown", { kind: "create-edge", groupId: "ghost", source: "x", target: "x", edgeId: "e1" }),
  applyCase("err-move-card-unknown", { kind: "move-card", id: "ghost", position: { x: 0, y: 0 } }),
  applyCase("err-delete-group-unknown", { kind: "delete-group", id: "ghost" }),
  applyCase("err-align-too-few", { kind: "align-cards", ids: ["n1"], axis: "x", mode: "start" }),
  applyCase("err-distribute-too-few", { kind: "distribute-cards", ids: ["n1", "n2"], axis: "x" }),
  applyCase("err-set-object-tags-unknown-tag", { kind: "set-object-tags", targetKind: "card", id: "n1", tagIds: ["ghost"] }),
  applyCase("err-select-unknown-node", { kind: "select", selection: { kind: "node", id: "ghost" } }),
  applyCase("err-batch-empty", { kind: "batch", ops: [] }),
  applyCase("err-group-objects-dup-frame", { kind: "group-objects", ids: ["n1"], frameId: "g1", title: "x" }),
  applyCase("err-duplicate-empty", { kind: "duplicate-objects", ids: [], delta: { x: 0, y: 0 } }),
];

const outDir = dirname(fileURLToPath(import.meta.url));
const outPath = join(outDir, "cases.json");
writeFileSync(outPath, JSON.stringify(cases, null, 2));
console.log(`wrote ${cases.length} cases -> ${outPath}`);
