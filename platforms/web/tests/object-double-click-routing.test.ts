import { beforeAll, describe, expect, it } from "vitest";

import { ShapeCanvasEngine, type EngineEvent } from "../renderer/engine";
import { ensureSceneCore, loadSceneCore, type SceneCore } from "../bridge/sceneCoreWasm";
import { emptyObjectScene, type Object as SceneObject, type ObjectScene } from "../shared/object";
import type { RustInputBatchResult, RustWebGpuRenderer } from "../bridge/wasmLoader";
import type { CameraState } from "../renderer/scene";

const CAMERA: CameraState = { x: 0, y: 0, zoom: 1 };

let core: SceneCore;

beforeAll(async () => {
  await ensureSceneCore();
  core = await loadSceneCore();
});

function obj(id: string, parent: string | undefined): SceneObject {
  return {
    id,
    ...(parent ? { parent } : {}),
    order: "a0",
    geometry: { d: "M 0 0 L 8 0" }
  } as SceneObject;
}

// `frame-1` is a container (has a child), `rect-1` is a leaf.
function routingScene(): ObjectScene {
  return { ...emptyObjectScene(), objects: [obj("frame-1", undefined), obj("child", "frame-1"), obj("rect-1", undefined)] };
}

// Returns the supplied objectDoubleClick on a double-click event, nothing otherwise.
function doubleClickRenderer(signal: RustInputBatchResult["objectDoubleClick"]): RustWebGpuRenderer {
  return {
    resize() {},
    renderFrame() {
      return { backend: "test" } as unknown as ReturnType<RustWebGpuRenderer["renderFrame"]>;
    },
    inputBatch(eventsJson): RustInputBatchResult {
      const events = JSON.parse(eventsJson) as Array<{ kind: string }>;
      const isDoubleClick = events.some((e) => e.kind === "double-click");
      return {
        camera: CAMERA,
        objectDoubleClick: isDoubleClick ? signal : null
      };
    }
  };
}

// Records the engine's listeners so the test can fire a real dblclick.
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

function overlayRoot(): HTMLElement {
  return { append() {} } as unknown as HTMLElement;
}

function driveDoubleClick(
  signal: RustInputBatchResult["objectDoubleClick"]
): Extract<EngineEvent, { type: "object-double-click" }> | null {
  const events: EngineEvent[] = [];
  const { canvas, fire } = recordingCanvas();
  // eslint-disable-next-line no-new
  new ShapeCanvasEngine({
    canvas,
    overlayRoot: overlayRoot(),
    backend: "test",
    webGpuRenderer: doubleClickRenderer(signal),
    onEvent: (event) => events.push(event)
  });
  fire("dblclick", { clientX: 120, clientY: 80, preventDefault() {} });
  return (events.find((e) => e.type === "object-double-click") ?? null) as
    | Extract<EngineEvent, { type: "object-double-click" }>
    | null;
}

describe("double-click drill-in routing", () => {
  it("routes a container double-click (hasChildren) to a drill-in", () => {
    const event = driveDoubleClick({ id: "frame-1", hasChildren: true });
    expect(event).toEqual({ type: "object-double-click", id: "frame-1", hasChildren: true });
    expect(core.doubleClickAction(routingScene(), event!.id)).toEqual({ kind: "drill-in-container" });
  });

  it("routes a leaf double-click (no children) to inline text edit, not drill-in", () => {
    const event = driveDoubleClick({ id: "rect-1", hasChildren: false });
    expect(event).toEqual({ type: "object-double-click", id: "rect-1", hasChildren: false });
    expect(core.doubleClickAction(routingScene(), event!.id)).toEqual({ kind: "edit-leaf" });
  });

  it("emits nothing when the double-click misses every object", () => {
    expect(driveDoubleClick(null)).toBeNull();
  });
});
