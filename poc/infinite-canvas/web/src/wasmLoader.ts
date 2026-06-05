export type RustCoreStatus = {
  available: boolean;
  backend: string;
  detail: string;
  core: RustCanvasCore | null;
  probeWebGpu: RustWebGpuProbe | null;
  createWebGpuRenderer: RustCreateWebGpuRenderer | null;
};

export type RustFrameStats = {
  totalGroups: number;
  totalCards: number;
  totalEdges: number;
  hitTestableCards: number;
  backend: string;
};

export type RustWebGpuFrameStats = {
  totalGroups: number;
  totalCards: number;
  totalEdges: number;
  vertexCount: number;
  textGlyphCount: number;
  fallbackTextGlyphCount: number;
  cjkTextGlyphCount: number;
  styleTokenCount: number;
  patchUpdateCount: number;
  dirtyRangeWriteCount: number;
  fullBufferRebuildCount: number;
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

export type RustCanvasCore = {
  mount(canvas: HTMLCanvasElement): void;
  resize(width: number, height: number, devicePixelRatio: number): void;
  loadScene(sceneJson: string): void;
  setCamera(x: number, y: number, zoom: number): void;
  renderFrame(): RustFrameStats;
  hitTest(screenX: number, screenY: number): RustHitResult | null;
};

export type RustWebGpuRenderer = {
  resize(width: number, height: number, devicePixelRatio: number): void;
  loadScene(sceneJson: string): void;
  applyPatch(patchJson: string): void;
  setCamera(x: number, y: number, zoom: number): void;
  renderFrame(): RustWebGpuFrameStats;
  hitTest(screenX: number, screenY: number): RustHitResult | null;
};

type RustWebGpuRendererClass = {
  create: RustCreateWebGpuRenderer;
};

type RustCoreModule = {
  default?: () => Promise<void> | void;
  renderer_backend?: () => string;
  probeWebGpu?: RustWebGpuProbe;
  ShapeCanvasCore?: new () => RustCanvasCore;
  ShapeWebGpuRenderer?: RustWebGpuRendererClass;
};

export async function loadRustCore(): Promise<RustCoreStatus> {
  const modulePath = "./wasm/shape_canvas_core.js";
  try {
    const wasmModule = (await import(/* @vite-ignore */ modulePath)) as RustCoreModule;
    if (typeof wasmModule.default === "function") await wasmModule.default();
    if (!wasmModule.ShapeCanvasCore) throw new Error("ShapeCanvasCore export is missing");
    const backend =
      typeof wasmModule.renderer_backend === "function"
        ? String(wasmModule.renderer_backend())
        : "rust-wasm-loaded";
    return {
      available: true,
      backend,
      detail: "Rust/WASM package loaded and ShapeCanvasCore instantiated.",
      core: new wasmModule.ShapeCanvasCore(),
      probeWebGpu: typeof wasmModule.probeWebGpu === "function" ? wasmModule.probeWebGpu : null,
      createWebGpuRenderer:
        typeof wasmModule.ShapeWebGpuRenderer?.create === "function"
          ? wasmModule.ShapeWebGpuRenderer.create.bind(wasmModule.ShapeWebGpuRenderer)
          : null
    };
  } catch (error) {
    return {
      available: false,
      backend: "typescript-canvas2d-poc",
      detail: error instanceof Error ? error.message : "Rust/WASM package has not been built yet.",
      core: null,
      probeWebGpu: null,
      createWebGpuRenderer: null
    };
  }
}
