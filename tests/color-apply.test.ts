// AP1 (#5) — color apply. The toolbar's selected color must (a) become the default
// fill/stroke of a NEW shape and (b) author a `set-style` op when a selected object
// is recolored. Tier-3 moved these builders into the Rust core; these are contract
// tests over the REAL scene-core wasm (`core.buildPrimitive` /
// `buildPrimitiveFromDrag` / `buildSetStyleOp`), plus assert the App.svelte wiring
// threads the color through the insert path and routes onSelectColor.

import { beforeAll, describe, expect, it } from "vitest";
import { THEME_DEFAULT_COLOR, type DragSpan } from "../platforms/web/controller/objectPrimitives";
import { buildColorApplyOp, buildInsertPrimitive } from "../platforms/web/controller/interactions";
import { ensureSceneCore, loadSceneCore, type SceneCore } from "../platforms/web/bridge/sceneCoreWasm";
import { emptyObjectScene, type Object as SceneObject, type ObjectScene } from "../platforms/web/shared/object";

const COLOR = "#abcdef";

let core: SceneCore;
beforeAll(async () => {
  await ensureSceneCore();
  core = await loadSceneCore();
});

describe("buildPrimitive default color (AP1 insert path)", () => {
  it("paints a NEW rectangle's fill AND stroke in the selected color", () => {
    const object = core.buildPrimitive("rectangle", { x: 0, y: 0 }, "rect-1", "a0", COLOR);
    expect(object.fill?.paint).toEqual({ kind: "solid", color: COLOR });
    expect(object.stroke?.paint).toEqual({ kind: "solid", color: COLOR });
  });

  it("paints a NEW line's stroke (no fill) in the selected color", () => {
    const object = core.buildPrimitive("line", { x: 0, y: 0 }, "line-1", "a0", COLOR);
    expect(object.fill).toBeUndefined();
    expect(object.stroke?.paint).toEqual({ kind: "solid", color: COLOR });
  });

  it("keeps the stroke width while swapping only the paint color", () => {
    const def = core.buildPrimitive("rectangle", { x: 0, y: 0 }, "rect-w", "a0");
    const colored = core.buildPrimitive("rectangle", { x: 0, y: 0 }, "rect-w2", "a0", COLOR);
    expect(colored.stroke?.width).toBe(def.stroke?.width);
  });

  it("falls back to the hardcoded default fill when no color is selected", () => {
    const object = core.buildPrimitive("rectangle", { x: 0, y: 0 }, "rect-d", "a0");
    expect(object.fill?.paint).toEqual({ kind: "solid", color: "#e8eefc" });
  });
});

describe("buildPrimitiveFromDrag default color (AP1 drag-create path)", () => {
  it("paints an ellipse dragged to a bbox in the selected color", () => {
    const span: DragSpan = { start: { x: 0, y: 0 }, end: { x: 100, y: 80 } };
    const object = core.buildPrimitiveFromDrag("ellipse", span, "ell-1", "a0", COLOR);
    expect(object.fill?.paint).toEqual({ kind: "solid", color: COLOR });
    expect(object.stroke?.paint).toEqual({ kind: "solid", color: COLOR });
  });
});

describe("buildSetStyleOp recolor (AP1 recolor-selection path)", () => {
  // Closed d: the closed-class recolor contract (anchor-semantics v3 §1 routes
  // OPEN-class color to the stroke — see the dedicated describe below).
  function obj(extra: Partial<SceneObject>): SceneObject {
    return { id: "o1", order: "a0", geometry: { d: "M 0 0 L 8 0 L 8 8 Z" }, ...extra } as SceneObject;
  }

  it("emits a set-style op recoloring both fill and stroke of a filled shape", () => {
    const object = obj({
      fill: { paint: { kind: "solid", color: "#000000" }, opacity: 1 },
      stroke: { paint: { kind: "solid", color: "#111111" }, width: 16 }
    });
    const op = core.buildSetStyleOp(object, COLOR);
    // The core re-serializes the canonical Stroke, so the recolored stroke carries
    // its full field set (the serde defaults the minimal fixture omitted).
    expect(op).toEqual({
      kind: "set-style",
      id: "o1",
      fill: { action: "set", value: { paint: { kind: "solid", color: COLOR }, opacity: 1 } },
      stroke: {
        action: "set",
        value: { paint: { kind: "solid", color: COLOR }, width: 16, opacity: 1, cap: "butt", join: "miter" }
      }
    });
  });

  it("recolors only the stroke for a stroke-only object (no fill added)", () => {
    const object = obj({ stroke: { paint: { kind: "solid", color: "#111111" }, width: 16 } });
    const op = core.buildSetStyleOp(object, COLOR);
    expect(op.kind).toBe("set-style");
    if (op.kind !== "set-style") throw new Error("expected set-style");
    expect(op.fill).toBeUndefined();
    expect(op.stroke).toEqual({
      action: "set",
      value: { paint: { kind: "solid", color: COLOR }, width: 16, opacity: 1, cap: "butt", join: "miter" }
    });
  });

  it("gives a styleless (borderless text) object a fill so the recolor is visible", () => {
    const object = obj({});
    const op = core.buildSetStyleOp(object, COLOR);
    if (op.kind !== "set-style") throw new Error("expected set-style");
    expect(op.fill).toEqual({ action: "set", value: { paint: { kind: "solid", color: COLOR }, opacity: 1 } });
    expect(op.stroke).toBeUndefined();
  });
});

describe("open-class recolor routes to the stroke (anchor-semantics v3 §1)", () => {
  // Open d (one open subpath): the color must reach the STROKE and never author
  // a fill — open-class carries no fill (the shell stays class-ignorant; the
  // routing lives in the core's build_set_style_op).
  function openObj(extra: Partial<SceneObject>): SceneObject {
    return { id: "o1", order: "a0", geometry: { d: "M 0 0 L 8 0" }, ...extra } as SceneObject;
  }

  it("recolors the stroke and leaves a legacy fill untouched", () => {
    const object = openObj({
      fill: { paint: { kind: "solid", color: "#000000" }, opacity: 1 },
      stroke: { paint: { kind: "solid", color: "#111111" }, width: 16 }
    });
    const op = core.buildSetStyleOp(object, COLOR);
    if (op.kind !== "set-style") throw new Error("expected set-style");
    expect(op.fill).toBeUndefined();
    expect(op.stroke).toEqual({
      action: "set",
      value: { paint: { kind: "solid", color: COLOR }, width: 16, opacity: 1, cap: "butt", join: "miter" }
    });
  });

  it("gives a strokeless open path the line-default stroke, NOT a fill", () => {
    const op = core.buildSetStyleOp(openObj({}), COLOR);
    if (op.kind !== "set-style") throw new Error("expected set-style");
    expect(op.fill).toBeUndefined();
    expect(op.stroke).toEqual({
      action: "set",
      value: { paint: { kind: "solid", color: COLOR }, width: 16, opacity: 1, cap: "butt", join: "miter" }
    });
  });

  it("classifies via the core bridge the shell consults (isOpenClassD)", () => {
    expect(core.isOpenClassD("M 0 0 L 8 0")).toBe(true);
    expect(core.isOpenClassD("M 0 0 L 8 0 L 8 8 Z")).toBe(false);
    expect(core.isOpenClassD("M 0 0 L 8 0 M 16 0 L 24 0")).toBe(false);
  });
});

describe("theme-default token (S2 / #5 — resolves through the core)", () => {
  it("paints a NEW shape authored with the sentinel as a text token (fill AND stroke)", () => {
    const object = core.buildPrimitive("rectangle", { x: 0, y: 0 }, "rect-t", "a0", THEME_DEFAULT_COLOR);
    expect(object.fill?.paint).toEqual({ kind: "token", name: "text" });
    expect(object.stroke?.paint).toEqual({ kind: "token", name: "text" });
  });

  it("maps a real hex to a solid paint (no token)", () => {
    const object = core.buildPrimitive("rectangle", { x: 0, y: 0 }, "rect-h", "a0", "#ef4444");
    expect(object.fill?.paint).toEqual({ kind: "solid", color: "#ef4444" });
  });

  it("paints a drag-created shape authored with the sentinel as a token", () => {
    const span: DragSpan = { start: { x: 0, y: 0 }, end: { x: 60, y: 40 } };
    const object = core.buildPrimitiveFromDrag("ellipse", span, "ell-t", "a0", THEME_DEFAULT_COLOR);
    expect(object.fill?.paint).toEqual({ kind: "token", name: "text" });
    expect(object.stroke?.paint).toEqual({ kind: "token", name: "text" });
  });

  it("recolors a selected object to the token via set-style when the sentinel is picked", () => {
    const object = {
      id: "o1",
      order: "a0",
      geometry: { d: "M 0 0 L 8 0" },
      stroke: { paint: { kind: "solid", color: "#111111" }, width: 16 }
    } as SceneObject;
    const op = core.buildSetStyleOp(object, THEME_DEFAULT_COLOR);
    if (op.kind !== "set-style") throw new Error("expected set-style");
    expect(op.stroke).toEqual({
      action: "set",
      value: { paint: { kind: "token", name: "text" }, width: 16, opacity: 1, cap: "butt", join: "miter" }
    });
  });
});

// The insertPrimitive / applySelectedColor wiring, exercised through the extracted
// controller functions the shell now composes (no .svelte source pin).
describe("controller color wiring (AP1)", () => {
  function sceneWith(object: SceneObject): ObjectScene {
    return { ...emptyObjectScene(), objects: [object] };
  }

  it("threads selectedColor into the immediate-insert core builder", () => {
    // buildInsertPrimitive paints the new object in the selected color (vs the
    // kind default when no color is passed).
    const object = buildInsertPrimitive(core, "rectangle", { x: 0, y: 0 }, "rect-1", "a0", COLOR);
    expect(object.fill?.paint).toEqual({ kind: "solid", color: COLOR });
    expect(object.stroke?.paint).toEqual({ kind: "solid", color: COLOR });
  });

  it("authors a set-style op (core.buildSetStyleOp) when a single object is selected", () => {
    const object = { id: "o1", order: "a0", geometry: { d: "M 0 0 L 8 0" }, stroke: { paint: { kind: "solid", color: "#111111" }, width: 16 } } as SceneObject;
    const op = buildColorApplyOp(core, sceneWith(object), { kind: "object", id: "o1" }, COLOR);
    expect(op).not.toBeNull();
    expect(op!.kind).toBe("set-style");
    if (op!.kind !== "set-style") throw new Error("expected set-style");
    expect(op!.id).toBe("o1");
  });

  it("authors NO recolor op when the selection is not a single object (color still adopted shell-side)", () => {
    const object = { id: "o1", order: "a0", geometry: { d: "M 0 0 L 8 0" } } as SceneObject;
    const scene = sceneWith(object);
    expect(buildColorApplyOp(core, scene, { kind: "canvas" }, COLOR)).toBeNull();
    expect(buildColorApplyOp(core, scene, { kind: "multi", ids: ["o1"] }, COLOR)).toBeNull();
  });
});
