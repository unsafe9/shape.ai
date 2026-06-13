import { beforeAll, describe, expect, it } from "vitest";

import { type DragSpan } from "../controller/objectPrimitives";
import { applyDocumentTheme } from "../renderer/scene";
import { ensureSceneCore, loadSceneCore, type SceneCore } from "../bridge/sceneCoreWasm";
import {
  GEOMETRY_QUANTUM_PER_PX,
  emptyObjectScene,
  toggleObjectSelection,
  translateTransform,
  type Object as SceneObject,
  type ObjectOp,
  type ObjectScene,
  type ObjectSelection,
  type Transform3x3
} from "../shared/object";

let core: SceneCore;

const PEN = { color: "#1f2933", widthPx: 2 };
const ZIGZAG_POINTS = [
  { x: 0, y: 0 },
  { x: 40, y: 40 },
  { x: 80, y: 0 },
  { x: 120, y: 40 },
  { x: 160, y: 0 }
];

// Rotate-about-center delta — the matrix the renderer previews while dragging the handle.
function rotateAboutDelta(theta: number, cx: number, cy: number): Transform3x3 {
  const c = Math.cos(theta);
  const s = Math.sin(theta);
  return [
    [c, -s, cx - c * cx + s * cy],
    [s, c, cy - s * cx - c * cy],
    [0, 0, 1]
  ];
}

beforeAll(async () => {
  await ensureSceneCore();
  core = await loadSceneCore();
});

// In-memory mirror of the App.svelte authorOp loop: apply through the real core,
// swap in the new scene, push the inverse onto an undo log. The spine every step
// runs through, so steps compose against one evolving scene + undo history.
class LiveShell {
  scene: ObjectScene = emptyObjectScene();
  selection: ObjectSelection = { kind: "canvas" };
  private undoLog: ObjectOp[] = [];

  author(op: ObjectOp): void {
    const result = core.applyObjectOp(this.scene, op);
    expect(result.errors).toEqual([]);
    this.scene = result.scene;
    if (result.inverse) this.undoLog.push(result.inverse);
  }

  undo(): void {
    const inverse = this.undoLog.pop();
    if (!inverse) return;
    const result = core.applyObjectOp(this.scene, inverse);
    expect(result.errors).toEqual([]);
    this.scene = result.scene;
  }

  byId(id: string): SceneObject | undefined {
    return this.scene.objects.find((o) => o.id === id);
  }
}

describe("wave-3 live paths compose through one shared scene", () => {
  it("(A) theme toggle flips clear + chrome together AND drives the renderer bit", () => {
    // applyDocumentTheme must flip the root attr, persist, and fire the renderer setter
    // in one call. A spy stands in for the GPU setter.
    const attrs = new Map<string, string>();
    const store = new Map<string, string>();
    const driven: boolean[] = [];
    const root = { setAttribute: (n: string, v: string) => void attrs.set(n, v) };
    const storage = { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v) };

    applyDocumentTheme("dark", { root, storage, setRendererTheme: (dark) => driven.push(dark) });
    expect(attrs.get("data-theme")).toBe("dark");
    expect(driven).toEqual([true]);
    applyDocumentTheme("light", { root, storage, setRendererTheme: (dark) => driven.push(dark) });
    expect(attrs.get("data-theme")).toBe("light");
    expect(driven).toEqual([true, false]);
  });

  it("(B–H) builds, recolors, groups, edits, anchors, and erases through the core", () => {
    const shell = new LiveShell();

    // (B) a new shape carries the toolbar's selected color into both fill and stroke.
    const SELECTED = "#abcdef";
    const rect = core.buildPrimitive("rectangle", { x: 100, y: 100 }, "rect-1", "a0", SELECTED);
    expect(rect.fill?.paint).toEqual({ kind: "solid", color: SELECTED });
    shell.author({ kind: "insert-object", object: rect });
    expect(shell.byId("rect-1")?.fill?.paint).toEqual({ kind: "solid", color: SELECTED });

    const ellipse = core.buildPrimitive("ellipse", { x: 400, y: 100 }, "ell-1", "a1", SELECTED);
    shell.author({ kind: "insert-object", object: ellipse });

    // (B cont.) recolor a selected object; the inverse restores the old paint.
    const RECOLOR = "#123456";
    shell.author(core.buildSetStyleOp(shell.byId("rect-1")!, RECOLOR));
    expect(shell.byId("rect-1")?.fill?.paint).toEqual({ kind: "solid", color: RECOLOR });
    shell.undo();
    expect(shell.byId("rect-1")?.fill?.paint).toEqual({ kind: "solid", color: SELECTED });

    // (C) marquee sweeps both shapes into a multi, then a modifier re-click toggles
    // the rect back out, and back in.
    const marqueeIds = shell.scene.objects.map((o) => o.id);
    shell.selection = marqueeIds.length >= 2 ? { kind: "multi", ids: marqueeIds } : { kind: "canvas" };
    expect(shell.selection).toEqual({ kind: "multi", ids: ["rect-1", "ell-1"] });
    shell.selection = toggleObjectSelection(shell.selection, "rect-1");
    expect(shell.selection).toEqual({ kind: "object", id: "ell-1" });
    shell.selection = toggleObjectSelection(shell.selection, "rect-1");
    expect(shell.selection).toEqual({ kind: "multi", ids: ["ell-1", "rect-1"] });

    // (D) group under a fresh frame: insert a container then reparent both shapes
    // under it, in one batch.
    const frame: SceneObject = {
      id: "frame-1",
      order: "a2",
      transform: translateTransform(100, 100),
      geometry: { d: "M 0 0 L 8 0", fillRule: "nonZero" },
      clip: false
    };
    shell.author({
      kind: "batch",
      ops: [
        { kind: "insert-object", object: frame },
        { kind: "reparent", id: "rect-1", parent: "frame-1", order: "a0" },
        { kind: "reparent", id: "ell-1", parent: "frame-1", order: "a0~" }
      ]
    });
    expect(shell.byId("rect-1")?.parent).toBe("frame-1");
    expect(core.hasChildren(shell.scene, "frame-1")).toBe(true);
    expect(core.ungroupEnabled(shell.scene, "frame-1")).toBe(true);
    expect(core.ungroupEnabled(shell.scene, "rect-1")).toBe(false);

    // (D cont.) double-click drills into the container; a leaf would edit-text.
    expect(core.doubleClickAction(shell.scene, "frame-1")).toEqual({ kind: "drill-in-container" });
    expect(core.doubleClickAction(shell.scene, "rect-1")).toEqual({ kind: "edit-leaf" });

    // Dragging the frame cascades the world-space delta to the children (one op per object).
    const rectOriginX = shell.byId("rect-1")!.transform![0][2];
    const dragDelta = translateTransform(40, 25);
    const cascade = core.moveOps(shell.scene, { kind: "single", id: "frame-1" }, dragDelta);
    expect(cascade.map((o) => (o.kind === "set-transform" ? o.id : "")).sort()).toEqual(["ell-1", "frame-1", "rect-1"]);
    shell.author({ kind: "batch", ops: cascade });
    expect(shell.byId("rect-1")?.transform?.[0][2]).toBe(rectOriginX + 40);

    // (D cont.) ungroup pops the children back out and deletes the now-empty frame.
    const children = shell.scene.objects.filter((o) => o.parent === "frame-1");
    shell.author({
      kind: "batch",
      ops: [...children.map((c) => ({ kind: "reparent", id: c.id, order: c.order }) as ObjectOp), { kind: "delete", id: "frame-1" }]
    });
    expect(shell.byId("frame-1")).toBeUndefined();
    expect(shell.byId("rect-1")?.parent).toBeUndefined();

    // (E) a borderless text primitive takes a set-text op carrying the runs the renderer shapes into glyph quads.
    const note = core.buildPrimitive("text", { x: 700, y: 100 }, "note-1", "a3");
    expect(note.text).toBeUndefined();
    shell.author({ kind: "insert-object", object: note });
    shell.author({ kind: "set-text", id: "note-1", text: { runs: [{ text: "Live", bold: false, italic: false }], align: "start", valign: "top" } });
    expect(shell.byId("note-1")?.text?.runs[0]?.text).toBe("Live");

    // (F) a rotate-handle delta composes onto the rect's transform and lands as ONE
    // undoable set-transform (the preview is the commit).
    const rectBase = shell.byId("rect-1")!.transform!;
    const handleDelta = rotateAboutDelta(Math.PI / 2, 0, 0);
    const rotateOps = core.moveOps(shell.scene, { kind: "single", id: "rect-1" }, handleDelta);
    expect(rotateOps).toHaveLength(1);
    shell.author(rotateOps[0]);
    const m = shell.byId("rect-1")!.transform!;
    expect(Math.abs(m[0][1])).toBeGreaterThan(0.5);
    shell.undo();
    expect(shell.byId("rect-1")!.transform).toEqual(rectBase);

    // (G) a snapped line drag binds its endpoint to the ellipse target; the endpoint
    // moves-together when the target moves (persistent anchor).
    const target = shell.byId("ell-1")!;
    const tx = target.transform?.[0][2] ?? 0;
    const ty = target.transform?.[1][2] ?? 0;
    const span: DragSpan = { start: { x: tx - 120, y: ty }, end: { x: tx, y: ty } };
    const edge = core.buildPrimitiveFromDrag("line", span, "edge-1", "a4");
    const anchors = core.synthesizeCreateAnchors(edge, target, span.end);
    expect(anchors).not.toBeNull();
    expect(anchors![0].target).toBe("ell-1");
    shell.author({ kind: "insert-object", object: edge });
    shell.author({ kind: "set-anchor", id: "edge-1", anchors: anchors! });
    expect(shell.byId("edge-1")?.anchors?.[0].target).toBe("ell-1");
    // Move-together: a set-transform on the target must reproject the bound node so it
    // tracks the +200/+60 delta. worldNode1 is the anchored endpoint before vs after.
    const worldNode1 = (obj: SceneObject): { x: number; y: number } => {
      const nums = (obj.geometry.d ?? "").match(/-?\d+(?:\.\d+)?/g)!;
      const lx = Number(nums[2]) / GEOMETRY_QUANTUM_PER_PX;
      const ly = Number(nums[3]) / GEOMETRY_QUANTUM_PER_PX;
      const t = obj.transform;
      return { x: t ? t[0][0] * lx + t[0][1] * ly + t[0][2] : lx, y: t ? t[1][0] * lx + t[1][1] * ly + t[1][2] : ly };
    };
    const atRest = worldNode1(shell.byId("edge-1")!);
    const moveOp: ObjectOp = { kind: "set-transform", id: "ell-1", transform: translateTransform(tx + 200, ty + 60) };
    const followOps = core.anchorFollowOps(shell.scene, [moveOp]);
    expect(followOps).toHaveLength(1);
    const follow = followOps[0];
    if (follow.kind !== "edit-geometry" || follow.id !== "edge-1") throw new Error("expected edge-1 reproject");
    const afterMove = worldNode1({ ...shell.byId("edge-1")!, geometry: follow.geometry });
    expect(afterMove.x).toBeCloseTo(atRest.x + 200);
    expect(afterMove.y).toBeCloseTo(atRest.y + 60);

    // (H) swept erase: a continuous erase drag cuts the stroke node-by-node. The whole
    // drag coalesces into ONE undo step — the coalesce window keeps the FIRST inverse,
    // so a single undo lands at the pre-sweep geometry.
    const stroke = core.freehandToObject(ZIGZAG_POINTS, PEN.color, PEN.widthPx, "draw-1", "a5", "free");
    shell.author({ kind: "insert-object", object: stroke });
    const originalGeometryD = shell.byId("draw-1")!.geometry.d;
    const undo = core.createUndoStack("ig1");
    const radius = 24 * GEOMETRY_QUANTUM_PER_PX;

    undo.beginCoalesce();
    // First touch: cut at the interior peak (80,0 local) -> two subpaths.
    const cut1 = core.splitSubpathAt(shell.byId("draw-1")!.geometry, 80 * GEOMETRY_QUANTUM_PER_PX, 0, radius);
    expect(cut1).not.toBeNull();
    expect(((cut1!.d ?? "").match(/M/g) ?? []).length).toBe(2);
    let edit = core.applyObjectOp(shell.scene, { kind: "edit-geometry", id: "draw-1", geometry: cut1! });
    undo.record({ kind: "edit-geometry", id: "draw-1", geometry: cut1! }, edit.inverse!);
    shell.scene = edit.scene;
    // Second touch of the same drag: another edit-geometry into the open coalesce
    // window keeps the gesture a single undo unit.
    const cut2 = core.splitSubpathAt(shell.byId("draw-1")!.geometry, 120 * GEOMETRY_QUANTUM_PER_PX, 40 * GEOMETRY_QUANTUM_PER_PX, radius);
    expect(cut2).not.toBeNull();
    edit = core.applyObjectOp(shell.scene, { kind: "edit-geometry", id: "draw-1", geometry: cut2! });
    undo.record({ kind: "edit-geometry", id: "draw-1", geometry: cut2! }, edit.inverse!);
    shell.scene = edit.scene;
    undo.endCoalesce();
    expect(shell.byId("draw-1")!.geometry.d).not.toBe(originalGeometryD);
    expect(undo.canUndo()).toBe(true);

    // One undo reverts the whole multi-touch sweep (not two undos, one).
    const restore = undo.undo();
    expect(restore?.kind).toBe("edit-geometry");
    const back = core.applyObjectOp(shell.scene, restore!);
    expect(back.errors).toEqual([]);
    shell.scene = back.scene;
    undo.noteUndoApplied(back.inverse!);
    expect(shell.byId("draw-1")!.geometry.d).toBe(originalGeometryD);
    expect(undo.canUndo()).toBe(false);
  });
});
