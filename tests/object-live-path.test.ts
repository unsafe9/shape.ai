// FC-16 — the live object path, exercised end-to-end without a GPU.
//
// This drives the same Rust-first object path the live render+input cutover uses,
// as far as is possible in Node/vitest (no WebGPU device):
//   (a) freehandToObject lowers a few world-px points to an insert-able open-path
//       Object carrying a stroke (the pen tool's commit step, FC-11);
//   (b) objectSceneToRenderObjectScene projects a scene of a rectangle + a text
//       note + the freehand object to the renderer-core feed JSON, and the shape
//       is well-formed (stroke widths de-quantized by GEOMETRY_QUANTUM_PER_PX,
//       text preserved) — the projection canvasHost feeds the GPU;
//   (c) apply_object_op (the wasm core) applies an insert-object for the freehand
//       object with no errors and captures the inverse delete (D21).

import { beforeAll, describe, expect, it } from "vitest";

import { objectSceneToRenderObjectScene } from "../src/client/lib/canvasHost";
import { buildPrimitiveObject } from "../src/client/lib/objectPrimitives";
import { ensureSceneCore, loadSceneCore, type SceneCore } from "../src/client/scene/sceneCoreWasm";
import {
  GEOMETRY_QUANTUM_PER_PX,
  emptyObjectScene,
  type Object as SceneObject,
  type ObjectScene,
  type Stroke
} from "../src/shared/object";

let core: SceneCore;

const PEN = { color: "#1f2933", widthPx: 2, epsilon: 2.0 };
const STROKE_POINTS = [
  { x: 100, y: 100 },
  { x: 140, y: 130 },
  { x: 180, y: 110 },
  { x: 220, y: 150 }
];

beforeAll(async () => {
  await ensureSceneCore();
  core = await loadSceneCore();
});

describe("(a) freehandToObject lowers a stroke to an insert-able open-path object", () => {
  it("yields an open path carrying a stroke positioned at the first point", () => {
    const object = core.freehandToObject(STROKE_POINTS, PEN.color, PEN.widthPx, PEN.epsilon, "draw-1", "a0");

    expect(object.id).toBe("draw-1");
    expect(object.order).toBe("a0");
    // A freehand stroke is an OPEN path: the `d` starts with a move and is not
    // closed with a trailing `Z`.
    expect(object.geometry.d.startsWith("M")).toBe(true);
    expect(object.geometry.d.trim().endsWith("Z")).toBe(false);
    // The pen brush lowered to a stroke (no fill — a stroke-only object).
    expect(object.stroke).toBeDefined();
    expect(object.stroke?.paint).toEqual({ kind: "solid", color: PEN.color });
    expect(object.fill).toBeUndefined();
    // Width rides in quantized units (per-px * GEOMETRY_QUANTUM_PER_PX).
    expect((object.stroke as Stroke).width).toBe(PEN.widthPx * GEOMETRY_QUANTUM_PER_PX);
    // Object-local geometry: position lives in the transform (P4 zero-rebake).
    expect(object.transform).toBeDefined();
  });
});

describe("(b) objectSceneToRenderObjectScene projects a heterogeneous scene", () => {
  it("produces a well-formed feed (de-quantized strokes, text preserved)", () => {
    const rectangle = buildPrimitiveObject("rectangle", { x: 0, y: 0 }, "rect-1", "a0");
    // W2-10: the text primitive is borderless + style-less (no default "Note"); set
    // text explicitly to verify the projection preserves an object's text runs.
    const note: SceneObject = { ...buildPrimitiveObject("text", { x: 400, y: 0 }, "note-1", "a1"), text: { runs: [{ text: "Note" }] } };
    const freehand = core.freehandToObject(STROKE_POINTS, PEN.color, PEN.widthPx, PEN.epsilon, "draw-1", "a2");

    const scene: ObjectScene = { ...emptyObjectScene(), objects: [rectangle, note, freehand] };
    const projected = objectSceneToRenderObjectScene(scene, { x: 0, y: 0, zoom: 1 }, { kind: "canvas" }, "test-scene");

    const objects = projected.objects as Array<Record<string, unknown>>;
    expect(objects).toHaveLength(3);
    // Field renaming: geometry.d -> geometryD on every projected object.
    for (const projectedObject of objects) {
      expect(typeof projectedObject.geometryD).toBe("string");
      expect((projectedObject.geometryD as string).length).toBeGreaterThan(0);
    }

    // The rectangle's stroke width is de-quantized to logical px in the feed.
    const projectedRect = objects.find((o) => o.id === "rect-1")!;
    const sourceRectWidth = (rectangle.stroke as Stroke).width;
    expect((projectedRect.stroke as { width: number }).width).toBe(sourceRectWidth / GEOMETRY_QUANTUM_PER_PX);

    // The freehand stroke width is likewise de-quantized.
    const projectedDraw = objects.find((o) => o.id === "draw-1")!;
    expect((projectedDraw.stroke as { width: number }).width).toBe(PEN.widthPx);

    // The text note's text survives the projection.
    const projectedNote = objects.find((o) => o.id === "note-1")!;
    const noteText = projectedNote.text as { runs: Array<{ text: string }> } | null;
    expect(noteText?.runs?.[0]?.text).toBe("Note");
  });
});

describe("(c) apply_object_op inserts the freehand object without errors", () => {
  it("applies an insert-object and captures the inverse delete (D21)", () => {
    const freehand = core.freehandToObject(STROKE_POINTS, PEN.color, PEN.widthPx, PEN.epsilon, "draw-1", "a0");
    const op = { kind: "insert-object", object: freehand } as const;

    const result = core.applyObjectOp(emptyObjectScene(), op);
    expect(result.errors).toEqual([]);
    expect(result.scene.objects.map((o: SceneObject) => o.id)).toEqual(["draw-1"]);
    expect(result.inverse).toEqual({ kind: "delete", id: "draw-1" });
  });
});
