import type { Scene, SceneComment, SceneEdge, SceneGroup, SceneNode, ScenePatch as AppScenePatch, SceneSelection, Tag } from "./schema";
import { shapeSceneToRenderSnapshot, type RenderCard, type RenderGroup, type RenderSnapshotOptions, type WorldPoint, type WorldRect } from "./renderScene";
import { synthesiseLocalEnvelope, type OperationEnvelope } from "./operation";

const generatedAt = "2026-06-05T00:00:00.000Z";

export type RenderScenePatch =
  | { kind: "create-group"; group: RenderGroup }
  | { kind: "delete-group"; id: string }
  | { kind: "move-group"; id: string; delta: WorldPoint }
  | { kind: "move-card"; id: string; position: WorldPoint }
  | { kind: "set-card-z-index"; id: string; zIndex: number }
  | { kind: "edit-card-text"; id: string; field: "title" | "summary" | "detail"; value: string }
  | { kind: "create-card"; card: RenderCard }
  | { kind: "delete-card"; id: string }
  | { kind: "create-edge"; groupId: string; source: string; target: string; edgeId: string; label?: string }
  | { kind: "delete-edge"; id: string }
  | { kind: "select"; selection: SceneSelection }
  // T2.2: minimal editing ops
  | { kind: "resize-card"; id: string; bounds: WorldRect }
  | { kind: "resize-group"; id: string; bounds: WorldRect }
  | { kind: "align-cards"; ids: string[]; axis: "x" | "y"; mode: "start" | "center" | "end" }
  | { kind: "distribute-cards"; ids: string[]; axis: "x" | "y" }
  | { kind: "duplicate-objects"; ids: string[]; delta: WorldPoint }
  | { kind: "batch"; ops: RenderScenePatch[] }
  // T2.4: grouping and labeling ops
  | { kind: "group-objects"; ids: string[]; frameId: string; parentGroupId?: string | null; title?: string; bounds?: WorldRect }
  | { kind: "ungroup"; id: string }
  | { kind: "set-object-group"; ids: string[]; frameId: string }
  | { kind: "set-object-tags"; targetKind: "card" | "edge" | "frame"; id: string; tagIds: string[] }
  | { kind: "create-tag"; tag: Tag };

/**
 * T2.5: Extended op members that the envelope can carry but are NOT part of
 * the core RenderScenePatch union (they go through different apply paths).
 *
 * - add-comment    : promotes addShapeSceneComment to a logged op slot
 * - export         : wraps export_group / addArtifact so exports become logged ops
 * - accept-proposal: commits a staged MCP proposal envelope
 * - reject-proposal: discards a staged MCP proposal envelope
 *
 * Full apply logic for these is deferred per T2.5 scope; the types live here so
 * the OperationEnvelope can reference them without a separate file.
 */
export type ExtendedOpPatch =
  | { kind: "add-comment"; target: SceneSelection; body: string; author?: string }
  | { kind: "export"; scopeIds: string[]; exportType: string }
  | { kind: "accept-proposal"; proposalId: string }
  | { kind: "reject-proposal"; proposalId: string };

/** Union of all patch kinds that can appear inside an OperationEnvelope. */
export type ExtendedRenderPatch = RenderScenePatch | ExtendedOpPatch;

export type AppliedRenderPatch = {
  scene: Scene;
  appPatch: AppScenePatch;
  errors: string[];
  /** T2.5: the operation envelope that was applied (synthesised or caller-supplied). */
  envelope: OperationEnvelope;
};

export type AppliedCommentUpdate = {
  scene: Scene;
  comment: SceneComment | null;
  errors: string[];
};

/**
 * Apply a RenderScenePatch to a Scene.
 *
 * T2.5: The optional `envelope` parameter threads the operation metadata through
 * without breaking existing callers. When omitted, a local-human envelope is
 * synthesised automatically (actorType:"human", clientId:"local-shell").
 * All existing callers and tests are unchanged.
 */
export function applyRenderPatchToShapeScene(
  scene: Scene,
  patch: RenderScenePatch,
  now = generatedAt,
  envelope?: OperationEnvelope
): AppliedRenderPatch {
  const errors = validateRenderPatchForShapeScene(scene, patch);
  // Resolve envelope once — either caller-supplied or synthesised local-human.
  const resolvedEnvelope = envelope ?? synthesiseLocalEnvelope(patch, scene.sceneVersion, now);
  if (errors.length > 0) {
    return { scene, appPatch: {}, errors, envelope: resolvedEnvelope };
  }

  if (patch.kind === "create-group") {
    const group = renderGroupToSceneGroup(patch.group, now);
    return commitAppPatch(scene, { groups: [group], selection: { kind: "group", id: group.id } }, now, resolvedEnvelope);
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
      now,
      resolvedEnvelope
    );
  }

  if (patch.kind === "move-group") {
    return commitAppPatch(scene, { translateGroups: [{ groupId: patch.id, dx: patch.delta.x, dy: patch.delta.y }], selection: { kind: "group", id: patch.id } }, now, resolvedEnvelope);
  }

  if (patch.kind === "move-card") {
    const node = scene.nodes.find((candidate) => candidate.id === patch.id)!;
    const nextNode = { ...node, position: patch.position, updatedAt: now };
    return commitAppPatch(scene, { nodes: [nextNode], selection: { kind: "node", id: node.id } }, now, resolvedEnvelope);
  }

  if (patch.kind === "set-card-z-index") {
    const node = scene.nodes.find((candidate) => candidate.id === patch.id)!;
    const nextNode = { ...node, zIndex: patch.zIndex, updatedAt: now };
    return commitAppPatch(scene, { nodes: [nextNode], selection: { kind: "node", id: node.id } }, now, resolvedEnvelope);
  }

  if (patch.kind === "edit-card-text") {
    const node = scene.nodes.find((candidate) => candidate.id === patch.id)!;
    const nextNode = { ...node, [patch.field]: patch.value, updatedAt: now };
    return commitAppPatch(scene, { nodes: [nextNode], selection: { kind: "node", id: node.id } }, now, resolvedEnvelope);
  }

  if (patch.kind === "create-card") {
    const node = renderCardToSceneNode(patch.card, now);
    return commitAppPatch(scene, { nodes: [node], selection: { kind: "node", id: node.id } }, now, resolvedEnvelope);
  }

  if (patch.kind === "delete-card") {
    const incidentEdgeIds = scene.edges.filter((edge) => edge.source === patch.id || edge.target === patch.id).map((edge) => edge.id);
    return commitAppPatch(scene, { removeNodeIds: [patch.id], removeEdgeIds: incidentEdgeIds, selection: { kind: "canvas" } }, now, resolvedEnvelope);
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
      tagIds: [],
      updatedAt: now
    };
    return commitAppPatch(scene, { edges: [edge], selection: { kind: "edge", id: edge.id } }, now, resolvedEnvelope);
  }

  if (patch.kind === "delete-edge") {
    return commitAppPatch(scene, { removeEdgeIds: [patch.id], selection: { kind: "canvas" } }, now, resolvedEnvelope);
  }

  if (patch.kind === "resize-card") {
    const node = scene.nodes.find((candidate) => candidate.id === patch.id)!;
    const nextNode = {
      ...node,
      position: { x: patch.bounds.x, y: patch.bounds.y },
      size: { width: patch.bounds.width, height: patch.bounds.height },
      updatedAt: now
    };
    return commitAppPatch(scene, { nodes: [nextNode], selection: { kind: "node", id: node.id } }, now, resolvedEnvelope);
  }

  if (patch.kind === "resize-group") {
    const group = scene.groups.find((candidate) => candidate.id === patch.id)!;
    const nextGroup = { ...group, bounds: patch.bounds, updatedAt: now };
    return commitAppPatch(scene, { groups: [nextGroup], selection: { kind: "group", id: group.id } }, now, resolvedEnvelope);
  }

  if (patch.kind === "align-cards") {
    const targets = scene.nodes.filter((node) => patch.ids.includes(node.id));
    if (targets.length === 0) return { scene, appPatch: {}, errors: [`No matching nodes for align-cards`], envelope: resolvedEnvelope };
    const aligned = alignNodes(targets, patch.axis, patch.mode, now);
    return commitAppPatch(scene, { nodes: aligned, selection: { kind: "canvas" } }, now, resolvedEnvelope);
  }

  if (patch.kind === "distribute-cards") {
    const targets = scene.nodes.filter((node) => patch.ids.includes(node.id));
    if (targets.length === 0) return { scene, appPatch: {}, errors: [`No matching nodes for distribute-cards`], envelope: resolvedEnvelope };
    const distributed = distributeNodes(targets, patch.axis, now);
    return commitAppPatch(scene, { nodes: distributed, selection: { kind: "canvas" } }, now, resolvedEnvelope);
  }

  if (patch.kind === "duplicate-objects") {
    return applyDuplicateObjects(scene, patch.ids, patch.delta, now, resolvedEnvelope);
  }

  if (patch.kind === "batch") {
    return applyBatch(scene, patch.ops, now, resolvedEnvelope);
  }

  // T2.4: grouping and labeling ops
  if (patch.kind === "group-objects") {
    return applyGroupObjects(scene, patch.ids, patch.frameId, patch.parentGroupId ?? null, patch.title, patch.bounds, now, resolvedEnvelope);
  }

  if (patch.kind === "ungroup") {
    return applyUngroup(scene, patch.id, now, resolvedEnvelope);
  }

  if (patch.kind === "set-object-group") {
    return applySetObjectGroup(scene, patch.ids, patch.frameId, now, resolvedEnvelope);
  }

  if (patch.kind === "set-object-tags") {
    return applySetObjectTags(scene, patch.targetKind, patch.id, patch.tagIds, now, resolvedEnvelope);
  }

  if (patch.kind === "create-tag") {
    return applyCreateTag(scene, patch.tag, now, resolvedEnvelope);
  }

  return commitAppPatch(scene, { selection: patch.selection }, now, resolvedEnvelope);
}

export function shapeSceneToFilteredRenderSnapshot(scene: Scene, tagIds: string[], options: RenderSnapshotOptions = {}) {
  const activeTagIds = [...new Set(tagIds)].filter(Boolean);
  if (activeTagIds.length === 0) return shapeSceneToRenderSnapshot(scene, options);

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
    ...options,
    sceneId: options.sceneId ?? `shape-scene-v${scene.sceneVersion}-tag-filter-${activeTagIds.sort().join("-")}`
  });
}

export function updateShapeSceneGroupTags(scene: Scene, groupId: string, tagIds: string[], now = generatedAt): AppliedRenderPatch {
  const group = scene.groups.find((candidate) => candidate.id === groupId);
  // Synthesise a system envelope for tag-update ops (no RenderScenePatch kind covers this yet).
  const groupTagsEnvelope: OperationEnvelope = synthesiseLocalEnvelope(
    { kind: "select", selection: { kind: "group", id: groupId } },
    scene.sceneVersion,
    now
  );
  if (!group) return { scene, appPatch: {}, errors: [`Unknown group id: ${groupId}`], envelope: groupTagsEnvelope };

  const knownTagIds = new Set(scene.tags.map((tag) => tag.id));
  const unknownTagId = tagIds.find((tagId) => !knownTagIds.has(tagId));
  if (unknownTagId) return { scene, appPatch: {}, errors: [`Unknown tag id: ${unknownTagId}`], envelope: groupTagsEnvelope };

  return commitAppPatch(
    scene,
    {
      groups: [{ ...group, tagIds, updatedAt: now }],
      selection: { kind: "group", id: group.id }
    },
    now,
    groupTagsEnvelope
  );
}

export function addShapeSceneComment(scene: Scene, target: SceneSelection, body: string, now = generatedAt): AppliedCommentUpdate {
  const trimmed = body.trim();
  if (!trimmed) return { scene, comment: null, errors: ["Comment body is required"] };
  const selectionErrors = validateSceneSelection(scene, target);
  if (selectionErrors.length > 0) return { scene, comment: null, errors: selectionErrors };

  const comment: SceneComment = {
    id: `renderer-comment-${Date.now().toString(36)}`,
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
    if (patch.card.bounds.width <= 0 || patch.card.bounds.height <= 0) errors.push("Card bounds must be positive");
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
  if (patch.kind === "resize-card") {
    if (!scene.nodes.some((node) => node.id === patch.id)) errors.push(`Unknown node id: ${patch.id}`);
    if (patch.bounds.width <= 0 || patch.bounds.height <= 0) errors.push("resize-card bounds must be positive");
  }
  if (patch.kind === "resize-group") {
    if (!scene.groups.some((group) => group.id === patch.id)) errors.push(`Unknown group id: ${patch.id}`);
    if (patch.bounds.width <= 0 || patch.bounds.height <= 0) errors.push("resize-group bounds must be positive");
  }
  if (patch.kind === "align-cards") {
    if (patch.ids.length < 2) errors.push("align-cards requires at least 2 ids");
    for (const id of patch.ids) {
      if (!scene.nodes.some((node) => node.id === id)) errors.push(`Unknown node id: ${id}`);
    }
  }
  if (patch.kind === "distribute-cards") {
    if (patch.ids.length < 3) errors.push("distribute-cards requires at least 3 ids");
    for (const id of patch.ids) {
      if (!scene.nodes.some((node) => node.id === id)) errors.push(`Unknown node id: ${id}`);
    }
  }
  if (patch.kind === "duplicate-objects") {
    if (patch.ids.length === 0) errors.push("duplicate-objects requires at least 1 id");
    for (const id of patch.ids) {
      if (!scene.nodes.some((node) => node.id === id) && !scene.groups.some((group) => group.id === id)) {
        errors.push(`Unknown id: ${id}`);
      }
    }
  }
  if (patch.kind === "batch") {
    if (patch.ops.length === 0) errors.push("batch requires at least 1 op");
  }
  // T2.4 validation
  if (patch.kind === "group-objects") {
    if (patch.ids.length === 0) errors.push("group-objects requires at least 1 id");
    if (!patch.frameId) errors.push("group-objects requires a frameId");
    if (scene.groups.some((g) => g.id === patch.frameId)) errors.push(`Duplicate group id: ${patch.frameId}`);
    for (const id of patch.ids) {
      if (!scene.nodes.some((n) => n.id === id) && !scene.groups.some((g) => g.id === id)) {
        errors.push(`Unknown id: ${id}`);
      }
    }
    if (patch.parentGroupId != null && !scene.groups.some((g) => g.id === patch.parentGroupId)) {
      errors.push(`Unknown parentGroupId: ${patch.parentGroupId}`);
    }
  }
  if (patch.kind === "ungroup") {
    if (!scene.groups.some((g) => g.id === patch.id)) errors.push(`Unknown group id: ${patch.id}`);
  }
  if (patch.kind === "set-object-group") {
    if (patch.ids.length === 0) errors.push("set-object-group requires at least 1 id");
    if (!scene.groups.some((g) => g.id === patch.frameId)) errors.push(`Unknown group id: ${patch.frameId}`);
    for (const id of patch.ids) {
      if (!scene.nodes.some((n) => n.id === id) && !scene.edges.some((e) => e.id === id)) {
        errors.push(`Unknown id: ${id}`);
      }
    }
  }
  if (patch.kind === "set-object-tags") {
    if (patch.targetKind === "frame" && !scene.groups.some((g) => g.id === patch.id)) errors.push(`Unknown group id: ${patch.id}`);
    if (patch.targetKind === "card" && !scene.nodes.some((n) => n.id === patch.id)) errors.push(`Unknown node id: ${patch.id}`);
    if (patch.targetKind === "edge" && !scene.edges.some((e) => e.id === patch.id)) errors.push(`Unknown edge id: ${patch.id}`);
    const knownTagIds = new Set(scene.tags.map((t) => t.id));
    for (const tagId of patch.tagIds) {
      if (!knownTagIds.has(tagId)) errors.push(`Unknown tag id: ${tagId}`);
    }
  }
  if (patch.kind === "create-tag") {
    if (scene.tags.some((t) => t.id === patch.tag.id)) errors.push(`Duplicate tag id: ${patch.tag.id}`);
  }
  return errors;
}

function selectionVisible(selection: SceneSelection, groupIds: Set<string>, nodeIds: Set<string>, edgeIds: Set<string>): boolean {
  if (selection.kind === "canvas") return true;
  if (selection.kind === "group") return groupIds.has(selection.id);
  if (selection.kind === "node") return nodeIds.has(selection.id);
  // T2.2 multi-select stays visible while any member node survives the filter.
  if (selection.kind === "multi") return selection.ids.some((id) => nodeIds.has(id));
  return edgeIds.has(selection.id);
}

function validateSceneSelection(scene: Scene, selection: SceneSelection): string[] {
  if (selection.kind === "canvas") return [];
  if (selection.kind === "group" && scene.groups.some((group) => group.id === selection.id)) return [];
  if (selection.kind === "node" && scene.nodes.some((node) => node.id === selection.id)) return [];
  if (selection.kind === "edge" && scene.edges.some((edge) => edge.id === selection.id)) return [];
  // T2.2 multi-select is valid when every member node id exists.
  if (selection.kind === "multi") {
    const unknown = selection.ids.find((id) => !scene.nodes.some((node) => node.id === id));
    return unknown ? [`Unknown node selection id: ${unknown}`] : [];
  }
  return [`Unknown ${selection.kind} selection id: ${selection.id}`];
}

function commitAppPatch(scene: Scene, patch: AppScenePatch, now: string, envelope: OperationEnvelope): AppliedRenderPatch {
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
    errors: [],
    envelope
  };
}

// T2.4 fix: accept parentGroupId param instead of hardcoding null so nested-frame
// creation works when the caller supplies a parent.
function renderGroupToSceneGroup(group: RenderGroup, now: string, parentGroupId: string | null = null): SceneGroup {
  return {
    id: group.id,
    parentGroupId,
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
    tagIds: [],
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

// ---------------------------------------------------------------------------
// T2.2 helper: align-cards
// ---------------------------------------------------------------------------

function alignNodes(nodes: SceneNode[], axis: "x" | "y", mode: "start" | "center" | "end", now: string): SceneNode[] {
  if (axis === "x") {
    const minX = Math.min(...nodes.map((n) => n.position.x));
    const maxRight = Math.max(...nodes.map((n) => n.position.x + n.size.width));
    return nodes.map((node) => {
      let newX: number;
      if (mode === "start") newX = minX;
      else if (mode === "end") newX = maxRight - node.size.width;
      else newX = (minX + maxRight) / 2 - node.size.width / 2;
      return { ...node, position: { x: newX, y: node.position.y }, updatedAt: now };
    });
  } else {
    const minY = Math.min(...nodes.map((n) => n.position.y));
    const maxBottom = Math.max(...nodes.map((n) => n.position.y + n.size.height));
    return nodes.map((node) => {
      let newY: number;
      if (mode === "start") newY = minY;
      else if (mode === "end") newY = maxBottom - node.size.height;
      else newY = (minY + maxBottom) / 2 - node.size.height / 2;
      return { ...node, position: { x: node.position.x, y: newY }, updatedAt: now };
    });
  }
}

// ---------------------------------------------------------------------------
// T2.2 helper: distribute-cards
// ---------------------------------------------------------------------------

function distributeNodes(nodes: SceneNode[], axis: "x" | "y", now: string): SceneNode[] {
  if (axis === "x") {
    const sorted = [...nodes].sort((a, b) => a.position.x - b.position.x);
    const minX = sorted[0].position.x;
    const maxRight = Math.max(...sorted.map((n) => n.position.x + n.size.width));
    const totalWidth = sorted.reduce((acc, n) => acc + n.size.width, 0);
    const gap = (maxRight - minX - totalWidth) / (sorted.length - 1);
    let cursor = minX;
    const positioned = sorted.map((node) => {
      const result = { ...node, position: { x: cursor, y: node.position.y }, updatedAt: now };
      cursor += node.size.width + gap;
      return result;
    });
    return positioned;
  } else {
    const sorted = [...nodes].sort((a, b) => a.position.y - b.position.y);
    const minY = sorted[0].position.y;
    const maxBottom = Math.max(...sorted.map((n) => n.position.y + n.size.height));
    const totalHeight = sorted.reduce((acc, n) => acc + n.size.height, 0);
    const gap = (maxBottom - minY - totalHeight) / (sorted.length - 1);
    let cursor = minY;
    const positioned = sorted.map((node) => {
      const result = { ...node, position: { x: node.position.x, y: cursor }, updatedAt: now };
      cursor += node.size.height + gap;
      return result;
    });
    return positioned;
  }
}

// ---------------------------------------------------------------------------
// T2.2 helper: duplicate-objects
// ---------------------------------------------------------------------------

function applyDuplicateObjects(
  scene: Scene,
  ids: string[],
  delta: WorldPoint,
  now: string,
  envelope: OperationEnvelope
): AppliedRenderPatch {
  const idSet = new Set(ids);
  const sourceNodes = scene.nodes.filter((n) => idSet.has(n.id));
  const oldToNew = new Map<string, string>();
  for (const id of ids) oldToNew.set(id, `${id}-dup-${now}`);

  const newNodes: SceneNode[] = sourceNodes.map((n) => ({
    ...n,
    id: oldToNew.get(n.id)!,
    position: { x: n.position.x + delta.x, y: n.position.y + delta.y },
    updatedAt: now
  }));

  // Rewire edges internal to the duplicated set; drop boundary-crossing edges.
  const newEdges: SceneEdge[] = scene.edges
    .filter((e) => idSet.has(e.source) && idSet.has(e.target))
    .map((e) => ({
      ...e,
      id: `${e.id}-dup-${now}`,
      source: oldToNew.get(e.source)!,
      target: oldToNew.get(e.target)!,
      updatedAt: now
    }));

  // Selection: the first clone (or canvas if nothing duplicated)
  const selection: SceneSelection = newNodes.length > 0 ? { kind: "node", id: newNodes[0].id } : { kind: "canvas" };
  return commitAppPatch(scene, { nodes: newNodes, edges: newEdges, selection }, now, envelope);
}

// ---------------------------------------------------------------------------
// T2.2 helper: batch
// ---------------------------------------------------------------------------

function applyBatch(
  scene: Scene,
  ops: RenderScenePatch[],
  now: string,
  envelope: OperationEnvelope
): AppliedRenderPatch {
  let current = scene;
  let lastErrors: string[] = [];
  for (const op of ops) {
    const result = applyRenderPatchToShapeScene(current, op, now, envelope);
    if (result.errors.length > 0) {
      lastErrors = result.errors;
      break;
    }
    current = result.scene;
  }
  if (lastErrors.length > 0) {
    return { scene, appPatch: {}, errors: lastErrors, envelope };
  }
  // Reconstruct a merged appPatch (nodes/edges/removes) from the diff
  const addedNodeIds = new Set(current.nodes.map((n) => n.id));
  const removedNodeIds = scene.nodes.filter((n) => !addedNodeIds.has(n.id)).map((n) => n.id);
  const changedNodes = current.nodes.filter((n) => {
    const orig = scene.nodes.find((o) => o.id === n.id);
    return !orig || orig !== n;
  });
  const addedEdgeIds = new Set(current.edges.map((e) => e.id));
  const removedEdgeIds = scene.edges.filter((e) => !addedEdgeIds.has(e.id)).map((e) => e.id);
  const changedEdges = current.edges.filter((e) => {
    const orig = scene.edges.find((o) => o.id === e.id);
    return !orig || orig !== e;
  });
  const addedGroupIds = new Set(current.groups.map((g) => g.id));
  const removedGroupIds = scene.groups.filter((g) => !addedGroupIds.has(g.id)).map((g) => g.id);
  const changedGroups = current.groups.filter((g) => {
    const orig = scene.groups.find((o) => o.id === g.id);
    return !orig || orig !== g;
  });
  return {
    scene: current,
    appPatch: {
      nodes: changedNodes.length > 0 ? changedNodes : undefined,
      edges: changedEdges.length > 0 ? changedEdges : undefined,
      groups: changedGroups.length > 0 ? changedGroups : undefined,
      removeNodeIds: removedNodeIds.length > 0 ? removedNodeIds : undefined,
      removeEdgeIds: removedEdgeIds.length > 0 ? removedEdgeIds : undefined,
      removeGroupIds: removedGroupIds.length > 0 ? removedGroupIds : undefined,
      selection: current.selection
    },
    errors: [],
    envelope
  };
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

// ---------------------------------------------------------------------------
// T2.4 helpers
// ---------------------------------------------------------------------------

/** Compute padded AABB from a set of node positions/sizes. */
function computeGroupBounds(members: SceneNode[], padding = 40): WorldRect {
  if (members.length === 0) return { x: 0, y: 0, width: 200, height: 200 };
  const minX = Math.min(...members.map((n) => n.position.x));
  const minY = Math.min(...members.map((n) => n.position.y));
  const maxX = Math.max(...members.map((n) => n.position.x + n.size.width));
  const maxY = Math.max(...members.map((n) => n.position.y + n.size.height));
  return {
    x: minX - padding,
    y: minY - padding,
    width: maxX - minX + padding * 2,
    height: maxY - minY + padding * 2
  };
}

/**
 * group-objects: create a new persistent frame from a selection of node/group ids,
 * re-pointing each node's groupId to the new frame, and each named child group's
 * parentGroupId to the new frame.
 */
function applyGroupObjects(
  scene: Scene,
  ids: string[],
  frameId: string,
  parentGroupId: string | null,
  title: string | undefined,
  bounds: WorldRect | undefined,
  now: string,
  envelope: OperationEnvelope
): AppliedRenderPatch {
  const idSet = new Set(ids);
  const memberNodes = scene.nodes.filter((n) => idSet.has(n.id));
  const computedBounds = bounds ?? computeGroupBounds(memberNodes);
  const newGroup: SceneGroup = {
    id: frameId,
    parentGroupId,
    title: title ?? "Group",
    summary: "",
    bounds: computedBounds,
    tagIds: [],
    zIndex: 0,
    collapsed: false,
    createdAt: now,
    updatedAt: now
  };
  // Re-point member nodes to the new frame
  const updatedNodes: SceneNode[] = memberNodes.map((n) => ({ ...n, groupId: frameId, updatedAt: now }));
  // Re-parent child groups to the new frame
  const updatedGroups: SceneGroup[] = scene.groups
    .filter((g) => idSet.has(g.id))
    .map((g) => ({ ...g, parentGroupId: frameId, updatedAt: now }));
  return commitAppPatch(
    scene,
    {
      groups: [newGroup, ...updatedGroups],
      nodes: updatedNodes,
      selection: { kind: "group", id: frameId }
    },
    now,
    envelope
  );
}

/**
 * ungroup: dissolve a frame by re-pointing all its direct members to the frame's
 * parentGroupId (or its own parentGroupId). The frame itself is removed but all
 * member nodes/edges and their ids/geometry/tags/text are preserved.
 */
function applyUngroup(scene: Scene, id: string, now: string, envelope: OperationEnvelope): AppliedRenderPatch {
  const frame = scene.groups.find((g) => g.id === id)!;
  const newParent = frame.parentGroupId;

  // Members that cannot be re-pointed (top-level frame with no parent) are kept at
  // their current groupId unchanged when there's no parent to re-point to.
  // Per §10: a node can never be group-less; if parentGroupId is null and there are
  // other groups, pick the first available group; otherwise do not change.
  const fallbackGroupId = newParent ?? scene.groups.find((g) => g.id !== id)?.id ?? null;

  // Re-parent direct member nodes
  const updatedNodes: SceneNode[] = scene.nodes
    .filter((n) => n.groupId === id)
    .map((n) => ({
      ...n,
      groupId: fallbackGroupId ?? n.groupId,
      updatedAt: now
    }));

  // Re-parent direct child frames
  const updatedGroups: SceneGroup[] = scene.groups
    .filter((g) => g.parentGroupId === id)
    .map((g) => ({ ...g, parentGroupId: newParent, updatedAt: now }));

  // Re-parent direct member edges
  const updatedEdges: SceneEdge[] = scene.edges
    .filter((e) => e.groupId === id)
    .map((e) => ({
      ...e,
      groupId: fallbackGroupId ?? e.groupId,
      updatedAt: now
    }));

  return commitAppPatch(
    scene,
    {
      groups: updatedGroups,
      nodes: updatedNodes,
      edges: updatedEdges,
      removeGroupIds: [id],
      selection: { kind: "canvas" }
    },
    now,
    envelope
  );
}

/**
 * set-object-group: move a set of nodes/edges to a different frame by updating
 * their groupId. The frame bounds are not re-clamped here (hull update is a
 * core/render concern per §4); only the membership field changes.
 */
function applySetObjectGroup(
  scene: Scene,
  ids: string[],
  frameId: string,
  now: string,
  envelope: OperationEnvelope
): AppliedRenderPatch {
  const idSet = new Set(ids);
  const updatedNodes: SceneNode[] = scene.nodes
    .filter((n) => idSet.has(n.id))
    .map((n) => ({ ...n, groupId: frameId, updatedAt: now }));
  const updatedEdges: SceneEdge[] = scene.edges
    .filter((e) => idSet.has(e.id))
    .map((e) => ({ ...e, groupId: frameId, updatedAt: now }));
  // Selection: first moved node or first moved edge
  const firstNode = updatedNodes[0];
  const selection: SceneSelection = firstNode
    ? { kind: "node", id: firstNode.id }
    : updatedEdges[0]
      ? { kind: "edge", id: updatedEdges[0].id }
      : { kind: "group", id: frameId };
  return commitAppPatch(
    scene,
    { nodes: updatedNodes, edges: updatedEdges, selection },
    now,
    envelope
  );
}

/**
 * set-object-tags: generalise updateShapeSceneGroupTags to any object kind.
 * For "frame" this is the existing group-tags path; for "card"/"edge" it's the
 * new generalisation.
 */
function applySetObjectTags(
  scene: Scene,
  targetKind: "card" | "edge" | "frame",
  id: string,
  tagIds: string[],
  now: string,
  envelope: OperationEnvelope
): AppliedRenderPatch {
  if (targetKind === "frame") {
    const group = scene.groups.find((g) => g.id === id)!;
    return commitAppPatch(
      scene,
      { groups: [{ ...group, tagIds, updatedAt: now }], selection: { kind: "group", id } },
      now,
      envelope
    );
  }
  if (targetKind === "card") {
    const node = scene.nodes.find((n) => n.id === id)!;
    return commitAppPatch(
      scene,
      { nodes: [{ ...node, tagIds, updatedAt: now }], selection: { kind: "node", id } },
      now,
      envelope
    );
  }
  // edge
  const edge = scene.edges.find((e) => e.id === id)!;
  return commitAppPatch(
    scene,
    { edges: [{ ...edge, tagIds, updatedAt: now }], selection: { kind: "edge", id } },
    now,
    envelope
  );
}

/**
 * create-tag: add a new Tag to the scene tag registry. The tag is validated by
 * the caller (duplicate-id check is in validation). This makes tag creation a
 * normal operation event.
 */
function applyCreateTag(scene: Scene, tag: Tag, now: string, envelope: OperationEnvelope): AppliedRenderPatch {
  const nextScene: Scene = {
    ...scene,
    sceneVersion: scene.sceneVersion + 1,
    tags: [...scene.tags, { ...tag, updatedAt: now }],
    updatedAt: now
  };
  return { scene: nextScene, appPatch: {}, errors: [], envelope };
}
