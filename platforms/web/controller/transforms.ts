// Pure affine/geometry helpers the op-authoring paths reach for: origin read, structural transform
// equality, translate shift, the inverse affine into object-local quantized space, union world-AABB,
// path-string local bbox, and a quantized rect path.

import {
  GEOMETRY_QUANTUM_PER_PX,
  IDENTITY_TRANSFORM,
  translateTransform,
  type Object as SceneObject,
  type Transform3x3
} from "../shared/object";

// The translation column of a (possibly absent) 3x3 transform; absent = origin.
export function transformOrigin(transform: SceneObject["transform"]): [number, number] {
  if (!transform) return [0, 0];
  return [transform[0][2], transform[1][2]];
}

// Structural equality of two (possibly absent) 3x3 transforms; an absent transform equals identity.
export function transformsEqual(a: SceneObject["transform"], b: SceneObject["transform"]): boolean {
  const m = a ?? IDENTITY_TRANSFORM;
  const n = b ?? IDENTITY_TRANSFORM;
  return m.every((row, i) => row.every((v, j) => v === n[i][j]));
}

// Shift a transform's origin by (dx, dy), preserving its linear part.
export function shiftTransform(transform: SceneObject["transform"], dx: number, dy: number): Transform3x3 {
  const [ox, oy] = transformOrigin(transform);
  if (!transform) return translateTransform(dx, dy);
  return [
    [transform[0][0], transform[0][1], ox + dx],
    [transform[1][0], transform[1][1], oy + dy],
    [transform[2][0], transform[2][1], transform[2][2]]
  ];
}

// Map a world point into an object's local quantized geometry space (inverse affine, then quantize
// by GEOMETRY_QUANTUM_PER_PX). Null when the transform is non-invertible (degenerate scale).
export function worldToObjectLocalQuantized(
  object: SceneObject,
  world: { x: number; y: number }
): { x: number; y: number } | null {
  const t = object.transform ?? IDENTITY_TRANSFORM;
  const a = t[0][0];
  const b = t[0][1];
  const c = t[1][0];
  const d = t[1][1];
  const e = t[0][2];
  const f = t[1][2];
  const det = a * d - b * c;
  if (Math.abs(det) < 1e-9) return null;
  const dx = world.x - e;
  const dy = world.y - f;
  const localX = (d * dx - b * dy) / det;
  const localY = (-c * dx + a * dy) / det;
  return { x: Math.round(localX * GEOMETRY_QUANTUM_PER_PX), y: Math.round(localY * GEOMETRY_QUANTUM_PER_PX) };
}

// The union world-AABB of the given objects, each from its geometry path bbox (object-local
// quantized px → logical px) run through its affine transform. Null when no object yields a finite bbox.
export function unionWorldAabb(
  objects: SceneObject[]
): { minX: number; minY: number; maxX: number; maxY: number } | null {
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const object of objects) {
    const local = pathLocalBbox(object.geometry.d ?? "");
    if (!local) continue;
    const t = object.transform;
    for (const [lx, ly] of [
      [local.minX, local.minY],
      [local.maxX, local.minY],
      [local.maxX, local.maxY],
      [local.minX, local.maxY]
    ]) {
      const [wx, wy] = t ? [t[0][0] * lx + t[0][1] * ly + t[0][2], t[1][0] * lx + t[1][1] * ly + t[1][2]] : [lx, ly];
      minX = Math.min(minX, wx);
      minY = Math.min(minY, wy);
      maxX = Math.max(maxX, wx);
      maxY = Math.max(maxY, wy);
    }
  }
  return Number.isFinite(minX) ? { minX, minY, maxX, maxY } : null;
}

// The object-local bbox (logical px) of a path-string's coordinate pairs. Coords are quantized
// integers (GEOMETRY_QUANTUM_PER_PX per px); reading every numeric pair covers M/L/C control points.
export function pathLocalBbox(d: string): { minX: number; minY: number; maxX: number; maxY: number } | null {
  const nums = d.match(/-?\d+(?:\.\d+)?/g);
  if (!nums || nums.length < 2) return null;
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (let i = 0; i + 1 < nums.length; i += 2) {
    const x = Number(nums[i]) / GEOMETRY_QUANTUM_PER_PX;
    const y = Number(nums[i + 1]) / GEOMETRY_QUANTUM_PER_PX;
    minX = Math.min(minX, x);
    minY = Math.min(minY, y);
    maxX = Math.max(maxX, x);
    maxY = Math.max(maxY, y);
  }
  return Number.isFinite(minX) ? { minX, minY, maxX, maxY } : null;
}

// A closed object-local rect path of `w`×`h` logical px, quantized.
export function rectPathQuantized(w: number, h: number): string {
  const qw = Math.round(Math.max(1, w) * GEOMETRY_QUANTUM_PER_PX);
  const qh = Math.round(Math.max(1, h) * GEOMETRY_QUANTUM_PER_PX);
  return `M 0 0 L ${qw} 0 L ${qw} ${qh} L 0 ${qh} Z`;
}
