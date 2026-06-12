// Multi-stroke endpoint merge. The freehand release branches merge-first: when
// mergeOpenStrokeOps returns ops, the stroke chains into the matched open-class
// object (edit-geometry batch on the survivor, no insert, anchors skipped); when it
// returns null, the existing insert + release-anchoring path runs. Acceptance: a
// rect drawn in three strokes ends as ONE closed rect object.

import { beforeAll, describe, expect, it } from "vitest";
import { ensureSceneCore, loadSceneCore, type SceneCore } from "../bridge/sceneCoreWasm";
import { MERGE_ENDPOINT_TOLERANCE_PX, synthesizeReleaseAnchors } from "../controller/objectPrimitives";
import { GEOMETRY_QUANTUM_PER_PX, type Object as SceneObject, type ObjectOp, type ObjectScene } from "../shared/object";

const Q = GEOMETRY_QUANTUM_PER_PX;

let core: SceneCore;

beforeAll(async () => {
  await ensureSceneCore();
  core = await loadSceneCore();
});

const PEN = { color: "#1f2933", widthPx: 2 };

function sceneOf(objects: SceneObject[]): ObjectScene {
  return { sceneVersion: 1, objects, tags: [], selection: { kind: "canvas" }, updatedAt: "" };
}

// n+1 samples along the segment a -> b (inclusive).
function strokeAlong(a: { x: number; y: number }, b: { x: number; y: number }, n = 10): { x: number; y: number }[] {
  return Array.from({ length: n + 1 }, (_, i) => ({
    x: a.x + ((b.x - a.x) * i) / n,
    y: a.y + ((b.y - a.y) * i) / n
  }));
}

// Apply the merge ops the shell would author (single op or one batch).
function applyMerge(scene: ObjectScene, ops: ObjectOp[]): ObjectScene {
  const op: ObjectOp = ops.length === 1 ? ops[0] : { kind: "batch", ops };
  const result = core.applyObjectOp(scene, op);
  expect(result.errors).toEqual([]);
  return result.scene;
}

describe("freehand release merges into a nearby open endpoint (merge-first branch)", () => {
  it("a stroke released ON an open object's endpoint authors an edit-geometry batch — no insert, anchors skipped", () => {
    const left = core.freehandToObject(strokeAlong({ x: 300, y: 200 }, { x: 300, y: 100 }), PEN.color, PEN.widthPx, "draw-1", "a1", "basic");
    const scene = sceneOf([left]);
    // Stroke 2 starts exactly on the vertical's top end.
    const gamma = [
      ...strokeAlong({ x: 300, y: 100 }, { x: 400, y: 100 }),
      ...strokeAlong({ x: 400, y: 100 }, { x: 400, y: 200 }).slice(1)
    ];
    const ops = core.mergeOpenStrokeOps(scene, gamma, "basic", MERGE_ENDPOINT_TOLERANCE_PX);
    expect(ops).not.toBeNull();
    expect(ops!.some((op) => op.kind === "insert-object")).toBe(false);
    expect(ops![0].kind).toBe("edit-geometry");
    if (ops![0].kind !== "edit-geometry") throw new Error("unreachable");
    expect(ops![0].id).toBe("draw-1");
    // The survivor's new path is the chained shape, one OPEN path, corners kept.
    expect(ops![0].geometry.d).toBe(`M 0 ${100 * Q} L 0 0 L ${100 * Q} 0 L ${100 * Q} ${100 * Q}`);
    expect(core.isOpenClassD(ops![0].geometry.d ?? "")).toBe(true);
  });

  it("THE acceptance: three strokes — vertical, ㄱ, bottom bar — close into ONE rect object", () => {
    const left = core.freehandToObject(strokeAlong({ x: 300, y: 200 }, { x: 300, y: 100 }), PEN.color, PEN.widthPx, "draw-1", "a1", "basic");
    let scene = sceneOf([left]);
    const gamma = [
      ...strokeAlong({ x: 300, y: 100 }, { x: 400, y: 100 }),
      ...strokeAlong({ x: 400, y: 100 }, { x: 400, y: 200 }).slice(1)
    ];
    scene = applyMerge(scene, core.mergeOpenStrokeOps(scene, gamma, "basic", MERGE_ENDPOINT_TOLERANCE_PX)!);
    // Stroke 3 lands on BOTH endpoints of the chained shape.
    const bar = strokeAlong({ x: 300, y: 200 }, { x: 400, y: 200 });
    const ops = core.mergeOpenStrokeOps(scene, bar, "basic", MERGE_ENDPOINT_TOLERANCE_PX);
    expect(ops).not.toBeNull();
    scene = applyMerge(scene, ops!);
    // One object, closed, Basic re-recognition snapped it to the rect.
    expect(scene.objects).toHaveLength(1);
    expect(scene.objects[0].id).toBe("draw-1");
    expect(scene.objects[0].geometry.d).toBe(`M 0 0 L ${100 * Q} 0 L ${100 * Q} ${100 * Q} L 0 ${100 * Q} Z`);
    expect(core.isOpenClassD(scene.objects[0].geometry.d ?? "")).toBe(false);
  });

  it("a release away from every open endpoint returns null — the existing insert + anchor path runs", () => {
    const left = core.freehandToObject(strokeAlong({ x: 300, y: 200 }, { x: 300, y: 100 }), PEN.color, PEN.widthPx, "draw-1", "a1", "basic");
    // Closed-class is never a merge candidate, but it IS a release-anchor target.
    const rectB = core.buildPrimitiveFromDrag("rectangle", { start: { x: 500, y: 0 }, end: { x: 600, y: 60 } }, "rect-b", "a0");
    const objects = [left, rectB];
    const stroke = strokeAlong({ x: 350, y: 150 }, { x: 500, y: 30 });
    const ops = core.mergeOpenStrokeOps(sceneOf(objects), stroke, "basic", MERGE_ENDPOINT_TOLERANCE_PX);
    expect(ops).toBeNull();
    // Fall-through: recognize + insert + release anchors.
    const object = core.freehandToObject(stroke, PEN.color, PEN.widthPx, "draw-2", "a2", "basic");
    expect(core.isOpenClassD(object.geometry.d ?? "")).toBe(true);
    const anchors = synthesizeReleaseAnchors(core, objects, object, [
      null,
      { target: "rect-b", at: { x: 500, y: 30 } }
    ]);
    expect(anchors).toHaveLength(1);
    expect(anchors[0].target).toBe("rect-b");
    expect(anchors[0].nodeIndex).toBe(1);
  });
});
