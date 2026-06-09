// U1 — object primitive input/overlay glue.
//
// Tier-3 moved primitive geometry construction + recolor/paint into the Rust core
// (scene-core `object::primitives`, bridged as `sceneCore.buildPrimitive` /
// `buildPrimitiveFromDrag` / `buildSetStyleOp`). What remains here is the thin
// shell glue the core does not own: the drag-span input type, the click-vs-drag
// threshold, the inline-text overlay placement math, and the theme-default UI
// sentinel (the *selection* value; its mapping to a Paint lives in the core).

import type { CameraState } from "../../shared/geometry";
import { GEOMETRY_QUANTUM_PER_PX, type Object as SceneObject } from "../../shared/object";
import { worldToScreen, type WorldRect } from "../renderer/scene";

const Q = GEOMETRY_QUANTUM_PER_PX;

// S2 (#5): the sentinel a user color carries when "Theme default" is picked. It is
// NOT a CSS hex — the core's `paint_for_color` maps it to a `Paint::Token { name:
// "text" }` so the authored object follows the theme. The shell keeps this literal
// only as a UI *selection* value (which swatch is active); the canonical
// sentinel→Paint rule lives in scene-core (build_primitive / build_set_style_op).
export const THEME_DEFAULT_COLOR = "token:text";

// W2-07: a drag span — the gesture's start corner and current/end corner (world
// px). Closed primitives (rect/ellipse/frame/text) are sized to the normalized
// bbox; the open line runs corner-to-corner so a diagonal drag draws a diagonal.
export type DragSpan = { start: { x: number; y: number }; end: { x: number; y: number } };

// Smallest extent (logical px) a drag must reach before a closed primitive is
// considered sized; below this the caller treats the gesture as a click.
export const MIN_DRAG_EXTENT_PX = 4;

// W2-10: the on-screen rect to place the inline text-edit overlay over, in canvas-
// local CSS px. The object's geometry path is object-local quantized integers; its
// world AABB is the path's local bbox (de-quantized) run through the object's affine
// transform, then projected to screen via {@link worldToScreen}. Pure so the shell
// test can pin the placement without a renderer or a Svelte mount. Returns null when
// the path carries no coordinate pairs.
export function textOverlayScreenRect(object: SceneObject, camera: CameraState): WorldRect | null {
  const local = pathLocalBbox(object.geometry.d);
  if (!local) return null;
  const t = object.transform;
  const corners: Array<[number, number]> = [
    [local.minX, local.minY],
    [local.maxX, local.minY],
    [local.maxX, local.maxY],
    [local.minX, local.maxY]
  ];
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const [lx, ly] of corners) {
    const wx = t ? t[0][0] * lx + t[0][1] * ly + t[0][2] : lx;
    const wy = t ? t[1][0] * lx + t[1][1] * ly + t[1][2] : ly;
    minX = Math.min(minX, wx);
    minY = Math.min(minY, wy);
    maxX = Math.max(maxX, wx);
    maxY = Math.max(maxY, wy);
  }
  const topLeft = worldToScreen({ x: minX, y: minY }, camera);
  const bottomRight = worldToScreen({ x: maxX, y: maxY }, camera);
  return { x: topLeft.x, y: topLeft.y, width: bottomRight.x - topLeft.x, height: bottomRight.y - topLeft.y };
}

// The object-local bbox (logical px) of a path-string's coordinate pairs. Coords are
// quantized integers (GEOMETRY_QUANTUM_PER_PX per px); reading every numeric pair
// covers M/L/C control points — a conservative enclosing box for the overlay.
function pathLocalBbox(d: string): { minX: number; minY: number; maxX: number; maxY: number } | null {
  const nums = d?.match(/-?\d+(?:\.\d+)?/g);
  if (!nums || nums.length < 2) return null;
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (let i = 0; i + 1 < nums.length; i += 2) {
    const x = Number(nums[i]) / Q;
    const y = Number(nums[i + 1]) / Q;
    minX = Math.min(minX, x);
    minY = Math.min(minY, y);
    maxX = Math.max(maxX, x);
    maxY = Math.max(maxY, y);
  }
  return Number.isFinite(minX) ? { minX, minY, maxX, maxY } : null;
}
