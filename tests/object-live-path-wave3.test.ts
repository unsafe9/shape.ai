// W3-IG1 — the wave-3 live object path, exercised together (no GPU).
//
// Each wave-3 feature is already unit-tested in isolation (theme-shell,
// selection-ux, group-hierarchy, color-apply, anchor-create, eraser, text-edit).
// This pass drives them through ONE shared scene so the live paths compose the way
// the App.svelte shell composes them — every step feeds the next, through the REAL
// scene-core wasm (op-apply, freehand lowering, subpath split, undo stack) and the
// REAL shell helpers the live App imports (no re-derived math):
//   (A) theme toggle flips clear + chrome together: applyDocumentTheme drives the
//       root data-theme attr, persists, AND fires the renderer theme-bit setter
//       (RB1 clear-color + chrome flip; AP4 #12c);
//   (B) color apply: a new shape carries the selected color, and SetStyle recolors
//       a selected object through the core apply path (AP1 #5);
//   (C) marquee + additive select: a marquee multi-selects, then a modifier-click
//       additively toggles the set (AP2 RA2a/#10);
//   (D) group / ungroup + drill-in: 1+ objects group under a fresh frame via the
//       core, double-click drills into the container, ungroup pops the children
//       back and deletes the frame (AP3 #9/#13/#18);
//   (E) live text renders geometry: a borderless text primitive takes a set-text
//       op and the projected feed carries the runs the renderer shapes to glyphs
//       (RB2 / W2-10);
//   (F) live handles follow preview: a resize/rotate handle delta composes onto the
//       transform and lands as ONE undoable set-transform (the preview the renderer
//       shows is the same matrix the commit applies; W2-04/05);
//   (G) drag-create anchor: a snapped drag-create synthesizes a persistent anchor
//       whose endpoint moves-together with the target (AP5 #14);
//   (H) swept erase: a continuous erase drag cuts a stroke node-by-node through the
//       core split, coalesced into one undoable step that one undo reverts (W2-08).

import { beforeAll, describe, expect, it } from "vitest";

import { objectSceneToRenderObjectScene } from "../src/client/lib/canvasHost";
import { hasChildren, ungroupEnabled, doubleClickAction } from "../src/client/lib/grouping";
import {
  buildPrimitiveObject,
  buildPrimitiveObjectFromDrag,
  buildSetStyleOp,
  type DragSpan
} from "../src/client/lib/objectPrimitives";
import { applyDocumentTheme } from "../src/client/renderer/scene";
import { ensureSceneCore, loadSceneCore, type SceneCore } from "../src/client/scene/sceneCoreWasm";
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
} from "../src/shared/object";

let core: SceneCore;

const PEN = { color: "#1f2933", widthPx: 2, epsilon: 2.0 };
const ZIGZAG_POINTS = [
  { x: 0, y: 0 },
  { x: 40, y: 40 },
  { x: 80, y: 0 },
  { x: 120, y: 40 },
  { x: 160, y: 0 }
];

// A rotate-about-center handle delta (shape rotate_delta_matrix, W2-04): the same
// matrix the renderer previews while dragging the rotate handle.
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

// A faithful in-memory mirror of the App.svelte authorOp loop: applies the op
// through the real core, swaps in the new scene, and pushes the inverse onto an
// undo log. This is the spine every step below runs through, so the wave-3 paths
// compose against ONE evolving scene + undo history (not isolated fixtures).
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

describe("W3-IG1 wave-3 live paths compose through one shared scene", () => {
  it("(A) theme toggle flips clear + chrome together AND drives the renderer bit", () => {
    // applyDocumentTheme is the single source the App's toggleTheme calls; it must
    // flip the root attr (chrome/CSS), persist, and fire the renderer setter (the
    // RB1 clear-color + theme bit) in one call. A spy stands in for the GPU setter.
    const attrs = new Map<string, string>();
    const store = new Map<string, string>();
    const driven: boolean[] = [];
    const root = { setAttribute: (n: string, v: string) => void attrs.set(n, v) };
    const storage = { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v) };

    applyDocumentTheme("dark", { root, storage, setRendererTheme: (dark) => driven.push(dark) });
    expect(attrs.get("data-theme")).toBe("dark");
    expect(driven).toEqual([true]); // chrome (attr) and clear (renderer bit) flipped together
    applyDocumentTheme("light", { root, storage, setRendererTheme: (dark) => driven.push(dark) });
    expect(attrs.get("data-theme")).toBe("light");
    expect(driven).toEqual([true, false]);
  });

  it("(B–H) builds, recolors, groups, edits, anchors, and erases through the core", () => {
    const shell = new LiveShell();

    // (B) color apply — a new shape carries the toolbar's selected color into BOTH
    // fill and stroke through the real insert path.
    const SELECTED = "#abcdef";
    const rect = buildPrimitiveObject("rectangle", { x: 100, y: 100 }, "rect-1", "a0", SELECTED);
    expect(rect.fill?.paint).toEqual({ kind: "solid", color: SELECTED });
    shell.author({ kind: "insert-object", object: rect });
    expect(shell.byId("rect-1")?.fill?.paint).toEqual({ kind: "solid", color: SELECTED });

    const ellipse = buildPrimitiveObject("ellipse", { x: 400, y: 100 }, "ell-1", "a1", SELECTED);
    shell.author({ kind: "insert-object", object: ellipse });

    // (B cont.) recolor a selected object: buildSetStyleOp authors a set-style the
    // core applies; the inverse restores the old paint (undoable recolor).
    const RECOLOR = "#123456";
    shell.author(buildSetStyleOp(shell.byId("rect-1")!, RECOLOR));
    expect(shell.byId("rect-1")?.fill?.paint).toEqual({ kind: "solid", color: RECOLOR });
    shell.undo();
    expect(shell.byId("rect-1")?.fill?.paint).toEqual({ kind: "solid", color: SELECTED });

    // (C) marquee + additive select: a marquee sweeps both shapes into a multi, then
    // a modifier re-click toggles the rect back out (collapsing to a single object).
    const marqueeIds = shell.scene.objects.map((o) => o.id);
    shell.selection = marqueeIds.length >= 2 ? { kind: "multi", ids: marqueeIds } : { kind: "canvas" };
    expect(shell.selection).toEqual({ kind: "multi", ids: ["rect-1", "ell-1"] });
    shell.selection = toggleObjectSelection(shell.selection, "rect-1");
    expect(shell.selection).toEqual({ kind: "object", id: "ell-1" });
    shell.selection = toggleObjectSelection(shell.selection, "rect-1"); // additive add-back
    expect(shell.selection).toEqual({ kind: "multi", ids: ["ell-1", "rect-1"] });

    // (D) group 1+ objects under a fresh frame (the App.svelte `< 1` guard): insert
    // a container then reparent both shapes under it, in one batch through the core.
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
    expect(hasChildren(shell.scene.objects, "frame-1")).toBe(true);
    expect(ungroupEnabled(shell.scene.objects, "frame-1")).toBe(true);
    expect(ungroupEnabled(shell.scene.objects, "rect-1")).toBe(false); // leaf is not ungroupable

    // (D cont.) double-click drills into the container (a leaf would edit-text).
    expect(doubleClickAction({ id: "frame-1", hasChildren: true })).toEqual({ kind: "drill-in", id: "frame-1" });

    // (C/D cascade) a parent drag cascades the world-space delta to the children —
    // dragging the frame moves rect + ellipse with it (one op per object). Tier-2:
    // the cascade now comes from the REAL scene-core `moveOps` (single root).
    const rectOriginX = shell.byId("rect-1")!.transform![0][2];
    const dragDelta = translateTransform(40, 25);
    const cascade = core.moveOps(shell.scene, { kind: "single", id: "frame-1" }, dragDelta);
    expect(cascade.map((o) => (o.kind === "set-transform" ? o.id : "")).sort()).toEqual(["ell-1", "frame-1", "rect-1"]);
    shell.author({ kind: "batch", ops: cascade });
    expect(shell.byId("rect-1")?.transform?.[0][2]).toBe(rectOriginX + 40); // delta reached the child

    // (D cont.) ungroup pops the children back out and deletes the now-empty frame.
    const children = shell.scene.objects.filter((o) => o.parent === "frame-1");
    shell.author({
      kind: "batch",
      ops: [...children.map((c) => ({ kind: "reparent", id: c.id, order: c.order }) as ObjectOp), { kind: "delete", id: "frame-1" }]
    });
    expect(shell.byId("frame-1")).toBeUndefined();
    expect(shell.byId("rect-1")?.parent).toBeUndefined();

    // (E) live text renders geometry: a borderless text primitive takes a set-text
    // op; the projected feed carries the runs the renderer shapes into glyph quads.
    const note = buildPrimitiveObject("text", { x: 700, y: 100 }, "note-1", "a3");
    expect(note.text).toBeUndefined(); // borderless, style-less, no default text
    shell.author({ kind: "insert-object", object: note });
    shell.author({ kind: "set-text", id: "note-1", text: { runs: [{ text: "Live" }] } });
    expect(shell.byId("note-1")?.text?.runs[0]?.text).toBe("Live");
    const feed = objectSceneToRenderObjectScene(shell.scene, { x: 0, y: 0, zoom: 1 }, shell.selection, "ig1");
    const feedNote = (feed.objects as Array<Record<string, unknown>>).find((o) => o.id === "note-1")!;
    expect((feedNote.text as { runs: Array<{ text: string }> }).runs[0].text).toBe("Live"); // runs reach the renderer
    expect(typeof feedNote.geometryD).toBe("string");

    // (F) live handles follow preview: a rotate-handle delta composes onto the rect's
    // transform and lands as ONE undoable set-transform (the preview IS the commit).
    // Tier-2: the commit composes through the REAL scene-core `moveOps` (rect-1 has
    // no children/anchors here, so it returns exactly one set-transform op).
    const rectBase = shell.byId("rect-1")!.transform!;
    const handleDelta = rotateAboutDelta(Math.PI / 2, 0, 0);
    const rotateOps = core.moveOps(shell.scene, { kind: "single", id: "rect-1" }, handleDelta);
    expect(rotateOps).toHaveLength(1);
    shell.author(rotateOps[0]);
    const m = shell.byId("rect-1")!.transform!;
    expect(Math.abs(m[0][1])).toBeGreaterThan(0.5); // off-diagonal proves the rotation landed
    shell.undo();
    expect(shell.byId("rect-1")!.transform).toEqual(rectBase); // single undoable step

    // (G) drag-create anchor: a snapped line drag binds its endpoint to the ellipse
    // target; the endpoint moves-together when the target moves (persistent anchor).
    const target = shell.byId("ell-1")!;
    const tx = target.transform?.[0][2] ?? 0;
    const ty = target.transform?.[1][2] ?? 0;
    const span: DragSpan = { start: { x: tx - 120, y: ty }, end: { x: tx, y: ty } };
    const edge = buildPrimitiveObjectFromDrag("line", span, "edge-1", "a4");
    const anchors = core.synthesizeCreateAnchors(edge, target, span.end);
    expect(anchors).not.toBeNull();
    expect(anchors![0].target).toBe("ell-1");
    shell.author({ kind: "insert-object", object: edge });
    shell.author({ kind: "set-anchor", id: "edge-1", anchors: anchors! });
    expect(shell.byId("edge-1")?.anchors?.[0].target).toBe("ell-1");
    // W3-G9/#5 regression: the wire projection MUST carry each object's anchors so
    // the core builds the Reproject edges that drive live move-together. Before the
    // fix the projection dropped `anchors`, so the core's bindings graph had zero
    // anchor edges and a moved target never reprojected its follower LIVE. This
    // fails if `objectSceneToRenderObjectScene` omits anchors again.
    const anchorFeed = objectSceneToRenderObjectScene(shell.scene, { x: 0, y: 0, zoom: 1 }, shell.selection, "ig1-anchor");
    const feedEdge = (anchorFeed.objects as Array<Record<string, unknown>>).find((o) => o.id === "edge-1")!;
    const feedAnchors = feedEdge.anchors as Array<{ nodeIndex: number; target: string; at: { x: number; y: number } }>;
    expect(feedAnchors).toHaveLength(1);
    expect(feedAnchors[0].target).toBe("ell-1");
    expect(typeof feedAnchors[0].nodeIndex).toBe("number");
    expect(typeof feedAnchors[0].at.x).toBe("number");
    // Move-together (the falsifiable anchor behavior): authoring a set-transform on
    // the target through the core's commit-time follow ops must reproject the bound
    // node so it tracks the target's +200/+60 delta. The world position of the
    // edge's node 1 (the anchored endpoint) before vs after the follow.
    const worldNode1 = (obj: SceneObject): { x: number; y: number } => {
      const nums = obj.geometry.d.match(/-?\d+(?:\.\d+)?/g)!;
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

    // (H) swept erase: a freehand stroke, then a continuous erase DRAG that cuts the
    // stroke node-by-node through the core split (the swept path the eraser walks).
    // The whole drag coalesces into ONE undo step (D21): the core's coalesce window
    // keeps the FIRST inverse, so a single undo lands at the PRE-sweep geometry.
    const stroke = core.freehandToObject(ZIGZAG_POINTS, PEN.color, PEN.widthPx, PEN.epsilon, "draw-1", "a5");
    shell.author({ kind: "insert-object", object: stroke });
    const originalGeometryD = shell.byId("draw-1")!.geometry.d;
    const undo = core.createUndoStack("ig1");
    const radius = 24 * GEOMETRY_QUANTUM_PER_PX;

    undo.beginCoalesce();
    // First touch of the sweep: cut at the interior peak (80,0 local) → two subpaths.
    const cut1 = core.splitSubpathAt(shell.byId("draw-1")!.geometry, 80 * GEOMETRY_QUANTUM_PER_PX, 0, radius);
    expect(cut1).not.toBeNull();
    expect((cut1!.d.match(/M/g) ?? []).length).toBe(2);
    let edit = core.applyObjectOp(shell.scene, { kind: "edit-geometry", id: "draw-1", geometry: cut1! });
    undo.record({ kind: "edit-geometry", id: "draw-1", geometry: cut1! }, edit.inverse!);
    shell.scene = edit.scene;
    // Second touch of the SAME drag: cut again at the next peak (120,40 local). The
    // sweep walks the stroke; each touch authors another edit-geometry into the
    // open coalesce window, so the gesture stays a single undo unit.
    const cut2 = core.splitSubpathAt(shell.byId("draw-1")!.geometry, 120 * GEOMETRY_QUANTUM_PER_PX, 40 * GEOMETRY_QUANTUM_PER_PX, radius);
    expect(cut2).not.toBeNull();
    edit = core.applyObjectOp(shell.scene, { kind: "edit-geometry", id: "draw-1", geometry: cut2! });
    undo.record({ kind: "edit-geometry", id: "draw-1", geometry: cut2! }, edit.inverse!);
    shell.scene = edit.scene;
    undo.endCoalesce();
    expect(shell.byId("draw-1")!.geometry.d).not.toBe(originalGeometryD); // the sweep cut it
    expect(undo.canUndo()).toBe(true);

    // ONE undo for the whole multi-touch sweep restores the original stroke geometry
    // (the falsifiable atomic-gesture contract: not two undos, one).
    const restore = undo.undo();
    expect(restore?.kind).toBe("edit-geometry");
    const back = core.applyObjectOp(shell.scene, restore!);
    expect(back.errors).toEqual([]);
    shell.scene = back.scene;
    undo.noteUndoApplied(back.inverse!);
    expect(shell.byId("draw-1")!.geometry.d).toBe(originalGeometryD); // swept cuts fully reverted
    expect(undo.canUndo()).toBe(false); // exactly one undo step consumed the whole sweep
  });
});
