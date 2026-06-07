import type { CameraState, DomOverlayRequest, ScenePatch, SceneSelection, WorldRect, WorldPoint } from "./scene";

export type RustCoreStatus = {
  available: boolean;
  backend: string;
  detail: string;
  probeWebGpu: RustWebGpuProbe | null;
  createWebGpuRenderer: RustCreateWebGpuRenderer | null;
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

export type RustInputBatchResult = {
  camera: CameraState;
  hit: RustHitResult | null;
  selection: SceneSelection;
  patches: ScenePatch[];
  overlay: DomOverlayRequest | null;
  // CC2.3: optional so a wasm build (or test mock) predating the field still
  // typechecks; the engine reads it defensively as "no marquee".
  marquee?: RustMarqueeResult | null;
};

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
  inputBatch(eventsJson: string): RustInputBatchResult;
  overlayRequest(cardId: string, field: string): DomOverlayRequest | null;
  debugSnapshot(): RustDebugSnapshot;
  // CC1.4/CC4.1: optional so a wasm build (or test mock) predating these methods
  // still satisfies the type; the engine feature-detects before calling.
  // setTool sets the active pointer tool ("select" | "hand"; unknown ignored);
  // hitTest is a pure pick (no mutation) for the right-click context menu.
  setTool?(tool: string): void;
  hitTest?(screenX: number, screenY: number): RustHitResult | null;
  // Replace the transient multi-select highlight set (JSON array of ids). Optional
  // so a wasm build predating it is treated as a no-op by the engine.
  setMultiSelect?(idsJson: string): void;
};

type RustWebGpuRendererClass = {
  create: RustCreateWebGpuRenderer;
};

type RustCoreModule = {
  default?: () => Promise<void> | void;
  renderer_backend?: () => string;
  probeWebGpu?: RustWebGpuProbe;
  ShapeWebGpuRenderer?: RustWebGpuRendererClass;
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
      createWebGpuRenderer: wasmModule.ShapeWebGpuRenderer.create.bind(wasmModule.ShapeWebGpuRenderer)
    };
  } catch (error) {
    return {
      available: false,
      backend: "webgpu-wasm-unavailable",
      detail: error instanceof Error ? error.message : "Rust/WASM package has not been built yet.",
      probeWebGpu: null,
      createWebGpuRenderer: null
    };
  }
}
