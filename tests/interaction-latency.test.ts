import { describe, expect, it } from "vitest";
import { ShapeCanvasEngine } from "../src/client/renderer/engine";
import {
  runInteractionLatency,
  runInteractionLatencyUnderFollow,
  INTERACTION_LATENCY_BUDGET_MS,
  type InteractionKind,
  type InteractionLatencyResult
} from "../src/client/renderer/benchmark";
import { createHeterogeneousFixture } from "../src/client/renderer/fixtures";
import { applyScenePatch } from "../src/client/renderer/scene";
import type { CameraState, ScenePatch, SceneSnapshot, WorldRect } from "../src/client/renderer/scene";
import type {
  RustDebugSnapshot,
  RustInputBatchResult,
  RustWebGpuFrameStats,
  RustWebGpuRenderer
} from "../src/client/renderer/wasmLoader";

// T3.4 interaction-under-load harness tests. They drive the seven direct-
// manipulation flows through the real engine input-batch/camera/overlay seams
// against a T3.3 heterogeneous fixture, measuring per-interaction latency and
// asserting it stays within the seed budget while detail degrades (the working
// set shrinks under load), plus the follow-vs-input contention contract.

describe("T3.4 interaction latency under load", () => {
  const flows: InteractionKind[] = ["pan", "zoom", "select", "drag", "text-edit", "follow"];

  for (const kind of flows) {
    it(`keeps ${kind} latency within the seed budget on a mixed-1k fixture`, () => {
      const { engine } = createLoadedEngine("mixed-1k", 41);
      const result = runInteractionLatency(engine, kind, 48);

      expect(result.kind).toBe(kind);
      expect(result.samples).toBe(48);
      expect(result.totalCards).toBeGreaterThan(800);
      // The harness reads the FrameStats the engine already returns.
      expect(result.memoryBytes === null || typeof result.memoryBytes === "number").toBe(true);
      // Per-flow p95 <= budget: a synthetic fake-renderer frame is sub-millisecond,
      // so the round trip must comfortably clear the seed budget.
      expect(result.p95LatencyMs).toBeLessThanOrEqual(INTERACTION_LATENCY_BUDGET_MS[kind]);
      expect(result.maxLatencyMs).toBeGreaterThanOrEqual(result.p95LatencyMs);
    });
  }

  it("reports a degrading working set as the camera zooms out (degrade-don't-stall)", () => {
    const { engine } = createLoadedEngine("mixed-1k", 7);
    // Far-zoom pan: most of the scene culls, so the working-set ratio is well
    // below 1 while latency stays bounded — the operational degradation read.
    const result = runInteractionLatency(engine, "pan", 48);

    expect(result.workingSetRatio).toBeGreaterThan(0);
    expect(result.workingSetRatio).toBeLessThan(1);
    expect(result.averageVisibleCards).toBeLessThan(result.totalCards);
    expect(result.droppedFrames).toBe(0);
  });

  it("selects the expected card identity at every step under load", () => {
    const { engine, snapshot } = createLoadedEngine("mixed-1k", 12);
    const expectedIds: string[] = [];
    const observedIds: string[] = [];
    const steps = 32;
    for (let index = 0; index < steps; index += 1) {
      const t = index / Math.max(1, steps - 1);
      const card = snapshot.cards[Math.floor(t * (snapshot.cards.length - 1))];
      expectedIds.push(card.id);
      engine.applyPatchBatch([{ kind: "select", selection: { kind: "node", id: card.id } }]);
      observedIds.push(selectionId(engine.getSnapshot()));
    }
    expect(observedIds).toEqual(expectedIds);
  });

  it("holds user-input latency within budget while a follow loop animates (contention contract)", () => {
    const seed = 23;
    const noFollow = runInteractionLatency(createLoadedEngine("mixed-1k", seed).engine, "pan", 48);
    const underFollow = runInteractionLatencyUnderFollow(createLoadedEngine("mixed-1k", seed).engine, "pan", 48);

    // Both ride the single sendInputBatch boundary and are serialized, not raced:
    // user-input p95 under follow must stay within the same flow's no-follow budget.
    expect(underFollow.p95LatencyMs).toBeLessThanOrEqual(INTERACTION_LATENCY_BUDGET_MS.pan);
    expect(noFollow.p95LatencyMs).toBeLessThanOrEqual(INTERACTION_LATENCY_BUDGET_MS.pan);
    expect(underFollow.samples).toBe(noFollow.samples);
    expect(underFollow.totalCards).toBe(noFollow.totalCards);
  });

  it("scales the harness to a mixed-5k fixture without breaching the pan budget", () => {
    const { engine } = createLoadedEngine("mixed-5k", 99);
    const result: InteractionLatencyResult = runInteractionLatency(engine, "pan", 32);

    expect(result.totalCards).toBeGreaterThan(4_000);
    expect(result.p95LatencyMs).toBeLessThanOrEqual(INTERACTION_LATENCY_BUDGET_MS.pan);
    expect(result.workingSetRatio).toBeLessThan(1);
  });
});

// -- test rig --

function createLoadedEngine(
  profile: "mixed-1k" | "mixed-5k" | "mixed-workspace",
  seed: number
): { engine: ShapeCanvasEngine; snapshot: SceneSnapshot } {
  const { snapshot } = createHeterogeneousFixture({ profile, seed });
  const renderer = createCullingTestRenderer(snapshot);
  const engine = new ShapeCanvasEngine({
    canvas: testCanvas(),
    overlayRoot: testOverlayRoot(),
    backend: "test",
    webGpuRenderer: renderer,
    onEvent() {}
  });
  engine.loadScene(snapshot);
  return { engine, snapshot };
}

function selectionId(snapshot: SceneSnapshot | null): string {
  const selection = snapshot?.selection;
  if (!selection || selection.kind === "canvas") return "";
  if (selection.kind === "multi") return selection.ids[0] ?? "";
  return selection.id;
}

/**
 * A fake renderer that mirrors the real one's contract but computes a *real*
 * viewport-culled visible-card count from the loaded scene + camera, so the
 * working-set ratio actually degrades as the camera moves/zooms. This lets the
 * harness exercise the degrade-don't-stall signal without a GPU.
 */
function createCullingTestRenderer(initialScene: SceneSnapshot): RustWebGpuRenderer {
  let scene = initialScene;
  let viewportWidth = 1240;
  let viewportHeight = 760;
  return {
    resize(width, height) {
      viewportWidth = width;
      viewportHeight = height;
    },
    loadScene(sceneJson) {
      scene = JSON.parse(sceneJson) as SceneSnapshot;
    },
    applyPatchBatch(patchesJson) {
      const patches = JSON.parse(patchesJson) as ScenePatch[];
      for (const patch of patches) scene = applyScenePatch(scene, patch);
    },
    renderFrame() {
      return cullingFrameStats(scene, viewportWidth, viewportHeight);
    },
    inputBatch(eventsJson): RustInputBatchResult {
      const events = JSON.parse(eventsJson) as Array<{ kind: string; camera?: CameraState; bounds?: WorldRect; screen?: { x: number; y: number }; zoom?: number; deltaY?: number }>;
      for (const event of events) {
        if (event.kind === "set-camera" && event.camera) scene = { ...scene, camera: event.camera };
        if (event.kind === "wheel" && event.screen && typeof event.deltaY === "number") {
          const factor = event.deltaY < 0 ? 1.1 : 1 / 1.1;
          const zoom = clampZoom(scene.camera.zoom * factor);
          const worldX = (event.screen.x - scene.camera.x) / scene.camera.zoom;
          const worldY = (event.screen.y - scene.camera.y) / scene.camera.zoom;
          scene = { ...scene, camera: { zoom, x: event.screen.x - worldX * zoom, y: event.screen.y - worldY * zoom } };
        }
        if (event.kind === "focus-bounds" && event.bounds && event.screen && typeof event.zoom === "number") {
          scene = {
            ...scene,
            camera: {
              zoom: event.zoom,
              x: event.screen.x - (event.bounds.x + event.bounds.width / 2) * event.zoom,
              y: event.screen.y - (event.bounds.y + event.bounds.height / 2) * event.zoom
            }
          };
        }
      }
      return { camera: scene.camera, hit: null, selection: scene.selection, patches: [], overlay: null };
    },
    overlayRequest() {
      return null;
    },
    debugSnapshot(): RustDebugSnapshot {
      return {
        camera: scene.camera,
        selection: scene.selection,
        selectionWorldRect: null,
        selectionScreenRect: null,
        lastHit: null,
        totalGroups: scene.groups.length,
        totalCards: scene.cards.length,
        totalEdges: scene.edges.length,
        patchUpdateCount: 0,
        dirtyRangeWriteCount: 0,
        fullBufferRebuildCount: 0
      };
    }
  };
}

function clampZoom(zoom: number): number {
  return Math.min(2, Math.max(0.05, zoom));
}

function cullingFrameStats(scene: SceneSnapshot, width: number, height: number): RustWebGpuFrameStats {
  const camera = scene.camera;
  const viewport: WorldRect = {
    x: (0 - camera.x) / camera.zoom,
    y: (0 - camera.y) / camera.zoom,
    width: width / camera.zoom,
    height: height / camera.zoom
  };
  let visibleCards = 0;
  for (const card of scene.cards) if (rectsIntersect(card.bounds, viewport)) visibleCards += 1;
  let visibleGroups = 0;
  for (const group of scene.groups) if (rectsIntersect(group.bounds, viewport)) visibleGroups += 1;
  return {
    totalGroups: scene.groups.length,
    totalCards: scene.cards.length,
    totalEdges: scene.edges.length,
    visibleGroupCount: visibleGroups,
    visibleCardCount: visibleCards,
    visibleEdgeCount: 0,
    vertexCount: 0,
    drawnVertexCount: 0,
    drawRangeCount: 0,
    textGlyphCount: 0,
    fallbackTextGlyphCount: 0,
    cjkTextGlyphCount: 0,
    fontFallbackRunCount: 0,
    missingTextGlyphCount: 0,
    textAtlasOverflowGlyphCount: 0,
    textMissingRasterGlyphCount: 0,
    textAtlasGlyphCount: 0,
    textRasterCacheHits: 0,
    textRasterCacheMisses: 0,
    textLayoutCacheHits: 0,
    textLayoutCacheMisses: 0,
    styleTokenCount: scene.styles.length,
    patchUpdateCount: 0,
    dirtyRangeWriteCount: 0,
    fullBufferRebuildCount: 0,
    vertexTruncationCount: 0,
    truncatedVertexCount: 0,
    edgeCapacityGrowCount: 0,
    edgeCompactionCount: 0,
    edgeSlotCount: 0,
    edgeSlotFreeCount: 0,
    cardCapacityGrowCount: 0,
    cardCompactionCount: 0,
    cardSlotCount: 0,
    cardSlotFreeCount: 0,
    groupCapacityGrowCount: 0,
    groupCompactionCount: 0,
    groupSlotCount: 0,
    groupSlotFreeCount: 0,
    backend: "test"
  };
}

function rectsIntersect(a: WorldRect, b: WorldRect): boolean {
  return a.x <= b.x + b.width && a.x + a.width >= b.x && a.y <= b.y + b.height && a.y + a.height >= b.y;
}

function testCanvas(): HTMLCanvasElement {
  const listeners = new Map<string, Set<EventListener>>();
  const element = {
    width: 0,
    height: 0,
    addEventListener(type: string, listener: EventListenerOrEventListenerObject) {
      if (typeof listener !== "function") return;
      const set = listeners.get(type) ?? new Set();
      set.add(listener);
      listeners.set(type, set);
    },
    removeEventListener(type: string, listener: EventListenerOrEventListenerObject) {
      if (typeof listener !== "function") return;
      listeners.get(type)?.delete(listener);
    },
    setPointerCapture() {},
    releasePointerCapture() {},
    getBoundingClientRect() {
      return { left: 0, top: 0, width: 800, height: 600 };
    }
  };
  return element as unknown as HTMLCanvasElement;
}

function testOverlayRoot(): HTMLElement {
  return {
    append() {}
  } as unknown as HTMLElement;
}
