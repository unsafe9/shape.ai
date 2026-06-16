import { describe, expect, it, vi } from "vitest";

import { ShapeCanvasEngine, type EngineEvent } from "../renderer/engine";
import type { RustInputBatchResult, RustWebGpuRenderer, UiDispatchResult } from "../bridge/wasmLoader";
import type { CameraState } from "../renderer/scene";

const CAMERA: CameraState = { x: 0, y: 0, zoom: 1 };

// The UI widget sits at this screen point; `hitUi` returns its id there, null elsewhere.
const WIDGET_POINT = { x: 40, y: 40 };
const OFF_WIDGET_POINT = { x: 400, y: 400 };

// Tracks every inputBatch call and (separately) the batches carrying a pointer-down — the canvas
// "the click reached me" signal. `hitUi` returns "ui-proof-button" at WIDGET_POINT, null elsewhere,
// exactly the renderer's screen-space pick.
function uiRenderer(): {
  renderer: RustWebGpuRenderer;
  inputBatchCalls: () => number;
  pointerDownBatches: () => number;
} {
  let inputBatchCalls = 0;
  let pointerDownBatches = 0;
  const renderer: RustWebGpuRenderer = {
    resize() {},
    renderFrame() {
      return { backend: "test" } as unknown as ReturnType<RustWebGpuRenderer["renderFrame"]>;
    },
    hitUi(screenX: number, screenY: number): string | null {
      return screenX === WIDGET_POINT.x && screenY === WIDGET_POINT.y ? "ui-proof-button" : null;
    },
    inputBatch(eventsJson): RustInputBatchResult {
      inputBatchCalls += 1;
      const events = JSON.parse(eventsJson) as Array<{ kind: string }>;
      if (events.some((e) => e.kind === "pointer-down")) pointerDownBatches += 1;
      return { camera: CAMERA, objectDoubleClick: null };
    }
  };
  return {
    renderer,
    inputBatchCalls: () => inputBatchCalls,
    pointerDownBatches: () => pointerDownBatches
  };
}

// A renderer exposing the ui-core RUNTIME dispatch (uiPointer). A pointer-DOWN at WIDGET_POINT is
// consumed (a captured widget); a DOWN elsewhere is not. Once a down consumed, the engine routes the
// whole move/up stream to uiPointer via uiPointerActive — the canvas never sees it. Tracks every
// uiPointer phase + every inputBatch so the test pins "0 inputBatch across a UI drag".
function runtimeRenderer(): {
  renderer: RustWebGpuRenderer;
  pointerEventBatches: () => number;
  uiPhases: () => string[];
} {
  // Counts batches carrying a pointer EVENT (down/move/up) — the canvas "the drag reached me" signal.
  // Modifier-mirror batches (set-coarse-rotate) are NOT pointer events and are excluded (as the P1 test
  // already notes), so an off-widget drag is exactly three pointer-event batches.
  let pointerEventBatches = 0;
  const uiPhases: string[] = [];
  const renderer: RustWebGpuRenderer = {
    resize() {},
    renderFrame() {
      return { backend: "test" } as unknown as ReturnType<RustWebGpuRenderer["renderFrame"]>;
    },
    uiPointer(phase: string, screenX: number, screenY: number): UiDispatchResult {
      uiPhases.push(phase);
      const onWidget = screenX === WIDGET_POINT.x && screenY === WIDGET_POINT.y;
      // Only a DOWN decides consumption (the captured drag follows via uiPointerActive); move/up while
      // captured return consumed too, matching the core (a captured slider owns the stream).
      const consumed = phase !== "down" || onWidget;
      return { consumed, sceneChanged: consumed, actions: [], edit: null };
    },
    inputBatch(eventsJson): RustInputBatchResult {
      const events = JSON.parse(eventsJson) as Array<{ kind: string }>;
      if (events.some((e) => e.kind.startsWith("pointer-"))) pointerEventBatches += 1;
      return { camera: CAMERA, objectDoubleClick: null };
    }
  };
  return { renderer, pointerEventBatches: () => pointerEventBatches, uiPhases: () => uiPhases };
}

// Records the engine's listeners so the test can fire real pointer/mouse events.
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

function makeEngine(renderer: RustWebGpuRenderer): {
  fire: (type: string, event: unknown) => void;
  events: EngineEvent[];
} {
  const events: EngineEvent[] = [];
  const { canvas, fire } = recordingCanvas();
  // eslint-disable-next-line no-new
  new ShapeCanvasEngine({
    canvas,
    overlayRoot: overlayRoot(),
    backend: "test",
    webGpuRenderer: renderer,
    onEvent: (event) => events.push(event)
  });
  return { fire, events };
}

// clientX/clientY map 1:1 to eventPoint (rect left/top = 0).
function pointerDownAt(p: { x: number; y: number }) {
  return { pointerType: "touch", pointerId: 1, button: 0, clientX: p.x, clientY: p.y, shiftKey: false, altKey: false };
}
function mouseDownAt(p: { x: number; y: number }) {
  return { button: 0, clientX: p.x, clientY: p.y, shiftKey: false, altKey: false, preventDefault() {} };
}
function pointerMoveAt(p: { x: number; y: number }) {
  return { pointerType: "touch", pointerId: 1, clientX: p.x, clientY: p.y, shiftKey: false, altKey: false };
}
function pointerUpAt(p: { x: number; y: number }) {
  return { pointerType: "touch", pointerId: 1, clientX: p.x, clientY: p.y, shiftKey: false, altKey: false };
}

describe("UI pre-pass consumes a pointer-down over a UI widget", () => {
  it("a pointer-down OVER the widget is consumed (zero inputBatch) and emits a ui-hit status", () => {
    const log = vi.spyOn(console, "log").mockImplementation(() => {});
    const { renderer, inputBatchCalls, pointerDownBatches } = uiRenderer();
    const { fire, events } = makeEngine(renderer);

    fire("pointerdown", pointerDownAt(WIDGET_POINT));

    // The pre-pass runs before any canvas forwarding, so a consumed click sends NOTHING to the core.
    expect(inputBatchCalls()).toBe(0);
    expect(pointerDownBatches()).toBe(0);
    const status = events.find((e) => e.type === "status") as Extract<EngineEvent, { type: "status" }> | undefined;
    expect(status?.message).toMatch(/^ui-hit ui-proof-button$/);
    log.mockRestore();
  });

  it("a mouse-down OVER the widget is consumed (zero inputBatch) and emits a ui-hit status", () => {
    const log = vi.spyOn(console, "log").mockImplementation(() => {});
    const { renderer, inputBatchCalls, pointerDownBatches } = uiRenderer();
    const { fire, events } = makeEngine(renderer);

    fire("mousedown", mouseDownAt(WIDGET_POINT));

    expect(inputBatchCalls()).toBe(0);
    expect(pointerDownBatches()).toBe(0);
    const status = events.find((e) => e.type === "status") as Extract<EngineEvent, { type: "status" }> | undefined;
    expect(status?.message).toMatch(/^ui-hit ui-proof-button$/);
    log.mockRestore();
  });

  it("a pointer-down OFF the widget falls through: exactly one pointer-down inputBatch", () => {
    const { renderer, pointerDownBatches } = uiRenderer();
    const { fire, events } = makeEngine(renderer);

    fire("pointerdown", pointerDownAt(OFF_WIDGET_POINT));

    // The canvas sees the click exactly as today — one pointer-down batch (modifier-mirror batches
    // are not pointer-downs and are not the click).
    expect(pointerDownBatches()).toBe(1);
    expect(events.some((e) => e.type === "status" && e.message.startsWith("ui-hit"))).toBe(false);
  });

  it("a mouse-down OFF the widget falls through: exactly one pointer-down inputBatch", () => {
    const { renderer, pointerDownBatches } = uiRenderer();
    const { fire, events } = makeEngine(renderer);

    fire("mousedown", mouseDownAt(OFF_WIDGET_POINT));

    expect(pointerDownBatches()).toBe(1);
    expect(events.some((e) => e.type === "status" && e.message.startsWith("ui-hit"))).toBe(false);
  });
});

describe("UI runtime dispatch routes a whole drag and never touches the canvas", () => {
  it("down->move->up OVER a widget routes ALL THREE phases to uiPointer and sends 0 inputBatch", () => {
    const { renderer, pointerEventBatches, uiPhases } = runtimeRenderer();
    const { fire } = makeEngine(renderer);

    // A slider drag: the down captures, then the engine routes move/up via uiPointerActive.
    fire("pointerdown", pointerDownAt(WIDGET_POINT));
    fire("pointermove", pointerMoveAt({ x: 80, y: 40 }));
    fire("pointermove", pointerMoveAt({ x: 120, y: 40 }));
    fire("pointerup", pointerUpAt({ x: 120, y: 40 }));

    // Every phase reached the UI runtime; the captured drag never forwarded a canvas pointer batch.
    expect(uiPhases()).toEqual(["down", "move", "move", "up"]);
    expect(pointerEventBatches()).toBe(0);
  });

  it("an OFF-widget down->move->up is UNCHANGED: it routes to the canvas, not the UI runtime", () => {
    const { renderer, pointerEventBatches, uiPhases } = runtimeRenderer();
    const { fire } = makeEngine(renderer);

    fire("pointerdown", pointerDownAt(OFF_WIDGET_POINT));
    fire("pointermove", pointerMoveAt({ x: 420, y: 400 }));
    fire("pointerup", pointerUpAt({ x: 420, y: 400 }));

    // The DOWN is offered to the runtime once (not consumed) — move/up never reach it (no capture); the
    // canvas saw the full drag exactly as today (down + move + up = three pointer batches).
    expect(uiPhases()).toEqual(["down"]);
    expect(pointerEventBatches()).toBe(3);
  });

  it("a UI-consumed pointer-down emits the dispatch actions as ui-action EngineEvents (opaque relay)", () => {
    let downSeen = false;
    const renderer: RustWebGpuRenderer = {
      resize() {},
      renderFrame() {
        return { backend: "test" } as unknown as ReturnType<RustWebGpuRenderer["renderFrame"]>;
      },
      uiPointer(phase: string): UiDispatchResult {
        if (phase === "down") {
          downSeen = true;
          return {
            consumed: true,
            sceneChanged: true,
            actions: [{ type: "sliderChanged", id: "demo-slider", value: 0.5 }],
            edit: null
          };
        }
        return { consumed: true, sceneChanged: false, actions: [], edit: null };
      },
      inputBatch(): RustInputBatchResult {
        return { camera: CAMERA, objectDoubleClick: null };
      }
    };
    const { fire, events } = makeEngine(renderer);
    fire("pointerdown", pointerDownAt(WIDGET_POINT));

    expect(downSeen).toBe(true);
    const action = events.find((e) => e.type === "ui-action") as Extract<EngineEvent, { type: "ui-action" }> | undefined;
    expect(action?.action).toEqual({ type: "sliderChanged", id: "demo-slider", value: 0.5 });
  });

  it("a focus dispatch carrying result.edit emits a ui-edit EngineEvent (the dead-wire fix)", () => {
    // The S4 fix: uiPointer must SURFACE result.edit (a focused TextInput's mount request) as a ui-edit
    // event so the shell mounts the IME surface. FAILS if the edit is dropped on the floor again (the
    // exact regression this re-wires) — a behavioral assertion, not a source-string match.
    const edit = { id: "demo-input", rect: [10, 20, 200, 32] as [number, number, number, number], value: "ab" };
    const renderer: RustWebGpuRenderer = {
      resize() {},
      renderFrame() {
        return { backend: "test" } as unknown as ReturnType<RustWebGpuRenderer["renderFrame"]>;
      },
      uiPointer(phase: string): UiDispatchResult {
        if (phase === "down") return { consumed: true, sceneChanged: true, actions: [], edit };
        return { consumed: true, sceneChanged: false, actions: [], edit: null };
      },
      inputBatch(): RustInputBatchResult {
        return { camera: CAMERA, objectDoubleClick: null };
      }
    };
    const { fire, events } = makeEngine(renderer);
    fire("pointerdown", pointerDownAt(WIDGET_POINT));

    const uiEdit = events.find((e) => e.type === "ui-edit") as Extract<EngineEvent, { type: "ui-edit" }> | undefined;
    expect(uiEdit, "uiPointer must relay result.edit as a ui-edit event").toBeDefined();
    expect(uiEdit?.edit).toEqual(edit);
  });
});
