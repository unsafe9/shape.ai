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
// The renderer crate now exposes `ShapeWebGpuRenderer.loadObjectScene` (build +
// upload the object geometry on the live device/surface) and `.drawObjects` (record
// the object GPU pass). `loadObjectScene` below feature-detects and calls them when
// the live renderer is present.
//
// RUNTIME-DEFERRED (no GPU in CI): the live object GPU PASS still cannot run in the
// test/build environment because there is no WebGPU device — `createWebGpuRenderer`
// only succeeds in a real browser. So in CI the object geometry is built through the
// CPU `buildObjectSceneGeometry` entry (and its counts surfaced); the actual
// `loadObjectScene`/`drawObjects` rasterization is exercised at runtime in the
// browser. The legacy `ShapeCanvasEngine` keeps the camera/input/stats loop alive.
// This is the flagged live-pixels gap: build-verified, GPU-runtime-deferred.

import type { CameraState } from "../../shared/geometry";
import { GEOMETRY_QUANTUM_PER_PX, type ObjectScene, type ObjectSelection, type Stroke } from "../../shared/object";
import { ShapeCanvasEngine, type ActiveTool, type EngineEvent, type FocusBoundsOptions, type TransformKind } from "../renderer/engine";
import type { FrameStats, RenderTransform3x3, WorldRect } from "../renderer/scene";
import { loadRustCore, type HoverAffordance, type RustCoreStatus, type RustWebGpuRenderer } from "../renderer/wasmLoader";

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
  // FC-08/W2-05: object-path input results from the renderer. `onSelectObject` fires
  // on the pointer-down that picked an object; `onTransformPreview` on each drag move
  // (the cumulative world-space delta matrix + gesture kind — a non-destructive
  // preview); `onTransformCommit` once on pointer-up when the drag moved (the single
  // undoable op); `onMarquee` on an empty-start drag's pointer-up.
  // W2-03: `additive` is true when shift/meta was held at pick time, so the shell
  // toggles the object in/out of the multi-select set instead of replacing it.
  onSelectObject: (id: string, additive: boolean) => void;
  onTransformPreview: (id: string, matrix: RenderTransform3x3, kind: TransformKind) => void;
  // v3 §3 (DU4): `detach` is the C2 `detach-alt` gesture bit (Alt held at release)
  // — the shell branches an anchored open-class body drag into a whole translate
  // plus an anchor-clearing set-anchor (the class/anchor judgment is the core's).
  onTransformCommit: (id: string, matrix: RenderTransform3x3, kind: TransformKind, detach: boolean) => void;
  // v3 §2b: open-class endpoint drag. `onEndpointPreview` rides each endpoint-drag
  // move (the chord deform is already live on the GPU; the payload carries the
  // release-snap probe so the shell drives the anchor ring); `onEndpointCommit`
  // rides the pointer-up — the shell authors `endpointReleaseOps` from it (chord
  // EditGeometry + anchor rebind/unbind). Optional so a host that omits them
  // loses nothing.
  onEndpointPreview?: (id: string, nodeIndex: number, world: { x: number; y: number }, snapped: boolean, targetId: string | null) => void;
  onEndpointCommit?: (id: string, nodeIndex: number, world: { x: number; y: number }, snapped: boolean, targetId: string | null) => void;
  onMarquee: (ids: string[]) => void;
  // RA2b/AP3: a double-click landed on an object. The shell drills into a container
  // (hasChildren) or enters inline text edit on a leaf; a missed double-click never
  // fires this. Optional so a host that omits it loses nothing.
  onObjectDoubleClick?: (payload: { id: string; hasChildren: boolean }) => void;
  // FC-16: optional — no engine event routes to it. The right-click context pick
  // runs synchronously via `hitTestObjectAt` in the shell, not through an engine
  // event, so a host that omits this loses nothing.
  onContextPick?: (id: string | null) => void;
  // FC-11: freehand pen capture phases (world px). The shell accumulates the
  // points across start/move and commits the stroke to an object on `end`.
  onDraw: (phase: "start" | "move" | "end" | "cancel", world: { x: number; y: number }) => void;
  // W2-07: shape drag-create phases. `world` is the dragged corner (already snapped
  // to the nearest outline anchor when `snapped`); the shell rubber-bands a bbox
  // preview and commits a sized primitive on `end`. AP5 (#14): `targetId` is the
  // object whose outline was snapped to (null when not snapped), so the shell can
  // author a persistent anchor binding the created endpoint to it.
  onCreate: (
    phase: "start" | "move" | "end" | "cancel",
    world: { x: number; y: number },
    snapped: boolean,
    targetId: string | null
  ) => void;
  // W3-G9 (#3): a bare create-tool hover snap probe (no button down). `targetId` is
  // the object whose edge the cursor is over (null when not snapped); the shell
  // renders a PERSISTENT anchor ring from it before any drag. Optional so a host
  // that omits it loses nothing.
  onCreateHover?: (world: { x: number; y: number }, snapped: boolean, targetId: string | null) => void;
  // W2-08: eraser touch over a stroke. `id` is the hit object; `partial` is the
  // partial-erase modifier (default = whole-stroke delete, modifier = subpath
  // cut). The shell authors the delete / edit-geometry op from `world` + `id`.
  onErase: (id: string, world: { x: number; y: number }, partial: boolean) => void;
  // W2-03: hover affordance under the cursor (empty/body/resize-*/rotate). The
  // shell maps it to a CSS cursor.
  onAffordance: (affordance: HoverAffordance) => void;
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
      stroke: projectStroke(object.stroke),
      text: object.text ?? null,
      // W3-G9/#5: the D5 anchors must reach the core so `Bindings::build` inverts
      // them into Reproject edges and a moved target reprojects its followers LIVE
      // during the drag. Dropping them here silently leaves the bindings graph
      // anchor-free (the host bindings test builds scenes directly, so it can't
      // catch a missing wire projection).
      anchors: object.anchors ?? [],
      clip: object.clip ?? false
    }))
  };
}

/** Project an object's stroke into the renderer feed. The model stores stroke
 *  width and dash run lengths in QUANTIZED units (GEOMETRY_QUANTUM_PER_PX per px),
 *  but the renderer treats `RStroke.width`/`dash` as logical px (geometry coords
 *  are de-quantized inside the renderer), so convert width and each dash entry. */
function projectStroke(stroke: Stroke | undefined): Record<string, unknown> | null {
  if (!stroke) return null;
  return {
    ...stroke,
    width: stroke.width / GEOMETRY_QUANTUM_PER_PX,
    ...(stroke.dash ? { dash: stroke.dash.map((d) => d / GEOMETRY_QUANTUM_PER_PX) } : {})
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
   * renderer-core `RenderObjectScene`, then:
   *  - if a live `ShapeWebGpuRenderer` with `loadObjectScene` is present, uploads
   *    the object geometry to the GPU (the RAF `renderFrame` loop then draws it),
   *    and
   *  - always builds the CPU geometry through the crate's `buildObjectSceneGeometry`
   *    for the returned counts.
   *
   * Returns the geometry build (counts + raw), or null when no build entry is
   * available. The live GPU PASS only runs in a real browser (no WebGPU device in
   * CI).
   */
  loadObjectScene(scene: ObjectScene, selection: ObjectSelection, collectGeometry = false): ObjectGeometryBuild {
    this.lastObjectScene = scene;
    this.lastSelection = selection;
    const json = JSON.stringify(
      objectSceneToRenderObjectScene(scene, this.camera, selection, `object-scene-v${scene.sceneVersion}`)
    );
    this.uploadObjectSceneToRenderer(json);
    // The CPU geometry build is diagnostics-only (the live GPU upload above already
    // tessellates + uploads). The live feed re-runs every drag/freehand frame and
    // discards the result, so skip it unless a caller explicitly wants the counts.
    const build = collectGeometry ? this.rustStatus.buildObjectSceneGeometry : null;
    if (!build) return null;
    try {
      const raw = build(json);
      return summarizeObjectGeometry(raw);
    } catch (error) {
      this.callbacks.onStatus(errorMessage(error, "Object geometry build failed."));
      return null;
    }
  }

  /** W2-11 drag zero-rebake: revert the dragged object's GPU instance matrix to its
   *  canonical baked transform, dropping the live preview. The shell calls this on a
   *  commit-failure (defensive snap-back) before the canonical-scene rebake lands;
   *  the success path lets the next `loadObjectScene` rebake drop the stale matrix.
   *  No-op without a live renderer or on a wasm build predating the method. */
  clearObjectPreview(id: string): void {
    this.webGpuRenderer?.clearObjectPreview?.(id);
  }

  /** v3 §2b: revert an endpoint drag's live chord deform to the canonical baked
   *  geometry. The shell calls this when the release authored nothing (no-op
   *  release / failed commit); the success path lets the committed scene's re-feed
   *  land the deformed geometry canonically. No-op without a live renderer or on a
   *  wasm build predating the method. */
  clearObjectEndpointPreview(id: string): void {
    this.webGpuRenderer?.clearObjectEndpointPreview?.(id);
  }

  /** Upload the object scene to the live renderer when available (browser-only;
   *  a no-op without a WebGPU device). FC-05: this only UPLOADS the geometry; the
   *  RAF `renderFrame` loop is the sole frame driver and records the object pass
   *  itself, so calling `drawObjects` here would double-acquire the swapchain. */
  private uploadObjectSceneToRenderer(sceneJson: string): void {
    const renderer = this.webGpuRenderer;
    if (!renderer || typeof renderer.loadObjectScene !== "function") return;
    try {
      renderer.loadObjectScene(sceneJson);
      // v3 §2b: a re-feed rebuilds every baked geometry from the canonical scene,
      // wiping a live endpoint chord deform (the renderer resets its patch
      // bookkeeping on feed). Re-apply the in-flight drag's latest sample — e.g.
      // when the shell's snap ring rides the feed mid-drag.
      this.engine?.refreshEndpointPreview();
    } catch (error) {
      this.callbacks.onStatus(errorMessage(error, "Object scene upload failed."));
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

  /** W2-03/W2-07/W2-08: set the active pointer tool (Select/Draw/Create/Erase). */
  setTool(tool: ActiveTool): void {
    this.engine?.setTool(tool);
  }

  /** W2-03: mirror the Space key state so a Space-held drag pans. */
  setSpaceHeld(held: boolean): void {
    this.engine?.setSpaceHeld(held);
  }

  /** AP4 (#12c): drive RB1's renderer theme-bit (dark/light). No-op without a live
   *  renderer or on a wasm build predating the export; the shell feature-detects. */
  setObjectTheme(dark: boolean): void {
    this.webGpuRenderer?.setObjectTheme?.(dark);
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
      return;
    }

    // FC-08: object-path input results route to the shell callbacks.
    if (event.type === "object-select") {
      this.callbacks.onSelectObject(event.id, event.additive);
      return;
    }
    if (event.type === "object-transform-preview") {
      this.callbacks.onTransformPreview(event.id, event.matrix, event.kind);
      return;
    }
    if (event.type === "object-transform-commit") {
      this.callbacks.onTransformCommit(event.id, event.matrix, event.kind, event.detach);
      return;
    }
    // v3 §2b: open-class endpoint drag preview/commit route to the shell.
    if (event.type === "object-endpoint-preview") {
      this.callbacks.onEndpointPreview?.(event.id, event.nodeIndex, event.world, event.snapped, event.targetId);
      return;
    }
    if (event.type === "object-endpoint-commit") {
      this.callbacks.onEndpointCommit?.(event.id, event.nodeIndex, event.world, event.snapped, event.targetId);
      return;
    }
    if (event.type === "object-marquee") {
      this.callbacks.onMarquee(event.ids);
      return;
    }
    // RA2b/AP3: a double-click on an object routes to the shell's drill-in / text-edit.
    if (event.type === "object-double-click") {
      this.callbacks.onObjectDoubleClick?.({ id: event.id, hasChildren: event.hasChildren });
      return;
    }
    // FC-11: freehand pen capture phase routes to the shell's draw controller.
    if (event.type === "draw") {
      this.callbacks.onDraw(event.phase, event.world);
      return;
    }
    // W2-07: shape drag-create phase routes to the shell's create controller.
    if (event.type === "create") {
      this.callbacks.onCreate(event.phase, event.world, event.snapped, event.targetId);
      return;
    }
    // W3-G9 (#3): a bare create-tool hover snap probe routes to the shell's
    // persistent anchor-ring controller.
    if (event.type === "create-hover") {
      this.callbacks.onCreateHover?.(event.world, event.snapped, event.targetId);
      return;
    }
    // W2-08: eraser touch routes to the shell's erase controller.
    if (event.type === "erase") {
      this.callbacks.onErase(event.id, event.world, event.partial);
      return;
    }
    // W2-03: hover affordance routes to the shell's cursor.
    if (event.type === "affordance") {
      this.callbacks.onAffordance(event.affordance);
    }
  }

  /** FC-08: pure object pick (no mutation) at canvas-local screen coords, used by
   *  the shell's right-click context menu. */
  hitTestObjectAt(screenX: number, screenY: number): string | null {
    return this.engine?.objectHitTest({ x: screenX, y: screenY }) ?? null;
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

/** Summarize the opaque crate geometry build into counts for diagnostics. The
 *  wasm `buildObjectSceneGeometry` returns a flat summary
 *  `{ objects, fillVertices, fillTriangles, strokeVertices, draws }` whose counts
 *  are already numbers (see `ObjectGeometrySummary` in lib.rs), not arrays. */
function summarizeObjectGeometry(raw: unknown): ObjectGeometryBuild {
  const value = raw as { fillVertices?: unknown; strokeVertices?: unknown } | null;
  return {
    fillVertexCount: typeof value?.fillVertices === "number" ? value.fillVertices : 0,
    strokeInstanceCount: typeof value?.strokeVertices === "number" ? value.strokeVertices : 0,
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
