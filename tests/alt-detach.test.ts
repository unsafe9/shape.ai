// Anchor-semantics v3 §3 (DU4) — Alt-drag detach.
//
// An Alt-held BODY drag of an anchored open-class object must move it WHOLE and
// detach its anchors: one set-anchor clearing the anchors vector plus a plain
// SetTransform translate (the 0-rebake path) — NOT the pinned chord deform the
// same drag produces without Alt. Three layers are pinned:
//   1. the engine forwards the C2 `detach-alt` gesture bit on the transform
//      commit (real engine, fake renderer);
//   2. `altDetachOps` (the App.svelte commit composition) yields set-anchor [] +
//      set-transform translate through the REAL core, and the un-detached
//      moveOps on the SAME scene does NOT (the branch is load-bearing);
//   3. the App.svelte source wires the branch through the core's isOpenClassD.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { beforeAll, describe, expect, it } from "vitest";

import { ShapeCanvasEngine, type EngineEvent } from "../src/client/renderer/engine";
import { altDetachOps } from "../src/client/lib/objectPrimitives";
import { GESTURE_BINDINGS, GESTURE_DETACH_ALT, isDetachDrag } from "../src/client/lib/gestureBindings";
import { ensureSceneCore, loadSceneCore, type SceneCore } from "../src/client/scene/sceneCoreWasm";
import {
  emptyObjectScene,
  translateTransform,
  type Object as SceneObject,
  type ObjectScene,
  type Transform3x3
} from "../src/shared/object";
import type { RustInputBatchResult, RustWebGpuRenderer } from "../src/client/renderer/wasmLoader";
import type { CameraState } from "../src/client/renderer/scene";

const CAMERA: CameraState = { x: 0, y: 0, zoom: 1 };
const LINE_ID = "line-1";
const TARGET_ID = "rect-1";

let core: SceneCore;
beforeAll(async () => {
  await ensureSceneCore();
  core = await loadSceneCore();
});

// ---------------------------------------------------------------------------
// 1. Engine: the transform commit carries the detach-alt gesture bit.
// ---------------------------------------------------------------------------

const TRANSLATE_40_30: Transform3x3 = [
  [1, 0, 40],
  [0, 1, 30],
  [0, 0, 1]
];

function translateDragRenderer(): RustWebGpuRenderer {
  return {
    resize() {},
    loadScene() {},
    applyPatchBatch() {},
    renderFrame() {
      return { backend: "test" } as unknown as ReturnType<RustWebGpuRenderer["renderFrame"]>;
    },
    inputBatch(eventsJson: string): RustInputBatchResult {
      const events = JSON.parse(eventsJson) as Array<{ kind: string }>;
      const isMove = events.some((e) => e.kind === "pointer-move");
      return {
        camera: CAMERA,
        hit: null,
        selection: { kind: "canvas" },
        patches: [],
        overlay: null,
        objectTransformDelta: isMove ? { id: LINE_ID, matrix: TRANSLATE_40_30, kind: "translate" } : null
      };
    },
    overlayRequest() {
      return null;
    },
    debugSnapshot() {
      return {
        camera: CAMERA,
        selection: { kind: "canvas" },
        selectionWorldRect: null,
        selectionScreenRect: null,
        lastHit: null,
        totalGroups: 0,
        totalCards: 0,
        totalEdges: 0,
        patchUpdateCount: 0,
        dirtyRangeWriteCount: 0,
        fullBufferRebuildCount: 0
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

  it("routes through the frozen C2 detach-alt binding (single source)", () => {
    expect(GESTURE_BINDINGS[GESTURE_DETACH_ALT]).toEqual({ modifier: "Alt" });
    expect(isDetachDrag({ altKey: true })).toBe(true);
    expect(isDetachDrag({ altKey: false })).toBe(false);
    // The runtime catalog carries the registered gesture.
    expect(core.objectGestureCatalog().some((g) => g.id === GESTURE_DETACH_ALT)).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// 2. altDetachOps: the commit composition through the REAL core.
// ---------------------------------------------------------------------------

function obj(partial: Partial<SceneObject> & { id: string; geometry: SceneObject["geometry"] }): SceneObject {
  return { order: "a0", ...partial } as SceneObject;
}

// line-1's node 0 is anchored onto rect-1 — the body-drag policy without Alt pins
// that end (chord deform); with Alt the whole line translates and detaches.
function anchoredScene(): ObjectScene {
  return {
    ...emptyObjectScene(),
    objects: [
      obj({
        id: TARGET_ID,
        geometry: { d: "M 0 0 L 800 0 L 800 800 L 0 800 Z" },
        transform: translateTransform(100, 100)
      }),
      obj({
        id: LINE_ID,
        order: "a1",
        geometry: { d: "M 0 0 L 1600 0" },
        transform: translateTransform(150, 150),
        anchors: [{ nodeIndex: 0, target: TARGET_ID, at: { x: 400, y: 400 } }]
      })
    ]
  };
}

describe("altDetachOps composition (the App onTransformCommit detach branch)", () => {
  it("authors set-anchor [] + a WHOLE SetTransform translate (no chord deform)", () => {
    const ops = altDetachOps(core, anchoredScene(), LINE_ID, TRANSLATE_40_30);
    expect(ops[0]).toEqual({ kind: "set-anchor", id: LINE_ID, anchors: [] });
    const transform = ops.find((o) => o.kind === "set-transform" && o.id === LINE_ID);
    expect(transform).toBeDefined();
    if (transform?.kind !== "set-transform") throw new Error("expected set-transform");
    // The translate composed onto the line's base translate(150,150).
    expect(transform.transform[0][2]).toBe(190);
    expect(transform.transform[1][2]).toBe(180);
    // No pinned-endpoint geometry rewrite rides the detach commit.
    expect(ops.some((o) => o.kind === "edit-geometry" && o.id === LINE_ID)).toBe(false);
  });

  it("is load-bearing: the SAME drag without detach pins the anchored end instead", () => {
    const ops = core.moveOps(anchoredScene(), { kind: "single", id: LINE_ID }, TRANSLATE_40_30);
    // The anchored body drag must NOT author a whole SetTransform translate for
    // the line (the anchored end is pinned; the free end chord-deforms).
    expect(ops.some((o) => o.kind === "set-transform" && o.id === LINE_ID)).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// 3. App.svelte wiring (source pins — no DOM in the node test env).
// ---------------------------------------------------------------------------

describe("App.svelte detach wiring (source pins)", () => {
  const source = readFileSync(fileURLToPath(new URL("../src/client/svelte/App.svelte", import.meta.url)), "utf8");

  it("branches the commit through altDetachOps gated on the core isOpenClassD", () => {
    expect(source).toMatch(/sceneCore\.isOpenClassD\(src\.geometry\.d\)/);
    expect(source).toMatch(/altDetachOps\(sceneCore, scene, id, matrix\)/);
  });
});
