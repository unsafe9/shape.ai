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
import { isPanIntent, ShapeCanvasEngine, shouldQuerySnap, type EngineEvent } from "../renderer/engine";
import * as engineModule from "../renderer/engine";
import type { HoverAffordance, RustInputBatchResult, RustWebGpuRenderer } from "../bridge/wasmLoader";
import type { RenderTransform3x3, CameraState } from "../renderer/scene";

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

function angleOf(m: RenderTransform3x3): number {
  return (Math.atan2(m[1][0], m[0][0]) * 180) / Math.PI;
}

const SNAPPED_15: RenderTransform3x3 = (() => {
  const t = (15 * Math.PI) / 180;
  const c = Math.cos(t);
  const s = Math.sin(t);
  return [
    [c, -s, 0],
    [s, c, 0],
    [0, 0, 1]
  ];
})();

const ROTATE_CAMERA: CameraState = { x: 0, y: 0, zoom: 1 };

// A renderer stub that snaps the rotate delta in-core exactly when coarse-rotate is active (the real
// behavior, proven by the Rust `coarse_rotate_drag_emits_snapped_delta_in_core` test), and records the
// setCoarseRotate calls the engine pushes off the mirrored predicate.
function rotateDragRenderer(): { renderer: RustWebGpuRenderer; coarseCalls: boolean[] } {
  const coarseCalls: boolean[] = [];
  let coarse = false;
  const renderer = {
    resize() {},
    renderFrame() {
      return { backend: "test" } as unknown as ReturnType<RustWebGpuRenderer["renderFrame"]>;
    },
    setCoarseRotate(active: boolean) {
      coarse = active;
      coarseCalls.push(active);
    },
    setObjectPreviewTransform() {},
    inputBatch(eventsJson: string): RustInputBatchResult {
      const events = JSON.parse(eventsJson) as Array<{ kind: string }>;
      const isMove = events.some((e) => e.kind === "pointer-move");
      // The core only emits the snapped 15° delta when coarse-rotate is active.
      const matrix = coarse ? SNAPPED_15 : ([[1, 0, 0], [0, 1, 0], [0, 0, 1]] as RenderTransform3x3);
      return {
        camera: ROTATE_CAMERA,
        objectTransformDelta: isMove ? { id: "obj-1", matrix, kind: "rotate" } : null
      };
    }
  } as unknown as RustWebGpuRenderer;
  return { renderer, coarseCalls };
}

function recordingCanvas(): { canvas: HTMLCanvasElement; fire: (type: string, event: unknown) => void } {
  const listeners = new Map<string, Set<EventListener>>();
  const element = {
    width: 0,
    height: 0,
    addEventListener(type: string, listener: EventListenerOrEventListenerObject) {
      if (typeof listener !== "function") return;
      const set = listeners.get(type) ?? new Set();
      set.add(listener);
      listeners.set(type, set);
    },
    removeEventListener(type: string, listener: EventListenerOrEventListenerObject) {
      if (typeof listener !== "function") return;
      listeners.get(type)?.delete(listener);
    },
    setPointerCapture() {},
    releasePointerCapture() {},
    getBoundingClientRect() {
      return { left: 0, top: 0, width: 800, height: 600 };
    }
  };
  return {
    canvas: element as unknown as HTMLCanvasElement,
    fire: (type, event) => {
      for (const listener of listeners.get(type) ?? []) listener(event as Event);
    }
  };
}

function driveRotateDrag(shiftHeld: boolean): {
  commit: Extract<EngineEvent, { type: "object-transform-commit" }> | null;
  coarseCalls: boolean[];
} {
  const events: EngineEvent[] = [];
  const { canvas, fire } = recordingCanvas();
  const { renderer, coarseCalls } = rotateDragRenderer();
  // eslint-disable-next-line no-new
  new ShapeCanvasEngine({
    canvas,
    overlayRoot: { append() {} } as unknown as HTMLElement,
    backend: "test",
    webGpuRenderer: renderer,
    onEvent: (event) => events.push(event)
  });
  const base = { pointerId: 7, pointerType: "pen", shiftKey: shiftHeld, altKey: false, metaKey: false, ctrlKey: false };
  fire("pointerdown", { ...base, button: 0, clientX: 10, clientY: 10, preventDefault() {} });
  fire("pointermove", { ...base, clientX: 60, clientY: 40, preventDefault() {} });
  fire("pointerup", { ...base, clientX: 60, clientY: 40, preventDefault() {} });
  const commit = (events.find((e) => e.type === "object-transform-commit") ?? null) as
    | Extract<EngineEvent, { type: "object-transform-commit" }>
    | null;
  return { commit, coarseCalls };
}

describe("coarse-rotate is core-driven (no shell matrix decomposition)", () => {
  it("the engine exports no snapRotateDeltaMatrix (the shell never decomposes the rotate matrix)", () => {
    expect((engineModule as Record<string, unknown>).snapRotateDeltaMatrix).toBeUndefined();
  });

  it("drives setCoarseRotate(true) off the mirrored predicate when Shift is NOT held (coarse by default)", () => {
    const { commit, coarseCalls } = driveRotateDrag(false);
    // Inverted predicate: no Shift => coarse active.
    expect(coarseCalls.at(-1)).toBe(true);
    // The committed transform carries the core's already-snapped 15° angle, consumed verbatim.
    expect(commit).not.toBeNull();
    expect(commit!.kind).toBe("rotate");
    expect(angleOf(commit!.matrix)).toBeCloseTo(15, 5);
  });

  it("drives setCoarseRotate(false) when Shift IS held (free rotation), so the core does not snap", () => {
    const { coarseCalls } = driveRotateDrag(true);
    expect(coarseCalls.at(-1)).toBe(false);
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
