// The two-tier UI dispatch resolves a fired widget into a typed intent through the set model
// (`shape_ui::resolve`), and the engine relays each resolved intent VERBATIM as a `ui-intent` EngineEvent
// — the shell then routes it to its existing op-authoring handler. These drive the ENGINE relay with a
// renderer whose uiPointer/uiKey return intents, and assert the engine forwards each one without computing
// anything from it (the thin-shell rule). FAILS if the relay drops the intent or starts deriving its own.

import { describe, expect, it } from "vitest";

import { ShapeCanvasEngine, type EngineEvent } from "../renderer/engine";
import type { RustInputBatchResult, RustWebGpuRenderer, UiDispatchResult, UiIntent } from "../bridge/wasmLoader";
import type { CameraState } from "../renderer/scene";

const CAMERA: CameraState = { x: 0, y: 0, zoom: 1 };

function canvasStub(): HTMLCanvasElement {
  return {
    width: 0,
    height: 0,
    addEventListener() {},
    removeEventListener() {},
    setPointerCapture() {},
    releasePointerCapture() {},
    getBoundingClientRect() {
      return { left: 0, top: 0, width: 800, height: 600 };
    }
  } as unknown as HTMLCanvasElement;
}

function overlayRoot(): HTMLElement {
  return { append() {} } as unknown as HTMLElement;
}

// A renderer whose UI dispatch returns the given intents (the core-resolved requests). The down phase
// consumes; move/up are inert. Mirrors the wasm dispatch shape the shell forwards.
function rendererWithIntents(intents: UiIntent[]): RustWebGpuRenderer {
  const consumed = (phase: string): UiDispatchResult => ({
    consumed: phase === "down",
    sceneChanged: phase === "down",
    actions: [{ type: "pressed", id: "cmd:undo" }],
    edit: null,
    intents: phase === "down" ? intents : []
  });
  return {
    resize() {},
    renderFrame() {
      return { backend: "test" } as unknown as ReturnType<RustWebGpuRenderer["renderFrame"]>;
    },
    inputBatch(): RustInputBatchResult {
      return { camera: CAMERA, objectDoubleClick: null };
    },
    uiPointer(phase: string): UiDispatchResult {
      return consumed(phase);
    },
    uiKey(): UiDispatchResult {
      return consumed("down");
    },
    setUiModel() {}
  };
}

function collectEvents(renderer: RustWebGpuRenderer): { engine: ShapeCanvasEngine; events: EngineEvent[] } {
  const events: EngineEvent[] = [];
  const engine = new ShapeCanvasEngine({
    canvas: canvasStub(),
    overlayRoot: overlayRoot(),
    backend: "test",
    webGpuRenderer: renderer,
    onEvent: (event) => events.push(event)
  });
  return { engine, events };
}

describe("the engine relays each core-resolved UI intent as a verbatim ui-intent event", () => {
  it("a uiKey dispatch carrying a Command intent forwards it (and the raw action) unchanged", () => {
    const { engine, events } = collectEvents(rendererWithIntents([{ type: "command", id: "undo" }]));
    engine.uiKey({ key: "z", text: null, ctrl: false, meta: true, alt: false });
    const intentEvents = events.filter((e) => e.type === "ui-intent");
    expect(intentEvents).toHaveLength(1);
    expect(intentEvents[0]).toEqual({ type: "ui-intent", intent: { type: "command", id: "undo" } });
    // The raw action is still relayed for the shell's focus/IME bookkeeping (no decision from it).
    expect(events.some((e) => e.type === "ui-action")).toBe(true);
  });

  it("forwards a SelectColor intent verbatim (the shell, not the engine, applies it)", () => {
    const intent: UiIntent = { type: "selectColor", hex: "#00ff00" };
    const { engine, events } = collectEvents(rendererWithIntents([intent]));
    // A consumed pointer-down routes the dispatch (the engine's routeUiPointerDown path).
    engine["routeUiPointerDown"]({ pointerId: 1, clientX: 10, clientY: 10, button: 0 } as unknown as PointerEvent);
    const intentEvents = events.filter((e) => e.type === "ui-intent");
    expect(intentEvents).toHaveLength(1);
    expect(intentEvents[0]).toEqual({ type: "ui-intent", intent });
  });

  it("forwards every resolved intent in order (no drop, no re-derive)", () => {
    const intents: UiIntent[] = [
      { type: "selectCanvas", id: "c2" },
      { type: "inspectorEdit", controlId: "x", opKind: "set-transform", field: "x", unitScale: 1, value: 120 }
    ];
    const { engine, events } = collectEvents(rendererWithIntents(intents));
    engine.uiKey({ key: "Enter", text: null, ctrl: false, meta: false, alt: false });
    const relayed = events.filter((e) => e.type === "ui-intent").map((e) => (e as { intent: UiIntent }).intent);
    expect(relayed).toEqual(intents);
  });
});
