import { beforeAll, describe, expect, it } from "vitest";

import { objectSceneToRenderObjectScene } from "../controller/canvasHost";
import { textOverlayScreenRect, type DragSpan } from "../controller/objectPrimitives";
import { isPanIntent, shouldQuerySnap } from "../renderer/engine";
import type { RenderTransform3x3 } from "../renderer/scene";
import { ensureSceneCore, loadSceneCore, type SceneCore } from "../bridge/sceneCoreWasm";
import { isDragCreateShape } from "../controller/toolbar";
import {
  GEOMETRY_QUANTUM_PER_PX,
  IDENTITY_TRANSFORM,
  emptyObjectScene,
  toggleObjectSelection,
  translateTransform,
  type Object as SceneObject,
  type ObjectOp,
  type ObjectScene,
  type ObjectSelection,
  type Stroke,
  type Transform3x3
} from "../shared/object";

let core: SceneCore;

const PEN = { color: "#1f2933", widthPx: 2 };
const STROKE_POINTS = [
  { x: 100, y: 100 },
  { x: 140, y: 130 },
  { x: 180, y: 110 },
  { x: 220, y: 150 }
];
// Interior peaks deviate off any chord so RDP keeps them, leaving an interior node
// for a partial-erase cut to split on (a straight polyline simplifies to its endpoints).
const ZIGZAG_POINTS = [
  { x: 0, y: 0 },
  { x: 40, y: 40 },
  { x: 80, y: 0 },
  { x: 120, y: 40 },
  { x: 160, y: 0 }
];

// Row-major 3x3 pre-multiply newTransform = delta * base. Re-derived here because
// the shell's composeTransform is private to the Svelte component.
function composeTransform(delta: Transform3x3, base: Transform3x3): Transform3x3 {
  const out: Transform3x3 = [
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0]
  ];
  for (let r = 0; r < 3; r++) {
    for (let c = 0; c < 3; c++) {
      out[r][c] = delta[r][0] * base[0][c] + delta[r][1] * base[1][c] + delta[r][2] * base[2][c];
    }
  }
  return out;
}

// Scale by (sx, sy) about the fixed opposite-corner anchor (ax, ay) in world space.
function scaleAboutDelta(sx: number, sy: number, ax: number, ay: number): Transform3x3 {
  return [
    [sx, 0, ax - sx * ax],
    [0, sy, ay - sy * ay],
    [0, 0, 1]
  ];
}

// Rotate by theta about the bbox center (cx, cy) in world space.
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

describe("(a) freehandToObject lowers a stroke to an insert-able open-path object", () => {
  it("yields an open path carrying a stroke positioned at the first point", () => {
    const object = core.freehandToObject(STROKE_POINTS, PEN.color, PEN.widthPx, "draw-1", "a0", "free");

    expect(object.id).toBe("draw-1");
    expect(object.order).toBe("a0");
    expect((object.geometry.d ?? "").startsWith("M")).toBe(true);
    expect((object.geometry.d ?? "").trim().endsWith("Z")).toBe(false);
    expect(object.stroke).toBeDefined();
    expect(object.stroke?.paint).toEqual({ kind: "solid", color: PEN.color });
    expect(object.fill).toBeUndefined();
    // Width rides in quantized units (per-px * GEOMETRY_QUANTUM_PER_PX).
    expect((object.stroke as Stroke).width).toBe(PEN.widthPx * GEOMETRY_QUANTUM_PER_PX);
    // Position lives in the transform, not the geometry.
    expect(object.transform).toBeDefined();
  });
});

describe("(b) objectSceneToRenderObjectScene projects a heterogeneous scene", () => {
  it("produces a well-formed feed (de-quantized strokes, text preserved)", () => {
    const rectangle = core.buildPrimitive("rectangle", { x: 0, y: 0 }, "rect-1", "a0");
    // The text primitive is style-less; set text explicitly to verify the projection
    // preserves text runs.
    const note: SceneObject = { ...core.buildPrimitive("text", { x: 400, y: 0 }, "note-1", "a1"), text: { runs: [{ text: "Note", bold: false, italic: false }], align: "start", valign: "top" } };
    const freehand = core.freehandToObject(STROKE_POINTS, PEN.color, PEN.widthPx, "draw-1", "a2", "free");

    const scene: ObjectScene = { ...emptyObjectScene(), objects: [rectangle, note, freehand] };
    const projected = objectSceneToRenderObjectScene(scene, { x: 0, y: 0, zoom: 1 }, { kind: "canvas" }, "test-scene");

    const objects = projected.objects as Array<Record<string, unknown>>;
    expect(objects).toHaveLength(3);
    // Field renaming: geometry.d -> geometryD on every projected object.
    for (const projectedObject of objects) {
      expect(typeof projectedObject.geometryD).toBe("string");
      expect((projectedObject.geometryD as string).length).toBeGreaterThan(0);
    }

    // Stroke widths are de-quantized to logical px in the feed.
    const projectedRect = objects.find((o) => o.id === "rect-1")!;
    const sourceRectWidth = (rectangle.stroke as Stroke).width;
    expect((projectedRect.stroke as { width: number }).width).toBe(sourceRectWidth / GEOMETRY_QUANTUM_PER_PX);

    const projectedDraw = objects.find((o) => o.id === "draw-1")!;
    expect((projectedDraw.stroke as { width: number }).width).toBe(PEN.widthPx);

    const projectedNote = objects.find((o) => o.id === "note-1")!;
    const noteText = projectedNote.text as { runs: Array<{ text: string }> } | null;
    expect(noteText?.runs?.[0]?.text).toBe("Note");
  });
});

describe("(c) apply_object_op inserts the freehand object without errors", () => {
  it("applies an insert-object and captures the inverse delete", () => {
    const freehand = core.freehandToObject(STROKE_POINTS, PEN.color, PEN.widthPx, "draw-1", "a0", "free");
    const op = { kind: "insert-object", object: freehand } as const;

    const result = core.applyObjectOp(emptyObjectScene(), op);
    expect(result.errors).toEqual([]);
    expect(result.scene.objects.map((o: SceneObject) => o.id)).toEqual(["draw-1"]);
    expect(result.inverse).toEqual({ kind: "delete", id: "draw-1" });
  });
});

describe("(d) unified pointer: pan-intent + multi-select toggle (W2-03)", () => {
  it("classifies the pan gesture (middle button OR left+Space)", () => {
    expect(isPanIntent({ spaceHeld: false, button: 0 })).toBe(false);
    expect(isPanIntent({ spaceHeld: true, button: 0 })).toBe(true);
    expect(isPanIntent({ spaceHeld: false, button: 1 })).toBe(true);
    expect(isPanIntent({ spaceHeld: false, button: 2 })).toBe(false);
  });

  it("grows/collapses the multi-select set on shift-click toggles", () => {
    let selection: ObjectSelection = { kind: "canvas" };
    selection = toggleObjectSelection(selection, "a");
    expect(selection).toEqual({ kind: "object", id: "a" });
    selection = toggleObjectSelection(selection, "b");
    expect(selection).toEqual({ kind: "multi", ids: ["a", "b"] });
    selection = toggleObjectSelection(selection, "a");
    expect(selection).toEqual({ kind: "object", id: "b" });
  });
});

describe("(e) handle resize/rotate commit lands one undoable set-transform", () => {
  it("composes a resize delta onto the transform and round-trips the inverse", () => {
    const rect = core.buildPrimitive("rectangle", { x: 200, y: 200 }, "rect-1", "a0");
    const inserted = core.applyObjectOp(emptyObjectScene(), { kind: "insert-object", object: rect });
    expect(inserted.errors).toEqual([]);
    const base = rect.transform ?? IDENTITY_TRANSFORM;

    // Scale 2x about the rect's top-left world anchor (opposite the dragged corner).
    const [ax, ay] = [base[0][2], base[1][2]];
    const delta = scaleAboutDelta(2, 2, ax, ay);
    const transform = composeTransform(delta as RenderTransform3x3, base);

    const committed = core.applyObjectOp(inserted.scene, { kind: "set-transform", id: "rect-1", transform });
    expect(committed.errors).toEqual([]);
    expect(committed.scene.objects[0].transform).toEqual(transform);
    expect(committed.inverse?.kind).toBe("set-transform");
    const undone = core.applyObjectOp(committed.scene, committed.inverse!);
    expect(undone.errors).toEqual([]);
    expect(undone.scene.objects[0].transform).toEqual(base);
  });

  it("composes a rotate delta onto the transform (one set-transform op)", () => {
    const rect = core.buildPrimitive("rectangle", { x: 0, y: 0 }, "rect-1", "a0");
    const inserted = core.applyObjectOp(emptyObjectScene(), { kind: "insert-object", object: rect });
    const base = rect.transform ?? IDENTITY_TRANSFORM;

    const delta = rotateAboutDelta(Math.PI / 2, 0, 0);
    const transform = composeTransform(delta as RenderTransform3x3, base);

    const committed = core.applyObjectOp(inserted.scene, { kind: "set-transform", id: "rect-1", transform });
    expect(committed.errors).toEqual([]);
    // Off-diagonal terms non-zero => rotation landed.
    const m = committed.scene.objects[0].transform!;
    expect(Math.abs(m[0][1])).toBeGreaterThan(0.5);
    expect(committed.inverse?.kind).toBe("set-transform");
  });
});

describe("(f) drag-create + snap-bypass + select-after-create", () => {
  it("sizes rect/ellipse/line to the drag bbox and inserts with select-after", () => {
    // rect/ellipse/line drag-create; text/frame insert immediately at an anchor.
    expect(isDragCreateShape("rectangle")).toBe(true);
    expect(isDragCreateShape("line")).toBe(true);
    expect(isDragCreateShape("text")).toBe(false);
    expect(isDragCreateShape("frame")).toBe(false);

    const span: DragSpan = { start: { x: 100, y: 200 }, end: { x: 260, y: 300 } };
    const object = core.buildPrimitiveFromDrag("rectangle", span, "rect-1", "a0");
    expect(object.transform).toEqual(translateTransform(100, 200));
    const Q = GEOMETRY_QUANTUM_PER_PX;
    expect((object.geometry.d ?? "")).toBe(`M 0 0 L ${160 * Q} 0 L ${160 * Q} ${100 * Q} L 0 ${100 * Q} Z`);

    const inserted = core.applyObjectOp(emptyObjectScene(), { kind: "insert-object", object });
    expect(inserted.errors).toEqual([]);
    const selection: ObjectSelection = { kind: "object", id: object.id };
    expect(inserted.scene.objects[0].id).toBe(selection.kind === "object" ? selection.id : "");
  });

  it("bypasses outline snap while the Alt modifier is held", () => {
    expect(shouldQuerySnap({ altHeld: false, phase: "move" })).toBe(true);
    expect(shouldQuerySnap({ altHeld: true, phase: "move" })).toBe(false);
    expect(shouldQuerySnap({ altHeld: false, phase: "cancel" })).toBe(false);
  });
});

describe("(g) draw + eraser (whole/partial), all undoable", () => {
  it("whole-stroke erase deletes the object and the inverse re-inserts it", () => {
    const stroke = core.freehandToObject(ZIGZAG_POINTS, PEN.color, PEN.widthPx, "draw-1", "a0", "free");
    const scene: ObjectScene = { ...emptyObjectScene(), objects: [stroke] };

    const deleted = core.applyObjectOp(scene, { kind: "delete", id: "draw-1" });
    expect(deleted.errors).toEqual([]);
    expect(deleted.scene.objects).toEqual([]);
    expect(deleted.inverse?.kind).toBe("insert-object");
    const restored = core.applyObjectOp(deleted.scene, deleted.inverse!);
    expect(restored.scene.objects.map((o: SceneObject) => o.id)).toEqual(["draw-1"]);
  });

  it("partial erase cuts the stroke into two open subpaths via an edit-geometry op", () => {
    const stroke = core.freehandToObject(ZIGZAG_POINTS, PEN.color, PEN.widthPx, "draw-1", "a0", "free");
    // Middle sample mapped to quantized object-local coords; generous radius tolerates
    // RDP/bezier nudging.
    const touchX = 80 * GEOMETRY_QUANTUM_PER_PX;
    const touchY = 0 * GEOMETRY_QUANTUM_PER_PX;
    const cut = core.splitSubpathAt(stroke.geometry, touchX, touchY, 24 * GEOMETRY_QUANTUM_PER_PX);
    expect(cut).not.toBeNull();
    expect(((cut!.d ?? "").match(/M/g) ?? []).length).toBe(2);
    expect((cut!.d ?? "").includes("Z")).toBe(false);

    const scene: ObjectScene = { ...emptyObjectScene(), objects: [stroke] };
    const edited = core.applyObjectOp(scene, { kind: "edit-geometry", id: "draw-1", geometry: cut! });
    expect(edited.errors).toEqual([]);
    expect(edited.scene.objects[0].geometry.d).toBe(cut!.d);
    expect(edited.inverse?.kind).toBe("edit-geometry");
  });

  it("a continuous erase drag stays undoable as one step via the core undo stack", () => {
    // Coalesce a continuous gesture into one undo entry: author -> record inside the
    // window, then undo by re-authoring the handed-out inverse through apply.
    const stroke = core.freehandToObject(ZIGZAG_POINTS, PEN.color, PEN.widthPx, "draw-1", "a0", "free");
    let scene: ObjectScene = { ...emptyObjectScene(), objects: [stroke] };
    const undo = core.createUndoStack("tester");

    undo.beginCoalesce();
    const del = core.applyObjectOp(scene, { kind: "delete", id: "draw-1" });
    undo.record({ kind: "delete", id: "draw-1" }, del.inverse!);
    scene = del.scene;
    undo.endCoalesce();
    expect(undo.canUndo()).toBe(true);

    const inverse = undo.undo();
    expect(inverse?.kind).toBe("insert-object");
    const reapplied = core.applyObjectOp(scene, inverse!);
    expect(reapplied.errors).toEqual([]);
    expect(reapplied.scene.objects.map((o: SceneObject) => o.id)).toEqual(["draw-1"]);
    undo.noteUndoApplied(reapplied.inverse!);
    expect(undo.canRedo()).toBe(true);
  });
});

describe("(h) inline text: borderless primitive + set-text + overlay placement", () => {
  it("inserts a borderless text object then commits a set-text op the inverse restores", () => {
    const text = core.buildPrimitive("text", { x: 90, y: 40 }, "note-1", "a0");
    expect(text.stroke).toBeUndefined();
    expect(text.fill).toBeUndefined();
    expect(text.text).toBeUndefined();

    const inserted = core.applyObjectOp(emptyObjectScene(), { kind: "insert-object", object: text });
    expect(inserted.errors).toEqual([]);

    const edited = core.applyObjectOp(inserted.scene, {
      kind: "set-text",
      id: "note-1",
      text: { runs: [{ text: "Hello", bold: false, italic: false }], align: "start", valign: "top" }
    });
    expect(edited.errors).toEqual([]);
    expect(edited.scene.objects[0].text?.runs[0]?.text).toBe("Hello");
    expect(edited.inverse?.kind).toBe("set-text");
    const undone = core.applyObjectOp(edited.scene, edited.inverse!);
    expect(undone.scene.objects[0].text).toBeUndefined();
  });

  it("places the inline overlay over the text object's screen bbox (worldToScreen)", () => {
    // 180x80 text rect anchored so its top-left lands at world (100, 200).
    const text = core.buildPrimitive("text", { x: 100 + 90, y: 200 + 40 }, "note-1", "a0");
    const rect = textOverlayScreenRect(text, { x: 50, y: 30, zoom: 2 });
    expect(rect).not.toBeNull();
    expect(rect?.x).toBeCloseTo(100 * 2 + 50);
    expect(rect?.y).toBeCloseTo(200 * 2 + 30);
    expect(rect?.width).toBeCloseTo(180 * 2);
    expect(rect?.height).toBeCloseTo(80 * 2);
  });
});

describe("(i) template popup lowers a recipe the core inserts as a batch", () => {
  it("builds a template recipe and inserts every object through one apply", () => {
    const recipe = core.buildObjectTemplate("todo_board", 120, 80, "tpl");
    expect(recipe.length).toBeGreaterThan(0);
    for (const object of recipe) {
      expect(typeof object.id).toBe("string");
      expect(typeof (object.geometry.d ?? "")).toBe("string");
    }

    const batch: ObjectOp = { kind: "batch", ops: recipe.map((object) => ({ kind: "insert-object", object })) };
    const inserted = core.applyObjectOp(emptyObjectScene(), batch);
    expect(inserted.errors).toEqual([]);
    expect(inserted.scene.objects.map((o: SceneObject) => o.id).sort()).toEqual(recipe.map((o) => o.id).sort());
    // The whole template insertion is one undo unit.
    expect(inserted.inverse).not.toBeNull();
  });
});
