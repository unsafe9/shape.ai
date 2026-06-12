// Object primitive input/overlay glue the core does not own: the drag-span input type, the
// click-vs-drag threshold, the inline-text overlay placement math, and the theme-default UI sentinel.

import type { CameraState } from "../shared/geometry";
import {
  type Anchor,
  type Object as SceneObject,
  type ObjectOp,
  type ObjectScene,
  type Transform3x3
} from "../shared/object";
import { worldToScreen, type WorldRect } from "../renderer/scene";
import { pathLocalBbox } from "./transforms";

// The sentinel a user color carries when "Theme default" is picked. NOT a CSS hex — the core's
// `paint_for_color` maps it to a `Paint::Token { name: "text" }` so the authored object follows the
// theme. The shell keeps this literal only as a UI selection value; the sentinel→Paint rule lives in scene-core.
export const THEME_DEFAULT_COLOR = "token:text";

// A drag span — the gesture's start corner and current/end corner (world px). Closed primitives are
// sized to the normalized bbox; the open line runs corner-to-corner so a diagonal drag draws a diagonal.
export type DragSpan = { start: { x: number; y: number }; end: { x: number; y: number } };

// Smallest extent (logical px) a drag must reach before a closed primitive counts as sized; below this the gesture is a click.
export const MIN_DRAG_EXTENT_PX = 4;

// A create gesture's most recent successful outline snap (world point + bound object), tracked
// sticky across the drag so a release that misses the snap can still author the anchor.
export type CreateSnap = { at: { x: number; y: number }; target: string };

// Screen-px radius within which a release that MISSED the outline snap reuses the gesture's last
// in-flight snap (wider than the per-move 8px tolerance so a near-miss release still binds). Caller divides by zoom.
export const CREATE_ANCHOR_REUSE_TOLERANCE_PX = 24;

// Screen-px radius within which a freehand release END landing on an open-class object's ENDPOINT
// merges the stroke into it instead of inserting. Caller divides by zoom; the merge judgment lives in the core.
export const MERGE_ENDPOINT_TOLERANCE_PX = 12;

// Resolve a shape drag-create RELEASE to its final endpoint + anchor target: honor the release's own
// snap, else reuse the gesture's last snap when the release landed within `reuseTolerance` (WORLD units),
// so a near-miss release still authors the anchor instead of dropping it.
export function resolveCreateRelease(
  release: { end: { x: number; y: number }; snapped: boolean; target: string | null },
  lastSnap: CreateSnap | null,
  reuseTolerance: number
): { end: { x: number; y: number }; target: string | null } {
  if (release.snapped && release.target) return { end: release.end, target: release.target };
  if (lastSnap) {
    const dx = release.end.x - lastSnap.at.x;
    const dy = release.end.y - lastSnap.at.y;
    if (dx * dx + dy * dy <= reuseTolerance * reuseTolerance) {
      return { end: lastSnap.at, target: lastSnap.target };
    }
  }
  return { end: release.end, target: null };
}

// Release-time anchor authoring shared by shape drag-create AND the freehand pen: both gesture
// corners (start + end) bind the created object's nearest node to the snapped target's outline.
// Composition over the core's synthesizeCreateAnchors; a null corner or a corner whose target left
// the scene authors nothing. The caller decides eligibility (a recognized CLOSED stroke never calls this).
export function synthesizeReleaseAnchors(
  core: {
    synthesizeCreateAnchors(
      created: SceneObject,
      target: SceneObject,
      endpoint: { x: number; y: number }
    ): Anchor[] | null;
  },
  objects: readonly SceneObject[],
  created: SceneObject,
  corners: ReadonlyArray<{ target: string; at: { x: number; y: number } } | null>
): Anchor[] {
  const anchors: Anchor[] = [];
  for (const corner of corners) {
    if (!corner) continue;
    const target = objects.find((o) => o.id === corner.target);
    if (!target) continue;
    const a = core.synthesizeCreateAnchors(created, target, corner.at);
    // One anchor per node: corners resolving to the SAME nearest node keep only the first binding.
    if (a) anchors.push(...a.filter((anchor) => !anchors.some((prior) => prior.nodeIndex === anchor.nodeIndex)));
  }
  return anchors;
}

// Alt-detach: commit ops of an Alt-held body drag of an ANCHORED open-class object — clear its
// anchors (one whole-vector set-anchor), then move it WHOLE. Move ops run against the scene with the
// dragged object's anchors already cleared, so endpoint routing sees no pins and keeps the 0-rebake
// SetTransform translate (followers anchored TO the dragged object still follow).
export function altDetachOps(
  core: { moveOps(scene: ObjectScene, roots: { kind: "single"; id: string }, delta: Transform3x3): ObjectOp[] },
  scene: ObjectScene,
  id: string,
  delta: Transform3x3
): ObjectOp[] {
  const detached: ObjectScene = {
    ...scene,
    objects: scene.objects.map((o) => (o.id === id ? { ...o, anchors: [] } : o))
  };
  return [{ kind: "set-anchor", id, anchors: [] }, ...core.moveOps(detached, { kind: "single", id }, delta)];
}

// The on-screen rect (canvas-local CSS px) to place the inline text-edit overlay over: the path's
// local bbox (de-quantized) run through the object's affine transform, then projected via
// `worldToScreen`. Null when the path carries no coordinate pairs.
export function textOverlayScreenRect(object: SceneObject, camera: CameraState): WorldRect | null {
  const local = pathLocalBbox(object.geometry.d ?? "");
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
