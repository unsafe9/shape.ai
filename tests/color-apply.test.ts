// AP1 (#5) — color apply. The toolbar's selected color must (a) become the default
// fill/stroke of a NEW shape and (b) author a `set-style` op when a selected object
// is recolored. Tier-3 moved these builders into the Rust core; these are contract
// tests over the REAL scene-core wasm (`core.buildPrimitive` /
// `buildPrimitiveFromDrag` / `buildSetStyleOp`), plus assert the App.svelte wiring
// threads the color through the insert path and routes onSelectColor.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { beforeAll, describe, expect, it } from "vitest";
import { THEME_DEFAULT_COLOR, type DragSpan } from "../src/client/lib/objectPrimitives";
import { ensureSceneCore, loadSceneCore, type SceneCore } from "../src/client/scene/sceneCoreWasm";
import type { Object as SceneObject } from "../src/shared/object";

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
  function obj(extra: Partial<SceneObject>): SceneObject {
    return { id: "o1", order: "a0", geometry: { d: "M 0 0 L 8 0" }, ...extra } as SceneObject;
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

describe("App.svelte color wiring (AP1)", () => {
  // No DOM in the node test env: assert the wiring against the .svelte source.
  // Falsifiable — dropping the selectedColor arg from the insert path, the
  // onSelectColor route, or the set-style authoring all fail these.
  const source = readFileSync(fileURLToPath(new URL("../src/client/svelte/App.svelte", import.meta.url)), "utf8");

  it("threads selectedColor into the immediate-insert core builder", () => {
    expect(source).toMatch(/sceneCore\.buildPrimitive\(kind, center, freshId\(kind\), nextOrderKey\(\), selectedColor\)/);
  });

  it("routes the toolbar onSelectColor to applySelectedColor", () => {
    expect(source).toMatch(/onSelectColor=\{applySelectedColor\}/);
  });

  it("authors a set-style op (core.buildSetStyleOp) when a single object is selected", () => {
    expect(source).toMatch(/authorOp\(sceneCore\.buildSetStyleOp\(object, color\)\)/);
    expect(source).toMatch(/selection\.kind !== "object"/);
  });
});
