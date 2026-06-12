import { describe, expect, it } from "vitest";
import { ShapeCanvasEngine, createMoveEmission, type EngineEvent } from "../platforms/web/renderer/engine";
import { buildFeedScene, canonicalizeHoverSnap } from "../platforms/web/controller/interactions";
import { THEME_DEFAULT_COLOR } from "../platforms/web/controller/objectPrimitives";
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
  engine.renderFrame(performance.now());
  engine.setTool("create");
  return { engine, fire, events };
}

function lastHover(events: EngineEvent[]): Extract<EngineEvent, { type: "create-hover" }> | null {
  for (let i = events.length - 1; i >= 0; i -= 1) {
    const event = events[i];
    if (event.type === "create-hover") return event;
  }
  return null;
}

describe("createMoveEmission (hover-vs-drag classification)", () => {
  it("a button-up create move is a HOVER probe (NOT drag-gated)", () => {
    expect(createMoveEmission({ dragActive: false })).toBe("hover");
  });

  it("a move during an active drag stays the regular rubber-band drag move", () => {
    expect(createMoveEmission({ dragActive: true })).toBe("drag");
  });
});

describe("create-hover engine probe", () => {
  // Non-identity pan/zoom proves the engine uses the live camera.
  const camera = { x: 40, y: 30, zoom: 0.5 };
  const toScreen = (wx: number, wy: number) => ({ clientX: wx * camera.zoom + camera.x, clientY: wy * camera.zoom + camera.y });

  it("emits create-hover snapped=true + real targetId on a BARE mouse hover (no button down)", () => {
    const { fire, events } = makeEngine(camera);
    // No mousedown, just a bare hover over the outline.
    const onOutline = toScreen(200, OUTLINE_Y);
    fire("mousemove", { button: 0, buttons: 0, altKey: false, clientX: onOutline.clientX, clientY: onOutline.clientY });

    const hover = lastHover(events);
    expect(hover).not.toBeNull();
    expect(hover!.snapped).toBe(true);
    // The REAL object's id, not the transient preview (exclude list forwarded).
    expect(hover!.targetId).toBe(TARGET_ID);
    expect(hover!.world.x).toBeCloseTo(200, 6);
    expect(hover!.world.y).toBeCloseTo(OUTLINE_Y, 6);
    // A bare hover must NEVER author a drag create event.
    expect(events.some((e) => e.type === "create")).toBe(false);
  });

  it("emits create-hover snapped=false + null targetId when the hover is off any edge", () => {
    const { fire, events } = makeEngine(camera);
    const farOff = toScreen(200, 900);
    fire("mousemove", { button: 0, buttons: 0, altKey: false, clientX: farOff.clientX, clientY: farOff.clientY });

    const hover = lastHover(events);
    expect(hover).not.toBeNull();
    expect(hover!.snapped).toBe(false);
    expect(hover!.targetId).toBeNull();
  });

  it("honors the Alt snap-bypass — a hover under Alt suggests no anchor", () => {
    const { fire, events } = makeEngine(camera);
    const onOutline = toScreen(200, OUTLINE_Y);
    fire("mousemove", { button: 0, buttons: 0, altKey: true, clientX: onOutline.clientX, clientY: onOutline.clientY });

    const hover = lastHover(events);
    expect(hover).not.toBeNull();
    expect(hover!.snapped).toBe(false);
    expect(hover!.targetId).toBeNull();
  });

  it("a bare hover does NOT fire under the select tool (the ring is create-only)", () => {
    const { engine, fire, events } = makeEngine(camera);
    engine.setTool("select");
    const onOutline = toScreen(200, OUTLINE_Y);
    fire("mousemove", { button: 0, buttons: 0, altKey: false, clientX: onOutline.clientX, clientY: onOutline.clientY });

    expect(lastHover(events)).toBeNull();
  });
});

// A hover snap is honored as a ring only when its target is a REAL object in the
// canonical scene (the transient preview / snap-indicator never are).
function sceneOf(ids: string[]): ObjectScene {
  return { ...emptyObjectScene(), objects: ids.map((id) => ({ id, order: "a0", geometry: { d: "M 0 0 L 8 0" } }) as SceneObject) };
}

describe("canonicalizeHoverSnap (handleCreateHover)", () => {
  const scene = sceneOf(["rect-1", "ell-1"]);

  it("sets the ring when snapped onto a real canonical object", () => {
    expect(canonicalizeHoverSnap(scene, true, "rect-1", { x: 7, y: 9 })).toEqual({ at: { x: 7, y: 9 }, target: "rect-1" });
  });

  it("clears the ring when not snapped", () => {
    expect(canonicalizeHoverSnap(scene, false, null, { x: 0, y: 0 })).toBeNull();
  });

  it("clears the ring for a phantom self-snap onto the transient preview", () => {
    expect(canonicalizeHoverSnap(scene, true, "create-preview", { x: 0, y: 0 })).toBeNull();
    expect(canonicalizeHoverSnap(scene, true, "create-snap-indicator", { x: 0, y: 0 })).toBeNull();
  });
});

// A hover snap with no drag in progress appends one snap-indicator ring; a drag's
// own preview takes over so the ring is never doubled.
describe("buildFeedScene renders the persistent hover ring", () => {
  const base = sceneOf(["rect-1"]);
  const order = () => "z0";

  it("appends a snap-indicator ring from the hover snap when no drag is in progress", () => {
    const feed = buildFeedScene(base, null, null, null, { at: { x: 200, y: 200 }, target: "rect-1" }, order, THEME_DEFAULT_COLOR, 2);
    const added = feed.objects.filter((o) => !base.objects.some((b) => b.id === o.id));
    expect(added.map((o) => o.id)).toEqual(["create-snap-indicator"]);
  });

  it("does NOT append the hover ring while a drag-create is in progress (no doubled ring)", () => {
    // A drag's own preview/snap-indicator takes over: the standalone hover ring is
    // suppressed (the else-if branch), so only the drag preview objects are added.
    const feed = buildFeedScene(
      base,
      null,
      "rectangle",
      { span: { start: { x: 0, y: 0 }, end: { x: 50, y: 50 } }, snapped: true },
      { at: { x: 200, y: 200 }, target: "rect-1" },
      order,
      THEME_DEFAULT_COLOR,
      2
    );
    const addedIds = feed.objects.filter((o) => !base.objects.some((b) => b.id === o.id)).map((o) => o.id);
    // The drag preview + its own snap-indicator; the standalone hover ring is NOT doubled.
    expect(addedIds).toEqual(["create-preview", "create-snap-indicator"]);
  });

  it("leaves the feed untouched when there is no pen / drag / hover", () => {
    const feed = buildFeedScene(base, null, null, null, null, order, THEME_DEFAULT_COLOR, 2);
    expect(feed).toBe(base);
  });
});

function emptyFrameStats(): RustWebGpuFrameStats {
  return {
    visibleGroupCount: 0,
    visibleCardCount: 0,
    visibleEdgeCount: 0,
    textLayoutCacheHits: 0,
    textLayoutCacheMisses: 0,
    totalCards: 0,
    totalEdges: 0,
    vertexCount: 0,
    drawnVertexCount: 0,
    drawRangeCount: 0
  } as unknown as RustWebGpuFrameStats;
}
