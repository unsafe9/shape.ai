import type { CameraState, RenderTransform3x3, WorldRect, WorldPoint } from "../renderer/scene";

export type RustCoreStatus = {
  available: boolean;
  backend: string;
  detail: string;
  probeWebGpu: RustWebGpuProbe | null;
  createWebGpuRenderer: RustCreateWebGpuRenderer | null;
  buildObjectSceneGeometry: ((sceneJson: string) => unknown) | null;
  // Project a canonical ObjectScene (JSON) to the renderer-core RenderObjectScene
  // (JSON) through the Rust core — the single owner of the identity default /
  // selection flatten / stroke de-quant. Returns the projected JSON string.
  projectObjectScene:
    | ((sceneJson: string, cameraJson: string, selectionJson: string, sceneId: string) => string)
    | null;
  // Build the screen-space P1 UI scene JSON (ui-core-authored, identity camera) for the viewport,
  // fed to the renderer's `loadUiScene`. Null when the wasm build predates the export.
  buildP1UiScene: ((viewportW: number, viewportH: number) => string) | null;
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
  // Nullable so a wasm build (or test mock) predating these fields still typechecks.
  objectCount?: number | null;
  objectFillIndexCount?: number | null;
  objectStrokeVertexCount?: number | null;
  objectDrawCount?: number | null;
  objectPatchCount?: number | null;
  objectRebuildCount?: number | null;
  uiPatchCount?: number | null;
  uiRebuildCount?: number | null;
  backend: string;
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
  | { kind: "set-tool"; tool: "select" | "hand" }
  // Coarse-rotate modifier (e.g. Shift held); while active a rotate-handle drag snaps its swept delta in-core.
  | { kind: "set-coarse-rotate"; active: boolean }
  // Active drill-in container scope; the core scopes the next pointer-down pick to its direct children (null clears).
  | { kind: "set-active-container"; id: string | null }
  // Empty clears the transient multi-select highlight; the persisted single-anchor selection is untouched.
  | { kind: "set-multi-select"; ids: string[] }
  // Right-click pick — populates result.hit without mutating selection.
  | { kind: "context-pick"; screen: WorldPoint };


// Cumulative object transform delta. `matrix` is a ROW-MAJOR world-space delta to
// PRE-MULTIPLY onto the object's existing transform (newWorld = matrix * objTransform);
// cumulative from the fixed pointer-down anchor, not per-move.
export type RustObjectTransformDelta = {
  id: string;
  matrix: RenderTransform3x3;
  kind: "translate" | "resize" | "rotate";
};

// Live endpoint-drag sample for an OPEN-CLASS selection. `nodeIndex` is the dragged
// endpoint in geometry PAIR space (0 | last, the space anchors and scene-core
// `endpoint_release_ops` address); (x, y) is the cumulative pointer WORLD position.
export type RustObjectEndpointDelta = {
  id: string;
  nodeIndex: number;
  x: number;
  y: number;
};

export type RustInputBatchResult = {
  camera: CameraState;
  // Object-path results: non-null only when an object scene is loaded and the matching event occurred.
  objectSelection?: string | null;
  objectTransformDelta?: RustObjectTransformDelta | null;
  objectEndpointDelta?: RustObjectEndpointDelta | null;
  objectMarqueeIds?: string[] | null;
  // Double-click that landed on an object; null/absent on a miss. `hasChildren` lets the shell drill in vs edit a leaf.
  objectDoubleClick?: { id: string; hasChildren: boolean } | null;
  // Hover affordance the shell maps to a cursor; defaults to "empty".
  hoverAffordance?: HoverAffordance;
};

// A ui-core dispatch action, mirroring the Rust `shape_ui_core::Action` (serialized
// as a `type`-tagged camelCase union). The shell treats these as OPAQUE forward
// payloads — it emits them as EngineEvents and computes NOTHING from them.
export type UiAction =
  | { type: "pressed"; id: string }
  | { type: "toggleChanged"; id: string; on: boolean }
  | { type: "sliderChanged"; id: string; value: number }
  | { type: "segmentChanged"; id: string; index: number }
  | { type: "textChanged"; id: string; text: string }
  | { type: "focus"; id: string };

// A ui-core edit request (a focused TextInput's IME mount target, NEXT slice).
// `rect` is [x, y, w, h] in screen px. Opaque to the shell.
export type UiEditRequest = {
  id: string;
  rect: [number, number, number, number];
  value: string;
  sizePx: number;
};

// The typed request a fired built-in-UI widget resolved to through the set
// `UiModel` (`shape_ui::resolve`), mirroring the Rust `UiIntentDto` (a `type`-tagged
// camelCase union). The shell forwards each to its EXISTING op-authoring handler; it
// invents no op and re-derives no intent from the raw actions.
export type UiIntent =
  | { type: "command"; id: string }
  | { type: "selectColor"; hex: string }
  | { type: "selectPenWidth"; px: number }
  | { type: "applyTemplate"; id: string }
  | { type: "selectCanvas"; id: string }
  | { type: "newCanvas" }
  | { type: "deleteCanvas"; id: string }
  | { type: "inspectorEdit"; controlId: string; opKind: string; field: string | null; unitScale: number; value: unknown }
  | { type: "inspectorAction"; controlId: string }
  | { type: "dismiss" };

// The wasm dispatch return. `sceneChanged` is the runtime's dirty bit AFTER the
// renderer re-fed the UI scene; the shell only knows the RAF must redraw. `intents`
// are the typed requests the fired actions resolved to through the set model (empty
// on the demo path / when no action authored); the shell forwards each opaquely.
// `intents` is optional + feature-detected: a wasm build predating the resolver omits
// it, and the engine reads `?? []` so an absent list is a clean no-route.
export type UiDispatchResult = {
  consumed: boolean;
  sceneChanged: boolean;
  actions: UiAction[];
  edit: UiEditRequest | null;
  intents?: UiIntent[];
};

// A neutral key the shell forwards (the core decides; no key-literal BEHAVIOR branch). The raw
// modifier flags let the core tell a shortcut chord from a bare key, so a focused field does not
// swallow Cmd+Z/Ctrl+C/etc.
export type UiKeyInput = { key: string; text: string | null; ctrl: boolean; meta: boolean; alt: boolean };

// Stable hover-affordance wire strings (mirror the Rust enum).
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
  renderFrame(): RustWebGpuFrameStats;
  // Object draw path: `loadObjectScene` builds + uploads geometry for a `RenderObjectScene`
  // JSON; `drawObjects` records the live GPU pass. Methods are optional + feature-detected
  // here and below so a wasm build (or test mock) predating an export still satisfies the type.
  loadObjectScene?(sceneJson: string): { objects: number; fillIndices: number; strokeVertices: number };
  drawObjects?(): void;
  // Drag zero-rebake: `setObjectPreviewTransform` pushes ONLY the dragged object's instance
  // matrix (row-major [[f64;3];3] cumulative world DELTA, JSON) with no re-tessellation;
  // `clearObjectPreview` reverts to the canonical baked transform.
  setObjectPreviewTransform?(id: string, matrixJson: string): void;
  clearObjectPreview?(id: string): void;
  // Endpoint drag: chord-deforms ONE open-class object so its dragged endpoint (`nodeIndex`,
  // pair space 0 | last) lands on the live pointer WORLD position (scene-core `deform_open_path`
  // math, same as the release commit). `clearObjectEndpointPreview` restores the baked geometry.
  setObjectEndpointPreview?(id: string, nodeIndex: number, worldX: number, worldY: number): void;
  clearObjectEndpointPreview?(id: string): void;
  inputBatch(eventsJson: string): RustInputBatchResult;
  // Sets the active pointer tool ("select" | "hand"; unknown ignored).
  setTool?(tool: string): void;
  // Sets the coarse-rotate modifier; while active a rotate-handle drag snaps its swept delta in-core.
  setCoarseRotate?(active: boolean): void;
  // Pure object pick for the right-click context menu — top-most object id under the screen point, no mutation.
  hitTestObject?(screenX: number, screenY: number): string | null;
  // UI analog of loadObjectScene: feed the screen-space UI scene (ui-core-authored, identity camera) to a
  // SECOND renderer drawn above the world pass. Returns the built draw counts.
  loadUiScene?(sceneJson: string): { objects: number; fillIndices: number; strokeVertices: number };
  // UI analog of hitTestObject: top-most UI widget id under the SCREEN point (identity-camera screen-space),
  // no mutation. The shell forwards raw canvas-local px and consumes the core-returned id.
  hitUi?(screenX: number, screenY: number): string | null;
  // Pure swept erase pick — every object crossed between two consecutive SCREEN samples (prev -> curr),
  // top-down, so a fast drag erases the whole swept path; falls back to single-sample `hitTestObject` when absent.
  sweptEraseAt?(prevX: number, prevY: number, currX: number, currY: number): string[];
  // Nearest point on any object outline to a WORLD query point, for drag-create anchor snapping.
  // `tolPx` is a screen-pixel radius converted to world via `zoom` internally; inputs are WORLD coords.
  // `excludeIdsJson` is a JSON array of region ids to skip (the transient create-preview/snap-indicator,
  // which ride the feed and would otherwise self-snap under the cursor).
  nearestOutlinePoint?(
    worldX: number,
    worldY: number,
    tolPx: number,
    zoom: number,
    excludeIdsJson: string
  ): { snapped: boolean; x: number; y: number; targetId: string | null };
  // Project a WORLD point to SCREEN through the LIVE core camera (inverse of `screenToWorld`); the shell
  // calls this instead of recomputing the transform from a mirrored CameraState.
  worldToScreen?(worldX: number, worldY: number): { x: number; y: number };
  // Un-project a SCREEN point to WORLD through the LIVE core camera (inverse of `worldToScreen`).
  screenToWorld?(screenX: number, screenY: number): { x: number; y: number };
  // Replace the transient multi-select highlight set (JSON array of ids).
  setMultiSelect?(idsJson: string): void;
  // Theme-bit: flip the renderer dark/light. Re-resolves token-backed instance colors and writes ONLY the color slot (zero rebake).
  setObjectTheme?(dark: boolean): void;
  // Active drill-in container scope. A forwarded token (like setCoarseRotate): the core scopes the next
  // pointer-down pick to this container's direct children; `null` clears the scope. The shell decides nothing.
  setActiveContainer?(id: string | null): void;
  // Seed the renderer-owned ui-core runtime from the demo widget tree at the viewport + theme, then feed
  // its first render. After this the runtime OWNS the UI scene (uiPointer/uiKey re-feed it). One-time seed.
  initUiRuntime?(viewportW: number, viewportH: number, themeDark: boolean): void;
  // Feed the built-in UI model (toolbar/inspector/settings/context-menu/presence/status) from the shell.
  // `modelJson` is the UiModelInput shape (camelCase): theme/viewport/tool/selection/inspector view + the
  // shell-owned UI state. The core composes build_root from it, re-trees the runtime (preserving interaction
  // caches), and resolves fired actions into typed intents. A model-only change rides the partial-patch path.
  setUiModel?(modelJson: string): void;
  // Drive the UI runtime with a pointer phase ("down"|"move"|"up"|"cancel") at SCREEN px. The core decides
  // everything (hit, slider value, actuation); on sceneChanged the renderer has re-fed the UI scene + regions.
  uiPointer?(phase: string, screenX: number, screenY: number): UiDispatchResult;
  // Forward a neutral key (KeyInput JSON) to the focused UI widget; the core decides whether it owns it.
  uiKey?(keyJson: string): UiDispatchResult;
  // Commit ONE finished string from the shell's IME surface into the focused UI field (the core blurs it
  // and emits the final TextChanged). Correct even when CJK composition deleted/replaced in place.
  uiCommitText?(value: string): UiDispatchResult;
  // True when a ui-core widget owns text focus (the window arbiter ORs this into its `typing` predicate).
  uiHasFocus?(): boolean;
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
  projectObjectScene?: (
    sceneJson: string,
    cameraJson: string,
    selectionJson: string,
    sceneId: string
  ) => string;
  // Build the P1 UI scene JSON (ui-core-authored, screen-space px) for the current viewport, fed to loadUiScene.
  buildP1UiScene?: (viewportW: number, viewportH: number) => string;
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
        typeof wasmModule.buildObjectSceneGeometry === "function" ? wasmModule.buildObjectSceneGeometry : null,
      projectObjectScene:
        typeof wasmModule.projectObjectScene === "function" ? wasmModule.projectObjectScene : null,
      buildP1UiScene:
        typeof wasmModule.buildP1UiScene === "function" ? wasmModule.buildP1UiScene : null
    };
  } catch (error) {
    return {
      available: false,
      backend: "webgpu-wasm-unavailable",
      detail: error instanceof Error ? error.message : "Rust/WASM package has not been built yet.",
      probeWebGpu: null,
      createWebGpuRenderer: null,
      buildObjectSceneGeometry: null,
      projectObjectScene: null,
      buildP1UiScene: null
    };
  }
}
