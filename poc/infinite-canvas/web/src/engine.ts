import {
  applyScenePatch,
  clamp,
  fitCameraToBounds,
  sceneWorldBounds,
  screenToWorld,
  validateScenePatch,
  worldRectToScreen,
  type CameraState,
  type DomOverlayRequest,
  type FrameStats,
  type HitResult,
  type RenderCard,
  type ScenePatch,
  type SceneSelection,
  type SceneSnapshot,
  type WorldPoint,
  type WorldRect
} from "./scene";
import type { RustHitResult, RustWebGpuFrameStats, RustWebGpuRenderer } from "./wasmLoader";

export type EngineEvent =
  | { type: "stats"; stats: FrameStats }
  | { type: "selection"; hit: HitResult | null }
  | { type: "patch"; patch: ScenePatch; errors: string[] }
  | { type: "overlay"; request: DomOverlayRequest | null }
  | { type: "status"; message: string };

export type ShapeCanvasEngineOptions = {
  canvas: HTMLCanvasElement;
  overlayRoot: HTMLElement;
  backend: string;
  webGpuRenderer?: RustWebGpuRenderer | null;
  onEvent: (event: EngineEvent) => void;
};

type DragState =
  | { kind: "pan"; pointerId: number; start: WorldPoint; camera: CameraState }
  | { kind: "group"; pointerId: number; groupId: string; start: WorldPoint }
  | { kind: "card"; pointerId: number; cardId: string; start: WorldPoint; startBounds: WorldRect }
  | { kind: "edge"; pointerId: number; sourceId: string; current: WorldPoint };

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
  private drag: DragState | null = null;
  private selected: HitResult | null = null;
  private activeOverlay: HTMLTextAreaElement | null = null;
  private activeOverlayRequest: DomOverlayRequest | null = null;
  private inputBatchSize = 0;
  private boundaryCalls = 0;
  private rustBoundaryCalls = 0;
  private rustCameraFlushes = 0;
  private cameraDirty = false;
  private lastWebGpuFrame: RustWebGpuFrameStats | null = null;
  private backend: string;
  private webGpuRenderer: RustWebGpuRenderer | null;
  private webGpuUnavailableNotified = false;

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
    this.snapshot = snapshot;
    this.camera = snapshot.camera;
    this.syncWebGpuScene(snapshot);
    this.boundaryCalls += 1;
    this.renderFrame(performance.now());
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
    cancelAnimationFrame(this.raf);
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
    if (!this.snapshot) return;
    this.camera = fitCameraToBounds(sceneWorldBounds(this.snapshot), this.width, this.height);
    this.queueWebGpuCamera();
    this.boundaryCalls += 1;
    this.updateOverlayPosition();
  }

  setCamera(camera: CameraState) {
    this.camera = { ...camera, zoom: clamp(camera.zoom, 0.025, 2.8) };
    this.queueWebGpuCamera();
    this.boundaryCalls += 1;
    this.updateOverlayPosition();
  }

  getCamera(): CameraState {
    return this.camera;
  }

  getSnapshot(): SceneSnapshot | null {
    return this.snapshot;
  }

  applyPatch(patch: ScenePatch): string[] {
    if (!this.snapshot) return ["No scene loaded"];
    const errors = validateScenePatch(this.snapshot, patch);
    if (errors.length === 0) {
      this.snapshot = applyScenePatch(this.snapshot, patch);
      this.syncWebGpuPatch(patch, this.snapshot);
      this.boundaryCalls += 1;
    }
    this.onEvent({ type: "patch", patch, errors });
    return errors;
  }

  hitTest(screen: WorldPoint): HitResult | null {
    if (!this.webGpuRenderer) return null;
    try {
      this.flushWebGpuCamera();
      const hit = rustHitToEngineHit(this.webGpuRenderer.hitTest(screen.x, screen.y), screen);
      this.rustBoundaryCalls += 1;
      return hit;
    } catch (error) {
      this.onEvent({
        type: "status",
        message: error instanceof Error ? `WebGPU hit test failed: ${error.message}` : "WebGPU hit test failed"
      });
      return null;
    }
  }

  beginTextEdit(hit: HitResult): DomOverlayRequest | null {
    if (!this.snapshot || hit.kind !== "text") return null;
    const card = this.snapshot.cards.find((candidate) => candidate.id === hit.id);
    if (!card) return null;
    const field = hit.field ?? "title";
    const worldRect = textFieldRect(card, field);
    const request: DomOverlayRequest = {
      target: { kind: "card-text", id: card.id, field },
      value: String(card[field]),
      worldRect,
      screenRect: worldRectToScreen(worldRect, this.camera)
    };
    this.mountOverlay(request);
    this.onEvent({ type: "overlay", request });
    return request;
  }

  commitTextEdit() {
    if (!this.activeOverlay || !this.activeOverlayRequest) return;
    const { id, field } = this.activeOverlayRequest.target;
    const value = this.activeOverlay.value;
    this.applyPatch({ kind: "edit-card-text", id, field, value });
    this.removeOverlay(true);
  }

  renderFrame(now: number): FrameStats {
    const start = performance.now();
    if (this.webGpuRenderer) {
      try {
        if (this.cameraDirty) {
          this.lastWebGpuFrame = this.webGpuRenderer.renderFrameWithCamera(this.camera.x, this.camera.y, this.camera.zoom);
          this.cameraDirty = false;
          this.rustCameraFlushes += 1;
        } else {
          this.lastWebGpuFrame = this.webGpuRenderer.renderFrame();
        }
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
      rustTextLayoutCacheHits: this.lastWebGpuFrame?.textLayoutCacheHits ?? null,
      rustTextLayoutCacheMisses: this.lastWebGpuFrame?.textLayoutCacheMisses ?? null,
      rustStyleTokens: this.lastWebGpuFrame?.styleTokenCount ?? null,
      rustCameraFlushes: this.rustCameraFlushes,
      rustPatchUpdates: this.lastWebGpuFrame?.patchUpdateCount ?? null,
      rustDirtyWrites: this.lastWebGpuFrame?.dirtyRangeWriteCount ?? null,
      rustFullRebuilds: this.lastWebGpuFrame?.fullBufferRebuildCount ?? null,
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
      rustFreeGroupSlots: this.lastWebGpuFrame?.groupSlotFreeCount ?? null
    };
    this.inputBatchSize = 0;
    return stats;
  }

  private bindInput() {
    this.canvas.addEventListener("pointerdown", this.onPointerDown);
    this.canvas.addEventListener("pointermove", this.onPointerMove);
    this.canvas.addEventListener("pointerup", this.onPointerUp);
    this.canvas.addEventListener("pointercancel", this.onPointerUp);
    this.canvas.addEventListener("dblclick", this.onDoubleClick);
    this.canvas.addEventListener("wheel", this.onWheel, { passive: false });
  }

  private onPointerDown = (event: PointerEvent) => {
    const screen = this.eventPoint(event);
    const hit = this.hitTest(screen);
    this.selected = hit;
    this.syncSelection(hit);
    this.inputBatchSize += 1;
    this.canvas.setPointerCapture(event.pointerId);
    if (hit?.kind === "port" && hit.port === "source") {
      this.drag = { kind: "edge", pointerId: event.pointerId, sourceId: hit.id, current: hit.world };
    } else if (hit?.kind === "card" || hit?.kind === "text") {
      const card = this.snapshot?.cards.find((candidate) => candidate.id === hit.id);
      if (card) this.drag = { kind: "card", pointerId: event.pointerId, cardId: card.id, start: hit.world, startBounds: card.bounds };
    } else if (hit?.kind === "group") {
      this.drag = { kind: "group", pointerId: event.pointerId, groupId: hit.id, start: hit.world };
    } else {
      this.drag = { kind: "pan", pointerId: event.pointerId, start: screen, camera: this.camera };
    }
    this.onEvent({ type: "selection", hit });
  };

  private onPointerMove = (event: PointerEvent) => {
    if (!this.drag || !this.snapshot) return;
    this.inputBatchSize += 1;
    const screen = this.eventPoint(event);
    if (this.drag.kind === "pan") {
      this.camera = {
        ...this.drag.camera,
        x: this.drag.camera.x + screen.x - this.drag.start.x,
        y: this.drag.camera.y + screen.y - this.drag.start.y
      };
      this.queueWebGpuCamera();
      this.updateOverlayPosition();
      return;
    }
    const world = screenToWorld(screen, this.camera);
    if (this.drag.kind === "edge") {
      this.drag.current = world;
      return;
    }
    const dx = world.x - this.drag.start.x;
    const dy = world.y - this.drag.start.y;
    if (this.drag.kind === "group") {
      this.drag.start = world;
      this.applyPatch({
        kind: "move-group",
        id: this.drag.groupId,
        delta: { x: dx, y: dy }
      });
      return;
    }
    this.applyPatch({
      kind: "move-card",
      id: this.drag.cardId,
      position: {
        x: this.drag.startBounds.x + dx,
        y: this.drag.startBounds.y + dy
      }
    });
  };

  private onPointerUp = (event: PointerEvent) => {
    if (this.drag?.kind === "edge") {
      const sourceId = this.drag.sourceId;
      const hit = this.hitTest(this.eventPoint(event));
      if (hit?.kind === "port" && hit.port === "target" && hit.id !== sourceId && this.snapshot) {
        const source = this.snapshot.cards.find((card) => card.id === sourceId);
        const target = this.snapshot.cards.find((card) => card.id === hit.id);
        const groupId = source?.groupId ?? target?.groupId ?? hit.groupId ?? this.snapshot.groups[0]?.id ?? "poc-group";
        this.applyPatch({
          kind: "create-edge",
          groupId,
          source: sourceId,
          target: hit.id,
          edgeId: `poc-edge-${crypto.randomUUID().slice(0, 8)}`,
          label: "supports"
        });
      }
    }
    this.drag = null;
    try {
      this.canvas.releasePointerCapture(event.pointerId);
    } catch {
      // Pointer capture may already be released after cancellation.
    }
  };

  private onDoubleClick = (event: MouseEvent) => {
    const hit = this.hitTest(this.eventPoint(event));
    if (hit?.kind === "text") this.beginTextEdit(hit);
  };

  private onWheel = (event: WheelEvent) => {
    event.preventDefault();
    const screen = this.eventPoint(event);
    const world = screenToWorld(screen, this.camera);
    const zoom = clamp(this.camera.zoom * Math.exp(-event.deltaY * 0.0012), 0.025, 2.8);
    this.camera = {
      zoom,
      x: screen.x - world.x * zoom,
      y: screen.y - world.y * zoom
    };
    this.queueWebGpuCamera();
    this.inputBatchSize += 1;
    this.updateOverlayPosition();
  };

  private mountOverlay(request: DomOverlayRequest) {
    this.removeOverlay(false);
    const textarea = document.createElement("textarea");
    textarea.className = "poc-edit-overlay";
    textarea.value = request.value;
    textarea.autocomplete = "off";
    textarea.spellcheck = true;
    textarea.addEventListener("keydown", (event) => {
      if ((event.metaKey || event.ctrlKey) && event.key === "Enter") this.commitTextEdit();
      if (event.key === "Escape") this.removeOverlay(false);
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
    this.activeOverlay?.remove();
    this.activeOverlay = null;
    this.activeOverlayRequest = null;
    this.onEvent({ type: "overlay", request: null });
    if (!committed) this.onEvent({ type: "status", message: "Text edit cancelled" });
  }

  private updateOverlayPosition() {
    if (!this.activeOverlay || !this.activeOverlayRequest) return;
    const screenRect = worldRectToScreen(this.activeOverlayRequest.worldRect, this.camera);
    this.activeOverlay.style.left = `${screenRect.x}px`;
    this.activeOverlay.style.top = `${screenRect.y}px`;
    this.activeOverlay.style.width = `${screenRect.width}px`;
    this.activeOverlay.style.height = `${screenRect.height}px`;
    this.activeOverlay.style.fontSize = `${Math.max(13, 15 * this.camera.zoom)}px`;
  }

  private eventPoint(event: MouseEvent | PointerEvent | WheelEvent): WorldPoint {
    const rect = this.canvas.getBoundingClientRect();
    return { x: event.clientX - rect.left, y: event.clientY - rect.top };
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
      rustTextLayoutCacheHits: this.lastWebGpuFrame?.textLayoutCacheHits ?? null,
      rustTextLayoutCacheMisses: this.lastWebGpuFrame?.textLayoutCacheMisses ?? null,
      rustStyleTokens: this.lastWebGpuFrame?.styleTokenCount ?? null,
      rustCameraFlushes: this.rustCameraFlushes,
      rustPatchUpdates: this.lastWebGpuFrame?.patchUpdateCount ?? null,
      rustDirtyWrites: this.lastWebGpuFrame?.dirtyRangeWriteCount ?? null,
      rustFullRebuilds: this.lastWebGpuFrame?.fullBufferRebuildCount ?? null,
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
      rustFreeGroupSlots: this.lastWebGpuFrame?.groupSlotFreeCount ?? null
    };
  }

  private syncWebGpuScene(snapshot: SceneSnapshot) {
    if (!this.webGpuRenderer) return;
    this.webGpuRenderer.loadScene(JSON.stringify(snapshot));
    this.cameraDirty = false;
    this.rustBoundaryCalls += 1;
  }

  private syncWebGpuPatch(patch: ScenePatch, fallbackSnapshot: SceneSnapshot) {
    if (!this.webGpuRenderer) return;
    try {
      this.webGpuRenderer.applyPatch(JSON.stringify(patch));
      this.rustBoundaryCalls += 1;
    } catch (error) {
      this.onEvent({
        type: "status",
        message: error instanceof Error ? `WebGPU patch sync failed, reloaded scene: ${error.message}` : "WebGPU patch sync failed, reloaded scene"
      });
      this.syncWebGpuScene(fallbackSnapshot);
    }
  }

  private syncSelection(hit: HitResult | null) {
    if (!this.snapshot) return;
    const patch: ScenePatch = { kind: "select", selection: hitToSelection(hit) };
    this.snapshot = applyScenePatch(this.snapshot, patch);
    this.syncWebGpuPatch(patch, this.snapshot);
    this.boundaryCalls += 1;
  }

  private queueWebGpuCamera() {
    if (!this.webGpuRenderer) return;
    this.cameraDirty = true;
  }

  private flushWebGpuCamera() {
    if (!this.webGpuRenderer || !this.cameraDirty) return;
    this.webGpuRenderer.setCamera(this.camera.x, this.camera.y, this.camera.zoom);
    this.cameraDirty = false;
    this.rustCameraFlushes += 1;
    this.rustBoundaryCalls += 1;
  }
}

function hitToSelection(hit: HitResult | null): SceneSelection {
  if (!hit) return { kind: "canvas" };
  if (hit.kind === "group" && hit.groupId) return { kind: "group", id: hit.groupId };
  if (hit.kind === "edge") return { kind: "edge", id: hit.id };
  return { kind: "node", id: hit.id };
}

function rustHitToEngineHit(hit: RustHitResult | null, screen: WorldPoint): HitResult | null {
  if (!hit) return null;
  const world = { x: hit.worldX, y: hit.worldY };
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

function textFieldRect(card: RenderCard, field: "title" | "summary" | "detail"): WorldRect {
  if (field === "title") return { x: card.bounds.x + 16, y: card.bounds.y + 44, width: card.bounds.width - 32, height: 48 };
  if (field === "summary") return { x: card.bounds.x + 16, y: card.bounds.y + 96, width: card.bounds.width - 32, height: 58 };
  return { x: card.bounds.x + 16, y: card.bounds.y + 92, width: card.bounds.width - 32, height: card.bounds.height - 108 };
}

function readMemoryBytes(): number | null {
  const performanceWithMemory = performance as Performance & { memory?: { usedJSHeapSize: number } };
  return performanceWithMemory.memory?.usedJSHeapSize ?? null;
}
