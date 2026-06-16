import { describe, expect, it } from "vitest";
import { ShapeCanvasEngine } from "../renderer/engine";
import type { RustInputBatchResult, RustWebGpuRenderer } from "../bridge/wasmLoader";
import { TextEditHost, type TextEditDocument, type TextEditEditable, type TextEditEvent } from "../ime/textEditHost";

// Defect D: the canvas left-button mousedown calls preventDefault() to suppress native text-selection /
// image-drag, which also suppresses the browser blur of a focused editing surface — so the surface's
// blur->commit never fires and typed text is lost. The fix blurs the active surface BEFORE preventDefault
// (preventDefault does not cancel an explicit .blur()), committing the edit. The blur logic now lives in
// ONE place — the shared TextEditHost.blurActive — which the engine invokes through an injected hook.
// Re-pointed after the hazard moved out of engine.ts into the library.

const CAMERA = { x: 0, y: 0, zoom: 1 };

function quietRenderer(): RustWebGpuRenderer {
  return {
    resize() {},
    renderFrame() {
      return { backend: "test" } as unknown as ReturnType<RustWebGpuRenderer["renderFrame"]>;
    },
    inputBatch(): RustInputBatchResult {
      return { camera: CAMERA };
    }
  } as unknown as RustWebGpuRenderer;
}

function captureCanvas(order: string[]): {
  canvas: HTMLCanvasElement;
  fire: (type: string, init: Record<string, unknown>) => void;
} {
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
    const event = { type, preventDefault: () => order.push("preventDefault"), ...init } as unknown as Event;
    for (const listener of listeners.get(type) ?? []) listener(event);
  };
  return { canvas, fire };
}

describe("inline text commit on canvas click-away", () => {
  it("the engine invokes the injected blur hook BEFORE preventDefault on a left-button mousedown", () => {
    const order: string[] = [];
    const { canvas, fire } = captureCanvas(order);
    const engine = new ShapeCanvasEngine({
      canvas,
      overlayRoot: { append() {} } as unknown as HTMLElement,
      backend: "test",
      webGpuRenderer: quietRenderer(),
      onEvent: () => {},
      blurActiveEditable: () => order.push("blur")
    });
    engine.renderFrame(performance.now());

    fire("mousedown", { button: 0, shiftKey: false, altKey: false, clientX: 100, clientY: 100 });

    // The hook ran BEFORE preventDefault so the commit is not itself a victim of focus suppression. FAILS
    // if the engine drops the hook or moves it after preventDefault.
    expect(order).toEqual(["blur", "preventDefault"]);
  });

  it("the engine still calls preventDefault when no blur hook is injected", () => {
    const order: string[] = [];
    const { canvas, fire } = captureCanvas(order);
    const engine = new ShapeCanvasEngine({
      canvas,
      overlayRoot: { append() {} } as unknown as HTMLElement,
      backend: "test",
      webGpuRenderer: quietRenderer(),
      onEvent: () => {}
    });
    engine.renderFrame(performance.now());

    fire("mousedown", { button: 0, shiftKey: false, altKey: false, clientX: 100, clientY: 100 });

    expect(order).toEqual(["preventDefault"]);
  });

  it("TextEditHost.blurActive() commits the focused surface synchronously (the click-away invariant)", () => {
    // The hook the engine injects is the library's blurActive. Lifting the invariant into the library:
    // a focused editing surface is committed (blurred) synchronously, so a subsequent preventDefault can
    // no longer suppress the commit. FAILS if blurActive stops blurring the focused surface.
    let active: { isContentEditable?: boolean; blur?: () => void } | null = null;
    const committed: string[] = [];
    const listeners = new Map<string, ((event: TextEditEvent) => void)[]>();
    const editable: TextEditEditable = {
      textContent: "",
      contentEditable: "",
      className: "",
      role: "",
      ariaLabel: "",
      tabIndex: -1,
      style: {},
      isContentEditable: true,
      focus() {
        active = editable as unknown as { isContentEditable?: boolean; blur?: () => void };
      },
      blur() {
        active = null;
        for (const l of listeners.get("blur") ?? []) l({ preventDefault() {}, stopPropagation() {} });
      },
      remove() {},
      addEventListener(type, listener) {
        const list = listeners.get(type) ?? [];
        list.push(listener);
        listeners.set(type, list);
      }
    };
    const doc: TextEditDocument = {
      createElement: () => editable,
      getSelection: () => ({ removeAllRanges() {}, addRange() {} }),
      createRange: () => ({ selectNodeContents() {}, collapse() {} }),
      get activeElement() {
        return active;
      }
    };
    const host = new TextEditHost({ overlayRoot: { append() {} }, doc });
    host.open(
      { rect: { x: 0, y: 0, width: 10, height: 10 }, value: "" },
      { onInput: () => {}, onCommit: (v) => committed.push(v) }
    );
    editable.textContent = "kept";

    host.blurActive();

    expect(committed).toEqual(["kept"]);
  });
});
