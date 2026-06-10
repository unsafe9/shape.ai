// Anchor-semantics v3 §4 — freehand pen-up recognition + release anchoring,
// driven through the SAME scene-core wasm the live app runs (contract tests,
// like anchor-create.test.ts):
//   (1) freehandToObject RECOGNIZES the stroke — a straight stroke commits as a
//       canonical 2-node open line with its endpoints preserved (the anchoring
//       premise), a circular stroke as a CLOSED ring;
//   (2) the release authors endpoint anchors through synthesizeReleaseAnchors
//       (the same both-corner path as drag-create) when the result is OPEN,
//       and the shell's isOpenClassD gate blocks anchoring for CLOSED results
//       (DU7=(b): anchors live only on open-class endpoints).

import { beforeAll, describe, expect, it } from "vitest";
import { ensureSceneCore, loadSceneCore, type SceneCore } from "../src/client/scene/sceneCoreWasm";
import { synthesizeReleaseAnchors } from "../src/client/lib/objectPrimitives";
import { GEOMETRY_QUANTUM_PER_PX, type Object as SceneObject } from "../src/shared/object";

const Q = GEOMETRY_QUANTUM_PER_PX;

let core: SceneCore;

beforeAll(async () => {
  await ensureSceneCore();
  core = await loadSceneCore();
});

const PEN = { color: "#1f2933", widthPx: 2 };

/** A target rectangle at world top-left (tx,ty), 100x60 logical px. */
function targetRect(id: string, tx: number, ty: number): SceneObject {
  return core.buildPrimitiveFromDrag("rectangle", { start: { x: tx, y: ty }, end: { x: tx + 100, y: ty + 60 } }, id, "a0");
}

/** The world position of an object's geometry node `i` under its transform. */
function worldNode(obj: SceneObject, i: number): { x: number; y: number } {
  const nums = obj.geometry.d.match(/-?\d+(?:\.\d+)?/g)!;
  const lx = Number(nums[i * 2]) / Q;
  const ly = Number(nums[i * 2 + 1]) / Q;
  const t = obj.transform;
  return { x: t ? t[0][0] * lx + t[0][1] * ly + t[0][2] : lx, y: t ? t[1][0] * lx + t[1][1] * ly + t[1][2] : ly };
}

/** `n+1` samples along the segment a -> b (inclusive). */
function strokeAlong(a: { x: number; y: number }, b: { x: number; y: number }, n = 10): { x: number; y: number }[] {
  return Array.from({ length: n + 1 }, (_, i) => ({
    x: a.x + ((b.x - a.x) * i) / n,
    y: a.y + ((b.y - a.y) * i) / n
  }));
}

/** A closed circular stroke around (cx,cy), radius r (36 samples, pen-up short of the start). */
function circleStroke(cx: number, cy: number, r: number): { x: number; y: number }[] {
  return Array.from({ length: 36 }, (_, i) => {
    const theta = (i * 10 * Math.PI) / 180;
    return { x: cx + r * Math.cos(theta), y: cy + r * Math.sin(theta) };
  });
}

describe("(1) freehandToObject recognizes the pen-up stroke (v3 §4)", () => {
  it("commits a straight stroke as a canonical 2-node OPEN line with preserved endpoints", () => {
    const stroke = core.freehandToObject(strokeAlong({ x: 300, y: 30 }, { x: 500, y: 30 }), PEN.color, PEN.widthPx, "draw-1", "a2");
    expect(stroke.geometry.d).toBe(`M 0 0 L ${200 * Q} 0`);
    expect(core.isOpenClassD(stroke.geometry.d)).toBe(true);
    // Endpoint preservation — the anchoring premise: the recognized endpoints
    // sit exactly at the input start/end in world space.
    expect(worldNode(stroke, 0)).toEqual({ x: 300, y: 30 });
    expect(worldNode(stroke, 1)).toEqual({ x: 500, y: 30 });
    expect(stroke.stroke?.width).toBe(PEN.widthPx * Q);
  });

  it("commits a circular stroke as a CLOSED ring (not open-class)", () => {
    const stroke = core.freehandToObject(circleStroke(100, 100, 40), PEN.color, PEN.widthPx, "draw-2", "a2");
    expect(stroke.geometry.d.trim().endsWith("Z")).toBe(true);
    expect(core.isOpenClassD(stroke.geometry.d)).toBe(false);
  });

  it("commits a rough rectangular stroke as an axis-snapped closed rect", () => {
    // A perimeter walk of (0,0)-(120,80): exact corners, sub-epsilon inward
    // edge noise, pen-up a little short of the start.
    const rectStroke = [
      { x: 0, y: 0 }, { x: 30, y: 1 }, { x: 60, y: 1.2 }, { x: 90, y: 0.8 },
      { x: 120, y: 0 }, { x: 119, y: 26 }, { x: 119.2, y: 53 },
      { x: 120, y: 80 }, { x: 90, y: 79 }, { x: 60, y: 79.2 }, { x: 30, y: 78.8 },
      { x: 0, y: 80 }, { x: 1, y: 55 }, { x: 0.8, y: 30 }, { x: 0, y: 10 }
    ];
    const stroke = core.freehandToObject(rectStroke, PEN.color, PEN.widthPx, "draw-3", "a2");
    expect(stroke.geometry.d).toBe(`M 0 0 L ${120 * Q} 0 L ${120 * Q} ${80 * Q} L 0 ${80 * Q} Z`);
  });
});

describe("(2) freehand release anchoring (v3 §4 — open results, both corners)", () => {
  it("binds BOTH stroke endpoints to their snapped targets via synthesizeReleaseAnchors", () => {
    // rect-a outline spans x∈[200,300]; rect-b x∈[500,600] — a stroke drawn
    // edge-to-edge between them (the start snap and the release snap).
    const rectA = targetRect("rect-a", 200, 0);
    const rectB = targetRect("rect-b", 500, 0);
    const stroke = core.freehandToObject(strokeAlong({ x: 300, y: 30 }, { x: 500, y: 30 }), PEN.color, PEN.widthPx, "draw-1", "a2");
    expect(core.isOpenClassD(stroke.geometry.d)).toBe(true); // the shell's anchor gate

    const anchors = synthesizeReleaseAnchors(core, [rectA, rectB], stroke, [
      { target: "rect-a", at: { x: 300, y: 30 } },
      { target: "rect-b", at: { x: 500, y: 30 } }
    ]);
    expect(anchors).toHaveLength(2);
    // Start corner: node 0 onto rect-a's right edge (target-local (100,30)px).
    expect(anchors[0].target).toBe("rect-a");
    expect(anchors[0].nodeIndex).toBe(0);
    expect(anchors[0].at).toEqual({ x: 100 * Q, y: 30 * Q });
    // End corner: node 1 (the last node) onto rect-b's left edge.
    expect(anchors[1].target).toBe("rect-b");
    expect(anchors[1].nodeIndex).toBe(1);
    expect(anchors[1].at).toEqual({ x: 0, y: 30 * Q });
  });

  it("skips null corners and corners whose target left the scene", () => {
    const rectA = targetRect("rect-a", 200, 0);
    const stroke = core.freehandToObject(strokeAlong({ x: 300, y: 30 }, { x: 500, y: 30 }), PEN.color, PEN.widthPx, "draw-1", "a2");
    const anchors = synthesizeReleaseAnchors(core, [rectA], stroke, [
      null, // the stroke never started on an edge
      { target: "rect-gone", at: { x: 500, y: 30 } } // stale release target
    ]);
    expect(anchors).toEqual([]);
  });

  it("a degenerate tap binds at most one anchor per node (D5: no duplicate node bindings)", () => {
    // A pen tap on an edge recognizes as a zero-length 2-node line whose nodes
    // coincide — both corners resolve to the same nearest node, and only the
    // first binding survives.
    const rectA = targetRect("rect-a", 200, 0);
    const tap = core.freehandToObject(strokeAlong({ x: 300, y: 30 }, { x: 300, y: 30 }), PEN.color, PEN.widthPx, "draw-4", "a2");
    const anchors = synthesizeReleaseAnchors(core, [rectA], tap, [
      { target: "rect-a", at: { x: 300, y: 30 } },
      { target: "rect-a", at: { x: 300, y: 30 } }
    ]);
    expect(anchors).toHaveLength(1);
    expect(anchors[0].target).toBe("rect-a");
  });

  it("a CLOSED recognition fails the shell's open-class gate, so no anchors are authored (DU7=(b))", () => {
    // A circle drawn ON a target's edge still recognizes closed — and the shell
    // only calls the anchor path for open-class results.
    const stroke = core.freehandToObject(circleStroke(300, 30, 25), PEN.color, PEN.widthPx, "draw-2", "a2");
    expect(core.isOpenClassD(stroke.geometry.d)).toBe(false);
  });
});
