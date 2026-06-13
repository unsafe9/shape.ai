// B4: world↔screen projection lives in the Rust core. The shell holds no TS affine — every
// projection routes through the renderer's `worldToScreen` / `screenToWorld` wasm bridge over the LIVE
// core camera. These assertions fail if the shell reintroduces its own camera math.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { ShapeCanvasEngine } from "../renderer/engine";
import type { RustInputBatchResult, RustWebGpuFrameStats, RustWebGpuRenderer } from "../bridge/wasmLoader";

// A mock renderer projecting through a fixed pan+zoom camera (`screen = world * zoom + offset`), the
// shape of the real core's affine. Non-identity values prove the bridge, not a {0,0,1} accident.
const CAMERA = { x: 40, y: 30, zoom: 0.5 };

function mockRenderer(): RustWebGpuRenderer {
  return {
    resize() {},
    renderFrame() {
      return {} as unknown as RustWebGpuFrameStats;
    },
    inputBatch(): RustInputBatchResult {
      return { camera: CAMERA };
    },
    worldToScreen(worldX: number, worldY: number) {
      return { x: worldX * CAMERA.zoom + CAMERA.x, y: worldY * CAMERA.zoom + CAMERA.y };
    },
    screenToWorld(screenX: number, screenY: number) {
      return { x: (screenX - CAMERA.x) / CAMERA.zoom, y: (screenY - CAMERA.y) / CAMERA.zoom };
    }
  } as unknown as RustWebGpuRenderer;
}

function makeEngine(renderer: RustWebGpuRenderer | null): ShapeCanvasEngine {
  return new ShapeCanvasEngine({
    canvas: {
      addEventListener() {},
      removeEventListener() {},
      getBoundingClientRect() {
        return { left: 0, top: 0, width: 800, height: 600 };
      }
    } as unknown as HTMLCanvasElement,
    overlayRoot: { append() {} } as unknown as HTMLElement,
    backend: "test",
    webGpuRenderer: renderer,
    onEvent() {}
  });
}

describe("B4: the core projection bridge maps world↔screen under pan+zoom", () => {
  it("projectWorldToScreen returns the expected screen px through the core", () => {
    const engine = makeEngine(mockRenderer());
    const screen = engine.projectWorldToScreen({ x: 200, y: 160 });
    expect(screen).not.toBeNull();
    expect(screen!.x).toBeCloseTo(200 * 0.5 + 40, 6);
    expect(screen!.y).toBeCloseTo(160 * 0.5 + 30, 6);
  });

  it("projectScreenToWorld is the exact inverse through the core", () => {
    const engine = makeEngine(mockRenderer());
    const world = engine.projectScreenToWorld({ x: 200 * 0.5 + 40, y: 160 * 0.5 + 30 });
    expect(world).not.toBeNull();
    expect(world!.x).toBeCloseTo(200, 6);
    expect(world!.y).toBeCloseTo(160, 6);
  });

  it("returns null when the renderer is not live (no TS affine fallback)", () => {
    const engine = makeEngine(null);
    expect(engine.projectWorldToScreen({ x: 1, y: 2 })).toBeNull();
    expect(engine.projectScreenToWorld({ x: 1, y: 2 })).toBeNull();
  });
});

describe("B4: renderer/scene.ts holds no world↔screen affine", () => {
  const sceneSource = readFileSync(fileURLToPath(new URL("../renderer/scene.ts", import.meta.url)), "utf8");

  it("exports no screenToWorld / worldToScreen", () => {
    expect(sceneSource).not.toContain("export function screenToWorld");
    expect(sceneSource).not.toContain("export function worldToScreen");
  });
});
