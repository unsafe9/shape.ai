import {
  applyScenePatch,
  screenToWorld,
  type CameraState,
  type DomOverlayRequest,
  type FrameStats,
  type HitResult,
  type RenderTransform3x3,
  type ScenePatch,
  type SceneSelection,
  type SceneSnapshot,
  type WorldPoint,
  type WorldRect
} from "./scene";
import type {
  HoverAffordance,
  RustCanvasInputEvent,
  RustDebugSnapshot,
  RustHitResult,
  RustInputBatchResult,
  RustMarqueeResult,
  RustWebGpuFrameStats,
  RustWebGpuRenderer
} from "./wasmLoader";
import {
  COARSE_ROTATE_SNAP_DEG,
  isAdditiveSelect,
  isCoarseRotate,
  isPanGesture,
  isPartialErase,
  isSnapBypass
} from "../lib/gestureBindings";
import { detectMac } from "../lib/shortcuts";

/** W2-03: the active pointer tool. One unified "select" Move/Select pointer
 *  (picks/drags/marquees), "draw" (freehand capture, FC-11), "create" (W2-07
 *  drag-to-create shapes), and "erase" (W2-08 whole/partial stroke eraser). Pan
 *  is no longer a separate tool — it rides Space-hold/middle-button/wheel (see
 *  {@link isPanIntent}). */
export type ActiveTool = "select" | "draw" | "create" | "erase";

// W2-03: a pointer-down is a pan gesture (not a pick/marquee) when the Space key
// is held OR the middle mouse button is used. EN1: the binding (Space / middle
// button) is the C2 `pan-space`/`pan-middle` gesture, routed through the single
// source in gestureBindings (no magic literal here). Pure so the shell test can
// pin the classification without a renderer.
export function isPanIntent(intent: { spaceHeld: boolean; button: number }): boolean {
  return isPanGesture(intent);
}

// W2-04/W2-05: the gesture that produced a transform delta. Mirrors the Rust
// ObjectTransformDelta.kind; the shell uses it to drive the commit op kind/status.
export type TransformKind = "translate" | "resize" | "rotate";

// W2-07: whether a shape drag-create phase should run the outline snap query. Snap
// is bypassed when the snap-bypass gesture (C2 `no-snap-alt`: Alt held) is active
// — routed through the single source in gestureBindings — or on the terminal
// `cancel` phase (no preview to snap). Pure so the shell test can pin the decision
// without a renderer.
export function shouldQuerySnap(intent: { altHeld: boolean; phase: "start" | "move" | "end" | "cancel" }): boolean {
  return !isSnapBypass({ altKey: intent.altHeld }) && intent.phase !== "cancel";
}

// W3-G9 (#3): classify a create-tool pointer move while the create tool is armed.
// A move with NO drag in progress is a HOVER probe (it emits a persistent anchor-
// ring snap before any drag, request #3); a move during a drag is the regular
// rubber-band drag move. Pure so the shell test can pin the classification without
// a renderer — falsifiable: it returns "hover" for a button-up move (it would
// return "drag" if the old drag-gated behavior leaked back in).
export function createMoveEmission(intent: { dragActive: boolean }): "hover" | "drag" {
  return intent.dragActive ? "drag" : "hover";
}

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
  // object drag (a non-destructive preview the shell composes onto the scene); it
  // is emitted once on pointer-up when the drag moved, and is the single undoable
  // op; `object-marquee` rides the pointer-up of an empty-start drag.
  // W2-04/W2-05: the transform is the full cumulative world-space delta matrix
  // (row-major, PRE-multiplied onto the object's transform) plus the gesture
  // `kind`, generalizing the FC-08 translate-only path to resize/rotate.
  // W2-03: `additive` carries the shift/meta held at pick time so the shell can
  // toggle the object in/out of the multi-select set instead of replacing it.
  | { type: "object-select"; id: string; additive: boolean }
  | { type: "object-transform-preview"; id: string; matrix: RenderTransform3x3; kind: TransformKind }
  | { type: "object-transform-commit"; id: string; matrix: RenderTransform3x3; kind: TransformKind }
  | { type: "object-marquee"; ids: string[] }
  // RA2b: a double-click landed on an object. The shell drills into a container
  // (hasChildren) or enters inline text edit on a leaf. Missed double-clicks emit
  // nothing (the core returns null), so this event only rides a real object hit.
  | { type: "object-double-click"; id: string; hasChildren: boolean }
  // FC-11: freehand pen capture. While the draw tool is active, pointer/mouse
  // down/move/up emit draw phases instead of the select/marquee path; the shell
  // accumulates the world points and commits the stroke to an object on `end`.
  | { type: "draw"; phase: "start" | "move" | "end" | "cancel"; world: WorldPoint }
  // W2-07: drag-to-create shapes. While the create tool is active, pointer/mouse
  // down-drag-up rubber-band a bbox; the shell renders a transient preview and
  // commits a sized primitive on `end`. `world` is the pointer in world space,
  // already snapped to the nearest object outline anchor when within tolerance
  // (`snapped` true) unless the snap-bypass modifier (Alt) was held. AP5 (#14):
  // `targetId` is the object whose outline the corner snapped to (null when not
  // snapped), so the shell can author a persistent anchor binding the endpoint.
  | { type: "create"; phase: "start" | "move" | "end" | "cancel"; world: WorldPoint; snapped: boolean; targetId: string | null }
  // W3-G9 (#3): a HOVER snap probe emitted while the create tool is armed and no
  // button is down — a bare mouse/pen hover over an existing object's edge. The
  // shell renders a PERSISTENT anchor ring from it (before any drag) so the user
  // sees where the next create would anchor. Distinct from the drag "create" event
  // (which only rides an active rubber-band). `snapped`/`targetId` mirror "create":
  // the snapped WORLD outline point + the object whose edge it landed on (null when
  // not snapped, so the shell clears the ring). Honors the Alt snap-bypass upstream.
  | { type: "create-hover"; world: WorldPoint; snapped: boolean; targetId: string | null }
  // W2-08: eraser. While the erase tool is active, a pointer/mouse down/move over a
  // stroke emits an erase touch carrying the hit object id + the touch in world
  // space, plus whether the partial-erase modifier (Alt) was held (default = whole-
  // stroke delete, modifier = partial subpath cut). The id comes from the core's
  // object hit-test, so hit-test stays in the core (boundary); the shell authors
  // the delete/edit-geometry op.
  | { type: "erase"; id: string; world: WorldPoint; partial: boolean }
  // W2-03: the hover affordance under the cursor (from result.hoverAffordance on a
  // no-drag pointer move). The shell maps it to a CSS cursor.
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

// W2-07: screen-pixel snap radius for shape drag-create. Passed to the W2-06 core
// query (nearestOutlinePoint), which converts it to world via the camera zoom.
const CREATE_SNAP_TOLERANCE_PX = 8;

// W3-G6 (#6): the transient preview regions buildFeedScene appends to the renderer
// feed (the create rubber-band + its snap-indicator + the pen-stroke preview). The
// create-drag snap query MUST exclude these, or the preview corner sitting under
// the cursor self-snaps at distance ~0 and occludes every real object's edge.
const SNAP_EXCLUDE_IDS_JSON = JSON.stringify(["create-preview", "create-snap-indicator", "draw-preview"]);

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
  // FC-08/W2-05: the in-progress object drag (id + cumulative world-space delta
  // matrix + gesture kind). Set on the pointer-down that picks an object, updated on
  // each move, and committed once on pointer-up when the matrix moved off identity.
  // The renderer never mutates object transforms.
  private objectDrag: { id: string; matrix: RenderTransform3x3; kind: TransformKind } | null = null;
  // W2-03: whether Space is currently held (pushed from the shell). A pointer-down
  // while Space is held — or a middle-button drag — is a pan gesture: the engine
  // arms the core's hand-pan path for the gesture, then restores the user's tool on
  // up. The pan state machine itself stays in the Rust core (boundary).
  private spaceHeld = false;
  private panGestureActive = false;
  // W2-08: true while the erase tool's pointer is held down, so a move keeps
  // erasing along the drag (a bare hover never erases). Set on erase down, cleared
  // on up/cancel.
  private eraseDragActive = false;
  // W3-G9 (#3): true while a create drag is in progress (between create start and
  // up/cancel). A pen/touch onPointerMove with this false is a bare hover, so it
  // emits a create-hover snap probe instead of a drag create event. Set on create
  // start, cleared on up/cancel.
  private createDragActive = false;
  // W3-G9 (#3): the always-on hover mousemove target bound while the create tool is
  // armed (mouse has no down-less move otherwise). Bound in setTool on entering
  // create, unbound on leaving. Distinct from the mousedown-bound drag-move target,
  // so a bare mouse hover emits a create-hover probe without disturbing the drag.
  private createHoverTarget: EventTarget | null = null;
  // EN1 (#3): the previous eraser sample's screen point, so each move runs RA3's
  // swept hit-test over the segment (prev -> curr) and erases every crossed object
  // — a fast drag that skips between samples still erases what the segment passes
  // through. Set on erase down, advanced each move, cleared on up/cancel.
  private lastEraseScreen: WorldPoint | null = null;
  // EN1 (#2): Shift held at the latest pointer/mouse event, mirroring the C2
  // coarse-rotate gesture (`coarse-rotate-shift`). When a rotate transform delta
  // arrives with Shift held, the engine re-snaps it to the catalog step (15°).
  private shiftHeld = false;

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

  // W2-03/W2-07/W2-08: set the active pointer tool ("select" | "draw" | "create" |
  // "erase"). The renderer-core only knows select/hand; "draw"/"create"/"erase" are
  // shell-side routing (their input is intercepted by the engine before it reaches
  // the renderer), so the core stays in "select".
  setTool(tool: ActiveTool) {
    this.activeTool = tool;
    this.coreSetTool("select");
    // W3-G9 (#3): a bare mouse hover has no down-less move under the canvas, so the
    // create tool arms an always-on hover mousemove to drive the persistent anchor
    // ring; leaving create unbinds it (and clears the stale drag flag).
    if (tool === "create") {
      this.bindCreateHoverMove();
    } else {
      this.unbindCreateHoverMove();
      this.createDragActive = false;
    }
  }

  // W2-03: the shell mirrors the Space key down/up here. A pointer-down while
  // Space is held becomes a pan gesture instead of a pick/marquee.
  setSpaceHeld(held: boolean) {
    this.spaceHeld = held;
  }

  // W2-03: arm the core's hand-pan path for the duration of one pan gesture
  // (Space-hold + left-drag, or a middle-button drag). The core owns the pan state
  // machine; the engine only flips the core tool to "hand" for the gesture and
  // restores the user's tool ("select"/"draw") on pointer-up.
  private armPanGesture() {
    this.panGestureActive = true;
    this.coreSetTool("hand");
  }

  private disarmPanGesture() {
    if (!this.panGestureActive) return;
    this.panGestureActive = false;
    // "draw"/"create" are shell-side routing tools; the core only knows
    // select/hand, so restore the core to "select" for any non-pan tool.
    this.coreSetTool("select");
  }

  // Push a raw core tool string (select|hand) without touching the shell-facing
  // activeTool. Used only by the transient pan gesture.
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
    this.unbindCreateHoverMove();
    this.canvas.removeEventListener("dblclick", this.onDoubleClick);
    this.canvas.removeEventListener("wheel", this.onWheel);
  }

  private onPointerDown = (event: PointerEvent) => {
    if (isMousePointerEvent(event)) return;
    this.shiftHeld = event.shiftKey;
    // W2-03: Space-hold pans even under the draw tool; arm the core pan path first.
    const pan = isPanIntent({ spaceHeld: this.spaceHeld, button: event.button });
    if (this.activeTool === "draw" && !pan) {
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
    // EN1: additive-select (C2 `additive-select-shift`/`-mod`) — Shift or the
    // platform primary modifier (Cmd/Ctrl) held at pick time, via the single source.
    this.lastPointerAdditive = isAdditiveSelect(event, detectMac());
    this.beginInputGesture();
    const screen = this.eventPoint(event);
    this.sendInputBatch([{ kind: "pointer-down", pointerId: event.pointerId, screen }]);
    this.canvas.setPointerCapture(event.pointerId);
  };

  private onPointerMove = (event: PointerEvent) => {
    if (isMousePointerEvent(event)) return;
    this.shiftHeld = event.shiftKey;
    // W2-03: a Space-armed pan stays on the pan path for the whole gesture, even
    // under the draw tool, so the move feeds the core pan instead of the stroke.
    if (this.activeTool === "draw" && !this.panGestureActive) {
      this.emitDraw("move", event);
      return;
    }
    if (this.activeTool === "create" && !this.panGestureActive) {
      // W3-G9 (#3): a pen/touch move with no create drag in progress is a bare hover
      // — emit a persistent anchor-ring snap probe instead of a drag create event.
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
    this.disarmPanGesture();
    try {
      this.canvas.releasePointerCapture(event.pointerId);
    } catch {
      // Pointer capture may already be released after cancellation.
    }
    this.finishInputGesture();
  };

  private onPointerCancel = (event: PointerEvent) => {
    if (isMousePointerEvent(event)) return;
    if (this.activeTool === "draw" && !this.panGestureActive) {
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
    this.disarmPanGesture();
    this.finishInputGesture();
  };

  private onMouseDown = (event: MouseEvent) => {
    this.shiftHeld = event.shiftKey;
    // W2-03: left (0) drives select/draw; middle (1) is a pan gesture. Right (2)
    // is the context menu (handled in the shell) — ignore it here.
    const pan = isPanIntent({ spaceHeld: this.spaceHeld, button: event.button });
    if (event.button !== 0 && !pan) return;
    event.preventDefault();
    if (this.activeTool === "draw" && !pan) {
      this.mouseDragActive = true;
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
    // EN1: additive-select (C2 `additive-select-shift`/`-mod`) — Shift or the
    // platform primary modifier (Cmd/Ctrl) held at pick time, via the single source.
    this.lastPointerAdditive = isAdditiveSelect(event, detectMac());
    this.beginInputGesture();
    this.mouseDragActive = true;
    this.bindMouseFallbackMove();
    this.sendInputBatch([{ kind: "pointer-down", pointerId: MOUSE_POINTER_ID, screen: this.eventPoint(event) }]);
  };

  private onMouseMove = (event: MouseEvent) => {
    if (!this.mouseDragActive) return;
    event.preventDefault();
    this.shiftHeld = event.shiftKey;
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
    this.disarmPanGesture();
    this.finishInputGesture();
  };

  // W3-G9 (#3): the always-on create-tool hover mousemove. A bare mouse hover (no
  // button down) over an object's edge emits a create-hover snap probe so the
  // persistent anchor ring tracks the cursor BEFORE any drag. Skipped while a drag
  // is in progress (mouseDragActive) so it never fights the mousedown-bound drag
  // move, and while a Space-armed pan rides the create tool.
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

  // W3-G9 (#3): bind/unbind the always-on create-tool hover mousemove on the canvas
  // (a bare mouse hover, no button down, has no other move source). Idempotent;
  // bound when entering create in setTool, unbound on leave + on stop().
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
      this.objectDrag = { id: result.objectSelection, matrix: IDENTITY_MATRIX, kind: "translate" };
      this.onEvent({ type: "object-select", id: result.objectSelection, additive: this.lastPointerAdditive });
    }
    if (result.objectTransformDelta) {
      // W2-05: the cumulative delta is a full world-space matrix + gesture kind
      // (translate/resize/rotate). Remember it and emit a non-destructive preview;
      // the shell composes the matrix onto the object and commits on pointer-up.
      const { id, kind } = result.objectTransformDelta;
      // EN1 (#2): the C2 coarse-rotate gesture (`coarse-rotate-shift`) — Shift held
      // during a rotate quantizes the sweep to the catalog step (15°). Applied
      // shell-side to the returned rotate matrix (RA2c semantics): extract the swept
      // angle + center, re-snap, rebuild. Non-rotate deltas and a released Shift pass
      // through unchanged.
      const matrix =
        kind === "rotate" && isCoarseRotate({ shiftKey: this.shiftHeld })
          ? snapRotateDeltaMatrix(result.objectTransformDelta.matrix, COARSE_ROTATE_SNAP_DEG)
          : result.objectTransformDelta.matrix;
      this.objectDrag = { id, matrix, kind };
      // W2-11 drag zero-rebake: push the cumulative delta straight to the GPU
      // instance matrix (no Svelte round-trip, lowest latency, no re-tessellation).
      // The preview event still rides through so the shell does its commit/snap-back
      // bookkeeping; it no longer drives a full feedScene rebuild.
      this.webGpuRenderer?.setObjectPreviewTransform?.(id, JSON.stringify(matrix));
      this.onEvent({ type: "object-transform-preview", id, matrix, kind });
    }
    if (result.objectMarqueeIds != null) {
      this.onEvent({ type: "object-marquee", ids: result.objectMarqueeIds });
    }
    // RA2b: a double-click that hit an object drills in (container) or edits a leaf;
    // a missed double-click is null and emits nothing.
    if (result.objectDoubleClick) {
      const { id, hasChildren } = result.objectDoubleClick;
      this.onEvent({ type: "object-double-click", id, hasChildren });
    }
    // W2-03: surface the hover affordance so the shell can set the cursor. The core
    // computes it per no-drag move; older wasm builds omit it (defaults to "empty").
    this.onEvent({ type: "affordance", affordance: result.hoverAffordance ?? "empty" });
  }

  // FC-08/W2-05: emit the single undoable transform commit when an object drag
  // moved (the cumulative matrix is off identity), then clear the remembered drag.
  // Called on pointer-up/mouse-up AFTER the batch.
  private commitObjectDrag() {
    const drag = this.objectDrag;
    this.objectDrag = null;
    if (drag && !isIdentityMatrix(drag.matrix)) {
      this.onEvent({ type: "object-transform-commit", id: drag.id, matrix: drag.matrix, kind: drag.kind });
    }
  }

  // FC-11: emit a draw phase with the world point under the cursor. Used by the
  // pointer/mouse handlers while the draw tool is active, replacing the renderer
  // select/marquee input path.
  private emitDraw(phase: "start" | "move" | "end" | "cancel", event: MouseEvent | PointerEvent) {
    const world = screenToWorld(this.eventPoint(event), this.camera);
    this.onEvent({ type: "draw", phase, world });
  }

  // W2-07: emit a create phase with the dragged corner in world space. During the
  // drag the corner snaps to the nearest object outline anchor when within
  // tolerance, unless the snap-bypass modifier (Alt) is held (request 4: "modifier
  // nullifies snap"). The snap query is the W2-06 core path (nearestOutlinePoint),
  // so the geometry truth stays in Rust (P1) — the shell only forwards the result.
  private emitCreate(phase: "start" | "move" | "end" | "cancel", event: MouseEvent | PointerEvent) {
    const raw = screenToWorld(this.eventPoint(event), this.camera);
    const snap = shouldQuerySnap({ altHeld: event.altKey, phase }) ? this.querySnap(raw) : null;
    const world = snap ? { x: snap.x, y: snap.y } : raw;
    this.onEvent({ type: "create", phase, world, snapped: snap !== null, targetId: snap?.targetId ?? null });
  }

  // W3-G9 (#3): emit a HOVER snap probe for a bare create-tool move (no button
  // down). Runs the same W2-06 outline snap query as the drag move (it already
  // excludes the transient preview ids), so the persistent anchor ring shows where
  // the next create would anchor. Honors the Alt snap-bypass (no ring when an
  // Alt-create would author no anchor), mirroring emitCreate's `shouldQuerySnap`
  // gate — `phase: "move"` so a held Alt suppresses the query, never the position.
  private emitCreateHover(event: MouseEvent | PointerEvent) {
    const raw = screenToWorld(this.eventPoint(event), this.camera);
    const snap = shouldQuerySnap({ altHeld: event.altKey, phase: "move" }) ? this.querySnap(raw) : null;
    const world = snap ? { x: snap.x, y: snap.y } : raw;
    this.onEvent({ type: "create-hover", world, snapped: snap !== null, targetId: snap?.targetId ?? null });
  }

  // W2-08: emit an erase touch for the stroke under the cursor. The object id
  // comes from the core's object hit-test (hit-test stays in the core, boundary);
  // a touch over empty canvas (no hit) emits nothing. `partial` is the C2
  // partial-erase gesture (`partial-erase-alt`: Alt held — whole-stroke delete by
  // default, partial subpath cut with the modifier), routed through the single
  // gesture source. The shell authors the delete / edit-geometry op from the event.
  private emitErase(event: MouseEvent | PointerEvent) {
    const screen = this.eventPoint(event);
    const id = this.objectHitTest(screen);
    if (!id) return;
    const world = screenToWorld(screen, this.camera);
    this.onEvent({ type: "erase", id, world, partial: isPartialErase(event) });
  }

  // EN1 (#3): emit one erase touch for EVERY object the eraser crossed since the
  // previous sample. RA3's swept hit-test (`sweptEraseAt`) returns each object the
  // segment (prev -> curr SCREEN samples) passes through, so a fast drag that skips
  // between samples still erases the whole swept path — not just the object under
  // the latest sample. Falls back to the single-sample `emitErase` when no prior
  // sample exists or the swept method is unavailable (older wasm build). The world
  // point + `partial` mirror `emitErase`.
  private emitSweptErase(event: MouseEvent | PointerEvent) {
    const curr = this.eventPoint(event);
    const prev = this.lastEraseScreen;
    this.lastEraseScreen = curr;
    const ids = prev ? this.sweptEraseHitTest(prev, curr) : null;
    if (!ids) {
      this.emitErase(event);
      return;
    }
    const world = screenToWorld(curr, this.camera);
    const partial = isPartialErase(event);
    for (const id of ids) this.onEvent({ type: "erase", id, world, partial });
  }

  // EN1 (#3): RA3 swept hit-test boundary — the ids of every object crossed by the
  // eraser between two consecutive SCREEN samples. Feature-detected: a wasm build
  // predating `sweptEraseAt` returns null so the caller falls back to the
  // single-sample pick.
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

  // W2-07: snap a world point to the nearest object outline anchor via the W2-06
  // core query. Returns the snapped WORLD point plus the target object id (AP5
  // #14: the object whose outline was snapped to) when within tolerance, else
  // null. Feature-detected: a wasm build predating the method never snaps.
  private querySnap(world: WorldPoint): { x: number; y: number; targetId: string | null } | null {
    const renderer = this.webGpuRenderer;
    if (!renderer || typeof renderer.nearestOutlinePoint !== "function") return null;
    try {
      const result = renderer.nearestOutlinePoint(world.x, world.y, CREATE_SNAP_TOLERANCE_PX, this.camera.zoom, SNAP_EXCLUDE_IDS_JSON);
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

// W2-05: the row-major identity transform and an exact-equality check, used to
// seed the remembered object drag and to detect a no-op (un-moved) drag so the
// commit op is skipped — matching the FC-08 dx===0 && dy===0 predicate.
const IDENTITY_MATRIX: RenderTransform3x3 = [
  [1, 0, 0],
  [0, 1, 0],
  [0, 0, 1]
];

function isIdentityMatrix(m: RenderTransform3x3): boolean {
  return m.every((row, i) => row.every((v, j) => v === IDENTITY_MATRIX[i][j]));
}

// EN1 (#2) coarse-rotate: re-quantize a core-returned rotate-delta matrix to the
// nearest `snapDeg`-degree step, mirroring RA2c's `rotate_delta_matrix_snapped`
// shell-side. The core builds the delta as `rotate_about_3x3(theta, cx, cy)` =
// `[[c,-s, cx-c*cx+s*cy],[s,c, cy-s*cx-c*cy],[0,0,1]]`, so the swept angle is
// `theta = atan2(s, c)` and the center solves `(I - R) c = t` (det = 2(1-c), the
// rotation part is recovered from the rotated translation column). A zero-angle
// delta (nothing to snap) and a singular `I - R` (theta == 0) both return the
// matrix unchanged. Pure; the GEOMETRY truth (sin/cos) stays standard math.
export function snapRotateDeltaMatrix(m: RenderTransform3x3, snapDeg: number): RenderTransform3x3 {
  const cos = m[0][0];
  const sin = m[1][0];
  const theta = Math.atan2(sin, cos);
  const step = (snapDeg * Math.PI) / 180;
  const snapped = Math.round(theta / step) * step;
  // (I - R) c = t, with R = [[cos,-sin],[sin,cos]] and t the translation column.
  // det(I - R) = (1-cos)^2 + sin^2 = 2(1-cos); zero only at theta == 0.
  const det = 2 * (1 - cos);
  if (Math.abs(det) < 1e-12) return m;
  const tx = m[0][2];
  const ty = m[1][2];
  // c = (I - R)^-1 t; (I - R) = [[1-cos, sin],[-sin, 1-cos]].
  const cx = ((1 - cos) * tx - sin * ty) / det;
  const cy = (sin * tx + (1 - cos) * ty) / det;
  const sc = Math.sin(snapped);
  const cc = Math.cos(snapped);
  return [
    [cc, -sc, cx - cc * cx + sc * cy],
    [sc, cc, cy - sc * cx - cc * cy],
    [0, 0, 1]
  ];
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
