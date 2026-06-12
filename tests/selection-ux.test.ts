// AP2 (#10/#7/#15) — selection UX. Pins, at the code level (no renderer / no Svelte
// mount):
//  (a) additive modifier-click accumulates a multi-select and a re-click removes
//      (the pure `toggleObjectSelection`), and App.svelte routes the engine's C2
//      additive flag through it;
//  (b) the marquee ids (RA2a) are applied to the selection;
//  (c) a parent/Multi drag cascades the world-space delta to its descendants via
//      the scene-core `moveOps` core call — the delta reaches children (and
//      grandchildren), with the multi-union deduped.

import { beforeAll, describe, expect, it } from "vitest";
import {
  emptyObjectScene,
  toggleObjectSelection,
  type Object as SceneObject,
  type ObjectOp,
  type ObjectScene,
  type ObjectSelection
} from "../platforms/web/shared/object";
import { loadSceneCore, type SceneCore } from "../platforms/web/bridge/sceneCoreWasm";
import { commitBodyDrag, routeMarquee, routeSelectObject } from "../platforms/web/controller/interactions";

describe("toggleObjectSelection (additive modifier-click, #10)", () => {
  it("accumulates a multi-select as ids are modifier-clicked in", () => {
    const a = toggleObjectSelection({ kind: "canvas" }, "o1");
    expect(a).toEqual({ kind: "object", id: "o1" });
    const b = toggleObjectSelection(a, "o2");
    expect(b).toEqual({ kind: "multi", ids: ["o1", "o2"] });
    const c = toggleObjectSelection(b, "o3");
    expect(c).toEqual({ kind: "multi", ids: ["o1", "o2", "o3"] });
  });

  it("removes an already-selected id on re-click, collapsing the kind", () => {
    const multi: ObjectSelection = { kind: "multi", ids: ["o1", "o2", "o3"] };
    // Re-clicking the middle id drops it but keeps the multi.
    expect(toggleObjectSelection(multi, "o2")).toEqual({ kind: "multi", ids: ["o1", "o3"] });
    // Down to one id collapses to a single-object selection.
    expect(toggleObjectSelection({ kind: "multi", ids: ["o1", "o2"] }, "o2")).toEqual({
      kind: "object",
      id: "o1"
    });
    // Re-clicking the only selected object clears to canvas.
    expect(toggleObjectSelection({ kind: "object", id: "o1" }, "o1")).toEqual({ kind: "canvas" });
  });
});

describe("onSelectObject routing (W3-G5 #10 — no collapse-on-drag)", () => {
  it("an additive pick over an existing single selection yields a Multi of both ids", () => {
    expect(routeSelectObject({ kind: "object", id: "o1" }, "o2", true)).toEqual({
      kind: "multi",
      ids: ["o1", "o2"]
    });
  });

  it("a second additive pick on a member removes it (and collapses when one remains)", () => {
    const multi: ObjectSelection = { kind: "multi", ids: ["o1", "o2"] };
    expect(routeSelectObject(multi, "o1", true)).toEqual({ kind: "object", id: "o2" });
  });

  it("a PLAIN pick on a member of the Multi KEEPS the whole Multi (so a group-drag never collapses)", () => {
    const multi: ObjectSelection = { kind: "multi", ids: ["o1", "o2", "o3"] };
    // Falsifiable: the pre-fix code returned { kind:"object", id:"o2" } here, which
    // is exactly the collapse that broke multi-drag.
    expect(routeSelectObject(multi, "o2", false)).toBe(multi);
  });

  it("a plain pick OUTSIDE the Multi replaces it with the single object", () => {
    const multi: ObjectSelection = { kind: "multi", ids: ["o1", "o2"] };
    expect(routeSelectObject(multi, "o9", false)).toEqual({ kind: "object", id: "o9" });
  });

  it("a plain pick on a single selection replaces it (single-select unchanged)", () => {
    expect(routeSelectObject({ kind: "object", id: "o1" }, "o2", false)).toEqual({ kind: "object", id: "o2" });
  });
});

// A translation delta (move by (dx,dy) in world space): the same matrix the
// renderer hands the commit for a pure drag.
function translateDelta(dx: number, dy: number): [[number, number, number], [number, number, number], [number, number, number]] {
  return [
    [1, 0, dx],
    [0, 1, dy],
    [0, 0, 1]
  ];
}

function obj(id: string, parent: string | undefined, tx: number, ty: number): SceneObject {
  return {
    id,
    ...(parent ? { parent } : {}),
    order: "a0",
    transform: [
      [1, 0, tx],
      [0, 1, ty],
      [0, 0, 1]
    ],
    geometry: { d: "M 0 0 L 8 0" }
  } as SceneObject;
}

function sceneOf(objects: SceneObject[]): ObjectScene {
  return { ...emptyObjectScene(), objects };
}

/** The ordered set-transform ids of a move-ops batch. */
function setTransformIds(ops: ObjectOp[]): string[] {
  return ops.filter((o) => o.kind === "set-transform").map((o) => (o.kind === "set-transform" ? o.id : ""));
}

/** The composed `(x, y)` translate of the set-transform op for `id`. */
function originOf(ops: ObjectOp[], id: string): [number, number] {
  const op = ops.find((o) => o.kind === "set-transform" && o.id === id);
  if (op?.kind !== "set-transform") throw new Error(`expected set-transform for ${id}`);
  return [op.transform[0][2], op.transform[1][2]];
}

// Tier-2: the cascade now lives in scene-core; these are CONTRACT tests over the
// REAL wasm `sceneCore.moveOps` (single/multi roots), one-to-one with the deleted
// `transformCascade.ts` shell tests. None of these objects carry anchors, so
// move_ops returns the cascade only (no trailing edit-geometry follow ops).
describe("sceneCore.moveOps cascade (parent-drag #15 / multi #10)", () => {
  let core: SceneCore;
  beforeAll(async () => {
    core = await loadSceneCore();
  });

  it("applies the world-space delta to the parent AND its children, parent first", () => {
    const scene = sceneOf([obj("frame", undefined, 100, 100), obj("c1", "frame", 110, 120), obj("c2", "frame", 130, 140)]);
    const ops = core.moveOps(scene, { kind: "single", id: "frame" }, translateDelta(40, 25));
    // One op per object: the dragged frame first, then each child in scene order.
    expect(setTransformIds(ops)).toEqual(["frame", "c1", "c2"]);
    expect(originOf(ops, "frame")).toEqual([140, 125]);
    expect(originOf(ops, "c1")).toEqual([150, 145]); // delta reached the child
    expect(originOf(ops, "c2")).toEqual([170, 165]);
  });

  it("cascades transitively to a grandchild (child frame nested under the parent)", () => {
    const scene = sceneOf([obj("frame", undefined, 0, 0), obj("inner", "frame", 50, 50), obj("leaf", "inner", 70, 80)]);
    const ops = core.moveOps(scene, { kind: "single", id: "frame" }, translateDelta(10, -5));
    expect(setTransformIds(ops)).toEqual(["frame", "inner", "leaf"]);
    expect(originOf(ops, "leaf")).toEqual([80, 75]);
  });

  it("does NOT touch siblings outside the dragged subtree", () => {
    const scene = sceneOf([obj("frame", undefined, 0, 0), obj("c1", "frame", 10, 10), obj("loner", undefined, 200, 200)]);
    const ops = core.moveOps(scene, { kind: "single", id: "frame" }, translateDelta(5, 5));
    expect(setTransformIds(ops).sort()).toEqual(["c1", "frame"]);
  });

  it("returns no ops when the dragged id is not in the scene", () => {
    const scene = sceneOf([obj("a", undefined, 0, 0)]);
    expect(core.moveOps(scene, { kind: "single", id: "ghost" }, translateDelta(1, 1))).toEqual([]);
  });

  it("a Multi drag moves EVERY member together (#10) — same world delta to each, deduped", () => {
    const scene = sceneOf([obj("a", undefined, 100, 100), obj("b", undefined, 300, 50), obj("c", undefined, 500, 500)]);
    // Drag the Multi {a, b}: both move by (40, 25); the unselected `c` does not.
    const ops = core.moveOps(scene, { kind: "multi", ids: ["a", "b"] }, translateDelta(40, 25));
    expect(setTransformIds(ops)).toEqual(["a", "b"]);
    expect(originOf(ops, "a")).toEqual([140, 125]);
    expect(originOf(ops, "b")).toEqual([340, 75]);
  });

  it("a Multi drag where one member is a frame cascades to its children AND dedupes overlap", () => {
    // `frame` contains `child`; the Multi also explicitly selects `child`. The
    // delta must reach `child` exactly once (frame's cascade), not twice.
    const scene = sceneOf([obj("frame", undefined, 0, 0), obj("child", "frame", 50, 50)]);
    const ops = core.moveOps(scene, { kind: "multi", ids: ["frame", "child"] }, translateDelta(10, 10));
    expect(setTransformIds(ops).filter((id) => id === "child")).toHaveLength(1); // deduped
    expect(originOf(ops, "child")).toEqual([60, 60]); // 50 + 10, applied once
  });

  it("composes delta*base (pre-multiply), so a rotation about origin rotates the child position", () => {
    // 90° rotation delta about the world origin; pre-multiply must move a child at
    // (1,0) to (0,1), proving the core composes delta on the LEFT. A CLOSED rect:
    // open-class members route non-translate deltas through their endpoints
    // (anchor-semantics v3 §2c) instead of composing a set-transform.
    const rot90: [[number, number, number], [number, number, number], [number, number, number]] = [
      [0, -1, 0],
      [1, 0, 0],
      [0, 0, 1]
    ];
    const closed = { ...obj("c", undefined, 1, 0), geometry: { d: "M 0 0 L 80 0 L 80 40 L 0 40 Z" } } as SceneObject;
    const scene = sceneOf([closed]);
    const ops = core.moveOps(scene, { kind: "single", id: "c" }, rot90);
    const [x, y] = originOf(ops, "c");
    expect(x).toBeCloseTo(0, 9);
    expect(y).toBeCloseTo(1, 9);
  });
});

// The onSelectObject / onMarquee / onTransformCommit wiring, exercised through the
// extracted controller functions the shell now composes (no .svelte source pin).
// Falsifiable — dropping the additive toggle, the marquee apply, or the cascade
// route all change these results.
describe("controller selection-UX wiring (AP2)", () => {
  it("routes the additive flag through toggleObjectSelection on pick", () => {
    // routeSelectObject(additive=true) must equal the pure toggle the shell uses.
    const before: ObjectSelection = { kind: "object", id: "o1" };
    expect(routeSelectObject(before, "o2", true)).toEqual(toggleObjectSelection(before, "o2"));
  });

  it("applies the marquee ids (RA2a) to the selection", () => {
    expect(routeMarquee(["o1", "o2"])).toEqual({ kind: "multi", ids: ["o1", "o2"] });
    expect(routeMarquee(["o1"])).toEqual({ kind: "object", id: "o1" });
    expect(routeMarquee([])).toEqual({ kind: "canvas" });
  });

  describe("commitBodyDrag (parent/Multi drag through the single moveOps call, #10/#15)", () => {
    let core: SceneCore;
    beforeAll(async () => {
      core = await loadSceneCore();
    });

    it("routes a single-object drag through { kind: 'single', id } and bare-ops the result", () => {
      const scene = sceneOf([obj("a", undefined, 100, 100)]);
      const { op, allOps } = commitBodyDrag(core, scene, { kind: "object", id: "a" }, "a", translateDelta(40, 25), "translate", false);
      // One member dragged -> one set-transform op, returned bare (not wrapped in a batch).
      expect(setTransformIds(allOps)).toEqual(["a"]);
      expect(originOf(allOps, "a")).toEqual([140, 125]);
      expect(op).toEqual(allOps[0]);
    });

    it("routes a Multi drag through { kind: 'multi', ids } so every member moves, wrapped in a batch", () => {
      const scene = sceneOf([obj("a", undefined, 100, 100), obj("b", undefined, 300, 50), obj("c", undefined, 500, 500)]);
      const selection: ObjectSelection = { kind: "multi", ids: ["a", "b"] };
      const { op, allOps } = commitBodyDrag(core, scene, selection, "a", translateDelta(40, 25), "translate", false);
      expect(setTransformIds(allOps)).toEqual(["a", "b"]);
      expect(originOf(allOps, "b")).toEqual([340, 75]);
      // Many ops -> wrapped in a single batch op.
      expect(op).toEqual({ kind: "batch", ops: allOps });
    });

    it("anchors a plain drag on the picked id even when a different Multi is selected", () => {
      // The picked id is NOT in the multi -> the drag falls back to the single root.
      const scene = sceneOf([obj("frame", undefined, 0, 0), obj("c1", "frame", 10, 10), obj("loner", undefined, 200, 200)]);
      const { allOps } = commitBodyDrag(core, scene, { kind: "multi", ids: ["loner"] }, "frame", translateDelta(5, 5), "translate", false);
      expect(setTransformIds(allOps).sort()).toEqual(["c1", "frame"]);
    });
  });
});
