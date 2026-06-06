// T6.2 §1: framework-neutral renderer-patch save orchestration.
//
// This is the "commit on gesture-end, debounce otherwise" flow that lived in
// App.tsx (rendererPatchSaveRef / pendingRendererPatchSaveRef /
// rendererPatchSaveTimerRef / rendererGestureActiveRef). It is pure
// orchestration — debounce + merge + last-write-wins guard + optimistic
// rollback — with no canvas math and no framework dependency. Both the React
// shell and the Svelte shell can drive it.

import { saveScenePatch } from "./api";
import type { RenderScenePatch } from "../../shared/renderPatch";
import type { Scene, ScenePatch, SceneSelection } from "../../shared/schema";

const rendererPatchSaveDebounceMs = 80;

export type PendingRendererPatchSave = {
  patch: ScenePatch;
  rollbackScene: Scene;
  rollbackSelection: SceneSelection;
  patchKind: RenderScenePatch["kind"];
};

export type PatchSaverCallbacks = {
  /** Apply a freshly-saved server scene (last-write-wins, only when no gesture is active). */
  onSaved: (scene: Scene, selection: SceneSelection) => void;
  /** Restore the rollback scene + report an error message after a failed save. */
  onError: (rollbackScene: Scene, rollbackSelection: SceneSelection, message: string) => void;
  /** Whether a pointer gesture is currently active (suppresses response-scene application). */
  isGestureActive: () => boolean;
};

export type PatchSaver = {
  /** Queue a continuous patch (move-group/move-card) for debounced or gesture-gated save. */
  queue: (
    patch: ScenePatch,
    rollbackScene: Scene,
    rollbackSelection: SceneSelection,
    patchKind: RenderScenePatch["kind"],
    waitForGestureEnd?: boolean
  ) => void;
  /** Flush any pending save immediately (e.g. on gesture end). */
  flush: () => void;
  /** Save a discrete patch right now, applying the response scene. */
  saveNow: (
    patch: ScenePatch,
    rollbackScene: Scene,
    rollbackSelection: SceneSelection,
    patchKind: RenderScenePatch["kind"]
  ) => void;
  /** Clear any pending timer (call on teardown). */
  dispose: () => void;
};

export function createPatchSaver(callbacks: PatchSaverCallbacks): PatchSaver {
  let saveId = 0;
  let pending: PendingRendererPatchSave | null = null;
  let timer: number | null = null;

  function queue(
    patch: ScenePatch,
    rollbackScene: Scene,
    rollbackSelection: SceneSelection,
    patchKind: RenderScenePatch["kind"],
    waitForGestureEnd = false
  ) {
    pending = {
      patch: pending ? mergeScenePatches(pending.patch, patch) : patch,
      rollbackScene: pending?.rollbackScene ?? rollbackScene,
      rollbackSelection: pending?.rollbackSelection ?? rollbackSelection,
      patchKind
    };
    if (timer !== null) window.clearTimeout(timer);
    if (waitForGestureEnd) {
      timer = null;
      return;
    }
    timer = window.setTimeout(flush, rendererPatchSaveDebounceMs);
  }

  function flush() {
    const next = pending;
    if (!next) return;
    pending = null;
    if (timer !== null) {
      window.clearTimeout(timer);
      timer = null;
    }
    save(next.patch, next.rollbackScene, next.rollbackSelection, next.patchKind, false);
  }

  function saveNow(
    patch: ScenePatch,
    rollbackScene: Scene,
    rollbackSelection: SceneSelection,
    patchKind: RenderScenePatch["kind"]
  ) {
    save(patch, rollbackScene, rollbackSelection, patchKind, true);
  }

  function save(
    patch: ScenePatch,
    rollbackScene: Scene,
    rollbackSelection: SceneSelection,
    patchKind: RenderScenePatch["kind"],
    applyResponseScene: boolean
  ) {
    const id = ++saveId;
    void saveScenePatch(patch)
      .then((response) => {
        if (id !== saveId) return;
        if (!applyResponseScene) return;
        if (callbacks.isGestureActive()) return;
        callbacks.onSaved(response.scene, response.scene.selection);
      })
      .catch((error) => {
        if (id !== saveId) return;
        const message = error instanceof Error ? error.message : `Renderer patch save failed: ${patchKind}`;
        callbacks.onError(rollbackScene, rollbackSelection, message);
      });
  }

  function dispose() {
    if (timer !== null) window.clearTimeout(timer);
    timer = null;
    pending = null;
  }

  return { queue, flush, saveNow, dispose };
}

// ---------------------------------------------------------------------------
// Patch merge helpers (moved verbatim from App.tsx — pure ScenePatch algebra).
// ---------------------------------------------------------------------------

export function mergeScenePatches(left: ScenePatch, right: ScenePatch): ScenePatch {
  const groups = mergeById(left.groups, right.groups);
  const nodes = mergeById(left.nodes, right.nodes);
  const edges = mergeById(left.edges, right.edges);
  const translateGroups = mergeTranslateGroups(left.translateGroups, right.translateGroups);
  const removeGroupIds = mergeUnique(left.removeGroupIds, right.removeGroupIds);
  const removeNodeIds = mergeUnique(left.removeNodeIds, right.removeNodeIds);
  const removeEdgeIds = mergeUnique(left.removeEdgeIds, right.removeEdgeIds);
  return {
    ...(groups.length > 0 ? { groups } : {}),
    ...(nodes.length > 0 ? { nodes } : {}),
    ...(edges.length > 0 ? { edges } : {}),
    ...(translateGroups.length > 0 ? { translateGroups } : {}),
    ...(removeGroupIds.length > 0 ? { removeGroupIds } : {}),
    ...(removeNodeIds.length > 0 ? { removeNodeIds } : {}),
    ...(removeEdgeIds.length > 0 ? { removeEdgeIds } : {}),
    ...(right.selection ? { selection: right.selection } : left.selection ? { selection: left.selection } : {})
  };
}

function mergeById<T extends { id: string }>(left: T[] | undefined, right: T[] | undefined): T[] {
  const items = new Map<string, T>();
  for (const item of left ?? []) items.set(item.id, item);
  for (const item of right ?? []) items.set(item.id, item);
  return Array.from(items.values());
}

function mergeTranslateGroups(
  left: ScenePatch["translateGroups"] | undefined,
  right: ScenePatch["translateGroups"] | undefined
): NonNullable<ScenePatch["translateGroups"]> {
  const movements = new Map<string, { groupId: string; dx: number; dy: number }>();
  for (const movement of [...(left ?? []), ...(right ?? [])]) {
    const current = movements.get(movement.groupId);
    movements.set(movement.groupId, {
      groupId: movement.groupId,
      dx: (current?.dx ?? 0) + movement.dx,
      dy: (current?.dy ?? 0) + movement.dy
    });
  }
  return Array.from(movements.values());
}

function mergeUnique(left: string[] | undefined, right: string[] | undefined): string[] {
  return Array.from(new Set([...(left ?? []), ...(right ?? [])]));
}

export function isContinuousRendererPatch(patch: RenderScenePatch): boolean {
  return patch.kind === "move-group" || patch.kind === "move-card";
}
