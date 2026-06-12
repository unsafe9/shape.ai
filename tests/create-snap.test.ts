import { describe, expect, it } from "vitest";
import { ShapeCanvasEngine, type EngineEvent } from "../platforms/web/renderer/engine";
import { canonicalizeCreateSnap } from "../platforms/web/controller/interactions";
import { emptyObjectScene, type Object as SceneObject, type ObjectScene } from "../platforms/web/shared/object";
import type {
  RustInputBatchResult,
  RustWebGpuFrameStats,
  RustWebGpuRenderer
} from "../platforms/web/bridge/wasmLoader";

// Horizontal outline segment at world y=200, x in [100, 300], object "rect-1".
// The mock snaps a WORLD query to the nearest point within tolWorld.
const OUTLINE_Y = 200;
const OUTLINE_X0 = 100;
const OUTLINE_X1 = 300;
const TARGET_ID = "rect-1";

function mockRenderer(camera: { x: number; y: number; zoom: number }): RustWebGpuRenderer {
  const frame = emptyFrameStats();
  return {
    resize() {},
    renderFrame() {
      return frame;
    },
    inputBatch(): RustInputBatchResult {
      return { camera };
    },
    // WORLD query in, snapped WORLD point + targetId out; tolPx -> world via zoom.
    nearestOutlinePoint(worldX: number, worldY: number, tolPx: number, zoom: number, excludeIdsJson: string) {
      const tolWorld = tolPx / Math.max(0.025, zoom);
      const nearestX = Math.min(OUTLINE_X1, Math.max(OUTLINE_X0, worldX));
      const dx = worldX - nearestX;
      const dy = worldY - OUTLINE_Y;
      const within = dx * dx + dy * dy <= tolWorld * tolWorld;
      return within
        ? { snapped: true, x: nearestX, y: OUTLINE_Y, targetId: snapTargetFor(excludeIdsJson) }
        : { snapped: false, x: 0, y: 0, targetId: null };
    }
  } as unknown as RustWebGpuRenderer;
}

// Exclude-ids contract: the transient "create-preview" region under the cursor wins
// every snap until the engine excludes it. The mock surfaces the preview id when the
// engine drops the exclude list, the real target only once it is excluded.
const PREVIEW_ID = "create-preview";
function snapTargetFor(excludeIdsJson: string): string {
  let excluded: string[] = [];
  try {
    excluded = JSON.parse(excludeIdsJson ?? "[]");
  } catch {
    excluded = [];
  }
  return excluded.includes(PREVIEW_ID) ? TARGET_ID : PREVIEW_ID;
}

// Captures the engine's listeners so the test can fire DOM-shape mouse events.
// getBoundingClientRect is origin-anchored so client coords == canvas-local screen coords.
function captureCanvas(): { canvas: HTMLCanvasElement; fire: (type: string, init: Record<string, unknown>) => void } {
  const listeners = new Map<string, Set<EventListener>>();
  const canvas = {
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
  } as unknown as HTMLCanvasElement;
  const fire = (type: string, init: Record<string, unknown>) => {
    const event = { type, preventDefault() {}, ...init } as unknown as Event;
    for (const listener of listeners.get(type) ?? []) listener(event);
  };
  return { canvas, fire };
}

function makeEngine(camera: { x: number; y: number; zoom: number }) {
  const events: EngineEvent[] = [];
  const { canvas, fire } = captureCanvas();
  const engine = new ShapeCanvasEngine({
    canvas,
    overlayRoot: { append() {} } as unknown as HTMLElement,
    backend: "test",
    webGpuRenderer: mockRenderer(camera),
    onEvent: (event) => events.push(event)
  });
  // Refresh the cached camera from the renderer (a render tick does this at runtime).
  engine.renderFrame(performance.now());
  engine.setTool("create");
  return { engine, fire, events };
}

function lastCreate(events: EngineEvent[]): Extract<EngineEvent, { type: "create" }> | null {
  for (let i = events.length - 1; i >= 0; i -= 1) {
    const event = events[i];
    if (event.type === "create") return event;
  }
  return null;
}

describe("create-drag outline snap", () => {
  // Non-identity pan/zoom proves the engine uses the live camera, not {0,0,1}.
  const camera = { x: 40, y: 30, zoom: 0.5 };
  const toScreen = (wx: number, wy: number) => ({ clientX: wx * camera.zoom + camera.x, clientY: wy * camera.zoom + camera.y });

  it("emits snapped=true + targetId when the create-move hovers an existing outline", () => {
    const { fire, events } = makeEngine(camera);
    // Move the corner onto the outline (world (200, 200) sits on the segment).
    fire("mousedown", { button: 0, altKey: false, clientX: toScreen(0, 0).clientX, clientY: toScreen(0, 0).clientY });
    const onOutline = toScreen(200, OUTLINE_Y);
    fire("mousemove", { button: 0, altKey: false, clientX: onOutline.clientX, clientY: onOutline.clientY });

    const create = lastCreate(events);
    expect(create).not.toBeNull();
    expect(create!.phase).toBe("move");
    expect(create!.snapped).toBe(true);
    // The REAL object's id, not the transient preview — fails if the engine drops
    // the exclude arg.
    expect(create!.targetId).toBe(TARGET_ID);
    // The end position is the SNAPPED outline point, not the raw pointer.
    expect(create!.world.x).toBeCloseTo(200, 6);
    expect(create!.world.y).toBeCloseTo(OUTLINE_Y, 6);
  });

  it("excludes the transient create-preview from the snap query (no phantom self-snap)", () => {
    const { fire, events } = makeEngine(camera);
    fire("mousedown", { button: 0, altKey: false, clientX: toScreen(0, 0).clientX, clientY: toScreen(0, 0).clientY });
    const onOutline = toScreen(200, OUTLINE_Y);
    fire("mousemove", { button: 0, altKey: false, clientX: onOutline.clientX, clientY: onOutline.clientY });

    const create = lastCreate(events);
    expect(create).not.toBeNull();
    expect(create!.snapped).toBe(true);
    expect(create!.targetId).toBe(TARGET_ID);
    expect(create!.targetId).not.toBe(PREVIEW_ID);
  });

  it("emits snapped=false + null targetId when the create-move is far from any outline", () => {
    const { fire, events } = makeEngine(camera);
    fire("mousedown", { button: 0, altKey: false, clientX: toScreen(0, 0).clientX, clientY: toScreen(0, 0).clientY });
    // World (200, 900) is well below the segment, outside tolerance.
    const farOff = toScreen(200, 900);
    fire("mousemove", { button: 0, altKey: false, clientX: farOff.clientX, clientY: farOff.clientY });

    const create = lastCreate(events);
    expect(create).not.toBeNull();
    expect(create!.snapped).toBe(false);
    expect(create!.targetId).toBeNull();
    // Falls back to the raw pointer world position.
    expect(create!.world.x).toBeCloseTo(200, 6);
    expect(create!.world.y).toBeCloseTo(900, 6);
  });

  it("does NOT snap when the snap-bypass modifier (Alt) is held over the outline", () => {
    const { fire, events } = makeEngine(camera);
    fire("mousedown", { button: 0, altKey: true, clientX: toScreen(0, 0).clientX, clientY: toScreen(0, 0).clientY });
    const onOutline = toScreen(200, OUTLINE_Y);
    fire("mousemove", { button: 0, altKey: true, clientX: onOutline.clientX, clientY: onOutline.clientY });

    const create = lastCreate(events);
    expect(create).not.toBeNull();
    expect(create!.snapped).toBe(false);
    expect(create!.targetId).toBeNull();
  });
});

// A snap is honored only when its target is a real object in the canonical scene
// (the transient drag-create preview / snap-indicator never are).
describe("canonicalizeCreateSnap (handleCreate snap-target)", () => {
  function sceneOf(ids: string[]): ObjectScene {
    return { ...emptyObjectScene(), objects: ids.map((id) => ({ id, order: "a0", geometry: { d: "M 0 0 L 8 0" } }) as SceneObject) };
  }
  const scene = sceneOf(["rect-1", "ell-1"]);

  it("keeps a snap whose target is a real canonical object", () => {
    expect(canonicalizeCreateSnap(scene, true, "rect-1")).toEqual({ snapped: true, target: "rect-1" });
  });

  it("drops a phantom self-snap onto the transient create-preview", () => {
    expect(canonicalizeCreateSnap(scene, true, "create-preview")).toEqual({ snapped: false, target: null });
    expect(canonicalizeCreateSnap(scene, true, "create-snap-indicator")).toEqual({ snapped: false, target: null });
  });

  it("a non-snapped move stays non-snapped", () => {
    expect(canonicalizeCreateSnap(scene, false, null)).toEqual({ snapped: false, target: null });
  });
});

function emptyFrameStats(): RustWebGpuFrameStats {
  return {
    visibleGroupCount: 0,
    visibleCardCount: 0,
    visibleEdgeCount: 0,
    totalCards: 0,
    totalEdges: 0,
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
    styleTokenCount: 0,
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
    objectCount: 0,
    objectFillIndexCount: 0,
    objectStrokeVertexCount: 0,
    objectDrawCount: 0
  } as unknown as RustWebGpuFrameStats;
}
