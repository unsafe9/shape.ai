// An empty-start mouse drag across multiple objects must carry ALL marquee ids out
// of the engine as one object-marquee event, and onMarquee (ids >= 2 -> multi) must
// form a >= 2 Multi from them.

import { describe, expect, it } from "vitest";

import { ShapeCanvasEngine, type EngineEvent } from "../platforms/web/renderer/engine";
import type { RustInputBatchResult, RustWebGpuRenderer } from "../platforms/web/bridge/wasmLoader";
import type { CameraState } from "../platforms/web/renderer/scene";
import type { Object as SceneObject, ObjectScene, ObjectSelection } from "../platforms/web/shared/object";

const CAMERA: CameraState = { x: 0, y: 0, zoom: 1 };

// Returns the supplied marquee ids on the pointer-up batch (an empty-start marquee
// rides the up); other events return an empty object-path result.
function marqueeRenderer(ids: string[]): RustWebGpuRenderer {
  return {
    resize() {},
    renderFrame() {
      return { backend: "test" } as unknown as ReturnType<RustWebGpuRenderer["renderFrame"]>;
    },
    inputBatch(eventsJson: string): RustInputBatchResult {
      const events = JSON.parse(eventsJson) as Array<{ kind: string }>;
      const isUp = events.some((e) => e.kind === "pointer-up");
      return {
        camera: CAMERA,
        objectMarqueeIds: isUp ? ids : null
      };
    }
  } as unknown as RustWebGpuRenderer;
}

// Records the engine's listeners so the test can fire DOM-shape mouse events. The
// engine binds mousemove/mouseup to the canvas in a no-window env, so this drives the drag.
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
  // Empty-start drag: down -> move -> up (the up batch carries the marquee ids).
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

// Mirrors App.svelte's validSelection: keep only live ids, collapse the kind as the
// set shrinks. Kept in lockstep with the App's collapse rule.
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

// App.onMarquee rule: >= 2 ids -> Multi, exactly 1 -> single object, 0 -> canvas,
// then validated against the live scene.
function marqueeSelection(scene: ObjectScene, ids: string[]): ObjectSelection {
  const next: ObjectSelection =
    ids.length >= 2 ? { kind: "multi", ids } : ids.length === 1 ? { kind: "object", id: ids[0] } : { kind: "canvas" };
  return validSelection(scene, next);
}

describe("marquee multi-select routing", () => {
  it("carries ALL marquee ids out of the engine and forms a >= 2 Multi", () => {
    const event = driveMarquee(["a", "b", "c"]);
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
