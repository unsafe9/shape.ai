// OB4.3 — object-native client data types (facade).
//
// The single source of truth for the canvas model and op-apply is the Rust
// scene-core crate; the client loads the SAME logic compiled to wasm
// (`platforms/web/bridge/wasm`). The transport TYPES are no longer hand-mirrored:
// they are GENERATED from scene-core's serde surface by ts-rs (`npm run
// types:gen`) into `./generated/`, and this file is a thin facade that re-exports
// them plus the few hand-written helpers and the one type ts-rs cannot represent.
//
// To change a wire type, edit the Rust serde surface (`crates/scene-core/src/
// object/kernel/{model,op}.rs`, `wire.rs`) and run `npm run types:gen`; the
// `types:check` gate fails the build if the committed `generated/` drifts.
//
// Serde quirk (preserved by the generator, NOT by hand): an enum-level
// `rename_all = "camelCase"` renames VARIANT names only — struct-variant FIELDS
// stay snake_case. So `CommentAnchor`'s `node_index`, `Paint`'s `content_ref`,
// and every `FeatureRequest`/`FeatureResponse` field are snake_case on the wire,
// while `ObjectOp` variants (which carry a per-variant `rename_all`) are camelCase.

// ---------------------------------------------------------------------------
// Generated wire types (ts-rs export of the scene-core serde surface).
// ---------------------------------------------------------------------------

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

// `ObjectId` is a `string` alias; ts-rs inlines it, so re-declare it here for the
// import sites that reference the named alias.
export type ObjectId = string;

export { GEOMETRY_QUANTUM_PER_PX } from "./generated/geometry-const";

import type { ObjectOp, ObjectScene, ObjectSelection } from "./generated/object-wire";

// ---------------------------------------------------------------------------
// EXCEPTION — Transform3x3 stays hand-written. The crate type is
// `#[serde(transparent)]`, so it appears on the wire as a BARE 3x3 array
// (`[[1,0,x],[0,1,y],[0,0,1]]`, NOT `{ m: [...] }`). ts-rs cannot represent
// `transparent`, so the generated `Object.transform` / `set-transform` emit the
// bare-array shape directly (structurally identical to this) and this named alias
// is supplied here for the import sites that reference it.
// ---------------------------------------------------------------------------

/** Bare 3x3 row-major matrix `[[a,b,c],[d,e,f],[g,h,i]]`. Affine: g=h=0,i=1. */
export type Transform3x3 = [
  [number, number, number],
  [number, number, number],
  [number, number, number]
];

/** The identity transform (the wire default). */
export const IDENTITY_TRANSFORM: Transform3x3 = [
  [1, 0, 0],
  [0, 1, 0],
  [0, 0, 1]
];

/** A pure-translation transform in logical pixels. */
export function translateTransform(tx: number, ty: number): Transform3x3 {
  return [
    [1, 0, tx],
    [0, 1, ty],
    [0, 0, 1]
  ];
}

// ---------------------------------------------------------------------------
// Hand-written helpers (no wire shape; pure shell-side conveniences).
// ---------------------------------------------------------------------------

/**
 * W2-03: Shift+click toggle of `id` in/out of the current selection set. Adding to
 * a single/empty selection grows it to a `multi`; removing collapses it back to
 * `object`/`canvas`. Pure (no scene access) so the shell test can pin the
 * classification. Selection order is preserved; the toggled id appends to the end.
 */
export function toggleObjectSelection(current: ObjectSelection, id: string): ObjectSelection {
  const ids = current.kind === "object" ? [current.id] : current.kind === "multi" ? [...current.ids] : [];
  const at = ids.indexOf(id);
  if (at >= 0) ids.splice(at, 1);
  else ids.push(id);
  if (ids.length === 0) return { kind: "canvas" };
  if (ids.length === 1) return { kind: "object", id: ids[0] };
  return { kind: "multi", ids };
}

/** A fresh empty object scene. */
export function emptyObjectScene(updatedAt = new Date(0).toISOString()): ObjectScene {
  return {
    sceneVersion: 0,
    objects: [],
    tags: [],
    selection: { kind: "canvas" },
    updatedAt
  };
}

/** The primary target id of an op (rides `WireOp.objectId`). */
export function opPrimaryTargetId(op: ObjectOp): string {
  switch (op.kind) {
    case "insert-object":
      return op.object.id;
    case "merge":
      return op.into ?? op.ids[0] ?? "";
    case "batch":
      return op.ops.length > 0 ? opPrimaryTargetId(op.ops[0]) : "";
    case "delete":
    case "edit-geometry":
    case "set-transform":
    case "set-style":
    case "set-text":
    case "set-anchor":
    case "set-layout":
    case "set-clip":
    case "add-comment":
    case "set-comments":
    case "set-tags":
    case "reparent":
    case "reorder":
    case "split":
      return op.id;
    default:
      return "";
  }
}
