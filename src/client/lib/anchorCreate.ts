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
  type Object as SceneObject,
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
