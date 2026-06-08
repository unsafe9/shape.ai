// W2-07 — shape drag-create: the pure shell pieces that size a primitive to a drag
// bbox, classify which primitives drag-create, and decide whether a create phase
// runs the outline snap (the "modifier nullifies snap" rule). Framework-neutral so
// they pin without a renderer or a Svelte mount (the engine/App only wire them).

import { describe, expect, it } from "vitest";
import { buildPrimitiveObjectFromDrag, type DragSpan } from "../src/client/lib/objectPrimitives";
import { isDragCreateShape } from "../src/client/lib/toolbar";
import { shouldQuerySnap } from "../src/client/renderer/engine";
import { GEOMETRY_QUANTUM_PER_PX } from "../src/shared/object";

const Q = GEOMETRY_QUANTUM_PER_PX;

describe("buildPrimitiveObjectFromDrag (W2-07 bbox sizing)", () => {
  it("sizes a rectangle to the normalized drag bbox and positions it at the top-left", () => {
    const span: DragSpan = { start: { x: 100, y: 200 }, end: { x: 260, y: 300 } };
    const object = buildPrimitiveObjectFromDrag("rectangle", span, "rect-1", "a0");
    // The pure-translation transform sits at the bbox top-left (D7).
    expect(object.transform).toEqual([
      [1, 0, 100],
      [0, 1, 200],
      [0, 0, 1]
    ]);
    // 160 x 100 logical px, object-local quantized integers.
    expect(object.geometry.d).toBe(`M 0 0 L ${160 * Q} 0 L ${160 * Q} ${100 * Q} L 0 ${100 * Q} Z`);
  });

  it("normalizes a drag dragged up-left so width/height stay positive", () => {
    const span: DragSpan = { start: { x: 300, y: 400 }, end: { x: 100, y: 250 } };
    const object = buildPrimitiveObjectFromDrag("rectangle", span, "rect-2", "a0");
    expect(object.transform?.[0][2]).toBe(100);
    expect(object.transform?.[1][2]).toBe(250);
    expect(object.geometry.d).toBe(`M 0 0 L ${200 * Q} 0 L ${200 * Q} ${150 * Q} L 0 ${150 * Q} Z`);
  });

  it("draws a line corner-to-corner (diagonal), anchored at the drag start", () => {
    const span: DragSpan = { start: { x: 50, y: 60 }, end: { x: 150, y: 110 } };
    const object = buildPrimitiveObjectFromDrag("line", span, "line-1", "a0");
    expect(object.transform).toEqual([
      [1, 0, 50],
      [0, 1, 60],
      [0, 0, 1]
    ]);
    // Object-local from (0,0) to the end delta (100, 50).
    expect(object.geometry.d).toBe(`M 0 0 L ${100 * Q} ${50 * Q}`);
  });

  it("sizes an ellipse to the drag bbox (four cubic arcs)", () => {
    const span: DragSpan = { start: { x: 0, y: 0 }, end: { x: 140, y: 140 } };
    const object = buildPrimitiveObjectFromDrag("ellipse", span, "ell-1", "a0");
    expect(object.transform?.[0][2]).toBe(0);
    expect(object.geometry.d.startsWith("M 0")).toBe(true);
    expect(object.geometry.d).toContain("C ");
    expect(object.geometry.d.endsWith("Z")).toBe(true);
  });
});

describe("isDragCreateShape (W2-07 tool routing)", () => {
  it("rect/ellipse/line drag-create", () => {
    expect(isDragCreateShape("rectangle")).toBe(true);
    expect(isDragCreateShape("ellipse")).toBe(true);
    expect(isDragCreateShape("line")).toBe(true);
  });

  it("text/frame insert immediately (not drag-create)", () => {
    expect(isDragCreateShape("text")).toBe(false);
    expect(isDragCreateShape("frame")).toBe(false);
  });
});

describe("shouldQuerySnap (W2-07 modifier nullifies snap)", () => {
  it("snaps on a normal drag move/start/end", () => {
    expect(shouldQuerySnap({ altHeld: false, phase: "start" })).toBe(true);
    expect(shouldQuerySnap({ altHeld: false, phase: "move" })).toBe(true);
    expect(shouldQuerySnap({ altHeld: false, phase: "end" })).toBe(true);
  });

  it("Alt held bypasses snap on every phase", () => {
    expect(shouldQuerySnap({ altHeld: true, phase: "start" })).toBe(false);
    expect(shouldQuerySnap({ altHeld: true, phase: "move" })).toBe(false);
    expect(shouldQuerySnap({ altHeld: true, phase: "end" })).toBe(false);
  });

  it("the cancel phase never snaps (no preview to snap)", () => {
    expect(shouldQuerySnap({ altHeld: false, phase: "cancel" })).toBe(false);
  });
});
