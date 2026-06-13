import {
  type CameraState,
  type FrameStats,
  type RenderTransform3x3,
  type WorldPoint,
  type WorldRect
} from "./scene";
import type {
  HoverAffordance,
  RustCanvasInputEvent,
  RustInputBatchResult,
  RustWebGpuFrameStats,
  RustWebGpuRenderer
} from "../bridge/wasmLoader";
import {
  isAdditiveSelect,
  isCoarseRotate,
  isDetachDrag,
  isPanGesture,
  isPartialErase,
  isSnapBypass
} from "../controller/gestureBindings";
import { detectMac } from "../controller/shortcuts";

// The active pointer tool: "select" (picks/drags/marquees), "draw" (freehand capture), "create"
// (drag-to-create shapes), "erase" (whole/partial stroke eraser). Pan is not a tool — it rides
// Space-hold/middle-button/wheel (see `isPanIntent`).
export type ActiveTool = "select" | "draw" | "create" | "erase";

// A pointer-down is a pan gesture (not a pick/marquee) when Space is held OR the middle button is
// used; the binding is routed through the single source in gestureBindings (no magic literal here).
export function isPanIntent(intent: { spaceHeld: boolean; button: number }): boolean {
  return isPanGesture(intent);
}

// The gesture that produced a transform delta; mirrors the Rust ObjectTransformDelta.kind.
export type TransformKind = "translate" | "resize" | "rotate";

// Whether a drag-create phase should run the outline snap query: bypassed when the snap-bypass
// gesture (Alt held) is active, or on the terminal `cancel` phase (no preview to snap).
export function shouldQuerySnap(intent: { altHeld: boolean; phase: "start" | "move" | "end" | "cancel" }): boolean {
  return !isSnapBypass({ altKey: intent.altHeld }) && intent.phase !== "cancel";
}

// A create/draw-tool move with NO drag in progress is a HOVER probe (emits a persistent anchor-ring
// snap before any drag); a move during a drag is the regular rubber-band drag move.
export function createMoveEmission(intent: { dragActive: boolean }): "hover" | "drag" {
  return intent.dragActive ? "drag" : "hover";
}

export type EngineEvent =
  | { type: "stats"; stats: FrameStats }
  // Object-path input results. `object-select` rides the pick pointer-down (`additive` = shift/meta
  // held, toggling multi-select instead of replacing); `object-transform-preview` rides each drag move
  // (non-destructive; the matrix is the cumulative world-space delta PRE-multiplied onto the transform,
  // plus the gesture `kind`); `object-marquee` rides an empty-start drag's pointer-up.
  | { type: "object-select"; id: string; additive: boolean }
  | { type: "object-transform-preview"; id: string; matrix: RenderTransform3x3; kind: TransformKind }
  // `detach` is the Alt-held-at-release bit — the shell branches an anchored open-class body drag into
  // a whole translate plus an anchor-clearing set-anchor; the class/anchor judgment stays in the core.
  | { type: "object-transform-commit"; id: string; matrix: RenderTransform3x3; kind: TransformKind; detach: boolean }
  // Open-class endpoint drag. `object-endpoint-preview` rides each move (chord deform already live on the
  // GPU; payload carries the release-snap probe for the anchor ring); `object-endpoint-commit` rides
  // pointer-up (the shell authors endpoint_release_ops). `nodeIndex` is the dragged endpoint in PAIR
  // space (0 | last); `world` its position (snapped outline point when `snapped`); `targetId` = null unbinds.
  | { type: "object-endpoint-preview"; id: string; nodeIndex: number; world: WorldPoint; snapped: boolean; targetId: string | null }
  | { type: "object-endpoint-commit"; id: string; nodeIndex: number; world: WorldPoint; snapped: boolean; targetId: string | null }
  | { type: "object-marquee"; ids: string[] }
  // A double-click landed on an object; the shell drills into a container (hasChildren) or edits a leaf. Misses emit nothing.
  | { type: "object-double-click"; id: string; hasChildren: boolean }
  // Freehand pen capture phases; the shell accumulates world points and commits the stroke on `end`.
  // `snap` is the outline snap probe under the cursor (same query as create, honoring the Alt bypass) so
  // the shell can seed/author endpoint anchors. `world` stays the RAW pointer — mid-stroke samples are
  // never pulled onto an edge (that would distort the drawn silhouette).
  | { type: "draw"; phase: "start" | "move" | "end" | "cancel"; world: WorldPoint; snap: { at: WorldPoint; targetId: string } | null }
  // Drag-to-create shapes; the shell rubber-bands a bbox and commits a sized primitive on `end`. `world`
  // is already snapped to the nearest outline anchor within tolerance (`snapped` true) unless Alt was held;
  // `targetId` is the snapped object (null when not snapped) so the shell can bind the endpoint to it.
  | { type: "create"; phase: "start" | "move" | "end" | "cancel"; world: WorldPoint; snapped: boolean; targetId: string | null }
  // A HOVER snap probe emitted while the create tool is armed and no button is down. The shell renders a
  // PERSISTENT anchor ring before any drag; `snapped`/`targetId` mirror "create" (null clears the ring).
  | { type: "create-hover"; world: WorldPoint; snapped: boolean; targetId: string | null }
  // Eraser touch over a stroke. The id comes from the core's hit-test (hit-test stays in the core);
  // `partial` = the Alt modifier (default whole-stroke delete, modifier = partial subpath cut).
  | { type: "erase"; id: string; world: WorldPoint; partial: boolean }
  // The hover affordance under the cursor (from result.hoverAffordance on a no-drag move); the shell maps it to a cursor.
  | { type: "affordance"; affordance: HoverAffordance }
  | { type: "status"; message: string };

export type ShapeCanvasEngineOptions = {
  canvas: HTMLCanvasElement;
  overlayRoot: HTMLElement;
  backend: string;
  webGpuRenderer?: RustWebGpuRenderer | null;
  onEvent: (event: EngineEvent) => void;
};

export type FocusBoundsOptions = {
  screen?: WorldPoint;
  zoom?: number;
  padding?: WorldPoint;
  minZoom?: number;
  maxZoom?: number;
};

const MOUSE_POINTER_ID = -1;

// Screen-pixel snap radius for drag-create; the core query converts it to world via the camera zoom.
const CREATE_SNAP_TOLERANCE_PX = 8;

// The transient preview regions buildFeedScene appends to the feed. The snap query MUST exclude
// these, or the preview corner under the cursor self-snaps at ~0 and occludes every real object's edge.
const SNAP_EXCLUDE_IDS = ["create-preview", "create-snap-indicator", "draw-preview"];
const SNAP_EXCLUDE_IDS_JSON = JSON.stringify(SNAP_EXCLUDE_IDS);

export class ShapeCanvasEngine {
  private canvas: HTMLCanvasElement;
  private overlayRoot: HTMLElement;
  private onEvent: (event: EngineEvent) => void;
  private camera: CameraState = { x: 0, y: 0, zoom: 1 };
  private dpr = 1;
  private width = 1;
  private height = 1;
  private running = false;
  private raf = 0;
  private inputBatchSize = 0;
  private boundaryCalls = 0;
  private rustBoundaryCalls = 0;
  private lastWebGpuFrame: RustWebGpuFrameStats | null = null;
  private backend: string;
  private webGpuRenderer: RustWebGpuRenderer | null;
  private webGpuUnavailableNotified = false;
  private mouseDragActive = false;
  private mouseFallbackTarget: EventTarget | null = null;
  // Shift/meta held at the most recent down; consumed by object-select to build a transient multi-select set.
  private lastPointerAdditive = false;
  // When "draw"/"create"/"erase", the handlers intercept input before it reaches the renderer (which stays "select").
  private activeTool: ActiveTool = "select";
  // The in-progress object drag (id + cumulative world-space delta matrix + gesture kind), committed
  // once on pointer-up when the matrix moved off identity. The renderer never mutates object transforms.
  private objectDrag: { id: string; matrix: RenderTransform3x3; kind: TransformKind } | null = null;
  // A pointer-down while Space is held (or a middle-button drag) is a pan gesture; the engine flips the
  // core tool to "hand" for the gesture and restores it on up. The pan state machine stays in the core.
  private spaceHeld = false;
  private panGestureActive = false;
  // True while the erase tool's pointer is held down, so a move keeps erasing along the drag (a bare hover never erases).
  private eraseDragActive = false;
  // True while a create drag is in progress; a move with this false is a bare hover (emits a create-hover probe).
  private createDragActive = false;
  // True while a freehand stroke is in progress; a move with this false is a bare hover (the draw tool shows the same pre-stroke ring as create).
  private drawDragActive = false;
  // The always-on hover mousemove target bound while create/draw is armed (mouse has no down-less move
  // otherwise). Distinct from the mousedown-bound drag-move target so a bare hover never disturbs the drag.
  private createHoverTarget: EventTarget | null = null;
  // The previous eraser sample's screen point, so each move runs the swept hit-test over (prev -> curr)
  // and erases every crossed object — a fast drag that skips between samples still erases the whole path.
  private lastEraseScreen: WorldPoint | null = null;
  // Shift held at the latest event; drives the core's coarse-rotate mode bit (inverted: Shift = free).
  private shiftHeld = false;
  // The last coarse-rotate active bit pushed to the core, so syncCoarseRotate skips redundant boundary calls.
  private coarseRotatePushed: boolean | null = null;
  // Alt held at the latest event. Two gestures read it outside a handler: the endpoint-drag release-snap
  // bypass (processInputResult) and the transform-commit detach bit (commitObjectDrag).
  private altHeld = false;
  // The in-progress open-class endpoint drag — the dragged endpoint (PAIR index) plus the latest world
  // sample and its release-snap probe. Committed once on pointer-up, cleared on cancel (GPU deform reverted).
  private endpointDrag: { id: string; nodeIndex: number; world: WorldPoint; snapped: boolean; targetId: string | null } | null = null;

  constructor(options: ShapeCanvasEngineOptions) {
    this.canvas = options.canvas;
    this.overlayRoot = options.overlayRoot;
    this.onEvent = options.onEvent;
    this.backend = options.backend;
    this.webGpuRenderer = options.webGpuRenderer ?? null;
    if (!this.webGpuRenderer) {
      this.onEvent({ type: "status", message: "WebGPU renderer unavailable: build WASM and use a WebGPU-capable browser." });
      this.webGpuUnavailableNotified = true;
    }
    this.bindInput();
  }

  start() {
    if (this.running) return;
    this.running = true;
    const tick = (now: number) => {
      if (!this.running) return;
      const stats = this.renderFrame(now);
      this.onEvent({ type: "stats", stats });
      this.raf = requestAnimationFrame(tick);
    };
    this.raf = requestAnimationFrame(tick);
  }

  stop() {
    this.running = false;
    if (this.raf && typeof cancelAnimationFrame === "function") cancelAnimationFrame(this.raf);
    this.raf = 0;
    this.unbindInput();
  }

  resize(width: number, height: number, dpr = window.devicePixelRatio || 1) {
    this.width = Math.max(1, width);
    this.height = Math.max(1, height);
    this.dpr = Math.max(1, dpr);
    this.canvas.width = Math.round(this.width * this.dpr);
    this.canvas.height = Math.round(this.height * this.dpr);
    if (this.webGpuRenderer) {
      this.webGpuRenderer.resize(this.width, this.height, this.dpr);
      this.rustBoundaryCalls += 1;
    }
    this.boundaryCalls += 1;
  }

  fitScene() {
    this.sendInputBatch([{ kind: "fit-scene" }]);
  }

  // Set the active pointer tool. The core only knows select/hand; draw/create/erase are shell-side
  // routing (intercepted before reaching the renderer), so the core stays "select".
  setTool(tool: ActiveTool) {
    this.activeTool = tool;
    this.coreSetTool("select");
    // The create AND draw tools arm an always-on hover mousemove (mouse has no down-less move) to drive
    // the persistent anchor ring; leaving them unbinds it and clears the stale drag flags.
    if (tool === "create" || tool === "draw") {
      this.bindCreateHoverMove();
    } else {
      this.unbindCreateHoverMove();
    }
    if (tool !== "create") this.createDragActive = false;
    if (tool !== "draw") this.drawDragActive = false;
  }

  // The shell mirrors the Space key down/up here; a pointer-down while Space is held becomes a pan gesture.
  setSpaceHeld(held: boolean) {
    this.spaceHeld = held;
  }

  // Arm the core's hand-pan path for one pan gesture; the core owns the pan state machine, the engine
  // only flips the core tool to "hand" and restores it on pointer-up.
  private armPanGesture() {
    this.panGestureActive = true;
    this.coreSetTool("hand");
  }

  private disarmPanGesture() {
    if (!this.panGestureActive) return;
    this.panGestureActive = false;
    // The core only knows select/hand, so restore it to "select" for any non-pan tool.
    this.coreSetTool("select");
  }

  // Push a raw core tool string (select|hand) without touching the shell-facing activeTool. Pan-gesture only.
  private coreSetTool(tool: "select" | "hand") {
    if (!this.webGpuRenderer) return;
    if (typeof this.webGpuRenderer.setTool === "function") {
      try {
        this.webGpuRenderer.setTool(tool);
        this.rustBoundaryCalls += 1;
      } catch (error) {
        this.onEvent({
          type: "status",
          message: error instanceof Error ? `Rust setTool failed: ${error.message}` : "Rust setTool failed"
        });
      }
      return;
    }
    this.sendInputBatch([{ kind: "set-tool", tool }]);
  }

  // Push the transient multi-select highlight set (marquee / shift-click). Prefers the direct wasm
  // method, falls back to a set-multi-select input event. Empty clears.
  setMultiSelect(ids: string[]) {
    if (!this.webGpuRenderer) return;
    if (typeof this.webGpuRenderer.setMultiSelect === "function") {
      try {
        this.webGpuRenderer.setMultiSelect(JSON.stringify(ids));
        this.rustBoundaryCalls += 1;
      } catch (error) {
        this.onEvent({
          type: "status",
          message: error instanceof Error ? `Rust setMultiSelect failed: ${error.message}` : "Rust setMultiSelect failed"
        });
      }
      return;
    }
    this.sendInputBatch([{ kind: "set-multi-select", ids }]);
  }

  // Push the coarse-rotate modifier (e.g. Shift held) as a renderer-held mode bit; the core's rotate
  // drag arm snaps the swept delta in-core. Prefers the direct wasm method, falls back to a
  // set-coarse-rotate input event. The shell never decomposes/rebuilds the rotate matrix.
  setCoarseRotate(active: boolean) {
    if (!this.webGpuRenderer) return;
    if (typeof this.webGpuRenderer.setCoarseRotate === "function") {
      try {
        this.webGpuRenderer.setCoarseRotate(active);
        this.rustBoundaryCalls += 1;
      } catch (error) {
        this.onEvent({
          type: "status",
          message: error instanceof Error ? `Rust setCoarseRotate failed: ${error.message}` : "Rust setCoarseRotate failed"
        });
      }
      return;
    }
    this.sendInputBatch([{ kind: "set-coarse-rotate", active }]);
  }

  // Push the coarse-rotate mode bit derived from the latest Shift state through the sanctioned mirrored
  // predicate (inverted: coarse by default, Shift = free). The core then emits the 15°-snapped rotate
  // delta directly, so the shell never decomposes the matrix. Skips redundant pushes (idempotent).
  private syncCoarseRotate() {
    const active = !isCoarseRotate({ shiftKey: this.shiftHeld });
    if (active === this.coarseRotatePushed) return;
    this.coarseRotatePushed = active;
    this.setCoarseRotate(active);
  }

  wheelAtScreen(screen: WorldPoint, deltaY: number) {
    this.sendInputBatch([{ kind: "wheel", screen, deltaY }]);
  }

  focusBounds(bounds: WorldRect, options: FocusBoundsOptions = {}) {
    this.sendInputBatch([{ kind: "focus-bounds", bounds, ...options }]);
  }

  setCamera(camera: CameraState) {
    this.sendInputBatch([{ kind: "set-camera", camera }]);
  }

  getCamera(): CameraState {
    return this.camera;
  }

  renderFrame(now: number): FrameStats {
    const start = performance.now();
    if (this.webGpuRenderer) {
      try {
        this.lastWebGpuFrame = this.webGpuRenderer.renderFrame();
        this.rustBoundaryCalls += 1;
      } catch (error) {
        this.onEvent({
          type: "status",
          message: error instanceof Error ? `WebGPU render failed: ${error.message}` : "WebGPU render failed"
        });
        this.webGpuRenderer = null;
      }
    } else if (!this.webGpuUnavailableNotified) {
      this.onEvent({ type: "status", message: "WebGPU renderer unavailable: no TypeScript canvas fallback is installed." });
      this.webGpuUnavailableNotified = true;
    }

    const renderMs = performance.now() - start;
    const stats: FrameStats = {
      frameMs: renderMs,
      renderMs,
      totalGroups: this.lastWebGpuFrame?.totalGroups ?? 0,
      totalCards: this.lastWebGpuFrame?.totalCards ?? 0,
      totalEdges: this.lastWebGpuFrame?.totalEdges ?? 0,
      visibleGroups: this.lastWebGpuFrame?.visibleGroupCount ?? 0,
      visibleCards: this.lastWebGpuFrame?.visibleCardCount ?? 0,
      visibleEdges: this.lastWebGpuFrame?.visibleEdgeCount ?? 0,
      cacheHits: this.lastWebGpuFrame?.textLayoutCacheHits ?? 0,
      cacheMisses: this.lastWebGpuFrame?.textLayoutCacheMisses ?? 0,
      boundaryCalls: this.boundaryCalls,
      inputBatchSize: this.inputBatchSize,
      memoryBytes: readMemoryBytes(),
      backend: this.backend,
      drawBackend: "rust-wgpu-visible",
      webGpuRendererAvailable: Boolean(this.webGpuRenderer),
      rustBoundaryCalls: this.rustBoundaryCalls,
      rustFrameCards: this.lastWebGpuFrame?.totalCards ?? null,
      rustFrameEdges: this.lastWebGpuFrame?.totalEdges ?? null,
      rustGpuVertices: this.lastWebGpuFrame?.vertexCount ?? null,
      rustDrawnVertices: this.lastWebGpuFrame?.drawnVertexCount ?? null,
      rustDrawRanges: this.lastWebGpuFrame?.drawRangeCount ?? null,
      rustTextGlyphs: this.lastWebGpuFrame?.textGlyphCount ?? null,
      rustFallbackGlyphs: this.lastWebGpuFrame?.fallbackTextGlyphCount ?? null,
      rustCjkGlyphs: this.lastWebGpuFrame?.cjkTextGlyphCount ?? null,
      rustFontFallbackRuns: this.lastWebGpuFrame?.fontFallbackRunCount ?? null,
      rustMissingGlyphs: this.lastWebGpuFrame?.missingTextGlyphCount ?? null,
      rustTextAtlasOverflowGlyphs: this.lastWebGpuFrame?.textAtlasOverflowGlyphCount ?? null,
      rustTextMissingRasterGlyphs: this.lastWebGpuFrame?.textMissingRasterGlyphCount ?? null,
      rustTextAtlasGlyphs: this.lastWebGpuFrame?.textAtlasGlyphCount ?? null,
      rustTextRasterCacheHits: this.lastWebGpuFrame?.textRasterCacheHits ?? null,
      rustTextRasterCacheMisses: this.lastWebGpuFrame?.textRasterCacheMisses ?? null,
      rustTextLayoutCacheHits: this.lastWebGpuFrame?.textLayoutCacheHits ?? null,
      rustTextLayoutCacheMisses: this.lastWebGpuFrame?.textLayoutCacheMisses ?? null,
      rustStyleTokens: this.lastWebGpuFrame?.styleTokenCount ?? null,
      rustPatchUpdates: this.lastWebGpuFrame?.patchUpdateCount ?? null,
      rustDirtyWrites: this.lastWebGpuFrame?.dirtyRangeWriteCount ?? null,
      rustFullRebuilds: this.lastWebGpuFrame?.fullBufferRebuildCount ?? null,
      rustVertexTruncations: this.lastWebGpuFrame?.vertexTruncationCount ?? null,
      rustTruncatedVertices: this.lastWebGpuFrame?.truncatedVertexCount ?? null,
      rustEdgeCapacityGrows: this.lastWebGpuFrame?.edgeCapacityGrowCount ?? null,
      rustEdgeCompactions: this.lastWebGpuFrame?.edgeCompactionCount ?? null,
      rustEdgeSlots: this.lastWebGpuFrame?.edgeSlotCount ?? null,
      rustFreeEdgeSlots: this.lastWebGpuFrame?.edgeSlotFreeCount ?? null,
      rustCardCapacityGrows: this.lastWebGpuFrame?.cardCapacityGrowCount ?? null,
      rustCardCompactions: this.lastWebGpuFrame?.cardCompactionCount ?? null,
      rustCardSlots: this.lastWebGpuFrame?.cardSlotCount ?? null,
      rustFreeCardSlots: this.lastWebGpuFrame?.cardSlotFreeCount ?? null,
      rustGroupCapacityGrows: this.lastWebGpuFrame?.groupCapacityGrowCount ?? null,
      rustGroupCompactions: this.lastWebGpuFrame?.groupCompactionCount ?? null,
      rustGroupSlots: this.lastWebGpuFrame?.groupSlotCount ?? null,
      rustFreeGroupSlots: this.lastWebGpuFrame?.groupSlotFreeCount ?? null,
      rustObjectCount: this.lastWebGpuFrame?.objectCount ?? null,
      rustObjectFillIndices: this.lastWebGpuFrame?.objectFillIndexCount ?? null,
      rustObjectStrokeVertices: this.lastWebGpuFrame?.objectStrokeVertexCount ?? null,
      rustObjectDraws: this.lastWebGpuFrame?.objectDrawCount ?? null,
      rustCameraX: this.camera.x,
      rustCameraY: this.camera.y,
      rustCameraZoom: this.camera.zoom,
      rustSelectionKind: null,
      rustSelectionId: null,
      rustLastHitKind: null,
      rustLastHitId: null,
      rustLastHitField: null,
      rustLastHitPort: null,
      rustLastHitScreenX: null,
      rustLastHitScreenY: null
    };
    this.inputBatchSize = 0;
    return stats;
  }

  private bindInput() {
    this.canvas.addEventListener("pointerdown", this.onPointerDown);
    this.canvas.addEventListener("pointermove", this.onPointerMove);
    this.canvas.addEventListener("pointerup", this.onPointerUp);
    this.canvas.addEventListener("pointercancel", this.onPointerCancel);
    this.canvas.addEventListener("mousedown", this.onMouseDown);
    this.canvas.addEventListener("dblclick", this.onDoubleClick);
    this.canvas.addEventListener("wheel", this.onWheel, { passive: false });
  }

  private unbindInput() {
    this.canvas.removeEventListener("pointerdown", this.onPointerDown);
    this.canvas.removeEventListener("pointermove", this.onPointerMove);
    this.canvas.removeEventListener("pointerup", this.onPointerUp);
    this.canvas.removeEventListener("pointercancel", this.onPointerCancel);
    this.canvas.removeEventListener("mousedown", this.onMouseDown);
    this.unbindMouseFallbackMove();
    this.unbindCreateHoverMove();
    this.canvas.removeEventListener("dblclick", this.onDoubleClick);
    this.canvas.removeEventListener("wheel", this.onWheel);
  }

  private onPointerDown = (event: PointerEvent) => {
    if (isMousePointerEvent(event)) return;
    this.shiftHeld = event.shiftKey;
    this.altHeld = event.altKey;
    this.syncCoarseRotate();
    // Space-hold pans even under the draw tool; arm the core pan path first.
    const pan = isPanIntent({ spaceHeld: this.spaceHeld, button: event.button });
    if (this.activeTool === "draw" && !pan) {
      this.drawDragActive = true;
      this.emitDraw("start", event);
      this.canvas.setPointerCapture(event.pointerId);
      return;
    }
    if (this.activeTool === "create" && !pan) {
      this.createDragActive = true;
      this.emitCreate("start", event);
      this.canvas.setPointerCapture(event.pointerId);
      return;
    }
    if (this.activeTool === "erase" && !pan) {
      this.eraseDragActive = true;
      this.lastEraseScreen = this.eventPoint(event);
      this.emitErase(event);
      this.canvas.setPointerCapture(event.pointerId);
      return;
    }
    if (pan) this.armPanGesture();
    this.lastPointerAdditive = isAdditiveSelect(event, detectMac());
    const screen = this.eventPoint(event);
    this.sendInputBatch([{ kind: "pointer-down", pointerId: event.pointerId, screen }]);
    this.canvas.setPointerCapture(event.pointerId);
  };

  private onPointerMove = (event: PointerEvent) => {
    if (isMousePointerEvent(event)) return;
    this.shiftHeld = event.shiftKey;
    this.altHeld = event.altKey;
    this.syncCoarseRotate();
    // A Space-armed pan stays on the pan path for the whole gesture, even under the draw tool.
    if (this.activeTool === "draw" && !this.panGestureActive) {
      // A move with no stroke in progress is a bare hover — emit the anchor-ring snap probe instead of a stroke sample.
      if (createMoveEmission({ dragActive: this.drawDragActive }) === "hover") {
        this.emitCreateHover(event);
      } else {
        this.emitDraw("move", event);
      }
      return;
    }
    if (this.activeTool === "create" && !this.panGestureActive) {
      // A move with no create drag in progress is a bare hover — emit the anchor-ring snap probe.
      if (createMoveEmission({ dragActive: this.createDragActive }) === "hover") {
        this.emitCreateHover(event);
      } else {
        this.emitCreate("move", event);
      }
      return;
    }
    if (this.activeTool === "erase" && !this.panGestureActive && this.eraseDragActive) {
      this.emitSweptErase(event);
      return;
    }
    this.sendInputBatch([{ kind: "pointer-move", pointerId: event.pointerId, screen: this.eventPoint(event) }]);
  };

  private onPointerUp = (event: PointerEvent) => {
    if (isMousePointerEvent(event)) return;
    if (this.activeTool === "draw" && !this.panGestureActive) {
      this.drawDragActive = false;
      this.emitDraw("end", event);
      try {
        this.canvas.releasePointerCapture(event.pointerId);
      } catch {
        // Pointer capture may already be released after cancellation.
      }
      return;
    }
    if (this.activeTool === "create" && !this.panGestureActive) {
      this.createDragActive = false;
      this.emitCreate("end", event);
      try {
        this.canvas.releasePointerCapture(event.pointerId);
      } catch {
        // Pointer capture may already be released after cancellation.
      }
      return;
    }
    if (this.activeTool === "erase" && !this.panGestureActive) {
      this.eraseDragActive = false;
      this.lastEraseScreen = null;
      try {
        this.canvas.releasePointerCapture(event.pointerId);
      } catch {
        // Pointer capture may already be released after cancellation.
      }
      return;
    }
    this.sendInputBatch([
      {
        kind: "pointer-up",
        pointerId: event.pointerId,
        screen: this.eventPoint(event),
        edgeId: `renderer-edge-${crypto.randomUUID().slice(0, 8)}`
      }
    ]);
    this.commitObjectDrag();
    this.commitEndpointDrag();
    this.disarmPanGesture();
    try {
      this.canvas.releasePointerCapture(event.pointerId);
    } catch {
      // Pointer capture may already be released after cancellation.
    }
  };

  private onPointerCancel = (event: PointerEvent) => {
    if (isMousePointerEvent(event)) return;
    if (this.activeTool === "draw" && !this.panGestureActive) {
      this.drawDragActive = false;
      this.emitDraw("cancel", event);
      try {
        this.canvas.releasePointerCapture(event.pointerId);
      } catch {
        // Pointer capture may already be released.
      }
      return;
    }
    if (this.activeTool === "create" && !this.panGestureActive) {
      this.createDragActive = false;
      this.emitCreate("cancel", event);
      try {
        this.canvas.releasePointerCapture(event.pointerId);
      } catch {
        // Pointer capture may already be released.
      }
      return;
    }
    if (this.activeTool === "erase" && !this.panGestureActive) {
      this.eraseDragActive = false;
      this.lastEraseScreen = null;
      try {
        this.canvas.releasePointerCapture(event.pointerId);
      } catch {
        // Pointer capture may already be released.
      }
      return;
    }
    this.sendInputBatch([{ kind: "pointer-cancel", pointerId: event.pointerId }]);
    this.objectDrag = null;
    this.cancelEndpointDrag();
    this.disarmPanGesture();
  };

  private onMouseDown = (event: MouseEvent) => {
    this.shiftHeld = event.shiftKey;
    this.altHeld = event.altKey;
    this.syncCoarseRotate();
    // Left (0) drives select/draw; middle (1) is a pan gesture; right (2) is the context menu (shell-handled) — ignore here.
    const pan = isPanIntent({ spaceHeld: this.spaceHeld, button: event.button });
    if (event.button !== 0 && !pan) return;
    event.preventDefault();
    if (this.activeTool === "draw" && !pan) {
      this.mouseDragActive = true;
      this.drawDragActive = true;
      this.bindMouseFallbackMove();
      this.emitDraw("start", event);
      return;
    }
    if (this.activeTool === "create" && !pan) {
      this.mouseDragActive = true;
      this.createDragActive = true;
      this.bindMouseFallbackMove();
      this.emitCreate("start", event);
      return;
    }
    if (this.activeTool === "erase" && !pan) {
      this.mouseDragActive = true;
      this.eraseDragActive = true;
      this.lastEraseScreen = this.eventPoint(event);
      this.bindMouseFallbackMove();
      this.emitErase(event);
      return;
    }
    if (pan) this.armPanGesture();
    this.lastPointerAdditive = isAdditiveSelect(event, detectMac());
    this.mouseDragActive = true;
    this.bindMouseFallbackMove();
    this.sendInputBatch([{ kind: "pointer-down", pointerId: MOUSE_POINTER_ID, screen: this.eventPoint(event) }]);
  };

  private onMouseMove = (event: MouseEvent) => {
    if (!this.mouseDragActive) return;
    event.preventDefault();
    this.shiftHeld = event.shiftKey;
    this.altHeld = event.altKey;
    this.syncCoarseRotate();
    if (this.activeTool === "draw" && !this.panGestureActive) {
      this.emitDraw("move", event);
      return;
    }
    if (this.activeTool === "create" && !this.panGestureActive) {
      this.emitCreate("move", event);
      return;
    }
    if (this.activeTool === "erase" && !this.panGestureActive && this.eraseDragActive) {
      this.emitSweptErase(event);
      return;
    }
    this.sendInputBatch([{ kind: "pointer-move", pointerId: MOUSE_POINTER_ID, screen: this.eventPoint(event) }]);
  };

  private onMouseUp = (event: MouseEvent) => {
    if (!this.mouseDragActive) return;
    event.preventDefault();
    this.mouseDragActive = false;
    this.unbindMouseFallbackMove();
    if (this.activeTool === "draw" && !this.panGestureActive) {
      this.drawDragActive = false;
      this.emitDraw("end", event);
      return;
    }
    if (this.activeTool === "create" && !this.panGestureActive) {
      this.createDragActive = false;
      this.emitCreate("end", event);
      return;
    }
    if (this.activeTool === "erase" && !this.panGestureActive) {
      this.eraseDragActive = false;
      this.lastEraseScreen = null;
      return;
    }
    this.sendInputBatch([
      {
        kind: "pointer-up",
        pointerId: MOUSE_POINTER_ID,
        screen: this.eventPoint(event),
        edgeId: `renderer-edge-${crypto.randomUUID().slice(0, 8)}`
      }
    ]);
    this.commitObjectDrag();
    this.commitEndpointDrag();
    this.disarmPanGesture();
  };

  // The always-on create/draw-tool hover mousemove: a bare hover emits a create-hover snap probe so the
  // anchor ring tracks the cursor BEFORE any drag. Skipped during a drag or a Space-armed pan.
  private onCreateHoverMove = (event: MouseEvent) => {
    if (this.mouseDragActive || this.panGestureActive) return;
    this.emitCreateHover(event);
  };

  private onDoubleClick = (event: MouseEvent) => {
    this.sendInputBatch([{ kind: "double-click", screen: this.eventPoint(event) }]);
  };

  private onWheel = (event: WheelEvent) => {
    event.preventDefault();
    this.sendInputBatch([{ kind: "wheel", screen: this.eventPoint(event), deltaY: event.deltaY }]);
  };

  private eventPoint(event: MouseEvent | PointerEvent | WheelEvent): WorldPoint {
    const rect = this.canvas.getBoundingClientRect();
    return { x: event.clientX - rect.left, y: event.clientY - rect.top };
  }

  // The world point under a pointer event, un-projected through the LIVE core camera (the renderer's
  // own screen_to_world). The shell keeps no TS affine; without a live renderer this degrades to the
  // raw screen point (the emit paths it feeds are no-ops anyway when the renderer is absent).
  private eventWorld(event: MouseEvent | PointerEvent): WorldPoint {
    const screen = this.eventPoint(event);
    return this.projectScreenToWorld(screen) ?? screen;
  }

  // Bind/unbind the always-on create-tool hover mousemove (a bare hover has no other move source). Idempotent.
  private bindCreateHoverMove() {
    if (this.createHoverTarget) return;
    this.createHoverTarget = this.canvas;
    this.createHoverTarget.addEventListener("mousemove", this.onCreateHoverMove as EventListener);
  }

  private unbindCreateHoverMove() {
    if (!this.createHoverTarget) return;
    this.createHoverTarget.removeEventListener("mousemove", this.onCreateHoverMove as EventListener);
    this.createHoverTarget = null;
  }

  private bindMouseFallbackMove() {
    if (this.mouseFallbackTarget) return;
    this.mouseFallbackTarget = mouseFallbackTarget(this.canvas);
    this.mouseFallbackTarget.addEventListener("mousemove", this.onMouseMove as EventListener);
    this.mouseFallbackTarget.addEventListener("mouseup", this.onMouseUp as EventListener);
  }

  private unbindMouseFallbackMove() {
    if (!this.mouseFallbackTarget) return;
    this.mouseFallbackTarget.removeEventListener("mousemove", this.onMouseMove as EventListener);
    this.mouseFallbackTarget.removeEventListener("mouseup", this.onMouseUp as EventListener);
    this.mouseFallbackTarget = null;
    this.mouseDragActive = false;
  }

  private sendInputBatch(events: RustCanvasInputEvent[]): RustInputBatchResult | null {
    if (!this.webGpuRenderer) {
      if (!this.webGpuUnavailableNotified) {
        this.onEvent({ type: "status", message: "WebGPU renderer unavailable: no Rust input interpreter is installed." });
        this.webGpuUnavailableNotified = true;
      }
      return null;
    }
    try {
      const result = this.webGpuRenderer.inputBatch(JSON.stringify(events));
      this.boundaryCalls += 1;
      this.inputBatchSize += events.length;
      this.rustBoundaryCalls += 1;
      this.camera = result.camera;
      this.processInputResult(result);
      this.onEvent({ type: "stats", stats: this.renderFrame(performance.now()) });
      return result;
    } catch (error) {
      this.onEvent({
        type: "status",
        message: error instanceof Error ? `Rust input batch failed: ${error.message}` : "Rust input batch failed"
      });
      return null;
    }
  }

  private processInputResult(result: RustInputBatchResult) {
    // An object pick on pointer-down starts a remembered drag; each move delta updates it and previews;
    // an empty-start marquee forwards its ids. The commit op is authored on pointer-up.
    if (typeof result.objectSelection === "string") {
      this.objectDrag = { id: result.objectSelection, matrix: IDENTITY_MATRIX, kind: "translate" };
      this.onEvent({ type: "object-select", id: result.objectSelection, additive: this.lastPointerAdditive });
    }
    if (result.objectTransformDelta) {
      const { id, kind, matrix } = result.objectTransformDelta;
      // The core already snaps the swept rotate delta when coarse-rotate is active (driven via
      // setCoarseRotate off the mirrored predicate), so the shell consumes the matrix as-is.
      this.objectDrag = { id, matrix, kind };
      // Drag zero-rebake: push the delta straight to the GPU instance matrix (no Svelte round-trip, no
      // re-tessellation). The preview event still rides through for the shell's commit/snap-back bookkeeping.
      this.webGpuRenderer?.setObjectPreviewTransform?.(id, JSON.stringify(matrix));
      this.onEvent({ type: "object-transform-preview", id, matrix, kind });
    }
    // A live endpoint-drag sample. The chord deform rides setObjectEndpointPreview (no Svelte round-trip);
    // the release-snap probe reuses the drag-create outline query, excluding the dragged object so its own
    // outline never self-snaps, and honoring the Alt snap-bypass. The commit is authored once on pointer-up.
    if (result.objectEndpointDelta) {
      const { id, nodeIndex, x, y } = result.objectEndpointDelta;
      const snap = shouldQuerySnap({ altHeld: this.altHeld, phase: "move" }) ? this.querySnap({ x, y }, [id]) : null;
      const world = snap ? { x: snap.x, y: snap.y } : { x, y };
      const drag = { id, nodeIndex, world, snapped: snap !== null, targetId: snap?.targetId ?? null };
      this.endpointDrag = drag;
      this.webGpuRenderer?.setObjectEndpointPreview?.(id, nodeIndex, world.x, world.y);
      this.onEvent({ type: "object-endpoint-preview", ...drag });
    }
    if (result.objectMarqueeIds != null) {
      this.onEvent({ type: "object-marquee", ids: result.objectMarqueeIds });
    }
    if (result.objectDoubleClick) {
      const { id, hasChildren } = result.objectDoubleClick;
      this.onEvent({ type: "object-double-click", id, hasChildren });
    }
    // Older wasm builds omit hoverAffordance (defaults to "empty").
    this.onEvent({ type: "affordance", affordance: result.hoverAffordance ?? "empty" });
  }

  // Emit the single undoable transform commit when an object drag moved (matrix off identity), then clear
  // it. Called on pointer-up AFTER the batch. `detach` is the Alt-held-at-release bit; the shell owns the branch.
  private commitObjectDrag() {
    const drag = this.objectDrag;
    this.objectDrag = null;
    if (drag && !isIdentityMatrix(drag.matrix)) {
      this.onEvent({
        type: "object-transform-commit",
        id: drag.id,
        matrix: drag.matrix,
        kind: drag.kind,
        detach: isDetachDrag({ altKey: this.altHeld })
      });
    }
  }

  // Emit the single undoable endpoint release when a drag is in flight, then clear it. The release point
  // + snap are the LAST move's sample (the pointer-up lands where the final move left it).
  private commitEndpointDrag() {
    const drag = this.endpointDrag;
    this.endpointDrag = null;
    if (drag) this.onEvent({ type: "object-endpoint-commit", ...drag });
  }

  // Drop an in-flight endpoint drag without committing (pointer-cancel), reverting the GPU chord deform.
  private cancelEndpointDrag() {
    const drag = this.endpointDrag;
    this.endpointDrag = null;
    if (drag) this.webGpuRenderer?.clearObjectEndpointPreview?.(drag.id);
  }

  // Re-apply an in-flight endpoint drag's latest chord deform. A mid-drag re-feed rebuilds every baked
  // geometry, wiping the live deform — the host calls this after each feed so the preview survives.
  refreshEndpointPreview(): void {
    const drag = this.endpointDrag;
    if (!drag) return;
    this.webGpuRenderer?.setObjectEndpointPreview?.(drag.id, drag.nodeIndex, drag.world.x, drag.world.y);
  }

  // Emit a draw phase with the world point under the cursor (replaces the select/marquee path while the
  // draw tool is active). Carries the outline snap probe so the shell can seed anchors, but `world` stays
  // the RAW pointer — recognition normalizes the silhouette, so mid-stroke samples must not be pulled onto an edge.
  private emitDraw(phase: "start" | "move" | "end" | "cancel", event: MouseEvent | PointerEvent) {
    const world = this.eventWorld(event);
    const snap = shouldQuerySnap({ altHeld: event.altKey, phase }) ? this.querySnap(world) : null;
    this.onEvent({
      type: "draw",
      phase,
      world,
      snap: snap && snap.targetId !== null ? { at: { x: snap.x, y: snap.y }, targetId: snap.targetId } : null
    });
  }

  // Emit a create phase with the dragged corner in world space. The corner snaps to the nearest outline
  // anchor within tolerance unless Alt is held. The snap query is the core path, so geometry truth stays in Rust.
  private emitCreate(phase: "start" | "move" | "end" | "cancel", event: MouseEvent | PointerEvent) {
    const raw = this.eventWorld(event);
    const snap = shouldQuerySnap({ altHeld: event.altKey, phase }) ? this.querySnap(raw) : null;
    const world = snap ? { x: snap.x, y: snap.y } : raw;
    this.onEvent({ type: "create", phase, world, snapped: snap !== null, targetId: snap?.targetId ?? null });
  }

  // Emit a HOVER snap probe for a bare create-tool move. Runs the same outline snap query as the drag
  // move so the anchor ring shows where the next create would anchor. `phase: "move"` so a held Alt suppresses the query.
  private emitCreateHover(event: MouseEvent | PointerEvent) {
    const raw = this.eventWorld(event);
    const snap = shouldQuerySnap({ altHeld: event.altKey, phase: "move" }) ? this.querySnap(raw) : null;
    const world = snap ? { x: snap.x, y: snap.y } : raw;
    this.onEvent({ type: "create-hover", world, snapped: snap !== null, targetId: snap?.targetId ?? null });
  }

  // Emit an erase touch for the stroke under the cursor. The id comes from the core's hit-test (stays in
  // the core); a touch over empty canvas emits nothing. `partial` = Alt held (whole-stroke delete by default).
  private emitErase(event: MouseEvent | PointerEvent) {
    const screen = this.eventPoint(event);
    const id = this.objectHitTest(screen);
    if (!id) return;
    const world = this.eventWorld(event);
    this.onEvent({ type: "erase", id, world, partial: isPartialErase(event) });
  }

  // Emit one erase touch for EVERY object the eraser crossed since the previous sample (swept hit-test
  // over prev -> curr SCREEN samples), so a fast drag erases the whole path. Falls back to single-sample
  // `emitErase` when no prior sample exists or the swept method is unavailable.
  private emitSweptErase(event: MouseEvent | PointerEvent) {
    const curr = this.eventPoint(event);
    const prev = this.lastEraseScreen;
    this.lastEraseScreen = curr;
    const ids = prev ? this.sweptEraseHitTest(prev, curr) : null;
    if (!ids) {
      this.emitErase(event);
      return;
    }
    const world = this.eventWorld(event);
    const partial = isPartialErase(event);
    for (const id of ids) this.onEvent({ type: "erase", id, world, partial });
  }

  // The ids of every object crossed by the eraser between two consecutive SCREEN samples.
  // Feature-detected: a wasm build predating `sweptEraseAt` returns null so the caller falls back to single-sample.
  private sweptEraseHitTest(prev: WorldPoint, curr: WorldPoint): string[] | null {
    const renderer = this.webGpuRenderer;
    if (!renderer || typeof renderer.sweptEraseAt !== "function") return null;
    try {
      const ids = renderer.sweptEraseAt(prev.x, prev.y, curr.x, curr.y);
      this.rustBoundaryCalls += 1;
      return ids ?? [];
    } catch (error) {
      this.onEvent({
        type: "status",
        message: error instanceof Error ? `Rust sweptEraseAt failed: ${error.message}` : "Rust sweptEraseAt failed"
      });
      return null;
    }
  }

  // Snap a world point to the nearest object outline anchor via the core query: returns the snapped
  // WORLD point + target object id within tolerance, else null. `extraExcludeIds` lets the endpoint drag
  // exclude the DRAGGED object, whose own outline under the cursor would otherwise self-snap at ~0.
  private querySnap(world: WorldPoint, extraExcludeIds?: string[]): { x: number; y: number; targetId: string | null } | null {
    const renderer = this.webGpuRenderer;
    if (!renderer || typeof renderer.nearestOutlinePoint !== "function") return null;
    const excludeIdsJson = extraExcludeIds?.length
      ? JSON.stringify([...SNAP_EXCLUDE_IDS, ...extraExcludeIds])
      : SNAP_EXCLUDE_IDS_JSON;
    try {
      const result = renderer.nearestOutlinePoint(world.x, world.y, CREATE_SNAP_TOLERANCE_PX, this.camera.zoom, excludeIdsJson);
      this.rustBoundaryCalls += 1;
      return result.snapped ? { x: result.x, y: result.y, targetId: result.targetId } : null;
    } catch (error) {
      this.onEvent({
        type: "status",
        message: error instanceof Error ? `Rust nearestOutlinePoint failed: ${error.message}` : "Rust nearestOutlinePoint failed"
      });
      return null;
    }
  }

  // Pure object pick for the shell's right-click context menu.
  objectHitTest(screen: WorldPoint): string | null {
    if (!this.webGpuRenderer || typeof this.webGpuRenderer.hitTestObject !== "function") return null;
    try {
      const id = this.webGpuRenderer.hitTestObject(screen.x, screen.y);
      this.rustBoundaryCalls += 1;
      return id ?? null;
    } catch (error) {
      this.onEvent({
        type: "status",
        message: error instanceof Error ? `Rust hitTestObject failed: ${error.message}` : "Rust hitTestObject failed"
      });
      return null;
    }
  }

  // Project a WORLD point to SCREEN space through the LIVE core camera so the shell never recomputes
  // the transform from a mirrored CameraState (the shell holds no TS affine). Feature-detected: a wasm
  // build predating `worldToScreen` returns null and the caller skips the projection.
  projectWorldToScreen(world: WorldPoint): WorldPoint | null {
    const renderer = this.webGpuRenderer;
    if (!renderer || typeof renderer.worldToScreen !== "function") return null;
    try {
      const screen = renderer.worldToScreen(world.x, world.y) as WorldPoint;
      this.rustBoundaryCalls += 1;
      return screen;
    } catch (error) {
      this.onEvent({
        type: "status",
        message: error instanceof Error ? `Rust worldToScreen failed: ${error.message}` : "Rust worldToScreen failed"
      });
      return null;
    }
  }

  // Un-project a SCREEN point to WORLD space through the LIVE core camera (exact inverse of
  // `projectWorldToScreen`). Feature-detected like above.
  projectScreenToWorld(screen: WorldPoint): WorldPoint | null {
    const renderer = this.webGpuRenderer;
    if (!renderer || typeof renderer.screenToWorld !== "function") return null;
    try {
      const world = renderer.screenToWorld(screen.x, screen.y) as WorldPoint;
      this.rustBoundaryCalls += 1;
      return world;
    } catch (error) {
      this.onEvent({
        type: "status",
        message: error instanceof Error ? `Rust screenToWorld failed: ${error.message}` : "Rust screenToWorld failed"
      });
      return null;
    }
  }

}

// Row-major identity transform + exact-equality check, used to seed the remembered drag and detect a
// no-op (un-moved) drag so the commit op is skipped.
const IDENTITY_MATRIX: RenderTransform3x3 = [
  [1, 0, 0],
  [0, 1, 0],
  [0, 0, 1]
];

function isIdentityMatrix(m: RenderTransform3x3): boolean {
  return m.every((row, i) => row.every((v, j) => v === IDENTITY_MATRIX[i][j]));
}

function readMemoryBytes(): number | null {
  const performanceWithMemory = performance as Performance & { memory?: { usedJSHeapSize: number } };
  return performanceWithMemory.memory?.usedJSHeapSize ?? null;
}

function mouseFallbackTarget(canvas: HTMLCanvasElement): EventTarget {
  return typeof window === "undefined" ? canvas : window;
}

function isMousePointerEvent(event: PointerEvent): boolean {
  return event.pointerType === "mouse";
}
