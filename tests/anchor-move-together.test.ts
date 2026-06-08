// W3-G2 (AP5 follow-up) — live anchor move-together: committing a move of a
// target object must reproject the bound node of every object anchored to it so
// the anchored endpoint tracks the target's delta. AP5 shipped the pure reproject
// (`reprojectAnchoredEndpoint`) and the create-time anchor synthesis, but never
// invoked it on the live target-drag path. These tests pin the wiring unit
// (`anchorFollowOps` / `reprojectAnchoredGeometry`) the transform-commit path now
// calls: they FAIL if a target move does not reproject the anchored geometry.

import { describe, expect, it } from "vitest";
import {
  anchorFollowOps,
  reprojectAnchoredEndpoint,
  reprojectAnchoredGeometry,
  synthesizeCreateAnchors
} from "../src/client/lib/anchorCreate";
import { buildPrimitiveObjectFromDrag, type DragSpan } from "../src/client/lib/objectPrimitives";
import {
  GEOMETRY_QUANTUM_PER_PX,
  translateTransform,
  type Object as SceneObject,
  type ObjectOp,
  type Transform3x3
} from "../src/shared/object";

const Q = GEOMETRY_QUANTUM_PER_PX;

/** A target rectangle at world top-left (tx,ty), 100x60 logical px. */
function targetRect(id: string, tx: number, ty: number): SceneObject {
  return buildPrimitiveObjectFromDrag("rectangle", { start: { x: tx, y: ty }, end: { x: tx + 100, y: ty + 60 } }, id, "a0");
}

/** A line whose end was snapped to the target's left edge, anchored to it. */
function anchoredLine(target: SceneObject): SceneObject {
  const span: DragSpan = { start: { x: 40, y: 30 }, end: { x: 200, y: 30 } };
  const line = buildPrimitiveObjectFromDrag("line", span, "edge-1", "a1");
  line.anchors = synthesizeCreateAnchors(line, target, span.end)!;
  return line;
}

/** The world position of an object's geometry node `i` under its transform. */
function worldNode(obj: SceneObject, i: number): { x: number; y: number } {
  const nums = obj.geometry.d.match(/-?\d+(?:\.\d+)?/g)!;
  const lx = Number(nums[i * 2]) / Q;
  const ly = Number(nums[i * 2 + 1]) / Q;
  const t = obj.transform;
  return { x: t ? t[0][0] * lx + t[0][1] * ly + t[0][2] : lx, y: t ? t[1][0] * lx + t[1][1] * ly + t[1][2] : ly };
}

describe("reprojectAnchoredGeometry (W3-G2 — node reproject in the anchored object's local space)", () => {
  it("rewrites the bound node so its WORLD position tracks the moved target", () => {
    const target = targetRect("rect-a", 200, 0);
    const line = anchoredLine(target);
    // At rest the endpoint resolves to the snap point (200,30).
    expect(worldNode(line, 1)).toEqual({ x: 200, y: 30 });

    // Move the target +50 x / +20 y: the reprojected node must land at (250,50).
    const moved: SceneObject = { ...target, transform: translateTransform(250, 20) };
    const geometry = reprojectAnchoredGeometry(line, moved);
    expect(geometry).toBeDefined();
    const followed: SceneObject = { ...line, geometry: geometry! };
    expect(worldNode(followed, 1)).toEqual(reprojectAnchoredEndpoint(moved, line.anchors![0]));
    expect(worldNode(followed, 1)).toEqual({ x: 250, y: 50 });
    // The un-anchored node 0 (the line's free origin) is untouched.
    expect(worldNode(followed, 0)).toEqual(worldNode(line, 0));
  });

  it("returns undefined when the object is not anchored to the moved target", () => {
    const target = targetRect("rect-a", 200, 0);
    const line = anchoredLine(target);
    const other: SceneObject = { ...targetRect("rect-b", 0, 0), transform: translateTransform(500, 500) };
    expect(reprojectAnchoredGeometry(line, other)).toBeUndefined();
  });
});

describe("anchorFollowOps (W3-G2 — the commit path's follow ops)", () => {
  // Mirror App.svelte's onTransformCommit: a set-transform op carrying the moved
  // object's NEW transform is the input to anchorFollowOps.
  function moveOp(id: string, transform: Transform3x3): ObjectOp {
    return { kind: "set-transform", id, transform };
  }

  it("authors an edit-geometry that reprojects the endpoint of an object anchored to the moved target", () => {
    const target = targetRect("rect-a", 200, 0);
    const line = anchoredLine(target);
    const objects = [target, line];

    const ops = anchorFollowOps(objects, [moveOp("rect-a", translateTransform(250, 20))]);
    expect(ops).toHaveLength(1);
    const op = ops[0];
    expect(op.kind).toBe("edit-geometry");
    if (op.kind !== "edit-geometry") throw new Error("unreachable");
    expect(op.id).toBe("edge-1");
    // The reprojected geometry's bound node must sit at the target's new world point.
    const followed: SceneObject = { ...line, geometry: op.geometry };
    expect(worldNode(followed, 1)).toEqual({ x: 250, y: 50 });
  });

  it("leaves an unrelated object untouched (it is not anchored to the moved target)", () => {
    const target = targetRect("rect-a", 200, 0);
    const line = anchoredLine(target);
    const unrelated = targetRect("rect-c", -300, -300); // no anchors at all
    const objects = [target, line, unrelated];

    const ops = anchorFollowOps(objects, [moveOp("rect-a", translateTransform(250, 20))]);
    expect(ops.map((o) => (o.kind === "edit-geometry" ? o.id : ""))).toEqual(["edge-1"]);
  });

  it("an Alt-created (unanchored) object does NOT move when its would-be target moves", () => {
    const target = targetRect("rect-a", 200, 0);
    // Same drag, but the snap was bypassed (Alt-create) — no anchor synthesized.
    const altLine = buildPrimitiveObjectFromDrag("line", { start: { x: 40, y: 30 }, end: { x: 200, y: 30 } }, "edge-1", "a1");
    expect(altLine.anchors).toBeUndefined();
    const objects = [target, altLine];

    const ops = anchorFollowOps(objects, [moveOp("rect-a", translateTransform(250, 20))]);
    expect(ops).toEqual([]);
  });

  it("a move of an object that nothing is anchored to authors no follow ops (the no-op case)", () => {
    const target = targetRect("rect-a", 200, 0);
    const line = anchoredLine(target);
    const objects = [target, line];
    // Move the LINE itself (nothing is anchored to it) — no follow ops.
    const ops = anchorFollowOps(objects, [moveOp("edge-1", translateTransform(10, 10))]);
    expect(ops).toEqual([]);
  });
});
