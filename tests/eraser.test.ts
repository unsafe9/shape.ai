// W2-08 — the eraser, exercised through the same scene-core the live app runs.
//
// Two modes (D4):
//   (1) whole-stroke delete: erasing a stroke authors a `delete` op whose inverse
//       re-inserts it (D21) — proven via apply_object_op;
//   (2) partial erase: splitSubpathAt cuts the touched node out of a stroke's
//       subpath, yielding two open subpaths; the cut geometry then rides an
//       `edit-geometry` op (the shell's partial-erase path).

import { beforeAll, describe, expect, it } from "vitest";

import { ensureSceneCore, loadSceneCore, type SceneCore } from "../src/client/scene/sceneCoreWasm";
import { GEOMETRY_QUANTUM_PER_PX, emptyObjectScene, type Object as SceneObject } from "../src/shared/object";

let core: SceneCore;

const PEN = { color: "#1f2933", widthPx: 2 };
// A zigzag stroke: the interior peaks deviate off any chord, so RDP keeps them
// (a straight polyline would simplify to its 2 endpoints, leaving no interior
// node to cut). Origin is min(x, y) over the points; geometry is object-local.
const STROKE_POINTS = [
  { x: 0, y: 0 },
  { x: 40, y: 40 },
  { x: 80, y: 0 },
  { x: 120, y: 40 },
  { x: 160, y: 0 }
];
const ORIGIN = { x: 0, y: 0 };

beforeAll(async () => {
  await ensureSceneCore();
  core = await loadSceneCore();
});

describe("(1) whole-stroke delete authors a delete op with an inverse re-insert", () => {
  it("removes the stroke and captures the inverse that restores it (D21)", () => {
    const stroke = core.freehandToObject(STROKE_POINTS, PEN.color, PEN.widthPx, "draw-1", "a0", "free");
    const scene = { ...emptyObjectScene(), objects: [stroke] };

    const deleted = core.applyObjectOp(scene, { kind: "delete", id: "draw-1" });
    expect(deleted.errors).toEqual([]);
    expect(deleted.scene.objects.map((o: SceneObject) => o.id)).toEqual([]);
    // The inverse re-inserts the exact stroke (whole-stroke erase is undoable).
    expect(deleted.inverse?.kind).toBe("insert-object");

    const restored = core.applyObjectOp(deleted.scene, deleted.inverse!);
    expect(restored.errors).toEqual([]);
    expect(restored.scene.objects.map((o: SceneObject) => o.id)).toEqual(["draw-1"]);
  });
});

describe("(2) partial erase cuts the touched subpath into two open pieces", () => {
  it("splits the stroke geometry at the touched node (object-local quantized)", () => {
    const stroke = core.freehandToObject(STROKE_POINTS, PEN.color, PEN.widthPx, "draw-1", "a0", "free");
    // The middle sample (80,0 world) maps to object-local (80 - origin.x, 0 -
    // origin.y) px, then quantized. A generous radius tolerates RDP/bezier-fit
    // nudging the interior node slightly.
    const touchLocalX = (80 - ORIGIN.x) * GEOMETRY_QUANTUM_PER_PX;
    const touchLocalY = (0 - ORIGIN.y) * GEOMETRY_QUANTUM_PER_PX;
    const radius = 24 * GEOMETRY_QUANTUM_PER_PX;

    const cut = core.splitSubpathAt(stroke.geometry, touchLocalX, touchLocalY, radius);
    expect(cut).not.toBeNull();
    // Two `M` subpaths (a split), and both open (no trailing Z).
    expect((cut!.d.match(/M/g) ?? []).length).toBe(2);
    expect(cut!.d.includes("Z")).toBe(false);

    // The cut geometry rides an edit-geometry op (the shell's partial-erase path).
    const scene = { ...emptyObjectScene(), objects: [stroke] };
    const edited = core.applyObjectOp(scene, { kind: "edit-geometry", id: "draw-1", geometry: cut! });
    expect(edited.errors).toEqual([]);
    expect(edited.scene.objects[0].geometry.d).toBe(cut!.d);
    // The inverse restores the original (uncut) geometry — partial erase is undoable.
    expect(edited.inverse?.kind).toBe("edit-geometry");
  });

  it("returns null when the touch misses every stroke node (no-op)", () => {
    const stroke = core.freehandToObject(STROKE_POINTS, PEN.color, PEN.widthPx, "draw-1", "a0", "free");
    const cut = core.splitSubpathAt(stroke.geometry, 9999, 9999, 12 * GEOMETRY_QUANTUM_PER_PX);
    expect(cut).toBeNull();
  });
});
