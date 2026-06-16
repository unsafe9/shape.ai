// Typed lazy loader for the scene-core wasm bridge. scene-core is the single source
// of truth for op-apply; the server links it as a Rust crate and the web client loads
// the SAME logic as wasm, so a patch applies identically on both sides.
//
// Each wrapper mirrors one `#[wasm_bindgen]` export in `crates/scene-core/src/wasm_api.rs`:
// JSON strings in, `JSON.parse` of the returned JSON out. The bridge never throws across
// FFI — a malformed-input or serialize failure comes back as `{ error: string }`, which
// these wrappers re-throw as a typed Error. Domain failures (unknown id, invalid patch)
// are NOT thrown: they ride the `errors: string[]` field of the result.

import type {
  Anchor,
  Object as SceneObject,
  ObjectOp,
  ObjectScene,
  ObjectSelection,
  Transform3x3
} from "../shared/object";
import type { DragSpan } from "../controller/objectPrimitives";
import type { PrimitiveKindId } from "../controller/toolbar";

export type MoveRoots = { kind: "single"; id: string } | { kind: "multi"; ids: string[] };

// On a domain failure `scene` is unchanged, `inverse` is null, and the message rides `errors`.
export type ObjectApplyResult = {
  scene: ObjectScene;
  inverse: ObjectOp | null;
  errors: string[];
};

export type ObjectCommand = {
  id: string;
  label: string;
  category: string;
  defaultShortcut?: string;
  description?: string;
  opKind?: string;
  [extra: string]: unknown;
};

// Exactly one of `key`/`button`/`modifier` is populated, the one matching `input`.
export type HoldTrigger = {
  input: "key" | "button" | "modifier";
  key?: string;
  button?: string;
  modifier?: string;
  degrees?: number;
};

export type ObjectGesture = {
  id: string;
  label: string;
  category: string;
  trigger: HoldTrigger;
  description: string;
};

// The inspector catalog/view types mirror `crates/scene-core/src/object/catalog/inspector.rs`.
// Like ObjectCommand/ObjectGesture they are Serialize-only on the Rust side, so they are
// hand-declared here (NOT ts-rs). Enum strings are kebab-case (section / appliesTo); the
// widget union is tagged on `kind`, camelCase.

// Which roles a control is offered for; the dynamic view intersects this across the selection.
export type AppliesTo =
  | "always"
  | "free-placed"
  | "flow-child"
  | "container"
  | "flow-container"
  | "has-text";

export type InspectorSection =
  | "header"
  | "placement"
  | "layout"
  | "appearance"
  | "text"
  | "action";

// The widget a control renders as. `number`'s bounds/step are advisory hints (null = unbounded).
export type InspectorWidget =
  | { kind: "text" }
  | { kind: "toggle" }
  | { kind: "badge" }
  | { kind: "button" }
  | { kind: "paint" }
  | { kind: "number"; unit: string; min: number | null; max: number | null; step: number }
  | { kind: "segment"; options: string[] }
  | { kind: "lanes" }
  | { kind: "align9" };

// A static catalog entry: one inspector control and how it lowers to an op.
// `opKind`/`field` are omitted (not null) on the wire for a read-only display / a button.
export type InspectorControl = {
  id: string;
  label: string;
  section: InspectorSection;
  widget: InspectorWidget;
  appliesTo: AppliesTo;
  opKind?: string;
  field?: string;
  // Quantized-units-per-px for a px-denominated numeric control (spacing/stroke-width/font-size),
  // else 1.0. The view value is already divided to px; the shell multiplies a px edit back by it.
  unitScale: number;
  description: string;
};

export type Placement = "free" | "flow-child";

// The resolved role of a single object — the basis for filtering the catalog.
export type InspectorRole = {
  placement: Placement;
  container: boolean;
  flowContainer: boolean;
  hasText: boolean;
};

// A control resolved for the current selection: catalog metadata + its current value
// (`null` when unset/not-applicable or `mixed`) and the multi-select divergence flag.
export type InspectorControlValue = {
  id: string;
  label: string;
  widget: InspectorWidget;
  value: unknown;
  mixed: boolean;
  opKind?: string;
  field?: string;
  // Carried from the catalog so a px edit re-quantizes (px * unitScale) without the shell owning the Q.
  unitScale: number;
};

export type InspectorSectionView = {
  section: InspectorSection;
  controls: InspectorControlValue[];
};

// The full dynamic inspector view for a selection.
export type InspectorView = {
  role: InspectorRole;
  sections: InspectorSectionView[];
};

export type DerivedRegion = Record<string, unknown>;

// Container-vs-leaf decision for a double-click: a container drills in, a leaf edits its text.
export type DoubleClickAction = { kind: "drill-in-container" } | { kind: "edit-leaf" };

// `"basic"` force-snaps every stroke to a basic primitive (line / ellipse / rect / triangle);
// `"free"` runs the full pipeline with polygon + silhouette fallbacks.
export type RecognizeMode = "basic" | "free";

// Single-step z-order move over the flat scene order: toward the front or the back.
export type ReorderDirection = "forward" | "backward";

// The create-gesture SCREEN-px thresholds owned only by scene-core (recognize): the
// click-vs-drag extent, the release anchor-reuse radius, and the stroke merge-endpoint
// radius. The shell reads them once and divides each by zoom; it holds no literal copy.
export type CreateThresholds = {
  minDragExtentPx: number;
  createAnchorReuseTolerancePx: number;
  mergeEndpointTolerancePx: number;
};

// Shape of the generated wasm-pack module. Declared locally (matching wasmLoader.ts) so
// this file does not statically import the gitignored build artifact's types; the dynamic
// import is `@vite-ignore`d. Built `--target web`, exposing both an async `default`
// (browser: fetch the `.wasm`) and a sync `initSync` (Node/vitest, which cannot `fetch()`).
type SceneCoreModule = {
  default: (init?: unknown) => Promise<unknown>;
  initSync: (module: { module: BufferSource | WebAssembly.Module }) => unknown;
  apply_object_op: (sceneJson: string, opJson: string) => string;
  derive_region: (geometryJson: string, flatness: number) => string;
  object_command_catalog: () => string;
  object_gesture_catalog: () => string;
  object_inspector_catalog: () => string;
  object_inspector_view: (sceneJson: string, selectionJson: string) => string;
  object_set_transform_field: (matrixJson: string, field: string, value: number) => string;
  object_quantize_units: (px: number, scale: number) => number;
  object_resize_axis: (
    transformJson: string,
    geometryJson: string,
    axis: string,
    targetPx: number
  ) => string;
  object_inspector_edit_op: (
    objectJson: string,
    controlId: string,
    valueJson: string,
    unitScale: number
  ) => string;
  template_anchor: (
    sceneJson: string,
    fallbackX: number,
    fallbackY: number
  ) => string;
  build_object_template: (
    templateId: string,
    anchorX: number,
    anchorY: number,
    idPrefix: string
  ) => string;
  freehand_to_object: (
    pointsJson: string,
    color: string,
    widthPx: number,
    id: string,
    order: string,
    mode: string
  ) => string;
  build_primitive: (
    kind: string,
    anchorX: number,
    anchorY: number,
    color: string,
    id: string,
    order: string
  ) => string;
  build_primitive_from_drag: (
    kind: string,
    startX: number,
    startY: number,
    endX: number,
    endY: number,
    color: string,
    id: string,
    order: string
  ) => string;
  build_set_style_op: (objectJson: string, color: string) => string;
  is_open_class_d: (d: string) => string;
  merge_open_stroke_ops: (
    sceneJson: string,
    pointsJson: string,
    mode: string,
    tolerancePx: number
  ) => string;
  split_subpath_at: (geometryJson: string, x: number, y: number, radius: number) => string;
  partial_erase_ops: (sceneJson: string, id: string, wx: number, wy: number, radius: number) => string;
  object_world_aabb: (objectJson: string) => string;
  anchor_follow_ops: (sceneJson: string, transformOpsJson: string) => string;
  move_ops: (sceneJson: string, rootsJson: string, deltaJson: string) => string;
  move_ops_for_pick: (sceneJson: string, selectionJson: string, id: string, deltaJson: string) => string;
  valid_selection: (sceneJson: string, selectionJson: string) => string;
  select_all: (sceneJson: string) => string;
  duplicate_ops: (sceneJson: string, idsJson: string, idPrefix: string, orderSeed: number) => string;
  detach_move_ops: (sceneJson: string, id: string, deltaJson: string) => string;
  synthesize_create_anchors: (
    createdJson: string,
    targetJson: string,
    endpointX: number,
    endpointY: number
  ) => string;
  synthesize_create_anchors_both: (
    sceneJson: string,
    createdJson: string,
    cornersJson: string
  ) => string;
  resolve_create_release: (
    releaseJson: string,
    lastSnapJson: string,
    toleranceWorld: number
  ) => string;
  endpoint_release_ops: (
    sceneJson: string,
    id: string,
    nodeIndex: number,
    newXPx: number,
    newYPx: number,
    snapTargetId: string,
    snapAtJson: string
  ) => string;
  pop_out_op: (sceneJson: string, id: string) => string;
  has_children: (sceneJson: string, id: string) => string;
  ungroup_enabled: (sceneJson: string, selectedId: string) => string;
  double_click_action: (sceneJson: string, id: string) => string;
  object_selection_in_scope: (
    sceneJson: string,
    selectionJson: string,
    container: string
  ) => string;
  group_ops: (sceneJson: string, idsJson: string, frameId: string) => string;
  ungroup_ops: (sceneJson: string, frameId: string) => string;
  create_thresholds: () => string;
  next_order_key: (sceneJson: string) => string;
  back_order_key: (sceneJson: string) => string;
  key_between: (a: string, b: string) => string;
  reorder_step_ops: (sceneJson: string, id: string, direction: string) => string;
  WasmUndoStack: new (actorId: string) => WasmUndoStack;
  WasmSession: new (
    welcomeSceneJson: string,
    clientId: string,
    selfUserId: string,
    coalesceMs: number,
    peerTtlMs: number
  ) => WasmSession;
  WasmWindow: new (seedBboxJson: string, margin: number) => WasmWindow;
};

// The collaboration session (crates/client-runtime), a wasm-bindgen class owning the
// optimistic sync engine + peer registry. Payloads cross as JSON strings; method names
// stay snake_case. The TS runtime adapters drive it and keep the IO seams (WS, IndexedDB, timers).
type WasmSession = {
  author: (opJson: string, ts: string) => string;
  apply_remote: (opJson: string) => string;
  on_ack: (opIdsJson: string, revision: number) => string;
  on_rejected: (opIdsJson: string) => string;
  reconcile_snapshot: (snapshotJson: string, persistedEntriesJson: string) => string;
  flush: () => void;
  on_flush_due: () => void;
  take_pending: () => string;
  scene: () => string;
  base_revision: () => number;
  flush_armed: () => boolean;
  outbox_len: () => number;
  owned_key_set: () => string;
  take_settled_keys: () => string;
  ingest_presence: (payloadJson: string, nowMs: number) => string;
  expire_peers: (nowMs: number) => string;
  peers: () => string;
  clear_peers: () => void;
  free: () => void;
};

// The viewport-windowing DECISION layer (crates/client-runtime WindowState), a wasm-bindgen
// class owning the subscribed window + margin and deciding when a re-subscribe is warranted;
// the shell drives the debounce timer and transport. Bboxes cross as `{x,y,width,height}` JSON;
// `""`/`"null"` is whole-canvas. Method names stay snake_case.
type WasmWindow = {
  on_viewport: (viewportJson: string) => string;
  set_window: (bboxJson: string) => string;
  subscribe_whole_canvas: () => boolean;
  current_window: () => string;
  free: () => void;
};

// The core's per-actor undo stack, a wasm-bindgen class. Ops cross as ObjectOp JSON;
// `undo`/`redo` return the op JSON to apply (or `undefined` when empty). Method names stay snake_case.
type WasmUndoStack = {
  record: (forwardJson: string, inverseJson: string) => boolean;
  begin_coalesce: () => void;
  end_coalesce: () => void;
  is_coalescing: () => boolean;
  undo: () => string | undefined;
  note_undo_applied: (reInverseJson: string) => boolean;
  redo: () => string | undefined;
  note_redo_applied: (inverseJson: string) => boolean;
  abort: () => void;
  can_undo: () => boolean;
  can_redo: () => boolean;
};

// Per-actor undo/redo stack backed by the core. Owns no scene and performs no apply:
// `undo`/`redo` hand back the op the caller must re-author through the SAME `applyObjectOp`
// path, then the caller reports the re-inverse via `noteUndoApplied`/`noteRedoApplied`.
// A continuous gesture collapses to one step inside a `beginCoalesce`/`endCoalesce` window.
export type UndoStack = {
  // A fresh record clears redo; during a coalesce window it folds into one entry.
  record(forward: ObjectOp, inverse: ObjectOp): void;
  beginCoalesce(): void;
  endCoalesce(): void;
  isCoalescing(): boolean;
  // The inverse op to apply, or null when nothing to undo.
  undo(): ObjectOp | null;
  noteUndoApplied(reInverse: ObjectOp): void;
  // The forward op to re-apply, or null when nothing to redo.
  redo(): ObjectOp | null;
  noteRedoApplied(inverse: ObjectOp): void;
  // Abort an in-flight undo/redo handshake whose apply failed (clears pending).
  abort(): void;
  canUndo(): boolean;
  canRedo(): boolean;
};

export type SceneCore = {
  applyObjectOp(scene: ObjectScene, op: ObjectOp): ObjectApplyResult;
  deriveRegion(geometry: SceneObject["geometry"], flatness: number): DerivedRegion;
  objectCommandCatalog(): ObjectCommand[];
  objectGestureCatalog(): ObjectGesture[];
  // The static inspector control catalog (every property control + how it lowers to an op),
  // in panel order. The settings-style read-only mirror; the property panel filters it via the view.
  objectInspectorCatalog(): InspectorControl[];
  // The dynamic inspector view for a selection: the applicable controls (intersected across a
  // multi-select) with their current values + `mixed` divergence flags. Inputs are JSON strings of
  // the render-only mirror's scene + selection so the panel reads ONE core-resolved snapshot.
  objectInspectorView(sceneJson: string, selectionJson: string): InspectorView;
  // Patch ONE transform field (x/y in px, rotation/rotation-flow in DISPLAY degrees) on a bare 3x3 and
  // return the new 3x3 for a `set-transform` op. The core owns the decompose/recompose seam AND the
  // deg->rad conversion; the shell does no matrix or angle-unit math.
  objectSetTransformField(matrixJson: string, field: string, value: number): Transform3x3;
  // Re-quantize an inspector px edit to its stored i32 via the cores' single `round(px * scale)`
  // discipline; `scale` is the control's `unitScale`, so the shell owns neither the rounding nor the Q.
  objectQuantizeUnits(px: number, scale: number): number;
  // The transform that makes a width (`axis: "x"`) or height (`axis: "y"`) edit read `targetPx` absolute
  // px in the inspector. The core does the geometry/scale math; the shell authors the result as
  // `set-transform`. `geometryJson` is the object's `Geometry`, `transformJson` its current 3x3.
  objectResizeAxis(transformJson: string, geometryJson: string, axis: "x" | "y", targetPx: number): Transform3x3;
  // Lower ONE inspector property edit (set-style/set-text/set-sizing/set-layout/set-meta/set-clip) to its
  // ObjectOp, patching the edited field on the object's current value. The core owns the per-field op
  // synthesis AND every style/layout/sizing/text default a borderless or layout-less object gains, so the
  // shell makes no styling decision. `null` for a control this surface does not own (the transform/resize
  // fields x/y/rotation/width/height route through objectSetTransformField/objectResizeAxis) or an edit
  // that cannot apply (a layout field on a layout-less object). `unitScale` re-quantizes a px edit in-core.
  objectInspectorEditOp(object: SceneObject, controlId: string, value: unknown, unitScale: number): ObjectOp | null;
  // Where a new template should land: `gap` right of the right-most object's transform origin and
  // top-aligned, or `fallback` (the shell's viewport center) when the scene is empty.
  templateAnchor(
    scene: ObjectScene,
    fallback: { x: number; y: number }
  ): { x: number; y: number };
  buildObjectTemplate(
    templateId: string,
    anchorX: number,
    anchorY: number,
    idPrefix: string
  ): SceneObject[];
  // Recognize one freehand stroke (world-px points) at pen-up and commit it as one `Object`
  // per `mode`. Points become a JSON array of `[x, y]` pairs for the bridge.
  freehandToObject(
    points: { x: number; y: number }[],
    color: string,
    widthPx: number,
    id: string,
    order: string,
    mode: RecognizeMode
  ): SceneObject;
  // Build a basic primitive centered on a world anchor, in `color` (omit for the kind default —
  // a hex or the theme-default sentinel). Geometry is object-local; the world position rides a translate.
  buildPrimitive(
    kind: PrimitiveKindId,
    anchor: { x: number; y: number },
    id: string,
    order: string,
    color?: string
  ): SceneObject;
  // Build a primitive sized to a drag `span` — closed kinds to the normalized bbox, the line corner-to-corner.
  buildPrimitiveFromDrag(
    kind: PrimitiveKindId,
    span: DragSpan,
    id: string,
    order: string,
    color?: string
  ): SceneObject;
  // Author a `set-style` op recoloring `object` to `color` (hex or theme-default sentinel).
  // Recolors only existing style fields; a borderless object gains a fill.
  buildSetStyleOp(object: SceneObject, color: string): ObjectOp;
  // True iff the geometry is exactly one OPEN subpath. Class-dependent shell branches consult
  // THE core classifier instead of re-parsing geometry in TS.
  isOpenClassD(d: string): boolean;
  // Ops merging a released freehand stroke (world-px points) into the open-class object(s) whose
  // endpoint(s) landed within `tolerancePx` (WORLD px — divide the screen-px constant by the zoom):
  // one `edit-geometry` on the survivor plus the anchor release / absorbed-object delete, never an
  // insert. Null = no merge (shell keeps its insert + release-anchoring path). Merge takes priority.
  mergeOpenStrokeOps(
    scene: ObjectScene,
    points: { x: number; y: number }[],
    mode: RecognizeMode,
    tolerancePx: number
  ): ObjectOp[] | null;
  // Cut a stroke's geometry at an object-local quantized touch point + radius. Returns the new
  // geometry (two open subpaths around the removed node), or null when the touch missed every node.
  splitSubpathAt(
    geometry: SceneObject["geometry"],
    x: number,
    y: number,
    radius: number
  ): SceneObject["geometry"] | null;
  // Cut the stroke at a WORLD touch point (logical px) and return the WHOLE op batch — `[]` on a miss
  // (or a singular transform), `[delete]` when the cut empties the object, else
  // `[edit-geometry, ...follower-reprojection]` (a reshape re-projects anchored followers onto the new
  // outline, chained). The core maps world → object-local quantized internally (no inverse-affine in TS).
  // `radius` is the object-local quantized erase tolerance. `moveOps` / `endpointReleaseOps` fold the same follow.
  partialEraseOps(scene: ObjectScene, id: string, wx: number, wy: number, radius: number): ObjectOp[];
  // The world-space AABB (logical px) of an object: its geometry nodes carried through the transform,
  // min/max'd. The geometry half of the shell's world-bbox math. Null when the object has no geometry nodes.
  objectWorldAabb(object: SceneObject): { minX: number; minY: number; maxX: number; maxY: number } | null;
  // Commit-time anchor-follow ops for an arbitrary committed batch (`set-transform` moves a target,
  // `edit-geometry` reshapes one): the chord-deform ops making every anchored follower track its
  // target (chained). The standalone entry; the op-authoring bridges already fold this in. `[]` when nothing follows.
  anchorFollowOps(scene: ObjectScene, ops: ObjectOp[]): ObjectOp[];
  // Combined commit-time move ops for a parent-drag / multi-select drag: the transform CASCADE ops
  // (dragged subtree, or every multi member + subtree, deduped) FOLLOWED BY the anchor-follow
  // `edit-geometry` ops, as ONE batch (cascade BEFORE follow). `delta` is the world-space gesture matrix.
  moveOps(scene: ObjectScene, roots: MoveRoots, delta: Transform3x3): ObjectOp[];
  // The body-drag commit ops, deriving the MoveRoots from the live `selection` + picked `id` in-core
  // (a Multi-on-member drag moves the whole set; otherwise the picked single root cascades its subtree).
  // The shell stops branching the drag-root policy in TS. Cascade BEFORE follow, same as `moveOps`.
  moveOpsForPick(scene: ObjectScene, selection: ObjectSelection, id: string, delta: Transform3x3): ObjectOp[];
  // Reconcile `selection` against the scene: drop ids no longer present and collapse the kind
  // (>=2 live -> multi, 1 -> object, 0 -> canvas). The single source of truth for the collapse rule.
  validSelection(scene: ObjectScene, selection: ObjectSelection): ObjectSelection;
  // Select every object, collapsed by the same rule: empty -> canvas, one -> object, otherwise multi.
  selectAll(scene: ObjectScene): ObjectSelection;
  // The insert-object ops cloning each id in `ids` with a fresh id (`{idPrefix}-{n}`, indexed from
  // `orderSeed`) and a fresh fractional order key, offset by the canonical duplicate translate (+40/+40).
  // Unknown ids are skipped; `[]` when nothing resolves.
  duplicateOps(scene: ObjectScene, ids: string[], idPrefix: string, orderSeed: number): ObjectOp[];
  // Alt-detach commit ops for an Alt-held body drag of an anchored open-class object: a set-anchor clearing
  // its anchors, THEN the single-root move ops computed against the scene with that object's anchors already
  // cleared (so the move keeps the 0-rebake whole-object translate, not an anchor-follow reprojection).
  // `[]` when `id` is not in the scene.
  detachMoveOps(scene: ObjectScene, id: string, delta: Transform3x3): ObjectOp[];
  // Synthesize the persistent anchor(s) binding `created`'s endpoint node to `target` on a snapped
  // drag-create, or null when no anchor should be authored (target is the created object, or no node / no snap).
  synthesizeCreateAnchors(
    created: SceneObject,
    target: SceneObject,
    endpoint: { x: number; y: number }
  ): Anchor[] | null;
  // Release-time anchor authoring for BOTH gesture corners (shape drag-create AND the freehand pen):
  // each non-null corner binds `created`'s nearest node to its snapped target's outline, deduped to
  // ONE anchor per node (first wins). A null corner, a corner whose target left the scene, or a
  // degenerate tap (corners collapsing onto one node) binds at most one anchor. The (possibly empty) set.
  synthesizeCreateAnchorsBoth(
    scene: ObjectScene,
    created: SceneObject,
    corners: ReadonlyArray<{ target: string; at: { x: number; y: number } } | null>
  ): Anchor[];
  // Resolve a shape drag-create RELEASE to its final endpoint + anchor target: honor the release's own
  // snap, else reuse `lastSnap` when the release landed within `toleranceWorld` (WORLD units) by squared
  // distance, so a near-miss release still binds. `target` null = author no anchor. The reuse radius lives
  // in core (CREATE_ANCHOR_REUSE_TOLERANCE_PX); the caller passes it / zoom as `toleranceWorld`.
  resolveCreateRelease(
    release: { end: { x: number; y: number }; snapped: boolean; target: string | null },
    lastSnap: { at: { x: number; y: number }; target: string } | null,
    toleranceWorld: number
  ): { end: { x: number; y: number }; target: string | null };
  // Commit ops of an endpoint-drag release on an open-class object: one chord-deform `edit-geometry`
  // moving the endpoint (`nodeIndex`, 0 or last) to the world-px release point, plus the `set-anchor`
  // whole-vector rewrite — rebound to `snap.targetId` (at `snap.at`, defaulting to the release point)
  // on a snap, or the endpoint's anchor removed when `snap` is null. `[]` when the op does not apply.
  endpointReleaseOps(
    scene: ObjectScene,
    id: string,
    nodeIndex: number,
    newPoint: { x: number; y: number },
    snap: { targetId: string; at?: { x: number; y: number } } | null
  ): ObjectOp[];
  // The `reparent` op popping `id` out one level (to its grandparent, or the canvas root when the
  // parent sits at the root), preserving its order key. Null when `id` is unknown or already at the root.
  popOutOp(scene: ObjectScene, id: string): ObjectOp | null;
  hasChildren(scene: ObjectScene, id: string): boolean;
  // True only when a non-null `selectedId` is a container (has children).
  ungroupEnabled(scene: ObjectScene, selectedId: string | null): boolean;
  doubleClickAction(scene: ObjectScene, id: string): DoubleClickAction;
  // The core verdict the shell mirrors for active-container token lockstep: true when `selection` is
  // the drill-in `container` itself or a direct child (stays in scope), false otherwise (the shell
  // retracts the token). The scope-exit decision lives in the core, never recomputed in the shell.
  objectSelectionInScope(scene: ObjectScene, selection: ObjectSelection, container: string): boolean;
  // The ops grouping `ids` under a new frame `frameId`: an insert-object for a clipped frame sized +
  // placed to the children's union world-AABB, then one reparent per child re-homing it into the frame.
  // Null when fewer than two known members resolve or no member yields a derivable region.
  groupOps(scene: ObjectScene, ids: string[], frameId: string): ObjectOp[] | null;
  // The ops dissolving the container `frameId`: one reparent per child re-homing it to the frame's parent
  // (grandparent or canvas root), then a delete of the empty frame. Null when `frameId` is unknown.
  ungroupOps(scene: ObjectScene, frameId: string): ObjectOp[] | null;
  // The create-gesture screen-px thresholds (recognize), read once at init so the shell holds no
  // literal copy. Each is divided by the live zoom before it crosses back into a core call.
  createThresholds(): CreateThresholds;
  // The order key for a NEW object landing on top, minted through fractional indexing (strictly above
  // the scene's max order; the canonical first key on an empty scene). The shell never invents an order key.
  nextOrderKey(scene: ObjectScene): string;
  // The order key for a NEW object landing at the back (strictly below the scene's min order).
  backOrderKey(scene: ObjectScene): string;
  // A key strictly between `a` and `b` (null = open end). Throws when a bound is malformed or `a >= b`.
  keyBetween(a: string | null, b: string | null): string;
  // The 2-op `reorder` swap stepping `id` one place toward the front/back over the flat scene order,
  // or null when `id` is unknown or has no neighbor in that direction (already at the relevant extent).
  reorderStepOps(scene: ObjectScene, id: string, direction: ReorderDirection): ObjectOp[] | null;
  createUndoStack(actorId: string): UndoStack;
};

let modulePromise: Promise<SceneCoreModule> | null = null;
// Set once the wasm instance is initialized; backs the synchronous op-apply on the sync engine's hot path.
let readyModule: SceneCoreModule | null = null;

// True under Node/vitest (no `fetch`-served wasm), false in the browser.
function isNodeRuntime(): boolean {
  return (
    typeof process !== "undefined" &&
    process.versions != null &&
    process.versions.node != null &&
    typeof (globalThis as { window?: unknown }).window === "undefined"
  );
}

// Init the wasm instance under Node/vitest: the `--target web` module cannot `fetch()`, so read
// the sibling `.wasm` from disk and init synchronously via `initSync`. The `node:` specifiers are
// assembled at runtime and `@vite-ignore`d so Vite's browser bundler never statically resolves them.
async function initUnderNode(mod: SceneCoreModule): Promise<void> {
  const nodeImport = (name: string) => import(/* @vite-ignore */ `node:${name}`);
  const { readFileSync } = (await nodeImport("fs")) as typeof import("node:fs");
  const { fileURLToPath } = (await nodeImport("url")) as typeof import("node:url");
  const wasmUrl = new URL("./wasm/shape_scene_core_bg.wasm", import.meta.url);
  const bytes = readFileSync(fileURLToPath(wasmUrl));
  mod.initSync({ module: bytes });
}

// Parse a bridge return string: throw on the `{ error }` shape (malformed input / serialize failure),
// else return the parsed value. Domain `errors[]` fields pass through untouched.
function parseBridge<T>(fnName: string, raw: string): T {
  const value = JSON.parse(raw) as T | { error: string };
  if (value && typeof value === "object" && "error" in value && typeof value.error === "string") {
    throw new Error(`scene-core ${fnName}: ${value.error}`);
  }
  return value as T;
}

async function loadModule(): Promise<SceneCoreModule> {
  if (!modulePromise) {
    modulePromise = (async () => {
      const modulePath = "./wasm/shape_scene_core.js";
      const mod = (await import(/* @vite-ignore */ modulePath)) as SceneCoreModule;
      // Browser: `default` fetches the sibling `.wasm`. Node/vitest: read bytes from disk, init sync.
      if (isNodeRuntime()) await initUnderNode(mod);
      else await mod.default();
      readyModule = mod;
      return mod;
    })();
  }
  return modulePromise;
}

// Lazy-load the scene-core wasm package and return a typed handle. Idempotent: the underlying
// module + wasm instance are initialized once and shared.
export async function loadSceneCore(): Promise<SceneCore> {
  const mod = await loadModule();
  return {
    applyObjectOp(scene, op) {
      return parseBridge<ObjectApplyResult>(
        "apply_object_op",
        mod.apply_object_op(JSON.stringify(scene), JSON.stringify(op))
      );
    },
    deriveRegion(geometry, flatness) {
      return parseBridge<DerivedRegion>(
        "derive_region",
        mod.derive_region(JSON.stringify(geometry), flatness)
      );
    },
    objectCommandCatalog() {
      return parseBridge<ObjectCommand[]>(
        "object_command_catalog",
        mod.object_command_catalog()
      );
    },
    objectGestureCatalog() {
      return parseBridge<ObjectGesture[]>(
        "object_gesture_catalog",
        mod.object_gesture_catalog()
      );
    },
    objectInspectorCatalog() {
      return parseBridge<InspectorControl[]>(
        "object_inspector_catalog",
        mod.object_inspector_catalog()
      );
    },
    objectInspectorView(sceneJson, selectionJson) {
      return parseBridge<InspectorView>(
        "object_inspector_view",
        mod.object_inspector_view(sceneJson, selectionJson)
      );
    },
    objectSetTransformField(matrixJson, field, value) {
      return parseBridge<Transform3x3>(
        "object_set_transform_field",
        mod.object_set_transform_field(matrixJson, field, value)
      );
    },
    objectQuantizeUnits(px, scale) {
      return mod.object_quantize_units(px, scale);
    },
    objectResizeAxis(transformJson, geometryJson, axis, targetPx) {
      return parseBridge<Transform3x3>(
        "object_resize_axis",
        mod.object_resize_axis(transformJson, geometryJson, axis, targetPx)
      );
    },
    objectInspectorEditOp(object, controlId, value, unitScale) {
      return parseBridge<ObjectOp | null>(
        "object_inspector_edit_op",
        mod.object_inspector_edit_op(
          JSON.stringify(object),
          controlId,
          JSON.stringify(value ?? null),
          unitScale
        )
      );
    },
    templateAnchor(scene, fallback) {
      const [x, y] = parseBridge<[number, number]>(
        "template_anchor",
        mod.template_anchor(JSON.stringify(scene), fallback.x, fallback.y)
      );
      return { x, y };
    },
    buildObjectTemplate(templateId, anchorX, anchorY, idPrefix) {
      return parseBridge<SceneObject[]>(
        "build_object_template",
        mod.build_object_template(templateId, anchorX, anchorY, idPrefix)
      );
    },
    freehandToObject(points, color, widthPx, id, order, mode) {
      return parseBridge<SceneObject>(
        "freehand_to_object",
        mod.freehand_to_object(
          JSON.stringify(points.map((p) => [p.x, p.y])),
          color,
          widthPx,
          id,
          order,
          mode
        )
      );
    },
    buildPrimitive(kind, anchor, id, order, color) {
      return parseBridge<SceneObject>(
        "build_primitive",
        mod.build_primitive(kind, anchor.x, anchor.y, color ?? "", id, order)
      );
    },
    buildPrimitiveFromDrag(kind, span, id, order, color) {
      return parseBridge<SceneObject>(
        "build_primitive_from_drag",
        mod.build_primitive_from_drag(
          kind,
          span.start.x,
          span.start.y,
          span.end.x,
          span.end.y,
          color ?? "",
          id,
          order
        )
      );
    },
    buildSetStyleOp(object, color) {
      return parseBridge<ObjectOp>(
        "build_set_style_op",
        mod.build_set_style_op(JSON.stringify(object), color)
      );
    },
    isOpenClassD(d) {
      return parseBridge<boolean>("is_open_class_d", mod.is_open_class_d(d));
    },
    mergeOpenStrokeOps(scene, points, mode, tolerancePx) {
      return parseBridge<ObjectOp[] | null>(
        "merge_open_stroke_ops",
        mod.merge_open_stroke_ops(
          JSON.stringify(scene),
          JSON.stringify(points.map((p) => [p.x, p.y])),
          mode,
          tolerancePx
        )
      );
    },
    splitSubpathAt(geometry, x, y, radius) {
      // A missed touch comes back as `{error}` (nothing to cut); treat as a no-op (null), not a throw.
      const raw = mod.split_subpath_at(JSON.stringify(geometry), x, y, radius);
      const value = JSON.parse(raw) as SceneObject["geometry"] | { error: string };
      if (value && typeof value === "object" && "error" in value && typeof value.error === "string") {
        return null;
      }
      return value as SceneObject["geometry"];
    },
    partialEraseOps(scene, id, wx, wy, radius) {
      return parseBridge<ObjectOp[]>(
        "partial_erase_ops",
        mod.partial_erase_ops(JSON.stringify(scene), id, wx, wy, radius)
      );
    },
    objectWorldAabb(object) {
      // A node-less object comes back as `{error}`; treat as null (no bbox), not a throw.
      const raw = mod.object_world_aabb(JSON.stringify(object));
      const value = JSON.parse(raw) as
        | { minX: number; minY: number; maxX: number; maxY: number }
        | { error: string };
      if (value && typeof value === "object" && "error" in value && typeof value.error === "string") {
        return null;
      }
      return value as { minX: number; minY: number; maxX: number; maxY: number };
    },
    anchorFollowOps(scene, ops) {
      return parseBridge<ObjectOp[]>(
        "anchor_follow_ops",
        mod.anchor_follow_ops(JSON.stringify(scene), JSON.stringify(ops))
      );
    },
    moveOps(scene, roots, delta) {
      return parseBridge<ObjectOp[]>(
        "move_ops",
        mod.move_ops(JSON.stringify(scene), JSON.stringify(roots), JSON.stringify(delta))
      );
    },
    moveOpsForPick(scene, selection, id, delta) {
      return parseBridge<ObjectOp[]>(
        "move_ops_for_pick",
        mod.move_ops_for_pick(JSON.stringify(scene), JSON.stringify(selection), id, JSON.stringify(delta))
      );
    },
    validSelection(scene, selection) {
      return parseBridge<ObjectSelection>(
        "valid_selection",
        mod.valid_selection(JSON.stringify(scene), JSON.stringify(selection))
      );
    },
    selectAll(scene) {
      return parseBridge<ObjectSelection>("select_all", mod.select_all(JSON.stringify(scene)));
    },
    duplicateOps(scene, ids, idPrefix, orderSeed) {
      return parseBridge<ObjectOp[]>(
        "duplicate_ops",
        mod.duplicate_ops(JSON.stringify(scene), JSON.stringify(ids), idPrefix, orderSeed)
      );
    },
    detachMoveOps(scene, id, delta) {
      return parseBridge<ObjectOp[]>(
        "detach_move_ops",
        mod.detach_move_ops(JSON.stringify(scene), id, JSON.stringify(delta))
      );
    },
    synthesizeCreateAnchors(created, target, endpoint) {
      return parseBridge<Anchor[] | null>(
        "synthesize_create_anchors",
        mod.synthesize_create_anchors(
          JSON.stringify(created),
          JSON.stringify(target),
          endpoint.x,
          endpoint.y
        )
      );
    },
    synthesizeCreateAnchorsBoth(scene, created, corners) {
      const wire = corners.map((c) => (c ? { target: c.target, x: c.at.x, y: c.at.y } : null));
      return parseBridge<Anchor[]>(
        "synthesize_create_anchors_both",
        mod.synthesize_create_anchors_both(
          JSON.stringify(scene),
          JSON.stringify(created),
          JSON.stringify(wire)
        )
      );
    },
    resolveCreateRelease(release, lastSnap, toleranceWorld) {
      const resolved = parseBridge<{ end: [number, number]; target: string | null }>(
        "resolve_create_release",
        mod.resolve_create_release(
          JSON.stringify({
            x: release.end.x,
            y: release.end.y,
            snapped: release.snapped,
            target: release.target ?? ""
          }),
          lastSnap ? JSON.stringify({ x: lastSnap.at.x, y: lastSnap.at.y, target: lastSnap.target }) : "",
          toleranceWorld
        )
      );
      return { end: { x: resolved.end[0], y: resolved.end[1] }, target: resolved.target };
    },
    endpointReleaseOps(scene, id, nodeIndex, newPoint, snap) {
      return parseBridge<ObjectOp[]>(
        "endpoint_release_ops",
        mod.endpoint_release_ops(
          JSON.stringify(scene),
          id,
          nodeIndex,
          newPoint.x,
          newPoint.y,
          snap?.targetId ?? "",
          snap?.at ? JSON.stringify(snap.at) : ""
        )
      );
    },
    popOutOp(scene, id) {
      return parseBridge<ObjectOp | null>("pop_out_op", mod.pop_out_op(JSON.stringify(scene), id));
    },
    hasChildren(scene, id) {
      return parseBridge<boolean>("has_children", mod.has_children(JSON.stringify(scene), id));
    },
    ungroupEnabled(scene, selectedId) {
      return parseBridge<boolean>(
        "ungroup_enabled",
        mod.ungroup_enabled(JSON.stringify(scene), selectedId ?? "")
      );
    },
    doubleClickAction(scene, id) {
      return parseBridge<DoubleClickAction>(
        "double_click_action",
        mod.double_click_action(JSON.stringify(scene), id)
      );
    },
    objectSelectionInScope(scene, selection, container) {
      return parseBridge<boolean>(
        "object_selection_in_scope",
        mod.object_selection_in_scope(JSON.stringify(scene), JSON.stringify(selection), container)
      );
    },
    groupOps(scene, ids, frameId) {
      return parseBridge<ObjectOp[] | null>(
        "group_ops",
        mod.group_ops(JSON.stringify(scene), JSON.stringify(ids), frameId)
      );
    },
    ungroupOps(scene, frameId) {
      return parseBridge<ObjectOp[] | null>(
        "ungroup_ops",
        mod.ungroup_ops(JSON.stringify(scene), frameId)
      );
    },
    createThresholds() {
      return parseBridge<CreateThresholds>("create_thresholds", mod.create_thresholds());
    },
    nextOrderKey(scene) {
      return parseBridge<string>("next_order_key", mod.next_order_key(JSON.stringify(scene)));
    },
    backOrderKey(scene) {
      return parseBridge<string>("back_order_key", mod.back_order_key(JSON.stringify(scene)));
    },
    keyBetween(a, b) {
      return parseBridge<string>("key_between", mod.key_between(a ?? "", b ?? ""));
    },
    reorderStepOps(scene, id, direction) {
      return parseBridge<ObjectOp[] | null>(
        "reorder_step_ops",
        mod.reorder_step_ops(JSON.stringify(scene), id, direction)
      );
    },
    createUndoStack(actorId) {
      const inner = new mod.WasmUndoStack(actorId);
      const popOp = (raw: string | undefined): ObjectOp | null =>
        raw === undefined ? null : (JSON.parse(raw) as ObjectOp);
      return {
        record(forward, inverse) {
          inner.record(JSON.stringify(forward), JSON.stringify(inverse));
        },
        beginCoalesce() {
          inner.begin_coalesce();
        },
        endCoalesce() {
          inner.end_coalesce();
        },
        isCoalescing() {
          return inner.is_coalescing();
        },
        undo() {
          return popOp(inner.undo());
        },
        noteUndoApplied(reInverse) {
          inner.note_undo_applied(JSON.stringify(reInverse));
        },
        redo() {
          return popOp(inner.redo());
        },
        noteRedoApplied(inverse) {
          inner.note_redo_applied(JSON.stringify(inverse));
        },
        abort() {
          inner.abort();
        },
        canUndo() {
          return inner.can_undo();
        },
        canRedo() {
          return inner.can_redo();
        }
      };
    }
  };
}

// Idempotently initialize the scene-core wasm instance so the synchronous `applyObjectOpSync`
// can run afterwards. The same `--target web` artifact backs the browser and Node/vitest.
export async function ensureSceneCore(): Promise<void> {
  await loadModule();
}

// Synchronous object op-apply, the sync engine's hot path. Requires `ensureSceneCore` to have
// resolved (throws otherwise), since the engine's `author`/`applyRemote`/`reconcileSnapshot` are
// synchronous and cannot await an init. Returns the next scene plus the inverse op (undo entry).
export function applyObjectOpSync(scene: ObjectScene, op: ObjectOp): ObjectApplyResult {
  if (!readyModule) {
    throw new Error("scene-core wasm is not initialized; await ensureSceneCore() before applyObjectOpSync()");
  }
  return parseBridge<ObjectApplyResult>(
    "apply_object_op",
    readyModule.apply_object_op(JSON.stringify(scene), JSON.stringify(op))
  );
}

export type { WasmSession, WasmWindow };

// Construct a collaboration session over the wasm core. Requires `ensureSceneCore` to have resolved
// (throws otherwise), since the session methods are synchronous. `coalesceMs < 0` / `peerTtlMs < 0`
// use the core defaults; an empty `selfUserId` disables peer self-skip.
export function createWasmSession(args: {
  welcomeScene: ObjectScene;
  clientId: string;
  selfUserId?: string;
  coalesceMs?: number;
  peerTtlMs?: number;
}): WasmSession {
  if (!readyModule) {
    throw new Error("scene-core wasm is not initialized; await ensureSceneCore() before createWasmSession()");
  }
  return new readyModule.WasmSession(
    JSON.stringify(args.welcomeScene),
    args.clientId,
    args.selfUserId ?? "",
    args.coalesceMs ?? -1,
    args.peerTtlMs ?? -1
  );
}

// Construct the viewport-windowing decision state over the wasm core. Requires `ensureSceneCore`
// to have resolved. The `seed` bbox is the connect region's window (omit for whole-canvas);
// `margin < 0` uses the core default.
export function createWasmWindow(args: {
  seed?: { x: number; y: number; width: number; height: number };
  margin?: number;
}): WasmWindow {
  if (!readyModule) {
    throw new Error("scene-core wasm is not initialized; await ensureSceneCore() before createWasmWindow()");
  }
  return new readyModule.WasmWindow(args.seed ? JSON.stringify(args.seed) : "", args.margin ?? -1);
}
