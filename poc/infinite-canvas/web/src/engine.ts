import {
  applyScenePatch,
  buildSpatialIndex,
  clamp,
  defaultStyles,
  fitCameraToBounds,
  pointInRect,
  querySpatialIndex,
  rectsIntersect,
  sceneWorldBounds,
  screenToWorld,
  truncateText,
  validateScenePatch,
  viewportToWorld,
  worldRectToScreen,
  worldToScreen,
  type CameraState,
  type DomOverlayRequest,
  type FrameStats,
  type HitResult,
  type RenderCard,
  type RenderEdge,
  type RenderGroup,
  type ScenePatch,
  type SceneSelection,
  type SceneSnapshot,
  type SceneStyleToken,
  type SpatialIndex,
  type WorldPoint,
  type WorldRect
} from "./scene";
import type { RustCanvasCore, RustFrameStats, RustHitResult, RustWebGpuFrameStats, RustWebGpuRenderer } from "./wasmLoader";

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
  drawBackend: FrameStats["drawBackend"];
  rustCore?: RustCanvasCore | null;
  webGpuRenderer?: RustWebGpuRenderer | null;
  onEvent: (event: EngineEvent) => void;
};

type DragState =
  | { kind: "pan"; pointerId: number; start: WorldPoint; camera: CameraState }
  | { kind: "group"; pointerId: number; groupId: string; start: WorldPoint }
  | { kind: "card"; pointerId: number; cardId: string; start: WorldPoint; startBounds: WorldRect }
  | { kind: "edge"; pointerId: number; sourceId: string; current: WorldPoint };

type RenderCaches = {
  text: Map<string, string[]>;
  edge: Map<string, { start: WorldPoint; end: WorldPoint; cp1: WorldPoint; cp2: WorldPoint }>;
  geometry: Map<string, Path2D>;
  hits: number;
  misses: number;
};

export class ShapeCanvasEngine {
  private canvas: HTMLCanvasElement;
  private overlayRoot: HTMLElement;
  private ctx: CanvasRenderingContext2D;
  private onEvent: (event: EngineEvent) => void;
  private snapshot: SceneSnapshot | null = null;
  private spatialIndex: SpatialIndex = buildSpatialIndex([]);
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
  private caches: RenderCaches = {
    text: new Map(),
    edge: new Map(),
    geometry: new Map(),
    hits: 0,
    misses: 0
  };
  private inputBatchSize = 0;
  private boundaryCalls = 0;
  private rustBoundaryCalls = 0;
  private lastRustFrame: RustFrameStats | null = null;
  private lastWebGpuFrame: RustWebGpuFrameStats | null = null;
  private backend: string;
  private drawBackend: FrameStats["drawBackend"];
  private rustCore: RustCanvasCore | null;
  private webGpuRenderer: RustWebGpuRenderer | null;

  constructor(options: ShapeCanvasEngineOptions) {
    const context = options.canvas.getContext("2d");
    if (!context) throw new Error("2D canvas context is unavailable");
    this.canvas = options.canvas;
    this.overlayRoot = options.overlayRoot;
    this.ctx = context;
    this.onEvent = options.onEvent;
    this.backend = options.backend;
    this.drawBackend = options.drawBackend;
    this.rustCore = options.rustCore ?? null;
    this.webGpuRenderer = options.webGpuRenderer ?? null;
    if (this.rustCore) {
      try {
        this.rustCore.mount(this.canvas);
        this.rustBoundaryCalls += 1;
      } catch (error) {
        this.onEvent({
          type: "status",
          message: error instanceof Error ? `Rust core mount failed: ${error.message}` : "Rust core mount failed"
        });
        this.rustCore = null;
        this.drawBackend = "typescript-canvas2d";
      }
    }
    this.bindInput();
  }

  loadScene(snapshot: SceneSnapshot) {
    this.snapshot = snapshot;
    this.camera = snapshot.camera;
    this.spatialIndex = buildSpatialIndex(snapshot.cards);
    this.caches.text.clear();
    this.caches.edge.clear();
    this.caches.geometry.clear();
    this.syncRustScene(snapshot);
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
    if (this.rustCore) {
      this.rustCore.resize(this.width, this.height, this.dpr);
      this.rustBoundaryCalls += 1;
    }
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
    this.syncRustCamera();
    this.syncWebGpuCamera();
    this.boundaryCalls += 1;
    this.updateOverlayPosition();
  }

  setCamera(camera: CameraState) {
    this.camera = { ...camera, zoom: clamp(camera.zoom, 0.025, 2.8) };
    this.syncRustCamera();
    this.syncWebGpuCamera();
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
      if (patch.kind === "move-group" || patch.kind === "move-card" || patch.kind === "create-card" || patch.kind === "delete-card" || patch.kind === "delete-group") {
        this.spatialIndex = buildSpatialIndex(this.snapshot.cards);
      }
      if (patch.kind === "edit-card-text") this.caches.text.clear();
      if (patch.kind === "move-group" || patch.kind === "create-edge" || patch.kind === "delete-edge" || patch.kind === "delete-card" || patch.kind === "delete-group") {
        this.caches.edge.clear();
      }
      this.syncRustScene(this.snapshot);
      this.syncWebGpuPatch(patch, this.snapshot);
      this.boundaryCalls += 1;
    }
    this.onEvent({ type: "patch", patch, errors });
    return errors;
  }

  hitTest(screen: WorldPoint): HitResult | null {
    if (this.drawBackend === "rust-wgpu-visible" && this.webGpuRenderer) {
      try {
        const hit = rustHitToEngineHit(this.webGpuRenderer.hitTest(screen.x, screen.y), screen);
        this.rustBoundaryCalls += 1;
        return hit;
      } catch (error) {
        this.onEvent({
          type: "status",
          message: error instanceof Error ? `WebGPU hit test failed: ${error.message}` : "WebGPU hit test failed"
        });
      }
    }
    if (!this.snapshot) return null;
    const world = screenToWorld(screen, this.camera);
    const cards = [...this.snapshot.cards].sort((a, b) => b.zIndex - a.zIndex);
    for (const card of cards) {
      if (!pointInRect(world, card.bounds)) continue;
      const port = portAtPoint(card, world);
      if (port) return { kind: "port", id: card.id, groupId: card.groupId, port, world, screen };
      const field = textFieldAtPoint(card, world);
      return { kind: field ? "text" : "card", id: card.id, groupId: card.groupId, field, world, screen };
    }
    for (const edge of [...this.snapshot.edges].reverse()) {
      const route = this.edgeRoute(edge);
      if (route && distanceToCubic(world, route.start, route.cp1, route.cp2, route.end) <= 18 / this.camera.zoom) {
        return { kind: "edge", id: edge.id, groupId: edge.groupId, world, screen };
      }
    }
    for (const group of [...this.snapshot.groups].sort((a, b) => b.zIndex - a.zIndex)) {
      if (pointInRect(world, group.bounds)) return { kind: "group", id: group.id, groupId: group.id, world, screen };
    }
    return null;
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
    const useRustDraw = this.drawBackend === "rust-wasm-debug" && this.rustCore !== null;
    const useWebGpuDraw = this.drawBackend === "rust-wgpu-visible" && this.webGpuRenderer !== null;
    if (useWebGpuDraw) {
      try {
        this.lastWebGpuFrame = this.webGpuRenderer!.renderFrame();
        this.rustBoundaryCalls += 1;
      } catch (error) {
        this.onEvent({
          type: "status",
          message: error instanceof Error ? `WebGPU render failed: ${error.message}` : "WebGPU render failed"
        });
        this.drawBackend = "typescript-canvas2d";
      }
    } else if (useRustDraw) {
      this.lastRustFrame = this.rustCore!.renderFrame();
      this.rustBoundaryCalls += 1;
    } else {
      this.ctx.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
      this.ctx.clearRect(0, 0, this.width, this.height);
      this.paintBackground(now);
    }
    if (!this.snapshot) return this.emptyStats(start);

    const viewport = viewportToWorld(this.width, this.height, this.camera);
    const paddedViewport = {
      x: viewport.x - 400,
      y: viewport.y - 400,
      width: viewport.width + 800,
      height: viewport.height + 800
    };
    const visibleCardIds = querySpatialIndex(this.spatialIndex, paddedViewport);
    const visibleCards = this.snapshot.cards
      .filter((card) => visibleCardIds.has(card.id) && rectsIntersect(card.bounds, paddedViewport))
      .sort((a, b) => a.zIndex - b.zIndex);
    const visibleCardIdSet = new Set(visibleCards.map((card) => card.id));
    const visibleGroups = this.snapshot.groups.filter((group) => rectsIntersect(group.bounds, paddedViewport));
    const visibleEdges = this.snapshot.edges.filter((edge) => visibleCardIdSet.has(edge.source) && visibleCardIdSet.has(edge.target));

    if (!useRustDraw && !useWebGpuDraw) {
      this.ctx.save();
      this.ctx.translate(this.camera.x, this.camera.y);
      this.ctx.scale(this.camera.zoom, this.camera.zoom);
      for (const group of visibleGroups) this.drawGroup(group);
      for (const edge of visibleEdges) this.drawEdge(edge);
      if (this.drag?.kind === "edge") this.drawEdgePreview(this.drag);
      for (const card of visibleCards) this.drawCard(card);
      this.ctx.restore();
    }

    this.drawHudOverlay(visibleGroups.length, visibleCards.length, visibleEdges.length);
    this.updateOverlayPosition();
    const renderMs = performance.now() - start;
    const stats: FrameStats = {
      frameMs: renderMs,
      renderMs,
      totalGroups: this.snapshot.groups.length,
      totalCards: this.snapshot.cards.length,
      totalEdges: this.snapshot.edges.length,
      visibleGroups: visibleGroups.length,
      visibleCards: visibleCards.length,
      visibleEdges: visibleEdges.length,
      cacheHits: this.caches.hits,
      cacheMisses: this.caches.misses,
      boundaryCalls: this.boundaryCalls,
      inputBatchSize: this.inputBatchSize,
      memoryBytes: readMemoryBytes(),
      backend: this.backend,
      drawBackend: this.drawBackend,
      rustCoreAvailable: Boolean(this.rustCore || this.webGpuRenderer),
      rustBoundaryCalls: this.rustBoundaryCalls,
      rustFrameCards: this.lastWebGpuFrame?.totalCards ?? this.lastRustFrame?.totalCards ?? null,
      rustFrameEdges: this.lastWebGpuFrame?.totalEdges ?? this.lastRustFrame?.totalEdges ?? null,
      rustGpuVertices: this.lastWebGpuFrame?.vertexCount ?? null,
      rustTextGlyphs: this.lastWebGpuFrame?.textGlyphCount ?? null,
      rustFallbackGlyphs: this.lastWebGpuFrame?.fallbackTextGlyphCount ?? null,
      rustCjkGlyphs: this.lastWebGpuFrame?.cjkTextGlyphCount ?? null,
      rustStyleTokens: this.lastWebGpuFrame?.styleTokenCount ?? null,
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
      this.syncRustCamera();
      this.syncWebGpuCamera();
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
    this.syncRustCamera();
    this.syncWebGpuCamera();
    this.inputBatchSize += 1;
    this.updateOverlayPosition();
  };

  private drawGroup(group: RenderGroup) {
    const style = this.style(group.styleKey);
    this.ctx.save();
    this.ctx.globalAlpha = 0.86;
    const path = this.roundRect(group.bounds.x, group.bounds.y, group.bounds.width, group.bounds.height, 36);
    this.ctx.fillStyle = "rgba(255, 255, 255, 0.32)";
    this.ctx.strokeStyle = withAlpha(style.stroke, 0.42);
    this.ctx.lineWidth = 2 / this.camera.zoom;
    this.ctx.fill(path);
    this.ctx.stroke(path);
    this.ctx.fillStyle = style.text;
    this.ctx.font = `${Math.max(20, 22 / Math.sqrt(this.camera.zoom))}px Inter, system-ui, sans-serif`;
    this.ctx.fillText(group.title, group.bounds.x + 28, group.bounds.y + 42);
    this.ctx.fillStyle = style.mutedText;
    this.ctx.font = `${Math.max(14, 14 / Math.sqrt(this.camera.zoom))}px Inter, system-ui, sans-serif`;
    this.ctx.fillText(truncateText(group.summary, 90), group.bounds.x + 28, group.bounds.y + 68);
    this.ctx.restore();
  }

  private drawCard(card: RenderCard) {
    const style = this.style(card.styleKey);
    const selected = this.selected?.id === card.id;
    const radius = 18;
    this.ctx.save();
    const cardPath = this.roundRect(card.bounds.x, card.bounds.y, card.bounds.width, card.bounds.height, radius);
    this.ctx.shadowColor = "rgba(20, 30, 40, 0.12)";
    this.ctx.shadowBlur = selected ? 28 : 14;
    this.ctx.shadowOffsetY = selected ? 12 : 7;
    this.ctx.fillStyle = style.fill;
    this.ctx.fill(cardPath);
    this.ctx.shadowColor = "transparent";
    this.ctx.strokeStyle = selected ? style.accent : withAlpha(style.stroke, 0.54);
    this.ctx.lineWidth = selected ? 4 / this.camera.zoom : 1.6 / this.camera.zoom;
    this.ctx.stroke(cardPath);

    this.ctx.fillStyle = style.accent;
    this.ctx.fill(this.roundRect(card.bounds.x + 18, card.bounds.y + 16, 68, 22, 11));
    this.ctx.fillStyle = "#ffffff";
    this.ctx.font = "700 11px Inter, system-ui, sans-serif";
    this.ctx.fillText(truncateText(card.type.replace(/_/g, " "), 12), card.bounds.x + 28, card.bounds.y + 31);

    this.ctx.fillStyle = style.text;
    this.ctx.font = "800 19px Inter, system-ui, sans-serif";
    for (const [lineIndex, line] of this.textLines(`${card.id}:title`, card.title, card.bounds.width - 34, 2, "800 19px Inter, system-ui, sans-serif").entries()) {
      this.ctx.fillText(line, card.bounds.x + 18, card.bounds.y + 66 + lineIndex * 23);
    }
    this.ctx.fillStyle = style.mutedText;
    this.ctx.font = "500 13px Inter, system-ui, sans-serif";
    for (const [lineIndex, line] of this.textLines(`${card.id}:summary`, card.summary, card.bounds.width - 36, 3, "500 13px Inter, system-ui, sans-serif").entries()) {
      this.ctx.fillText(line, card.bounds.x + 18, card.bounds.y + 112 + lineIndex * 17);
    }
    this.drawPort(card, "source");
    this.drawPort(card, "target");
    this.ctx.restore();
  }

  private drawEdge(edge: RenderEdge) {
    const route = this.edgeRoute(edge);
    if (!route) return;
    const selected = this.selected?.kind === "edge" && this.selected.id === edge.id;
    const style = this.style(edge.styleKey);
    this.ctx.save();
    this.ctx.beginPath();
    this.ctx.moveTo(route.start.x, route.start.y);
    this.ctx.bezierCurveTo(route.cp1.x, route.cp1.y, route.cp2.x, route.cp2.y, route.end.x, route.end.y);
    this.ctx.strokeStyle = selected ? style.accent : withAlpha(style.stroke, 0.52);
    this.ctx.lineWidth = selected ? 4 / this.camera.zoom : 2.2 / this.camera.zoom;
    this.ctx.stroke();
    this.drawArrow(route.cp2, route.end, selected ? style.accent : withAlpha(style.stroke, 0.62));
    if (this.camera.zoom >= 0.12 && edge.label) {
      this.ctx.font = "700 12px Inter, system-ui, sans-serif";
      this.ctx.fillStyle = withAlpha(style.text, 0.72);
      this.ctx.fillText(edge.label, (route.start.x + route.end.x) / 2, (route.start.y + route.end.y) / 2 - 8);
    }
    this.ctx.restore();
  }

  private drawEdgePreview(drag: Extract<DragState, { kind: "edge" }>) {
    const source = this.snapshot?.cards.find((card) => card.id === drag.sourceId);
    if (!source) return;
    const start = { x: source.bounds.x + source.bounds.width, y: source.bounds.y + source.bounds.height / 2 };
    this.ctx.save();
    this.ctx.setLineDash([10 / this.camera.zoom, 8 / this.camera.zoom]);
    this.ctx.strokeStyle = "rgba(47, 126, 230, 0.72)";
    this.ctx.lineWidth = 2.5 / this.camera.zoom;
    this.ctx.beginPath();
    this.ctx.moveTo(start.x, start.y);
    this.ctx.lineTo(drag.current.x, drag.current.y);
    this.ctx.stroke();
    this.ctx.restore();
  }

  private drawPort(card: RenderCard, port: "source" | "target") {
    const point = port === "source" ? { x: card.bounds.x + card.bounds.width, y: card.bounds.y + card.bounds.height / 2 } : { x: card.bounds.x, y: card.bounds.y + card.bounds.height / 2 };
    this.ctx.beginPath();
    this.ctx.arc(point.x, point.y, 7 / this.camera.zoom, 0, Math.PI * 2);
    this.ctx.fillStyle = port === "source" ? "#2f7ee6" : "#15a796";
    this.ctx.fill();
    this.ctx.strokeStyle = "#ffffff";
    this.ctx.lineWidth = 2 / this.camera.zoom;
    this.ctx.stroke();
  }

  private drawArrow(from: WorldPoint, to: WorldPoint, color: string) {
    const angle = Math.atan2(to.y - from.y, to.x - from.x);
    const length = 12 / this.camera.zoom;
    this.ctx.beginPath();
    this.ctx.moveTo(to.x, to.y);
    this.ctx.lineTo(to.x - Math.cos(angle - 0.42) * length, to.y - Math.sin(angle - 0.42) * length);
    this.ctx.lineTo(to.x - Math.cos(angle + 0.42) * length, to.y - Math.sin(angle + 0.42) * length);
    this.ctx.closePath();
    this.ctx.fillStyle = color;
    this.ctx.fill();
  }

  private drawHudOverlay(groups: number, cards: number, edges: number) {
    this.ctx.save();
    this.ctx.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
    this.ctx.fillStyle = "rgba(255, 255, 255, 0.86)";
    this.ctx.fill(this.roundRect(16, this.height - 44, 300, 28, 14));
    this.ctx.fillStyle = "#52606d";
    this.ctx.font = "700 12px Inter, system-ui, sans-serif";
    this.ctx.fillText(`${groups} groups · ${cards} cards · ${edges} edges · ${Math.round(this.camera.zoom * 100)}%`, 30, this.height - 26);
    this.ctx.restore();
  }

  private paintBackground(now: number) {
    const pulse = 0.5 + Math.sin(now / 1600) * 0.06;
    const gradient = this.ctx.createLinearGradient(0, 0, this.width, this.height);
    gradient.addColorStop(0, "#f8fafb");
    gradient.addColorStop(1, "#e8eff4");
    this.ctx.fillStyle = gradient;
    this.ctx.fillRect(0, 0, this.width, this.height);
    this.ctx.strokeStyle = `rgba(100, 116, 139, ${0.08 * pulse})`;
    this.ctx.lineWidth = 1;
    const grid = 48 * this.camera.zoom;
    if (grid >= 10) {
      const offsetX = this.camera.x % grid;
      const offsetY = this.camera.y % grid;
      for (let x = offsetX; x < this.width; x += grid) {
        this.ctx.beginPath();
        this.ctx.moveTo(x, 0);
        this.ctx.lineTo(x, this.height);
        this.ctx.stroke();
      }
      for (let y = offsetY; y < this.height; y += grid) {
        this.ctx.beginPath();
        this.ctx.moveTo(0, y);
        this.ctx.lineTo(this.width, y);
        this.ctx.stroke();
      }
    }
  }

  private edgeRoute(edge: RenderEdge) {
    const cached = this.caches.edge.get(edge.id);
    if (cached) {
      this.caches.hits += 1;
      return cached;
    }
    const source = this.snapshot?.cards.find((card) => card.id === edge.source);
    const target = this.snapshot?.cards.find((card) => card.id === edge.target);
    if (!source || !target) return null;
    const start = { x: source.bounds.x + source.bounds.width, y: source.bounds.y + source.bounds.height / 2 };
    const end = { x: target.bounds.x, y: target.bounds.y + target.bounds.height / 2 };
    const curve = Math.max(80, Math.abs(end.x - start.x) * 0.34);
    const route = {
      start,
      end,
      cp1: { x: start.x + curve, y: start.y },
      cp2: { x: end.x - curve, y: end.y }
    };
    this.caches.edge.set(edge.id, route);
    this.caches.misses += 1;
    return route;
  }

  private textLines(key: string, value: string, maxWidth: number, maxLines: number, font: string): string[] {
    const cacheKey = `${key}:${maxWidth}:${maxLines}:${value}`;
    const cached = this.caches.text.get(cacheKey);
    if (cached) {
      this.caches.hits += 1;
      return cached;
    }
    this.caches.misses += 1;
    this.ctx.save();
    this.ctx.font = font;
    const words = value.split(/\s+/);
    const lines: string[] = [];
    let line = "";
    for (const word of words) {
      const next = line ? `${line} ${word}` : word;
      if (this.ctx.measureText(next).width > maxWidth && line) {
        lines.push(line);
        line = word;
      } else {
        line = next;
      }
      if (lines.length === maxLines) break;
    }
    if (lines.length < maxLines && line) lines.push(line);
    if (lines.length > maxLines) lines.length = maxLines;
    if (lines.length === maxLines) lines[maxLines - 1] = truncateText(lines[maxLines - 1], 52);
    this.ctx.restore();
    this.caches.text.set(cacheKey, lines);
    return lines;
  }

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

  private roundRect(x: number, y: number, width: number, height: number, radius: number): Path2D {
    const key = `${Math.round(x * 10) / 10}:${Math.round(y * 10) / 10}:${Math.round(width * 10) / 10}:${Math.round(height * 10) / 10}:${radius}`;
    const cached = this.caches.geometry.get(key);
    if (cached) {
      this.caches.hits += 1;
      return cached;
    }
    this.caches.misses += 1;
    const path = new Path2D();
    this.ctx.beginPath();
    path.moveTo(x + radius, y);
    path.arcTo(x + width, y, x + width, y + height, radius);
    path.arcTo(x + width, y + height, x, y + height, radius);
    path.arcTo(x, y + height, x, y, radius);
    path.arcTo(x, y, x + width, y, radius);
    path.closePath();
    this.caches.geometry.set(key, path);
    return path;
  }

  private style(styleKey: string): SceneStyleToken {
    return this.snapshot?.styles.find((style) => style.id === styleKey) ?? defaultStyles[0];
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
      cacheHits: this.caches.hits,
      cacheMisses: this.caches.misses,
      boundaryCalls: this.boundaryCalls,
      inputBatchSize: this.inputBatchSize,
      memoryBytes: readMemoryBytes(),
      backend: this.backend,
      drawBackend: this.drawBackend,
      rustCoreAvailable: Boolean(this.rustCore || this.webGpuRenderer),
      rustBoundaryCalls: this.rustBoundaryCalls,
      rustFrameCards: this.lastWebGpuFrame?.totalCards ?? this.lastRustFrame?.totalCards ?? null,
      rustFrameEdges: this.lastWebGpuFrame?.totalEdges ?? this.lastRustFrame?.totalEdges ?? null,
      rustGpuVertices: this.lastWebGpuFrame?.vertexCount ?? null,
      rustTextGlyphs: this.lastWebGpuFrame?.textGlyphCount ?? null,
      rustFallbackGlyphs: this.lastWebGpuFrame?.fallbackTextGlyphCount ?? null,
      rustCjkGlyphs: this.lastWebGpuFrame?.cjkTextGlyphCount ?? null,
      rustStyleTokens: this.lastWebGpuFrame?.styleTokenCount ?? null,
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

  private syncRustScene(snapshot: SceneSnapshot) {
    if (!this.rustCore) return;
    this.rustCore.loadScene(JSON.stringify(snapshot));
    this.rustBoundaryCalls += 1;
  }

  private syncRustCamera() {
    if (!this.rustCore) return;
    this.rustCore.setCamera(this.camera.x, this.camera.y, this.camera.zoom);
    this.rustBoundaryCalls += 1;
  }

  private syncWebGpuScene(snapshot: SceneSnapshot) {
    if (!this.webGpuRenderer) return;
    this.webGpuRenderer.loadScene(JSON.stringify(snapshot));
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

  private syncWebGpuCamera() {
    if (!this.webGpuRenderer) return;
    this.webGpuRenderer.setCamera(this.camera.x, this.camera.y, this.camera.zoom);
    this.rustBoundaryCalls += 1;
  }
}

function hitToSelection(hit: HitResult | null): SceneSelection {
  if (!hit) return { kind: "canvas" };
  if (hit.kind === "group" && hit.groupId) return { kind: "group", id: hit.groupId };
  if (hit.kind === "edge") return { kind: "edge", id: hit.id };
  return { kind: "node", id: hit.id };
}

function textFieldAtPoint(card: RenderCard, point: WorldPoint): "title" | "summary" | "detail" | undefined {
  if (pointInRect(point, textFieldRect(card, "title"))) return "title";
  if (pointInRect(point, textFieldRect(card, "summary"))) return "summary";
  return undefined;
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

function portAtPoint(card: RenderCard, point: WorldPoint): "source" | "target" | null {
  const source = { x: card.bounds.x + card.bounds.width, y: card.bounds.y + card.bounds.height / 2 };
  const target = { x: card.bounds.x, y: card.bounds.y + card.bounds.height / 2 };
  if (distance(point, source) <= 16) return "source";
  if (distance(point, target) <= 16) return "target";
  return null;
}

function distance(a: WorldPoint, b: WorldPoint): number {
  return Math.hypot(a.x - b.x, a.y - b.y);
}

function distanceToCubic(point: WorldPoint, start: WorldPoint, cp1: WorldPoint, cp2: WorldPoint, end: WorldPoint): number {
  let best = Number.POSITIVE_INFINITY;
  let previous = start;
  for (let step = 1; step <= 18; step += 1) {
    const t = step / 18;
    const current = cubicPoint(start, cp1, cp2, end, t);
    best = Math.min(best, distanceToSegment(point, previous, current));
    previous = current;
  }
  return best;
}

function cubicPoint(a: WorldPoint, b: WorldPoint, c: WorldPoint, d: WorldPoint, t: number): WorldPoint {
  const mt = 1 - t;
  return {
    x: mt ** 3 * a.x + 3 * mt ** 2 * t * b.x + 3 * mt * t ** 2 * c.x + t ** 3 * d.x,
    y: mt ** 3 * a.y + 3 * mt ** 2 * t * b.y + 3 * mt * t ** 2 * c.y + t ** 3 * d.y
  };
}

function distanceToSegment(point: WorldPoint, start: WorldPoint, end: WorldPoint): number {
  const dx = end.x - start.x;
  const dy = end.y - start.y;
  const lengthSquared = dx * dx + dy * dy;
  if (lengthSquared === 0) return distance(point, start);
  const t = clamp(((point.x - start.x) * dx + (point.y - start.y) * dy) / lengthSquared, 0, 1);
  return distance(point, { x: start.x + t * dx, y: start.y + t * dy });
}

function readMemoryBytes(): number | null {
  const performanceWithMemory = performance as Performance & { memory?: { usedJSHeapSize: number } };
  return performanceWithMemory.memory?.usedJSHeapSize ?? null;
}

function withAlpha(color: string, alpha: number): string {
  if (!color.startsWith("#") || color.length !== 7) return color;
  const r = Number.parseInt(color.slice(1, 3), 16);
  const g = Number.parseInt(color.slice(3, 5), 16);
  const b = Number.parseInt(color.slice(5, 7), 16);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}
