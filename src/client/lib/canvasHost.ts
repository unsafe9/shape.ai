// Object-native Svelte↔renderer host (OB4.3).
//
// Owns the renderer lifecycle (load Rust core → create the visible WebGPU
// renderer → construct/observe/start the input+camera engine) and bridges the
// object scene to the renderer's object pipeline.
//
// The canonical scene is an `ObjectScene` (D1). `loadObjectScene` projects it to
// the renderer-core `RenderObjectScene` JSON and builds CPU object fill/stroke
// geometry through the crate's `buildObjectSceneGeometry`
// (`ObjectPipeline::build_scene_geometry`), proving the object→geometry path.
//
// RUNTIME-DEFERRED (no GPU in CI): the live object GPU PASS — uploading the built
// geometry through the visible renderer's frame loop — needs a renderer-crate
// `ShapeWebGpuRenderer.drawObjects` method that does not exist yet. Until that
// lands, the legacy `ShapeCanvasEngine` keeps the camera/input/stats loop alive
// for navigation; the object geometry is built (and its vertex/instance counts
// surfaced) but not yet rasterized. This is the flagged live-pixels gap.

import type { CameraState } from "../../shared/renderScene";
import type { ObjectScene, ObjectSelection } from "../../shared/object";
import { ShapeCanvasEngine, type ActiveTool, type EngineEvent, type FocusBoundsOptions } from "../renderer/engine";
import type { FrameStats, WorldRect } from "../renderer/scene";
import { loadRustCore, type RustCoreStatus, type RustWebGpuRenderer } from "../renderer/wasmLoader";

export type RendererStats = FrameStats;

export type RendererHealth = {
  state: "ready" | "webgpu-unavailable" | "wasm-unavailable";
  detail: string;
  rustAvailable: boolean;
  rustBackend: string;
  webGpuRendererAvailable: boolean;
};

/** Result of building object geometry: the opaque crate geometry plus counts the
 *  diagnostics can surface. `null` when the object build entry is unavailable. */
export type ObjectGeometryBuild = {
  fillVertexCount: number;
  strokeInstanceCount: number;
  raw: unknown;
} | null;

export type ShapeCanvasHostCallbacks = {
  onCameraChange: (camera: CameraState) => void;
  onStats: (stats: RendererStats) => void;
  onStatus: (message: string) => void;
  onHealthChange: (health: RendererHealth) => void;
};

const initialRustStatus: RustCoreStatus = {
  available: false,
  backend: "detecting",
  detail: "Checking generated Rust/WASM package.",
  probeWebGpu: null,
  createWebGpuRenderer: null,
  buildObjectSceneGeometry: null
};

// ---------------------------------------------------------------------------
// Object scene -> renderer-core RenderObjectScene projection (the renderer feed).
// Pure field renaming (transform / geometry.d -> geometryD); no domain op-apply.
// ---------------------------------------------------------------------------

const IDENTITY_3X3: [[number, number, number], [number, number, number], [number, number, number]] = [
  [1, 0, 0],
  [0, 1, 0],
  [0, 0, 1]
];

/** Project an `ObjectScene` to the renderer-core `RenderObjectScene` JSON shape. */
export function objectSceneToRenderObjectScene(
  scene: ObjectScene,
  camera: CameraState,
  selection: ObjectSelection,
  sceneId: string
): Record<string, unknown> {
  return {
    sceneId,
    camera,
    selection: selection.kind === "object" ? selection.id : null,
    multiSelect: selection.kind === "multi" ? selection.ids : [],
    objects: scene.objects.map((object) => ({
      id: object.id,
      parent: object.parent ?? null,
      order: object.order,
      transform: object.transform ?? IDENTITY_3X3,
      geometryD: object.geometry.d,
      fill: object.fill ?? null,
      stroke: object.stroke ?? null,
      text: object.text ?? null,
      clip: object.clip ?? false
    }))
  };
}

/**
 * Object-native renderer host. The Svelte shell provides the three DOM nodes
 * through mount(); it reads camera/stats/health through callbacks and pushes the
 * object scene through {@link loadObjectScene}.
 */
export class ShapeCanvasHost {
  private callbacks: ShapeCanvasHostCallbacks;
  private engine: ShapeCanvasEngine | null = null;
  private inputCanvas: HTMLCanvasElement | null = null;
  private webGpuCanvas: HTMLCanvasElement | null = null;
  private overlayRoot: HTMLElement | null = null;
  private observer: ResizeObserver | null = null;
  private camera: CameraState = { x: 0, y: 0, zoom: 1 };
  private rustStatus: RustCoreStatus = initialRustStatus;
  private webGpuRenderer: RustWebGpuRenderer | null = null;
  private engineWebGpuAvailable: boolean | null = null;
  private engineWebGpuDetail: string | null = null;
  private webGpuDetail = "Visible Rust/wgpu renderer has not been created.";
  private disposed = false;
  /** Latest projected object scene, so a renderer recreation can re-feed it. */
  private lastObjectScene: ObjectScene | null = null;
  private lastSelection: ObjectSelection = { kind: "canvas" };

  constructor(callbacks: ShapeCanvasHostCallbacks) {
    this.callbacks = callbacks;
  }

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

    this.rustStatus = await loadRustCore();
    if (this.disposed) return;

    await this.createWebGpuRenderer();
    if (this.disposed) return;

    this.createEngine();
    this.emitHealth();
    if (this.lastObjectScene) this.loadObjectScene(this.lastObjectScene, this.lastSelection);
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

  // ----- object scene feed -------------------------------------------------

  /**
   * Push the canonical `ObjectScene` to the renderer. Projects it to the
   * renderer-core `RenderObjectScene` and builds CPU object geometry through the
   * crate's `buildObjectSceneGeometry`. Returns the geometry build (counts +
   * raw), or null when the object build entry is unavailable.
   *
   * The live GPU object PASS is runtime-deferred (see file header): this builds
   * and validates the geometry but does not yet rasterize it through the visible
   * renderer's frame loop.
   */
  loadObjectScene(scene: ObjectScene, selection: ObjectSelection): ObjectGeometryBuild {
    this.lastObjectScene = scene;
    this.lastSelection = selection;
    const build = this.rustStatus.buildObjectSceneGeometry;
    if (!build) return null;
    try {
      const json = JSON.stringify(
        objectSceneToRenderObjectScene(scene, this.camera, selection, `object-scene-v${scene.sceneVersion}`)
      );
      const raw = build(json);
      return summarizeObjectGeometry(raw);
    } catch (error) {
      this.callbacks.onStatus(errorMessage(error, "Object geometry build failed."));
      return null;
    }
  }

  // ----- camera ------------------------------------------------------------

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

  /** CC1.4: set the active pointer tool (Select/Hand). */
  setTool(tool: ActiveTool): void {
    this.engine?.setTool(tool);
  }

  getCamera(): CameraState {
    return this.camera;
  }

  // ----- engine event routing ---------------------------------------------

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

/** Summarize the opaque crate geometry build into counts for diagnostics. */
function summarizeObjectGeometry(raw: unknown): ObjectGeometryBuild {
  const value = raw as { fillVertices?: unknown[]; strokeInstances?: unknown[] } | null;
  return {
    fillVertexCount: Array.isArray(value?.fillVertices) ? value!.fillVertices.length : 0,
    strokeInstanceCount: Array.isArray(value?.strokeInstances) ? value!.strokeInstances.length : 0,
    raw
  };
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
