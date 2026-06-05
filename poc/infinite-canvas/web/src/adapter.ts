import type { Scene, SceneComment, SceneEdge, SceneGroup, SceneNode, ScenePatch as AppScenePatch, SceneSelection } from "../../../../src/shared/schema";
import { excludedBusinessFields, shapeSceneToRenderSnapshot } from "../../../../src/shared/renderScene";
import type { RenderCard, RenderGroup, ScenePatch as RenderScenePatch } from "./scene";

const generatedAt = "2026-06-05T00:00:00.000Z";

export { excludedBusinessFields, shapeSceneToRenderSnapshot };

export type AppliedRenderPatch = {
  scene: Scene;
  appPatch: AppScenePatch;
  errors: string[];
};

export type AppliedCommentUpdate = {
  scene: Scene;
  comment: SceneComment | null;
  errors: string[];
};

export function applyRenderPatchToShapeScene(scene: Scene, patch: RenderScenePatch, now = generatedAt): AppliedRenderPatch {
  const errors = validateRenderPatchForShapeScene(scene, patch);
  if (errors.length > 0) return { scene, appPatch: {}, errors };

  if (patch.kind === "create-group") {
    const group = renderGroupToSceneGroup(patch.group, now);
    return commitAppPatch(scene, { groups: [group], selection: { kind: "group", id: group.id } }, now);
  }

  if (patch.kind === "delete-group") {
    const removedGroupIds = descendantGroupIds(scene, patch.id);
    const removedNodeIds = scene.nodes.filter((node) => removedGroupIds.has(node.groupId)).map((node) => node.id);
    const removedNodeIdSet = new Set(removedNodeIds);
    const removedEdgeIds = scene.edges
      .filter((edge) => removedGroupIds.has(edge.groupId) || removedNodeIdSet.has(edge.source) || removedNodeIdSet.has(edge.target))
      .map((edge) => edge.id);
    return commitAppPatch(
      scene,
      {
        removeGroupIds: Array.from(removedGroupIds),
        removeNodeIds: removedNodeIds,
        removeEdgeIds: removedEdgeIds,
        selection: { kind: "canvas" }
      },
      now
    );
  }

  if (patch.kind === "move-group") {
    return commitAppPatch(scene, { translateGroups: [{ groupId: patch.id, dx: patch.delta.x, dy: patch.delta.y }], selection: { kind: "group", id: patch.id } }, now);
  }

  if (patch.kind === "move-card") {
    const node = scene.nodes.find((candidate) => candidate.id === patch.id)!;
    const nextNode = { ...node, position: patch.position, updatedAt: now };
    return commitAppPatch(scene, { nodes: [nextNode], selection: { kind: "node", id: node.id } }, now);
  }

  if (patch.kind === "set-card-z-index") {
    const node = scene.nodes.find((candidate) => candidate.id === patch.id)!;
    const nextNode = { ...node, zIndex: patch.zIndex, updatedAt: now };
    return commitAppPatch(scene, { nodes: [nextNode], selection: { kind: "node", id: node.id } }, now);
  }

  if (patch.kind === "edit-card-text") {
    const node = scene.nodes.find((candidate) => candidate.id === patch.id)!;
    const nextNode = { ...node, [patch.field]: patch.value, updatedAt: now };
    return commitAppPatch(scene, { nodes: [nextNode], selection: { kind: "node", id: node.id } }, now);
  }

  if (patch.kind === "create-card") {
    const node = renderCardToSceneNode(patch.card, now);
    return commitAppPatch(scene, { nodes: [node], selection: { kind: "node", id: node.id } }, now);
  }

  if (patch.kind === "delete-card") {
    const incidentEdgeIds = scene.edges.filter((edge) => edge.source === patch.id || edge.target === patch.id).map((edge) => edge.id);
    return commitAppPatch(scene, { removeNodeIds: [patch.id], removeEdgeIds: incidentEdgeIds, selection: { kind: "canvas" } }, now);
  }

  if (patch.kind === "create-edge") {
    const edge: SceneEdge = {
      id: patch.edgeId,
      groupId: patch.groupId,
      type: "supports",
      source: patch.source,
      target: patch.target,
      label: patch.label ?? "relates",
      rationale: "",
      confidence: 0.5,
      updatedAt: now
    };
    return commitAppPatch(scene, { edges: [edge], selection: { kind: "edge", id: edge.id } }, now);
  }

  if (patch.kind === "delete-edge") {
    return commitAppPatch(scene, { removeEdgeIds: [patch.id], selection: { kind: "canvas" } }, now);
  }

  return commitAppPatch(scene, { selection: patch.selection }, now);
}

export function createShapeSceneFixture(): Scene {
  return {
    version: 1,
    sceneVersion: 12,
    groups: [
      {
        id: "shape-group-renderer",
        parentGroupId: null,
        title: "Renderer migration",
        summary: "Current app scene projected into the POC renderer contract.",
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

export function shapeSceneToFilteredRenderSnapshot(scene: Scene, tagIds: string[]) {
  const activeTagIds = [...new Set(tagIds)].filter(Boolean);
  if (activeTagIds.length === 0) return shapeSceneToRenderSnapshot(scene);

  const groups = scene.groups.filter((group) => activeTagIds.every((tagId) => group.tagIds.includes(tagId)));
  const groupIds = new Set(groups.map((group) => group.id));
  const nodes = scene.nodes.filter((node) => groupIds.has(node.groupId));
  const nodeIds = new Set(nodes.map((node) => node.id));
  const edges = scene.edges.filter((edge) => groupIds.has(edge.groupId) && nodeIds.has(edge.source) && nodeIds.has(edge.target));
  const filteredScene: Scene = {
    ...scene,
    groups,
    nodes,
    edges,
    selection: selectionVisible(scene.selection, groupIds, nodeIds, new Set(edges.map((edge) => edge.id))) ? scene.selection : { kind: "canvas" }
  };
  return shapeSceneToRenderSnapshot(filteredScene, {
    sceneId: `shape-scene-v${scene.sceneVersion}-tag-filter-${activeTagIds.sort().join("-")}`
  });
}

export function updateShapeSceneGroupTags(scene: Scene, groupId: string, tagIds: string[], now = generatedAt): AppliedRenderPatch {
  const group = scene.groups.find((candidate) => candidate.id === groupId);
  if (!group) return { scene, appPatch: {}, errors: [`Unknown group id: ${groupId}`] };

  const knownTagIds = new Set(scene.tags.map((tag) => tag.id));
  const unknownTagId = tagIds.find((tagId) => !knownTagIds.has(tagId));
  if (unknownTagId) return { scene, appPatch: {}, errors: [`Unknown tag id: ${unknownTagId}`] };

  return commitAppPatch(
    scene,
    {
      groups: [{ ...group, tagIds, updatedAt: now }],
      selection: { kind: "group", id: group.id }
    },
    now
  );
}

export function addShapeSceneComment(scene: Scene, target: SceneSelection, body: string, now = generatedAt): AppliedCommentUpdate {
  const trimmed = body.trim();
  if (!trimmed) return { scene, comment: null, errors: ["Comment body is required"] };
  const selectionErrors = validateSceneSelection(scene, target);
  if (selectionErrors.length > 0) return { scene, comment: null, errors: selectionErrors };

  const comment: SceneComment = {
    id: `poc-comment-${Date.now().toString(36)}`,
    target,
    body: trimmed,
    author: "human",
    resolved: false,
    createdAt: now,
    updatedAt: now
  };
  return {
    scene: {
      ...scene,
      sceneVersion: scene.sceneVersion + 1,
      comments: [comment, ...scene.comments],
      updatedAt: now
    },
    comment,
    errors: []
  };
}

function validateRenderPatchForShapeScene(scene: Scene, patch: RenderScenePatch): string[] {
  const errors: string[] = [];
  if (patch.kind === "move-card" || patch.kind === "set-card-z-index" || patch.kind === "edit-card-text") {
    if (!scene.nodes.some((node) => node.id === patch.id)) errors.push(`Unknown node id: ${patch.id}`);
  }
  if (patch.kind === "create-group") {
    if (scene.groups.some((group) => group.id === patch.group.id)) errors.push(`Duplicate group id: ${patch.group.id}`);
    if (patch.group.bounds.width <= 0 || patch.group.bounds.height <= 0) errors.push("Group bounds must be positive");
  }
  if (patch.kind === "delete-group" && !scene.groups.some((group) => group.id === patch.id)) errors.push(`Unknown group id: ${patch.id}`);
  if (patch.kind === "move-group" && !scene.groups.some((group) => group.id === patch.id)) errors.push(`Unknown group id: ${patch.id}`);
  if (patch.kind === "create-card") {
    if (scene.nodes.some((node) => node.id === patch.card.id)) errors.push(`Duplicate node id: ${patch.card.id}`);
    if (!scene.groups.some((group) => group.id === patch.card.groupId)) errors.push(`Unknown group id: ${patch.card.groupId}`);
  }
  if (patch.kind === "delete-card" && !scene.nodes.some((node) => node.id === patch.id)) errors.push(`Unknown node id: ${patch.id}`);
  if (patch.kind === "create-edge") {
    if (patch.source === patch.target) errors.push("Edge source and target must differ");
    if (!scene.nodes.some((node) => node.id === patch.source)) errors.push(`Unknown source node id: ${patch.source}`);
    if (!scene.nodes.some((node) => node.id === patch.target)) errors.push(`Unknown target node id: ${patch.target}`);
    if (!scene.groups.some((group) => group.id === patch.groupId)) errors.push(`Unknown group id: ${patch.groupId}`);
    if (scene.edges.some((edge) => edge.id === patch.edgeId)) errors.push(`Duplicate edge id: ${patch.edgeId}`);
  }
  if (patch.kind === "delete-edge" && !scene.edges.some((edge) => edge.id === patch.id)) errors.push(`Unknown edge id: ${patch.id}`);
  if (patch.kind === "select") errors.push(...validateSceneSelection(scene, patch.selection));
  return errors;
}

function selectionVisible(selection: SceneSelection, groupIds: Set<string>, nodeIds: Set<string>, edgeIds: Set<string>): boolean {
  if (selection.kind === "canvas") return true;
  if (selection.kind === "group") return groupIds.has(selection.id);
  if (selection.kind === "node") return nodeIds.has(selection.id);
  return edgeIds.has(selection.id);
}

function validateSceneSelection(scene: Scene, selection: SceneSelection): string[] {
  if (selection.kind === "canvas") return [];
  if (selection.kind === "group" && scene.groups.some((group) => group.id === selection.id)) return [];
  if (selection.kind === "node" && scene.nodes.some((node) => node.id === selection.id)) return [];
  if (selection.kind === "edge" && scene.edges.some((edge) => edge.id === selection.id)) return [];
  return [`Unknown ${selection.kind} selection id: ${selection.id}`];
}

function commitAppPatch(scene: Scene, patch: AppScenePatch, now: string): AppliedRenderPatch {
  const removeGroupIds = new Set(patch.removeGroupIds ?? []);
  const removeNodeIds = new Set(patch.removeNodeIds ?? []);
  const removeEdgeIds = new Set(patch.removeEdgeIds ?? []);
  const groupsById = new Map(scene.groups.filter((group) => !removeGroupIds.has(group.id)).map((group) => [group.id, group]));
  for (const group of patch.groups ?? []) groupsById.set(group.id, group);
  const nodesById = new Map(scene.nodes.map((node) => [node.id, node]));
  for (const nodeId of removeNodeIds) nodesById.delete(nodeId);
  for (const node of patch.nodes ?? []) nodesById.set(node.id, node);
  for (const movement of patch.translateGroups ?? []) {
    const group = groupsById.get(movement.groupId);
    if (!group) continue;
    groupsById.set(movement.groupId, {
      ...group,
      bounds: { ...group.bounds, x: group.bounds.x + movement.dx, y: group.bounds.y + movement.dy },
      updatedAt: now
    });
    for (const [nodeId, node] of nodesById) {
      if (node.groupId !== movement.groupId) continue;
      nodesById.set(nodeId, {
        ...node,
        position: { x: node.position.x + movement.dx, y: node.position.y + movement.dy },
        updatedAt: now
      });
    }
  }
  const edgesById = new Map(scene.edges.map((edge) => [edge.id, edge]));
  for (const edgeId of removeEdgeIds) edgesById.delete(edgeId);
  for (const edge of patch.edges ?? []) edgesById.set(edge.id, edge);
  const groups = Array.from(groupsById.values());
  const nodes = Array.from(nodesById.values()).filter((node) => groupsById.has(node.groupId));
  const liveNodeIds = new Set(nodes.map((node) => node.id));
  return {
    scene: {
      ...scene,
      sceneVersion: scene.sceneVersion + 1,
      groups,
      nodes,
      edges: Array.from(edgesById.values()).filter((edge) => groupsById.has(edge.groupId) && liveNodeIds.has(edge.source) && liveNodeIds.has(edge.target)),
      selection: patch.selection ?? scene.selection,
      updatedAt: now
    },
    appPatch: patch,
    errors: []
  };
}

function renderGroupToSceneGroup(group: RenderGroup, now: string): SceneGroup {
  return {
    id: group.id,
    parentGroupId: null,
    title: group.title || "Untitled group",
    summary: group.summary,
    bounds: group.bounds,
    tagIds: group.tagIds,
    zIndex: group.zIndex,
    collapsed: false,
    createdAt: now,
    updatedAt: now
  };
}

function renderCardToSceneNode(card: RenderCard, now: string): SceneNode {
  return {
    id: card.id,
    groupId: card.groupId,
    type: sceneNodeType(card.type),
    title: card.title || "Untitled node",
    summary: card.summary,
    detail: card.detail,
    status: sceneNodeStatus(card.status),
    confidence: 0.5,
    evidenceRefs: [],
    childDecisionIds: [],
    position: { x: card.bounds.x, y: card.bounds.y },
    size: { width: card.bounds.width, height: card.bounds.height },
    zIndex: card.zIndex,
    updatedAt: now
  };
}

function sceneNodeType(type: string): SceneNode["type"] {
  if (
    type === "proposition" ||
    type === "decision_point" ||
    type === "option" ||
    type === "evidence" ||
    type === "tradeoff" ||
    type === "blocker" ||
    type === "subdecision" ||
    type === "task" ||
    type === "artifact"
  ) {
    return type;
  }
  return "task";
}

function sceneNodeStatus(status: string): SceneNode["status"] {
  if (
    status === "draft" ||
    status === "viable" ||
    status === "conditional" ||
    status === "infeasible" ||
    status === "unknown" ||
    status === "selected" ||
    status === "deferred" ||
    status === "complete"
  ) {
    return status;
  }
  return "draft";
}

function descendantGroupIds(scene: Scene, rootGroupId: string): Set<string> {
  const ids = new Set([rootGroupId]);
  let changed = true;
  while (changed) {
    changed = false;
    for (const group of scene.groups) {
      if (group.parentGroupId && ids.has(group.parentGroupId) && !ids.has(group.id)) {
        ids.add(group.id);
        changed = true;
      }
    }
  }
  return ids;
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
    updatedAt: generatedAt
  };
}
