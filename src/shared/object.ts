// OB4.3 — object-native client data types.
//
// The single source of truth for the canvas model and op-apply is the Rust
// scene-core crate; the client loads the SAME logic compiled to wasm
// (`src/client/scene/wasm`). These TS types are a transport mirror of the
// crate's serde surface (`crates/scene-core/src/object/{model,op}.rs` and
// `crates/scene-core/src/wire.rs`), so a JSON value crossing the wasm/WS
// boundary is typed without any TS-side domain logic (P1). They carry NO
// behavior — every mutation rides `apply_object_op` in the wasm core.
//
// Conventions (matching the crate): serde camelCase keys; geometry coordinates
// are object-local quantized integers; the 3x3 transform is serde-transparent so
// it appears on the wire as a BARE `[[1,0,x],[0,1,y],[0,0,1]]` array (NOT
// `{ m: [...] }`).

// ---------------------------------------------------------------------------
// Geometry (D2) — single path substrate. The path-string `d` is the canonical
// at-rest + wire form; the parsed `subpaths` are runtime-only (never serialized)
// and the client never builds them — the wasm core owns parsing.
// ---------------------------------------------------------------------------

export type ObjectId = string;

/** Quantized units per logical pixel (mirrors `GEOMETRY_QUANTUM_PER_PX`). */
export const GEOMETRY_QUANTUM_PER_PX = 8;

export type FillRule = "evenOdd" | "nonZero";

export type Geometry = {
  /** SVG-subset path-string (M/L/C/Z), object-local quantized integer coords. */
  d: string;
  fillRule?: FillRule;
};

// ---------------------------------------------------------------------------
// Transform (D7) — row-major 3x3 projective matrix, serde-transparent.
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
// Style (D4) — fill / stroke / text(runs[]).
// ---------------------------------------------------------------------------

export type GradientStop = { offset: number; color: string };

export type Paint =
  | { kind: "solid"; color: string }
  | { kind: "gradient"; stops: GradientStop[]; angle: number }
  | { kind: "image"; contentRef: string };

export type Fill = {
  paint: Paint;
  opacity?: number;
};

export type LineCap = "butt" | "round" | "square";
export type LineJoin = "miter" | "round" | "bevel";

export type Stroke = {
  paint: Paint;
  /** Default stroke width in quantized units (per-node width wins). */
  width: number;
  opacity?: number;
  /** Dash on/off run lengths in quantized units; empty/omitted => solid. */
  dash?: number[];
  cap?: LineCap;
  join?: LineJoin;
};

export type TextAlign = "start" | "center" | "end" | "justify";
export type TextVAlign = "top" | "middle" | "bottom";

export type TextRun = {
  text: string;
  color?: string;
  /** Font size in quantized units. */
  size?: number;
  bold?: boolean;
  italic?: boolean;
  font?: string;
};

export type Text = {
  runs: TextRun[];
  align?: TextAlign;
  valign?: TextVAlign;
};

// ---------------------------------------------------------------------------
// Anchors (D5) — per-node attachment; edges are absorbed into this.
// ---------------------------------------------------------------------------

export type LocalPoint = { x: number; y: number };

export type Anchor = {
  nodeIndex: number;
  target: ObjectId;
  at: LocalPoint;
};

// ---------------------------------------------------------------------------
// Children grouping (D3) / clip (D18) / comments + tags (D20).
// ---------------------------------------------------------------------------

export type LayoutDirection = "row" | "column";
export type LayoutAlign = "start" | "center" | "end" | "stretch";
export type LayoutSizing = "hug" | "fixed" | "fill";

export type Layout = {
  direction: LayoutDirection;
  gap: number;
  padding: number;
  align: LayoutAlign;
  sizing: LayoutSizing;
};

export type CommentAnchor =
  | { kind: "node"; nodeIndex: number }
  | { kind: "point"; at: LocalPoint };

export type Comment = {
  id: string;
  author: string;
  body: string;
  at?: CommentAnchor;
  resolved?: boolean;
};

export type ContentEmbed = {
  kind: string;
  contentRef: string;
};

// ---------------------------------------------------------------------------
// D1 — the single Object substrate. Replaces SceneGroup/SceneNode/SceneEdge.
// ---------------------------------------------------------------------------

export type Object = {
  id: ObjectId;
  /** Parent object id (children-group containment). Absent = canvas root. */
  parent?: ObjectId;
  /** Fractional z-order key (base-62); sorts by plain string Ord. */
  order: string;
  transform?: Transform3x3;
  geometry: Geometry;
  fill?: Fill;
  stroke?: Stroke;
  text?: Text;
  anchors?: Anchor[];
  layout?: Layout;
  clip?: boolean;
  comments?: Comment[];
  tags?: string[];
  componentOf?: ObjectId;
  content?: ContentEmbed;
  meta?: Record<string, unknown>;
};

export type TagDef = {
  id: string;
  name: string;
  color: string;
};

/** Selection union (`kind`-tagged). `multi` is ephemeral shell-only. */
export type ObjectSelection =
  | { kind: "canvas" }
  | { kind: "object"; id: ObjectId }
  | { kind: "multi"; ids: ObjectId[] };

/** The canonical object scene snapshot (the welcome payload). */
export type ObjectScene = {
  sceneVersion: number;
  objects: Object[];
  tags: TagDef[];
  selection: ObjectSelection;
  updatedAt: string;
};

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

// ---------------------------------------------------------------------------
// ObjectOp (OB1.2) — the object op union, internally tagged on `kind` (kebab).
// This is exactly what a `WireOp.propDelta` carries; the wasm core decodes it.
// ---------------------------------------------------------------------------

/** Three-state edit for an optional style field (mirrors the crate `FieldEdit`). */
export type FieldEdit<T> = { action: "set"; value: T } | { action: "clear" };

export type ObjectOp =
  | { kind: "insert-object"; object: Object }
  | { kind: "edit-geometry"; id: ObjectId; geometry: Geometry }
  | { kind: "set-transform"; id: ObjectId; transform: Transform3x3 }
  | { kind: "set-style"; id: ObjectId; fill?: FieldEdit<Fill>; stroke?: FieldEdit<Stroke> }
  | { kind: "set-text"; id: ObjectId; text?: Text }
  | { kind: "set-anchor"; id: ObjectId; anchors: Anchor[] }
  | { kind: "set-layout"; id: ObjectId; layout?: Layout }
  | { kind: "set-clip"; id: ObjectId; clip?: boolean }
  | { kind: "add-comment"; id: ObjectId; comment: Comment }
  | { kind: "set-comments"; id: ObjectId; comments: Comment[] }
  | { kind: "set-tags"; id: ObjectId; tags: string[] }
  | { kind: "reparent"; id: ObjectId; parent?: ObjectId; order: string }
  | { kind: "reorder"; id: ObjectId; order: string }
  | { kind: "delete"; id: ObjectId }
  | { kind: "split"; id: ObjectId; newIds: ObjectId[]; contours?: number[] }
  | { kind: "merge"; ids: ObjectId[]; into?: ObjectId }
  | { kind: "batch"; ops: ObjectOp[] };

// ---------------------------------------------------------------------------
// Feature channel (OB1.2 / OB4.5) — request/response RPC replacing bespoke REST.
//
// The crate's `FeatureRequest`/`FeatureResponse` enums carry
// `#[serde(tag = "feature", rename_all = "camelCase")]`. In serde the enum-level
// `rename_all` renames the VARIANT NAMES only (so the `feature` tag is camelCase:
// `commentUpsert`, `templateApply`, …) — it does NOT rename struct-variant
// fields. The variants have no per-variant `rename_all`, so the FIELDS stay
// snake_case on the wire (`canvas_id`, `object_id`, `comment_id`, `anchor_x`,
// `request_id`, …). Verified against `crates/server/tests/ws.rs`
// (`feature_comment_upsert_round_trips_over_ws`).
// ---------------------------------------------------------------------------

export type FeatureRequest =
  | { feature: "canvasSwitch"; canvas_id: string }
  | { feature: "commentUpsert"; canvas_id: string; object_id: ObjectId; comment: Comment }
  | { feature: "templateApply"; canvas_id: string; recipe: Object[]; anchor_x: number; anchor_y: number }
  | {
      feature: "exportRequest";
      canvas_id: string;
      scope_ids: string[];
      export_type: string;
      request_id: string;
    };

export type FeatureResponse =
  | { feature: "canvasSwitched"; canvas_id: string; seq: number; revision: number }
  | { feature: "commentUpserted"; object_id: ObjectId; comment_id: string }
  | { feature: "templateApplied"; object_ids: string[] }
  | { feature: "exportReady"; request_id: string; artifact_ref: string; content_type: string }
  | { feature: "featureError"; request_id?: string; message: string };

// ---------------------------------------------------------------------------
// Wire op envelope (wire.rs WireOp). `propDelta` carries the ObjectOp delta JSON.
// ---------------------------------------------------------------------------

/** `(clientId, localSeq)` idempotency key — mirrors the server `OpId`. */
export type OpId = {
  clientId: string;
  localSeq: number;
};

/** One granular op as it travels on the wire (`WireOp`). */
export type WireOp = {
  opId: OpId;
  /** Primary target object id (descriptive; the full delta is in `propDelta`). */
  objectId: string;
  /** Kebab op discriminant (mirrors `ObjectOp.kind`). */
  kind: string;
  /** The `ObjectOp` delta JSON. */
  propDelta: ObjectOp;
  baseRevision: number;
  /** userId of the authoring actor. */
  actor: string;
  ts: string;
};

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
