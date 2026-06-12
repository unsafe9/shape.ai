// Data-loss regression: undoing a multi-delete used to restore one object then throw
// "object not found" when a Delete-inverse set-anchor peer restore hard-failed
// against a diverged-out peer. The captured inverse re-inserts deleted objects AND
// restores peer anchors; when a peer is absent the set-anchor restore must no-op so
// the re-insert still commits.

import { beforeAll, describe, expect, it } from "vitest";

import {
  ensureSceneCore,
  loadSceneCore,
  type SceneCore
} from "../platforms/web/bridge/sceneCoreWasm";
import {
  emptyObjectScene,
  IDENTITY_TRANSFORM,
  type Anchor,
  type Object as SceneObject,
  type ObjectOp
} from "../platforms/web/shared/object";

let core: SceneCore;

beforeAll(async () => {
  await ensureSceneCore();
  core = await loadSceneCore();
});

function rect(id: string, order: string, anchors?: Anchor[]): SceneObject {
  return {
    id,
    order,
    transform: IDENTITY_TRANSFORM,
    geometry: { d: "M 0 0 L 80 0 L 80 40 L 0 40 Z", fillRule: "evenOdd" },
    ...(anchors ? { anchors } : {})
  };
}

function anchorTo(target: string): Anchor {
  return { nodeIndex: 0, target, at: { x: 0, y: 0 } };
}

describe("multi-select-delete undo over a divergent scene", () => {
  it("restores the remaining object when a peer is absent at undo time", () => {
    // A anchors B, B anchors A.
    let scene = emptyObjectScene();
    scene = core.applyObjectOp(scene, { kind: "insert-object", object: rect("A", "a0") }).scene;
    scene = core.applyObjectOp(scene, { kind: "insert-object", object: rect("B", "a1") }).scene;
    scene = core.applyObjectOp(scene, { kind: "set-anchor", id: "A", anchors: [anchorTo("B")] }).scene;
    scene = core.applyObjectOp(scene, { kind: "set-anchor", id: "B", anchors: [anchorTo("A")] }).scene;

    // Multi-select delete of both (mirrors App.svelte deleteSelection).
    const del: ObjectOp = { kind: "batch", ops: [{ kind: "delete", id: "A" }, { kind: "delete", id: "B" }] };
    const deleted = core.applyObjectOp(scene, del);
    expect(deleted.errors).toEqual([]);
    expect(deleted.scene.objects).toEqual([]);
    const undoOp = deleted.inverse!;
    expect(undoOp).not.toBeNull();

    // Apply the undo against a scene that has neither A nor B, so the set-anchor
    // peer restores hit both the no-op-owner and filter-target paths.
    const undone = core.applyObjectOp(emptyObjectScene(), undoOp);

    // No error, objects restored: peer restores degrade gracefully instead of
    // aborting the batch.
    expect(undone.errors).toEqual([]);
    const ids = undone.scene.objects.map((o) => o.id).sort();
    expect(ids).toEqual(["A", "B"]);
  });

  it("restores objects even when one peer stays diverged out of the scene", () => {
    // A anchors B, delete only A. The inverse is Batch[insert A, set-anchor B [A]];
    // applied against a scene where B diverged away, A must still come back.
    let scene = emptyObjectScene();
    scene = core.applyObjectOp(scene, { kind: "insert-object", object: rect("A", "a0") }).scene;
    scene = core.applyObjectOp(scene, { kind: "insert-object", object: rect("B", "a1") }).scene;
    scene = core.applyObjectOp(scene, { kind: "set-anchor", id: "B", anchors: [anchorTo("A")] }).scene;

    const deleted = core.applyObjectOp(scene, { kind: "delete", id: "A" });
    expect(deleted.errors).toEqual([]);
    const undoOp = deleted.inverse!;

    let diverged = deleted.scene; // A removed; B still here
    diverged = core.applyObjectOp(diverged, { kind: "delete", id: "B" }).scene; // now B gone too

    const undone = core.applyObjectOp(diverged, undoOp);
    expect(undone.errors).toEqual([]);
    expect(undone.scene.objects.map((o) => o.id)).toContain("A");
  });
});
