// Object-native Svelte↔renderer host. Owns the renderer lifecycle (load Rust core → create the
// visible WebGPU renderer → construct/observe/start the input+camera engine) and bridges the
// canonical `ObjectScene` to the renderer's object pipeline.
//
// The live object GPU pass only runs in a real browser (no WebGPU device in CI); in CI the geometry
// is built through the CPU `buildObjectSceneGeometry` entry, with `ShapeCanvasEngine` keeping the loop alive.

import type { CameraState } from "../shared/geometry";
import type { ObjectScene, ObjectSelection } from "../shared/object";
import { ShapeCanvasEngine, type ActiveTool, type EngineEvent, type FocusBoundsOptions, type TransformKind } from "../renderer/engine";
import type { FrameStats, RenderTransform3x3, WorldRect } from "../renderer/scene";
import { loadRustCore, type HoverAffordance, type RustCoreStatus, type RustWebGpuRenderer, type UiEditRequest, type UiIntent, type UiKeyInput } from "../bridge/wasmLoader";

export type RendererStats = FrameStats;

export type RendererHealth = {
  state: "ready" | "webgpu-unavailable" | "wasm-unavailable";
  detail: string;
  rustAvailable: boolean;
  rustBackend: string;
  webGpuRendererAvailable: boolean;
};

// The opaque crate geometry plus counts for diagnostics; `null` when no build entry is available.
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
  // Object-path input results. `onSelectObject` fires on the pick pointer-down (`additive` = shift/meta
  // held, so the shell toggles the multi-select set instead of replacing it); `onTransformPreview` on
  // each drag move (cumulative world-space delta + gesture kind, non-destructive); `onTransformCommit`
  // once on pointer-up when the drag moved; `onMarquee` on an empty-start drag's pointer-up.
  onSelectObject: (id: string, additive: boolean) => void;
  onTransformPreview: (id: string, matrix: RenderTransform3x3, kind: TransformKind) => void;
  // `detach` is the Alt-held-at-release bit — the shell branches an anchored open-class body drag
  // into a whole translate plus an anchor-clearing set-anchor (the class/anchor judgment is the core's).
  onTransformCommit: (id: string, matrix: RenderTransform3x3, kind: TransformKind, detach: boolean) => void;
  // Open-class endpoint drag. `onEndpointPreview` rides each move (chord deform already live on the GPU;
  // payload carries the release-snap probe for the anchor ring); `onEndpointCommit` rides pointer-up —
  // the shell authors `endpointReleaseOps` from it.
  onEndpointPreview?: (id: string, nodeIndex: number, world: { x: number; y: number }, snapped: boolean, targetId: string | null) => void;
  onEndpointCommit?: (id: string, nodeIndex: number, world: { x: number; y: number }, snapped: boolean, targetId: string | null) => void;
  onMarquee: (ids: string[]) => void;
  // A double-click landed on an object; the shell drills into a container (hasChildren) or edits a leaf.
  onObjectDoubleClick?: (payload: { id: string; hasChildren: boolean }) => void;
  // No engine event routes here — the right-click context pick runs synchronously via `hitTestObjectAt`.
  onContextPick?: (id: string | null) => void;
  // Freehand pen capture phases (world px). The shell accumulates points across start/move and commits
  // the RECOGNIZED stroke on `end`. `snap` is the outline snap probe under the cursor (null off any
  // edge / Alt held) so the shell can seed/author endpoint anchors for an open result.
  onDraw: (
    phase: "start" | "move" | "end" | "cancel",
    world: { x: number; y: number },
    snap: { at: { x: number; y: number }; targetId: string } | null
  ) => void;
  // Shape drag-create phases. `world` is the dragged corner (already snapped to the nearest outline
  // anchor when `snapped`); the shell rubber-bands a bbox and commits a sized primitive on `end`.
  // `targetId` is the snapped object (null when not snapped) so the shell can bind the created endpoint to it.
  onCreate: (
    phase: "start" | "move" | "end" | "cancel",
    world: { x: number; y: number },
    snapped: boolean,
    targetId: string | null
  ) => void;
  // A bare create-tool hover snap probe (no button down); the shell renders a PERSISTENT anchor ring before any drag.
  onCreateHover?: (world: { x: number; y: number }, snapped: boolean, targetId: string | null) => void;
  // Eraser touch over a stroke. `partial` = the modifier (default whole-stroke delete, modifier = subpath cut).
  onErase: (id: string, world: { x: number; y: number }, partial: boolean) => void;
  // Hover affordance under the cursor (empty/body/resize-*/rotate); the shell maps it to a CSS cursor.
  onAffordance: (affordance: HoverAffordance) => void;
  // A focused ui-core TextInput's IME mount request (the core decided "edit here"); the shell opens the
  // shared IME library. Opaque relay — the shell decides nothing from it.
  onUiEdit?: (edit: UiEditRequest) => void;
  // A typed intent a built-in-UI widget resolved to through the set model (the core's `shape_ui::resolve`).
  // The shell routes it to its existing op-authoring handler; this host only relays it (no decision here).
  onUiIntent?: (intent: UiIntent) => void;
};

const initialRustStatus: RustCoreStatus = {
  available: false,
  backend: "detecting",
  detail: "Checking generated Rust/WASM package.",
  probeWebGpu: null,
  createWebGpuRenderer: null,
  buildObjectSceneGeometry: null,
  projectObjectScene: null,
  buildP1UiScene: null
};

// The Svelte shell provides the three DOM nodes through mount(), reads camera/stats/health through
// callbacks, and pushes the object scene through `loadObjectScene`.
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
  // True once the ui-core runtime is seeded (initUiRuntime). The runtime then owns the UI scene, so
  // loadUiScene becomes a no-op (re-seeding would drop a dragged slider value / focused field).
  private uiRuntimeSeeded = false;
  // True once the shell has fed the real built-in-UI model via setUiModel. The runtime then owns the
  // REAL UI (toolbar/inspector/…), so loadUiScene must stop seeding the viewport-fixed demo panel —
  // a demo seed would replace the live tree and drop the model's interaction state.
  private uiModelFed = false;
  private engineWebGpuAvailable: boolean | null = null;
  private engineWebGpuDetail: string | null = null;
  private webGpuDetail = "Visible Rust/wgpu renderer has not been created.";
  private disposed = false;
  // Latest projected object scene, so a renderer recreation can re-feed it.
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
    this.loadUiScene();
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
    this.uiRuntimeSeeded = false;
    this.uiModelFed = false;
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
      // The screen-space UI scene is viewport-sized, so re-feed it on every resize. P1
      // rebuilds the static widget tree; a viewport-only PlanPatch update is a P2 follow-up.
      this.loadUiScene();
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

  // Push the canonical `ObjectScene` to the renderer: project it to `RenderObjectScene`, upload the
  // object geometry to the GPU when a live renderer is present, and (when `collectGeometry`) build the
  // CPU geometry for the returned counts. Returns the build (counts + raw), or null when no build entry exists.
  loadObjectScene(scene: ObjectScene, selection: ObjectSelection, collectGeometry = false): ObjectGeometryBuild {
    this.lastObjectScene = scene;
    this.lastSelection = selection;
    // The core owns the projection (identity default / selection flatten / stroke de-quant); the shell
    // passes scene + live camera/selection JSON straight through and feeds the returned wire to the renderer.
    const project = this.rustStatus.projectObjectScene;
    if (!project) return null;
    const json = project(
      JSON.stringify(scene),
      JSON.stringify(this.camera),
      JSON.stringify(selection),
      `object-scene-v${scene.sceneVersion}`
    );
    this.uploadObjectSceneToRenderer(json);
    // The CPU geometry build is diagnostics-only (the live GPU upload above already tessellates +
    // uploads), so skip it unless a caller explicitly wants the counts.
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

  // Drag zero-rebake: revert the dragged object's GPU instance matrix to its canonical baked
  // transform on a commit-failure (defensive snap-back); the success path lets the next
  // `loadObjectScene` rebake drop the stale matrix. No-op without a live renderer.
  clearObjectPreview(id: string): void {
    this.webGpuRenderer?.clearObjectPreview?.(id);
  }

  // Revert an endpoint drag's live chord deform to the canonical baked geometry when the release
  // authored nothing; the success path lets the committed scene's re-feed land it. No-op without a live renderer.
  clearObjectEndpointPreview(id: string): void {
    this.webGpuRenderer?.clearObjectEndpointPreview?.(id);
  }

  // Upload the object scene to the live renderer (browser-only; no-op without a WebGPU device). Only
  // UPLOADS the geometry — the RAF `renderFrame` loop is the sole frame driver and records the object
  // pass itself, so calling `drawObjects` here would double-acquire the swapchain.
  private uploadObjectSceneToRenderer(sceneJson: string): void {
    const renderer = this.webGpuRenderer;
    if (!renderer || typeof renderer.loadObjectScene !== "function") return;
    try {
      renderer.loadObjectScene(sceneJson);
      // A re-feed rebuilds every baked geometry, wiping a live endpoint chord deform, so re-apply the in-flight drag's latest sample.
      this.engine?.refreshEndpointPreview();
    } catch (error) {
      this.callbacks.onStatus(errorMessage(error, "Object scene upload failed."));
    }
  }

  // Seed / re-feed the screen-space UI scene for the current viewport. When the renderer exposes the
  // ui-core RUNTIME (`initUiRuntime`), seed it ONCE — the runtime then OWNS the scene and re-feeds itself
  // on dispatch, so subsequent calls (e.g. resize) do nothing here (the demo panel is viewport-fixed; the
  // app's theme effect drives the runtime's theme via setObjectTheme). When the runtime is unavailable,
  // fall back to the static P1 build + upload. Feature-detected end to end: a silent no-op on an old wasm.
  loadUiScene(): void {
    // Once the shell has fed the real built-in-UI model, the runtime owns the live UI — re-seeding the
    // demo panel here would replace it and drop the model's interaction state, so stop seeding.
    if (this.uiModelFed) return;
    const sizeSource = this.inputCanvas;
    if (!sizeSource) return;
    const rect = sizeSource.getBoundingClientRect();
    const w = rect.width || 1;
    const h = rect.height || 1;
    const renderer = this.webGpuRenderer;
    if (renderer && typeof renderer.initUiRuntime === "function") {
      // The runtime owns its scene once seeded; re-seeding would drop interaction state, so seed once.
      if (this.uiRuntimeSeeded) return;
      // Seed in light; the app applies the real theme via setObjectTheme once the host is wired
      // (handleHost), which re-resolves the runtime's text + re-feeds — so the initial theme lands
      // regardless of effect ordering. The core seeds the runtime from its persisted theme bit, so a
      // setObjectTheme that ran BEFORE this seed is adopted, not clobbered.
      renderer.initUiRuntime(w, h, false);
      this.uiRuntimeSeeded = true;
      return;
    }
    const build = this.rustStatus.buildP1UiScene;
    if (!build) return;
    const json = build(w, h);
    this.uploadUiSceneToRenderer(json);
  }

  // Upload the UI scene to the live renderer's second (identity-camera) renderer. Like the object
  // upload, the RAF `renderFrame` loop draws the UI pass — no `drawObjects`-style call here.
  private uploadUiSceneToRenderer(sceneJson: string): void {
    const renderer = this.webGpuRenderer;
    if (!renderer || typeof renderer.loadUiScene !== "function") return;
    try {
      renderer.loadUiScene(sceneJson);
    } catch (error) {
      this.callbacks.onStatus(errorMessage(error, "UI scene upload failed."));
    }
  }

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

  // Set the active pointer tool (Select/Draw/Create/Erase).
  setTool(tool: ActiveTool): void {
    this.engine?.setTool(tool);
  }

  // Mirror the Space key state so a Space-held drag pans.
  setSpaceHeld(held: boolean): void {
    this.engine?.setSpaceHeld(held);
  }

  // Wire the shell's IME-library blur hook so the engine commits an active text-edit before its
  // mousedown preventDefault swallows the blur. Pure passthrough to the engine; no logic here.
  setBlurActiveEditable(blur: () => void): void {
    this.engine?.setBlurActiveEditable(blur);
  }

  // Drive the renderer theme-bit (dark/light). No-op without a live renderer.
  setObjectTheme(dark: boolean): void {
    this.webGpuRenderer?.setObjectTheme?.(dark);
  }

  // Forward the active drill-in container scope token to the core (which scopes the next pointer-down
  // pick); `null` clears it. Pure passthrough — no selection logic here. No-op without a live renderer.
  setActiveContainer(id: string | null): void {
    this.webGpuRenderer?.setActiveContainer?.(id);
  }

  // Forward a neutral key to the focused UI widget (the core decides whether it owns it). Pure
  // passthrough to the engine wrapper — no key-literal branch here. Returns the dispatch verdict.
  uiKey(key: UiKeyInput): { consumed: boolean; sceneChanged: boolean } | null {
    return this.engine?.uiKey(key) ?? null;
  }

  // Commit the IME surface's single finished string into the focused UI field. Pure passthrough — the
  // core blurs the field and emits the final TextChanged; the shell authors nothing.
  uiCommitText(value: string): { consumed: boolean; sceneChanged: boolean } | null {
    return this.engine?.uiCommitText(value) ?? null;
  }

  // Feed the built-in UI model JSON to the core (composes build_root + re-trees the runtime, preserving
  // interaction caches). Returns true when fed, so the shell knows the runtime now owns the real UI and
  // the demo seed must stop. Pure passthrough — the shell authors no widget. No-op on an old renderer.
  setUiModel(modelJson: string): boolean {
    const fed = this.engine?.setUiModel(modelJson) ?? false;
    if (fed) this.uiModelFed = true;
    return fed;
  }

  // True when a ui-core widget owns text focus — the window arbiter ORs this into its `typing` predicate.
  uiHasFocus(): boolean {
    return this.engine?.uiHasFocus() ?? false;
  }

  getCamera(): CameraState {
    return this.camera;
  }

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
    if (event.type === "object-double-click") {
      this.callbacks.onObjectDoubleClick?.({ id: event.id, hasChildren: event.hasChildren });
      return;
    }
    if (event.type === "draw") {
      this.callbacks.onDraw(event.phase, event.world, event.snap);
      return;
    }
    if (event.type === "create") {
      this.callbacks.onCreate(event.phase, event.world, event.snapped, event.targetId);
      return;
    }
    if (event.type === "create-hover") {
      this.callbacks.onCreateHover?.(event.world, event.snapped, event.targetId);
      return;
    }
    if (event.type === "erase") {
      this.callbacks.onErase(event.id, event.world, event.partial);
      return;
    }
    if (event.type === "affordance") {
      this.callbacks.onAffordance(event.affordance);
      return;
    }
    if (event.type === "ui-action") {
      // The shell consumes NOTHING from a raw UI action — the typed `ui-intent` below carries the
      // core-resolved request the shell authors. The raw action is otherwise opaque bookkeeping the
      // engine already used for focus/IME, so it stops here (no status string, no decision).
      return;
    }
    if (event.type === "ui-intent") {
      // Pure relay of the core-resolved intent to the shell's existing op-authoring handler. The host
      // decides nothing; the App routes each intent kind to the handler it already has.
      this.callbacks.onUiIntent?.(event.intent);
      return;
    }
    if (event.type === "ui-edit") {
      // Pure relay of the core's "mount an edit here" request to the shell's shared IME library.
      this.callbacks.onUiEdit?.(event.edit);
    }
  }

  // Pure object pick (no mutation) at canvas-local screen coords, used by the right-click context menu.
  hitTestObjectAt(screenX: number, screenY: number): string | null {
    return this.engine?.objectHitTest({ x: screenX, y: screenY }) ?? null;
  }

  // Project a SCREEN point to WORLD through the LIVE core camera (no TS affine in the shell). Null when
  // the renderer is not yet live.
  projectScreenToWorld(screen: { x: number; y: number }): { x: number; y: number } | null {
    return this.engine?.projectScreenToWorld(screen) ?? null;
  }

  // Project a WORLD point to SCREEN through the LIVE core camera (inverse of the above). Null when the
  // renderer is not yet live.
  projectWorldToScreen(world: { x: number; y: number }): { x: number; y: number } | null {
    return this.engine?.projectWorldToScreen(world) ?? null;
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

// Summarize the opaque crate geometry build into counts for diagnostics. `buildObjectSceneGeometry`
// returns a flat summary `{ objects, fillVertices, fillTriangles, strokeVertices, draws }` of numbers, not arrays.
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
