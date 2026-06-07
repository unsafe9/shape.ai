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

import {
  GEOMETRY_QUANTUM_PER_PX,
  translateTransform,
  type Fill,
  type Object as SceneObject,
  type Stroke
} from "../../shared/object";
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
      return {
        d: rectPath(180, 80),
        fill: { paint: { kind: "solid", color: "#fff7d6" }, opacity: 1 },
        text: { runs: [{ text: "Note" }], align: "start", valign: "top" },
        size: { width: 180, height: 80 }
      };
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
