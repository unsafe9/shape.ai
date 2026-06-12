// A snapped drag-create synthesizes a persistent anchor binding the created object's
// endpoint to the snap target; the endpoint reprojects as the target moves
// (move-together); an Alt-create (no target) authors no anchor.

import { beforeAll, describe, expect, it } from "vitest";
import { ensureSceneCore, loadSceneCore, type SceneCore } from "../platforms/web/bridge/sceneCoreWasm";
import { type DragSpan } from "../platforms/web/controller/objectPrimitives";
import {
  GEOMETRY_QUANTUM_PER_PX,
  translateTransform,
  type Object as SceneObject,
  type ObjectOp
} from "../platforms/web/shared/object";

const Q = GEOMETRY_QUANTUM_PER_PX;

let core: SceneCore;

beforeAll(async () => {
  await ensureSceneCore();
  core = await loadSceneCore();
});

function targetRect(id: string, tx: number, ty: number): SceneObject {
  return core.buildPrimitiveFromDrag("rectangle", { start: { x: tx, y: ty }, end: { x: tx + 100, y: ty + 60 } }, id, "a0");
}

// World position of geometry node `i` under the object's transform.
function worldNode(obj: SceneObject, i: number): { x: number; y: number } {
  const nums = (obj.geometry.d ?? "").match(/-?\d+(?:\.\d+)?/g)!;
  const lx = Number(nums[i * 2]) / Q;
  const ly = Number(nums[i * 2 + 1]) / Q;
  const t = obj.transform;
  return { x: t ? t[0][0] * lx + t[0][1] * ly + t[0][2] : lx, y: t ? t[1][0] * lx + t[1][1] * ly + t[1][2] : ly };
}

describe("synthesizeCreateAnchors (snapped drag-create binds the endpoint)", () => {
  it("authors one anchor referencing the snap target, addressing the dragged endpoint node", () => {
    const target = targetRect("rect-a", 200, 0); // outline spans world x in [200,300], y in [0,60]
    const span: DragSpan = { start: { x: 40, y: 30 }, end: { x: 200, y: 30 } };
    const line = core.buildPrimitiveFromDrag("line", span, "edge-1", "a1");

    const anchors = core.synthesizeCreateAnchors(line, target, span.end);
    expect(anchors).not.toBeNull();
    expect(anchors!).toHaveLength(1);
    const anchor = anchors![0];
    expect(anchor.target).toBe("rect-a");
    // node 1 is the dragged endpoint (node 0 is the start origin).
    expect(anchor.nodeIndex).toBe(1);
    // `at` is the snapped WORLD point (200,30) in the target's LOCAL quantized space:
    // local (0,30) -> quantized (0, 30*Q).
    expect(anchor.at).toEqual({ x: 0, y: 30 * Q });
  });

  it("authors NO anchor when there is no snap target (Alt-create bypasses snap upstream)", () => {
    // Passing the created object as its own target is the in-core null case (never
    // anchor onto self).
    const span: DragSpan = { start: { x: 40, y: 30 }, end: { x: 200, y: 30 } };
    const line = core.buildPrimitiveFromDrag("line", span, "edge-1", "a1");
    expect(core.synthesizeCreateAnchors(line, line, span.end)).toBeNull();
  });
});

describe("anchorFollowOps (move-together — the commit path's follow ops)", () => {
  it("reprojects the anchored endpoint so it moves WITH the target's transform", () => {
    const target = targetRect("rect-a", 200, 0);
    const span: DragSpan = { start: { x: 40, y: 30 }, end: { x: 200, y: 30 } };
    const line = core.buildPrimitiveFromDrag("line", span, "edge-1", "a1");
    line.anchors = core.synthesizeCreateAnchors(line, target, span.end)!;

    // At rest the bound node sits at the snap world point (200,30).
    expect(worldNode(line, 1)).toEqual({ x: 200, y: 30 });

    // Moving the target reprojects the endpoint to (250,50).
    const scene = { sceneVersion: 1, objects: [target, line], tags: [], selection: { kind: "canvas" as const }, updatedAt: "" };
    const moveOp: ObjectOp = { kind: "set-transform", id: "rect-a", transform: translateTransform(250, 20) };
    const ops = core.anchorFollowOps(scene, [moveOp]);
    expect(ops).toHaveLength(1);
    const op = ops[0];
    if (op.kind !== "edit-geometry") throw new Error("expected edit-geometry");
    expect(op.id).toBe("edge-1");
    const followed: SceneObject = { ...line, geometry: op.geometry };
    expect(worldNode(followed, 1)).toEqual({ x: 250, y: 50 });
  });

  it("a non-anchored endpoint is the falsifying control: it does NOT track the target", () => {
    // Without an anchor the follow authors no op, so the endpoint stays put.
    const target = targetRect("rect-a", 200, 0);
    const span: DragSpan = { start: { x: 40, y: 30 }, end: { x: 200, y: 30 } };
    const altLine = core.buildPrimitiveFromDrag("line", span, "edge-1", "a1");
    expect(altLine.anchors).toBeUndefined();
    const scene = { sceneVersion: 1, objects: [target, altLine], tags: [], selection: { kind: "canvas" as const }, updatedAt: "" };
    const moveOp: ObjectOp = { kind: "set-transform", id: "rect-a", transform: translateTransform(250, 20) };
    expect(core.anchorFollowOps(scene, [moveOp])).toEqual([]);
  });
});
