// AP5 (#14) — drag-create anchoring, driven through the scene-core wasm core.
// A snapped drag-create synthesizes a persistent D5 anchor binding the created
// object's endpoint to the snap target; the endpoint reprojects as the target
// moves (move-together via the commit-time follow ops); and an Alt-create (snap
// bypassed upstream, so no target) authors no anchor. The canvas logic now lives
// in the Rust core — these are contract tests over the SAME wasm the server runs,
// like object-op-apply.test.ts.

import { beforeAll, describe, expect, it } from "vitest";
import { ensureSceneCore, loadSceneCore, type SceneCore } from "../src/client/scene/sceneCoreWasm";
import { buildPrimitiveObjectFromDrag, type DragSpan } from "../src/client/lib/objectPrimitives";
import {
  GEOMETRY_QUANTUM_PER_PX,
  translateTransform,
  type Object as SceneObject,
  type ObjectOp
} from "../src/shared/object";

const Q = GEOMETRY_QUANTUM_PER_PX;

let core: SceneCore;

beforeAll(async () => {
  await ensureSceneCore();
  core = await loadSceneCore();
});

/** A target rectangle at world top-left (tx,ty), 100x60 logical px. */
function targetRect(id: string, tx: number, ty: number): SceneObject {
  return buildPrimitiveObjectFromDrag("rectangle", { start: { x: tx, y: ty }, end: { x: tx + 100, y: ty + 60 } }, id, "a0");
}

/** The world position of an object's geometry node `i` under its transform. */
function worldNode(obj: SceneObject, i: number): { x: number; y: number } {
  const nums = obj.geometry.d.match(/-?\d+(?:\.\d+)?/g)!;
  const lx = Number(nums[i * 2]) / Q;
  const ly = Number(nums[i * 2 + 1]) / Q;
  const t = obj.transform;
  return { x: t ? t[0][0] * lx + t[0][1] * ly + t[0][2] : lx, y: t ? t[1][0] * lx + t[1][1] * ly + t[1][2] : ly };
}

describe("synthesizeCreateAnchors (AP5 snapped drag-create binds the endpoint)", () => {
  it("authors one anchor referencing the snap target, addressing the dragged endpoint node", () => {
    const target = targetRect("rect-a", 200, 0); // outline spans world x∈[200,300], y∈[0,60]
    // A line dragged from (40,30) to the target's left edge (200,30): the corner snapped.
    const span: DragSpan = { start: { x: 40, y: 30 }, end: { x: 200, y: 30 } };
    const line = buildPrimitiveObjectFromDrag("line", span, "edge-1", "a1");

    const anchors = core.synthesizeCreateAnchors(line, target, span.end);
    expect(anchors).not.toBeNull();
    expect(anchors!).toHaveLength(1);
    const anchor = anchors![0];
    expect(anchor.target).toBe("rect-a");
    // The line's node 1 is the dragged endpoint (node 0 is the start origin).
    expect(anchor.nodeIndex).toBe(1);
    // `at` is the snapped WORLD point (200,30) in the target's LOCAL quantized space
    // (target translation is (200,0)): local (0,30) -> quantized (0, 30*Q).
    expect(anchor.at).toEqual({ x: 0, y: 30 * Q });
  });

  it("authors NO anchor when there is no snap target (Alt-create bypasses snap upstream)", () => {
    // An Alt-create reports targetId=null upstream, so the shell never calls
    // synthesize; passing the created object as its own target is the in-core null
    // case (never anchor onto self).
    const span: DragSpan = { start: { x: 40, y: 30 }, end: { x: 200, y: 30 } };
    const line = buildPrimitiveObjectFromDrag("line", span, "edge-1", "a1");
    expect(core.synthesizeCreateAnchors(line, line, span.end)).toBeNull();
  });
});

describe("anchorFollowOps (AP5 move-together — the commit path's follow ops)", () => {
  it("reprojects the anchored endpoint so it moves WITH the target's transform", () => {
    const target = targetRect("rect-a", 200, 0);
    const span: DragSpan = { start: { x: 40, y: 30 }, end: { x: 200, y: 30 } };
    const line = buildPrimitiveObjectFromDrag("line", span, "edge-1", "a1");
    line.anchors = core.synthesizeCreateAnchors(line, target, span.end)!;

    // At rest the bound node sits at the snap world point (200,30).
    expect(worldNode(line, 1)).toEqual({ x: 200, y: 30 });

    // Move the target +50 x / +20 y: the follow op reprojects the endpoint to (250,50).
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
    // Without an anchor the line's endpoint is its fixed object-local geometry under
    // its own transform — the follow authors no op, so it stays put when the
    // (would-be) target moves. This is the behavior the anchor must override.
    const target = targetRect("rect-a", 200, 0);
    const span: DragSpan = { start: { x: 40, y: 30 }, end: { x: 200, y: 30 } };
    const altLine = buildPrimitiveObjectFromDrag("line", span, "edge-1", "a1");
    expect(altLine.anchors).toBeUndefined();
    const scene = { sceneVersion: 1, objects: [target, altLine], tags: [], selection: { kind: "canvas" as const }, updatedAt: "" };
    const moveOp: ObjectOp = { kind: "set-transform", id: "rect-a", transform: translateTransform(250, 20) };
    expect(core.anchorFollowOps(scene, [moveOp])).toEqual([]);
  });
});
