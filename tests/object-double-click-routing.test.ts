// W3-G1 — live double-click drill-in routing (RA2b + AP3 glue).
//
// RA2b makes the core emit `objectDoubleClick` ({ id, hasChildren } | null) on the
// inputBatch result; AP3 added App.handleObjectDoubleClick + activeContainer but it
// never fired at runtime because the TS plumbing was missing. This pins the wiring
// end-to-end through the REAL engine: a fake renderer returns `objectDoubleClick` on
// a double-click batch, and the engine must read it and emit an `object-double-click`
// EngineEvent (the event ShapeCanvasHost routes to onObjectDoubleClick). The shell's
// drill-in vs. edit-text decision is the real `doubleClickAction` App calls, so the
// branch is exercised, not re-derived. Falsifiable: if the field is dropped anywhere
// (wasmLoader type / engine read / engine event), no event fires and the test fails.

import { describe, expect, it } from "vitest";

import { ShapeCanvasEngine, type EngineEvent } from "../src/client/renderer/engine";
import { doubleClickAction } from "../src/client/lib/grouping";
import type { RustInputBatchResult, RustWebGpuRenderer } from "../src/client/renderer/wasmLoader";
import type { CameraState } from "../src/client/renderer/scene";

const CAMERA: CameraState = { x: 0, y: 0, zoom: 1 };

// A minimal fake renderer whose `inputBatch` returns the supplied objectDoubleClick
// on a double-click event (and nothing on any other event). Only the methods the
// engine touches on this path are real; the rest are inert.
function doubleClickRenderer(signal: RustInputBatchResult["objectDoubleClick"]): RustWebGpuRenderer {
  return {
    resize() {},
    loadScene() {},
    applyPatchBatch() {},
    renderFrame() {
      return { backend: "test" } as unknown as ReturnType<RustWebGpuRenderer["renderFrame"]>;
    },
    inputBatch(eventsJson): RustInputBatchResult {
      const events = JSON.parse(eventsJson) as Array<{ kind: string }>;
      const isDoubleClick = events.some((e) => e.kind === "double-click");
      return {
        camera: CAMERA,
        hit: null,
        selection: { kind: "canvas" },
        patches: [],
        overlay: null,
        objectDoubleClick: isDoubleClick ? signal : null
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
  };
}

// A canvas stub that records listeners (engine binds them in its constructor) and
// can dispatch a recorded handler so the test can fire a real `dblclick`.
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

// Drive a real dblclick through the engine + fake renderer, returning the
// object-double-click EngineEvent the engine emitted (or null if none fired).
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

describe("W3-G1 double-click drill-in routing", () => {
  it("routes a container double-click (hasChildren) to a drill-in", () => {
    const event = driveDoubleClick({ id: "frame-1", hasChildren: true });
    // The field reached the engine and rode out as the event the host forwards.
    expect(event).toEqual({ type: "object-double-click", id: "frame-1", hasChildren: true });
    // App.handleObjectDoubleClick runs this exact decision: a container drills in
    // (sets activeContainer to the id), not into text edit.
    expect(doubleClickAction(event)).toEqual({ kind: "drill-in", id: "frame-1" });
  });

  it("routes a leaf double-click (no children) to inline text edit, not drill-in", () => {
    const event = driveDoubleClick({ id: "rect-1", hasChildren: false });
    expect(event).toEqual({ type: "object-double-click", id: "rect-1", hasChildren: false });
    expect(doubleClickAction(event)).toEqual({ kind: "edit-text", id: "rect-1" });
  });

  it("emits nothing when the double-click misses every object", () => {
    expect(driveDoubleClick(null)).toBeNull();
  });
});
