// T6.2 §4: framework-neutral Svelte↔adapter bridge.
//
// This re-hosts the glue that lived in RendererCanvasHost.tsx (the three React
// effects + refs) as a plain class with no framework dependency. The command
// surface is the existing RendererCanvasHostHandle; the event surface is the
// existing EngineEvent. No engine method, event variant, or batching path is
// added, removed, or rewritten — we only own the lifecycle (load Rust core →
// create WebGPU renderer → construct/resize/start engine) and forward events.

import type { CameraState } from "../../shared/renderScene";
import type { RenderScenePatch } from "../../shared/renderPatch";
import type { Scene, SceneSelection } from "../../shared/schema";
import { ShapeCanvasEngine, type ActiveTool, type EngineEvent, type FocusBoundsOptions } from "../renderer/engine";
import type { FrameStats, HitResult, SceneSnapshot, WorldPoint, WorldRect } from "../renderer/scene";
import { shapeSceneToFilteredRenderSnapshot } from "../../shared/renderPatch";
import { loadRustCore, type RustCoreStatus, type RustWebGpuRenderer } from "../renderer/wasmLoader";

export type RendererStats = FrameStats;

export type RendererHealth = {
  state: "ready" | "webgpu-unavailable" | "wasm-unavailable";
  detail: string;
  rustAvailable: boolean;
  rustBackend: string;
  webGpuRendererAvailable: boolean;
};

export type ShapeCanvasHostCallbacks = {
  onCameraChange: (camera: CameraState) => void;
  // T2.2: `additive` carries the shift/meta modifier held at pick time so the
  // shell can fold the hit into a transient `multi` selection.
  onSelectionChange: (selection: SceneSelection, additive: boolean) => void;
  onPatch: (patch: RenderScenePatch) => void;
  onGestureChange: (active: boolean) => void;
  onStats: (stats: RendererStats) => void;
  onStatus: (message: string) => void;
  onHealthChange: (health: RendererHealth) => void;
  // CC2.3: marquee drag ended; ids are node-then-group ids inside the rect.
  onMarquee?: (ids: string[]) => void;
  // CC4.1: right-click pick result for the context menu (selection-neutral).
  onContextPick?: (selection: SceneSelection, screen: { x: number; y: number }) => void;
};

const initialRustStatus: RustCoreStatus = {
  available: false,
  backend: "detecting",
  detail: "Checking generated Rust/WASM package.",
  probeWebGpu: null,
  createWebGpuRenderer: null
};

/**
 * Framework-neutral host that owns the ShapeCanvasEngine lifecycle. The Svelte
 * (or React) shell provides the three DOM nodes through mount(), and reads
 * everything else through the callbacks. Command methods mirror
 * RendererCanvasHostHandle one-to-one.
 */
export class ShapeCanvasHost {
  private callbacks: ShapeCanvasHostCallbacks;
  private engine: ShapeCanvasEngine | null = null;
  private inputCanvas: HTMLCanvasElement | null = null;
  private webGpuCanvas: HTMLCanvasElement | null = null;
  private overlayRoot: HTMLElement | null = null;
  private observer: ResizeObserver | null = null;
  private camera: CameraState = { x: 0, y: 0, zoom: 1 };
  private selection: SceneSelection = { kind: "canvas" };
  private rustStatus: RustCoreStatus = initialRustStatus;
  private webGpuRenderer: RustWebGpuRenderer | null = null;
  private engineWebGpuAvailable: boolean | null = null;
  private engineWebGpuDetail: string | null = null;
  private webGpuDetail = "Visible Rust/wgpu renderer has not been created.";
  private disposed = false;

  constructor(callbacks: ShapeCanvasHostCallbacks) {
    this.callbacks = callbacks;
  }

  /**
   * Mount against the three DOM nodes the shell owns. Replaces the three
   * RendererCanvasHost.tsx effects: load Rust core → create the WebGPU renderer
   * → construct/resize/start the engine. Each stage runs in order; the renderer
   * recreation re-runs the engine construction (mirroring the effect deps).
   */
  async mount(
    inputCanvas: HTMLCanvasElement,
    webGpuCanvas: HTMLCanvasElement,
    overlayRoot: HTMLElement,
    initialCamera: CameraState
  ): Promise<void> {
    this.inputCanvas = inputCanvas;
    this.webGpuCanvas = webGpuCanvas;
    this.overlayRoot = overlayRoot;
    this.camera = initialCamera;

    // Effect 1: load the generated Rust/WASM package.
    this.rustStatus = await loadRustCore();
    if (this.disposed) return;

    // Effect 2: create the visible Rust/wgpu renderer.
    await this.createWebGpuRenderer();
    if (this.disposed) return;

    // Effect 3: construct, observe, and start the engine.
    this.createEngine();
    this.emitHealth();
  }

  private async createWebGpuRenderer(): Promise<void> {
    const webGpuCanvas = this.webGpuCanvas;
    const sizeSource = this.inputCanvas;
    if (!webGpuCanvas || !sizeSource || !this.rustStatus.createWebGpuRenderer) {
      this.webGpuRenderer = null;
      this.engineWebGpuAvailable = false;
      this.engineWebGpuDetail = null;
      this.webGpuDetail = this.rustStatus.detail;
      return;
    }
    this.webGpuRenderer = null;
    this.engineWebGpuAvailable = null;
    this.engineWebGpuDetail = null;
    this.webGpuDetail = "Creating visible Rust/wgpu renderer.";
    const rect = sizeSource.getBoundingClientRect();
    try {
      const renderer = await this.rustStatus.createWebGpuRenderer(
        webGpuCanvas,
        rect.width || 1,
        rect.height || 1,
        window.devicePixelRatio || 1
      );
      if (this.disposed) return;
      this.webGpuRenderer = renderer;
      this.engineWebGpuAvailable = null;
      this.engineWebGpuDetail = null;
      this.webGpuDetail = "Visible Rust/wgpu renderer created.";
    } catch (error) {
      if (this.disposed) return;
      this.webGpuRenderer = null;
      this.engineWebGpuAvailable = false;
      this.engineWebGpuDetail = null;
      this.webGpuDetail = errorMessage(error, "Visible Rust/wgpu renderer failed to initialize.");
    }
  }

  private createEngine(): void {
    const canvas = this.inputCanvas;
    const overlayRoot = this.overlayRoot;
    if (!canvas || !overlayRoot) return;

    const engine = new ShapeCanvasEngine({
      canvas,
      overlayRoot,
      backend: this.rustStatus.available ? this.rustStatus.backend : "webgpu-wasm-unavailable",
      webGpuRenderer: this.webGpuRenderer,
      onEvent: (event) => this.handleEngineEvent(event)
    });
    this.engine = engine;

    const resize = () => {
      const rect = canvas.getBoundingClientRect();
      engine.resize(rect.width, rect.height, window.devicePixelRatio || 1);
    };
    this.observer = new ResizeObserver(resize);
    this.observer.observe(canvas);
    resize();
    engine.start();
  }

  resize(width: number, height: number, dpr = window.devicePixelRatio || 1): void {
    this.engine?.resize(width, height, dpr);
  }

  destroy(): void {
    this.disposed = true;
    this.observer?.disconnect();
    this.observer = null;
    this.engine?.stop();
    this.engine = null;
  }

  // ----- load scene -------------------------------------------------------

  /** Build a snapshot via the EXISTING adapter fn and hand it to the engine. */
  loadScene(scene: Scene, activeTagIds: string[], selection: SceneSelection): void {
    if (!this.engine) return;
    const sceneForRenderer: Scene = { ...scene, selection };
    this.engine.loadScene(
      shapeSceneToFilteredRenderSnapshot(sceneForRenderer, activeTagIds, {
        camera: this.camera,
        sceneId: `shape-scene-v${scene.sceneVersion}-production-renderer`
      })
    );
  }

  /** Hand a pre-built snapshot straight to the engine (no adapter call). */
  loadSnapshot(snapshot: SceneSnapshot): void {
    this.engine?.loadScene(snapshot);
  }

  // ----- operation batch + selection -------------------------------------

  applyPatch(patch: RenderScenePatch): string[] {
    return this.engine?.applyPatch(patch) ?? ["Renderer engine is not ready"];
  }

  syncSelection(selection: SceneSelection): string[] {
    this.selection = selection;
    const errors = this.engine?.syncSelection(selection) ?? [];
    if (errors.length > 0) this.callbacks.onStatus(errors.join("; "));
    return errors;
  }

  // ----- input batch + camera --------------------------------------------

  setCamera(camera: CameraState): void {
    if (cameraAlmostEqual(this.camera, camera)) return;
    this.camera = camera;
    this.engine?.setCamera(camera);
  }

  focusBounds(bounds: WorldRect, options?: FocusBoundsOptions): void {
    this.engine?.focusBounds(bounds, options);
  }

  fitScene(): void {
    this.engine?.fitScene();
  }

  wheelAtScreen(screen: { x: number; y: number }, deltaY: number): void {
    this.engine?.wheelAtScreen(screen, deltaY);
  }

  // CC1.4: set the active pointer tool (Select/Hand). Switching to hand cancels
  // any in-flight drag in the core.
  setTool(tool: ActiveTool): void {
    this.engine?.setTool(tool);
  }

  // CC4.1: right-click pick. Returns the picked selection (or canvas) and emits
  // the onContextPick callback; does not mutate selection or start a drag.
  contextPick(screen: WorldPoint): SceneSelection {
    return hitToSceneSelection(this.engine?.contextPick(screen) ?? null);
  }

  // ----- diagnostics ------------------------------------------------------

  getSnapshot(): SceneSnapshot | null {
    return this.engine?.getSnapshot() ?? null;
  }

  getCamera(): CameraState {
    return this.camera;
  }

  // ----- engine event routing (mirrors RendererCanvasHost.handleEngineEvent)

  private handleEngineEvent(event: EngineEvent): void {
    if (event.type === "stats") {
      this.engineWebGpuAvailable = event.stats.webGpuRendererAvailable;
      this.callbacks.onStats(event.stats);
      const nextCamera = cameraFromStats(event.stats);
      if (nextCamera && !cameraAlmostEqual(this.camera, nextCamera)) {
        this.camera = nextCamera;
        this.callbacks.onCameraChange(nextCamera);
      }
      this.emitHealth();
      return;
    }

    if (event.type === "selection") {
      this.callbacks.onSelectionChange(hitToSceneSelection(event.hit), event.additive);
      return;
    }

    if (event.type === "patch") {
      if (event.errors.length > 0) {
        this.callbacks.onStatus(event.errors.join("; "));
        return;
      }
      this.callbacks.onPatch(event.patch);
      return;
    }

    if (event.type === "gesture") {
      this.callbacks.onGestureChange(event.active);
      return;
    }

    if (event.type === "marquee") {
      this.callbacks.onMarquee?.(event.ids);
      return;
    }

    if (event.type === "context-pick") {
      this.callbacks.onContextPick?.(hitToSceneSelection(event.hit), event.screen);
      return;
    }

    if (event.type === "status") {
      if (isWebGpuRuntimeStatus(event.message)) {
        this.engineWebGpuAvailable = false;
        this.engineWebGpuDetail = event.message;
        this.emitHealth();
      }
      this.callbacks.onStatus(event.message);
    }
  }

  private emitHealth(): void {
    const runtimeWebGpuAvailable = this.engineWebGpuAvailable ?? Boolean(this.webGpuRenderer);
    const state = runtimeWebGpuAvailable
      ? "ready"
      : this.rustStatus.available
        ? "webgpu-unavailable"
        : "wasm-unavailable";
    this.callbacks.onHealthChange({
      state,
      detail: this.engineWebGpuDetail ?? this.webGpuDetail,
      rustAvailable: this.rustStatus.available,
      rustBackend: this.rustStatus.backend,
      webGpuRendererAvailable: runtimeWebGpuAvailable
    });
  }
}

// Pure helpers moved from RendererCanvasHost.tsx (no React, no DOM).

export function hitToSceneSelection(hit: HitResult | null): SceneSelection {
  if (!hit) return { kind: "canvas" };
  if (hit.kind === "group") return { kind: "group", id: hit.id };
  if (hit.kind === "edge") return { kind: "edge", id: hit.id };
  return { kind: "node", id: hit.id };
}

export function cameraFromStats(stats: RendererStats): CameraState | null {
  if (stats.rustCameraX === null || stats.rustCameraY === null || stats.rustCameraZoom === null) return null;
  return { x: stats.rustCameraX, y: stats.rustCameraY, zoom: stats.rustCameraZoom };
}

export function cameraAlmostEqual(left: CameraState, right: CameraState): boolean {
  return Math.abs(left.x - right.x) < 0.5 && Math.abs(left.y - right.y) < 0.5 && Math.abs(left.zoom - right.zoom) < 0.0005;
}

function errorMessage(error: unknown, fallback: string): string {
  if (error instanceof Error) return error.message || fallback;
  if (typeof error === "string") return error || fallback;
  const message = String(error);
  return message === "[object Object]" ? fallback : message;
}

function isWebGpuRuntimeStatus(message: string): boolean {
  return message.startsWith("WebGPU renderer unavailable:") || message.startsWith("WebGPU render failed");
}
