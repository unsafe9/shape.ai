// CC3.3 — template library data layer.
//
// Talks to the phase-1 Rust template HTTP endpoints (vite proxies /api to the
// server, which seeds builtins on router build and persists user templates as
// Record kind=template with tombstone-on-delete). Also builds a TemplateContract
// from the current selection client-side — the TS mirror of scene-core's
// recipe_from_selection (single object / multi-select / single group), which
// round-trips through applyTemplate.

import type { Scene, SceneSelection } from "../../shared/schema";
import type { TemplateContract, TemplateMetadata, TemplateRecipe } from "../../shared/templates/contract";

async function request<T>(url: string, init?: RequestInit): Promise<T> {
  const response = await fetch(url, {
    headers: { "content-type": "application/json" },
    ...init
  });
  if (!response.ok) {
    const error = await response.json().catch(() => ({ message: response.statusText }));
    throw new Error(error.message || response.statusText);
  }
  return (await response.json()) as T;
}

export async function listTemplates(): Promise<TemplateContract[]> {
  const data = await request<{ templates: TemplateContract[] }>("/api/templates");
  return data.templates;
}

export async function createTemplate(contract: TemplateContract): Promise<TemplateContract> {
  const data = await request<{ template: TemplateContract }>("/api/templates", {
    method: "POST",
    body: JSON.stringify(contract)
  });
  return data.template;
}

export async function deleteTemplate(templateId: string): Promise<void> {
  await request<{ deleted: string }>(`/api/templates/${encodeURIComponent(templateId)}`, {
    method: "DELETE"
  });
}

/**
 * Resolve the ids the "save as template" action should capture from a selection:
 * a single group captures the group; a multi-select captures its node set; a
 * single node captures that node. Edge/canvas selections capture nothing.
 */
export function selectionCaptureIds(selection: SceneSelection): { groupId: string | null; nodeIds: string[] } {
  if (selection.kind === "group") return { groupId: selection.id, nodeIds: [] };
  if (selection.kind === "multi") return { groupId: null, nodeIds: selection.ids };
  if (selection.kind === "node") return { groupId: null, nodeIds: [selection.id] };
  return { groupId: null, nodeIds: [] };
}

/**
 * Build a TemplateContract from the current selection (CC3.1 TS mirror of
 * recipe_from_selection). Positions are recorded relative to the selection's
 * bounding-box top-left so applyTemplate re-anchors them anywhere. Returns null
 * when the selection captures no usable objects.
 */
export function recipeFromSelection(scene: Scene, selection: SceneSelection, metadata: TemplateMetadata): TemplateContract | null {
  const { groupId, nodeIds } = selectionCaptureIds(selection);

  // Resolve the captured frames + nodes. A group capture pulls in its members.
  const frameIds = new Set<string>();
  const capturedNodeIds = new Set<string>(nodeIds);
  if (groupId) {
    frameIds.add(groupId);
    for (const node of scene.nodes) if (node.groupId === groupId) capturedNodeIds.add(node.id);
  }
  for (const id of capturedNodeIds) {
    const node = scene.nodes.find((candidate) => candidate.id === id);
    if (node) frameIds.add(node.groupId);
  }
  if (capturedNodeIds.size === 0 && frameIds.size === 0) return null;

  const nodes = scene.nodes.filter((node) => capturedNodeIds.has(node.id));
  const groups = scene.groups.filter((group) => frameIds.has(group.id));
  const edges = scene.edges.filter((edge) => capturedNodeIds.has(edge.source) && capturedNodeIds.has(edge.target));

  // Bounding-box top-left of everything captured → the recipe origin.
  let minX = Infinity;
  let minY = Infinity;
  for (const group of groups) {
    minX = Math.min(minX, group.bounds.x);
    minY = Math.min(minY, group.bounds.y);
  }
  for (const node of nodes) {
    minX = Math.min(minX, node.position.x);
    minY = Math.min(minY, node.position.y);
  }
  if (!Number.isFinite(minX)) minX = 0;
  if (!Number.isFinite(minY)) minY = 0;

  const recipe: TemplateRecipe = {
    frames: groups.map((group) => ({
      localId: group.id,
      title: group.title,
      summary: group.summary,
      parentLocalId: group.parentGroupId && frameIds.has(group.parentGroupId) ? group.parentGroupId : undefined
    })),
    shapes: nodes.map((node) => ({
      localId: node.id,
      frameLocalId: node.groupId,
      title: node.title,
      summary: node.summary,
      detail: node.detail,
      styleKey: node.type,
      position: { x: node.position.x - minX, y: node.position.y - minY },
      size: node.size
    })),
    edges: edges.map((edge) => ({
      localId: edge.id,
      frameLocalId: edge.groupId,
      sourceLocalId: edge.source,
      targetLocalId: edge.target,
      label: edge.label
    }))
  };

  return {
    metadata,
    recipe,
    layout: { origin: { x: 0, y: 0 } },
    exports: { allowed: [] },
    tags: { suggested: [] }
  };
}
