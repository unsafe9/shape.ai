import type { Scene, SceneEdge, SceneNode } from "../../shared/schema";

export { excludedBusinessFields, shapeSceneToRenderSnapshot } from "../../shared/renderScene";
export {
  addShapeSceneComment,
  applyRenderPatchToShapeScene,
  shapeSceneToFilteredRenderSnapshot,
  updateShapeSceneGroupTags
} from "../../shared/renderPatch";
export type { AppliedCommentUpdate, AppliedRenderPatch, RenderScenePatch } from "../../shared/renderPatch";

const generatedAt = "2026-06-05T00:00:00.000Z";

export function createShapeSceneFixture(): Scene {
  return {
    version: 1,
    sceneVersion: 12,
    groups: [
      {
        id: "shape-group-renderer",
        parentGroupId: null,
        title: "Renderer migration",
        summary: "Current app scene projected into the production renderer contract.",
        bounds: { x: 0, y: 0, width: 1160, height: 780 },
        tagIds: ["tag-renderer"],
        zIndex: 0,
        collapsed: false,
        createdAt: generatedAt,
        updatedAt: generatedAt
      },
      {
        id: "shape-group-parity",
        parentGroupId: null,
        title: "Parity checks",
        summary: "Editing, selection, edge workflow, and export boundaries stay app-owned.",
        bounds: { x: 1380, y: 120, width: 980, height: 680 },
        tagIds: ["tag-product"],
        zIndex: 1,
        collapsed: false,
        createdAt: generatedAt,
        updatedAt: generatedAt
      }
    ],
    nodes: [
      sceneNode("shape-node-contract", "shape-group-renderer", "decision_point", "Scene contract", "Renderer receives stable ids, bounds, text snippets, style keys, and selection seed.", 120, 150, 0),
      sceneNode("shape-node-wgpu", "shape-group-renderer", "evidence", "Visible WebGPU path", "Rust owns a WebGPU canvas and uploads retained primitive vertices for cards and edges.", 530, 210, 1),
      sceneNode("shape-node-overlay", "shape-group-parity", "task", "DOM edit bridge", "Active text editing stays a native DOM textarea while normal objects remain renderer scene objects.", 1500, 250, 2),
      sceneNode("shape-node-export", "shape-group-parity", "tradeoff", "Export compatibility", "Comments, artifacts, MCP workflow, and deterministic exports remain outside the renderer.", 1900, 390, 3)
    ],
    edges: [
      sceneEdge("shape-edge-contract-wgpu", "shape-group-renderer", "shape-node-contract", "shape-node-wgpu", "supports"),
      sceneEdge("shape-edge-wgpu-overlay", "shape-group-parity", "shape-node-wgpu", "shape-node-overlay", "depends on"),
      sceneEdge("shape-edge-overlay-export", "shape-group-parity", "shape-node-overlay", "shape-node-export", "trades off")
    ],
    tags: [
      {
        id: "tag-renderer",
        name: "Renderer",
        color: "#2f7ee6",
        description: "Business tag registry remains in the app layer.",
        createdAt: generatedAt,
        updatedAt: generatedAt
      },
      {
        id: "tag-product",
        name: "Product",
        color: "#158f83",
        description: "Product workflow metadata is not serialized into Rust.",
        createdAt: generatedAt,
        updatedAt: generatedAt
      }
    ],
    comments: [
      {
        id: "shape-comment-renderer",
        target: { kind: "node", id: "shape-node-contract" },
        body: "This comment is intentionally excluded from SceneSnapshot.",
        author: "human",
        resolved: false,
        createdAt: generatedAt,
        updatedAt: generatedAt
      }
    ],
    artifacts: [
      {
        id: "shape-artifact-madr",
        type: "madr",
        title: "Renderer decision export",
        target: { kind: "group", id: "shape-group-renderer" },
        path: "exports/renderer-decision.md",
        contentType: "text/markdown",
        createdAt: generatedAt,
        sceneVersion: 12
      }
    ],
    selection: { kind: "node", id: "shape-node-contract" },
    updatedAt: generatedAt
  };
}

function sceneNode(
  id: string,
  groupId: string,
  type: SceneNode["type"],
  title: string,
  summary: string,
  x: number,
  y: number,
  zIndex: number
): SceneNode {
  return {
    id,
    groupId,
    type,
    title,
    summary,
    detail: `${summary} Detail stays in app state until projected into a render text field.`,
    status: "draft",
    confidence: 0.74,
    evidenceRefs: [`evidence-${id}`],
    childDecisionIds: [],
    position: { x, y },
    size: { width: 320, height: 172 },
    zIndex,
    tagIds: [],
    updatedAt: generatedAt
  };
}

function sceneEdge(id: string, groupId: string, source: string, target: string, label: string): SceneEdge {
  return {
    id,
    groupId,
    type: label === "trades off" ? "trades_off_with" : label === "depends on" ? "depends_on" : "supports",
    source,
    target,
    label,
    rationale: `Business rationale for ${id} remains outside renderer scene.`,
    confidence: 0.66,
    tagIds: [],
    updatedAt: generatedAt
  };
}
