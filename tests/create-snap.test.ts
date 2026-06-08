// W3-G5 (#6) — create-drag outline snap. Pins the REAL engine create path end-to-
// end against a mock renderer that carries a real `nearestOutlinePoint`: a create-
// move whose pointer is near an existing object's outline must emit a `create`
// event with `snapped = true` + a non-null `targetId` (and the snapped world end),
// while a move far from any outline emits `snapped = false` / `targetId = null`.
//
// Falsifiable: if the engine swallows the snap (stale camera, wrong feature-detect,
// or never querying), the near-outline case fails snapped/targetId.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { ShapeCanvasEngine, type EngineEvent } from "../src/client/renderer/engine";
import type {
  RustDebugSnapshot,
  RustInputBatchResult,
  RustWebGpuFrameStats,
  RustWebGpuRenderer
} from "../src/client/renderer/wasmLoader";

// A single horizontal outline segment at world y=200, spanning x in [100, 300],
// belonging to object "rect-1". The mock snaps a WORLD query to the nearest point
// on that segment when within `tolWorld`, mirroring the renderer's contract.
const OUTLINE_Y = 200;
const OUTLINE_X0 = 100;
const OUTLINE_X1 = 300;
const TARGET_ID = "rect-1";

function mockRenderer(camera: { x: number; y: number; zoom: number }): RustWebGpuRenderer {
  const frame = emptyFrameStats();
  return {
    resize() {},
    loadScene() {},
    applyPatchBatch() {},
    renderFrame() {
      return frame;
    },
    inputBatch(): RustInputBatchResult {
      return { camera, hit: null, selection: { kind: "canvas" }, patches: [], overlay: null };
    },
    overlayRequest() {
      return null;
    },
    debugSnapshot(): RustDebugSnapshot {
      return {
        camera,
        selection: { kind: "canvas" },
        selectionWorldRect: null,
        selectionScreenRect: null,
        lastHit: null,
        totalGroups: 0,
        totalCards: 0,
        totalEdges: 0,
        patchUpdateCount: 0,
        dirtyRangeWriteCount: 0,
        fullBufferRebuildCount: 0
      };
    },
    // The real W2-06 contract: WORLD query in, snapped WORLD point + targetId out.
    // `tolPx` is converted to world via the camera zoom, exactly like the renderer.
    nearestOutlinePoint(worldX: number, worldY: number, tolPx: number, zoom: number) {
      const tolWorld = tolPx / Math.max(0.025, zoom);
      const nearestX = Math.min(OUTLINE_X1, Math.max(OUTLINE_X0, worldX));
      const dx = worldX - nearestX;
      const dy = worldY - OUTLINE_Y;
      const within = dx * dx + dy * dy <= tolWorld * tolWorld;
      return within
        ? { snapped: true, x: nearestX, y: OUTLINE_Y, targetId: TARGET_ID }
        : { snapped: false, x: 0, y: 0, targetId: null };
    }
  } as unknown as RustWebGpuRenderer;
}

// A mock canvas that captures the engine's listeners so the test can fire DOM-shape
// mouse events at them. `getBoundingClientRect` returns origin-anchored so client
// coords == canvas-local screen coords.
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
  // Refresh the engine's cached camera from the renderer (a render tick does this
  // continuously at runtime via debugSnapshot).
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

describe("create-drag outline snap (W3-G5 #6)", () => {
  // Camera with a non-identity pan/zoom so the test proves the engine uses the live
  // camera (not the {0,0,1} default) when converting the pointer to world.
  const camera = { x: 40, y: 30, zoom: 0.5 };
  // World->screen for this camera: screen = world * zoom + pan.
  const toScreen = (wx: number, wy: number) => ({ clientX: wx * camera.zoom + camera.x, clientY: wy * camera.zoom + camera.y });

  it("emits snapped=true + targetId when the create-move hovers an existing outline", () => {
    const { fire, events } = makeEngine(camera);
    // Start the drag away from the outline, then move the corner ONTO the outline
    // (world (200, 200) sits exactly on the segment).
    fire("mousedown", { button: 0, altKey: false, clientX: toScreen(0, 0).clientX, clientY: toScreen(0, 0).clientY });
    const onOutline = toScreen(200, OUTLINE_Y);
    fire("mousemove", { button: 0, altKey: false, clientX: onOutline.clientX, clientY: onOutline.clientY });

    const create = lastCreate(events);
    expect(create).not.toBeNull();
    expect(create!.phase).toBe("move");
    expect(create!.snapped).toBe(true);
    expect(create!.targetId).toBe(TARGET_ID);
    // The emitted end position is the SNAPPED outline point, not the raw pointer.
    expect(create!.world.x).toBeCloseTo(200, 6);
    expect(create!.world.y).toBeCloseTo(OUTLINE_Y, 6);
  });

  it("emits snapped=false + null targetId when the create-move is far from any outline", () => {
    const { fire, events } = makeEngine(camera);
    fire("mousedown", { button: 0, altKey: false, clientX: toScreen(0, 0).clientX, clientY: toScreen(0, 0).clientY });
    // World (200, 900) is far below the segment at y=200 — well outside tolerance.
    const farOff = toScreen(200, 900);
    fire("mousemove", { button: 0, altKey: false, clientX: farOff.clientX, clientY: farOff.clientY });

    const create = lastCreate(events);
    expect(create).not.toBeNull();
    expect(create!.snapped).toBe(false);
    expect(create!.targetId).toBeNull();
    // The end position falls back to the raw pointer world position.
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

// The pure mirror of App.svelte's handleCreate snap-target canonicalization (#6):
// a snap is honored ONLY when its target is a real object in the canonical scene
// (the transient drag-create preview / snap-indicator never are), so a self-snap
// onto the preview is dropped while a real-edge snap is kept.
function canonicalizeSnap(
  sceneIds: string[],
  snapped: boolean,
  targetId: string | null
): { snapped: boolean; target: string | null } {
  const target = targetId !== null && sceneIds.includes(targetId) ? targetId : null;
  return { snapped: snapped && target !== null, target };
}

describe("handleCreate snap-target canonicalization (W3-G5 #6)", () => {
  const sceneIds = ["rect-1", "ell-1"];

  it("keeps a snap whose target is a real canonical object (ring + AP5 fire)", () => {
    expect(canonicalizeSnap(sceneIds, true, "rect-1")).toEqual({ snapped: true, target: "rect-1" });
  });

  it("drops a phantom self-snap onto the transient create-preview", () => {
    // The renderer can return the preview's own id when nothing real is near; that
    // must NOT count as a snap (no phantom ring, no anchor to a discarded object).
    expect(canonicalizeSnap(sceneIds, true, "create-preview")).toEqual({ snapped: false, target: null });
    expect(canonicalizeSnap(sceneIds, true, "create-snap-indicator")).toEqual({ snapped: false, target: null });
  });

  it("a non-snapped move stays non-snapped", () => {
    expect(canonicalizeSnap(sceneIds, false, null)).toEqual({ snapped: false, target: null });
  });

  it("App.svelte handleCreate filters the snap target to canonical objects", () => {
    const source = readFileSync(fileURLToPath(new URL("../src/client/svelte/App.svelte", import.meta.url)), "utf8");
    expect(source).toMatch(/scene\.objects\.some\(\(o\)\s*=>\s*o\.id\s*===\s*targetIdIn\)\s*\?\s*targetIdIn\s*:\s*null/);
    expect(source).toMatch(/snappedIn\s*&&\s*targetId\s*!==\s*null/);
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
