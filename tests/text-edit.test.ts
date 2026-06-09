// W2-10 — inline text editing + the borderless text primitive: the pure shell
// pieces. The text primitive must be a borderless, style-less rect (no stroke, no
// fill, no default "Note" text), and `textOverlayScreenRect` must place the inline
// contenteditable overlay over the object's screen bbox via worldToScreen.
// Framework-neutral so they pin without a renderer or a Svelte mount.

import { beforeAll, describe, expect, it } from "vitest";
import { textOverlayScreenRect, type DragSpan } from "../src/client/lib/objectPrimitives";
import { ensureSceneCore, loadSceneCore, type SceneCore } from "../src/client/scene/sceneCoreWasm";
import { GEOMETRY_QUANTUM_PER_PX } from "../src/shared/object";

const Q = GEOMETRY_QUANTUM_PER_PX;

let core: SceneCore;
beforeAll(async () => {
  await ensureSceneCore();
  core = await loadSceneCore();
});

describe("text primitive (W2-10 borderless, style-less)", () => {
  it("is a borderless rect: no stroke, no fill, no default text", () => {
    const object = core.buildPrimitive("text", { x: 0, y: 0 }, "text-1", "a0");
    expect(object.stroke).toBeUndefined();
    expect(object.fill).toBeUndefined();
    expect(object.text).toBeUndefined();
    // Still a rect geometry the inline editor can size to.
    expect(object.geometry.d).toBe(`M 0 0 L ${180 * Q} 0 L ${180 * Q} ${80 * Q} L 0 ${80 * Q} Z`);
  });

  it("rectangle still carries its border + fill (text-only stripping)", () => {
    const object = core.buildPrimitive("rectangle", { x: 0, y: 0 }, "rect-1", "a0");
    expect(object.stroke).toBeDefined();
    expect(object.fill).toBeDefined();
  });
});

describe("textOverlayScreenRect (W2-10 overlay placement)", () => {
  it("maps the object world bbox to screen via the camera (translation + zoom)", () => {
    // A 180x80 text rect anchored so its top-left lands at world (100, 200).
    const object = core.buildPrimitive("text", { x: 100 + 90, y: 200 + 40 }, "text-1", "a0");
    const rect = textOverlayScreenRect(object, { x: 50, y: 30, zoom: 2 });
    expect(rect).not.toBeNull();
    // worldToScreen: screen = world * zoom + cameraOffset.
    expect(rect?.x).toBeCloseTo(100 * 2 + 50);
    expect(rect?.y).toBeCloseTo(200 * 2 + 30);
    expect(rect?.width).toBeCloseTo(180 * 2);
    expect(rect?.height).toBeCloseTo(80 * 2);
  });

  it("tracks the object's transform (a dragged text rect's top-left)", () => {
    const span: DragSpan = { start: { x: 300, y: 400 }, end: { x: 100, y: 250 } };
    const object = core.buildPrimitiveFromDrag("text", span, "text-2", "a0");
    const rect = textOverlayScreenRect(object, { x: 0, y: 0, zoom: 1 });
    // Normalized bbox top-left is (100, 250); identity camera => same screen coords.
    expect(rect?.x).toBeCloseTo(100);
    expect(rect?.y).toBeCloseTo(250);
    expect(rect?.width).toBeCloseTo(200);
    expect(rect?.height).toBeCloseTo(150);
  });

  it("returns null for an empty path", () => {
    expect(textOverlayScreenRect({ id: "x", order: "a0", geometry: { d: "" } }, { x: 0, y: 0, zoom: 1 })).toBeNull();
  });
});
