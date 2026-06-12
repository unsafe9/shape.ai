import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { beforeAll, describe, expect, it } from "vitest";
import { ensureSceneCore, loadSceneCore, type ObjectGesture } from "../bridge/sceneCoreWasm";
import {
  COARSE_ROTATE_SNAP_DEG,
  GESTURE_BINDINGS,
  isAdditiveSelect,
  isCoarseRotate,
  isPartialErase,
  isPanGesture,
  isSnapBypass,
  verifyGestureBindings
} from "../controller/gestureBindings";
import { AFFORDANCE_CURSOR, affordanceToCursor, cursorAffordance } from "../controller/cursor";
import { isPanIntent, shouldQuerySnap, snapRotateDeltaMatrix } from "../renderer/engine";
import type { HoverAffordance } from "../bridge/wasmLoader";
import type { RenderTransform3x3 } from "../renderer/scene";

let gestures: ObjectGesture[];

beforeAll(async () => {
  await ensureSceneCore();
  const core = await loadSceneCore();
  gestures = core.objectGestureCatalog();
});

const stylesCss = readFileSync(
  fileURLToPath(new URL("../styles.css", import.meta.url)),
  "utf8"
);

const mods = (over: Partial<Record<"shiftKey" | "metaKey" | "ctrlKey" | "altKey", boolean>> = {}) => ({
  shiftKey: false,
  metaKey: false,
  ctrlKey: false,
  altKey: false,
  ...over
});

describe("gesture bindings are the single source, equal to the catalog", () => {
  it("every routed binding matches the runtime gesture (no drift)", () => {
    expect(verifyGestureBindings(gestures)).toEqual([]);
  });

  it("flags a drift when a binding diverges from the catalog", () => {
    const skewed = gestures.map((g) =>
      g.id === "pan-space" ? { ...g, trigger: { ...g.trigger, key: "Enter" } } : g
    );
    expect(verifyGestureBindings(skewed)).toContain("pan-space");
  });

  it("carries the coarse-rotate step from the catalog (15°)", () => {
    expect(COARSE_ROTATE_SNAP_DEG).toBe(15);
    expect(GESTURE_BINDINGS["coarse-rotate-shift"].degrees).toBe(15);
  });
});

describe("pure gesture predicates route the held inputs", () => {
  it("pan: middle button always, left+Space only (isPanIntent delegates here)", () => {
    expect(isPanGesture({ spaceHeld: false, button: 1 })).toBe(true);
    expect(isPanGesture({ spaceHeld: true, button: 0 })).toBe(true);
    expect(isPanGesture({ spaceHeld: false, button: 0 })).toBe(false);
    // The exported engine entry routes through the same predicate.
    expect(isPanIntent({ spaceHeld: true, button: 0 })).toBe(true);
    expect(isPanIntent({ spaceHeld: false, button: 2 })).toBe(false);
  });

  it("additive-select: Shift always; Mod = Cmd on mac, Ctrl off mac", () => {
    expect(isAdditiveSelect(mods({ shiftKey: true }), true)).toBe(true);
    expect(isAdditiveSelect(mods({ metaKey: true }), true)).toBe(true);
    expect(isAdditiveSelect(mods({ metaKey: true }), false)).toBe(false);
    expect(isAdditiveSelect(mods({ ctrlKey: true }), false)).toBe(true);
    expect(isAdditiveSelect(mods(), true)).toBe(false);
  });

  it("snap-bypass + partial-erase ride Alt (shouldQuerySnap routes here)", () => {
    expect(isSnapBypass({ altKey: true })).toBe(true);
    expect(isPartialErase({ altKey: true })).toBe(true);
    expect(shouldQuerySnap({ altHeld: true, phase: "move" })).toBe(false);
    expect(shouldQuerySnap({ altHeld: false, phase: "move" })).toBe(true);
  });

  it("coarse-rotate rides Shift", () => {
    expect(isCoarseRotate({ shiftKey: true })).toBe(true);
    expect(isCoarseRotate({ shiftKey: false })).toBe(false);
  });
});

// Build the same rotate-delta the core returns, to compare snap semantics from the
// matrix alone.
function rotateAbout(thetaDeg: number, cx: number, cy: number): RenderTransform3x3 {
  const t = (thetaDeg * Math.PI) / 180;
  const c = Math.cos(t);
  const s = Math.sin(t);
  return [
    [c, -s, cx - c * cx + s * cy],
    [s, c, cy - s * cx - c * cy],
    [0, 0, 1]
  ];
}

function angleOf(m: RenderTransform3x3): number {
  return (Math.atan2(m[1][0], m[0][0]) * 180) / Math.PI;
}

describe("snapRotateDeltaMatrix (coarse-rotate)", () => {
  it("snaps a swept angle to the nearest 15° step", () => {
    const center: [number, number] = [200, 140];
    // Nearest 15° multiple: 22 -> 15, 7 -> 0, -52 -> -45, 38 -> 45.
    expect(angleOf(snapRotateDeltaMatrix(rotateAbout(22, ...center), 15))).toBeCloseTo(15, 5);
    expect(angleOf(snapRotateDeltaMatrix(rotateAbout(7, ...center), 15))).toBeCloseTo(0, 5);
    expect(angleOf(snapRotateDeltaMatrix(rotateAbout(-52, ...center), 15))).toBeCloseTo(-45, 5);
    expect(angleOf(snapRotateDeltaMatrix(rotateAbout(38, ...center), 15))).toBeCloseTo(45, 5);
  });

  it("rotates about the SAME center it recovered from the matrix", () => {
    const cx = 311;
    const cy = -88;
    const snapped = snapRotateDeltaMatrix(rotateAbout(44, cx, cy), 15);
    // Rebuilt about (cx,cy) must fix that center point.
    const fx = snapped[0][0] * cx + snapped[0][1] * cy + snapped[0][2];
    const fy = snapped[1][0] * cx + snapped[1][1] * cy + snapped[1][2];
    expect(fx).toBeCloseTo(cx, 4);
    expect(fy).toBeCloseTo(cy, 4);
    expect(angleOf(snapped)).toBeCloseTo(45, 6);
  });

  it("leaves an exact-multiple sweep unchanged", () => {
    const m = rotateAbout(90, 50, 50);
    const snapped = snapRotateDeltaMatrix(m, 15);
    expect(angleOf(snapped)).toBeCloseTo(90, 6);
  });
});

const ALL_AFFORDANCES: HoverAffordance[] = [
  "empty",
  "body",
  "resize-nw",
  "resize-n",
  "resize-ne",
  "resize-e",
  "resize-se",
  "resize-s",
  "resize-sw",
  "resize-w",
  "rotate"
];

describe("HoverAffordance -> CSS cursor map", () => {
  it("maps every resize affordance to a resize cursor, rotate to crosshair, pan to grab", () => {
    expect(affordanceToCursor("resize-nw")).toBe("nwse-resize");
    expect(affordanceToCursor("resize-ne")).toBe("nesw-resize");
    expect(affordanceToCursor("resize-n")).toBe("ns-resize");
    expect(affordanceToCursor("resize-e")).toBe("ew-resize");
    expect(affordanceToCursor("rotate")).toBe("crosshair");
    expect(affordanceToCursor("body")).toBe("move");
    expect(affordanceToCursor("pan")).toBe("grab");
  });

  it("styles.css carries the mapped cursor for every affordance (no drift, no override)", () => {
    for (const affordance of ALL_AFFORDANCES) {
      const cursor = AFFORDANCE_CURSOR[affordance];
      if (affordance === "empty") continue; // falls back to the surface default
      const rule = new RegExp(
        `\\[data-affordance="${affordance}"\\][^{]*\\{[^}]*cursor:\\s*${cursor}`,
        "s"
      );
      expect(stylesCss, `styles.css must map ${affordance} -> ${cursor}`).toMatch(rule);
    }
  });

  it("does NOT hardcode a grab cursor on the input canvas", () => {
    // The input canvas must inherit the cursor so the affordance shows through.
    expect(stylesCss).toMatch(/\.renderer-input-canvas\s*\{[^}]*cursor:\s*inherit/s);
    expect(stylesCss).not.toMatch(/\.renderer-input-canvas\s*\{[^}]*cursor:\s*grab/s);
  });
});

describe("cursorAffordance derivation (pointer over a handle)", () => {
  it("a pointer over a resize handle / rotate zone yields the resize/rotate cursor", () => {
    const over = cursorAffordance(false, "select", "resize-se");
    expect(over).toBe("resize-se");
    expect(affordanceToCursor(over!)).toBe("nwse-resize");

    const rot = cursorAffordance(false, "select", "rotate");
    expect(rot).toBe("rotate");
    expect(affordanceToCursor(rot!)).toBe("crosshair");
  });

  it("Space-hold forces pan; crosshair tools suppress the affordance override", () => {
    expect(cursorAffordance(true, "select", "resize-se")).toBe("pan");
    expect(cursorAffordance(false, "draw", "resize-se")).toBeNull();
    expect(cursorAffordance(false, "erase", "body")).toBeNull();
  });
});
