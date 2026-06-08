// U1 — object primitive builders.
//
// The shell composes a basic primitive (rectangle/ellipse/line/text/frame) into
// an `Object` the wasm core validates and applies via an `insert-object`
// ObjectOp. This is geometry-vocabulary construction (a path-string `d` + inline
// style + transform placing the object at a world anchor), not op-apply,
// validation, or template lowering — those stay in the wasm core (P1).
//
// Geometry coordinates are object-local quantized integers at GEOMETRY_QUANTUM_PER_PX
// (D2); the object is positioned in world space by a pure-translation transform.

import type { CameraState } from "../../shared/geometry";
import {
  GEOMETRY_QUANTUM_PER_PX,
  type Fill,
  type Object as SceneObject,
  type Stroke,
  translateTransform
} from "../../shared/object";
import { worldToScreen, type WorldRect } from "../renderer/scene";
import type { PrimitiveKindId } from "./toolbar";

const Q = GEOMETRY_QUANTUM_PER_PX;

/** Quantize logical pixels to object-local integer geometry units. */
function q(px: number): number {
  return Math.round(px * Q);
}

const DEFAULT_FILL: Fill = { paint: { kind: "solid", color: "#e8eefc" }, opacity: 1 };
const DEFAULT_STROKE: Stroke = { paint: { kind: "solid", color: "#2f7ee6" }, width: q(1.5) };
const LINE_STROKE: Stroke = { paint: { kind: "solid", color: "#5b6472" }, width: q(2) };

/** A closed rectangle path-string of `w`×`h` logical px (object-local). */
function rectPath(w: number, h: number): string {
  return `M 0 0 L ${q(w)} 0 L ${q(w)} ${q(h)} L 0 ${q(h)} Z`;
}

/** A closed ellipse path-string of `w`×`h` logical px via four cubic arcs. */
function ellipsePath(w: number, h: number): string {
  const cx = q(w / 2);
  const cy = q(h / 2);
  const rx = q(w / 2);
  const ry = q(h / 2);
  const kx = Math.round(rx * 0.5523);
  const ky = Math.round(ry * 0.5523);
  return [
    `M 0 ${cy}`,
    `C 0 ${cy - ky} ${cx - kx} 0 ${cx} 0`,
    `C ${cx + kx} 0 ${q(w)} ${cy - ky} ${q(w)} ${cy}`,
    `C ${q(w)} ${cy + ky} ${cx + kx} ${q(h)} ${cx} ${q(h)}`,
    `C ${cx - kx} ${q(h)} 0 ${cy + ky} 0 ${cy}`,
    "Z"
  ].join(" ");
}

/** A horizontal open line of `len` logical px. */
function linePath(len: number): string {
  return `M 0 0 L ${q(len)} 0`;
}

export type PrimitiveSpec = {
  /** Object-local geometry path-string. */
  d: string;
  fill?: Fill;
  stroke?: Stroke;
  text?: SceneObject["text"];
  /** Logical-px size used to center the object on the anchor. */
  size: { width: number; height: number };
};

/** The default geometry/style for each primitive kind. */
function primitiveSpec(kind: PrimitiveKindId): PrimitiveSpec {
  switch (kind) {
    case "rectangle":
      return { d: rectPath(160, 100), fill: DEFAULT_FILL, stroke: DEFAULT_STROKE, size: { width: 160, height: 100 } };
    case "ellipse":
      return { d: ellipsePath(140, 140), fill: DEFAULT_FILL, stroke: DEFAULT_STROKE, size: { width: 140, height: 140 } };
    case "line":
      return { d: linePath(200), stroke: LINE_STROKE, size: { width: 200, height: 0 } };
    case "text":
      // W2-10: the text primitive is a borderless, style-less rect — no border, no
      // fill, no default "Note" text. Every object can hold text; the text "shape"
      // is just one with no border that enters inline edit immediately on create.
      return { d: rectPath(180, 80), size: { width: 180, height: 80 } };
    case "frame":
      return { d: rectPath(420, 300), stroke: { paint: { kind: "solid", color: "#94a3b8" }, width: q(1) }, size: { width: 420, height: 300 } };
  }
}

/**
 * Build the `Object` for a primitive kind, centered on `anchor` (world px) with a
 * fresh `id`/`order`. The geometry is object-local; the world position rides a
 * pure-translation transform (D7) so a later move is matrix-only (P4).
 */
export function buildPrimitiveObject(
  kind: PrimitiveKindId,
  anchor: { x: number; y: number },
  id: string,
  order: string
): SceneObject {
  const spec = primitiveSpec(kind);
  const tx = anchor.x - spec.size.width / 2;
  const ty = anchor.y - spec.size.height / 2;
  return {
    id,
    order,
    transform: translateTransform(tx, ty),
    geometry: { d: spec.d, fillRule: "nonZero" },
    ...(spec.fill ? { fill: spec.fill } : {}),
    ...(spec.stroke ? { stroke: spec.stroke } : {}),
    ...(spec.text ? { text: spec.text } : {}),
    ...(kind === "frame" ? { clip: true } : {})
  };
}

// W2-07: a drag span — the gesture's start corner and current/end corner (world
// px). Closed primitives (rect/ellipse/frame/text) are sized to the normalized
// bbox; the open line runs corner-to-corner so a diagonal drag draws a diagonal.
export type DragSpan = { start: { x: number; y: number }; end: { x: number; y: number } };

// Smallest extent (logical px) a drag must reach before a closed primitive is
// considered sized; below this the caller treats the gesture as a click.
export const MIN_DRAG_EXTENT_PX = 4;

/** Object-local geometry + the world translation for a primitive sized to a drag. */
function dragGeometry(kind: PrimitiveKindId, span: DragSpan): { d: string; tx: number; ty: number } {
  if (kind === "line") {
    // The line rides corner-to-corner: object-local from start (0,0) to the end
    // delta, positioned by a translation at the start point.
    const dx = span.end.x - span.start.x;
    const dy = span.end.y - span.start.y;
    return { d: `M 0 0 L ${q(dx)} ${q(dy)}`, tx: span.start.x, ty: span.start.y };
  }
  const minX = Math.min(span.start.x, span.end.x);
  const minY = Math.min(span.start.y, span.end.y);
  const w = Math.abs(span.end.x - span.start.x);
  const h = Math.abs(span.end.y - span.start.y);
  const d = kind === "ellipse" ? ellipsePath(w, h) : rectPath(w, h);
  return { d, tx: minX, ty: minY };
}

/**
 * Build the `Object` for a primitive kind sized to a drag span (W2-07), with a
 * fresh `id`/`order`. The geometry is object-local; the world position rides a
 * pure-translation transform (D7) so a later move is matrix-only (P4). Style
 * mirrors {@link buildPrimitiveObject}; only the size/position come from the drag.
 */
export function buildPrimitiveObjectFromDrag(
  kind: PrimitiveKindId,
  span: DragSpan,
  id: string,
  order: string
): SceneObject {
  const spec = primitiveSpec(kind);
  const { d, tx, ty } = dragGeometry(kind, span);
  return {
    id,
    order,
    transform: translateTransform(tx, ty),
    geometry: { d, fillRule: "nonZero" },
    ...(spec.fill ? { fill: spec.fill } : {}),
    ...(spec.stroke ? { stroke: spec.stroke } : {}),
    ...(spec.text ? { text: spec.text } : {}),
    ...(kind === "frame" ? { clip: true } : {})
  };
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
