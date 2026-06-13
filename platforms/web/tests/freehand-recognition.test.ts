import { beforeAll, describe, expect, it } from "vitest";
import { ensureSceneCore, loadSceneCore, type SceneCore } from "../bridge/sceneCoreWasm";
import { GEOMETRY_QUANTUM_PER_PX, type Object as SceneObject, type ObjectScene } from "../shared/object";

const Q = GEOMETRY_QUANTUM_PER_PX;

let core: SceneCore;

beforeAll(async () => {
  await ensureSceneCore();
  core = await loadSceneCore();
});

const PEN = { color: "#1f2933", widthPx: 2 };

function sceneOf(objects: SceneObject[]): ObjectScene {
  return { sceneVersion: 1, objects, tags: [], selection: { kind: "canvas" }, updatedAt: "" };
}

function targetRect(id: string, tx: number, ty: number): SceneObject {
  return core.buildPrimitiveFromDrag("rectangle", { start: { x: tx, y: ty }, end: { x: tx + 100, y: ty + 60 } }, id, "a0");
}

// World position of geometry node `i` under the object's transform.
function worldNode(obj: SceneObject, i: number): { x: number; y: number } {
  const nums = (obj.geometry.d ?? "").match(/-?\d+(?:\.\d+)?/g)!;
  const lx = Number(nums[i * 2]) / Q;
  const ly = Number(nums[i * 2 + 1]) / Q;
  const t = obj.transform;
  return { x: t ? t[0][0] * lx + t[0][1] * ly + t[0][2] : lx, y: t ? t[1][0] * lx + t[1][1] * ly + t[1][2] : ly };
}

// n+1 samples along the segment a -> b (inclusive).
function strokeAlong(a: { x: number; y: number }, b: { x: number; y: number }, n = 10): { x: number; y: number }[] {
  return Array.from({ length: n + 1 }, (_, i) => ({
    x: a.x + ((b.x - a.x) * i) / n,
    y: a.y + ((b.y - a.y) * i) / n
  }));
}

// Closed circular stroke around (cx,cy), radius r (36 samples, pen-up short of start).
function circleStroke(cx: number, cy: number, r: number): { x: number; y: number }[] {
  return Array.from({ length: 36 }, (_, i) => {
    const theta = (i * 10 * Math.PI) / 180;
    return { x: cx + r * Math.cos(theta), y: cy + r * Math.sin(theta) };
  });
}

describe("(1) freehandToObject recognizes the pen-up stroke", () => {
  it("commits a straight stroke as a canonical 2-node OPEN line with preserved endpoints", () => {
    const stroke = core.freehandToObject(strokeAlong({ x: 300, y: 30 }, { x: 500, y: 30 }), PEN.color, PEN.widthPx, "draw-1", "a2", "free");
    expect((stroke.geometry.d ?? "")).toBe(`M 0 0 L ${200 * Q} 0`);
    expect(core.isOpenClassD((stroke.geometry.d ?? ""))).toBe(true);
    // Endpoint preservation (the anchoring premise): recognized endpoints sit at
    // the input start/end in world space.
    expect(worldNode(stroke, 0)).toEqual({ x: 300, y: 30 });
    expect(worldNode(stroke, 1)).toEqual({ x: 500, y: 30 });
    expect(stroke.stroke?.width).toBe(PEN.widthPx * Q);
  });

  it("commits a circular stroke as a CLOSED ring (not open-class)", () => {
    const stroke = core.freehandToObject(circleStroke(100, 100, 40), PEN.color, PEN.widthPx, "draw-2", "a2", "free");
    expect((stroke.geometry.d ?? "").trim().endsWith("Z")).toBe(true);
    expect(core.isOpenClassD((stroke.geometry.d ?? ""))).toBe(false);
  });

  it("commits a rough rectangular stroke as an axis-snapped closed rect", () => {
    // Perimeter walk of (0,0)-(120,80): exact corners, sub-epsilon edge noise,
    // pen-up short of the start.
    const rectStroke = [
      { x: 0, y: 0 }, { x: 30, y: 1 }, { x: 60, y: 1.2 }, { x: 90, y: 0.8 },
      { x: 120, y: 0 }, { x: 119, y: 26 }, { x: 119.2, y: 53 },
      { x: 120, y: 80 }, { x: 90, y: 79 }, { x: 60, y: 79.2 }, { x: 30, y: 78.8 },
      { x: 0, y: 80 }, { x: 1, y: 55 }, { x: 0.8, y: 30 }, { x: 0, y: 10 }
    ];
    const stroke = core.freehandToObject(rectStroke, PEN.color, PEN.widthPx, "draw-3", "a2", "free");
    expect((stroke.geometry.d ?? "")).toBe(`M 0 0 L ${120 * Q} 0 L ${120 * Q} ${80 * Q} L 0 ${80 * Q} Z`);
  });
});

describe("(2) freehand release anchoring (open results, both corners)", () => {
  it("binds BOTH stroke endpoints to their snapped targets via core synthesizeCreateAnchorsBoth", () => {
    // rect-a spans x in [200,300], rect-b x in [500,600]; stroke drawn edge-to-edge.
    const rectA = targetRect("rect-a", 200, 0);
    const rectB = targetRect("rect-b", 500, 0);
    const stroke = core.freehandToObject(strokeAlong({ x: 300, y: 30 }, { x: 500, y: 30 }), PEN.color, PEN.widthPx, "draw-1", "a2", "free");
    expect(core.isOpenClassD((stroke.geometry.d ?? ""))).toBe(true);

    const anchors = core.synthesizeCreateAnchorsBoth(sceneOf([rectA, rectB]), stroke, [
      { target: "rect-a", at: { x: 300, y: 30 } },
      { target: "rect-b", at: { x: 500, y: 30 } }
    ]);
    expect(anchors).toHaveLength(2);
    // node 0 onto rect-a's right edge (target-local (100,30)px).
    expect(anchors[0].target).toBe("rect-a");
    expect(anchors[0].nodeIndex).toBe(0);
    expect(anchors[0].at).toEqual({ x: 100 * Q, y: 30 * Q });
    // node 1 onto rect-b's left edge.
    expect(anchors[1].target).toBe("rect-b");
    expect(anchors[1].nodeIndex).toBe(1);
    expect(anchors[1].at).toEqual({ x: 0, y: 30 * Q });
  });

  it("skips null corners and corners whose target left the scene", () => {
    const rectA = targetRect("rect-a", 200, 0);
    const stroke = core.freehandToObject(strokeAlong({ x: 300, y: 30 }, { x: 500, y: 30 }), PEN.color, PEN.widthPx, "draw-1", "a2", "free");
    const anchors = core.synthesizeCreateAnchorsBoth(sceneOf([rectA]), stroke, [
      null,
      { target: "rect-gone", at: { x: 500, y: 30 } } // stale release target
    ]);
    expect(anchors).toEqual([]);
  });

  it("a degenerate tap binds at most one anchor per node (no duplicate node bindings)", () => {
    // A pen tap recognizes as a zero-length 2-node line whose nodes coincide;
    // both corners resolve to the same node and only the first binding survives.
    const rectA = targetRect("rect-a", 200, 0);
    const tap = core.freehandToObject(strokeAlong({ x: 300, y: 30 }, { x: 300, y: 30 }), PEN.color, PEN.widthPx, "draw-4", "a2", "free");
    const anchors = core.synthesizeCreateAnchorsBoth(sceneOf([rectA]), tap, [
      { target: "rect-a", at: { x: 300, y: 30 } },
      { target: "rect-a", at: { x: 300, y: 30 } }
    ]);
    expect(anchors).toHaveLength(1);
    expect(anchors[0].target).toBe("rect-a");
  });

  it("a CLOSED recognition fails the shell's open-class gate, so no anchors are authored", () => {
    // A circle drawn on a target's edge still recognizes closed; the shell only
    // calls the anchor path for open-class results.
    const stroke = core.freehandToObject(circleStroke(300, 30, 25), PEN.color, PEN.widthPx, "draw-2", "a2", "free");
    expect(core.isOpenClassD((stroke.geometry.d ?? ""))).toBe(false);
  });
});

describe("(3) recognition mode contract — the SAME stroke resolves differently per mode", () => {
  it("an open S-curve: Basic snaps to the 2-node chord line, Free keeps the bezier curve", () => {
    // y = 20*sin(2*pi*t) over x in [0,150]: smooth, far over the line threshold.
    const sCurve = Array.from({ length: 31 }, (_, i) => {
      const t = i / 30;
      return { x: 150 * t, y: 20 * Math.sin(2 * Math.PI * t) };
    });
    const basic = core.freehandToObject(sCurve, PEN.color, PEN.widthPx, "draw-b", "a2", "basic");
    expect((basic.geometry.d ?? "")).toMatch(/^M \d+ \d+ L \d+ \d+$/);
    expect(core.isOpenClassD((basic.geometry.d ?? ""))).toBe(true);
    const free = core.freehandToObject(sCurve, PEN.color, PEN.widthPx, "draw-f", "a2", "free");
    expect((free.geometry.d ?? "")).not.toBe((basic.geometry.d ?? ""));
    expect((free.geometry.d ?? "")).toContain("C");
  });

  it("a closed pentagon-ish stroke: Basic snaps to a basic primitive, Free keeps the 5-corner polygon", () => {
    const v = [
      { x: 0, y: 0 }, { x: 100, y: 10 }, { x: 130, y: 90 },
      { x: 40, y: 130 }, { x: -40, y: 70 }
    ];
    const pentagon = v.flatMap((a, k) => {
      const b = v[(k + 1) % v.length];
      const n = k === v.length - 1 ? 9 : 10; // last edge stops short
      return Array.from({ length: n }, (_, i) => ({
        x: a.x + ((b.x - a.x) * i) / n,
        y: a.y + ((b.y - a.y) * i) / n
      }));
    });
    const free = core.freehandToObject(pentagon, PEN.color, PEN.widthPx, "draw-f", "a2", "free");
    // Free: a 5-corner straight-edged closed polygon (M + 4 L + Z).
    expect((free.geometry.d ?? "").match(/L/g)).toHaveLength(4);
    expect((free.geometry.d ?? "").trim().endsWith("Z")).toBe(true);
    expect((free.geometry.d ?? "")).not.toContain("C");
    const basic = core.freehandToObject(pentagon, PEN.color, PEN.widthPx, "draw-b", "a2", "basic");
    // Basic forbids the polygon and resolves the bbox ellipse instead.
    expect((basic.geometry.d ?? "")).toContain("C");
    expect((basic.geometry.d ?? "").trim().endsWith("Z")).toBe(true);
    expect((basic.geometry.d ?? "")).not.toBe((free.geometry.d ?? ""));
  });
});
