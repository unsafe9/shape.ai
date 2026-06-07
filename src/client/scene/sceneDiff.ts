// Scene → ScenePatch diff for the shell's legacy HTTP-fallback save path.
//
// The scene-core wasm op-apply returns the next Scene (the same logic the server
// runs), not the legacy `appPatch` ScenePatch diff the old TS apply produced. The
// HTTP fallback's `patchSaver` still wants a ScenePatch to PATCH /api/scene, so
// this derives one by diffing the previous scene against the wasm-applied scene:
// added/changed groups/nodes/edges plus removed ids. The server re-applies the
// patch authoritatively, so this only needs to carry the delta accurately.

import type { Scene, ScenePatch, SceneEdge, SceneGroup, SceneNode, SceneSelection } from "../../shared/schema";

function changed<T extends { id: string }>(prev: T[], next: T[]): T[] {
  const prevById = new Map(prev.map((item) => [item.id, item]));
  const out: T[] = [];
  for (const item of next) {
    const before = prevById.get(item.id);
    // Reference inequality is sufficient: the wasm result is freshly parsed JSON,
    // so an unchanged object still differs by reference. Fall back to a value
    // compare to avoid sending objects that are structurally identical.
    if (!before || !sameJson(before, item)) out.push(item);
  }
  return out;
}

function removedIds<T extends { id: string }>(prev: T[], next: T[]): string[] {
  const nextIds = new Set(next.map((item) => item.id));
  return prev.filter((item) => !nextIds.has(item.id)).map((item) => item.id);
}

function sameJson(a: unknown, b: unknown): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/**
 * Diff `prev` against the wasm-applied `next` into a minimal {@link ScenePatch}
 * carrying only the changed/added objects, removed ids, and the next selection.
 */
export function scenePatchFromScenes(prev: Scene, next: Scene): ScenePatch {
  const groups: SceneGroup[] = changed(prev.groups, next.groups);
  const nodes: SceneNode[] = changed(prev.nodes, next.nodes);
  const edges: SceneEdge[] = changed(prev.edges, next.edges);
  const removeGroupIds = removedIds(prev.groups, next.groups);
  const removeNodeIds = removedIds(prev.nodes, next.nodes);
  const removeEdgeIds = removedIds(prev.edges, next.edges);
  const selection: SceneSelection | undefined = sameJson(prev.selection, next.selection)
    ? undefined
    : next.selection;
  return {
    ...(groups.length > 0 ? { groups } : {}),
    ...(nodes.length > 0 ? { nodes } : {}),
    ...(edges.length > 0 ? { edges } : {}),
    ...(removeGroupIds.length > 0 ? { removeGroupIds } : {}),
    ...(removeNodeIds.length > 0 ? { removeNodeIds } : {}),
    ...(removeEdgeIds.length > 0 ? { removeEdgeIds } : {}),
    ...(selection ? { selection } : {})
  };
}
