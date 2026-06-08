// AP5 (#14) — drag-create anchoring: a snapped drag-create synthesizes a
// persistent D5 anchor binding the created object's endpoint to the snap target,
// the endpoint reprojects as the target moves (move-together), and an Alt-create
// (snap bypassed upstream, so no target) authors no anchor. Pure shell pieces, so
// they pin without a renderer or a Svelte mount — the App/engine only wire them.

import { describe, expect, it } from "vitest";
import { reprojectAnchoredEndpoint, synthesizeCreateAnchors } from "../src/client/lib/anchorCreate";
import { buildPrimitiveObjectFromDrag, type DragSpan } from "../src/client/lib/objectPrimitives";
import {
  GEOMETRY_QUANTUM_PER_PX,
  translateTransform,
  type Anchor,
  type Object as SceneObject
} from "../src/shared/object";

const Q = GEOMETRY_QUANTUM_PER_PX;

/** A target rectangle at world top-left (tx,ty), 100x60 logical px. */
function targetRect(id: string, tx: number, ty: number): SceneObject {
  return buildPrimitiveObjectFromDrag("rectangle", { start: { x: tx, y: ty }, end: { x: tx + 100, y: ty + 60 } }, id, "a0");
}

describe("synthesizeCreateAnchors (AP5 snapped drag-create binds the endpoint)", () => {
  it("authors one anchor referencing the snap target, addressing the dragged endpoint node", () => {
    const target = targetRect("rect-a", 200, 0); // outline spans world x∈[200,300], y∈[0,60]
    // A line dragged from (40,30) to the target's left edge (200,30): the corner snapped.
    const span: DragSpan = { start: { x: 40, y: 30 }, end: { x: 200, y: 30 } };
    const line = buildPrimitiveObjectFromDrag("line", span, "edge-1", "a1");

    const anchors = synthesizeCreateAnchors(line, target, span.end);
    expect(anchors).toBeDefined();
    expect(anchors!).toHaveLength(1);
    const anchor = anchors![0];
    expect(anchor.target).toBe("rect-a");
    // The line's node 1 is the dragged endpoint (node 0 is the start origin).
    expect(anchor.nodeIndex).toBe(1);
    // `at` is the snapped WORLD point (200,30) in the target's LOCAL quantized space
    // (target translation is (200,0)): local (0,30) -> quantized (0, 30*Q).
    expect(anchor.at).toEqual({ x: 0, y: 30 * Q });
  });

  it("authors NO anchor when there is no snap target (Alt-create bypasses snap upstream)", () => {
    const span: DragSpan = { start: { x: 40, y: 30 }, end: { x: 200, y: 30 } };
    const line = buildPrimitiveObjectFromDrag("line", span, "edge-1", "a1");
    // The engine reports targetId=null on an Alt-create; the shell passes undefined.
    expect(synthesizeCreateAnchors(line, undefined, span.end)).toBeUndefined();
  });

  it("never anchors an object onto itself", () => {
    const span: DragSpan = { start: { x: 0, y: 0 }, end: { x: 100, y: 0 } };
    const line = buildPrimitiveObjectFromDrag("line", span, "edge-1", "a1");
    expect(synthesizeCreateAnchors(line, line, span.end)).toBeUndefined();
  });
});

describe("reprojectAnchoredEndpoint (AP5 move-together)", () => {
  it("reprojects the anchored endpoint through the target's CURRENT transform", () => {
    const target = targetRect("rect-a", 200, 0);
    const span: DragSpan = { start: { x: 40, y: 30 }, end: { x: 200, y: 30 } };
    const line = buildPrimitiveObjectFromDrag("line", span, "edge-1", "a1");
    const anchor = synthesizeCreateAnchors(line, target, span.end)![0];

    // At rest the endpoint resolves back to the snap point (200,30).
    expect(reprojectAnchoredEndpoint(target, anchor)).toEqual({ x: 200, y: 30 });

    // Move the target +50 x / +20 y: the endpoint tracks it (250,50) — moves WITH it.
    const moved: SceneObject = { ...target, transform: translateTransform(250, 20) };
    expect(reprojectAnchoredEndpoint(moved, anchor)).toEqual({ x: 250, y: 50 });
  });

  it("a non-anchored endpoint is the falsifying control: it does NOT track the target", () => {
    // Without an anchor the line's endpoint is its fixed object-local geometry under
    // its own transform — it stays put when the (would-be) target moves. This is the
    // behavior the anchor must override; if the move-together test above passed only
    // because the endpoint is constant, this control would also "track" and fail.
    const target = targetRect("rect-a", 200, 0);
    const anchor: Anchor = synthesizeCreateAnchors(
      buildPrimitiveObjectFromDrag("line", { start: { x: 40, y: 30 }, end: { x: 200, y: 30 } }, "edge-1", "a1"),
      target,
      { x: 200, y: 30 }
    )![0];
    const moved: SceneObject = { ...target, transform: translateTransform(250, 20) };
    const at = reprojectAnchoredEndpoint(moved, anchor);
    expect(at).not.toEqual({ x: 200, y: 30 });
  });
});
