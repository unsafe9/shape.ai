// G11 — multi-select-delete undo over a windowed/divergent scene, driven through
// the REAL scene-core wasm core (`apply_object_op`). No TS op-apply here: this is
// the data-loss regression for the bug where undoing a multi-delete restored ~one
// object then threw "object not found" because a Delete-inverse set-anchor peer
// restore hard-failed when the peer had diverged out of the local scene.
//
// Mirrors the App.svelte multi-select delete: `{ kind: "batch", ops: ids.map(id
// => ({ kind: "delete", id })) }`. The captured inverse re-inserts the deleted
// objects AND restores peer anchors via set-anchor; when a peer is ABSENT at undo
// time the set-anchor restore must no-op (best-effort) so the re-insert commits.

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

describe("multi-select-delete undo over a divergent scene (G11)", () => {
  it("restores the remaining object when a peer is absent at undo time", () => {
    // Two mutually-anchored objects: A anchors B, B anchors A.
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

    // Divergence: rebuild the post-delete scene but with B re-inserted only — i.e.
    // when undo runs, only ONE of the two objects is present to receive its peer
    // set-anchor restore. We drop A entirely so its set-anchor restore (owner A,
    // and B's set-anchor restore targeting A) exercises both no-op-owner and
    // filter-target paths. Apply the undo against a scene that has neither A nor B.
    const undone = core.applyObjectOp(emptyObjectScene(), undoOp);

    // The fix: NO error, and the deleted objects are restored — the set-anchor
    // peer restores degrade gracefully instead of aborting the whole batch.
    expect(undone.errors).toEqual([]);
    const ids = undone.scene.objects.map((o) => o.id).sort();
    expect(ids).toEqual(["A", "B"]);
  });

  it("restores objects even when one peer stays diverged out of the scene", () => {
    // Closer to the live bug: A anchors B, then delete only A. The inverse is a
    // Batch[insert A, set-anchor B [target A]]. Apply it against a scene where B
    // has diverged away (windowed load / applyRemote echo): A must still come back.
    let scene = emptyObjectScene();
    scene = core.applyObjectOp(scene, { kind: "insert-object", object: rect("A", "a0") }).scene;
    scene = core.applyObjectOp(scene, { kind: "insert-object", object: rect("B", "a1") }).scene;
    scene = core.applyObjectOp(scene, { kind: "set-anchor", id: "B", anchors: [anchorTo("A")] }).scene;

    const deleted = core.applyObjectOp(scene, { kind: "delete", id: "A" });
    expect(deleted.errors).toEqual([]);
    const undoOp = deleted.inverse!;

    // Build a divergent scene: only B's deletion happened elsewhere — B is gone.
    let diverged = deleted.scene; // A removed; B still here
    diverged = core.applyObjectOp(diverged, { kind: "delete", id: "B" }).scene; // now B gone too

    const undone = core.applyObjectOp(diverged, undoOp);
    expect(undone.errors).toEqual([]);
    expect(undone.scene.objects.map((o) => o.id)).toContain("A");
  });
});
