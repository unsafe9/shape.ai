// W3-G6 (#1) — live marquee -> Multi selection routing.
//
// The renderer core collects EVERY object whose world AABB intersects the marquee
// rect (scene_build.rs object_regions_in_marquee, unit-pinned there). This test
// pins the SHELL seam the diagnosis flagged as untested: an empty-start mouse drag
// across multiple objects must carry ALL marquee ids out of the engine as one
// `object-marquee` EngineEvent (engine.ts:1072 `result.objectMarqueeIds != null`),
// and the App's onMarquee rule (ids.length >= 2 -> { kind: "multi" }) must form a
// >= 2 Multi selection from them.
//
// Falsifiable: if the engine drops/loses objectMarqueeIds (stale wasm field, wrong
// feature-detect, never reading it) the `object-marquee` event never fires with the
// full id set, and the multi never forms.

import { describe, expect, it } from "vitest";

import { ShapeCanvasEngine, type EngineEvent } from "../platforms/web/renderer/engine";
import type { RustInputBatchResult, RustWebGpuRenderer } from "../platforms/web/bridge/wasmLoader";
import type { CameraState } from "../platforms/web/renderer/scene";
import type { Object as SceneObject, ObjectScene, ObjectSelection } from "../platforms/web/shared/object";

const CAMERA: CameraState = { x: 0, y: 0, zoom: 1 };

// A fake renderer whose `inputBatch` returns the supplied marquee ids on the
// pointer-up batch (an empty-start marquee rides the up). Other events return an
// empty object-path result (no selection, no marquee). Only the methods the engine
// touches on this path are real.
function marqueeRenderer(ids: string[]): RustWebGpuRenderer {
  return {
    resize() {},
    loadScene() {},
    applyPatchBatch() {},
    renderFrame() {
      return { backend: "test" } as unknown as ReturnType<RustWebGpuRenderer["renderFrame"]>;
    },
    inputBatch(eventsJson: string): RustInputBatchResult {
      const events = JSON.parse(eventsJson) as Array<{ kind: string }>;
      const isUp = events.some((e) => e.kind === "pointer-up");
      return {
        camera: CAMERA,
        hit: null,
        selection: { kind: "canvas" },
        patches: [],
        overlay: null,
        // The empty-start marquee result rides the pointer-up batch.
        objectMarqueeIds: isUp ? ids : null
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

// A canvas stub that records the engine's listeners so the test can fire DOM-shape
// mouse events at them (the engine binds mousemove/mouseup to the canvas itself in a
// no-`window` env, so a recording canvas drives the whole drag).
function captureCanvas(): { canvas: HTMLCanvasElement; fire: (type: string, init: Record<string, unknown>) => void } {
  const listeners = new Map<string, Set<EventListener>>();
  const canvas = {
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
  } as unknown as HTMLCanvasElement;
  const fire = (type: string, init: Record<string, unknown>) => {
    const event = { type, preventDefault() {}, ...init } as unknown as Event;
    for (const listener of listeners.get(type) ?? []) listener(event);
  };
  return { canvas, fire };
}

// Drive a real empty-start mouse marquee (down on empty canvas -> move -> up) through
// the engine + fake renderer, returning the `object-marquee` EngineEvent the engine
// emitted (or null if none fired).
function driveMarquee(ids: string[]): Extract<EngineEvent, { type: "object-marquee" }> | null {
  const events: EngineEvent[] = [];
  const { canvas, fire } = captureCanvas();
  // eslint-disable-next-line no-new
  new ShapeCanvasEngine({
    canvas,
    overlayRoot: { append() {} } as unknown as HTMLElement,
    backend: "test",
    webGpuRenderer: marqueeRenderer(ids),
    onEvent: (event) => events.push(event)
  });
  // Empty-start drag across the canvas: down -> move -> up (the up batch carries the
  // marquee ids in the fake renderer).
  fire("mousedown", { button: 0, clientX: 10, clientY: 10 });
  fire("mousemove", { button: 0, clientX: 400, clientY: 300 });
  fire("mouseup", { button: 0, clientX: 400, clientY: 300 });
  return (events.find((e) => e.type === "object-marquee") ?? null) as
    | Extract<EngineEvent, { type: "object-marquee" }>
    | null;
}

function sceneWith(ids: string[]): ObjectScene {
  const objects: SceneObject[] = ids.map((id, i) => ({
    id,
    order: `a${i}`,
    transform: [
      [1, 0, 0],
      [0, 1, 0],
      [0, 0, 1]
    ],
    geometry: { d: "M 0 0 L 8 0 L 8 8 L 0 8 Z", fillRule: "nonZero" },
    clip: false
  }));
  return { sceneId: "s", sceneVersion: 1, objects } as unknown as ObjectScene;
}

// A faithful mirror of App.svelte's validSelection (the multi branch): keep only
// live ids, then collapse the kind as the set shrinks. Kept in lockstep with
// App.svelte:1134-1144 so the seam under test is the App's actual collapse rule.
function validSelection(scene: ObjectScene, sel: ObjectSelection): ObjectSelection {
  if (sel.kind === "canvas") return sel;
  if (sel.kind === "object") {
    return scene.objects.some((o) => o.id === sel.id) ? sel : { kind: "canvas" };
  }
  const live = sel.ids.filter((id) => scene.objects.some((o) => o.id === id));
  if (live.length >= 2) return { kind: "multi", ids: live };
  if (live.length === 1) return { kind: "object", id: live[0] };
  return { kind: "canvas" };
}

// App.onMarquee rule (App.svelte:302-307): a >= 2 marquee is a Multi, exactly 1 is a
// single object, 0 clears to canvas — then validated against the live scene.
function marqueeSelection(scene: ObjectScene, ids: string[]): ObjectSelection {
  const next: ObjectSelection =
    ids.length >= 2 ? { kind: "multi", ids } : ids.length === 1 ? { kind: "object", id: ids[0] } : { kind: "canvas" };
  return validSelection(scene, next);
}

describe("W3-G6 marquee multi-select routing (#1)", () => {
  it("carries ALL marquee ids out of the engine and forms a >= 2 Multi", () => {
    const event = driveMarquee(["a", "b", "c"]);
    // The field reached the engine and rode out as the event the host forwards to
    // onMarquee — with every id, not one.
    expect(event).toEqual({ type: "object-marquee", ids: ["a", "b", "c"] });

    // App.onMarquee -> validSelection yields a Multi of all live ids.
    const scene = sceneWith(["a", "b", "c"]);
    expect(marqueeSelection(scene, event!.ids)).toEqual({ kind: "multi", ids: ["a", "b", "c"] });
  });

  it("a two-object marquee forms a Multi (the smallest multi the App keeps)", () => {
    const event = driveMarquee(["rect-1", "ell-1"]);
    expect(event).toEqual({ type: "object-marquee", ids: ["rect-1", "ell-1"] });
    const scene = sceneWith(["rect-1", "ell-1"]);
    expect(marqueeSelection(scene, event!.ids)).toEqual({ kind: "multi", ids: ["rect-1", "ell-1"] });
  });

  it("a single-object marquee collapses to an object selection, not a multi", () => {
    const event = driveMarquee(["only"]);
    expect(event).toEqual({ type: "object-marquee", ids: ["only"] });
    const scene = sceneWith(["only"]);
    expect(marqueeSelection(scene, event!.ids)).toEqual({ kind: "object", id: "only" });
  });

  it("an empty marquee clears to canvas (no objects swept)", () => {
    const event = driveMarquee([]);
    expect(event).toEqual({ type: "object-marquee", ids: [] });
    const scene = sceneWith(["a", "b"]);
    expect(marqueeSelection(scene, event!.ids)).toEqual({ kind: "canvas" });
  });
});
