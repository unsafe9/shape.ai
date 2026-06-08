// FC-02 regression — the renderer feed de-quantizes stroke units.
//
// The model stores stroke width/dash in QUANTIZED units (GEOMETRY_QUANTUM_PER_PX
// per px); `objectSceneToRenderObjectScene` must divide them back to logical px
// so the renderer (which treats RStroke.width as px) does not draw 8x too thick.
// Node-only, no GPU.

import { describe, expect, it } from "vitest";
import { objectSceneToRenderObjectScene } from "../src/client/lib/canvasHost";
import { buildPrimitiveObject } from "../src/client/lib/objectPrimitives";
import {
  GEOMETRY_QUANTUM_PER_PX,
  emptyObjectScene,
  type ObjectScene,
  type Stroke
} from "../src/shared/object";

describe("objectSceneToRenderObjectScene stroke units", () => {
  it("projects stroke width as source width / GEOMETRY_QUANTUM_PER_PX", () => {
    const object = buildPrimitiveObject("rectangle", { x: 0, y: 0 }, "o1", "a0");
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
