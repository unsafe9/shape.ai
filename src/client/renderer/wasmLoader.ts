import type { CameraState, DomOverlayRequest, RenderTransform3x3, ScenePatch, SceneSelection, WorldRect, WorldPoint } from "./scene";

export type RustCoreStatus = {
  available: boolean;
  backend: string;
  detail: string;
  probeWebGpu: RustWebGpuProbe | null;
  createWebGpuRenderer: RustCreateWebGpuRenderer | null;
  /**
   * OB-4 object render entry: builds CPU object fill/stroke geometry from a
   * `RenderObjectScene` JSON via the crate's `ObjectPipeline::build_scene_geometry`.
   * Present once the renderer wasm exports it. The live GPU object PASS (uploading
   * this geometry through the frame loop) is a deferred renderer-crate step — this
   * exercises the build path so the object scene round-trips through the renderer.
   */
  buildObjectSceneGeometry: ((sceneJson: string) => unknown) | null;
};

export type RustWebGpuFrameStats = {
  totalGroups: number;
  totalCards: number;
  totalEdges: number;
  visibleGroupCount: number;
  visibleCardCount: number;
  visibleEdgeCount: number;
  vertexCount: number;
  drawnVertexCount: number;
  drawRangeCount: number;
  textGlyphCount: number;
  fallbackTextGlyphCount: number;
  cjkTextGlyphCount: number;
  fontFallbackRunCount: number;
  missingTextGlyphCount: number;
  textAtlasOverflowGlyphCount: number;
  textMissingRasterGlyphCount: number;
  textAtlasGlyphCount: number;
  textRasterCacheHits: number;
  textRasterCacheMisses: number;
  textLayoutCacheHits: number;
  textLayoutCacheMisses: number;
  styleTokenCount: number;
  patchUpdateCount: number;
  dirtyRangeWriteCount: number;
  fullBufferRebuildCount: number;
  vertexTruncationCount: number;
  truncatedVertexCount: number;
  edgeCapacityGrowCount: number;
  edgeCompactionCount: number;
  edgeSlotCount: number;
  edgeSlotFreeCount: number;
  cardCapacityGrowCount: number;
  cardCompactionCount: number;
  cardSlotCount: number;
  cardSlotFreeCount: number;
  groupCapacityGrowCount: number;
  groupCompactionCount: number;
  groupSlotCount: number;
  groupSlotFreeCount: number;
  // FC-09: object draw-path diagnostics. Nullable so a wasm build (or test mock)
  // predating these fields still typechecks; read defensively as "no objects".
  objectCount?: number | null;
  objectFillIndexCount?: number | null;
  objectStrokeVertexCount?: number | null;
  objectDrawCount?: number | null;
  backend: string;
};

export type RustHitResult = {
  id: string;
  kind: string;
  groupId?: string | null;
  field?: string | null;
  port?: string | null;
  worldX: number;
  worldY: number;
  screenX: number;
  screenY: number;
};

export type RustCanvasInputEvent =
  | { kind: "pointer-down"; pointerId: number; screen: WorldPoint }
  | { kind: "pointer-move"; pointerId: number; screen: WorldPoint }
  | { kind: "pointer-up"; pointerId: number; screen: WorldPoint; edgeId?: string }
  | { kind: "pointer-cancel"; pointerId: number }
  | { kind: "wheel"; screen: WorldPoint; deltaY: number }
  | { kind: "double-click"; screen: WorldPoint }
  | { kind: "fit-scene" }
  | { kind: "focus-bounds"; bounds: WorldRect; screen?: WorldPoint; zoom?: number; padding?: WorldPoint; minZoom?: number; maxZoom?: number }
  | { kind: "set-camera"; camera: CameraState }
  // CC1.4: active tool toggle (Select/Hand). tool is camelCase ActiveTool.
  | { kind: "set-tool"; tool: "select" | "hand" }
  // Transient multi-select highlight set (marquee / shift-click). Empty clears it;
  // the persisted single-anchor selection is untouched.
  | { kind: "set-multi-select"; ids: string[] }
  // CC4.1: right-click pick — populates result.hit without mutating selection.
  | { kind: "context-pick"; screen: WorldPoint };

// CC2.3: a marquee result is non-null only on the pointer-up that ends a
// marquee drag. ids = node ids first, then group ids, whose world AABB
// intersects the final rect; the shell merges them into multiSelectIds.
export type RustMarqueeResult = {
  rect: WorldRect;
  ids: string[];
};

// W2-04: a cumulative object transform delta. `matrix` is a ROW-MAJOR world-space
// delta to PRE-MULTIPLY onto the object's existing transform (newWorld = matrix *
// objTransform); it is cumulative from the fixed pointer-down anchor (not per-move).
// `kind` names the gesture. The shell composes it non-destructively for preview and
// authors one undoable op on pointer-up (W2-05); W2-11 pushes the composed matrix as
// the per-object instance matrix.
export type RustObjectTransformDelta = {
  id: string;
  matrix: RenderTransform3x3;
  kind: "translate" | "resize" | "rotate";
};

export type RustInputBatchResult = {
  camera: CameraState;
  hit: RustHitResult | null;
  selection: SceneSelection;
  patches: ScenePatch[];
  overlay: DomOverlayRequest | null;
  // CC2.3: optional so a wasm build (or test mock) predating the field still
  // typechecks; the engine reads it defensively as "no marquee".
  marquee?: RustMarqueeResult | null;
  // FC-07: object-path input results, optional/non-null only when an object scene
  // is loaded and the matching event occurred.
  objectSelection?: string | null;
  objectTransformDelta?: RustObjectTransformDelta | null;
  objectMarqueeIds?: string[] | null;
  // RA2b: a double-click that landed on an object. null (or absent) when the
  // double-click missed every object; `hasChildren` lets the shell drill into a
  // container vs. edit a leaf. Optional so a wasm build predating it still typechecks.
  objectDoubleClick?: { id: string; hasChildren: boolean } | null;
  // W2-02: hover affordance the shell maps to a cursor. Optional so a wasm build /
  // test mock predating the field still typechecks; defaults to "empty".
  hoverAffordance?: HoverAffordance;
};

// W2-02: stable hover-affordance wire strings (mirror the Rust enum). The shell
// maps each to a cursor (W2-03).
export type HoverAffordance =
  | "empty"
  | "body"
  | "resize-nw"
  | "resize-n"
  | "resize-ne"
  | "resize-e"
  | "resize-se"
  | "resize-s"
  | "resize-sw"
  | "resize-w"
  | "rotate";

export type RustDebugSnapshot = {
  camera: CameraState;
  selection: SceneSelection;
  selectionWorldRect: WorldRect | null;
  selectionScreenRect: WorldRect | null;
  lastHit: RustHitResult | null;
  totalGroups: number;
  totalCards: number;
  totalEdges: number;
  patchUpdateCount: number;
  dirtyRangeWriteCount: number;
  fullBufferRebuildCount: number;
};

export type RustWebGpuProbeReport = {
  supported: boolean;
  adapterFound: boolean;
  deviceCreated: boolean;
  surfaceConfigured: boolean;
  renderPassSubmitted: boolean;
  presented: boolean;
  backend: string;
  enabledBackends: string;
  format: string | null;
  presentMode: string | null;
  width: number;
  height: number;
  detail: string;
};

export type RustWebGpuProbe = (
  canvas: HTMLCanvasElement,
  width: number,
  height: number,
  devicePixelRatio: number
) => Promise<RustWebGpuProbeReport>;

export type RustCreateWebGpuRenderer = (
  canvas: HTMLCanvasElement,
  width: number,
  height: number,
  devicePixelRatio: number
) => Promise<RustWebGpuRenderer>;

export type RustWebGpuRenderer = {
  resize(width: number, height: number, devicePixelRatio: number): void;
  loadScene(sceneJson: string): void;
  applyPatchBatch(patchesJson: string): void;
  renderFrame(): RustWebGpuFrameStats;
  // OB-4 object draw path. `loadObjectScene` builds + uploads the object geometry
  // for a `RenderObjectScene` JSON and returns `{ objects, fillIndices,
  // strokeVertices }`; `drawObjects` records the live object GPU pass. Optional so
  // a wasm build (or test mock) predating these methods still satisfies the type;
  // the host feature-detects before calling.
  loadObjectScene?(sceneJson: string): { objects: number; fillIndices: number; strokeVertices: number };
  drawObjects?(): void;
  // W2-11: drag zero-rebake. `setObjectPreviewTransform` pushes ONLY the dragged
  // object's instance model matrix (a row-major [[f64;3];3] cumulative world DELTA,
  // JSON) to the GPU with no re-tessellation; `clearObjectPreview` reverts it to the
  // canonical baked transform. Optional so a wasm build predating these stays valid;
  // the host feature-detects before calling.
  setObjectPreviewTransform?(id: string, matrixJson: string): void;
  clearObjectPreview?(id: string): void;
  inputBatch(eventsJson: string): RustInputBatchResult;
  overlayRequest(cardId: string, field: string): DomOverlayRequest | null;
  debugSnapshot(): RustDebugSnapshot;
  // CC1.4/CC4.1: optional so a wasm build (or test mock) predating these methods
  // still satisfies the type; the engine feature-detects before calling.
  // setTool sets the active pointer tool ("select" | "hand"; unknown ignored);
  // hitTest is a pure pick (no mutation) for the right-click context menu.
  setTool?(tool: string): void;
  hitTest?(screenX: number, screenY: number): RustHitResult | null;
  // FC-08: pure object pick for the right-click context menu — returns the id of
  // the top-most object under the screen point (no mutation). Optional so a wasm
  // build predating it is treated as "no object" by the engine.
  hitTestObject?(screenX: number, screenY: number): string | null;
  // RA3/EN1: pure swept erase pick — every object crossed by the eraser between two
  // consecutive SCREEN samples (prev -> curr), top-down order, so a fast drag erases
  // the whole swept path. No selection/camera/drag mutation. Optional so a wasm build
  // predating it falls back to the single-sample `hitTestObject` in the engine.
  sweptEraseAt?(prevX: number, prevY: number, currX: number, currY: number): string[];
  // W2-06: nearest point on any object outline to a WORLD query point, for shape
  // drag-create anchor snapping (W2-07 consumes this). `tolPx` is a screen-pixel
  // radius converted to world via `zoom` internally; the inputs are WORLD coords
  // (not screen). `excludeIdsJson` is a JSON array of region ids to skip (W3-G6 #6:
  // the transient create-preview / snap-indicator, which ride the feed and would
  // otherwise self-snap under the cursor). Optional + feature-detected, same pattern
  // as hitTestObject.
  nearestOutlinePoint?(
    worldX: number,
    worldY: number,
    tolPx: number,
    zoom: number,
    excludeIdsJson: string
  ): { snapped: boolean; x: number; y: number; targetId: string | null };
  // Replace the transient multi-select highlight set (JSON array of ids). Optional
  // so a wasm build predating it is treated as a no-op by the engine.
  setMultiSelect?(idsJson: string): void;
  // AP4/RB1 theme-bit: flip the renderer to dark/light. Re-resolves token-backed
  // instance colors and writes ONLY the color slot (zero rebake). Optional so a
  // wasm build predating the export stays valid; the shell feature-detects.
  setObjectTheme?(dark: boolean): void;
};

type RustWebGpuRendererClass = {
  create: RustCreateWebGpuRenderer;
};

type RustCoreModule = {
  default?: () => Promise<void> | void;
  renderer_backend?: () => string;
  probeWebGpu?: RustWebGpuProbe;
  ShapeWebGpuRenderer?: RustWebGpuRendererClass;
  buildObjectSceneGeometry?: (sceneJson: string) => unknown;
};

export async function loadRustCore(): Promise<RustCoreStatus> {
  const modulePath = "./wasm/shape_canvas_core.js";
  try {
    const wasmModule = (await import(/* @vite-ignore */ modulePath)) as RustCoreModule;
    if (typeof wasmModule.default === "function") await wasmModule.default();
    if (typeof wasmModule.ShapeWebGpuRenderer?.create !== "function") {
      throw new Error("ShapeWebGpuRenderer.create export is missing");
    }
    const backend =
      typeof wasmModule.renderer_backend === "function"
        ? String(wasmModule.renderer_backend())
        : "rust-wasm-loaded";
    return {
      available: true,
      backend,
      detail: "Rust/WASM package loaded with WebGPU renderer export.",
      probeWebGpu: typeof wasmModule.probeWebGpu === "function" ? wasmModule.probeWebGpu : null,
      createWebGpuRenderer: wasmModule.ShapeWebGpuRenderer.create.bind(wasmModule.ShapeWebGpuRenderer),
      buildObjectSceneGeometry:
        typeof wasmModule.buildObjectSceneGeometry === "function" ? wasmModule.buildObjectSceneGeometry : null
    };
  } catch (error) {
    return {
      available: false,
      backend: "webgpu-wasm-unavailable",
      detail: error instanceof Error ? error.message : "Rust/WASM package has not been built yet.",
      probeWebGpu: null,
      createWebGpuRenderer: null,
      buildObjectSceneGeometry: null
    };
  }
}
