// Object primitive input/overlay glue the core does not own: the drag-span input type, the
// inline-text overlay placement math, and the theme-default UI sentinel.

import type { WorldPoint, WorldRect } from "../shared/geometry";

// The sentinel a user color carries when "Theme default" is picked. NOT a CSS hex — the core's
// `paint_for_color` maps it to a `Paint::Token { name: "text" }` so the authored object follows the
// theme. The shell keeps this literal only as a UI selection value; the sentinel→Paint rule lives in scene-core.
export const THEME_DEFAULT_COLOR = "token:text";

// A drag span — the gesture's start corner and current/end corner (world px). Closed primitives are
// sized to the normalized bbox; the open line runs corner-to-corner so a diagonal drag draws a diagonal.
export type DragSpan = { start: { x: number; y: number }; end: { x: number; y: number } };

// A create gesture's most recent successful outline snap (world point + bound object), tracked
// sticky across the drag so a release that misses the snap can still author the anchor.
export type CreateSnap = { at: { x: number; y: number }; target: string };

// The on-screen rect (canvas-local CSS px) to place the inline text-edit overlay over: the object's
// WORLD-space AABB (from scene-core `objectWorldAabb` — the geometry+transform math stays in the core)
// projected to screen through the LIVE core camera. Null when the object has no bbox or the renderer
// can't project (not yet live).
export function textOverlayScreenRect(
  worldAabb: { minX: number; minY: number; maxX: number; maxY: number } | null,
  projectWorldToScreen: (world: WorldPoint) => WorldPoint | null
): WorldRect | null {
  if (!worldAabb) return null;
  const topLeft = projectWorldToScreen({ x: worldAabb.minX, y: worldAabb.minY });
  const bottomRight = projectWorldToScreen({ x: worldAabb.maxX, y: worldAabb.maxY });
  if (!topLeft || !bottomRight) return null;
  return { x: topLeft.x, y: topLeft.y, width: bottomRight.x - topLeft.x, height: bottomRight.y - topLeft.y };
}
