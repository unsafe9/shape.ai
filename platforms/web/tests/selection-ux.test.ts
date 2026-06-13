import { beforeAll, describe, expect, it } from "vitest";
import {
  emptyObjectScene,
  toggleObjectSelection,
  type Object as SceneObject,
  type ObjectOp,
  type ObjectScene,
  type ObjectSelection
} from "../shared/object";
import { loadSceneCore, type SceneCore } from "../bridge/sceneCoreWasm";
import { commitBodyDrag, routeMarquee, routeSelectObject } from "../controller/interactions";

describe("toggleObjectSelection (additive modifier-click)", () => {
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

describe("onSelectObject routing (no collapse-on-drag)", () => {
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

// Move-by-(dx,dy) delta in world space — the matrix the renderer hands the commit.
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

function setTransformIds(ops: ObjectOp[]): string[] {
  return ops.filter((o) => o.kind === "set-transform").map((o) => (o.kind === "set-transform" ? o.id : ""));
}

function originOf(ops: ObjectOp[], id: string): [number, number] {
  const op = ops.find((o) => o.kind === "set-transform" && o.id === id);
  if (op?.kind !== "set-transform") throw new Error(`expected set-transform for ${id}`);
  return [op.transform[0][2], op.transform[1][2]];
}

// None of these objects carry anchors, so move_ops returns the cascade only (no
// trailing edit-geometry follow ops).
describe("sceneCore.moveOps cascade (parent-drag / multi)", () => {
  let core: SceneCore;
  beforeAll(async () => {
    core = await loadSceneCore();
  });

  it("applies the world-space delta to the parent AND its children, parent first", () => {
    const scene = sceneOf([obj("frame", undefined, 100, 100), obj("c1", "frame", 110, 120), obj("c2", "frame", 130, 140)]);
    const ops = core.moveOps(scene, { kind: "single", id: "frame" }, translateDelta(40, 25));
    // One op per object: dragged frame first, then each child in scene order.
    expect(setTransformIds(ops)).toEqual(["frame", "c1", "c2"]);
    expect(originOf(ops, "frame")).toEqual([140, 125]);
    expect(originOf(ops, "c1")).toEqual([150, 145]);
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

  it("a Multi drag moves EVERY member together — same world delta to each, deduped", () => {
    const scene = sceneOf([obj("a", undefined, 100, 100), obj("b", undefined, 300, 50), obj("c", undefined, 500, 500)]);
    // Drag {a, b}: both move by (40, 25); the unselected `c` does not.
    const ops = core.moveOps(scene, { kind: "multi", ids: ["a", "b"] }, translateDelta(40, 25));
    expect(setTransformIds(ops)).toEqual(["a", "b"]);
    expect(originOf(ops, "a")).toEqual([140, 125]);
    expect(originOf(ops, "b")).toEqual([340, 75]);
  });

  it("a Multi drag where one member is a frame cascades to its children AND dedupes overlap", () => {
    // `frame` contains `child` and the Multi also selects `child`; the delta must
    // reach `child` exactly once.
    const scene = sceneOf([obj("frame", undefined, 0, 0), obj("child", "frame", 50, 50)]);
    const ops = core.moveOps(scene, { kind: "multi", ids: ["frame", "child"] }, translateDelta(10, 10));
    expect(setTransformIds(ops).filter((id) => id === "child")).toHaveLength(1);
    expect(originOf(ops, "child")).toEqual([60, 60]);
  });

  it("composes delta*base (pre-multiply), so a rotation about origin rotates the child position", () => {
    // 90deg rotation about the origin: pre-multiply moves a child at (1,0) to (0,1),
    // proving delta composes on the LEFT. A CLOSED rect; open-class members would
    // route non-translate deltas through their endpoints instead.
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

describe("controller selection-UX wiring", () => {
  it("routes the additive flag through toggleObjectSelection on pick", () => {
    const before: ObjectSelection = { kind: "object", id: "o1" };
    expect(routeSelectObject(before, "o2", true)).toEqual(toggleObjectSelection(before, "o2"));
  });

  it("applies the marquee ids to the selection", () => {
    expect(routeMarquee(["o1", "o2"])).toEqual({ kind: "multi", ids: ["o1", "o2"] });
    expect(routeMarquee(["o1"])).toEqual({ kind: "object", id: "o1" });
    expect(routeMarquee([])).toEqual({ kind: "canvas" });
  });

  describe("commitBodyDrag (parent/Multi drag through the single moveOps call)", () => {
    let core: SceneCore;
    beforeAll(async () => {
      core = await loadSceneCore();
    });

    it("routes a single-object drag through { kind: 'single', id } and bare-ops the result", () => {
      const scene = sceneOf([obj("a", undefined, 100, 100)]);
      const { op, allOps } = commitBodyDrag(core, scene, { kind: "object", id: "a" }, "a", translateDelta(40, 25), "translate", false);
      // One member dragged -> one set-transform op, returned bare (not batched).
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
      // Picked id is NOT in the multi -> the drag falls back to the single root.
      const scene = sceneOf([obj("frame", undefined, 0, 0), obj("c1", "frame", 10, 10), obj("loner", undefined, 200, 200)]);
      const { allOps } = commitBodyDrag(core, scene, { kind: "multi", ids: ["loner"] }, "frame", translateDelta(5, 5), "translate", false);
      expect(setTransformIds(allOps).sort()).toEqual(["c1", "frame"]);
    });
  });

  // The App nudgeSelection now routes through moveOpsForPick, the SAME core path commitBodyDrag's
  // non-detach branch uses. A keyboard nudge must therefore produce the SAME op set as an equal-delta
  // body drag — cascading to a parent's subtree, unlike the old independent per-id translate loop.
  describe("nudge == drag of an equal delta (moveOpsForPick parity)", () => {
    let core: SceneCore;
    beforeAll(async () => {
      core = await loadSceneCore();
    });

    it("a nudge on a parent cascades to its subtree, matching the equal-delta body drag op set", () => {
      const scene = sceneOf([obj("frame", undefined, 0, 0), obj("c1", "frame", 10, 10), obj("c2", "frame", 30, 40)]);
      const selection: ObjectSelection = { kind: "object", id: "frame" };
      // The shell nudge: moveOpsForPick(scene, selection, ids[0], translate(dx, dy)).
      const nudge = core.moveOpsForPick(scene, selection, "frame", translateDelta(8, 0));
      const { allOps: drag } = commitBodyDrag(core, scene, selection, "frame", translateDelta(8, 0), "translate", false);
      // Identical op set — proving the nudge rides the same cascade path as the drag.
      expect(nudge).toEqual(drag);
      // Falsifiable against the OLD per-id loop, which only touched the selected id (no cascade).
      expect(setTransformIds(nudge)).toEqual(["frame", "c1", "c2"]);
      expect(originOf(nudge, "c1")).toEqual([18, 10]);
    });

    it("a multi nudge moves every member, matching the equal-delta multi drag op set", () => {
      const scene = sceneOf([obj("a", undefined, 100, 100), obj("b", undefined, 300, 50), obj("c", undefined, 500, 500)]);
      const selection: ObjectSelection = { kind: "multi", ids: ["a", "b"] };
      const nudge = core.moveOpsForPick(scene, selection, "a", translateDelta(0, 8));
      const { allOps: drag } = commitBodyDrag(core, scene, selection, "a", translateDelta(0, 8), "translate", false);
      expect(nudge).toEqual(drag);
      expect(setTransformIds(nudge)).toEqual(["a", "b"]);
    });
  });
});
