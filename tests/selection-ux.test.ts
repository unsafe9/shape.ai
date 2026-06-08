// AP2 (#10/#7/#15) — selection UX. Pins, at the code level (no renderer / no Svelte
// mount):
//  (a) additive modifier-click accumulates a multi-select and a re-click removes
//      (the pure `toggleObjectSelection`), and App.svelte routes the engine's C2
//      additive flag through it;
//  (b) the marquee ids (RA2a) are applied to the selection;
//  (c) a parent drag cascades the world-space delta to its descendants
//      (`cascadeTransformOps`) — the delta reaches children (and grandchildren).

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { toggleObjectSelection, type Object as SceneObject, type ObjectSelection } from "../src/shared/object";
import { cascadeMultiTransformOps, cascadeTransformOps, composeTransform } from "../src/client/lib/transformCascade";

// The pure mirror of App.svelte's onSelectObject routing (W3-G5 #10). A plain pick
// on a member of the current Multi keeps the Multi (so a group-drag does not
// collapse); additive toggles; everything else replaces. Pinned both here (real
// logic) and against the source below, so the two cannot drift silently.
function routeSelectObject(current: ObjectSelection, id: string, additive: boolean): ObjectSelection {
  if (additive) return toggleObjectSelection(current, id);
  if (current.kind === "multi" && current.ids.includes(id)) return current;
  return { kind: "object", id };
}

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

describe("cascadeTransformOps (parent-drag cascade, #15)", () => {
  it("applies the world-space delta to the parent AND its children", () => {
    const scene = [obj("frame", undefined, 100, 100), obj("c1", "frame", 110, 120), obj("c2", "frame", 130, 140)];
    const ops = cascadeTransformOps(scene, "frame", translateDelta(40, 25));
    // One op per object: the dragged frame first, then each child.
    expect(ops).toHaveLength(3);
    const byId = new Map(ops.map((o) => [o.kind === "set-transform" ? o.id : "", o]));
    for (const [id, baseX, baseY] of [
      ["frame", 100, 100],
      ["c1", 110, 120],
      ["c2", 130, 140]
    ] as const) {
      const op = byId.get(id);
      expect(op?.kind).toBe("set-transform");
      if (op?.kind !== "set-transform") throw new Error("expected set-transform");
      // The delta reached the child: its origin shifted by exactly (40, 25).
      expect(op.transform[0][2]).toBe(baseX + 40);
      expect(op.transform[1][2]).toBe(baseY + 25);
    }
  });

  it("cascades transitively to a grandchild (child frame nested under the parent)", () => {
    const scene = [obj("frame", undefined, 0, 0), obj("inner", "frame", 50, 50), obj("leaf", "inner", 70, 80)];
    const ops = cascadeTransformOps(scene, "frame", translateDelta(10, -5));
    expect(ops).toHaveLength(3);
    const leaf = ops.find((o) => o.kind === "set-transform" && o.id === "leaf");
    if (leaf?.kind !== "set-transform") throw new Error("expected set-transform for leaf");
    expect(leaf.transform[0][2]).toBe(80);
    expect(leaf.transform[1][2]).toBe(75);
  });

  it("does NOT touch siblings outside the dragged subtree", () => {
    const scene = [obj("frame", undefined, 0, 0), obj("c1", "frame", 10, 10), obj("loner", undefined, 200, 200)];
    const ops = cascadeTransformOps(scene, "frame", translateDelta(5, 5));
    const ids = ops.map((o) => (o.kind === "set-transform" ? o.id : "")).sort();
    expect(ids).toEqual(["c1", "frame"]);
  });

  it("returns no ops when the dragged id is not in the scene", () => {
    expect(cascadeTransformOps([obj("a", undefined, 0, 0)], "ghost", translateDelta(1, 1))).toEqual([]);
  });

  it("a Multi drag moves EVERY member together (#10) — same world delta to each, deduped", () => {
    const scene = [obj("a", undefined, 100, 100), obj("b", undefined, 300, 50), obj("c", undefined, 500, 500)];
    // Drag the Multi {a, b}: both move by (40, 25); the unselected `c` does not.
    const ops = cascadeMultiTransformOps(scene, ["a", "b"], translateDelta(40, 25));
    const byId = new Map(ops.map((o) => [o.kind === "set-transform" ? o.id : "", o]));
    expect([...byId.keys()].sort()).toEqual(["a", "b"]);
    for (const [id, baseX, baseY] of [
      ["a", 100, 100],
      ["b", 300, 50]
    ] as const) {
      const op = byId.get(id);
      if (op?.kind !== "set-transform") throw new Error("expected set-transform");
      expect(op.transform[0][2]).toBe(baseX + 40);
      expect(op.transform[1][2]).toBe(baseY + 25);
    }
  });

  it("a Multi drag where one member is a frame cascades to its children AND dedupes overlap", () => {
    // `frame` contains `child`; the Multi also explicitly selects `child`. The
    // delta must reach `child` exactly once (frame's cascade), not twice.
    const scene = [obj("frame", undefined, 0, 0), obj("child", "frame", 50, 50)];
    const ops = cascadeMultiTransformOps(scene, ["frame", "child"], translateDelta(10, 10));
    const ids = ops.map((o) => (o.kind === "set-transform" ? o.id : ""));
    expect(ids.filter((id) => id === "child")).toHaveLength(1); // deduped
    const child = ops.find((o) => o.kind === "set-transform" && o.id === "child");
    if (child?.kind !== "set-transform") throw new Error("expected set-transform");
    expect(child.transform[0][2]).toBe(60); // 50 + 10, applied once
  });

  it("composes delta*base (pre-multiply), so a rotation about origin rotates the child position", () => {
    // 90° rotation delta about the world origin; pre-multiply must move a child at
    // (1,0) to (0,1), proving composeTransform applies delta on the left.
    const rot90: [[number, number, number], [number, number, number], [number, number, number]] = [
      [0, -1, 0],
      [1, 0, 0],
      [0, 0, 1]
    ];
    const child = obj("c", "frame", 1, 0);
    const composed = composeTransform(rot90, child.transform);
    expect(composed[0][2]).toBeCloseTo(0, 9);
    expect(composed[1][2]).toBeCloseTo(1, 9);
  });
});

describe("App.svelte selection-UX wiring (AP2)", () => {
  // No DOM in the node test env: assert the wiring against the .svelte source.
  // Falsifiable — dropping the additive toggle, the marquee apply, or the cascade
  // route all fail these.
  const source = readFileSync(fileURLToPath(new URL("../src/client/svelte/App.svelte", import.meta.url)), "utf8");

  it("routes the engine's C2 additive flag through toggleObjectSelection on pick", () => {
    expect(source).toMatch(/onSelectObject:\s*\(id,\s*additive\)\s*=>/);
    expect(source).toMatch(/additive\s*\n?\s*\?\s*toggleObjectSelection\(selection,\s*id\)/);
  });

  it("keeps the Multi when a plain pick lands on a member (no collapse-on-drag, #10)", () => {
    expect(source).toMatch(/selection\.kind\s*===\s*"multi"\s*&&\s*selection\.ids\.includes\(id\)\s*\n?\s*\?\s*selection/);
  });

  it("applies the marquee ids (RA2a) to the selection", () => {
    expect(source).toMatch(/onMarquee:\s*\(ids\)\s*=>/);
    expect(source).toMatch(/ids\.length\s*>=\s*2\s*\?\s*\{\s*kind:\s*"multi",\s*ids\s*\}/);
  });

  it("cascades a parent drag to its descendants, and a Multi drag to every member (#10/#15)", () => {
    expect(source).toMatch(/cascadeTransformOps\(scene\.objects,\s*id,\s*matrix\)/);
    expect(source).toMatch(/cascadeMultiTransformOps\(scene\.objects,\s*selection\.ids,\s*matrix\)/);
    expect(source).toMatch(/allOps\.length\s*===\s*1\s*\?\s*allOps\[0\]\s*:\s*\{\s*kind:\s*"batch",\s*ops:\s*allOps\s*\}/);
  });
});
