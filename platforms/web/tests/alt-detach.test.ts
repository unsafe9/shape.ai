// An Alt-held body drag of an anchored open-class object moves it WHOLE and detaches
// its anchors (set-anchor [] + plain SetTransform translate), NOT the pinned chord
// deform the same drag produces without Alt.

import { beforeAll, describe, expect, it } from "vitest";

import { ShapeCanvasEngine, type EngineEvent } from "../renderer/engine";
import { commitBodyDrag, isDetachableBodyDrag } from "../controller/interactions";
import type { ObjectSelection } from "../shared/object";
import { GESTURE_BINDINGS, GESTURE_DETACH_ALT, isDetachDrag } from "../controller/gestureBindings";
import { ensureSceneCore, loadSceneCore, type SceneCore } from "../bridge/sceneCoreWasm";
import {
  emptyObjectScene,
  translateTransform,
  type Object as SceneObject,
  type ObjectScene,
  type Transform3x3
} from "../shared/object";
import type { RustInputBatchResult, RustWebGpuRenderer } from "../bridge/wasmLoader";
import type { CameraState } from "../renderer/scene";

const CAMERA: CameraState = { x: 0, y: 0, zoom: 1 };
const LINE_ID = "line-1";
const TARGET_ID = "rect-1";

let core: SceneCore;
beforeAll(async () => {
  await ensureSceneCore();
  core = await loadSceneCore();
});

const TRANSLATE_40_30: Transform3x3 = [
  [1, 0, 40],
  [0, 1, 30],
  [0, 0, 1]
];

function translateDragRenderer(): RustWebGpuRenderer {
  return {
    resize() {},
    renderFrame() {
      return { backend: "test" } as unknown as ReturnType<RustWebGpuRenderer["renderFrame"]>;
    },
    inputBatch(eventsJson: string): RustInputBatchResult {
      const events = JSON.parse(eventsJson) as Array<{ kind: string }>;
      const isMove = events.some((e) => e.kind === "pointer-move");
      return {
        camera: CAMERA,
        objectTransformDelta: isMove ? { id: LINE_ID, matrix: TRANSLATE_40_30, kind: "translate" } : null
      };
    }
  } as unknown as RustWebGpuRenderer;
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

function driveBodyDrag(altHeld: boolean): Extract<EngineEvent, { type: "object-transform-commit" }> | null {
  const events: EngineEvent[] = [];
  const { canvas, fire } = recordingCanvas();
  // eslint-disable-next-line no-new
  new ShapeCanvasEngine({
    canvas,
    overlayRoot: { append() {} } as unknown as HTMLElement,
    backend: "test",
    webGpuRenderer: translateDragRenderer(),
    onEvent: (event) => events.push(event)
  });
  const base = { pointerId: 7, pointerType: "pen", shiftKey: false, altKey: altHeld, metaKey: false, ctrlKey: false };
  fire("pointerdown", { ...base, button: 0, clientX: 10, clientY: 10, preventDefault() {} });
  fire("pointermove", { ...base, clientX: 50, clientY: 40, preventDefault() {} });
  fire("pointerup", { ...base, clientX: 50, clientY: 40, preventDefault() {} });
  return (events.find((e) => e.type === "object-transform-commit") ?? null) as
    | Extract<EngineEvent, { type: "object-transform-commit" }>
    | null;
}

describe("engine detach-alt gesture bit on the transform commit", () => {
  it("forwards detach=true when Alt is held during the drag", () => {
    const commit = driveBodyDrag(true);
    expect(commit).not.toBeNull();
    expect(commit!.detach).toBe(true);
    expect(commit!.kind).toBe("translate");
  });

  it("forwards detach=false without Alt", () => {
    expect(driveBodyDrag(false)!.detach).toBe(false);
  });

  it("routes through the frozen detach-alt binding (single source)", () => {
    expect(GESTURE_BINDINGS[GESTURE_DETACH_ALT]).toEqual({ modifier: "Alt" });
    expect(isDetachDrag({ altKey: true })).toBe(true);
    expect(isDetachDrag({ altKey: false })).toBe(false);
    expect(core.objectGestureCatalog().some((g) => g.id === GESTURE_DETACH_ALT)).toBe(true);
  });
});

function obj(partial: Partial<SceneObject> & { id: string; geometry: SceneObject["geometry"] }): SceneObject {
  return { order: "a0", ...partial } as SceneObject;
}

// line-1 node 0 anchored onto rect-1: without Alt the body drag pins that end
// (chord deform); with Alt the whole line translates and detaches.
function anchoredScene(): ObjectScene {
  return {
    ...emptyObjectScene(),
    objects: [
      obj({
        id: TARGET_ID,
        geometry: { d: "M 0 0 L 800 0 L 800 800 L 0 800 Z", fillRule: "nonZero" },
        transform: translateTransform(100, 100)
      }),
      obj({
        id: LINE_ID,
        order: "a1",
        geometry: { d: "M 0 0 L 1600 0", fillRule: "nonZero" },
        transform: translateTransform(150, 150),
        anchors: [{ nodeIndex: 0, target: TARGET_ID, at: { x: 400, y: 400 } }]
      })
    ]
  };
}

describe("core detachMoveOps (the App onTransformCommit detach branch)", () => {
  it("authors set-anchor [] + a WHOLE SetTransform translate (no chord deform)", () => {
    const ops = core.detachMoveOps(anchoredScene(), LINE_ID, TRANSLATE_40_30);
    expect(ops[0]).toEqual({ kind: "set-anchor", id: LINE_ID, anchors: [] });
    const transform = ops.find((o) => o.kind === "set-transform" && o.id === LINE_ID);
    expect(transform).toBeDefined();
    if (transform?.kind !== "set-transform") throw new Error("expected set-transform");
    // Composed onto the line's base translate(150,150).
    expect(transform.transform[0][2]).toBe(190);
    expect(transform.transform[1][2]).toBe(180);
    // No pinned-endpoint geometry rewrite rides the detach commit.
    expect(ops.some((o) => o.kind === "edit-geometry" && o.id === LINE_ID)).toBe(false);
  });

  it("is load-bearing: the SAME drag without detach pins the anchored end instead", () => {
    const ops = core.moveOps(anchoredScene(), { kind: "single", id: LINE_ID }, TRANSLATE_40_30);
    // Anchored end pinned, free end chord-deforms -> no whole SetTransform translate.
    expect(ops.some((o) => o.kind === "set-transform" && o.id === LINE_ID)).toBe(false);
  });
});

describe("commitBodyDrag detach branch (the App onTransformCommit decision)", () => {
  const SINGLE: ObjectSelection = { kind: "object", id: LINE_ID };

  it("an Alt-held translate of the anchored open-class line detaches: set-anchor [] + whole translate", () => {
    const { op, allOps } = commitBodyDrag(core, anchoredScene(), SINGLE, LINE_ID, TRANSLATE_40_30, "translate", true);
    expect(allOps[0]).toEqual({ kind: "set-anchor", id: LINE_ID, anchors: [] });
    const transform = allOps.find((o) => o.kind === "set-transform" && o.id === LINE_ID);
    expect(transform?.kind === "set-transform" && transform.transform[0][2]).toBe(190);
    // Many ops -> wrapped in a batch.
    expect(op).toEqual({ kind: "batch", ops: allOps });
  });

  it("is load-bearing: the SAME drag WITHOUT detach pins the anchored end (no whole translate)", () => {
    const { allOps } = commitBodyDrag(core, anchoredScene(), SINGLE, LINE_ID, TRANSLATE_40_30, "translate", false);
    expect(allOps.some((o) => o.kind === "set-transform" && o.id === LINE_ID)).toBe(false);
  });

  it("gates detach on translate + single root + anchored + open-class (core isOpenClassD)", () => {
    const scene = anchoredScene();
    const line = scene.objects.find((o) => o.id === LINE_ID)!;
    const rect = scene.objects.find((o) => o.id === TARGET_ID)!;
    const single: ObjectSelection = { kind: "object", id: LINE_ID };
    expect(isDetachableBodyDrag(core, line, single, LINE_ID, "translate", true)).toBe(true);
    expect(isDetachableBodyDrag(core, line, single, LINE_ID, "rotate", true)).toBe(false);
    expect(isDetachableBodyDrag(core, line, single, LINE_ID, "translate", false)).toBe(false);
    // Closed-class object (the rect) is not detachable even with anchors held.
    expect(isDetachableBodyDrag(core, { ...rect, anchors: [{ nodeIndex: 0, target: LINE_ID, at: { x: 0, y: 0 } }] }, single, LINE_ID, "translate", true)).toBe(false);
    // A Multi drag anchored on a member (the whole-set move) is never a single-root detach.
    expect(isDetachableBodyDrag(core, line, { kind: "multi", ids: [LINE_ID, TARGET_ID] }, LINE_ID, "translate", true)).toBe(false);
  });
});
