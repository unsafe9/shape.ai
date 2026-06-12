import { beforeAll, describe, expect, it } from "vitest";
import {
  type DragSpan,
  type CreateSnap,
  resolveCreateRelease
} from "../controller/objectPrimitives";
import { ensureSceneCore, loadSceneCore, type SceneCore } from "../bridge/sceneCoreWasm";
import { isDragCreateShape } from "../controller/toolbar";
import { shouldQuerySnap } from "../renderer/engine";
import { GEOMETRY_QUANTUM_PER_PX } from "../shared/object";

const Q = GEOMETRY_QUANTUM_PER_PX;

describe("sceneCore.buildPrimitiveFromDrag (bbox sizing)", () => {
  let core: SceneCore;
  beforeAll(async () => {
    await ensureSceneCore();
    core = await loadSceneCore();
  });

  it("sizes a rectangle to the normalized drag bbox and positions it at the top-left", () => {
    const span: DragSpan = { start: { x: 100, y: 200 }, end: { x: 260, y: 300 } };
    const object = core.buildPrimitiveFromDrag("rectangle", span, "rect-1", "a0");
    // Pure-translation transform at the bbox top-left.
    expect(object.transform).toEqual([
      [1, 0, 100],
      [0, 1, 200],
      [0, 0, 1]
    ]);
    // 160 x 100 logical px, object-local quantized integers.
    expect((object.geometry.d ?? "")).toBe(`M 0 0 L ${160 * Q} 0 L ${160 * Q} ${100 * Q} L 0 ${100 * Q} Z`);
  });

  it("normalizes a drag dragged up-left so width/height stay positive", () => {
    const span: DragSpan = { start: { x: 300, y: 400 }, end: { x: 100, y: 250 } };
    const object = core.buildPrimitiveFromDrag("rectangle", span, "rect-2", "a0");
    expect(object.transform?.[0][2]).toBe(100);
    expect(object.transform?.[1][2]).toBe(250);
    expect((object.geometry.d ?? "")).toBe(`M 0 0 L ${200 * Q} 0 L ${200 * Q} ${150 * Q} L 0 ${150 * Q} Z`);
  });

  it("draws a line corner-to-corner (diagonal), anchored at the drag start", () => {
    const span: DragSpan = { start: { x: 50, y: 60 }, end: { x: 150, y: 110 } };
    const object = core.buildPrimitiveFromDrag("line", span, "line-1", "a0");
    expect(object.transform).toEqual([
      [1, 0, 50],
      [0, 1, 60],
      [0, 0, 1]
    ]);
    expect((object.geometry.d ?? "")).toBe(`M 0 0 L ${100 * Q} ${50 * Q}`);
  });

  it("sizes an ellipse to the drag bbox (four cubic arcs)", () => {
    const span: DragSpan = { start: { x: 0, y: 0 }, end: { x: 140, y: 140 } };
    const object = core.buildPrimitiveFromDrag("ellipse", span, "ell-1", "a0");
    expect(object.transform?.[0][2]).toBe(0);
    expect((object.geometry.d ?? "").startsWith("M 0")).toBe(true);
    expect((object.geometry.d ?? "")).toContain("C ");
    expect((object.geometry.d ?? "").endsWith("Z")).toBe(true);
  });
});

describe("isDragCreateShape (tool routing)", () => {
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

describe("shouldQuerySnap (modifier nullifies snap)", () => {
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

describe("resolveCreateRelease (anchor-on-release reuse)", () => {
  const lastSnap: CreateSnap = { at: { x: 200, y: 30 }, target: "rect-a" };

  it("honors the release-time snap when it hit (no reuse needed)", () => {
    const r = resolveCreateRelease(
      { end: { x: 201, y: 31 }, snapped: true, target: "rect-a" },
      null,
      24
    );
    expect(r).toEqual({ end: { x: 201, y: 31 }, target: "rect-a" });
  });

  it("reuses the gesture's last snap when the release MISSED but landed within tolerance", () => {
    // Release missed the 8px snap but is ~6.4 world units from the last snap, inside
    // tolerance 24, so the endpoint is pulled onto the edge point.
    const r = resolveCreateRelease(
      { end: { x: 205, y: 33 }, snapped: false, target: null },
      lastSnap,
      24
    );
    expect(r).toEqual({ end: { x: 200, y: 30 }, target: "rect-a" });
  });

  it("does NOT reuse when the release missed and is FAR from the last snap (deliberate empty release)", () => {
    const r = resolveCreateRelease(
      { end: { x: 400, y: 400 }, snapped: false, target: null },
      lastSnap,
      24
    );
    expect(r).toEqual({ end: { x: 400, y: 400 }, target: null });
  });

  it("authors nothing when the gesture never snapped and the release missed", () => {
    const r = resolveCreateRelease(
      { end: { x: 50, y: 50 }, snapped: false, target: null },
      null,
      24
    );
    expect(r).toEqual({ end: { x: 50, y: 50 }, target: null });
  });

  it("reuse boundary is the tolerance radius (just inside = reuse, just outside = drop)", () => {
    const justInside = resolveCreateRelease(
      { end: { x: 200 + 23, y: 30 }, snapped: false, target: null },
      lastSnap,
      24
    );
    expect(justInside.target).toBe("rect-a");
    const justOutside = resolveCreateRelease(
      { end: { x: 200 + 25, y: 30 }, snapped: false, target: null },
      lastSnap,
      24
    );
    expect(justOutside.target).toBeNull();
  });
});
