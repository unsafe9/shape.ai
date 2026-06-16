import { describe, expect, it } from "vitest";

import { ShapeCanvasEngine } from "../renderer/engine";
import type { RustInputBatchResult, RustWebGpuRenderer } from "../bridge/wasmLoader";
import type { CameraState } from "../renderer/scene";

const CAMERA: CameraState = { x: 0, y: 0, zoom: 1 };

// A renderer WITHOUT the UI exports — a wasm build (or CI mock) predating loadUiScene/hitUi must
// still satisfy RustWebGpuRenderer and leave the UI calls no-ops, never throwing.
function rendererWithoutUi(): RustWebGpuRenderer {
  return {
    resize() {},
    renderFrame() {
      return { backend: "test" } as unknown as ReturnType<RustWebGpuRenderer["renderFrame"]>;
    },
    inputBatch(): RustInputBatchResult {
      return { camera: CAMERA, objectDoubleClick: null };
    }
  };
}

function overlayRoot(): HTMLElement {
  return { append() {} } as unknown as HTMLElement;
}

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

describe("wasmLoader UI typings are optional + feature-detected", () => {
  it("a renderer without loadUiScene/hitUi/setUiModel still satisfies RustWebGpuRenderer (typechecks)", () => {
    // The assignment below is the type-level assertion: it would not compile if the UI methods
    // were non-optional on RustWebGpuRenderer. setUiModel is added optional for the same reason.
    const renderer: RustWebGpuRenderer = rendererWithoutUi();
    expect(typeof renderer.hitUi).toBe("undefined");
    expect(typeof renderer.loadUiScene).toBe("undefined");
    expect(typeof renderer.setUiModel).toBe("undefined");
  });

  it("engine.setUiModel is a no-op (returns false, never throws) when the renderer lacks setUiModel", () => {
    const engine = new ShapeCanvasEngine({
      canvas: canvasStub(),
      overlayRoot: overlayRoot(),
      backend: "test",
      webGpuRenderer: rendererWithoutUi(),
      onEvent: () => {}
    });
    expect(engine.setUiModel("{}")).toBe(false);
  });

  it("engine.hitUi is a no-op (returns null, never throws) when the renderer lacks hitUi", () => {
    const engine = new ShapeCanvasEngine({
      canvas: canvasStub(),
      overlayRoot: overlayRoot(),
      backend: "test",
      webGpuRenderer: rendererWithoutUi(),
      onEvent: () => {}
    });
    expect(engine.hitUi({ x: 40, y: 40 })).toBeNull();
  });

  it("engine.hitUi returns null without a renderer", () => {
    const engine = new ShapeCanvasEngine({
      canvas: canvasStub(),
      overlayRoot: overlayRoot(),
      backend: "test",
      webGpuRenderer: null,
      onEvent: () => {}
    });
    expect(engine.hitUi({ x: 40, y: 40 })).toBeNull();
  });
});
