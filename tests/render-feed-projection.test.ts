// The model stores stroke width/dash in quantized units (GEOMETRY_QUANTUM_PER_PX per
// px); the feed must divide back to logical px so the renderer does not draw 8x thick.

import { beforeAll, describe, expect, it } from "vitest";
import { objectSceneToRenderObjectScene } from "../platforms/web/controller/canvasHost";
import { ensureSceneCore, loadSceneCore, type SceneCore } from "../platforms/web/bridge/sceneCoreWasm";
import {
  GEOMETRY_QUANTUM_PER_PX,
  emptyObjectScene,
  type ObjectScene,
  type Stroke
} from "../platforms/web/shared/object";

describe("objectSceneToRenderObjectScene stroke units", () => {
  let core: SceneCore;
  beforeAll(async () => {
    await ensureSceneCore();
    core = await loadSceneCore();
  });

  it("projects stroke width as source width / GEOMETRY_QUANTUM_PER_PX", () => {
    const object = core.buildPrimitive("rectangle", { x: 0, y: 0 }, "o1", "a0");
    const sourceWidth = (object.stroke as Stroke).width;
    const scene: ObjectScene = {
      ...emptyObjectScene(),
      objects: [object]
    };

    const projected = objectSceneToRenderObjectScene(
      scene,
      { x: 0, y: 0, zoom: 1 },
      { kind: "canvas" },
      "test-scene"
    );

    const projectedObject = (projected.objects as Array<Record<string, unknown>>)[0];
    const projectedStroke = projectedObject.stroke as { width: number };
    expect(projectedStroke.width).toBe(sourceWidth / GEOMETRY_QUANTUM_PER_PX);
  });
});
