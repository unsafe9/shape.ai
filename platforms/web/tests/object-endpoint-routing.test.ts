import { beforeAll, describe, expect, it } from "vitest";

import { ShapeCanvasEngine, type EngineEvent } from "../renderer/engine";
import { ensureSceneCore, loadSceneCore, type SceneCore } from "../bridge/sceneCoreWasm";
import { endpointReleaseOp, endpointSnapTarget } from "../controller/interactions";
import {
  emptyObjectScene,
  translateTransform,
  type Object as SceneObject,
  type ObjectScene
} from "../shared/object";
import type { RustInputBatchResult, RustWebGpuRenderer } from "../bridge/wasmLoader";
import type { CameraState } from "../renderer/scene";

const CAMERA: CameraState = { x: 0, y: 0, zoom: 1 };
const LINE_ID = "line-1";
const TARGET_ID = "rect-1";
// Mock outline: horizontal segment y=200, x in [100, 300] (rect-1's edge).
const OUTLINE_Y = 200;
const OUTLINE_X0 = 100;
const OUTLINE_X1 = 300;

let core: SceneCore;
beforeAll(async () => {
  await ensureSceneCore();
  core = await loadSceneCore();
});

type PreviewCall = { id: string; nodeIndex: number; x: number; y: number };

// Fake renderer: emits an objectEndpointDelta per pointer-move, carries the
// outline-snap contract, and records the engine's endpoint preview pushes. The snap
// target is the dragged LINE itself (a phantom self-snap) unless the engine excludes
// it — then the real rect id comes back.
function endpointRenderer(samples: Array<{ nodeIndex: number; x: number; y: number }>) {
  const previews: PreviewCall[] = [];
  const cleared: string[] = [];
  let moveIndex = 0;
  const renderer = {
    resize() {},
    renderFrame() {
      return { backend: "test" } as unknown as ReturnType<RustWebGpuRenderer["renderFrame"]>;
    },
    inputBatch(eventsJson: string): RustInputBatchResult {
      const events = JSON.parse(eventsJson) as Array<{ kind: string }>;
      const isMove = events.some((e) => e.kind === "pointer-move");
      const sample = isMove ? samples[Math.min(moveIndex++, samples.length - 1)] : null;
      return {
        camera: CAMERA,
        objectEndpointDelta: sample ? { id: LINE_ID, ...sample } : null
      };
    },
    nearestOutlinePoint(worldX: number, worldY: number, tolPx: number, zoom: number, excludeIdsJson: string) {
      const tolWorld = tolPx / Math.max(0.025, zoom);
      const nearestX = Math.min(OUTLINE_X1, Math.max(OUTLINE_X0, worldX));
      const dx = worldX - nearestX;
      const dy = worldY - OUTLINE_Y;
      if (dx * dx + dy * dy > tolWorld * tolWorld) return { snapped: false, x: 0, y: 0, targetId: null };
      let excluded: string[] = [];
      try {
        excluded = JSON.parse(excludeIdsJson ?? "[]");
      } catch {
        excluded = [];
      }
      // The dragged line's own outline sits under the cursor: it wins the snap
      // unless the engine excludes it.
      const targetId = excluded.includes(LINE_ID) ? TARGET_ID : LINE_ID;
      return { snapped: true, x: nearestX, y: OUTLINE_Y, targetId };
    },
    setObjectEndpointPreview(id: string, nodeIndex: number, x: number, y: number) {
      previews.push({ id, nodeIndex, x, y });
    },
    clearObjectEndpointPreview(id: string) {
      cleared.push(id);
    }
  } as unknown as RustWebGpuRenderer;
  return { renderer, previews, cleared };
}

function recordingCanvas(): { canvas: HTMLCanvasElement; fire: (type: string, event: unknown) => void } {
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
  return {
    canvas: element as unknown as HTMLCanvasElement,
    fire: (type, event) => {
      for (const listener of listeners.get(type) ?? []) listener(event as Event);
    }
  };
}

function driveEndpointDrag(
  samples: Array<{ nodeIndex: number; x: number; y: number }>,
  options: { altHeld?: boolean; end?: "up" | "cancel" } = {}
) {
  const events: EngineEvent[] = [];
  const { canvas, fire } = recordingCanvas();
  const { renderer, previews, cleared } = endpointRenderer(samples);
  // eslint-disable-next-line no-new
  new ShapeCanvasEngine({
    canvas,
    overlayRoot: { append() {} } as unknown as HTMLElement,
    backend: "test",
    webGpuRenderer: renderer,
    onEvent: (event) => events.push(event)
  });
  const alt = options.altHeld ?? false;
  const base = { pointerId: 7, pointerType: "pen", shiftKey: false, altKey: alt, metaKey: false, ctrlKey: false };
  fire("pointerdown", { ...base, button: 0, clientX: 10, clientY: 10, preventDefault() {} });
  for (let i = 0; i < samples.length; i += 1) {
    fire("pointermove", { ...base, clientX: 20 + i, clientY: 20, preventDefault() {} });
  }
  fire(options.end === "cancel" ? "pointercancel" : "pointerup", { ...base, clientX: 30, clientY: 20, preventDefault() {} });
  const commits = events.filter((e) => e.type === "object-endpoint-commit");
  const previewsEmitted = events.filter((e) => e.type === "object-endpoint-preview");
  return { events, commits, previewsEmitted, previewCalls: previews, cleared };
}

describe("engine endpoint-drag routing", () => {
  it("routes a snapped endpoint drag: live preview at the snapped point + ONE commit with the real target", () => {
    // Sample lands 4px under rect-1's edge, within the 8px tolerance.
    const { commits, previewsEmitted, previewCalls } = driveEndpointDrag([{ nodeIndex: 1, x: 200, y: 204 }]);

    // Pushed at the SNAPPED point, not the raw sample.
    expect(previewCalls).toEqual([{ id: LINE_ID, nodeIndex: 1, x: 200, y: OUTLINE_Y }]);
    expect(previewsEmitted).toEqual([
      { type: "object-endpoint-preview", id: LINE_ID, nodeIndex: 1, world: { x: 200, y: OUTLINE_Y }, snapped: true, targetId: TARGET_ID }
    ]);
    // One commit, carrying the REAL target — the dragged id was excluded from the
    // snap query, so a self-snap would surface LINE_ID here and fail.
    expect(commits).toEqual([
      { type: "object-endpoint-commit", id: LINE_ID, nodeIndex: 1, world: { x: 200, y: OUTLINE_Y }, snapped: true, targetId: TARGET_ID }
    ]);
  });

  it("bypasses the release snap while Alt is held", () => {
    const { commits, previewCalls } = driveEndpointDrag([{ nodeIndex: 1, x: 200, y: 204 }], { altHeld: true });
    expect(previewCalls).toEqual([{ id: LINE_ID, nodeIndex: 1, x: 200, y: 204 }]);
    expect(commits).toEqual([
      { type: "object-endpoint-commit", id: LINE_ID, nodeIndex: 1, world: { x: 200, y: 204 }, snapped: false, targetId: null }
    ]);
  });

  it("emits no commit and reverts the live deform on pointer-cancel", () => {
    const { commits, cleared } = driveEndpointDrag([{ nodeIndex: 1, x: 200, y: 204 }], { end: "cancel" });
    expect(commits).toEqual([]);
    expect(cleared).toEqual([LINE_ID]);
  });

  it("emits nothing when no endpoint drag was in flight", () => {
    const { commits, previewsEmitted } = driveEndpointDrag([]);
    expect(commits).toEqual([]);
    expect(previewsEmitted).toEqual([]);
  });
});

function obj(partial: Partial<SceneObject> & { id: string; geometry: SceneObject["geometry"] }): SceneObject {
  return { order: "a0", ...partial } as SceneObject;
}

// rect-1: 100x100 rect at world (100,100); line-1: 200px horizontal line at the
// world origin, endpoint indices 0 and 1.
function releaseScene(lineAnchors?: SceneObject["anchors"]): ObjectScene {
  return {
    ...emptyObjectScene(),
    objects: [
      obj({
        id: TARGET_ID,
        geometry: { d: "M 0 0 L 800 0 L 800 800 L 0 800 Z", fillRule: "nonZero" },
        transform: translateTransform(100, 100)
      }),
      obj({
        id: LINE_ID,
        order: "a1",
        geometry: { d: "M 0 0 L 1600 0", fillRule: "nonZero" },
        ...(lineAnchors ? { anchors: lineAnchors } : {})
      })
    ]
  };
}

describe("endpointReleaseOps commit contract (the App onEndpointCommit call)", () => {
  it("a snapped release authors the chord-deform edit-geometry + the set-anchor rebind", () => {
    const ops = core.endpointReleaseOps(releaseScene(), LINE_ID, 1, { x: 200, y: 200 }, { targetId: TARGET_ID, at: { x: 200, y: 200 } });
    expect(ops.map((o) => o.kind)).toEqual(["edit-geometry", "set-anchor"]);
    const [geo, anchor] = ops;
    if (geo.kind !== "edit-geometry" || anchor.kind !== "set-anchor") throw new Error("unexpected op kinds");
    // Endpoint 1 moved (200,0)->(200,200); chord deform rewrites d in quantized units (8/px).
    expect(geo.id).toBe(LINE_ID);
    expect(geo.geometry.d).toBe("M 0 0 L 1600 1600");
    // node 1 anchored at target-local quantized point: world (200,200) - rect (100,100)
    // = local 100px = 800 units.
    expect(anchor.id).toBe(LINE_ID);
    expect(anchor.anchors).toEqual([{ nodeIndex: 1, target: TARGET_ID, at: { x: 800, y: 800 } }]);
  });

  it("an empty-space release unbinds the dragged endpoint's anchor", () => {
    const anchored = releaseScene([{ nodeIndex: 1, target: TARGET_ID, at: { x: 800, y: 800 } }]);
    const ops = core.endpointReleaseOps(anchored, LINE_ID, 1, { x: 500, y: 40 }, null);
    const anchorOp = ops.find((o) => o.kind === "set-anchor");
    expect(anchorOp).toBeDefined();
    if (anchorOp?.kind !== "set-anchor") throw new Error("expected set-anchor");
    expect(anchorOp.anchors).toEqual([]);
  });

  it("returns [] for a closed-class id (no endpoint surface)", () => {
    expect(core.endpointReleaseOps(releaseScene(), TARGET_ID, 0, { x: 0, y: 0 }, null)).toEqual([]);
  });
});

describe("controller endpoint wiring", () => {
  it("onEndpointCommit authors the release through the core endpointReleaseOps, snap canonicalized", () => {
    // A snapped release onto rect-1 -> chord-deform edit-geometry + rebind, in ONE batch.
    const target = endpointSnapTarget(releaseScene(), LINE_ID, true, TARGET_ID);
    expect(target).toBe(TARGET_ID);
    const op = endpointReleaseOp(core, releaseScene(), LINE_ID, 1, { x: 200, y: 200 }, target);
    expect(op).not.toBeNull();
    expect(op!.kind).toBe("batch");
    if (op!.kind !== "batch") throw new Error("expected batch");
    expect(op!.ops.map((o) => o.kind)).toEqual(["edit-geometry", "set-anchor"]);
  });

  it("onEndpointCommit returns null (no op) on a no-op release so the shell reverts the deform", () => {
    expect(endpointReleaseOp(core, releaseScene(), TARGET_ID, 0, { x: 0, y: 0 }, null)).toBeNull();
  });

  it("the preview snap probe honors a real OTHER object and rejects a self-snap (drives the ring)", () => {
    const scene = releaseScene();
    expect(endpointSnapTarget(scene, LINE_ID, true, TARGET_ID)).toBe(TARGET_ID);
    expect(endpointSnapTarget(scene, LINE_ID, true, LINE_ID)).toBeNull();
    expect(endpointSnapTarget(scene, LINE_ID, false, TARGET_ID)).toBeNull();
    expect(endpointSnapTarget(scene, LINE_ID, true, "ghost")).toBeNull();
  });
});
