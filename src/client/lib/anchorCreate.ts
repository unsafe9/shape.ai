// AP5 (#14) — drag-create anchoring.
//
// When a shape drag-create snaps its dragged endpoint to a target object's
// outline (the W2-06 snap query reports `targetId`), the created object should
// stay bound to that target: a persistent D5 `Anchor` is synthesized on the new
// object so the endpoint reprojects as the target moves. These are pure helpers
// (no scene mutation, no renderer, no op-apply) so the shell only wires them and
// the verification can pin them without a Svelte mount.
//
// The endpoint position is never stored on the anchored object — only `at` (the
// snap point in the target's LOCAL quantized space) is stored, and the world
// endpoint is re-derived from the target's CURRENT transform (the OB3.S4
// reproject step mirrored shell-side for a transform-only move).

import {
  GEOMETRY_QUANTUM_PER_PX,
  IDENTITY_TRANSFORM,
  type Anchor,
  type Geometry,
  type Object as SceneObject,
  type ObjectOp,
  type Transform3x3
} from "../../shared/object";

const Q = GEOMETRY_QUANTUM_PER_PX;

/** Apply a row-major affine 3x3 to a point (absent transform = identity). */
function applyTransform(t: Transform3x3 | undefined, x: number, y: number): { x: number; y: number } {
  const m = t ?? IDENTITY_TRANSFORM;
  return { x: m[0][0] * x + m[0][1] * y + m[0][2], y: m[1][0] * x + m[1][1] * y + m[1][2] };
}

/** Invert a row-major affine 3x3 (g=h=0, i=1). Returns identity if singular. */
function invertAffine(t: Transform3x3 | undefined): Transform3x3 {
  const m = t ?? IDENTITY_TRANSFORM;
  const [a, b, c] = m[0];
  const [d, e, f] = m[1];
  const det = a * e - b * d;
  if (det === 0) return IDENTITY_TRANSFORM;
  const ia = e / det;
  const ib = -b / det;
  const id = -d / det;
  const ie = a / det;
  return [
    [ia, ib, -(ia * c + ib * f)],
    [id, ie, -(id * c + ie * f)],
    [0, 0, 1]
  ];
}

/** Parsed object-local node anchor points from a path-string's M/L/C coords. */
function localNodes(d: string): Array<{ x: number; y: number }> {
  const nums = d?.match(/-?\d+(?:\.\d+)?/g);
  if (!nums || nums.length < 2) return [];
  const out: Array<{ x: number; y: number }> = [];
  for (let i = 0; i + 1 < nums.length; i += 2) {
    out.push({ x: Number(nums[i]), y: Number(nums[i + 1]) });
  }
  return out;
}

/**
 * The geometry node index of `object` closest to the world point `endpoint`
 * (the dragged, snapped corner). Coordinates are mapped into the object's local
 * quantized space before comparison, so the same point matches regardless of the
 * object's transform. Returns -1 when the geometry carries no nodes.
 */
function nodeIndexNearestWorld(object: SceneObject, endpoint: { x: number; y: number }): number {
  const nodes = localNodes(object.geometry.d);
  if (nodes.length === 0) return -1;
  const inv = invertAffine(object.transform);
  const lx = (inv[0][0] * endpoint.x + inv[0][1] * endpoint.y + inv[0][2]) * Q;
  const ly = (inv[1][0] * endpoint.x + inv[1][1] * endpoint.y + inv[1][2]) * Q;
  let best = 0;
  let bestD = Infinity;
  for (let i = 0; i < nodes.length; i++) {
    const dx = nodes[i].x - lx;
    const dy = nodes[i].y - ly;
    const dist = dx * dx + dy * dy;
    if (dist < bestD) {
      bestD = dist;
      best = i;
    }
  }
  return best;
}

/**
 * Synthesize the persistent anchor(s) for a snapped drag-create, or `undefined`
 * when no anchor should be authored.
 *
 * Authored only when the endpoint actually snapped to a known target
 * (`targetId` present and resolvable). An Alt-create bypasses the snap upstream
 * so `targetId` arrives null and no anchor is produced. The anchor binds the
 * created object's snapped node to the target; `at` is the snapped WORLD point
 * mapped into the target's LOCAL quantized space, so the endpoint reprojects
 * through the target's transform on a later move.
 *
 * `created` is the freshly built (not-yet-applied) object; `target` is the snap
 * target; `endpoint` is the snapped corner in world px.
 */
export function synthesizeCreateAnchors(
  created: SceneObject,
  target: SceneObject | undefined,
  endpoint: { x: number; y: number }
): Anchor[] | undefined {
  if (!target || target.id === created.id) return undefined;
  const nodeIndex = nodeIndexNearestWorld(created, endpoint);
  if (nodeIndex < 0) return undefined;
  const inv = invertAffine(target.transform);
  const at = {
    x: Math.round((inv[0][0] * endpoint.x + inv[0][1] * endpoint.y + inv[0][2]) * Q),
    y: Math.round((inv[1][0] * endpoint.x + inv[1][1] * endpoint.y + inv[1][2]) * Q)
  };
  return [{ nodeIndex, target: target.id, at }];
}

/**
 * Reproject an anchored endpoint to world px through the target's CURRENT
 * transform (the OB3.S4 reproject for a transform-only move). `anchor.at` is in
 * the target's local quantized space; de-quantize then apply the target's
 * transform so the endpoint tracks the target without drift.
 */
export function reprojectAnchoredEndpoint(target: SceneObject, anchor: Anchor): { x: number; y: number } {
  return applyTransform(target.transform, anchor.at.x / Q, anchor.at.y / Q);
}

/**
 * Rewrite the `nodeIndex`-th coordinate pair of a path-string `d` to `(x,y)`
 * (object-local quantized ints), preserving every command token and the rest of
 * the coords. Returns `d` unchanged when the path has no such pair.
 */
function setPathNode(d: string, nodeIndex: number, x: number, y: number): string {
  let pair = 0;
  let numbersSeen = 0;
  return d.replace(/-?\d+(?:\.\d+)?/g, (match) => {
    const isX = numbersSeen % 2 === 0;
    const atTarget = pair === nodeIndex;
    if (!isX) pair++;
    numbersSeen++;
    if (!atTarget) return match;
    return isX ? String(x) : String(y);
  });
}

/**
 * AP5 (#14) live move-together: reproject `anchored`'s anchored node so it tracks
 * `target`'s CURRENT transform, returning the updated geometry — or `undefined`
 * when `anchored` carries no anchor onto `target` (or the node is unaddressable).
 *
 * The world endpoint comes from {@link reprojectAnchoredEndpoint} (the target's
 * transform applied to `anchor.at`); it is then mapped into `anchored`'s OWN local
 * quantized space (the geometry coords are object-local, D2) and written back to
 * the addressed node. Pure (no scene mutation, no op-apply) so it pins shell-side.
 */
export function reprojectAnchoredGeometry(anchored: SceneObject, target: SceneObject): Geometry | undefined {
  const anchor = anchored.anchors?.find((a) => a.target === target.id);
  if (!anchor) return undefined;
  const world = reprojectAnchoredEndpoint(target, anchor);
  const inv = invertAffine(anchored.transform);
  const lx = Math.round((inv[0][0] * world.x + inv[0][1] * world.y + inv[0][2]) * Q);
  const ly = Math.round((inv[1][0] * world.x + inv[1][1] * world.y + inv[1][2]) * Q);
  const d = setPathNode(anchored.geometry.d, anchor.nodeIndex, lx, ly);
  if (d === anchored.geometry.d) return undefined;
  return { ...anchored.geometry, d };
}

/**
 * The `edit-geometry` ops a committed move produces so anchored objects follow
 * their target. `transformOps` are the move's `set-transform` ops (each a moved id
 * + its NEW transform, e.g. the AP2 cascade); for every moved object, each scene
 * object anchored to it has its bound node reprojected through that NEW transform.
 *
 * A moved object whose anchored object is itself being moved in the same batch is
 * skipped (it carries its own transform), and an object anchored to nothing moved
 * authors nothing — so an unanchored (Alt-created) move stays a no-op. Returns
 * `[]` when nothing follows.
 */
export function anchorFollowOps(objects: SceneObject[], transformOps: ObjectOp[]): ObjectOp[] {
  const movedIds = new Set<string>();
  for (const op of transformOps) if (op.kind === "set-transform") movedIds.add(op.id);
  const ops: ObjectOp[] = [];
  for (const op of transformOps) {
    if (op.kind !== "set-transform") continue;
    const movedTarget: SceneObject | undefined = objects.find((o) => o.id === op.id);
    if (!movedTarget) continue;
    const target: SceneObject = { ...movedTarget, transform: op.transform };
    for (const obj of objects) {
      if (movedIds.has(obj.id) || !obj.anchors) continue;
      const geometry = reprojectAnchoredGeometry(obj, target);
      if (geometry) ops.push({ kind: "edit-geometry", id: obj.id, geometry });
    }
  }
  return ops;
}
