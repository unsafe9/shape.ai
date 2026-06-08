import {
  applyScenePatch,
  screenToWorld,
  type CameraState,
  type DomOverlayRequest,
  type FrameStats,
  type HitResult,
  type ScenePatch,
  type SceneSelection,
  type SceneSnapshot,
  type WorldPoint,
  type WorldRect
} from "./scene";
import type {
  RustCanvasInputEvent,
  RustDebugSnapshot,
  RustHitResult,
  RustInputBatchResult,
  RustMarqueeResult,
  RustWebGpuFrameStats,
  RustWebGpuRenderer
} from "./wasmLoader";

/** CC1.4: the active pointer tool. "select" picks/drags, "hand" pans, "draw"
 *  captures a freehand stroke (FC-11). */
export type ActiveTool = "select" | "hand" | "draw";

export type EngineEvent =
  | { type: "stats"; stats: FrameStats }
  // T2.2: `additive` carries the shift/meta modifier held at pick time so the shell
  // can toggle the hit into a transient multi-select set instead of replacing it.
  | { type: "selection"; hit: HitResult | null; additive: boolean }
  | { type: "patch"; patch: ScenePatch; errors: string[] }
  | { type: "overlay"; request: DomOverlayRequest | null }
  | { type: "gesture"; active: boolean }
  // CC2.3: emitted on the pointer-up that ends a marquee drag. ids = node ids
  // first, then group ids, whose world AABB intersects the final rect. The shell
  // merges them into its transient multiSelectIds set.
  | { type: "marquee"; rect: WorldRect; ids: string[] }
  // CC4.1: right-click pick result, for the context menu. Does not change selection.
  | { type: "context-pick"; hit: HitResult | null; screen: WorldPoint }
  // FC-08: object-path input results. `object-select` rides the pointer-down that
  // picked an object; `object-transform-preview` rides each pointer-move during an
  // object drag (cumulative world-px delta from the pointer-down point — a
  // non-destructive preview, the shell does NOT author yet); `object-transform-commit`
  // is emitted once on pointer-up when the drag moved, and is the single undoable
  // op; `object-marquee` rides the pointer-up of an empty-start drag.
  | { type: "object-select"; id: string }
  | { type: "object-transform-preview"; id: string; dx: number; dy: number }
  | { type: "object-transform-commit"; id: string; dx: number; dy: number }
  | { type: "object-marquee"; ids: string[] }
  // FC-11: freehand pen capture. While the draw tool is active, pointer/mouse
  // down/move/up emit draw phases instead of the select/marquee path; the shell
  // accumulates the world points and commits the stroke to an object on `end`.
  | { type: "draw"; phase: "start" | "move" | "end" | "cancel"; world: WorldPoint }
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

export class ShapeCanvasEngine {
  private canvas: HTMLCanvasElement;
  private overlayRoot: HTMLElement;
  private onEvent: (event: EngineEvent) => void;
  private snapshot: SceneSnapshot | null = null;
  private camera: CameraState = { x: 0, y: 0, zoom: 1 };
  private dpr = 1;
  private width = 1;
  private height = 1;
  private running = false;
  private raf = 0;
  private activeOverlay: HTMLTextAreaElement | null = null;
  private activeOverlayRequest: DomOverlayRequest | null = null;
  private inputBatchSize = 0;
  private boundaryCalls = 0;
  private rustBoundaryCalls = 0;
  private lastWebGpuFrame: RustWebGpuFrameStats | null = null;
  private lastDebugSnapshot: RustDebugSnapshot | null = null;
  private backend: string;
  private webGpuRenderer: RustWebGpuRenderer | null;
  private webGpuUnavailableNotified = false;
  private mouseDragActive = false;
  private mouseFallbackTarget: EventTarget | null = null;
  private inputGestureActive = false;
  // T2.2: shift/meta held at the most recent pointer/mouse-down; consumed by the
  // selection event so the shell can build a transient multi-select set.
  private lastPointerAdditive = false;
  private deferredScene: SceneSnapshot | null = null;
  private deferredSceneRaf = 0;
  // FC-11: the locally-tracked active tool. When "draw", pointer/mouse handlers
  // emit draw phases instead of the renderer select/marquee input path.
  private activeTool: ActiveTool = "select";
  // FC-08: the in-progress object drag (id + cumulative world-px delta). Set on the
  // pointer-down that picks an object, updated on each move, and committed once on
  // pointer-up when it moved. The renderer never mutates object transforms.
  private objectDrag: { id: string; dx: number; dy: number } | null = null;

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

  loadScene(snapshot: SceneSnapshot) {
    if (this.inputGestureActive) {
      this.deferredScene = snapshot;
      return;
    }
    this.loadSceneNow(snapshot);
  }

  private loadSceneNow(snapshot: SceneSnapshot) {
    this.deferredScene = null;
    this.snapshot = snapshot;
    this.camera = snapshot.camera;
    this.syncWebGpuScene(snapshot);
    this.boundaryCalls += 1;
    this.onEvent({ type: "stats", stats: this.renderFrame(performance.now()) });
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
    if (this.deferredSceneRaf && typeof cancelAnimationFrame === "function") cancelAnimationFrame(this.deferredSceneRaf);
    this.raf = 0;
    this.deferredSceneRaf = 0;
    this.inputGestureActive = false;
    this.deferredScene = null;
    this.unbindInput();
    this.removeOverlay(false);
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
    this.updateOverlayPosition();
  }

  fitScene() {
    this.sendInputBatch([{ kind: "fit-scene" }]);
    this.updateOverlayPosition();
  }

  // CC1.4: set the active pointer tool. Prefer the direct wasm method (a tool
  // toggle rarely coincides with a pointer batch); fall back to a set-tool input
  // event if the build predates the direct method.
  setTool(tool: ActiveTool) {
    this.activeTool = tool;
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
    // The renderer-core only knows select/hand; "draw" is shell-side routing, so
    // the fallback keeps the renderer in the neutral select state (draw input is
    // intercepted by the engine before it reaches the renderer).
    this.sendInputBatch([{ kind: "set-tool", tool: tool === "hand" ? "hand" : "select" }]);
  }

  // Push the transient multi-select highlight set to the renderer (marquee /
  // shift-click). Prefers the direct wasm method; falls back to a set-multi-select
  // input event. A wasm build predating either is a no-op (the multi highlight is
  // additive over the single anchor, so older builds just lose it). Empty clears.
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

  // CC4.1: pure hit-test for the right-click context menu. Prefers the direct
  // wasm hitTest (no mutation); falls back to a context-pick input event whose
  // result.hit is surfaced without changing selection.
  contextPick(screen: WorldPoint): HitResult | null {
    if (!this.webGpuRenderer) return null;
    let hit: HitResult | null = null;
    if (typeof this.webGpuRenderer.hitTest === "function") {
      try {
        hit = rustHitToEngineHit(this.webGpuRenderer.hitTest(screen.x, screen.y));
        this.rustBoundaryCalls += 1;
      } catch (error) {
        this.onEvent({
          type: "status",
          message: error instanceof Error ? `Rust hitTest failed: ${error.message}` : "Rust hitTest failed"
        });
        hit = null;
      }
    } else {
      const result = this.sendInputBatch([{ kind: "context-pick", screen }]);
      hit = rustHitToEngineHit(result?.hit ?? null);
    }
    this.onEvent({ type: "context-pick", hit, screen });
    return hit;
  }

  wheelAtScreen(screen: WorldPoint, deltaY: number) {
    this.sendInputBatch([{ kind: "wheel", screen, deltaY }]);
    this.updateOverlayPosition();
  }

  focusBounds(bounds: WorldRect, options: FocusBoundsOptions = {}) {
    this.sendInputBatch([{ kind: "focus-bounds", bounds, ...options }]);
    this.updateOverlayPosition();
  }

  setCamera(camera: CameraState) {
    this.sendInputBatch([{ kind: "set-camera", camera }]);
    this.updateOverlayPosition();
  }

  getCamera(): CameraState {
    return this.camera;
  }

  getSnapshot(): SceneSnapshot | null {
    return this.snapshot;
  }

  debugSnapshot(): RustDebugSnapshot | null {
    return this.updateDebugSnapshot();
  }

  applyPatch(patch: ScenePatch): string[] {
    return this.applyPatchBatch([patch]);
  }

  applyPatchBatch(patches: ScenePatch[]): string[] {
    if (!this.snapshot) return ["No scene loaded"];
    const errors = this.applyPatchBatchInRust(patches);
    if (errors.length > 0) {
      for (const patch of patches) this.onEvent({ type: "patch", patch, errors });
      return errors;
    }
    this.mirrorAcceptedPatches(patches, false);
    this.boundaryCalls += 1;
    for (const patch of patches) this.onEvent({ type: "patch", patch, errors: [] });
    this.onEvent({ type: "stats", stats: this.renderFrame(performance.now()) });
    return [];
  }

  syncSelection(selection: SceneSelection): string[] {
    if (!this.snapshot || selectionEqual(this.snapshot.selection, selection)) return [];
    const patch: ScenePatch = { kind: "select", selection };
    const errors = this.applyPatchBatchInRust([patch]);
    if (errors.length > 0) return errors;
    this.mirrorAcceptedPatches([patch], false);
    this.boundaryCalls += 1;
    this.onEvent({ type: "stats", stats: this.renderFrame(performance.now()) });
    return [];
  }

  beginTextEdit(hit: HitResult): DomOverlayRequest | null {
    if (hit.kind !== "text" || !hit.field) return null;
    const request = this.requestOverlay(hit.id, hit.field);
    if (!request) return null;
    this.mountOverlay(request);
    this.onEvent({ type: "overlay", request });
    return request;
  }

  commitTextEdit() {
    if (!this.activeOverlay || !this.activeOverlayRequest) return;
    const { id, field } = this.activeOverlayRequest.target;
    const value = this.activeOverlay.value;
    this.removeOverlay(true);
    this.applyPatch({ kind: "edit-card-text", id, field, value });
  }

  renderFrame(now: number): FrameStats {
    const start = performance.now();
    if (this.webGpuRenderer) {
      try {
        this.lastWebGpuFrame = this.webGpuRenderer.renderFrame();
        this.rustBoundaryCalls += 1;
        this.updateDebugSnapshot();
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
    if (!this.snapshot) return this.emptyStats(start);

    this.updateOverlayPosition();
    const renderMs = performance.now() - start;
    const stats: FrameStats = {
      frameMs: renderMs,
      renderMs,
      totalGroups: this.snapshot.groups.length,
      totalCards: this.snapshot.cards.length,
      totalEdges: this.snapshot.edges.length,
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
      rustCameraX: this.lastDebugSnapshot?.camera.x ?? null,
      rustCameraY: this.lastDebugSnapshot?.camera.y ?? null,
      rustCameraZoom: this.lastDebugSnapshot?.camera.zoom ?? null,
      rustSelectionKind: this.lastDebugSnapshot?.selection.kind ?? null,
      rustSelectionId: debugSelectionId(this.lastDebugSnapshot?.selection ?? null),
      rustLastHitKind: this.lastDebugSnapshot?.lastHit?.kind ?? null,
      rustLastHitId: this.lastDebugSnapshot?.lastHit?.id ?? null,
      rustLastHitField: this.lastDebugSnapshot?.lastHit?.field ?? null,
      rustLastHitPort: this.lastDebugSnapshot?.lastHit?.port ?? null,
      rustLastHitScreenX: this.lastDebugSnapshot?.lastHit?.screenX ?? null,
      rustLastHitScreenY: this.lastDebugSnapshot?.lastHit?.screenY ?? null
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
    this.canvas.removeEventListener("dblclick", this.onDoubleClick);
    this.canvas.removeEventListener("wheel", this.onWheel);
  }

  private onPointerDown = (event: PointerEvent) => {
    if (isMousePointerEvent(event)) return;
    if (this.activeTool === "draw") {
      this.emitDraw("start", event);
      this.canvas.setPointerCapture(event.pointerId);
      return;
    }
    this.lastPointerAdditive = event.shiftKey || event.metaKey;
    this.beginInputGesture();
    const screen = this.eventPoint(event);
    this.sendInputBatch([{ kind: "pointer-down", pointerId: event.pointerId, screen }]);
    this.canvas.setPointerCapture(event.pointerId);
  };

  private onPointerMove = (event: PointerEvent) => {
    if (isMousePointerEvent(event)) return;
    if (this.activeTool === "draw") {
      this.emitDraw("move", event);
      return;
    }
    this.sendInputBatch([{ kind: "pointer-move", pointerId: event.pointerId, screen: this.eventPoint(event) }]);
  };

  private onPointerUp = (event: PointerEvent) => {
    if (isMousePointerEvent(event)) return;
    if (this.activeTool === "draw") {
      this.emitDraw("end", event);
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
    try {
      this.canvas.releasePointerCapture(event.pointerId);
    } catch {
      // Pointer capture may already be released after cancellation.
    }
    this.finishInputGesture();
  };

  private onPointerCancel = (event: PointerEvent) => {
    if (isMousePointerEvent(event)) return;
    if (this.activeTool === "draw") {
      this.emitDraw("cancel", event);
      return;
    }
    this.sendInputBatch([{ kind: "pointer-cancel", pointerId: event.pointerId }]);
    this.objectDrag = null;
    this.finishInputGesture();
  };

  private onMouseDown = (event: MouseEvent) => {
    if (event.button !== 0) return;
    event.preventDefault();
    if (this.activeTool === "draw") {
      this.mouseDragActive = true;
      this.bindMouseFallbackMove();
      this.emitDraw("start", event);
      return;
    }
    this.lastPointerAdditive = event.shiftKey || event.metaKey;
    this.beginInputGesture();
    this.mouseDragActive = true;
    this.bindMouseFallbackMove();
    this.sendInputBatch([{ kind: "pointer-down", pointerId: MOUSE_POINTER_ID, screen: this.eventPoint(event) }]);
  };

  private onMouseMove = (event: MouseEvent) => {
    if (!this.mouseDragActive) return;
    event.preventDefault();
    if (this.activeTool === "draw") {
      this.emitDraw("move", event);
      return;
    }
    this.sendInputBatch([{ kind: "pointer-move", pointerId: MOUSE_POINTER_ID, screen: this.eventPoint(event) }]);
  };

  private onMouseUp = (event: MouseEvent) => {
    if (!this.mouseDragActive) return;
    event.preventDefault();
    this.mouseDragActive = false;
    this.unbindMouseFallbackMove();
    if (this.activeTool === "draw") {
      this.emitDraw("end", event);
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
    this.finishInputGesture();
  };

  private onDoubleClick = (event: MouseEvent) => {
    this.sendInputBatch([{ kind: "double-click", screen: this.eventPoint(event) }]);
  };

  private onWheel = (event: WheelEvent) => {
    event.preventDefault();
    this.sendInputBatch([{ kind: "wheel", screen: this.eventPoint(event), deltaY: event.deltaY }]);
    this.updateOverlayPosition();
  };

  private mountOverlay(request: DomOverlayRequest) {
    this.removeOverlay(false);
    const textarea = document.createElement("textarea");
    textarea.className = "renderer-edit-overlay";
    textarea.value = request.value;
    textarea.autocomplete = "off";
    textarea.spellcheck = true;
    textarea.addEventListener("keydown", (event) => {
      if (event.isComposing) return;
      if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
        event.preventDefault();
        this.commitTextEdit();
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        this.removeOverlay(false);
      }
    });
    textarea.addEventListener("blur", () => this.commitTextEdit());
    this.overlayRoot.append(textarea);
    this.activeOverlay = textarea;
    this.activeOverlayRequest = request;
    this.updateOverlayPosition();
    textarea.focus();
    textarea.select();
  }

  private removeOverlay(committed: boolean) {
    const overlay = this.activeOverlay;
    if (!overlay) return;
    this.activeOverlay = null;
    this.activeOverlayRequest = null;
    overlay.remove();
    this.onEvent({ type: "overlay", request: null });
    if (!committed) this.onEvent({ type: "status", message: "Text edit cancelled" });
  }

  private updateOverlayPosition() {
    if (!this.activeOverlay || !this.activeOverlayRequest) return;
    const request = this.requestOverlay(this.activeOverlayRequest.target.id, this.activeOverlayRequest.target.field);
    if (request) this.activeOverlayRequest = request;
    const screenRect = this.activeOverlayRequest.screenRect;
    const overlayStyle = this.activeOverlayRequest.style;
    this.activeOverlay.style.left = `${screenRect.x}px`;
    this.activeOverlay.style.top = `${screenRect.y}px`;
    this.activeOverlay.style.width = `${screenRect.width}px`;
    this.activeOverlay.style.height = `${screenRect.height}px`;
    this.activeOverlay.style.fontFamily = overlayStyle.fontFamily;
    this.activeOverlay.style.fontSize = `${overlayStyle.fontSize}px`;
    this.activeOverlay.style.fontWeight = `${overlayStyle.fontWeight}`;
    this.activeOverlay.style.lineHeight = `${overlayStyle.lineHeight}px`;
    this.activeOverlay.style.letterSpacing = `${overlayStyle.letterSpacing}px`;
    this.activeOverlay.style.padding = `${overlayStyle.paddingY}px ${overlayStyle.paddingX}px`;
    this.activeOverlay.style.color = overlayStyle.textColor;
    this.activeOverlay.style.background = overlayStyle.backgroundColor;
    this.activeOverlay.style.border = `${overlayStyle.borderWidth}px solid ${overlayStyle.borderColor}`;
    this.activeOverlay.style.borderRadius = `${overlayStyle.borderRadius}px`;
    this.activeOverlay.style.outline = `${overlayStyle.focusRingWidth}px solid ${overlayStyle.focusRingColor}`;
    this.activeOverlay.style.boxShadow = overlayStyle.boxShadow;
    this.activeOverlay.style.caretColor = overlayStyle.caretColor;
    this.activeOverlay.style.overflowX = overlayStyle.overflowX;
    this.activeOverlay.style.overflowY = overlayStyle.overflowY;
    this.activeOverlay.style.setProperty("accent-color", overlayStyle.accentColor);
    this.activeOverlay.style.setProperty("--renderer-edit-selection-bg", overlayStyle.selectionBackgroundColor);
    this.activeOverlay.dataset.overlayState = overlayStyle.state;
    this.activeOverlay.dataset.maxLines = String(overlayStyle.maxLines);
  }

  private eventPoint(event: MouseEvent | PointerEvent | WheelEvent): WorldPoint {
    const rect = this.canvas.getBoundingClientRect();
    return { x: event.clientX - rect.left, y: event.clientY - rect.top };
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

  private beginInputGesture() {
    if (!this.inputGestureActive) this.onEvent({ type: "gesture", active: true });
    this.inputGestureActive = true;
    if (this.deferredSceneRaf && typeof cancelAnimationFrame === "function") cancelAnimationFrame(this.deferredSceneRaf);
    this.deferredSceneRaf = 0;
  }

  private finishInputGesture() {
    if (this.inputGestureActive) this.onEvent({ type: "gesture", active: false });
    this.inputGestureActive = false;
    this.flushDeferredSceneSoon();
  }

  private flushDeferredSceneSoon() {
    if (!this.deferredScene || this.deferredSceneRaf) return;
    const flush = () => {
      this.deferredSceneRaf = 0;
      if (this.inputGestureActive || !this.deferredScene) return;
      this.loadSceneNow(this.deferredScene);
    };
    if (typeof requestAnimationFrame === "function") {
      this.deferredSceneRaf = requestAnimationFrame(flush);
    } else {
      flush();
    }
  }

  private emptyStats(start: number): FrameStats {
    return {
      frameMs: performance.now() - start,
      renderMs: performance.now() - start,
      totalGroups: 0,
      totalCards: 0,
      totalEdges: 0,
      visibleGroups: 0,
      visibleCards: 0,
      visibleEdges: 0,
      cacheHits: 0,
      cacheMisses: 0,
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
      rustCameraX: this.lastDebugSnapshot?.camera.x ?? null,
      rustCameraY: this.lastDebugSnapshot?.camera.y ?? null,
      rustCameraZoom: this.lastDebugSnapshot?.camera.zoom ?? null,
      rustSelectionKind: this.lastDebugSnapshot?.selection.kind ?? null,
      rustSelectionId: debugSelectionId(this.lastDebugSnapshot?.selection ?? null),
      rustLastHitKind: this.lastDebugSnapshot?.lastHit?.kind ?? null,
      rustLastHitId: this.lastDebugSnapshot?.lastHit?.id ?? null,
      rustLastHitField: this.lastDebugSnapshot?.lastHit?.field ?? null,
      rustLastHitPort: this.lastDebugSnapshot?.lastHit?.port ?? null,
      rustLastHitScreenX: this.lastDebugSnapshot?.lastHit?.screenX ?? null,
      rustLastHitScreenY: this.lastDebugSnapshot?.lastHit?.screenY ?? null
    };
  }

  private syncWebGpuScene(snapshot: SceneSnapshot) {
    if (!this.webGpuRenderer) return;
    this.webGpuRenderer.loadScene(JSON.stringify(snapshot));
    this.camera = snapshot.camera;
    this.rustBoundaryCalls += 1;
    this.updateDebugSnapshot();
  }

  private applyPatchBatchInRust(patches: ScenePatch[]): string[] {
    if (!this.webGpuRenderer) return ["WebGPU renderer unavailable"];
    try {
      this.webGpuRenderer.applyPatchBatch(JSON.stringify(patches));
      this.rustBoundaryCalls += 1;
      this.updateDebugSnapshot();
      return [];
    } catch (error) {
      const message = error instanceof Error ? `Rust patch batch failed: ${error.message}` : "Rust patch batch failed";
      this.onEvent({ type: "status", message });
      return [message];
    }
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
      this.updateOverlayPosition();
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
    const hit = rustHitToEngineHit(result.hit);
    if (result.patches.some((patch) => patch.kind === "select")) {
      this.onEvent({ type: "selection", hit, additive: this.lastPointerAdditive });
    }
    this.mirrorAcceptedPatches(result.patches, true);
    if (result.overlay) {
      this.mountOverlay(result.overlay);
      this.onEvent({ type: "overlay", request: result.overlay });
    }
    // CC2.3: a marquee result rides the pointer-up that ends the drag. Forward
    // the intersected ids so the shell merges them into multiSelectIds.
    const marquee = marqueeFromResult(result);
    if (marquee) this.onEvent({ type: "marquee", rect: marquee.rect, ids: marquee.ids });

    // FC-08: object-path input results. An object pick on pointer-down starts a
    // remembered drag; each move delta updates it and previews; an empty-start
    // marquee forwards its ids. The commit op is authored on pointer-up.
    if (typeof result.objectSelection === "string") {
      this.objectDrag = { id: result.objectSelection, dx: 0, dy: 0 };
      this.onEvent({ type: "object-select", id: result.objectSelection });
    }
    if (result.objectTransformDelta) {
      const { id, dx, dy } = result.objectTransformDelta;
      this.objectDrag = { id, dx, dy };
      this.onEvent({ type: "object-transform-preview", id, dx, dy });
    }
    if (result.objectMarqueeIds != null) {
      this.onEvent({ type: "object-marquee", ids: result.objectMarqueeIds });
    }
  }

  // FC-08: emit the single undoable transform commit when an object drag moved,
  // then clear the remembered drag. Called on pointer-up/mouse-up AFTER the batch.
  private commitObjectDrag() {
    const drag = this.objectDrag;
    this.objectDrag = null;
    if (drag && (drag.dx !== 0 || drag.dy !== 0)) {
      this.onEvent({ type: "object-transform-commit", id: drag.id, dx: drag.dx, dy: drag.dy });
    }
  }

  // FC-11: emit a draw phase with the world point under the cursor. Used by the
  // pointer/mouse handlers while the draw tool is active, replacing the renderer
  // select/marquee input path.
  private emitDraw(phase: "start" | "move" | "end" | "cancel", event: MouseEvent | PointerEvent) {
    const world = screenToWorld(this.eventPoint(event), this.camera);
    this.onEvent({ type: "draw", phase, world });
  }

  // FC-08: pure object pick for the shell's right-click context menu.
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

  private mirrorAcceptedPatches(patches: ScenePatch[], emitPatchEvents: boolean) {
    if (!this.snapshot) return;
    for (const patch of patches) {
      this.snapshot = applyScenePatch(this.snapshot, patch);
      if (emitPatchEvents && patch.kind !== "select") this.onEvent({ type: "patch", patch, errors: [] });
    }
  }

  private requestOverlay(id: string, field: "title" | "summary" | "detail"): DomOverlayRequest | null {
    if (!this.webGpuRenderer) return null;
    try {
      const request = this.webGpuRenderer.overlayRequest(id, field);
      this.rustBoundaryCalls += 1;
      return request;
    } catch (error) {
      this.onEvent({
        type: "status",
        message: error instanceof Error ? `Rust overlay request failed: ${error.message}` : "Rust overlay request failed"
      });
      return null;
    }
  }

  private updateDebugSnapshot(): RustDebugSnapshot | null {
    if (!this.webGpuRenderer) return null;
    try {
      this.lastDebugSnapshot = this.webGpuRenderer.debugSnapshot();
      this.camera = this.lastDebugSnapshot.camera;
      this.rustBoundaryCalls += 1;
      return this.lastDebugSnapshot;
    } catch (error) {
      this.onEvent({
        type: "status",
        message: error instanceof Error ? `Rust debug snapshot failed: ${error.message}` : "Rust debug snapshot failed"
      });
      return null;
    }
  }
}

// CC2.3: read the marquee result defensively — a wasm build that predates the
// field returns it as undefined, which must behave like "no marquee".
function marqueeFromResult(result: RustInputBatchResult): RustMarqueeResult | null {
  const marquee = result.marquee;
  if (!marquee) return null;
  return { rect: marquee.rect, ids: marquee.ids };
}

function rustHitToEngineHit(hit: RustHitResult | null): HitResult | null {
  if (!hit) return null;
  const world = { x: hit.worldX, y: hit.worldY };
  const screen = { x: hit.screenX, y: hit.screenY };
  if (hit.kind === "port" && (hit.port === "source" || hit.port === "target")) {
    return { kind: "port", id: hit.id, groupId: hit.groupId ?? undefined, port: hit.port, world, screen };
  }
  if (hit.kind === "text" && (hit.field === "title" || hit.field === "summary" || hit.field === "detail")) {
    return { kind: "text", id: hit.id, groupId: hit.groupId ?? undefined, field: hit.field, world, screen };
  }
  if (hit.kind === "card" || hit.kind === "edge" || hit.kind === "group") {
    return { kind: hit.kind, id: hit.id, groupId: hit.groupId ?? undefined, world, screen };
  }
  return null;
}

function debugSelectionId(selection: SceneSelection | null): string | null {
  if (!selection || selection.kind === "canvas") return null;
  // The renderer/core only ever receives the single-anchor form; a `multi`
  // selection is down-projected before it reaches the engine, but handle it
  // defensively by reporting its primary id.
  if (selection.kind === "multi") return selection.ids[0] ?? null;
  return selection.id;
}

function selectionEqual(left: SceneSelection, right: SceneSelection): boolean {
  if (left.kind !== right.kind) return false;
  switch (left.kind) {
    case "canvas":
      return true;
    case "group":
      return right.kind === "group" && left.id === right.id;
    case "node":
      return right.kind === "node" && left.id === right.id;
    case "edge":
      return right.kind === "edge" && left.id === right.id;
    case "multi":
      return (
        right.kind === "multi" &&
        left.ids.length === right.ids.length &&
        left.ids.every((id, index) => id === right.ids[index])
      );
  }
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
