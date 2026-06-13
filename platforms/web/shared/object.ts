// Object-native client data types (facade). The canvas model + op-apply truth is the Rust scene-core
// crate; transport TYPES are GENERATED from its serde surface by ts-rs (`npm run types:gen`) into
// `./generated/`, and this file re-exports them plus the hand-written helpers and the one type ts-rs
// cannot represent. To change a wire type, edit the Rust serde surface and run `types:gen`.
//
// Serde quirk (preserved by the generator): an enum-level `rename_all = "camelCase"` renames VARIANT
// names only — struct-variant FIELDS stay snake_case. So `CommentAnchor.node_index`, `Paint.content_ref`,
// and every Feature request/response field are snake_case on the wire, while `ObjectOp` variants (which
// carry a per-variant `rename_all`) are camelCase.

export type {
  FillRule,
  Geometry,
  HandlePoint,
  PathNode,
  SubPath,
  Warp,
  Paint,
  GradientStop,
  Fill,
  LineCap,
  LineJoin,
  TextAlign,
  TextVAlign,
  TextRun,
  Text,
  Stroke,
  LocalPoint,
  Anchor,
  LayoutDirection,
  LayoutAlign,
  LayoutSizing,
  Layout,
  CommentAnchor,
  Comment,
  ContentEmbed,
  Object,
  TagDef,
  ObjectSelection,
  ObjectScene,
  FieldEdit,
  ObjectOp,
  FeatureRequest,
  FeatureResponse,
  OpId,
  WireOp
} from "./generated/object-wire";

// `ObjectId` is a `string` alias ts-rs inlines; re-declared for import sites referencing the named alias.
export type ObjectId = string;

export { GEOMETRY_QUANTUM_PER_PX } from "./generated/geometry-const";

import type { ObjectOp, ObjectScene, ObjectSelection } from "./generated/object-wire";

// Transform3x3 stays hand-written: the crate type is `#[serde(transparent)]`, so it appears on the wire
// as a BARE 3x3 array (NOT `{ m: [...] }`). ts-rs cannot represent `transparent`, so the generated
// `Object.transform` emits the bare-array shape directly and this named alias is supplied for import sites.

// Bare 3x3 row-major matrix `[[a,b,c],[d,e,f],[g,h,i]]`. Affine: g=h=0,i=1.
export type Transform3x3 = [
  [number, number, number],
  [number, number, number],
  [number, number, number]
];

export const IDENTITY_TRANSFORM: Transform3x3 = [
  [1, 0, 0],
  [0, 1, 0],
  [0, 0, 1]
];

// A pure-translation transform in logical pixels.
export function translateTransform(tx: number, ty: number): Transform3x3 {
  return [
    [1, 0, tx],
    [0, 1, ty],
    [0, 0, 1]
  ];
}

// Shift+click toggle of `id` in/out of the current selection set: adding grows a single/empty selection
// to a `multi`, removing collapses it back. Selection order is preserved; the toggled id appends to the end.
export function toggleObjectSelection(current: ObjectSelection, id: string): ObjectSelection {
  const ids = current.kind === "object" ? [current.id] : current.kind === "multi" ? [...current.ids] : [];
  const at = ids.indexOf(id);
  if (at >= 0) ids.splice(at, 1);
  else ids.push(id);
  if (ids.length === 0) return { kind: "canvas" };
  if (ids.length === 1) return { kind: "object", id: ids[0] };
  return { kind: "multi", ids };
}

// A fresh empty object scene.
export function emptyObjectScene(updatedAt = new Date(0).toISOString()): ObjectScene {
  return {
    sceneVersion: 0,
    objects: [],
    tags: [],
    selection: { kind: "canvas" },
    updatedAt
  };
}
