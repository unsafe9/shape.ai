// U1 — object primitive input/overlay glue.
//
// Tier-3 moved primitive geometry construction + recolor/paint into the Rust core
// (scene-core `object::primitives`, bridged as `sceneCore.buildPrimitive` /
// `buildPrimitiveFromDrag` / `buildSetStyleOp`). What remains here is the thin
// shell glue the core does not own: the drag-span input type, the click-vs-drag
// threshold, the inline-text overlay placement math, and the theme-default UI
// sentinel (the *selection* value; its mapping to a Paint lives in the core).

import type { CameraState } from "../shared/geometry";
import {
  GEOMETRY_QUANTUM_PER_PX,
  type Anchor,
  type Object as SceneObject,
  type ObjectOp,
  type ObjectScene,
  type Transform3x3
} from "../shared/object";
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

// AP5/#4: a create gesture's most recent successful outline snap — the world point
// its corner snapped to plus the real object it bound. Tracked (sticky) across the
// drag so a release that itself misses the snap can still author the anchor the
// ring was clearly promising.
export type CreateSnap = { at: { x: number; y: number }; target: string };

// AP5/#4: screen-px radius within which a release that MISSED the outline snap
// reuses the gesture's last in-flight snap. The renderer's per-move snap tolerance
// is 8px (engine `CREATE_SNAP_TOLERANCE_PX`); a release a little past that — the
// pointer-up landing just off the edge the ring was hugging — should still bind, so
// this is wider. Caller divides by zoom to get world units.
export const CREATE_ANCHOR_REUSE_TOLERANCE_PX = 24;

// v3 §4 multi-stroke merge: screen-px radius within which a freehand release
// END landing on an existing open-class object's ENDPOINT merges the stroke
// into that object instead of inserting it. Caller divides by zoom to get the
// world units the core's merge_open_stroke_ops takes (the same convention as
// CREATE_ANCHOR_REUSE_TOLERANCE_PX); the judgment itself lives in the core.
export const MERGE_ENDPOINT_TOLERANCE_PX = 12;

// AP5/#4: resolve a shape drag-create RELEASE to its final endpoint + anchor target.
// When the release itself snapped, honor it. Otherwise, when the release landed
// within `reuseTolerance` (WORLD units) of the gesture's last snap, reuse that snap
// (endpoint pulled onto the edge, target bound) so a near-miss release still authors
// the anchor instead of silently dropping it — the #1 cause of "anchored objects
// never follow" was the pointer-up missing the 8px snap the hover ring had shown.
// Pure + falsifiable (no renderer/camera): the caller passes the world tolerance.
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

// AP5 (#14)/#4/v3 §4: the release-time anchor authoring shared by shape drag-create
// AND the freehand pen — BOTH gesture corners (start + end) bind the created
// object's node nearest that corner to the snapped target's outline (G13's
// both-corner loop). Pure composition over the core's synthesizeCreateAnchors (the
// geometry judgment stays in the core); a null corner (never snapped) or a corner
// whose target is no longer in the scene authors nothing. The caller decides
// eligibility (e.g. v3 DU7(b): a recognized CLOSED stroke never calls this).
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
    // One anchor per node (the D5 invariant): a degenerate gesture whose corners
    // resolve to the SAME nearest node (e.g. a pen tap on an edge) keeps only the
    // first binding instead of authoring a conflicting duplicate.
    if (a) anchors.push(...a.filter((anchor) => !anchors.some((prior) => prior.nodeIndex === anchor.nodeIndex)));
  }
  return anchors;
}

// v3 §3 (DU4) Alt-detach: the commit ops of an Alt-held body drag of an ANCHORED
// open-class object — clear its anchors (one whole-vector set-anchor), then move it
// WHOLE. The move ops are computed by the core against the scene with the dragged
// object's anchors already cleared, so the open-class endpoint routing sees no pins
// and keeps the 0-rebake SetTransform translate (and followers anchored TO the
// dragged object still follow). Pure composition — every judgment is a core call;
// the caller decides eligibility (detach gesture + open-class + anchored).
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
